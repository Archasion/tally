//! Core records: business, clients, invoices, payments, refunds and credit notes.

use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

use super::money::{Money, Quantity};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    pub line1: String,
    pub line2: String,
    pub town: String,
    /// Postcode or ZIP; formats vary by country.
    pub postcode: String,
    /// Empty for your own business address; clients default to United Kingdom.
    /// (`default` keeps snapshots from before this field existed readable.)
    #[serde(default)]
    pub country: String,
}

impl Address {
    /// Non-empty address lines, in display order.
    pub fn lines(&self) -> Vec<String> {
        [
            &self.line1,
            &self.line2,
            &self.town,
            &self.postcode,
            &self.country,
        ]
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankDetails {
    pub account_name: String,
    pub sort_code: String,
    pub account_number: String,
}

/// Your own details and app settings. There is exactly one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Business {
    pub name: String,
    pub address: Address,
    pub email: String,
    pub phone: String,
    pub bank: BankDetails,
    pub payment_terms_days: u32,
    pub default_notes: String,
    pub invoice_pattern: String,
    pub credit_note_pattern: String,
    /// The (synced) folder for PDFs, backups, exports and template overrides.
    pub output_dir: String,
    pub backups_to_keep: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineKind {
    Hours,
    Days,
    Fixed,
}

impl LineKind {
    pub const ALL: [LineKind; 3] = [LineKind::Hours, LineKind::Days, LineKind::Fixed];

    pub fn as_str(self) -> &'static str {
        match self {
            LineKind::Hours => "hours",
            LineKind::Days => "days",
            LineKind::Fixed => "fixed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str().eq_ignore_ascii_case(s))
    }

    pub fn label(self) -> &'static str {
        match self {
            LineKind::Hours => "Hourly",
            LineKind::Days => "Daily",
            LineKind::Fixed => "Fixed price",
        }
    }

    /// Unit shown after a quantity, e.g. `7.5 hrs`.
    pub fn unit(self, qty: Quantity) -> &'static str {
        let one = qty == Quantity::ONE;
        match (self, one) {
            (LineKind::Hours, true) => "hr",
            (LineKind::Hours, false) => "hrs",
            (LineKind::Days, true) => "day",
            (LineKind::Days, false) => "days",
            (LineKind::Fixed, _) => "",
        }
    }
}

pub const DEFAULT_COUNTRY: &str = "United Kingdom";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client {
    pub id: i64,
    pub name: String,
    pub contact_name: String,
    pub email: String,
    pub address: Address,
    /// Short code used by the `{LABEL}` number token, e.g. `LGT`. Empty = none.
    /// Uppercase A–Z/0–9, unique across clients, locked once used on an issued invoice.
    #[serde(default)]
    pub label: String,
    /// What the client's tax number is called, e.g. `VAT ID`, `ABN`, `EIN`.
    #[serde(default)]
    pub tax_id_label: String,
    #[serde(default)]
    pub tax_id: String,
    /// Business clients can be sent the statutory late-payment notice.
    pub is_business: bool,
    pub late_payment_notice: bool,
    /// Optional default rate pre-filled for new lines.
    pub default_rate: Option<Money>,
    pub default_rate_kind: LineKind,
    pub archived: bool,
}

impl Client {
    pub fn new_empty() -> Self {
        Client {
            id: 0,
            name: String::new(),
            contact_name: String::new(),
            email: String::new(),
            address: Address {
                country: DEFAULT_COUNTRY.into(),
                ..Address::default()
            },
            label: String::new(),
            tax_id_label: String::new(),
            tax_id: String::new(),
            is_business: true,
            late_payment_notice: false,
            default_rate: None,
            default_rate_kind: LineKind::Hours,
            archived: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineItem {
    pub kind: LineKind,
    pub description: String,
    pub quantity: Quantity,
    pub unit_price: Money,
}

impl LineItem {
    pub fn amount(&self) -> Money {
        self.quantity.times(self.unit_price)
    }
}

pub fn lines_total(lines: &[LineItem]) -> Money {
    lines.iter().map(LineItem::amount).sum()
}

/// Business and client details frozen at the moment a document is issued,
/// so the PDF can always be regenerated exactly as sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub business: Business,
    pub client: Client,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invoice {
    pub id: i64,
    /// Assigned when the invoice is issued; drafts have none.
    pub number: Option<String>,
    pub client_id: i64,
    pub issue_date: NaiveDate,
    pub due_date: NaiveDate,
    pub supply_start: NaiveDate,
    /// `None` means the supply date is a single day (`supply_start`).
    pub supply_end: Option<NaiveDate>,
    pub notes: String,
    pub issued_at: Option<NaiveDateTime>,
    pub sent_on: Option<NaiveDate>,
    pub snapshot: Option<Snapshot>,
    pub lines: Vec<LineItem>,
}

impl Invoice {
    pub fn is_draft(&self) -> bool {
        self.issued_at.is_none()
    }

    pub fn total(&self) -> Money {
        lines_total(&self.lines)
    }

    pub fn display_number(&self) -> String {
        self.number
            .clone()
            .unwrap_or_else(|| format!("draft #{}", self.id))
    }
}

/// Money received from a client (`Payment`) or paid back to one (`Refund`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub id: i64,
    pub invoice_id: i64,
    pub date: NaiveDate,
    pub amount: Money,
    pub method: String,
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditNote {
    pub id: i64,
    pub number: String,
    pub invoice_id: i64,
    pub date: NaiveDate,
    pub reason: String,
    pub lines: Vec<LineItem>,
    pub snapshot: Snapshot,
}

impl CreditNote {
    pub fn total(&self) -> Money {
        lines_total(&self.lines)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Draft,
    Issued,
    Sent,
    PartPaid,
    Overdue,
    Paid,
    Credited,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Draft => "Draft",
            Status::Issued => "Issued",
            Status::Sent => "Sent",
            Status::PartPaid => "Part paid",
            Status::Overdue => "Overdue",
            Status::Paid => "Paid",
            Status::Credited => "Credited",
        }
    }

    /// Issued and money still owed.
    pub fn is_outstanding(self) -> bool {
        matches!(
            self,
            Status::Issued | Status::Sent | Status::PartPaid | Status::Overdue
        )
    }
}

/// Money movements against one invoice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub total: Money,
    pub paid: Money,
    pub credited: Money,
    pub refunded: Money,
}

impl Totals {
    /// What the client still owes. Negative means you owe them a refund.
    pub fn balance(&self) -> Money {
        self.total - self.paid - self.credited + self.refunded
    }

