//! Payments, refunds and credit notes — everything that changes what's owed
//! on an issued invoice.

use anyhow::{Result, bail};
use chrono::NaiveDate;
use rusqlite::{Row, params};

use super::invoices::{insert_lines, number_taken};
use super::{Db, json_column, line_from_row};
use crate::domain::{
    CreditNote, Invoice, LineItem, Money, Snapshot, Totals, Transaction, lines_total,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxKind {
    Payment,
    Refund,
}

impl TxKind {
    fn table(self) -> &'static str {
        match self {
            TxKind::Payment => "payments",
            TxKind::Refund => "refunds",
        }
    }
}

impl Db {
    pub fn transactions(&self, kind: TxKind, invoice_id: Option<i64>) -> Result<Vec<Transaction>> {
        let sql = format!(
            "SELECT * FROM {} WHERE ?1 IS NULL OR invoice_id = ?1 ORDER BY date, id",
            kind.table()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([invoice_id], tx_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Records money received. Overpayments are allowed (the balance goes
    /// negative and can be settled with a refund).
    pub fn add_payment(&self, t: &Transaction) -> Result<i64> {
        if !t.amount.is_positive() {
            bail!("the payment amount must be more than £0.00");
        }
        self.insert_tx(TxKind::Payment, t)
    }

    /// Records money paid back to the client. Only allowed up to the amount
    /// currently owed back (a negative balance).
    pub fn add_refund(&self, t: &Transaction) -> Result<i64> {
        if !t.amount.is_positive() {
            bail!("the refund amount must be more than £0.00");
        }
        let owed_back = -self.totals(t.invoice_id)?.balance();
        if t.amount > owed_back {
            bail!(
                "nothing like that is owed back on this invoice (refundable: {})",
                owed_back.max(Money::ZERO)
            );
        }
        self.insert_tx(TxKind::Refund, t)
    }

    fn insert_tx(&self, kind: TxKind, t: &Transaction) -> Result<i64> {
        let sql = format!(
            "INSERT INTO {} (invoice_id, date, amount_pence, method, reference) VALUES (?1, ?2, ?3, ?4, ?5)",
            kind.table()
        );
        self.conn.execute(
            &sql,
            params![
                t.invoice_id,
                t.date,
                t.amount.pence(),
                t.method.trim(),
                t.reference.trim()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Deletes a payment or refund entered by mistake.
    pub fn delete_transaction(&self, kind: TxKind, id: i64) -> Result<()> {
        let sql = format!("DELETE FROM {} WHERE id = ?1", kind.table());
        self.conn.execute(&sql, [id])?;
        Ok(())
    }

    pub fn credit_notes(&self, invoice_id: Option<i64>) -> Result<Vec<CreditNote>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM credit_notes WHERE ?1 IS NULL OR invoice_id = ?1 ORDER BY date, id",
        )?;
        let mut notes: Vec<CreditNote> = stmt
            .query_map([invoice_id], credit_note_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        let mut lines = self.conn.prepare_cached(
            "SELECT * FROM credit_note_lines WHERE credit_note_id = ?1 ORDER BY position",
        )?;
        for n in &mut notes {
            n.lines = lines
                .query_map([n.id], line_from_row)?
                .collect::<rusqlite::Result<_>>()?;
        }
        Ok(notes)
    }

    /// Raises a credit note against an issued invoice. The total may not exceed
    /// what's left uncredited. `before_commit` runs inside the transaction
    /// (used to write the PDF).
    pub fn create_credit_note(
        &self,
        invoice_id: i64,
        date: NaiveDate,
        reason: &str,
        lines: Vec<LineItem>,
        before_commit: impl FnOnce(&CreditNote, &Invoice) -> Result<()>,
    ) -> Result<CreditNote> {
        let invoice = self.invoice(invoice_id)?;
        if invoice.is_draft() {
            bail!("credit notes can only be raised against issued invoices");
        }
        if reason.trim().is_empty() {
            bail!("a credit note needs a reason");
        }
        if lines.is_empty() || lines.iter().any(|l| l.description.trim().is_empty()) {
            bail!("a credit note needs at least one described line");
        }
        let total = lines_total(&lines);
        if !total.is_positive() {
            bail!("the credit note total must be more than £0.00");
        }
        let creditable = self.totals(invoice_id)?.creditable();
        if total > creditable {
            bail!("only {creditable} of this invoice is left to credit");
        }
        if date < invoice.issue_date {
            bail!("a credit note can't be dated before its invoice");
        }
        let business = self.require_business()?;
        let snapshot = Snapshot {
            business: business.clone(),
            client: self.client(invoice.client_id)?,
        };

        let tx = self.conn.unchecked_transaction()?;
        let number = Self::next_number(
            &tx,
            "credit_note",
            &business.credit_note_pattern,
            date,
            &snapshot.client,
            |c, n| number_taken(c, "credit_notes", n),
        )?;
        tx.execute(
            "INSERT INTO credit_notes (number, invoice_id, date, reason, snapshot) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![number, invoice_id, date, reason.trim(), serde_json::to_string(&snapshot)?],
        )?;
        let id = tx.last_insert_rowid();
        insert_lines(&tx, "credit_note_lines", "credit_note_id", id, &lines)?;
        let note = CreditNote {
            id,
            number,
            invoice_id,
            date,
            reason: reason.trim().into(),
            lines,
            snapshot,
        };
        before_commit(&note, &invoice)?;
        tx.commit()?;
        Ok(note)
    }

    pub fn totals(&self, invoice_id: i64) -> Result<Totals> {
        Ok(self.conn.query_row(
            "SELECT
                (SELECT COALESCE(SUM((quantity_centi * unit_price_pence + CASE WHEN quantity_centi * unit_price_pence >= 0 THEN 50 ELSE -50 END) / 100), 0)
                   FROM line_items WHERE invoice_id = ?1),
                (SELECT COALESCE(SUM(amount_pence), 0) FROM payments WHERE invoice_id = ?1),
                (SELECT COALESCE(SUM((l.quantity_centi * l.unit_price_pence + CASE WHEN l.quantity_centi * l.unit_price_pence >= 0 THEN 50 ELSE -50 END) / 100), 0)
                   FROM credit_note_lines l JOIN credit_notes n ON n.id = l.credit_note_id
                   WHERE n.invoice_id = ?1),
                (SELECT COALESCE(SUM(amount_pence), 0) FROM refunds WHERE invoice_id = ?1)",
            [invoice_id],
            |r| {
                Ok(Totals {
                    total: Money::from_pence(r.get(0)?),
                    paid: Money::from_pence(r.get(1)?),
                    credited: Money::from_pence(r.get(2)?),
                    refunded: Money::from_pence(r.get(3)?),
                })
            },
        )?)
    }
}

fn tx_from_row(r: &Row) -> rusqlite::Result<Transaction> {
    Ok(Transaction {
        id: r.get("id")?,
        invoice_id: r.get("invoice_id")?,
        date: r.get("date")?,
        amount: Money::from_pence(r.get("amount_pence")?),
        method: r.get("method")?,
        reference: r.get("reference")?,
    })
}

fn credit_note_from_row(r: &Row) -> rusqlite::Result<CreditNote> {
    Ok(CreditNote {
        id: r.get("id")?,
        number: r.get("number")?,
        invoice_id: r.get("invoice_id")?,
        date: r.get("date")?,
        reason: r.get("reason")?,
        lines: Vec::new(),
        snapshot: json_column::<Snapshot>(r, "snapshot")?.expect("snapshot is NOT NULL"),
    })
}

#[cfg(test)]
mod tests {
    use super::super::invoices::tests::{d, draft, now};
    use super::super::test_support::*;
    use super::*;
    use crate::domain::{LineKind, Quantity, Status};

    fn payment(invoice_id: i64, pence: i64) -> Transaction {
        Transaction {
            id: 0,
            invoice_id,
            date: d(2026, 10, 10),
            amount: Money::from_pence(pence),
            method: "Bank transfer".into(),
            reference: String::new(),
        }
    }

    fn credit_line(pence: i64) -> Vec<LineItem> {
        vec![LineItem {
            kind: LineKind::Fixed,
            description: "Credit".into(),
            quantity: Quantity::ONE,
            unit_price: Money::from_pence(pence),
        }]
    }

    fn issued(db: &Db, client: i64) -> Invoice {
        let id = db.create_draft(&draft(client, d(2026, 10, 2))).unwrap();
        db.issue_invoice(id, now(), |_| Ok(())).unwrap()
    }

    #[test]
    fn sql_totals_match_rust_rounding() {
        let (db, client) = db_with_client();
        let mut inv = draft(client, d(2026, 10, 2));
        inv.lines[0].quantity = Quantity::from_hundredths(33);
        inv.lines[0].unit_price = Money::from_pence(50); // 16.5p → 17p
        let id = db.create_draft(&inv).unwrap();
        assert_eq!(
            db.totals(id).unwrap().total,
            db.invoice(id).unwrap().total()
        );
    }

    #[test]
    fn payments_need_an_issued_invoice() {
        let (db, client) = db_with_client();
        let id = db.create_draft(&draft(client, d(2026, 10, 2))).unwrap();
        assert!(db.add_payment(&payment(id, 100)).is_err());
    }

    #[test]
    fn partial_payments_then_paid() {
        let (db, client) = db_with_client();
        let inv = issued(&db, client);
        db.add_payment(&payment(inv.id, 20_000)).unwrap();
        let t = db.totals(inv.id).unwrap();
        assert_eq!(t.status(&inv, d(2026, 10, 15)), Status::PartPaid);
        db.add_payment(&payment(inv.id, 40_000)).unwrap();
        let t = db.totals(inv.id).unwrap();
        assert_eq!(t.balance(), Money::ZERO);
        assert_eq!(t.status(&inv, d(2026, 12, 1)), Status::Paid);
        assert_eq!(
            db.transactions(TxKind::Payment, Some(inv.id))
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn credit_note_after_payment_requires_refund() {
        let (db, client) = db_with_client();
        let inv = issued(&db, client);
        db.add_payment(&payment(inv.id, 60_000)).unwrap();
        // Nothing owed back yet.
        assert!(db.add_refund(&payment(inv.id, 1)).is_err());
        let cn = db
            .create_credit_note(
                inv.id,
                d(2026, 10, 20),
                "Hosting not needed",
                credit_line(15_000),
                |_, _| Ok(()),
            )
            .unwrap();
        assert_eq!(cn.number, "CN-2026-001");
        assert_eq!(
            db.totals(inv.id).unwrap().balance(),
            Money::from_pence(-15_000)
        );
        assert!(db.add_refund(&payment(inv.id, 15_001)).is_err());
        db.add_refund(&payment(inv.id, 15_000)).unwrap();
        let t = db.totals(inv.id).unwrap();
        assert_eq!(t.balance(), Money::ZERO);
        assert_eq!(t.net_received(), Money::from_pence(45_000));
    }

    #[test]
    fn credit_notes_cannot_exceed_the_invoice() {
        let (db, client) = db_with_client();
        let inv = issued(&db, client);
        db.create_credit_note(
            inv.id,
            d(2026, 10, 20),
            "Part",
            credit_line(50_000),
            |_, _| Ok(()),
        )
        .unwrap();
        assert!(
            db.create_credit_note(
                inv.id,
                d(2026, 10, 20),
                "Too much",
                credit_line(10_001),
                |_, _| Ok(())
            )
            .is_err()
        );
        let cn = db
            .create_credit_note(
                inv.id,
                d(2026, 10, 21),
                "Rest",
                credit_line(10_000),
                |_, _| Ok(()),
            )
            .unwrap();
        assert_eq!(cn.number, "CN-2026-002");
        assert_eq!(
            db.totals(inv.id).unwrap().status(&inv, d(2027, 1, 1)),
            Status::Credited
        );
        assert_eq!(db.credit_notes(Some(inv.id)).unwrap().len(), 2);
    }

    #[test]
    fn failed_credit_note_rolls_back() {
        let (db, client) = db_with_client();
        let inv = issued(&db, client);
        assert!(
            db.create_credit_note(
                inv.id,
                d(2026, 10, 20),
                "x",
                credit_line(100),
                |_, _| anyhow::bail!("pdf")
            )
            .is_err()
        );
        assert!(db.credit_notes(None).unwrap().is_empty());
    }
}
