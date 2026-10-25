use anyhow::{Context, Result, bail};
use chrono::{Days, NaiveDate, NaiveDateTime};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Db, json_column, line_from_row};
use crate::domain::{Invoice, LineItem, Quantity, Snapshot};

impl Db {
    /// All invoices, newest first, with their lines.
    pub fn invoices(&self) -> Result<Vec<Invoice>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM invoices
             ORDER BY issued_at IS NOT NULL, issue_date DESC, number DESC, id DESC",
        )?;
        let mut invoices: Vec<Invoice> = stmt
            .query_map([], invoice_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        for inv in &mut invoices {
            inv.lines = load_lines(&self.conn, inv.id)?;
        }
        Ok(invoices)
    }

    pub fn invoice(&self, id: i64) -> Result<Invoice> {
        let mut inv = self
            .conn
            .query_row(
                "SELECT * FROM invoices WHERE id = ?1",
                [id],
                invoice_from_row,
            )
            .optional()?
            .with_context(|| format!("invoice {id} not found"))?;
        inv.lines = load_lines(&self.conn, id)?;
        Ok(inv)
    }

    /// Saves a new draft and returns its id. `inv.id` is ignored.
    pub fn create_draft(&self, inv: &Invoice) -> Result<i64> {
        validate_draft(inv)?;
        self.validate_reimbursements(inv)?;
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO invoices (client_id, issue_date, due_date, supply_start, supply_end, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                inv.client_id,
                inv.issue_date,
                inv.due_date,
                inv.supply_start,
                inv.supply_end,
                inv.notes
            ],
        )?;
        let id = tx.last_insert_rowid();
        insert_lines(&tx, "line_items", "invoice_id", id, &inv.lines)?;
        tx.commit()?;
        Ok(id)
    }

    /// Replaces a draft's header and lines. Fails if the invoice has been issued.
    pub fn update_draft(&self, inv: &Invoice) -> Result<()> {
        validate_draft(inv)?;
        self.validate_reimbursements(inv)?;
        let tx = self.conn.unchecked_transaction()?;
        let issued: Option<Option<String>> = tx
            .query_row(
                "SELECT issued_at FROM invoices WHERE id = ?1",
                [inv.id],
                |r| r.get(0),
            )
            .optional()?;
        match issued {
            None => bail!("invoice {} not found", inv.id),
            Some(Some(_)) => bail!("invoice {} is issued and locked", inv.display_number()),
            Some(None) => {}
        }
        tx.execute(
            "UPDATE invoices SET client_id = ?2, issue_date = ?3, due_date = ?4,
                supply_start = ?5, supply_end = ?6, notes = ?7
             WHERE id = ?1",
            params![
                inv.id,
                inv.client_id,
                inv.issue_date,
                inv.due_date,
                inv.supply_start,
                inv.supply_end,
                inv.notes
            ],
        )?;
        tx.execute("DELETE FROM line_items WHERE invoice_id = ?1", [inv.id])?;
        insert_lines(&tx, "line_items", "invoice_id", inv.id, &inv.lines)?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_draft(&self, id: i64) -> Result<()> {
        let inv = self.invoice(id)?;
        if !inv.is_draft() {
            bail!("issued invoices cannot be deleted; raise a credit note instead");
        }
        self.conn
            .execute("DELETE FROM invoices WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Issues a draft: assigns the next number, freezes business and client
    /// details, and locks it. `before_commit` runs with the issued invoice
    /// inside the transaction (used to write the PDF); if it fails, nothing
    /// is issued and the number is not consumed.
    pub fn issue_invoice(
        &self,
        id: i64,
        now: NaiveDateTime,
        before_commit: impl FnOnce(&Invoice) -> Result<()>,
    ) -> Result<Invoice> {
        let mut inv = self.invoice(id)?;
        if !inv.is_draft() {
            bail!("invoice {} has already been issued", inv.display_number());
        }
        if inv.lines.is_empty() {
            bail!("add at least one line before issuing");
        }
        if !inv.total().is_positive() {
            bail!("the invoice total must be more than £0.00");
        }
        // The expense may have changed since the draft was saved.
        self.validate_reimbursements(&inv)?;
        let business = self.require_business()?;
        let client = self.client(inv.client_id)?;
        let snapshot = Snapshot {
            business: business.clone(),
            client,
        };

        let tx = self.conn.unchecked_transaction()?;
        // Freeze each reimbursement's detail as the expense stands now.
        self.refresh_reimbursement_details(&mut inv.lines)?;
        for (pos, line) in inv.lines.iter().enumerate() {
            if line.is_reimbursement() {
                tx.execute(
                    "UPDATE line_items SET detail = ?3 WHERE invoice_id = ?1 AND position = ?2",
                    params![id, pos as i64, line.detail],
                )?;
            }
        }
        let number = Self::next_number(
            &tx,
            "invoice",
            &business.invoice_pattern,
            inv.issue_date,
            &snapshot.client,
            |c, n| number_taken(c, "invoices", n),
        )?;
        tx.execute(
            "UPDATE invoices SET number = ?2, issued_at = ?3, snapshot = ?4 WHERE id = ?1",
            params![id, number, now, serde_json::to_string(&snapshot)?],
        )?;
        inv.number = Some(number);
        inv.issued_at = Some(now);
        inv.snapshot = Some(snapshot);
        before_commit(&inv)?;
        tx.commit()?;
        Ok(inv)
    }

    /// What a draft would look like if it were issued now, without saving
    /// anything or using up a number. `inv` may have unsaved edits.
    pub fn preview_invoice(&self, inv: &Invoice) -> Result<Preview> {
        if !inv.is_draft() {
            bail!("invoice {} is already issued", inv.display_number());
        }
        let business = self.require_business()?;
        let client = self.client(inv.client_id)?;
        let number = Self::peek_number(
            &self.conn,
            "invoice",
            &business.invoice_pattern,
            inv.issue_date,
            &client,
            |c, n| number_taken(c, "invoices", n),
        )
        .map(|(_, _, number)| number)
        .map_err(|e| format!("{e:#}"));
        let mut invoice = inv.clone();
        self.refresh_reimbursement_details(&mut invoice.lines)?;
        invoice.snapshot = Some(Snapshot { business, client });
        Ok(Preview { invoice, number })
    }

    /// Replaces an issued invoice's business and client details with the
    /// current ones and records `today` as when that happened. The number,
    /// dates, lines and amounts don't change, and the client label is kept so
    /// numbering stays continuous. Returns the updated invoice.
    pub fn update_issued_details(&self, id: i64, today: NaiveDate) -> Result<Invoice> {
        let mut inv = self.invoice(id)?;
        let Some(old) = &inv.snapshot else {
            bail!("only issued invoices have details to update");
        };
        let mut client = self.client(inv.client_id)?;
        client.label = old.client.label.clone();
        let snapshot = Snapshot {
            business: self.require_business()?,
            client,
        };
        self.conn.execute(
            "UPDATE invoices SET snapshot = ?2, details_updated_on = ?3 WHERE id = ?1",
            params![id, serde_json::to_string(&snapshot)?, today],
        )?;
        inv.snapshot = Some(snapshot);
        inv.details_updated_on = Some(today);
        Ok(inv)
    }

    /// Records (or with `None`, clears) the date an issued invoice was sent.
    pub fn set_sent(&self, id: i64, sent_on: Option<NaiveDate>) -> Result<()> {
        let inv = self.invoice(id)?;
        if inv.is_draft() {
            bail!("issue the invoice before marking it sent");
        }
        self.conn.execute(
            "UPDATE invoices SET sent_on = ?2 WHERE id = ?1",
            params![id, sent_on],
        )?;
        Ok(())
    }

    /// Copies an invoice's client, lines and notes into a new draft dated `today`.
    /// Reimbursement lines are left out: each purchase is billed only once.
    pub fn duplicate_invoice(&self, id: i64, today: NaiveDate) -> Result<i64> {
        let mut src = self.invoice(id)?;
        src.lines.retain(|l| !l.is_reimbursement());
        let terms = self.require_business()?.payment_terms_days;
        let copy = Invoice {
            id: 0,
            number: None,
            client_id: src.client_id,
            issue_date: today,
            due_date: today + Days::new(terms.into()),
            supply_start: today,
            supply_end: None,
            notes: src.notes,
            issued_at: None,
            sent_on: None,
            snapshot: None,
            details_updated_on: None,
            lines: src.lines,
        };
        self.create_draft(&copy)
    }
}

/// A draft as it would be issued now. See [`Db::preview_invoice`].
#[derive(Debug, Clone)]
pub struct Preview {
    /// The draft with today's business and client details attached; still unnumbered.
    pub invoice: Invoice,
    /// The number it would get, or why it can't get one yet.
    pub number: Result<String, String>,
}

impl Db {
    /// Sets each reimbursement line's detail from its expense as it is now.
    fn refresh_reimbursement_details(&self, lines: &mut [LineItem]) -> Result<()> {
        for line in lines {
            if let Some(expense_id) = line.expense_id {
                line.detail = self.expense(expense_id)?.invoice_detail();
            }
        }
        Ok(())
    }

    /// Each reimbursement line must bill, at its exact amount, an approved
    /// expense marked for this invoice's client that isn't on another invoice.
    fn validate_reimbursements(&self, inv: &Invoice) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for (i, line) in inv.lines.iter().enumerate() {
            let Some(expense_id) = line.expense_id else {
                continue;
            };
            let n = i + 1;
            let e = self.expense(expense_id)?;
            if !seen.insert(expense_id) {
                bail!("line {n} ({}) is on this invoice twice", e.description);
            }
            if e.rebill_client_id != Some(inv.client_id) {
                bail!(
                    "line {n} ({}) is a reimbursement for a different client. Remove it, or change who it's rebilled to under Expenses",
                    e.description
                );
            }
            if !e.is_approved() {
                bail!(
                    "line {n} ({}) hasn't been approved. Record who approved it and when under Expenses",
                    e.description
                );
            }
            if line.quantity != Quantity::ONE || line.unit_price != e.amount {
                bail!("line {n} must bill the expense's amount, {}", e.amount);
            }
            if let Some(billed) = super::tax::billed_on(&self.conn, expense_id)?
                && billed.invoice_id != inv.id
            {
                bail!(
                    "line {n} ({}) is already billed on {}",
                    e.description,
                    billed.display()
                );
            }
        }
        Ok(())
    }
}

fn validate_draft(inv: &Invoice) -> Result<()> {
    if inv.due_date < inv.issue_date {
        bail!("the due date can't be before the issue date");
    }
    if let Some(end) = inv.supply_end
        && end < inv.supply_start
    {
        bail!("the supply period ends before it starts");
    }
    for (i, line) in inv.lines.iter().enumerate() {
        if line.description.trim().is_empty() {
            bail!("line {} needs a description", i + 1);
        }
    }
    Ok(())
}

pub(super) fn number_taken(conn: &Connection, table: &str, number: &str) -> Result<bool> {
    let sql = format!("SELECT EXISTS (SELECT 1 FROM {table} WHERE number = ?1)");
    Ok(conn.query_row(&sql, [number], |r| r.get(0))?)
}

pub(super) fn insert_lines(
    conn: &Connection,
    table: &str,
    parent_col: &str,
    parent_id: i64,
    lines: &[LineItem],
) -> Result<()> {
    let sql = format!(
        "INSERT INTO {table} ({parent_col}, position, kind, description, quantity_centi, unit_price_pence,
             detail, expense_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
    );
    let mut stmt = conn.prepare(&sql)?;
    for (pos, line) in lines.iter().enumerate() {
        stmt.execute(params![
            parent_id,
            pos as i64,
            line.kind.as_str(),
            line.description.trim(),
            line.quantity.hundredths(),
            line.unit_price.pence(),
            line.detail.trim(),
            line.expense_id,
        ])?;
    }
    Ok(())
}

fn load_lines(conn: &Connection, invoice_id: i64) -> Result<Vec<LineItem>> {
    let mut stmt =
        conn.prepare_cached("SELECT * FROM line_items WHERE invoice_id = ?1 ORDER BY position")?;
    let rows = stmt.query_map([invoice_id], line_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn invoice_from_row(r: &Row) -> rusqlite::Result<Invoice> {
    Ok(Invoice {
        id: r.get("id")?,
        number: r.get("number")?,
        client_id: r.get("client_id")?,
        issue_date: r.get("issue_date")?,
        due_date: r.get("due_date")?,
        supply_start: r.get("supply_start")?,
        supply_end: r.get("supply_end")?,
        notes: r.get("notes")?,
        issued_at: r.get("issued_at")?,
        sent_on: r.get("sent_on")?,
        snapshot: json_column(r, "snapshot")?,
        details_updated_on: r.get("details_updated_on")?,
        lines: Vec::new(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::test_support::*;
    use super::*;
    use crate::domain::{LineKind, Money, Quantity};

    pub fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    pub fn now() -> NaiveDateTime {
        d(2026, 10, 2).and_hms_opt(12, 0, 0).unwrap()
    }

    pub fn draft(client_id: i64, issue: NaiveDate) -> Invoice {
        Invoice {
            id: 0,
            number: None,
            client_id,
            issue_date: issue,
            due_date: issue + Days::new(30),
            supply_start: issue,
            supply_end: None,
            notes: String::new(),
            issued_at: None,
            sent_on: None,
            snapshot: None,
            details_updated_on: None,
            lines: vec![
                LineItem::new(
                    LineKind::Hours,
                    "Development",
                    Quantity::from_hundredths(750),
                    Money::from_pence(6000),
                ),
                LineItem::new(
                    LineKind::Fixed,
                    "Hosting setup",
                    Quantity::ONE,
                    Money::from_pence(15000),
                ),
            ],
        }
    }

    #[test]
    fn draft_round_trip_and_update() {
        let (db, client) = db_with_client();
        let id = db.create_draft(&draft(client, d(2026, 10, 2))).unwrap();
        let mut inv = db.invoice(id).unwrap();
        assert!(inv.is_draft());
        assert_eq!(inv.total(), Money::from_pence(60_000));
        inv.lines.remove(0);
        inv.notes = "Updated".into();
        db.update_draft(&inv).unwrap();
        let reloaded = db.invoice(id).unwrap();
        assert_eq!(reloaded.lines.len(), 1);
        assert_eq!(reloaded.notes, "Updated");
    }

    #[test]
    fn issuing_assigns_sequential_numbers_and_locks() {
        let (db, client) = db_with_client();
        let a = db.create_draft(&draft(client, d(2026, 10, 2))).unwrap();
        let b = db.create_draft(&draft(client, d(2026, 10, 3))).unwrap();
        let a = db.issue_invoice(a, now(), |_| Ok(())).unwrap();
        let b = db.issue_invoice(b, now(), |_| Ok(())).unwrap();
        assert_eq!(a.number.as_deref(), Some("2026-0001"));
        assert_eq!(b.number.as_deref(), Some("2026-0002"));
        assert_eq!(a.snapshot.as_ref().unwrap().client.name, "Acme Ltd");

        // Locked: no edits, no deletes, even bypassing the Rust checks.
        assert!(db.update_draft(&a).is_err());
        assert!(db.delete_draft(a.id).is_err());
        assert!(
            db.connection()
                .execute("DELETE FROM line_items WHERE invoice_id = ?1", [a.id])
                .is_err()
        );
        assert!(
            db.connection()
                .execute("UPDATE invoices SET notes = 'x' WHERE id = ?1", [a.id])
                .is_err()
        );
        assert!(
            db.connection()
                .execute("DELETE FROM invoices WHERE id = ?1", [a.id])
                .is_err()
        );
        // ...but marking as sent is allowed.
        db.set_sent(a.id, Some(d(2026, 10, 2))).unwrap();
        assert_eq!(db.invoice(a.id).unwrap().sent_on, Some(d(2026, 10, 2)));
    }

    #[test]
    fn sequence_restarts_each_calendar_year() {
        let (db, client) = db_with_client();
        let a = db.create_draft(&draft(client, d(2026, 12, 31))).unwrap();
        let b = db.create_draft(&draft(client, d(2027, 1, 1))).unwrap();
        assert_eq!(
            db.issue_invoice(a, now(), |_| Ok(()))
                .unwrap()
                .number
                .unwrap(),
            "2026-0001"
        );
        assert_eq!(
            db.issue_invoice(b, now(), |_| Ok(()))
                .unwrap()
                .number
                .unwrap(),
            "2027-0001"
        );
    }

    #[test]
    fn failed_issue_rolls_back_and_does_not_consume_a_number() {
        let (db, client) = db_with_client();
        let a = db.create_draft(&draft(client, d(2026, 10, 2))).unwrap();
        assert!(
            db.issue_invoice(a, now(), |_| anyhow::bail!("pdf failed"))
                .is_err()
        );
        assert!(db.invoice(a).unwrap().is_draft());
        assert_eq!(
            db.issue_invoice(a, now(), |_| Ok(()))
                .unwrap()
                .number
                .unwrap(),
            "2026-0001"
        );
    }

    #[test]
    fn pattern_change_skips_numbers_already_used() {
        let (db, client) = db_with_client();
        let mut b = db.require_business().unwrap();
        b.invoice_pattern = "INV-{SEQ:3}".into();
        db.save_business(&b).unwrap();
        let a = db.create_draft(&draft(client, d(2026, 10, 2))).unwrap();
        db.issue_invoice(a, now(), |_| Ok(())).unwrap();
        // Simulate the counter being lost/reset: the next number must still be unique.
        db.connection()
            .execute("DELETE FROM sequences", [])
            .unwrap();
        let c = db.create_draft(&draft(client, d(2026, 10, 2))).unwrap();
        assert_eq!(
            db.issue_invoice(c, now(), |_| Ok(()))
                .unwrap()
                .number
                .unwrap(),
            "INV-002"
        );
    }

    fn use_label_pattern(db: &Db) {
        let mut b = db.require_business().unwrap();
        b.invoice_pattern = "{LABEL}-{YEAR}-{SEQ:4}".into();
        db.save_business(&b).unwrap();
    }

    fn add_client(db: &Db, name: &str, label: &str) -> i64 {
        let mut c = crate::domain::Client::new_empty();
        c.name = name.into();
        c.label = label.into();
        db.insert_client(&c).unwrap()
    }

    #[test]
    fn labelled_numbers_count_per_client() {
        let (db, acme) = db_with_client();
        use_label_pattern(&db);
        let mut c = db.client(acme).unwrap();
        c.label = "acm".into(); // normalised to ACM
        db.update_client(&c).unwrap();
        let lgt = add_client(&db, "Logitech", "LGT");

        let issue = |client| {
            let id = db.create_draft(&draft(client, d(2026, 10, 2))).unwrap();
            db.issue_invoice(id, now(), |_| Ok(()))
                .unwrap()
                .number
                .unwrap()
        };
        assert_eq!(issue(lgt), "LGT-2026-0001");
        assert_eq!(issue(acme), "ACM-2026-0001");
        assert_eq!(issue(lgt), "LGT-2026-0002");
        let next_year = db.create_draft(&draft(lgt, d(2027, 1, 4))).unwrap();
        assert_eq!(
            db.issue_invoice(next_year, now(), |_| Ok(()))
                .unwrap()
                .number
                .unwrap(),
            "LGT-2027-0001"
        );
    }

    #[test]
    fn issuing_requires_a_label_when_the_pattern_uses_one() {
        let (db, acme) = db_with_client();
        use_label_pattern(&db);
        let id = db.create_draft(&draft(acme, d(2026, 10, 2))).unwrap();
        let err = db.issue_invoice(id, now(), |_| Ok(())).unwrap_err();
        assert!(err.to_string().contains("Acme Ltd has no label"), "{err}");
        assert!(db.invoice(id).unwrap().is_draft());
    }

    #[test]
    fn labels_are_unique_and_validated() {
        let (db, _) = db_with_client();
        add_client(&db, "Logitech", "LGT");
        let mut dup = crate::domain::Client::new_empty();
        dup.name = "Lighthouse Group".into();
        dup.label = "lgt".into();
        let err = db.insert_client(&dup).unwrap_err();
        assert!(
            err.to_string().contains("already used by Logitech"),
            "{err}"
        );
        dup.label = "L-G".into();
        assert!(db.insert_client(&dup).is_err());
        // Several clients may have no label.
        add_client(&db, "No Label One", "");
        add_client(&db, "No Label Two", "");
    }

    #[test]
    fn label_locks_once_used_on_an_issued_invoice() {
        let (db, acme) = db_with_client();
        // Issued while the client had no label…
        let id = db.create_draft(&draft(acme, d(2026, 10, 2))).unwrap();
        db.issue_invoice(id, now(), |_| Ok(())).unwrap();
        // …so a first label can still be set, and changed while unused.
        let mut c = db.client(acme).unwrap();
        c.label = "ACM".into();
        db.update_client(&c).unwrap();
        c.label = "ACME".into();
        db.update_client(&c).unwrap();
        // Once an invoice is issued with it, it's locked.
        let id = db.create_draft(&draft(acme, d(2026, 10, 3))).unwrap();
        db.issue_invoice(id, now(), |_| Ok(())).unwrap();
        c.label = "AC".into();
        let err = db.update_client(&c).unwrap_err();
        assert!(err.to_string().contains("label can't change"), "{err}");
        // Other fields can still change.
        c.label = "ACME".into();
        c.address.country = "Ireland".into();
        db.update_client(&c).unwrap();
    }

    #[test]
    fn credit_note_patterns_can_use_the_label_too() {
        let (db, acme) = db_with_client();
        let mut b = db.require_business().unwrap();
        b.credit_note_pattern = "CN-{LABEL}-{SEQ:3}".into();
        db.save_business(&b).unwrap();
        let mut c = db.client(acme).unwrap();
        c.label = "ACM".into();
        db.update_client(&c).unwrap();
        let id = db.create_draft(&draft(acme, d(2026, 10, 2))).unwrap();
        db.issue_invoice(id, now(), |_| Ok(())).unwrap();
        let line = draft(acme, d(2026, 10, 2)).lines[1].clone();
        let cn = db
            .create_credit_note(id, d(2026, 10, 5), "Cancelled", vec![line], |_, _| Ok(()))
            .unwrap();
        assert_eq!(cn.number, "CN-ACM-001");
    }

    #[test]
    fn rejects_empty_or_invalid_drafts() {
        let (db, client) = db_with_client();
        let mut inv = draft(client, d(2026, 10, 2));
        inv.due_date = d(2026, 9, 1);
        assert!(db.create_draft(&inv).is_err());
        let mut inv = draft(client, d(2026, 10, 2));
        inv.lines.clear();
        let id = db.create_draft(&inv).unwrap();
        assert!(db.issue_invoice(id, now(), |_| Ok(())).is_err());
    }

    fn approved_reimbursement(db: &Db, client: i64) -> (i64, LineItem) {
        let mut e = super::super::tax::tests::expense(2026, 10, 1, 120_000);
        e.rebill_client_id = Some(client);
        e.approved_by = "Jo Smith".into();
        e.approved_on = Some(d(2026, 9, 28));
        let id = db.save_expense(&e).unwrap();
        let line = LineItem {
            expense_id: Some(id),
            detail: e.invoice_detail(),
            ..LineItem::new(LineKind::Fixed, "Laptop", Quantity::ONE, e.amount)
        };
        (id, line)
    }

    #[test]
    fn reimbursements_bill_once_at_their_amount_for_their_client() {
        let (db, acme) = db_with_client();
        let other = add_client(&db, "Logitech", "LGT");
        let (expense, line) = approved_reimbursement(&db, acme);
        let mut inv = draft(acme, d(2026, 10, 2));
        inv.lines.push(line.clone());

        let mut wrong = inv.clone();
        wrong.client_id = other;
        let err = db.create_draft(&wrong).unwrap_err();
        assert!(err.to_string().contains("different client"), "{err}");
        let mut wrong = inv.clone();
        wrong.lines[2].unit_price = Money::from_pence(1);
        assert!(db.create_draft(&wrong).is_err());

        let id = db.create_draft(&inv).unwrap();
        let saved = db.invoice(id).unwrap();
        assert_eq!(saved.lines[2], line);
        // Re-saving the same draft is fine; billing it on a second draft isn't.
        db.update_draft(&saved).unwrap();
        let err = db.create_draft(&inv).unwrap_err();
        assert!(
            err.to_string()
                .contains(&format!("already billed on draft #{id}")),
            "{err}"
        );
        // While on a draft, the expense can't be deleted or have its amount changed.
        assert!(db.delete_expense(expense).is_err());
        let mut e = db.expense(expense).unwrap();
        e.amount = Money::from_pence(1);
        assert!(
            db.save_expense(&e)
                .unwrap_err()
                .to_string()
                .contains("Remove it from the draft")
        );
        // Remove it from the draft, and it's free again.
        let mut saved = saved;
        saved.lines.pop();
        db.update_draft(&saved).unwrap();
        db.save_expense(&e).unwrap();
        assert_eq!(db.unbilled_reimbursements(acme, 0).unwrap().len(), 1);
    }

    #[test]
    fn issuing_rechecks_and_refreshes_reimbursements() {
        let (db, acme) = db_with_client();
        let (expense, line) = approved_reimbursement(&db, acme);
        let mut inv = draft(acme, d(2026, 10, 2));
        inv.lines.push(line.clone());
        let mut twice = inv.clone();
        twice.lines.push(line);
        let err = db.create_draft(&twice).unwrap_err();
        assert!(err.to_string().contains("on this invoice twice"), "{err}");

        let id = db.create_draft(&inv).unwrap();
        // Approval withdrawn after the draft was saved: issuing is refused.
        let mut e = db.expense(expense).unwrap();
        let approval = (e.approved_by.clone(), e.approved_on);
        e.approved_by.clear();
        e.approved_on = None;
        db.save_expense(&e).unwrap();
        let err = db.issue_invoice(id, now(), |_| Ok(())).unwrap_err();
        assert!(err.to_string().contains("hasn't been approved"), "{err}");
        // Re-approved with a supplier reference: the issued line shows it.
        (e.approved_by, e.approved_on) = approval;
        e.receipt_ref = "INV-9".into();
        db.save_expense(&e).unwrap();
        let issued = db.issue_invoice(id, now(), |_| Ok(())).unwrap();
        assert!(issued.lines[2].detail.ends_with("Supplier invoice INV-9"));
        assert_eq!(
            db.invoice(id).unwrap().lines[2].detail,
            issued.lines[2].detail
        );
    }

    #[test]
    fn issued_details_update_only_on_the_record() {
        let (db, acme) = db_with_client();
        let id = db.create_draft(&draft(acme, d(2026, 10, 2))).unwrap();
        let issued = db.issue_invoice(id, now(), |_| Ok(())).unwrap();
        let mut b = db.require_business().unwrap();
        b.bank.iban = "GB29NWBK60161331926819".into();
        db.save_business(&b).unwrap();
        let mut c = db.client(acme).unwrap();
        c.email = "ap@acme.example".into();
        c.label = "ACM".into(); // first label, set after issue
        db.update_client(&c).unwrap();

        let current = Snapshot {
            business: db.require_business().unwrap(),
            client: db.client(acme).unwrap(),
        };
        assert_eq!(
            issued.snapshot.as_ref().unwrap().printed_changes(&current),
            ["your bank details", "the client's email"]
        );

        let updated = db.update_issued_details(id, d(2026, 10, 25)).unwrap();
        let reloaded = db.invoice(id).unwrap();
        assert_eq!(reloaded, updated);
        assert_eq!(reloaded.details_updated_on, Some(d(2026, 10, 25)));
        let snap = reloaded.snapshot.unwrap();
        assert_eq!(snap.business.bank.iban, "GB29NWBK60161331926819");
        assert_eq!(snap.client.email, "ap@acme.example");
        assert_eq!(
            snap.client.label, "",
            "the label it was numbered with is kept"
        );
        assert_eq!(reloaded.number, issued.number);
        assert_eq!(reloaded.lines, issued.lines);

        // Raw SQL still can't change the details without recording it, or anything else.
        let conn = db.connection();
        assert!(
            conn.execute("UPDATE invoices SET snapshot = '{}' WHERE id = ?1", [id])
                .is_err()
        );
        assert!(
            conn.execute("UPDATE invoices SET notes = 'x' WHERE id = ?1", [id])
                .is_err()
        );
        assert!(
            conn.execute(
                "UPDATE invoices SET details_updated_on = NULL, snapshot = snapshot WHERE id = ?1",
                [id]
            )
            .is_err()
        );
        assert!(
            db.update_issued_details(
                db.create_draft(&draft(acme, d(2026, 10, 3))).unwrap(),
                d(2026, 10, 25)
            )
            .is_err()
        );
    }

    #[test]
    fn previewing_changes_nothing() {
        let (db, acme) = db_with_client();
        let first = db.create_draft(&draft(acme, d(2026, 10, 2))).unwrap();
        db.issue_invoice(first, now(), |_| Ok(())).unwrap();
        let id = db.create_draft(&draft(acme, d(2026, 10, 3))).unwrap();
        let before = db.invoice(id).unwrap();
        let sequences = || -> Vec<(String, String, i64)> {
            let mut stmt = db
                .connection()
                .prepare("SELECT * FROM sequences ORDER BY 1, 2")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        let seq_before = sequences();

        let p = db.preview_invoice(&before).unwrap();
        assert_eq!(p.number.as_deref(), Ok("2026-0002"));
        assert_eq!(
            db.preview_invoice(&before).unwrap().number.as_deref(),
            Ok("2026-0002")
        );
        assert_eq!(p.invoice.snapshot.as_ref().unwrap().client.name, "Acme Ltd");
        assert!(p.invoice.number.is_none());
        assert_eq!(db.invoice(id).unwrap(), before);
        assert_eq!(sequences(), seq_before);
        // Issuing afterwards gets the number the preview showed.
        let issued = db.issue_invoice(id, now(), |_| Ok(())).unwrap();
        assert_eq!(issued.number.as_deref(), Some("2026-0002"));
        assert!(db.preview_invoice(&issued).is_err());
    }

    #[test]
    fn preview_explains_a_missing_number() {
        let (db, acme) = db_with_client();
        use_label_pattern(&db);
        let inv = draft(acme, d(2026, 10, 2));
        let err = db.preview_invoice(&inv).unwrap().number.unwrap_err();
        assert!(err.contains("Acme Ltd has no label"), "{err}");
    }

    #[test]
    fn picker_includes_the_drafts_own_reimbursements() {
        let (db, acme) = db_with_client();
        let (_, line) = approved_reimbursement(&db, acme);
        let mut inv = draft(acme, d(2026, 10, 2));
        inv.lines.push(line);
        let id = db.create_draft(&inv).unwrap();
        assert!(db.unbilled_reimbursements(acme, 0).unwrap().is_empty());
        assert_eq!(db.unbilled_reimbursements(acme, id).unwrap().len(), 1);
    }

    #[test]
    fn duplicate_creates_a_fresh_draft() {
        let (db, client) = db_with_client();
        let a = db.create_draft(&draft(client, d(2026, 1, 5))).unwrap();
        db.issue_invoice(a, now(), |_| Ok(())).unwrap();
        let copy = db.duplicate_invoice(a, d(2026, 10, 2)).unwrap();
        let copy = db.invoice(copy).unwrap();
        assert!(copy.is_draft());
        assert_eq!(copy.issue_date, d(2026, 10, 2));
        assert_eq!(copy.due_date, d(2026, 11, 1));
        assert_eq!(copy.lines, db.invoice(a).unwrap().lines);
    }
}
