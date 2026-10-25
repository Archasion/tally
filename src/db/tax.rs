//! Expenses, tax settings and entered prior-year bills.

use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, Row, params};

use super::Db;
use crate::domain::Money;
use crate::domain::dates::TaxYear;
use crate::domain::expense::{Expense, ExpenseCategory};
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

    /// Inserts (id 0) or updates an expense; returns its id.
    pub fn save_expense(&self, e: &Expense) -> Result<i64> {
        if e.description.trim().is_empty() {
            bail!("An expense needs a description");
        }
        if !e.amount.is_positive() {
            bail!("The amount must be more than £0.00");
        }
        let args = params![
            e.id,
            e.date,
            e.description.trim(),
            e.category.as_str(),
            e.amount.pence(),
            e.notes.trim()
        ];
        if e.id == 0 {
            self.conn.execute(
                "INSERT INTO expenses (id, date, description, category, amount_pence, notes)
                 VALUES (NULL, ?2, ?3, ?4, ?5, ?6)",
                args,
            )?;
            Ok(self.conn.last_insert_rowid())
        } else {
            let changed = self.conn.execute(
                "UPDATE expenses SET date = ?2, description = ?3, category = ?4, amount_pence = ?5, notes = ?6
                 WHERE id = ?1",
                args,
            )?;
            if changed == 0 {
                bail!("expense {} not found", e.id);
            }
            Ok(e.id)
        }
    }

    pub fn delete_expense(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM expenses WHERE id = ?1", [id])?;
        Ok(())
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

fn expense_from_row(r: &Row) -> rusqlite::Result<Expense> {
    let category: String = r.get("category")?;
    Ok(Expense {
        id: r.get("id")?,
        date: r.get("date")?,
        description: r.get("description")?,
        category: ExpenseCategory::parse(&category).unwrap_or(ExpenseCategory::Other),
        amount: Money::from_pence(r.get("amount_pence")?),
        notes: r.get("notes")?,
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;
    use chrono::NaiveDate;

    fn expense(y: i32, m: u32, d: u32, pence: i64) -> Expense {
        Expense {
            id: 0,
            date: NaiveDate::from_ymd_opt(y, m, d).unwrap(),
            description: "Laptop".into(),
            category: ExpenseCategory::Other,
            amount: Money::from_pence(pence),
            notes: String::new(),
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