    pub fn net_received(&self) -> Money {
        self.paid - self.refunded
    }

    /// Room left for further credit notes.
    pub fn creditable(&self) -> Money {
        self.total - self.credited
    }

    pub fn status(&self, invoice: &Invoice, today: NaiveDate) -> Status {
        if invoice.is_draft() {
            return Status::Draft;
        }
        if self.total.is_positive() && self.credited >= self.total {
            return Status::Credited;
        }
        if !self.balance().is_positive() {
            return Status::Paid;
        }
        if invoice.due_date < today {
            return Status::Overdue;
        }
        if self.net_received().is_positive() {
            return Status::PartPaid;
        }
        if invoice.sent_on.is_some() {
            Status::Sent
        } else {
            Status::Issued
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_from_before_international_fields_still_load() {
        let old = r#"{"id":1,"name":"Acme","contact_name":"","email":"",
            "address":{"line1":"1 A St","line2":"","town":"Bath","postcode":"BA1 1AA"},
            "is_business":true,"late_payment_notice":false,"default_rate":null,
            "default_rate_kind":"Hours","archived":false}"#;
        let c: Client = serde_json::from_str(old).unwrap();
        assert_eq!(c.label, "");
        assert_eq!(c.address.country, "");
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn invoice(issued: bool, sent: bool) -> Invoice {
        Invoice {
            id: 1,
            number: issued.then(|| "2026-0001".into()),
            client_id: 1,
            issue_date: d(2026, 10, 1),
            due_date: d(2026, 10, 31),
            supply_start: d(2026, 10, 1),
            supply_end: None,
            notes: String::new(),
            issued_at: issued.then(|| d(2026, 10, 1).and_hms_opt(9, 0, 0).unwrap()),
            sent_on: sent.then(|| d(2026, 10, 1)),
            snapshot: None,
            lines: vec![],
        }
    }

    fn totals(total: i64, paid: i64, credited: i64, refunded: i64) -> Totals {
        Totals {
            total: Money::from_pence(total),
            paid: Money::from_pence(paid),
            credited: Money::from_pence(credited),
            refunded: Money::from_pence(refunded),
        }
    }

    #[test]
    fn status_progression() {
        let before_due = d(2026, 10, 15);
        let after_due = d(2026, 11, 1);
        assert_eq!(
            totals(1000, 0, 0, 0).status(&invoice(false, false), after_due),
            Status::Draft
        );
        assert_eq!(
            totals(1000, 0, 0, 0).status(&invoice(true, false), before_due),
            Status::Issued
        );
        assert_eq!(
            totals(1000, 0, 0, 0).status(&invoice(true, true), before_due),
            Status::Sent
        );
        assert_eq!(
            totals(1000, 400, 0, 0).status(&invoice(true, true), before_due),
            Status::PartPaid
        );
        assert_eq!(
            totals(1000, 400, 0, 0).status(&invoice(true, true), after_due),
            Status::Overdue
        );
        // Overdue counts from the due date even if never marked sent.
        assert_eq!(
            totals(1000, 0, 0, 0).status(&invoice(true, false), after_due),
            Status::Overdue
        );
        assert_eq!(
            totals(1000, 1000, 0, 0).status(&invoice(true, true), after_due),
            Status::Paid
        );
        assert_eq!(
            totals(1000, 600, 400, 0).status(&invoice(true, true), after_due),
            Status::Paid
        );
        assert_eq!(
            totals(1000, 0, 1000, 0).status(&invoice(true, true), after_due),
            Status::Credited
        );
        // Due today is not overdue.
        assert_eq!(
            totals(1000, 0, 0, 0).status(&invoice(true, true), d(2026, 10, 31)),
            Status::Sent
        );
    }

    #[test]
    fn balance_accounts_for_refunds() {
        // Paid in full, then a £200 credit note: £200 owed back to the client.
        let t = totals(1000, 1000, 200, 0);
        assert_eq!(t.balance(), Money::from_pence(-200));
        // After refunding it, the books balance.
        let t = totals(1000, 1000, 200, 200);
        assert_eq!(t.balance(), Money::ZERO);
        assert_eq!(t.net_received(), Money::from_pence(800));
    }

    #[test]
    fn address_lines_skip_blanks() {
        let a = Address {
            line1: "1 High St".into(),
            line2: " ".into(),
            town: "Bristol".into(),
            postcode: "BS1 1AA".into(),
            country: String::new(),
        };
        assert_eq!(a.lines(), vec!["1 High St", "Bristol", "BS1 1AA"]);
        let abroad = Address {
            line1: "Hauptstraße 1".into(),
            postcode: "10115".into(),
            town: "Berlin".into(),
            country: "Germany".into(),
            ..Address::default()
        };
        assert_eq!(
            abroad.lines(),
            vec!["Hauptstraße 1", "Berlin", "10115", "Germany"]
        );
    }
}
