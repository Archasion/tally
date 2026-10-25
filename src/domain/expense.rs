//! Business expenses, recorded on the cash basis (by the date paid).

use chrono::NaiveDate;

use super::Money;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expense {
    pub id: i64,
    /// The date you paid (cash basis).
    pub date: NaiveDate,
    pub description: String,
    pub category: ExpenseCategory,
    pub amount: Money,
    pub notes: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_round_trip() {
        for c in ExpenseCategory::ALL {
            assert_eq!(ExpenseCategory::parse(c.as_str()), Some(c));
        }
    }
}
