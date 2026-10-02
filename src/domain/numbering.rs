//! Configurable document number patterns such as `{YEAR}-{SEQ:4}`.
//!
//! Supported tokens: `{YEAR}` (2026), `{YY}` (26), `{MONTH}` (01–12),
//! `{LABEL}` (the client's label, e.g. LGT) and `{SEQ}` / `{SEQ:n}`
//! (sequence number zero-padded to `n` digits).
//!
//! If the pattern contains `{YEAR}` or `{YY}`, the sequence restarts every
//! calendar year (on 1 January). Otherwise it never restarts. `{MONTH}` is a
//! label only and does not reset the sequence. If the pattern contains
//! `{LABEL}`, each client label counts separately (LGT-2026-0001 and
//! ACM-2026-0001 can both exist).

use chrono::{Datelike, NaiveDate};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Literal(String),
    Year,
    ShortYear,
    Month,
    Label,
    Seq { width: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberPattern {
    source: String,
    tokens: Vec<Token>,
}

impl NumberPattern {
    pub fn parse(source: &str) -> Result<Self, String> {
        let mut tokens = Vec::new();
        let mut rest = source;
        let mut literal = String::new();
        let mut seq_count = 0;

        while let Some(c) = rest.chars().next() {
            if c == '{' {
                let end = rest
                    .find('}')
                    .ok_or_else(|| format!("Unclosed '{{' in pattern '{source}'"))?;
                let name = &rest[1..end];
                let token = match name {
                    "YEAR" => Token::Year,
                    "YY" => Token::ShortYear,
                    "MONTH" => Token::Month,
                    "LABEL" => Token::Label,
                    "SEQ" => Token::Seq { width: 1 },
                    _ => match name.strip_prefix("SEQ:") {
                        Some(w) => {
                            let width: usize = w
                                .parse()
                                .map_err(|_| format!("Invalid width in '{{{name}}}'"))?;
                            if !(1..=9).contains(&width) {
                                return Err("SEQ width must be between 1 and 9".into());
                            }
                            Token::Seq { width }
                        }
                        None => {
                            return Err(format!(
                                "Unknown token '{{{name}}}'. Use YEAR, YY, MONTH, LABEL, SEQ or SEQ:n"
                            ));
                        }
                    },
                };
                if matches!(token, Token::Seq { .. }) {
                    seq_count += 1;
                }
                if !literal.is_empty() {
                    tokens.push(Token::Literal(std::mem::take(&mut literal)));
                }
                tokens.push(token);
                rest = &rest[end + 1..];
            } else {
                if c == '}' {
                    return Err(format!("Unexpected '}}' in pattern '{source}'"));
                }
                if !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
                    return Err(format!(
                        "'{c}' is not allowed in a number pattern (letters, digits, - _ . only)"
                    ));
                }
                literal.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
        if !literal.is_empty() {
            tokens.push(Token::Literal(literal));
        }
        match seq_count {
            0 => Err("Pattern must contain {SEQ} or {SEQ:n}".into()),
            1 => Ok(NumberPattern {
                source: source.to_string(),
                tokens,
            }),
            _ => Err("Pattern must contain {SEQ} only once".into()),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// Whether the sequence restarts each calendar year.
    pub fn resets_yearly(&self) -> bool {
        self.tokens
            .iter()
            .any(|t| matches!(t, Token::Year | Token::ShortYear))
    }

    /// Whether numbers include the client label (and so count per label).
    pub fn uses_label(&self) -> bool {
        self.tokens.contains(&Token::Label)
    }

    /// The counter scope a document falls into: per label if the pattern
    /// uses `{LABEL}`, and per year if it resets yearly.
    pub fn scope(&self, date: NaiveDate, label: &str) -> String {
        let period = if self.resets_yearly() {
            date.year().to_string()
        } else {
            "all".into()
        };
        if self.uses_label() {
            format!("{label}:{period}")
        } else {
            period
        }
    }

    /// Renders a number. `label` is the client's label; it must be non-empty
    /// if the pattern uses `{LABEL}`.
    pub fn render(&self, seq: u32, date: NaiveDate, label: &str) -> String {
        let mut out = String::new();
        for token in &self.tokens {
            match token {
                Token::Literal(s) => out.push_str(s),
                Token::Year => out.push_str(&format!("{:04}", date.year())),
                Token::ShortYear => out.push_str(&format!("{:02}", date.year() % 100)),
                Token::Month => out.push_str(&format!("{:02}", date.month())),
                Token::Label => out.push_str(label),
                Token::Seq { width } => out.push_str(&format!("{seq:0width$}")),
            }
        }
        out
    }
}

/// Normalises a client label: trimmed and uppercased, A–Z and 0–9 only,
/// at most 10 characters. An empty label means "no label".
pub fn normalize_label(input: &str) -> Result<String, String> {
    let label = input.trim().to_ascii_uppercase();
    if label.len() > 10 {
        return Err("A label can be at most 10 characters".into());
    }
    if !label
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return Err("A label can only contain letters A–Z and digits 0–9".into());
    }
    Ok(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn renders_default_patterns() {
        let p = NumberPattern::parse("{YEAR}-{SEQ:4}").unwrap();
        assert_eq!(p.render(1, d(2026, 10, 2), ""), "2026-0001");
        assert_eq!(p.render(12345, d(2026, 10, 2), ""), "2026-12345");
        let cn = NumberPattern::parse("CN-{YEAR}-{SEQ:3}").unwrap();
        assert_eq!(cn.render(7, d(2027, 1, 1), ""), "CN-2027-007");
    }

    #[test]
    fn renders_all_tokens() {
        let p = NumberPattern::parse("INV{YY}{MONTH}_{SEQ}").unwrap();
        assert_eq!(p.render(42, d(2026, 3, 9), ""), "INV2603_42");
        let labelled = NumberPattern::parse("{LABEL}-{YEAR}-{SEQ:4}").unwrap();
        assert_eq!(labelled.render(1, d(2026, 3, 9), "LGT"), "LGT-2026-0001");
    }

    #[test]
    fn scope_resets_only_with_a_year_token() {
        let yearly = NumberPattern::parse("{YEAR}-{SEQ:4}").unwrap();
        assert_eq!(yearly.scope(d(2026, 12, 31), ""), "2026");
        assert_eq!(yearly.scope(d(2027, 1, 1), ""), "2027");
        let short = NumberPattern::parse("{YY}{SEQ:3}").unwrap();
        assert!(short.resets_yearly());
        let forever = NumberPattern::parse("INV-{MONTH}-{SEQ:4}").unwrap();
        assert!(!forever.resets_yearly());
        assert_eq!(
            forever.scope(d(2026, 1, 1), ""),
            forever.scope(d(2030, 6, 1), "")
        );
    }

    #[test]
    fn labelled_patterns_count_per_label() {
        let p = NumberPattern::parse("{LABEL}-{YEAR}-{SEQ:4}").unwrap();
        assert!(p.uses_label());
        assert_eq!(p.scope(d(2026, 5, 1), "LGT"), "LGT:2026");
        assert_ne!(p.scope(d(2026, 5, 1), "LGT"), p.scope(d(2026, 5, 1), "ACM"));
        let forever = NumberPattern::parse("{LABEL}{SEQ:3}").unwrap();
        assert_eq!(forever.scope(d(2026, 5, 1), "LGT"), "LGT:all");
        // A label can't collide with an unlabelled scope.
        assert!(!NumberPattern::parse("{YEAR}-{SEQ}").unwrap().uses_label());
    }

    #[test]
    fn normalises_labels() {
        assert_eq!(normalize_label(" lgt ").unwrap(), "LGT");
        assert_eq!(normalize_label("").unwrap(), "");
        assert_eq!(normalize_label("ACME2").unwrap(), "ACME2");
        assert!(normalize_label("AC-ME").is_err());
        assert!(normalize_label("ÄCME").is_err());
        assert!(normalize_label("ABCDEFGHIJK").is_err());
    }

    #[test]
    fn rejects_bad_patterns() {
        assert!(NumberPattern::parse("{YEAR}").is_err());
        assert!(NumberPattern::parse("{LABEL}").is_err());
        assert!(NumberPattern::parse("{SEQ}{SEQ}").is_err());
        assert!(NumberPattern::parse("{DAY}-{SEQ}").is_err());
        assert!(NumberPattern::parse("{SEQ:0}").is_err());
        assert!(NumberPattern::parse("{SEQ:4").is_err());
        assert!(NumberPattern::parse("INV/{SEQ}").is_err());
        assert!(NumberPattern::parse("INV {SEQ}").is_err());
    }
}
