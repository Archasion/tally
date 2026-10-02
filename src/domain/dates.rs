//! Date parsing/formatting and UK tax years.

use chrono::{Datelike, Local, NaiveDate};

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Parses `2026-10-02`, `02/10/2026` or `2/10/26` (UK day-first order).
pub fn parse_date(input: &str) -> Result<NaiveDate, String> {
    let s = input.trim();
    if s.eq_ignore_ascii_case("today") {
        return Ok(today());
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d);
    }
    // Pick the year format by digit count: chrono's %Y would read `26` as year 26.
    let two_digit_year = s.rsplit('/').next().is_some_and(|y| y.len() == 2);
    let fmt = if two_digit_year { "%d/%m/%y" } else { "%d/%m/%Y" };
    if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
        return Ok(d);
    }
    Err(format!("'{s}' is not a date (use YYYY-MM-DD or DD/MM/YYYY)"))
}

/// `2 October 2026`
pub fn long(date: NaiveDate) -> String {
    date.format("%-d %B %Y").to_string()
}

/// `02 Oct 2026`
pub fn short(date: NaiveDate) -> String {
    date.format("%d %b %Y").to_string()
}

/// Formats a supply date or period: `2 October 2026` or `1–31 October 2026`.
pub fn supply_period(start: NaiveDate, end: Option<NaiveDate>) -> String {
    match end {
        None => long(start),
        Some(end) if end == start => long(start),
        Some(end) if start.year() == end.year() && start.month() == end.month() => {
            format!("{}–{}", start.day(), long(end))
        }
        Some(end) if start.year() == end.year() => {
            format!("{}–{}", start.format("%-d %B"), long(end))
        }
        Some(end) => format!("{}–{}", long(start), long(end)),
    }
}

/// A UK tax year, identified by the calendar year it starts in.
/// Tax year 2026 runs from 6 April 2026 to 5 April 2027 inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaxYear(pub i32);

impl TaxYear {
    pub fn containing(date: NaiveDate) -> Self {
        let start = NaiveDate::from_ymd_opt(date.year(), 4, 6).expect("valid date");
        if date >= start {
            TaxYear(date.year())
        } else {
            TaxYear(date.year() - 1)
        }
    }

    pub fn start(self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.0, 4, 6).expect("valid date")
    }

    /// Last day of the tax year (inclusive).
    pub fn end(self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.0 + 1, 4, 5).expect("valid date")
    }

    pub fn contains(self, date: NaiveDate) -> bool {
        date >= self.start() && date <= self.end()
    }

    pub fn prev(self) -> Self {
        TaxYear(self.0 - 1)
    }

    pub fn next(self) -> Self {
        TaxYear(self.0 + 1)
    }

    /// `2026-27`
    pub fn label(self) -> String {
        format!("{}-{:02}", self.0, (self.0 + 1) % 100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn tax_year_boundaries() {
        assert_eq!(TaxYear::containing(d(2026, 4, 5)), TaxYear(2025));
        assert_eq!(TaxYear::containing(d(2026, 4, 6)), TaxYear(2026));
        assert_eq!(TaxYear::containing(d(2027, 1, 1)), TaxYear(2026));
        let ty = TaxYear(2026);
        assert_eq!(ty.start(), d(2026, 4, 6));
        assert_eq!(ty.end(), d(2027, 4, 5));
        assert!(ty.contains(d(2027, 4, 5)));
        assert!(!ty.contains(d(2027, 4, 6)));
        assert_eq!(ty.label(), "2026-27");
        assert_eq!(TaxYear(1999).label(), "1999-00");
    }

    #[test]
    fn parses_dates() {
        assert_eq!(parse_date("2026-10-02").unwrap(), d(2026, 10, 2));
        assert_eq!(parse_date("02/10/2026").unwrap(), d(2026, 10, 2));
        assert_eq!(parse_date("2/10/26").unwrap(), d(2026, 10, 2));
        assert!(parse_date("10/32/2026").is_err());
        assert!(parse_date("tomorrow").is_err());
    }

    #[test]
    fn formats_supply_periods() {
        assert_eq!(supply_period(d(2026, 10, 2), None), "2 October 2026");
        assert_eq!(supply_period(d(2026, 10, 1), Some(d(2026, 10, 31))), "1–31 October 2026");
        assert_eq!(
            supply_period(d(2026, 9, 15), Some(d(2026, 10, 14))),
            "15 September–14 October 2026"
        );
        assert_eq!(
            supply_period(d(2026, 12, 15), Some(d(2027, 1, 14))),
            "15 December 2026–14 January 2027"
        );
    }
}
