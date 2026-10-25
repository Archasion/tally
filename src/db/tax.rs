//! Expenses (including ones rebilled to clients), tax settings and entered
//! prior-year bills.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::Db;
use crate::domain::Money;
use crate::domain::dates::TaxYear;
use crate::domain::expense::{BillingPeriod, Expense, ExpenseCategory};

/// The invoice an expense has been billed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Billed {
    pub invoice_id: i64,
    /// `None` while the invoice is a draft.
    pub number: Option<String>,
}

impl Billed {
    pub fn display(&self) -> String {
        self.number
            .clone()
            .unwrap_or_else(|| format!("draft #{}", self.invoice_id))
    }
}
use crate::tax::{LoanPlan, TaxSettings};

impl Db {
    /// Expenses paid in a tax year (or all, if `None`), oldest first.
    pub fn expenses(&self, year: Option<TaxYear>) -> Result<Vec<Expense>> {
        let (from, to) = match year {
            Some(y) => (y.start().to_string(), y.end().to_string()),
            None => ("0000-01-01".into(), "9999-12-31".into()),
        };
        let mut stmt = self
            .conn
            .prepare("SELECT * FROM expenses WHERE date BETWEEN ?1 AND ?2 ORDER BY date, id")?;
        let rows = stmt.query_map([from, to], expense_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn expense(&self, id: i64) -> Result<Expense> {
        self.conn
            .query_row(
                "SELECT * FROM expenses WHERE id = ?1",
                [id],
                expense_from_row,
            )
            .with_context(|| format!("expense {id} not found"))
    }

    /// Inserts (id 0) or updates an expense; returns its id. Once it's on an
    /// invoice (even a draft), its amount and rebill client can't change.
    pub fn save_expense(&self, e: &Expense) -> Result<i64> {
        if e.description.trim().is_empty() {
            bail!("An expense needs a description");
        }
        if !e.amount.is_positive() {
            bail!("The amount must be more than £0.00");
        }
        if e.approved_by.trim().is_empty() != e.approved_on.is_none() {
            bail!("Enter both who approved it and when, or neither");
        }
        if e.id != 0
            && let Some(billed) = billed_on(&self.conn, e.id)?
        {
            let old = self.expense(e.id)?;
            if old.amount != e.amount || old.rebill_client_id != e.rebill_client_id {
                bail!(
                    "This is billed on {}, so its amount and client can't change{}",
                    billed.display(),
                    if billed.number.is_none() {
                        ". Remove it from the draft first"
                    } else {
                        ""
                    }
                );
            }
        }
        let args = params![
            e.id,
            e.date,
            e.description.trim(),
            e.category.as_str(),
            e.amount.pence(),
            e.notes.trim(),
            e.billing.as_str(),
            e.rebill_client_id,
            e.approved_by.trim(),
            e.approved_on,
            e.receipt_ref.trim(),
            e.receipt_file,
        ];
        if e.id == 0 {
            self.conn.execute(
                "INSERT INTO expenses (id, date, description, category, amount_pence, notes, billing,
                    rebill_client_id, approved_by, approved_on, receipt_ref, receipt_file)
                 VALUES (NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                args,
            )?;
            Ok(self.conn.last_insert_rowid())
        } else {
            let changed = self.conn.execute(
                "UPDATE expenses SET date = ?2, description = ?3, category = ?4, amount_pence = ?5,
                    notes = ?6, billing = ?7, rebill_client_id = ?8, approved_by = ?9,
                    approved_on = ?10, receipt_ref = ?11, receipt_file = ?12
                 WHERE id = ?1",
                args,
            )?;
            if changed == 0 {
                bail!("expense {} not found", e.id);
            }
            Ok(e.id)
        }
    }

    /// Deletes an expense that isn't on any invoice.
    pub fn delete_expense(&self, id: i64) -> Result<()> {
        if let Some(billed) = billed_on(&self.conn, id)? {
            bail!(
                "This is billed on {}. {}",
                billed.display(),
                if billed.number.is_none() {
                    "Remove it from the draft first"
                } else {
                    "Billed expenses are kept with the invoice"
                }
            );
        }
        self.conn
            .execute("DELETE FROM expenses WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Which invoice each billed expense is on, by expense id.
    pub fn billed_expenses(&self) -> Result<HashMap<i64, Billed>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.expense_id, i.id, i.number FROM line_items l
             JOIN invoices i ON i.id = l.invoice_id WHERE l.expense_id IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                Billed {
                    invoice_id: r.get(1)?,
                    number: r.get(2)?,
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Expenses to rebill to a client that aren't on any invoice yet, oldest
    /// first. Ones on the draft `editing` (0 for none) are included, since
    /// the editor may have removed them from its lines.
    pub fn unbilled_reimbursements(&self, client_id: i64, editing: i64) -> Result<Vec<Expense>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM expenses e WHERE rebill_client_id = ?1
               AND NOT EXISTS (SELECT 1 FROM line_items l
                               WHERE l.expense_id = e.id AND l.invoice_id <> ?2)
             ORDER BY date, id",
        )?;
        let rows = stmt.query_map([client_id, editing], expense_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn tax_settings(&self) -> Result<TaxSettings> {
        Ok(self
            .conn
            .query_row(
                "SELECT student_loan, student_loan_plan, postgraduate_loan, voluntary_class2 FROM business WHERE id = 1",
                [],
                |r| {
                    let plan: String = r.get(1)?;
                    Ok(TaxSettings {
                        student_loan: r.get(0)?,
                        plan: LoanPlan::parse(&plan),
                        postgraduate: r.get(2)?,
                        voluntary_class2: r.get(3)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Saves tax settings. The business row must exist (set up first).
    pub fn save_tax_settings(&self, s: &TaxSettings) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE business SET student_loan = ?1, student_loan_plan = ?2, postgraduate_loan = ?3,
                voluntary_class2 = ?4 WHERE id = 1",
            params![
                s.student_loan,
                s.plan.map(LoanPlan::as_str).unwrap_or(""),
                s.postgraduate,
                s.voluntary_class2
            ],
        )?;
        if changed == 0 {
            bail!("set up your business details first");
        }
        Ok(())
    }

    /// Income tax + Class 4 for a year, as entered from an HMRC statement.
    pub fn entered_bill(&self, year: TaxYear) -> Result<Option<Money>> {
        Ok(self
            .conn
            .query_row(
                "SELECT amount_pence FROM tax_year_bills WHERE year = ?1",
                [year.0],
                |r| r.get(0),
            )
            .optional()?
            .map(Money::from_pence))
    }

    /// Records (or with `None`, clears) the entered bill for a year.
    pub fn set_entered_bill(&self, year: TaxYear, amount: Option<Money>) -> Result<()> {
        match amount {
            Some(m) => self.conn.execute(
                "INSERT INTO tax_year_bills (year, amount_pence) VALUES (?1, ?2)
                 ON CONFLICT (year) DO UPDATE SET amount_pence = excluded.amount_pence",
                params![year.0, m.pence()],
            )?,
            None => self
                .conn
                .execute("DELETE FROM tax_year_bills WHERE year = ?1", [year.0])?,
        };
        Ok(())
    }
}

/// The invoice an expense is billed on, if any.
pub(super) fn billed_on(conn: &Connection, expense_id: i64) -> Result<Option<Billed>> {
    Ok(conn
        .query_row(
            "SELECT i.id, i.number FROM line_items l JOIN invoices i ON i.id = l.invoice_id
             WHERE l.expense_id = ?1",
            [expense_id],
            |r| {
                Ok(Billed {
                    invoice_id: r.get(0)?,
                    number: r.get(1)?,
                })
            },
        )
        .optional()?)
}

fn expense_from_row(r: &Row) -> rusqlite::Result<Expense> {
    let category: String = r.get("category")?;
    let billing: String = r.get("billing")?;
    Ok(Expense {
        id: r.get("id")?,
        date: r.get("date")?,
        description: r.get("description")?,
        category: ExpenseCategory::parse(&category).unwrap_or(ExpenseCategory::Other),
        amount: Money::from_pence(r.get("amount_pence")?),
        notes: r.get("notes")?,
        billing: BillingPeriod::parse(&billing).unwrap_or(BillingPeriod::OneOff),
        rebill_client_id: r.get("rebill_client_id")?,
        approved_by: r.get("approved_by")?,
        approved_on: r.get("approved_on")?,
        receipt_ref: r.get("receipt_ref")?,
        receipt_file: r.get("receipt_file")?,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::test_support::*;
    use super::*;
    use chrono::NaiveDate;

    pub fn expense(y: i32, m: u32, d: u32, pence: i64) -> Expense {
        Expense {
            description: "Laptop".into(),
            amount: Money::from_pence(pence),
            ..Expense::new(NaiveDate::from_ymd_opt(y, m, d).unwrap())
        }
    }

    #[test]
    fn expenses_by_tax_year() {
        let (db, _) = db_with_client();
        db.save_expense(&expense(2026, 4, 5, 100)).unwrap();
        let id = db.save_expense(&expense(2026, 4, 6, 200)).unwrap();
        db.save_expense(&expense(2027, 4, 5, 300)).unwrap();
        let y = db.expenses(Some(TaxYear(2026))).unwrap();
        assert_eq!(
            y.iter().map(|e| e.amount.pence()).collect::<Vec<_>>(),
            [200, 300]
        );
        let mut e = db.expense(id).unwrap();
        e.category = ExpenseCategory::Travel;
        db.save_expense(&e).unwrap();
        assert_eq!(db.expense(id).unwrap().category, ExpenseCategory::Travel);
        db.delete_expense(id).unwrap();
        assert_eq!(db.expenses(None).unwrap().len(), 2);
        assert!(db.save_expense(&expense(2026, 5, 1, 0)).is_err());
    }

    #[test]
    fn reimbursement_fields_round_trip_and_validate() {
        let (db, client) = db_with_client();
        let mut e = expense(2026, 10, 3, 2_490);
        e.billing = BillingPeriod::Monthly;
        e.rebill_client_id = Some(client);
        e.approved_by = "Jo Smith".into();
        assert!(db.save_expense(&e).is_err(), "approval needs a date too");
        e.approved_on = NaiveDate::from_ymd_opt(2026, 10, 1);
        e.receipt_ref = "INV-123".into();
        e.receipt_file = "receipts/2026/2026-10-03_inv.pdf".into();
        let id = db.save_expense(&e).unwrap();
        e.id = id;
        assert_eq!(db.expense(id).unwrap(), e);
        assert_eq!(db.unbilled_reimbursements(client, 0).unwrap(), vec![e]);
    }

    #[test]
    fn tax_settings_round_trip() {
        let (db, _) = db_with_client();
        assert_eq!(db.tax_settings().unwrap(), TaxSettings::default());
        let s = TaxSettings {
            student_loan: true,
            plan: Some(LoanPlan::Plan2),
            postgraduate: true,
            voluntary_class2: true,
        };
        db.save_tax_settings(&s).unwrap();
        assert_eq!(db.tax_settings().unwrap(), s);
        // Saving business details doesn't reset them.
        db.save_business(&business()).unwrap();
        assert_eq!(db.tax_settings().unwrap(), s);
    }

    #[test]
    fn entered_bills() {
        let (db, _) = db_with_client();
        assert_eq!(db.entered_bill(TaxYear(2025)).unwrap(), None);
        db.set_entered_bill(TaxYear(2025), Some(Money::from_pence(300_000)))
            .unwrap();
        db.set_entered_bill(TaxYear(2025), Some(Money::from_pence(310_000)))
            .unwrap();
        assert_eq!(
            db.entered_bill(TaxYear(2025)).unwrap(),
            Some(Money::from_pence(310_000))
        );
        db.set_entered_bill(TaxYear(2025), None).unwrap();
        assert_eq!(db.entered_bill(TaxYear(2025)).unwrap(), None);
    }
}
