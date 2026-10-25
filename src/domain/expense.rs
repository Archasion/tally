//! Business expenses, recorded on the cash basis (by the date paid).

use chrono::{Months, NaiveDate};

use super::{Money, dates};

/// Allowable-expense categories as on the Self Assessment full
/// self-employment pages (SA103F). Depreciation and bad debts are left out:
/// depreciation isn't allowable, and bad debts don't arise on the cash basis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExpenseCategory {
    GoodsForResale,
    Subcontractors,
    Staff,
    Travel,
    Premises,
    Repairs,
    Office,
    Advertising,
    LoanInterest,
    FinancialCharges,
    Professional,
    Other,
}

impl ExpenseCategory {
    pub const ALL: [ExpenseCategory; 12] = [
        ExpenseCategory::GoodsForResale,
        ExpenseCategory::Subcontractors,
        ExpenseCategory::Staff,
        ExpenseCategory::Travel,
        ExpenseCategory::Premises,
        ExpenseCategory::Repairs,
        ExpenseCategory::Office,
        ExpenseCategory::Advertising,
        ExpenseCategory::LoanInterest,
        ExpenseCategory::FinancialCharges,
        ExpenseCategory::Professional,
        ExpenseCategory::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ExpenseCategory::GoodsForResale => "goods",
            ExpenseCategory::Subcontractors => "subcontractors",
            ExpenseCategory::Staff => "staff",
            ExpenseCategory::Travel => "travel",
            ExpenseCategory::Premises => "premises",
            ExpenseCategory::Repairs => "repairs",
            ExpenseCategory::Office => "office",
            ExpenseCategory::Advertising => "advertising",
            ExpenseCategory::LoanInterest => "interest",
            ExpenseCategory::FinancialCharges => "financial",
            ExpenseCategory::Professional => "professional",
            ExpenseCategory::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }

    /// The category's name on the Self Assessment form.
    pub fn label(self) -> &'static str {
        match self {
            ExpenseCategory::GoodsForResale => "Cost of goods bought for resale or goods used",
            ExpenseCategory::Subcontractors => "Construction industry: payments to subcontractors",
            ExpenseCategory::Staff => "Wages, salaries and other staff costs",
            ExpenseCategory::Travel => "Car, van and travel expenses",
            ExpenseCategory::Premises => "Rent, rates, power and insurance costs",
            ExpenseCategory::Repairs => "Repairs and maintenance of property and equipment",
            ExpenseCategory::Office => "Phone, fax, stationery and other office costs",
            ExpenseCategory::Advertising => "Advertising costs",
            ExpenseCategory::LoanInterest => "Interest on bank and other loans",
            ExpenseCategory::FinancialCharges => "Bank, credit card and other financial charges",
            ExpenseCategory::Professional => "Accountancy, legal and other professional fees",
            ExpenseCategory::Other => "Other business expenses",
        }
    }

    /// A short name for narrow tables.
    pub fn short(self) -> &'static str {
        match self {
            ExpenseCategory::GoodsForResale => "Goods/stock",
            ExpenseCategory::Subcontractors => "Subcontractors",
            ExpenseCategory::Staff => "Staff",
            ExpenseCategory::Travel => "Travel",
            ExpenseCategory::Premises => "Premises",
            ExpenseCategory::Repairs => "Repairs",
            ExpenseCategory::Office => "Office & phone",
            ExpenseCategory::Advertising => "Advertising",
            ExpenseCategory::LoanInterest => "Loan interest",
            ExpenseCategory::FinancialCharges => "Bank charges",
            ExpenseCategory::Professional => "Professional",
            ExpenseCategory::Other => "Other",
        }
    }

    /// What usually goes here, and common things that aren't allowable.
    pub fn hint(self) -> &'static str {
        match self {
            ExpenseCategory::GoodsForResale => {
                "Stock and materials you sell or use in what you sell"
            }
            ExpenseCategory::Subcontractors => "Only for CIS construction work",
            ExpenseCategory::Staff => {
                "Employees' pay, pensions, agency staff. Not your own drawings"
            }
            ExpenseCategory::Travel => {
                "Business mileage, fuel, train, parking, hotels. Not commuting"
            }
            ExpenseCategory::Premises => {
                "Business premises rent, utilities, insurance; use of home share"
            }
            ExpenseCategory::Repairs => "Repairs to business premises and equipment",
            ExpenseCategory::Office => {
                "Phone, broadband, software subscriptions, stationery, postage"
            }
            ExpenseCategory::Advertising => {
                "Advertising, website, marketing. Entertaining clients isn't allowable"
            }
            ExpenseCategory::LoanInterest => "Interest on business loans and overdrafts",
            ExpenseCategory::FinancialCharges => "Bank fees, card and payment-processor charges",
            ExpenseCategory::Professional => {
                "Accountant, solicitor, professional indemnity insurance"
            }
            ExpenseCategory::Other => {
                "Equipment bought (cash basis), training, trade subscriptions, other"
            }
        }
    }
}

/// How often a purchase is paid for: equipment is usually one-off, software
/// often a subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingPeriod {
    OneOff,
    Monthly,
    Quarterly,
    Annual,
}

