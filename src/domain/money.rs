//! Money and quantities as fixed-point integers. No floats anywhere.

use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

/// An amount of money in pence (GBP).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Money(i64);

impl Money {
    pub const ZERO: Money = Money(0);

    pub const fn from_pence(pence: i64) -> Self {
        Money(pence)
    }

    pub const fn pence(self) -> i64 {
        self.0
    }

    pub fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub fn is_positive(self) -> bool {
        self.0 > 0
    }

    pub fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// Parses user input such as `12`, `12.5`, `£1,234.56` or `-3.20`.
    pub fn parse(input: &str) -> Result<Self, String> {
        let cleaned: String = input
            .trim()
            .chars()
            .filter(|c| !matches!(c, '£' | ',' | ' '))
            .collect();
        let hundredths = parse_hundredths(&cleaned)
            .ok_or_else(|| format!("'{}' is not a valid amount (e.g. 1250.00)", input.trim()))?;
        Ok(Money(hundredths))
    }

    /// Formats as a plain decimal without the currency symbol, e.g. `1234.50`.
    pub fn to_decimal_string(self) -> String {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        format!("{sign}{}.{:02}", abs / 100, abs % 100)
    }
}

impl fmt::Display for Money {
    /// Formats as `£1,234.56` (or `-£1,234.56`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        let pounds = group_thousands(abs / 100);
        let s = format!("{sign}£{pounds}.{:02}", abs % 100);
        f.pad(&s)
    }
}

impl Add for Money {
    type Output = Money;
    fn add(self, rhs: Money) -> Money {
        Money(self.0 + rhs.0)
    }
}

impl AddAssign for Money {
    fn add_assign(&mut self, rhs: Money) {
        self.0 += rhs.0;
    }
}

impl Sub for Money {
    type Output = Money;
    fn sub(self, rhs: Money) -> Money {
        Money(self.0 - rhs.0)
    }
}

impl SubAssign for Money {
    fn sub_assign(&mut self, rhs: Money) {
        self.0 -= rhs.0;
    }
}

impl Neg for Money {
    type Output = Money;
    fn neg(self) -> Money {
        Money(-self.0)
    }
}

impl Sum for Money {
    fn sum<I: Iterator<Item = Money>>(iter: I) -> Money {
        iter.fold(Money::ZERO, Add::add)
    }
}

/// A quantity (hours, days or units) stored in hundredths, so `1.5` hours is `150`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Quantity(i64);

impl Quantity {
    pub const ONE: Quantity = Quantity(100);

    pub const fn from_hundredths(h: i64) -> Self {
        Quantity(h)
    }

    pub const fn hundredths(self) -> i64 {
        self.0
    }

    /// Parses `1`, `1.5`, `0.25`. At most two decimal places.
    pub fn parse(input: &str) -> Result<Self, String> {
        let h = parse_hundredths(input.trim())
            .ok_or_else(|| format!("'{}' is not a valid quantity (e.g. 7.5)", input.trim()))?;
        if h <= 0 {
            return Err("Quantity must be greater than zero".into());
        }
        Ok(Quantity(h))
    }

    /// `quantity × unit price`, rounded half away from zero to the penny.
    pub fn times(self, unit: Money) -> Money {
        let product = i128::from(self.0) * i128::from(unit.pence());
        let rounded = if product >= 0 {
            (product + 50) / 100
        } else {
            (product - 50) / 100
        };
        Money::from_pence(i64::try_from(rounded).expect("line amount overflow"))
    }
}

impl fmt::Display for Quantity {
    /// Shows `1`, `1.5`, `0.25` — no trailing zeros.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        let (whole, frac) = (abs / 100, abs % 100);
        let s = match frac {
            0 => format!("{sign}{whole}"),
            f if f % 10 == 0 => format!("{sign}{whole}.{}", f / 10),
            f => format!("{sign}{whole}.{f:02}"),
        };
        f.pad(&s)
    }
}

/// Parses an optionally signed decimal with up to two fractional digits into hundredths.
fn parse_hundredths(s: &str) -> Option<i64> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    if digits.is_empty() {
        return None;
    }
    let (whole, frac) = match digits.split_once('.') {
        Some((w, f)) => (w, f),
        None => (digits, ""),
    };
    if frac.len() > 2
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !frac.chars().all(|c| c.is_ascii_digit())
        || (whole.is_empty() && frac.is_empty())
    {
        return None;
    }
    let whole: i64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let frac: i64 = match frac.len() {
        0 => 0,
        1 => frac.parse::<i64>().ok()? * 10,
        _ => frac.parse().ok()?,
    };
    let value = whole.checked_mul(100)?.checked_add(frac)?;
    Some(if neg { -value } else { value })
}

fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_money() {
        assert_eq!(Money::parse("12").unwrap(), Money::from_pence(1200));
        assert_eq!(Money::parse("12.5").unwrap(), Money::from_pence(1250));
        assert_eq!(Money::parse("£1,234.56").unwrap(), Money::from_pence(123_456));
        assert_eq!(Money::parse(".5").unwrap(), Money::from_pence(50));
        assert_eq!(Money::parse("-3.20").unwrap(), Money::from_pence(-320));
        assert!(Money::parse("1.234").is_err());
        assert!(Money::parse("abc").is_err());
        assert!(Money::parse("").is_err());
        assert!(Money::parse(".").is_err());
    }

    #[test]
    fn formats_money() {
        assert_eq!(Money::from_pence(0).to_string(), "£0.00");
        assert_eq!(Money::from_pence(5).to_string(), "£0.05");
        assert_eq!(Money::from_pence(123_456_789).to_string(), "£1,234,567.89");
        assert_eq!(Money::from_pence(-1050).to_string(), "-£10.50");
        assert_eq!(Money::from_pence(-1050).to_decimal_string(), "-10.50");
        assert_eq!(format!("{:>8}", Money::from_pence(100)), "   £1.00");
    }

    #[test]
    fn parses_and_formats_quantity() {
        assert_eq!(Quantity::parse("1.5").unwrap().hundredths(), 150);
        assert_eq!(Quantity::parse("0.25").unwrap().to_string(), "0.25");
        assert_eq!(Quantity::parse("7.50").unwrap().to_string(), "7.5");
        assert_eq!(Quantity::parse("3").unwrap().to_string(), "3");
        assert!(Quantity::parse("0").is_err());
        assert!(Quantity::parse("-1").is_err());
    }

    #[test]
    fn line_amounts_round_half_up_to_the_penny() {
        // 1.5 h × £45.00 = £67.50
        assert_eq!(Quantity::from_hundredths(150).times(Money::from_pence(4500)).pence(), 6750);
        // 0.33 × £0.50 = 16.5p → 17p
        assert_eq!(Quantity::from_hundredths(33).times(Money::from_pence(50)).pence(), 17);
        // 0.33 × £0.10 = 3.3p → 3p
        assert_eq!(Quantity::from_hundredths(33).times(Money::from_pence(10)).pence(), 3);
        // Negative prices round away from zero symmetrically.
        assert_eq!(Quantity::from_hundredths(33).times(Money::from_pence(-50)).pence(), -17);
    }
}
