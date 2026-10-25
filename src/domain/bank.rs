//! Bank details for international payments: IBAN and BIC checks, and
//! deciding which clients are outside the UK.

/// Normalises an IBAN to upper case without spaces and checks its shape and
/// check digits (ISO 13616, mod 97). UK IBANs must be 22 characters.
pub fn normalize_iban(input: &str) -> Result<String, String> {
    let iban: String = input
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    let bytes = iban.as_bytes();
    let shape_ok = (15..=34).contains(&iban.len())
        && bytes[..2].iter().all(u8::is_ascii_uppercase)
        && bytes[2..4].iter().all(u8::is_ascii_digit)
        && bytes.iter().all(u8::is_ascii_alphanumeric);
    if !shape_ok {
        return Err("An IBAN is 2 letters, 2 digits, then up to 30 letters or digits".into());
    }
    if iban.starts_with("GB") && iban.len() != 22 {
        return Err("A UK IBAN is 22 characters, e.g. GB29 NWBK 6016 1331 9268 19".into());
    }
    // Move the first four characters to the end, read letters as 10–35,
    // and the remainder mod 97 must be 1.
    let remainder = iban[4..]
        .chars()
        .chain(iban[..4].chars())
        .fold(0u32, |acc, c| {
            let value = c.to_digit(36).unwrap_or(0);
            if value >= 10 {
                (acc * 100 + value) % 97
            } else {
                (acc * 10 + value) % 97
            }
        });
    if remainder != 1 {
        return Err("That IBAN's check digits don't add up. Check for a typo".into());
    }
    Ok(iban)
}

/// Whether a UK IBAN carries this sort code and account number. Always true
/// for other countries' IBANs, which don't contain them.
pub fn gb_iban_matches(iban: &str, sort_code: &str, account_number: &str) -> bool {
    if !iban.starts_with("GB") || iban.len() != 22 {
        return true;
    }
    let sort: String = sort_code.chars().filter(char::is_ascii_digit).collect();
    // UK IBANs hold 8-digit account numbers; shorter ones are zero-padded.
    let account = format!("{account_number:0>8}");
    iban[8..14] == sort && iban[14..] == account
}

/// Groups an IBAN in fours for printing: `GB29 NWBK 6016 1331 9268 19`.
pub fn format_iban(iban: &str) -> String {
    iban.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Normalises a BIC (SWIFT code) to upper case and checks its shape:
/// 4 letters (bank), 2 letters (country), 2 letters/digits (location) and
/// an optional 3-character branch.
pub fn normalize_bic(input: &str) -> Result<String, String> {
    let bic: String = input
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    let b = bic.as_bytes();
    let ok = (bic.len() == 8 || bic.len() == 11)
        && b[..6].iter().all(u8::is_ascii_uppercase)
        && b[6..].iter().all(u8::is_ascii_alphanumeric);
    if ok {
        Ok(bic)
    } else {
        Err("A BIC is 8 or 11 letters/digits, e.g. NWBKGB2L".into())
    }
}

/// The two-letter country in a BIC, e.g. `GB` in `NWBKGB2L`.
pub fn bic_country(bic: &str) -> &str {
    bic.get(4..6).unwrap_or("")
}

/// Whether a client's country is the UK. Empty counts as the UK: snapshots
/// from before countries were recorded were all UK clients.
pub fn is_uk(country: &str) -> bool {
    let c = country.trim().to_lowercase();
    matches!(
        c.as_str(),
        "" | "united kingdom"
            | "uk"
            | "u.k."
            | "gb"
            | "great britain"
            | "britain"
            | "england"
            | "scotland"
            | "wales"
            | "northern ireland"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ibans_are_checked_and_normalised() {
        assert_eq!(
            normalize_iban("gb29 nwbk 6016 1331 9268 19"),
            Ok("GB29NWBK60161331926819".into())
        );
        assert_eq!(
            normalize_iban("DE89 3704 0044 0532 0130 00"),
            Ok("DE89370400440532013000".into())
        );
        // One digit wrong.
        assert!(normalize_iban("GB29NWBK60161331926818").is_err());
        assert!(normalize_iban("GB29NWBK601613319268").is_err());
        assert!(normalize_iban("12345").is_err());
        assert_eq!(
            format_iban("GB29NWBK60161331926819"),
            "GB29 NWBK 6016 1331 9268 19"
        );
    }

    #[test]
    fn uk_ibans_must_match_the_account() {
        let iban = "GB29NWBK60161331926819";
        assert!(gb_iban_matches(iban, "60-16-13", "31926819"));
        assert!(!gb_iban_matches(iban, "60-16-14", "31926819"));
        assert!(!gb_iban_matches(iban, "60-16-13", "31926810"));
        assert!(gb_iban_matches("DE89370400440532013000", "60-16-13", "1"));
    }

    #[test]
    fn bics_and_countries() {
        assert_eq!(normalize_bic("nwbk gb2l"), Ok("NWBKGB2L".into()));
        assert_eq!(normalize_bic("NWBKGB2LXXX"), Ok("NWBKGB2LXXX".into()));
        assert!(normalize_bic("NWBK1B2L").is_err());
        assert!(normalize_bic("NWBKGB2").is_err());
        assert_eq!(bic_country("NWBKGB2L"), "GB");
        assert!(is_uk("United Kingdom") && is_uk(" uk ") && is_uk("Scotland") && is_uk(""));
        assert!(!is_uk("Germany") && !is_uk("Ireland"));
    }
}
