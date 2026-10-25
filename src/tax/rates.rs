//! Tax rates per tax year, from the built-in `rates/uk.toml` plus an optional
//! `rates.toml` in the output folder that replaces whole years.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::domain::Money;
use crate::domain::dates::TaxYear;

pub const BUILTIN_RATES: &str = include_str!("../../rates/uk.toml");

/// A percentage stored in basis points (6% = 600) so no floats reach the maths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rate(u32);

impl Rate {
    pub fn from_bp(bp: u32) -> Self {
        Rate(bp)
    }

    /// `amount × rate`, rounded down to the penny.
    pub fn of(self, amount: Money) -> Money {
        let pence = i128::from(amount.pence()) * i128::from(self.0) / 10_000;
        Money::from_pence(pence as i64)
    }

    pub fn is_zero(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for Rate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (whole, frac) = (self.0 / 100, self.0 % 100);
        let s = match frac {
            0 => format!("{whole}%"),
            f if f % 10 == 0 => format!("{whole}.{}%", f / 10),
            f => format!("{whole}.{f:02}%"),
        };
        f.pad(&s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    BuiltIn,
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Band {
    pub name: String,
    /// Upper limit of taxable income for this band; `None` for the top band.
    pub up_to: Option<Money>,
    pub rate: Rate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomeTaxRates {
    pub personal_allowance: Money,
    pub allowance_taper_threshold: Money,
    pub bands: Vec<Band>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Class4Rates {
    pub lower_profits_limit: Money,
    pub upper_profits_limit: Money,
    pub main_rate: Rate,
    pub upper_rate: Rate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Class2Rates {
    pub small_profits_threshold: Money,
    pub weekly_rate: Money,
    pub weeks: u32,
}

impl Class2Rates {
    pub fn annual(&self) -> Money {
        Money::from_pence(self.weekly_rate.pence() * i64::from(self.weeks))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudentLoanRates {
    pub plan1: Option<Money>,
    pub plan2: Option<Money>,
    pub plan4: Option<Money>,
    pub plan5: Option<Money>,
    pub postgraduate: Option<Money>,
    pub rate: Rate,
    pub postgraduate_rate: Rate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MtdRule {
    pub threshold: Money,
    pub start: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YearRates {
    pub year: TaxYear,
    pub verified: Option<String>,
    pub sources: Vec<String>,
    pub origin: Origin,
    pub trading_allowance: Money,
    pub payments_on_account_threshold: Money,
    pub income_tax: IncomeTaxRates,
    pub class4: Class4Rates,
    pub class2: Class2Rates,
    pub student_loan: StudentLoanRates,
    pub mtd: Option<MtdRule>,
}

impl YearRates {
    /// e.g. `built-in rates, checked against GOV.UK on 2026-10-03`
    pub fn provenance(&self) -> String {
        let checked = self
            .verified
            .as_deref()
            .map(|v| format!(", checked against GOV.UK on {v}"))
            .unwrap_or_default();
        match &self.origin {
            Origin::BuiltIn => format!("built-in rates{checked}"),
            Origin::File(p) => format!("your {}{checked}", crate::paths::display(p)),
        }
    }
}

/// All known tax years.
#[derive(Debug, Clone, Default)]
pub struct RatesBook {
    years: BTreeMap<i32, YearRates>,
}

impl RatesBook {
    pub fn builtin() -> Self {
        let mut book = RatesBook::default();
        book.merge(BUILTIN_RATES, Origin::BuiltIn)
            .expect("built-in rates are valid");
        book
    }

    /// Built-in rates, with years from `<output>/rates.toml` replacing them.
    pub fn load(output_dir: &Path) -> Result<Self> {
        let mut book = Self::builtin();
        let path = crate::paths::rates_override(output_dir);
        if path.is_file() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            book.merge(&text, Origin::File(path.clone()))
                .with_context(|| format!("in {}", crate::paths::display(&path)))?;
        }
        Ok(book)
    }

    pub fn get(&self, year: TaxYear) -> Option<&YearRates> {
        self.years.get(&year.0)
    }

    pub fn years(&self) -> impl Iterator<Item = TaxYear> + '_ {
        self.years.keys().map(|y| TaxYear(*y))
    }

    fn merge(&mut self, text: &str, origin: Origin) -> Result<()> {
        let file: RawFile = toml::from_str(text)?;
        for (key, raw) in file.years {
            let year: i32 = key.parse().with_context(|| {
                format!("[years.{key}]: the key must be the year the tax year starts, e.g. 2026")
            })?;
            let rates = raw
                .into_rates(TaxYear(year), origin.clone())
                .with_context(|| format!("[years.{key}]"))?;
            self.years.insert(year, rates);
        }
        Ok(())
    }
}

// ----- TOML shape -----------------------------------------------------------

/// A TOML number: `12570` or `3.65`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(untagged)]
enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    /// Pounds → pence.
    fn money(self) -> Result<Money> {
        let pence = match self {
            Num::Int(i) => i.checked_mul(100).context("amount too large")?,
            Num::Float(f) => (f * 100.0).round() as i64,
        };
        if pence < 0 {
            bail!("amounts can't be negative");
        }
        Ok(Money::from_pence(pence))
    }

    /// Percent → basis points.
    fn rate(self) -> Result<Rate> {
        let bp = match self {
            Num::Int(i) => i * 100,
            Num::Float(f) => (f * 100.0).round() as i64,
        };
        if !(0..=10_000).contains(&bp) {
            bail!("rates must be between 0 and 100 (percent)");
        }
        Ok(Rate(bp as u32))
    }
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    years: BTreeMap<String, RawYear>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawYear {
    #[allow(dead_code)]
    label: Option<String>,
    verified: Option<String>,
    #[serde(default)]
    sources: Vec<String>,
    trading_allowance: Num,
    payments_on_account_threshold: Num,
    income_tax: RawIncomeTax,
    class4: RawClass4,
    class2: RawClass2,
    student_loan: RawStudentLoan,
    mtd: Option<RawMtd>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawIncomeTax {
    personal_allowance: Num,
    allowance_taper_threshold: Num,
    bands: Vec<RawBand>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBand {
    name: String,
    up_to: Option<Num>,
    rate: Num,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClass4 {
    lower_profits_limit: Num,
    upper_profits_limit: Num,
    main_rate: Num,
    upper_rate: Num,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClass2 {
    small_profits_threshold: Num,
    weekly_rate: Num,
    weeks: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStudentLoan {
    plan1: Option<Num>,
    plan2: Option<Num>,
    plan4: Option<Num>,
    plan5: Option<Num>,
    postgraduate: Option<Num>,
    rate: Num,
    postgraduate_rate: Num,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMtd {
    threshold: Num,
    start: String,
}

fn opt_money(n: Option<Num>) -> Result<Option<Money>> {
    n.map(Num::money).transpose()
}

impl RawYear {
    fn into_rates(self, year: TaxYear, origin: Origin) -> Result<YearRates> {
        let mut bands = Vec::new();
        let count = self.income_tax.bands.len();
        let mut previous = Money::ZERO;
        for (i, b) in self.income_tax.bands.into_iter().enumerate() {
            let up_to = opt_money(b.up_to)?;
            match (up_to, i + 1 == count) {
                (Some(_), true) => bail!("the last income tax band must not have `up_to`"),
                (None, false) => bail!(
                    "band '{}' needs `up_to` (only the last band has no limit)",
                    b.name
                ),
                (Some(limit), false) if limit <= previous => {
                    bail!(
                        "band '{}' must have a higher `up_to` than the band before it",
                        b.name
                    )
                }
                (Some(limit), false) => previous = limit,
                (None, true) => {}
            }
            bands.push(Band {
                name: b.name,
                up_to,
                rate: b.rate.rate()?,
            });
        }
        if bands.is_empty() {
            bail!("income_tax.bands can't be empty");
        }
        let class4 = Class4Rates {
            lower_profits_limit: self.class4.lower_profits_limit.money()?,
            upper_profits_limit: self.class4.upper_profits_limit.money()?,
            main_rate: self.class4.main_rate.rate()?,
            upper_rate: self.class4.upper_rate.rate()?,
        };
        if class4.upper_profits_limit < class4.lower_profits_limit {
            bail!("class4.upper_profits_limit is below lower_profits_limit");
        }
        let mtd = match self.mtd {
            Some(m) => Some(MtdRule {
                threshold: m.threshold.money()?,
                start: NaiveDate::parse_from_str(&m.start, "%Y-%m-%d")
                    .with_context(|| format!("mtd.start '{}' must be YYYY-MM-DD", m.start))?,
            }),
            None => None,
        };
        Ok(YearRates {
            year,
            verified: self.verified,
            sources: self.sources,
            origin,
            trading_allowance: self.trading_allowance.money()?,
            payments_on_account_threshold: self.payments_on_account_threshold.money()?,
            income_tax: IncomeTaxRates {
                personal_allowance: self.income_tax.personal_allowance.money()?,
                allowance_taper_threshold: self.income_tax.allowance_taper_threshold.money()?,
                bands,
            },
            class4,
            class2: Class2Rates {
                small_profits_threshold: self.class2.small_profits_threshold.money()?,
                weekly_rate: self.class2.weekly_rate.money()?,
                weeks: self.class2.weeks,
            },
            student_loan: StudentLoanRates {
                plan1: opt_money(self.student_loan.plan1)?,
                plan2: opt_money(self.student_loan.plan2)?,
                plan4: opt_money(self.student_loan.plan4)?,
                plan5: opt_money(self.student_loan.plan5)?,
                postgraduate: opt_money(self.student_loan.postgraduate)?,
                rate: self.student_loan.rate.rate()?,
                postgraduate_rate: self.student_loan.postgraduate_rate.rate()?,
            },
            mtd,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gbp(pounds: i64) -> Money {
        Money::from_pence(pounds * 100)
    }

    #[test]
    fn builtin_rates_match_gov_uk() {
        let book = RatesBook::builtin();
        assert_eq!(
            book.years().map(|y| y.0).collect::<Vec<_>>(),
            [2024, 2025, 2026]
        );
        let r = book.get(TaxYear(2026)).unwrap();
        assert_eq!(r.income_tax.personal_allowance, gbp(12_570));
        assert_eq!(r.income_tax.bands[0].up_to, Some(gbp(37_700)));
        assert_eq!(r.income_tax.bands[2].rate, Rate(4500));
        assert_eq!(r.class4.main_rate, Rate(600));
        assert_eq!(r.class2.small_profits_threshold, gbp(7_105));
        assert_eq!(r.class2.annual(), Money::from_pence(18_980)); // £3.65 × 52
        assert_eq!(r.student_loan.plan5, Some(gbp(25_000)));
        assert_eq!(book.get(TaxYear(2025)).unwrap().student_loan.plan5, None);
        assert_eq!(
            book.get(TaxYear(2025)).unwrap().class2.annual(),
            Money::from_pence(18_200)
        );
        assert!(book.get(TaxYear(2027)).is_none());
        assert_eq!(r.origin, Origin::BuiltIn);
    }

    #[test]
    fn rates_display_and_apply() {
        assert_eq!(Rate(600).to_string(), "6%");
        assert_eq!(Rate(875).to_string(), "8.75%");
        assert_eq!(Rate(350).to_string(), "3.5%");
        // 6% of £17,430.00 = £1,045.80
        assert_eq!(Rate(600).of(gbp(17_430)), Money::from_pence(104_580));
        // Rounds down to the penny.
        assert_eq!(Rate(600).of(Money::from_pence(1)), Money::ZERO);
    }

    #[test]
    fn override_file_replaces_a_year_and_adds_new_ones() {
        let mut book = RatesBook::builtin();
        let mut text = BUILTIN_RATES.replace("[years.2026", "[years.2027");
        text = text
            .replace("[years.2025", "[years.1925")
            .replace("[years.2024", "[years.1924");
        text = text.replace("personal_allowance = 12570", "personal_allowance = 13000");
        book.merge(&text, Origin::File("rates.toml".into()))
            .unwrap();
        let r = book.get(TaxYear(2027)).unwrap();
        assert_eq!(r.income_tax.personal_allowance, gbp(13_000));
        assert!(matches!(r.origin, Origin::File(_)));
        assert_eq!(book.get(TaxYear(2026)).unwrap().origin, Origin::BuiltIn);
    }

    #[test]
    fn rejects_malformed_years() {
        let mut book = RatesBook::default();
        let bad_band = BUILTIN_RATES.replace(
            "{ name = \"Additional rate\", rate = 45 }",
            "{ name = \"Additional rate\", up_to = 1, rate = 45 }",
        );
        assert!(book.merge(&bad_band, Origin::BuiltIn).is_err());
        let typo = BUILTIN_RATES.replace("main_rate = 6", "mian_rate = 6");
        assert!(book.merge(&typo, Origin::BuiltIn).is_err());
        let bad_rate = BUILTIN_RATES.replace("upper_rate = 2", "upper_rate = 200");
        assert!(book.merge(&bad_rate, Origin::BuiltIn).is_err());
    }
}
