//! Read-only views over the ledger: invoice summaries, aging, tax-year
//! income (cash basis) and CSV export.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{Datelike, Local, NaiveDate};

use crate::db::{Db, TxKind};
use crate::domain::dates::{self, TaxYear};
use crate::domain::{Invoice, Money, Status, Totals, Transaction};
use crate::paths;

/// An invoice with everything needed to list it.
#[derive(Debug, Clone)]
pub struct InvoiceRow {
    pub invoice: Invoice,
    pub client_name: String,
    pub totals: Totals,
    pub status: Status,
}

impl InvoiceRow {
    pub fn balance(&self) -> Money {
        self.totals.balance()
    }

    /// Days past the due date (0 if not yet due).
    pub fn days_overdue(&self, today: NaiveDate) -> i64 {
        (today - self.invoice.due_date).num_days().max(0)
    }
}

pub fn invoice_rows(db: &Db, today: NaiveDate) -> Result<Vec<InvoiceRow>> {
    let names: HashMap<i64, String> = db
        .clients(true)?
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect();
    db.invoices()?
        .into_iter()
        .map(|invoice| {
            let totals = db.totals(invoice.id)?;
            let status = totals.status(&invoice, today);
            // Issued invoices show the client name as it was on the invoice.
            let client_name = invoice
                .snapshot
                .as_ref()
                .map(|s| s.client.name.clone())
                .or_else(|| names.get(&invoice.client_id).cloned())
                .unwrap_or_else(|| "?".into());
            Ok(InvoiceRow {
                invoice,
                client_name,
                totals,
                status,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgingBucket {
    NotYetDue,
    Days1To30,
    Days31To60,
    Over60,
}

impl AgingBucket {
    pub const ALL: [AgingBucket; 4] = [
        AgingBucket::NotYetDue,
        AgingBucket::Days1To30,
        AgingBucket::Days31To60,
        AgingBucket::Over60,
    ];

    pub fn of(days_overdue: i64) -> Self {
        match days_overdue {
            ..=0 => AgingBucket::NotYetDue,
            1..=30 => AgingBucket::Days1To30,
            31..=60 => AgingBucket::Days31To60,
            _ => AgingBucket::Over60,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AgingBucket::NotYetDue => "Not yet due",
            AgingBucket::Days1To30 => "1–30 days overdue",
            AgingBucket::Days31To60 => "31–60 days overdue",
            AgingBucket::Over60 => "60+ days overdue",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgingReport {
    /// (bucket, total owed, invoice count)
    pub buckets: Vec<(AgingBucket, Money, usize)>,
    /// Outstanding invoices, most overdue first.
    pub rows: Vec<InvoiceRow>,
    pub total: Money,
}

pub fn aging(rows: &[InvoiceRow], today: NaiveDate) -> AgingReport {
    let mut outstanding: Vec<InvoiceRow> = rows
        .iter()
        .filter(|r| r.status.is_outstanding())
        .cloned()
        .collect();
    outstanding.sort_by_key(|r| r.invoice.due_date);
    let buckets = AgingBucket::ALL
        .into_iter()
        .map(|b| {
            let in_bucket = outstanding
                .iter()
                .filter(|r| AgingBucket::of(r.days_overdue(today)) == b);
            let (sum, count) =
                in_bucket.fold((Money::ZERO, 0), |(s, c), r| (s + r.balance(), c + 1));
            (b, sum, count)
        })
        .collect();
    let total = outstanding.iter().map(InvoiceRow::balance).sum();
    AgingReport {
        buckets,
        rows: outstanding,
        total,
    }
}

/// Cash-basis income for one tax year: money received minus refunds paid,
/// by the date the money actually moved.
#[derive(Debug, Clone)]
pub struct IncomeReport {
    pub year: TaxYear,
    pub received: Money,
    pub refunded: Money,
    /// (month label, received, refunded), April to March.
    pub months: Vec<(String, Money, Money)>,
}

impl IncomeReport {
    pub fn net(&self) -> Money {
        self.received - self.refunded
    }
}

pub fn income(db: &Db, year: TaxYear) -> Result<IncomeReport> {
    let payments = db.transactions(TxKind::Payment, None)?;
    let refunds = db.transactions(TxKind::Refund, None)?;
    Ok(income_from(year, &payments, &refunds))
}

fn income_from(year: TaxYear, payments: &[Transaction], refunds: &[Transaction]) -> IncomeReport {
    // Calendar months clipped to the tax year, so the first row is
    // 6–30 April and the last is 1–5 April.
    let mut months: Vec<(String, Money, Money)> = Vec::new();
    let mut month_start = year.start();
    while month_start <= year.end() {
        let next = first_of_next_month(month_start);
        let end = (next.pred_opt().expect("valid date")).min(year.end());
        let in_range = |t: &&Transaction| t.date >= month_start && t.date <= end;
        let received = payments.iter().filter(in_range).map(|t| t.amount).sum();
        let refunded = refunds.iter().filter(in_range).map(|t| t.amount).sum();
        let label = if month_start.day() == 1 && end.day() < 28 {
            format!("{} (1–{})", month_start.format("%b %Y"), end.day())
        } else if month_start.day() != 1 {
            format!(
                "{} ({}–{})",
                month_start.format("%b %Y"),
                month_start.day(),
                end.day()
            )
        } else {
            month_start.format("%b %Y").to_string()
        };
        months.push((label, received, refunded));
        month_start = next;
    }
    let in_year = |t: &&Transaction| year.contains(t.date);
    IncomeReport {
        year,
        received: payments.iter().filter(in_year).map(|t| t.amount).sum(),
        refunded: refunds.iter().filter(in_year).map(|t| t.amount).sum(),
        months,
    }
}

fn first_of_next_month(d: NaiveDate) -> NaiveDate {
    let (y, m) = if d.month() == 12 {
        (d.year() + 1, 1)
    } else {
        (d.year(), d.month() + 1)
    };
    NaiveDate::from_ymd_opt(y, m, 1).expect("valid date")
}

/// Writes invoices.csv, payments.csv, refunds.csv and credit_notes.csv into
/// `<output>/exports/<timestamp>/`. Amounts are plain decimals (e.g. 1250.00)
/// so spreadsheets treat them as numbers.
pub fn export_csv(db: &Db, output_dir: &Path, today: NaiveDate) -> Result<PathBuf> {
    let dir = paths::exports_dir(output_dir).join(Local::now().format("%Y%m%d-%H%M%S").to_string());
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let rows = invoice_rows(db, today)?;
    let numbers: HashMap<i64, String> = rows
        .iter()
        .map(|r| (r.invoice.id, r.invoice.display_number()))
        .collect();

    let mut w = csv::Writer::from_path(dir.join("invoices.csv"))?;
    w.write_record([
        "number",
        "status",
        "client",
        "issue_date",
        "supply_start",
        "supply_end",
        "due_date",
        "sent_on",
        "total",
        "paid",
        "credited",
        "refunded",
        "balance",
    ])?;
    for r in &rows {
        let i = &r.invoice;
        w.write_record([
            i.display_number(),
            r.status.label().to_string(),
            r.client_name.clone(),
            i.issue_date.to_string(),
            i.supply_start.to_string(),
            i.supply_end.map(|d| d.to_string()).unwrap_or_default(),
            i.due_date.to_string(),
            i.sent_on.map(|d| d.to_string()).unwrap_or_default(),
            r.totals.total.to_decimal_string(),
            r.totals.paid.to_decimal_string(),
            r.totals.credited.to_decimal_string(),
            r.totals.refunded.to_decimal_string(),
            r.balance().to_decimal_string(),
        ])?;
    }
    w.flush()?;

    for (kind, file) in [
        (TxKind::Payment, "payments.csv"),
        (TxKind::Refund, "refunds.csv"),
    ] {
        let mut w = csv::Writer::from_path(dir.join(file))?;
        w.write_record([
            "date",
            "tax_year",
            "invoice",
            "amount",
            "method",
            "reference",
        ])?;
        for t in db.transactions(kind, None)? {
            w.write_record([
                t.date.to_string(),
                TaxYear::containing(t.date).label(),
                numbers.get(&t.invoice_id).cloned().unwrap_or_default(),
                t.amount.to_decimal_string(),
                t.method,
                t.reference,
            ])?;
        }
        w.flush()?;
    }

    let mut w = csv::Writer::from_path(dir.join("credit_notes.csv"))?;
    w.write_record(["number", "date", "invoice", "client", "amount", "reason"])?;
    for n in db.credit_notes(None)? {
        w.write_record([
            n.number.clone(),
            n.date.to_string(),
            numbers.get(&n.invoice_id).cloned().unwrap_or_default(),
            n.snapshot.client.name.clone(),
            n.total().to_decimal_string(),
            n.reason.clone(),
        ])?;
    }
    w.flush()?;
    Ok(dir)
}

/// Headline numbers for the dashboard.
#[derive(Debug, Clone, Default)]
pub struct Overview {
    pub outstanding: Money,
    pub outstanding_count: usize,
    pub overdue: Money,
    pub overdue_count: usize,
    pub drafts: usize,
    pub refunds_owed: Money,
}

pub fn overview(rows: &[InvoiceRow]) -> Overview {
    let mut o = Overview::default();
    for r in rows {
        if r.status.is_outstanding() {
            o.outstanding += r.balance();
            o.outstanding_count += 1;
        }
        if r.status == Status::Overdue {
            o.overdue += r.balance();
            o.overdue_count += 1;
        }
        if r.status == Status::Draft {
            o.drafts += 1;
        }
        if !r.invoice.is_draft() && r.balance().is_negative() {
            o.refunds_owed -= r.balance();
        }
    }
    o
}

pub fn current_tax_year() -> TaxYear {
    TaxYear::containing(dates::today())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(y: i32, m: u32, d: u32, pence: i64) -> Transaction {
        Transaction {
            id: 0,
            invoice_id: 1,
            date: NaiveDate::from_ymd_opt(y, m, d).unwrap(),
            amount: Money::from_pence(pence),
            method: String::new(),
            reference: String::new(),
        }
    }

    #[test]
    fn income_respects_tax_year_boundaries() {
        let payments = [
            tx(2026, 4, 5, 100),
            tx(2026, 4, 6, 1_000),
            tx(2027, 4, 5, 2_000),
            tx(2027, 4, 6, 5),
        ];
        let refunds = [tx(2026, 12, 1, 300)];
        let r = income_from(TaxYear(2026), &payments, &refunds);
        assert_eq!(r.received, Money::from_pence(3_000));
        assert_eq!(r.refunded, Money::from_pence(300));
        assert_eq!(r.net(), Money::from_pence(2_700));
        // 6–30 Apr, May … Mar, 1–5 Apr = 13 rows, summing to the year total.
        assert_eq!(r.months.len(), 13);
        assert_eq!(r.months.first().unwrap().0, "Apr 2026 (6–30)");
        assert_eq!(r.months.last().unwrap().0, "Apr 2027 (1–5)");
        let summed: Money = r.months.iter().map(|m| m.1).sum();
        assert_eq!(summed, r.received);
    }

    #[test]
    fn aging_buckets() {
        assert_eq!(AgingBucket::of(0), AgingBucket::NotYetDue);
        assert_eq!(AgingBucket::of(1), AgingBucket::Days1To30);
        assert_eq!(AgingBucket::of(30), AgingBucket::Days1To30);
        assert_eq!(AgingBucket::of(31), AgingBucket::Days31To60);
        assert_eq!(AgingBucket::of(61), AgingBucket::Over60);
    }
}
