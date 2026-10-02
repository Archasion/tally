use anyhow::{Context, Result, bail};
use chrono::{Days, NaiveDate, NaiveDateTime};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Db, json_column, line_from_row};
use crate::domain::{Invoice, LineItem, Snapshot};

impl Db {
    /// All invoices, newest first, with their lines.
    pub fn invoices(&self) -> Result<Vec<Invoice>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM invoices
             ORDER BY issued_at IS NOT NULL, issue_date DESC, number DESC, id DESC",
        )?;
        let mut invoices: Vec<Invoice> =
            stmt.query_map([], invoice_from_row)?.collect::<rusqlite::Result<_>>()?;
        for inv in &mut invoices {
            inv.lines = load_lines(&self.conn, inv.id)?;
        }
        Ok(invoices)
    }

    pub fn invoice(&self, id: i64) -> Result<Invoice> {
        let mut inv = self
            .conn
            .query_row("SELECT * FROM invoices WHERE id = ?1", [id], invoice_from_row)
            .optional()?
            .with_context(|| format!("invoice {id} not found"))?;
        inv.lines = load_lines(&self.conn, id)?;
        Ok(inv)
    }

    /// Saves a new draft and returns its id. `inv.id` is ignored.
    pub fn create_draft(&self, inv: &Invoice) -> Result<i64> {
        validate_draft(inv)?;
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
        let tx = self.conn.unchecked_transaction()?;
        let issued: Option<Option<String>> = tx
            .query_row("SELECT issued_at FROM invoices WHERE id = ?1", [inv.id], |r| r.get(0))
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
        self.conn.execute("DELETE FROM invoices WHERE id = ?1", [id])?;
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
        let business = self.require_business()?;
        let client = self.client(inv.client_id)?;
        let snapshot = Snapshot { business: business.clone(), client };

        let tx = self.conn.unchecked_transaction()?;
        let number = Self::next_number(
            &tx,
            "invoice",
            &business.invoice_pattern,
            inv.issue_date,
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

    /// Records (or with `None`, clears) the date an issued invoice was sent.
    pub fn set_sent(&self, id: i64, sent_on: Option<NaiveDate>) -> Result<()> {
        let inv = self.invoice(id)?;
        if inv.is_draft() {
            bail!("issue the invoice before marking it sent");
        }
        self.conn
            .execute("UPDATE invoices SET sent_on = ?2 WHERE id = ?1", params![id, sent_on])?;
        Ok(())
    }

    /// Copies an invoice's client, lines and notes into a new draft dated `today`.
    pub fn duplicate_invoice(&self, id: i64, today: NaiveDate) -> Result<i64> {
        let src = self.invoice(id)?;
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
            lines: src.lines,
        };
        self.create_draft(&copy)
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
        "INSERT INTO {table} ({parent_col}, position, kind, description, quantity_centi, unit_price_pence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
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
            lines: vec![
                LineItem {
                    kind: LineKind::Hours,
                    description: "Development".into(),
                    quantity: Quantity::from_hundredths(750),
                    unit_price: Money::from_pence(6000),
                },
                LineItem {
                    kind: LineKind::Fixed,
                    description: "Hosting setup".into(),
                    quantity: Quantity::ONE,
                    unit_price: Money::from_pence(15000),
                },
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
        assert!(db.connection().execute("DELETE FROM line_items WHERE invoice_id = ?1", [a.id]).is_err());
        assert!(db.connection().execute("UPDATE invoices SET notes = 'x' WHERE id = ?1", [a.id]).is_err());
        assert!(db.connection().execute("DELETE FROM invoices WHERE id = ?1", [a.id]).is_err());
        // ...but marking as sent is allowed.
        db.set_sent(a.id, Some(d(2026, 10, 2))).unwrap();
        assert_eq!(db.invoice(a.id).unwrap().sent_on, Some(d(2026, 10, 2)));
    }

    #[test]
    fn sequence_restarts_each_calendar_year() {
        let (db, client) = db_with_client();
        let a = db.create_draft(&draft(client, d(2026, 12, 31))).unwrap();
        let b = db.create_draft(&draft(client, d(2027, 1, 1))).unwrap();
        assert_eq!(db.issue_invoice(a, now(), |_| Ok(())).unwrap().number.unwrap(), "2026-0001");
        assert_eq!(db.issue_invoice(b, now(), |_| Ok(())).unwrap().number.unwrap(), "2027-0001");
    }

    #[test]
    fn failed_issue_rolls_back_and_does_not_consume_a_number() {
        let (db, client) = db_with_client();
        let a = db.create_draft(&draft(client, d(2026, 10, 2))).unwrap();
        assert!(db.issue_invoice(a, now(), |_| anyhow::bail!("pdf failed")).is_err());
        assert!(db.invoice(a).unwrap().is_draft());
        assert_eq!(db.issue_invoice(a, now(), |_| Ok(())).unwrap().number.unwrap(), "2026-0001");
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
        db.connection().execute("DELETE FROM sequences", []).unwrap();
        let c = db.create_draft(&draft(client, d(2026, 10, 2))).unwrap();
        assert_eq!(db.issue_invoice(c, now(), |_| Ok(())).unwrap().number.unwrap(), "INV-002");
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