impl BillingPeriod {
    pub const ALL: [BillingPeriod; 4] = [
        BillingPeriod::OneOff,
        BillingPeriod::Monthly,
        BillingPeriod::Quarterly,
        BillingPeriod::Annual,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BillingPeriod::OneOff => "one_off",
            BillingPeriod::Monthly => "monthly",
            BillingPeriod::Quarterly => "quarterly",
            BillingPeriod::Annual => "annual",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.as_str() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            BillingPeriod::OneOff => "One-off",
            BillingPeriod::Monthly => "Monthly",
            BillingPeriod::Quarterly => "Quarterly",
            BillingPeriod::Annual => "Annual",
        }
    }

    /// As printed on an invoice line.
    pub fn describe(self) -> &'static str {
        match self {
            BillingPeriod::OneOff => "One-off purchase",
            BillingPeriod::Monthly => "Monthly subscription",
            BillingPeriod::Quarterly => "Quarterly subscription",
            BillingPeriod::Annual => "Annual subscription",
        }
    }

    /// The date of the next payment, or `None` for one-off purchases.
    /// Month ends are clamped, so 31 January is followed by 28/29 February.
    pub fn next_date(self, date: NaiveDate) -> Option<NaiveDate> {
        let months = match self {
            BillingPeriod::OneOff => return None,
            BillingPeriod::Monthly => 1,
            BillingPeriod::Quarterly => 3,
            BillingPeriod::Annual => 12,
        };
        date.checked_add_months(Months::new(months))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expense {
    pub id: i64,
    /// The date you paid (cash basis).
    pub date: NaiveDate,
    pub description: String,
    pub category: ExpenseCategory,
    pub amount: Money,
    pub notes: String,
    pub billing: BillingPeriod,
    /// The client who reimburses this purchase, if any. It's still your
    /// expense; the reimbursement is income when the invoice is paid.
    pub rebill_client_id: Option<i64>,
    /// Who approved the purchase on the client's side, and when.
    pub approved_by: String,
    pub approved_on: Option<NaiveDate>,
    /// The supplier's invoice or receipt number.
    pub receipt_ref: String,
    /// A copy of the receipt, relative to the output folder; empty if none.
    pub receipt_file: String,
}

impl Expense {
    pub fn new(date: NaiveDate) -> Self {
        Expense {
            id: 0,
            date,
            description: String::new(),
            category: ExpenseCategory::Other,
            amount: Money::ZERO,
            notes: String::new(),
            billing: BillingPeriod::OneOff,
            rebill_client_id: None,
            approved_by: String::new(),
            approved_on: None,
            receipt_ref: String::new(),
            receipt_file: String::new(),
        }
    }

    pub fn is_approved(&self) -> bool {
        !self.approved_by.trim().is_empty() && self.approved_on.is_some()
    }

    /// The text printed under the line when this is billed to a client, e.g.
    /// `Monthly subscription, paid 3 October 2026 · Approved by Jo Smith on
    /// 1 October 2026 · Supplier invoice INV-123`.
    pub fn invoice_detail(&self) -> String {
        let mut parts = vec![format!(
            "{}, paid {}",
            self.billing.describe(),
            dates::long(self.date)
        )];
        if let Some(on) = self.approved_on {
            parts.push(format!(
                "Approved by {} on {}",
                self.approved_by.trim(),
                dates::long(on)
            ));
        }
        if !self.receipt_ref.trim().is_empty() {
            parts.push(format!("Supplier invoice {}", self.receipt_ref.trim()));
        }
        parts.join(" · ")
    }

    /// A copy for the next billing period: same details and approval, the
    /// next payment date, and no receipt yet. `None` for one-off purchases.
    pub fn next_period(&self) -> Option<Expense> {
        Some(Expense {
            id: 0,
            date: self.billing.next_date(self.date)?,
            receipt_ref: String::new(),
            receipt_file: String::new(),
            ..self.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_round_trip() {
        for c in ExpenseCategory::ALL {
            assert_eq!(ExpenseCategory::parse(c.as_str()), Some(c));
        }
        for b in BillingPeriod::ALL {
            assert_eq!(BillingPeriod::parse(b.as_str()), Some(b));
        }
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn next_period_dates_clamp_to_month_end() {
        assert_eq!(BillingPeriod::OneOff.next_date(d(2026, 1, 31)), None);
        assert_eq!(
            BillingPeriod::Monthly.next_date(d(2026, 1, 31)),
            Some(d(2026, 2, 28))
        );
        assert_eq!(
            BillingPeriod::Quarterly.next_date(d(2026, 11, 30)),
            Some(d(2027, 2, 28))
        );
        assert_eq!(
            BillingPeriod::Annual.next_date(d(2028, 2, 29)),
            Some(d(2029, 2, 28))
        );
    }

    #[test]
    fn invoice_detail_and_next_period() {
        let mut e = Expense::new(d(2026, 10, 3));
        e.description = "IDE licence".into();
        e.amount = Money::from_pence(2_490);
        e.billing = BillingPeriod::Monthly;
        e.rebill_client_id = Some(1);
        assert!(!e.is_approved());
        assert_eq!(
            e.invoice_detail(),
            "Monthly subscription, paid 3 October 2026"
        );
        e.approved_by = "Jo Smith".into();
        e.approved_on = Some(d(2026, 10, 1));
        e.receipt_ref = "INV-123".into();
        e.receipt_file = "receipts/2026/x.pdf".into();
        assert!(e.is_approved());
        assert_eq!(
            e.invoice_detail(),
            "Monthly subscription, paid 3 October 2026 · Approved by Jo Smith on 1 October 2026 · Supplier invoice INV-123"
        );
        let next = e.next_period().unwrap();
        assert_eq!(next.date, d(2026, 11, 3));
        assert_eq!(next.approved_by, "Jo Smith");
        assert_eq!(next.receipt_ref, "");
        assert_eq!(next.receipt_file, "");
        e.billing = BillingPeriod::OneOff;
        assert!(e.next_period().is_none());
    }
}
