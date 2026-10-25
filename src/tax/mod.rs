//! Self Assessment estimate for a UK sole trader whose only income is
//! self-employment: income tax (England, Wales & NI rates), Class 4 and
//! Class 2 National Insurance, student loan repayments, payments on account
//! and Making Tax Digital thresholds.
//!
//! Everything is computed in pence with the rates from [`rates`]. Every line
//! of the [`breakdown`] states the rate and threshold it used.

pub mod rates;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::domain::Money;
use crate::domain::dates::{self, TaxYear};
use rates::{Rate, YearRates};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoanPlan {
    Plan1,
    Plan2,
    Plan4,
    Plan5,
}

impl LoanPlan {
    pub const ALL: [LoanPlan; 4] = [
        LoanPlan::Plan1,
        LoanPlan::Plan2,
        LoanPlan::Plan4,
        LoanPlan::Plan5,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            LoanPlan::Plan1 => "plan1",
            LoanPlan::Plan2 => "plan2",
            LoanPlan::Plan4 => "plan4",
            LoanPlan::Plan5 => "plan5",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            LoanPlan::Plan1 => "Plan 1",
            LoanPlan::Plan2 => "Plan 2",
            LoanPlan::Plan4 => "Plan 4",
            LoanPlan::Plan5 => "Plan 5",
        }
    }

    fn threshold(self, r: &rates::StudentLoanRates) -> Option<Money> {
        match self {
            LoanPlan::Plan1 => r.plan1,
            LoanPlan::Plan2 => r.plan2,
            LoanPlan::Plan4 => r.plan4,
            LoanPlan::Plan5 => r.plan5,
        }
    }
}

/// Personal tax settings (Settings tab).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaxSettings {
    /// Master switch for student loan repayments.
    pub student_loan: bool,
    /// Undergraduate plan; `None` if you only have a Postgraduate Loan.
    pub plan: Option<LoanPlan>,
    pub postgraduate: bool,
    /// Add voluntary Class 2 to the bill when profits are below the threshold.
    pub voluntary_class2: bool,
}

/// Money in and out for the year, cash basis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Figures {
    /// Payments received minus refunds paid.
    pub turnover: Money,
    pub expenses: Money,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deduction {
    Expenses(Money),
    TradingAllowance(Money),
}

impl Deduction {
    pub fn amount(self) -> Money {
        match self {
            Deduction::Expenses(m) | Deduction::TradingAllowance(m) => m,
        }
    }
}

/// One slice of income taxed at one rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slice {
    pub name: String,
    pub rate: Rate,
    pub from: Money,
    pub to: Option<Money>,
    pub amount: Money,
    pub tax: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Class2 {
    /// Profits at or above the Small Profits Threshold: treated as paid.
    TreatedAsPaid { threshold: Money },
    /// Below the threshold: optional voluntary contributions.
    Voluntary {
        threshold: Money,
        weekly: Money,
        weeks: u32,
        annual: Money,
        included: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanRepayment {
    pub name: String,
    pub threshold: Money,
    pub rate: Rate,
    pub over: Money,
    pub repayment: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Estimate {
    pub year: TaxYear,
    pub figures: Figures,
    pub deduction: Deduction,
    /// Expenses above turnover (not relieved in this estimate).
    pub loss: Money,
    /// Taxable profit, rounded down to whole pounds as HMRC does.
    pub profit: Money,
    pub personal_allowance: Money,
    /// How much the allowance was tapered (income over £100,000).
    pub allowance_reduction: Money,
    pub income_tax: Vec<Slice>,
    pub class4: Vec<Slice>,
    pub class2: Class2,
    pub student_loan_enabled: bool,
    pub student_loans: Vec<LoanRepayment>,
    pub student_loan_notes: Vec<String>,
}

impl Estimate {
    pub fn income_tax_total(&self) -> Money {
        self.income_tax.iter().map(|s| s.tax).sum()
    }

    pub fn class4_total(&self) -> Money {
        self.class4.iter().map(|s| s.tax).sum()
    }

    pub fn class2_due(&self) -> Money {
        match self.class2 {
            Class2::Voluntary {
                annual,
                included: true,
                ..
            } => annual,
            _ => Money::ZERO,
        }
    }

    pub fn student_loan_total(&self) -> Money {
        self.student_loans.iter().map(|l| l.repayment).sum()
    }

    /// Income tax + Class 4: what payments on account are based on.
    pub fn poa_basis(&self) -> Money {
        self.income_tax_total() + self.class4_total()
    }

    pub fn total(&self) -> Money {
        self.poa_basis() + self.class2_due() + self.student_loan_total()
    }

    pub fn taxable_income(&self) -> Money {
        (self.profit - self.personal_allowance).max(Money::ZERO)
    }
}

fn whole_pounds(m: Money) -> Money {
    Money::from_pence(m.pence().max(0) / 100 * 100)
}

/// Slices `amount` between consecutive limits and applies each rate.
fn slice(amount: Money, parts: &[(String, Money, Option<Money>, Rate)]) -> Vec<Slice> {
    parts
        .iter()
        .filter_map(|(name, from, to, rate)| {
            let top = to.map_or(amount, |t| amount.min(t));
            let taxed = top - *from;
            taxed.is_positive().then(|| Slice {
                name: name.clone(),
                rate: *rate,
                from: *from,
                to: *to,
                amount: taxed,
                tax: rate.of(taxed),
            })
        })
        .collect()
}

pub fn estimate(rates: &YearRates, figures: Figures, settings: &TaxSettings) -> Estimate {
    // Profit: actual expenses or the trading allowance, whichever is higher.
    let turnover = figures.turnover.max(Money::ZERO);
    let deduction = if figures.expenses >= rates.trading_allowance {
        Deduction::Expenses(figures.expenses)
    } else {
        // The allowance can't create a loss.
        Deduction::TradingAllowance(rates.trading_allowance.min(turnover))
    };
    let loss = (deduction.amount() - turnover).max(Money::ZERO);
    let profit = whole_pounds((turnover - deduction.amount()).max(Money::ZERO));

    // Personal allowance, tapered by £1 for every £2 over the threshold.
    let it = &rates.income_tax;
    let over_taper = (profit - it.allowance_taper_threshold).max(Money::ZERO);
    let reduction =
        whole_pounds(Money::from_pence(over_taper.pence() / 2)).min(it.personal_allowance);
    let personal_allowance = it.personal_allowance - reduction;
    let taxable = (profit - personal_allowance).max(Money::ZERO);

    let mut from = Money::ZERO;
    let mut parts = Vec::new();
    for band in &it.bands {
        parts.push((band.name.clone(), from, band.up_to, band.rate));
        if let Some(limit) = band.up_to {
            from = limit;
        }
    }
    let income_tax = slice(taxable, &parts);

    let c4 = &rates.class4;
    let class4 = slice(
        profit,
        &[
            (
                "Main rate".into(),
                c4.lower_profits_limit,
                Some(c4.upper_profits_limit),
                c4.main_rate,
            ),
            (
                "Upper rate".into(),
                c4.upper_profits_limit,
                None,
                c4.upper_rate,
            ),
        ],
    );

    let c2 = &rates.class2;
    let class2 = if profit >= c2.small_profits_threshold {
        Class2::TreatedAsPaid {
            threshold: c2.small_profits_threshold,
        }
    } else {
        Class2::Voluntary {
            threshold: c2.small_profits_threshold,
            weekly: c2.weekly_rate,
            weeks: c2.weeks,
            annual: c2.annual(),
            included: settings.voluntary_class2,
        }
    };

    let mut student_loans = Vec::new();
    let mut student_loan_notes = Vec::new();
    if settings.student_loan {
        let sl = &rates.student_loan;
        let mut add = |name: String, threshold: Option<Money>, rate: Rate| match threshold {
            Some(threshold) => {
                let over = (profit - threshold).max(Money::ZERO);
                // Self Assessment rounds student loan repayments down to whole pounds.
                student_loans.push(LoanRepayment {
                    name,
                    threshold,
                    rate,
                    over,
                    repayment: whole_pounds(rate.of(over)),
                });
            }
            None => student_loan_notes.push(format!(
                "{name} has no repayment threshold for {}",
                rates.year.label()
            )),
        };
        if let Some(plan) = settings.plan {
            add(plan.label().into(), plan.threshold(sl), sl.rate);
        }
        if settings.postgraduate {
            add(
                "Postgraduate Loan".into(),
                sl.postgraduate,
                sl.postgraduate_rate,
            );
        }
        if settings.plan.is_none() && !settings.postgraduate {
            student_loan_notes.push("Student loan is on, but no plan is chosen in Settings".into());
        }
    }

    Estimate {
        year: rates.year,
        figures,
        deduction,
        loss,
        profit,
        personal_allowance,
        allowance_reduction: reduction,
        income_tax,
        class4,
        class2,
        student_loan_enabled: settings.student_loan,
        student_loans,
        student_loan_notes,
    }
}

/// Scales year-to-date figures to the full tax year by days elapsed.
/// `None` if `today` isn't inside the year.
pub fn project(year: TaxYear, ytd: Figures, today: NaiveDate) -> Option<Projection> {
    if !year.contains(today) {
        return None;
    }
    let elapsed = (today - year.start()).num_days() + 1;
    let total = (year.end() - year.start()).num_days() + 1;
    let scale = |m: Money| {
        Money::from_pence((i128::from(m.pence()) * i128::from(total) / i128::from(elapsed)) as i64)
    };
    Some(Projection {
        figures: Figures {
            turnover: scale(ytd.turnover),
            expenses: scale(ytd.expenses),
        },
        elapsed_days: elapsed,
        total_days: total,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Projection {
    pub figures: Figures,
    pub elapsed_days: i64,
    pub total_days: i64,
}

/// Where last year's bill (for payments on account) came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorBill {
    /// Entered from an HMRC statement.
    Entered(Money),
    /// Calculated by tally from that year's records.
    Calculated(Money),
    /// No rates for the previous year, so it couldn't be calculated.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuePayment {
    pub due: NaiveDate,
    pub what: String,
    pub amount: Money,
}

/// Self Assessment payments linked to `estimate`'s year:
/// payments on account towards it (based on last year), the balancing
/// payment, and the first payments on account for the following year.
pub fn payment_schedule(
    estimate: &Estimate,
    rates: &YearRates,
    prior: PriorBill,
) -> (Vec<DuePayment>, Vec<String>) {
    let y = estimate.year.0;
    let jan1 = NaiveDate::from_ymd_opt(y + 1, 1, 31).expect("valid date");
    let jul1 = NaiveDate::from_ymd_opt(y + 1, 7, 31).expect("valid date");
    let jan2 = NaiveDate::from_ymd_opt(y + 2, 1, 31).expect("valid date");
    let jul2 = NaiveDate::from_ymd_opt(y + 2, 7, 31).expect("valid date");
    let threshold = rates.payments_on_account_threshold;
    let half = |m: Money| Money::from_pence(m.pence() / 2);
    let mut out = Vec::new();
    let mut notes = Vec::new();
    let this_label = estimate.year.label();
    let prev_label = TaxYear(y - 1).label();

    let prior_amount = match prior {
        PriorBill::Entered(m) => {
            notes.push(format!("Payments on account use your entered {prev_label} bill of {m} (income tax + Class 4)."));
            Some(m)
        }
        PriorBill::Calculated(m) => {
            notes.push(format!(
                "Payments on account use tally's {prev_label} estimate of {m} (income tax + Class 4). Press b to enter the figure from your HMRC statement instead."
            ));
            Some(m)
        }
        PriorBill::Unknown => {
            notes.push(format!(
                "tally has no {prev_label} rates to work out last year's bill. Press b to enter it from your HMRC statement."
            ));
            None
        }
    };

    let mut paid_on_account = Money::ZERO;
    if let Some(prior) = prior_amount {
        if prior >= threshold {
            let poa = half(prior);
            paid_on_account = poa + poa;
            out.push(DuePayment {
                due: jan1,
                what: format!("1st payment on account for {this_label}"),
                amount: poa,
            });
            out.push(DuePayment {
                due: jul1,
                what: format!("2nd payment on account for {this_label}"),
                amount: poa,
            });
        } else {
            notes.push(format!(
                "No payments on account for {this_label}: last year's bill was under {}.",
                whole(threshold)
            ));
        }
    }

    let balance = estimate.total() - paid_on_account;
    out.push(DuePayment {
        due: jan2,
        what: if balance.is_negative() {
            format!("Balancing payment for {this_label} (overpaid: refund or credit)")
        } else {
            format!("Balancing payment for {this_label}")
        },
        amount: balance,
    });

    let basis = estimate.poa_basis();
    let next_label = TaxYear(y + 1).label();
    if basis >= threshold {
        let poa = half(basis);
        out.push(DuePayment {
            due: jan2,
            what: format!("1st payment on account for {next_label}"),
            amount: poa,
        });
        out.push(DuePayment {
            due: jul2,
            what: format!("2nd payment on account for {next_label}"),
            amount: poa,
        });
        notes.push(format!(
            "Payments on account are each half of income tax + Class 4 ({basis}); Class 2 and student loans aren't included. You can ask HMRC to reduce them if you expect to earn less."
        ));
    } else {
        notes.push(format!(
            "No payments on account for {next_label}: income tax + Class 4 ({basis}) is under {}.",
            whole(threshold)
        ));
    }
    (out, notes)
}

/// How complete a year's turnover figure is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    SoFar,
    Projected,
    FullYear,
}

/// A Making Tax Digital notice if turnover is over the year's threshold.
pub fn mtd_notice(rates: &YearRates, turnover: Money, basis: Basis) -> Option<String> {
    let rule = rates.mtd.as_ref()?;
    (turnover > rule.threshold).then(|| {
        let verb = match basis {
            Basis::SoFar => "is already",
            Basis::Projected => "is on track to be",
            Basis::FullYear => "was",
        };
        format!(
            "Making Tax Digital: turnover {verb} {turnover} in {}, over the {} threshold. You'll need MTD-compatible software for quarterly updates from {} — tally can't submit to HMRC.",
            rates.year.label(),
            whole(rule.threshold),
            dates::long(rule.start)
        )
    })
}

// ----- Breakdown -------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Heading,
    Item,
    Subtotal,
    Total,
    Note,
}

/// Whether a payment must be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obligation {
    Required,
    Optional,
    NotPayable,
}

impl Obligation {
    pub fn label(self) -> &'static str {
        match self {
            Obligation::Required => "required",
            Obligation::Optional => "optional",
            Obligation::NotPayable => "not payable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    pub label: String,
    /// The rate/threshold used, or an explanation.
    pub detail: String,
    pub amount: Option<Money>,
    pub obligation: Option<Obligation>,
}

impl Line {
    fn new(
        kind: LineKind,
        label: impl Into<String>,
        detail: impl Into<String>,
        amount: Option<Money>,
    ) -> Self {
        Line {
            kind,
            label: label.into(),
            detail: detail.into(),
            amount,
            obligation: None,
        }
    }

    fn heading(label: impl Into<String>, obligation: Obligation) -> Self {
        Line {
            obligation: Some(obligation),
            ..Line::new(LineKind::Heading, label, "", None)
        }
    }

    fn note(text: impl Into<String>) -> Self {
        Line::new(LineKind::Note, text, "", None)
    }
}

fn range(from: Money, to: Option<Money>) -> String {
    match to {
        Some(to) if from.is_zero() => format!("up to {}", whole(to)),
        Some(to) => format!("{}–{}", whole(from), whole(to)),
        None => format!("over {}", whole(from)),
    }
}

/// `£12,570` (no pence when they're zero).
pub fn whole(m: Money) -> String {
    let s = m.to_string();
    s.strip_suffix(".00").map(str::to_string).unwrap_or(s)
}

/// A line-by-line explanation of the estimate, naming every rate used.
pub fn breakdown(e: &Estimate, rates: &YearRates) -> Vec<Line> {
    use LineKind::*;
    let mut out = Vec::new();

    out.push(Line::new(Heading, "Profit", "", None));
    out.push(Line::new(
        Item,
        "Income received",
        "payments minus refunds, cash basis",
        Some(e.figures.turnover),
    ));
    match e.deduction {
        Deduction::Expenses(m) => out.push(Line::new(
            Item,
            "Allowable expenses",
            format!(
                "your recorded expenses (more than the {} trading allowance)",
                whole(rates.trading_allowance)
            ),
            Some(-m),
        )),
        Deduction::TradingAllowance(m) => out.push(Line::new(
            Item,
            "Trading allowance",
            format!(
                "{} allowance instead of your {} of expenses (whichever is higher)",
                whole(rates.trading_allowance),
                e.figures.expenses
            ),
            Some(-m),
        )),
    }
    if e.loss.is_positive() {
        out.push(Line::note(format!(
            "Expenses exceed income by {}. Loss relief isn't modelled; taxable profit is £0.",
            e.loss
        )));
    }
    out.push(Line::new(
        Subtotal,
        "Taxable profit",
        "rounded down to whole pounds",
        Some(e.profit),
    ));

    out.push(Line::heading(
        "Income tax (England, Wales & NI rates)",
        Obligation::Required,
    ));
    let it = &rates.income_tax;
    let pa_detail = if e.allowance_reduction.is_positive() {
        format!(
            "{} reduced by {} (£1 for every £2 of income over {})",
            whole(it.personal_allowance),
            whole(e.allowance_reduction),
            whole(it.allowance_taper_threshold)
        )
    } else {
        format!(
            "{} tax-free (reduces by £1 per £2 of income over {})",
            whole(it.personal_allowance),
            whole(it.allowance_taper_threshold)
        )
    };
    out.push(Line::new(
        Item,
        "Personal allowance",
        pa_detail,
        Some(e.personal_allowance),
    ));
    if e.income_tax.is_empty() {
        out.push(Line::note(
            "Profit is within the personal allowance: no income tax.",
        ));
    }
    for s in &e.income_tax {
        out.push(Line::new(
            Item,
            format!("{} {}", s.name, s.rate),
            format!("on {} (taxable income {})", s.amount, range(s.from, s.to)),
            Some(s.tax),
        ));
    }
    out.push(Line::new(
        Subtotal,
        "Income tax",
        "",
        Some(e.income_tax_total()),
    ));

    let c4 = &rates.class4;
    let c4_obligation = if e.class4.is_empty() {
        Obligation::NotPayable
    } else {
        Obligation::Required
    };
    out.push(Line::heading("Class 4 National Insurance", c4_obligation));
    if e.class4.is_empty() {
        out.push(Line::note(format!(
            "Profit is at or below the Lower Profits Limit of {}: nothing to pay. ({} up to {}, {} above.)",
            whole(c4.lower_profits_limit),
            c4.main_rate,
            whole(c4.upper_profits_limit),
            c4.upper_rate
        )));
    }
    for s in &e.class4 {
        out.push(Line::new(
            Item,
            format!("{} {}", s.name, s.rate),
            format!("on {} (profits {})", s.amount, range(s.from, s.to)),
            Some(s.tax),
        ));
    }
    out.push(Line::new(Subtotal, "Class 4", "", Some(e.class4_total())));

    match &e.class2 {
        Class2::TreatedAsPaid { threshold } => {
            out.push(Line::heading(
                "Class 2 National Insurance",
                Obligation::NotPayable,
            ));
            out.push(Line::new(
                Item,
                "Treated as paid",
                format!(
                    "profit is at least the {} Small Profits Threshold, so your NI record is protected without paying",
                    whole(*threshold)
                ),
                Some(Money::ZERO),
            ));
        }
        Class2::Voluntary {
            threshold,
            weekly,
            weeks,
            annual,
            included,
        } => {
            out.push(Line::heading(
                "Class 2 National Insurance",
                Obligation::Optional,
            ));
            let status = if *included {
                "included in the total (Settings: pay voluntary Class 2)"
            } else {
                "not included in the total; switch on in Settings if you'll pay it"
            };
            out.push(Line::new(
                Item,
                "Voluntary contributions",
                format!(
                    "profit is under the {} Small Profits Threshold. {weekly} × {weeks} weeks = {annual}, protects your State Pension record; {status}",
                    whole(*threshold)
                ),
                included.then_some(*annual),
            ));
        }
    }

    let sl_obligation = if !e.student_loan_enabled {
        Obligation::NotPayable
    } else if e.student_loan_total().is_positive() {
        Obligation::Required
    } else {
        Obligation::NotPayable
    };
    out.push(Line::heading("Student loan repayments", sl_obligation));
    if !e.student_loan_enabled {
        out.push(Line::note(
            "Off. Switch on in Settings if you have a student loan.",
        ));
    }
    for l in &e.student_loans {
        out.push(Line::new(
            Item,
            format!("{} {}", l.name, l.rate),
            format!(
                "on {} of profit over {} (rounded down to whole pounds)",
                l.over,
                whole(l.threshold)
            ),
            Some(l.repayment),
        ));
    }
    for n in &e.student_loan_notes {
        out.push(Line::note(n.clone()));
    }
    if e.student_loan_enabled {
        out.push(Line::new(
            Subtotal,
            "Student loan",
            "",
            Some(e.student_loan_total()),
        ));
    }

    let effective = if e.profit.is_positive() {
        let bp = i128::from(e.total().pence()) * 10_000 / i128::from(e.profit.pence());
        format!("{} of taxable profit", Rate::from_bp(bp as u32))
    } else {
        String::new()
    };
    out.push(Line::new(
        Total,
        "Estimated bill",
        effective,
        Some(e.total()),
    ));
    out
}

/// Parses `2026` or `2026-27`.
pub fn parse_year(s: &str) -> Option<TaxYear> {
    let start = s.split('-').next()?.trim().parse::<i32>().ok()?;
    (1990..=2200).contains(&start).then_some(TaxYear(start))
}

/// Last day with data that a year-to-date figure covers.
pub fn as_of(year: TaxYear, today: NaiveDate) -> NaiveDate {
    today.min(year.end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rates::RatesBook;

    fn gbp(pounds: i64) -> Money {
        Money::from_pence(pounds * 100)
    }

    fn est(profit_pounds: i64, settings: &TaxSettings) -> Estimate {
        let book = RatesBook::builtin();
        // Expenses of exactly the trading allowance keep turnover = profit + 1000.
        estimate(
            book.get(TaxYear(2026)).unwrap(),
            Figures {
                turnover: gbp(profit_pounds + 1_000),
                expenses: gbp(1_000),
            },
            settings,
        )
    }

    #[test]
    fn basic_rate_example() {
        // Profit £30,000: income tax 20% × £17,430 = £3,486; Class 4 6% × £17,430 = £1,045.80.
        let e = est(30_000, &TaxSettings::default());
        assert_eq!(e.profit, gbp(30_000));
        assert_eq!(e.income_tax_total(), gbp(3_486));
        assert_eq!(e.class4_total(), Money::from_pence(104_580));
        assert!(matches!(e.class2, Class2::TreatedAsPaid { .. }));
        assert_eq!(e.total(), Money::from_pence(453_180));
    }

    #[test]
    fn higher_rate_example() {
        // Profit £60,000: taxable £47,430 → £37,700 × 20% + £9,730 × 40% = £7,540 + £3,892.
        // Class 4: £37,700 × 6% + £9,730 × 2% = £2,262 + £194.60.
        let e = est(60_000, &TaxSettings::default());
        assert_eq!(e.income_tax_total(), gbp(11_432));
        assert_eq!(e.class4_total(), Money::from_pence(245_660));
    }

    #[test]
    fn personal_allowance_taper() {
        // Profit £110,000: allowance £12,570 − £5,000 = £7,570, taxable £102,430.
        let e = est(110_000, &TaxSettings::default());
        assert_eq!(e.personal_allowance, gbp(7_570));
        assert_eq!(e.allowance_reduction, gbp(5_000));
        assert_eq!(e.income_tax_total(), gbp(7_540) + gbp(25_892));
        // Fully withdrawn at £125,140.
        let e = est(130_000, &TaxSettings::default());
        assert_eq!(e.personal_allowance, Money::ZERO);
        // £37,700 × 20% + £87,440 × 40% + £4,860 × 45%
        assert_eq!(e.income_tax_total(), gbp(7_540) + gbp(34_976) + gbp(2_187));
    }

    #[test]
    fn small_profits_make_class2_voluntary() {
        let e = est(5_000, &TaxSettings::default());
        assert_eq!(e.total(), Money::ZERO);
        assert!(
            matches!(e.class2, Class2::Voluntary { annual, included: false, .. } if annual == Money::from_pence(18_980))
        );
        let e = est(
            5_000,
            &TaxSettings {
                voluntary_class2: true,
                ..TaxSettings::default()
            },
        );
        assert_eq!(e.total(), Money::from_pence(18_980));
        // At the threshold it's treated as paid, even with the setting on.
        let e = est(
            7_105,
            &TaxSettings {
                voluntary_class2: true,
                ..TaxSettings::default()
            },
        );
        assert_eq!(e.class2_due(), Money::ZERO);
    }

    #[test]
    fn trading_allowance_vs_expenses() {
        let r = RatesBook::builtin();
        let r = r.get(TaxYear(2026)).unwrap();
        let s = TaxSettings::default();
        let e = estimate(
            r,
            Figures {
                turnover: gbp(10_000),
                expenses: gbp(600),
            },
            &s,
        );
        assert_eq!(e.deduction, Deduction::TradingAllowance(gbp(1_000)));
        assert_eq!(e.profit, gbp(9_000));
        let e = estimate(
            r,
            Figures {
                turnover: gbp(10_000),
                expenses: gbp(2_500),
            },
            &s,
        );
        assert_eq!(e.deduction, Deduction::Expenses(gbp(2_500)));
        // The allowance can't create a loss…
        let e = estimate(
            r,
            Figures {
                turnover: gbp(400),
                expenses: Money::ZERO,
            },
            &s,
        );
        assert_eq!(e.deduction, Deduction::TradingAllowance(gbp(400)));
        assert_eq!(e.loss, Money::ZERO);
        // …but real expenses can.
        let e = estimate(
            r,
            Figures {
                turnover: gbp(1_000),
                expenses: gbp(1_500),
            },
            &s,
        );
        assert_eq!(e.loss, gbp(500));
        assert_eq!(e.profit, Money::ZERO);
    }

    #[test]
    fn profit_rounds_down_to_whole_pounds() {
        let r = RatesBook::builtin();
        let e = estimate(
            r.get(TaxYear(2026)).unwrap(),
            Figures {
                turnover: Money::from_pence(3_100_099),
                expenses: gbp(1_000),
            },
            &TaxSettings::default(),
        );
        assert_eq!(e.profit, gbp(30_000));
    }

    #[test]
    fn student_loans() {
        let s = TaxSettings {
            student_loan: true,
            plan: Some(LoanPlan::Plan2),
            postgraduate: true,
            ..Default::default()
        };
        let e = est(40_000, &s);
        // Plan 2: 9% × (£40,000 − £29,385) = £955.35 → £955; PG: 6% × £19,000 = £1,140.
        assert_eq!(e.student_loans[0].repayment, gbp(955));
        assert_eq!(e.student_loans[1].repayment, gbp(1_140));
        assert_eq!(e.student_loan_total(), gbp(2_095));
        // Off by default.
        assert_eq!(
            est(40_000, &TaxSettings::default()).student_loan_total(),
            Money::ZERO
        );
        // Plan 5 didn't exist in 2025-26.
        let book = RatesBook::builtin();
        let s5 = TaxSettings {
            student_loan: true,
            plan: Some(LoanPlan::Plan5),
            ..Default::default()
        };
        let e = estimate(
            book.get(TaxYear(2025)).unwrap(),
            Figures {
                turnover: gbp(40_000),
                expenses: gbp(1_000),
            },
            &s5,
        );
        assert!(e.student_loans.is_empty());
        assert_eq!(e.student_loan_notes.len(), 1);
    }

    #[test]
    fn projection_scales_by_days() {
        let year = TaxYear(2026);
        // 6 Oct 2026 is day 184 of 365.
        let p = project(
            year,
            Figures {
                turnover: gbp(18_400),
                expenses: gbp(1_840),
            },
            NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
        )
        .unwrap();
        assert_eq!(p.elapsed_days, 184);
        assert_eq!(p.total_days, 365);
        assert_eq!(p.figures.turnover, gbp(36_500));
        assert!(
            project(
                year,
                Figures::default(),
                NaiveDate::from_ymd_opt(2027, 5, 1).unwrap()
            )
            .is_none()
        );
    }

    #[test]
    fn payments_on_account() {
        let book = RatesBook::builtin();
        let r = book.get(TaxYear(2026)).unwrap();
        let e = est(30_000, &TaxSettings::default()); // basis £4,531.80
        // Prior bill £3,000 → £1,500 on 31 Jan 2027 and 31 Jul 2027.
        let (sched, _) = payment_schedule(&e, r, PriorBill::Entered(gbp(3_000)));
        let amounts: Vec<(String, Money)> = sched
            .iter()
            .map(|p| (p.due.to_string(), p.amount))
            .collect();
        assert_eq!(
            amounts,
            vec![
                ("2027-01-31".into(), gbp(1_500)),
                ("2027-07-31".into(), gbp(1_500)),
                ("2028-01-31".into(), Money::from_pence(153_180)),
                ("2028-01-31".into(), Money::from_pence(226_590)),
                ("2028-07-31".into(), Money::from_pence(226_590)),
            ]
        );
        // First year: no prior bill, so only the balancing payment and next year's POAs.
        let (sched, _) = payment_schedule(&e, r, PriorBill::Calculated(Money::ZERO));
        assert_eq!(sched[0].amount, e.total());
        assert_eq!(sched.len(), 3);
        // Small bills: no payments on account at all.
        let small = est(14_000, &TaxSettings::default());
        assert!(small.poa_basis() < gbp(1_000));
        let (sched, _) = payment_schedule(&small, r, PriorBill::Calculated(Money::ZERO));
        assert_eq!(sched.len(), 1);
    }

    #[test]
    fn mtd_notices() {
        let book = RatesBook::builtin();
        let r = book.get(TaxYear(2025)).unwrap();
        assert!(mtd_notice(r, gbp(30_000), Basis::FullYear).is_none());
        let n = mtd_notice(r, gbp(32_400), Basis::FullYear).unwrap();
        assert!(n.contains("6 April 2027"), "{n}");
    }

    #[test]
    fn breakdown_names_every_rate() {
        let book = RatesBook::builtin();
        let r = book.get(TaxYear(2026)).unwrap();
        let lines = breakdown(&est(60_000, &TaxSettings::default()), r);
        let text: String = lines
            .iter()
            .map(|l| format!("{} {} ", l.label, l.detail))
            .collect();
        for needle in [
            "Basic rate 20%",
            "Higher rate 40%",
            "£12,570",
            "Main rate 6%",
            "Upper rate 2%",
            "£50,270",
            "£7,105",
        ] {
            assert!(text.contains(needle), "missing {needle} in {text}");
        }
        let class2 = lines
            .iter()
            .find(|l| l.label == "Class 2 National Insurance")
            .unwrap();
        assert_eq!(class2.obligation, Some(Obligation::NotPayable));
    }

    #[test]
    fn parses_years() {
        assert_eq!(parse_year("2026"), Some(TaxYear(2026)));
        assert_eq!(parse_year("2026-27"), Some(TaxYear(2026)));
        assert_eq!(parse_year("x"), None);
    }
}
