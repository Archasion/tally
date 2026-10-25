//! Where things live on disk.
//!
//! - The live database stays on this Mac (`~/Library/Application Support/tally/`),
//!   because sync services can corrupt an SQLite file by copying it mid-write.
//! - PDFs, receipts, backups, CSV exports and the optional `template.typ` go
//!   in the (synced) output folder chosen during setup.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Datelike;
use directories::{BaseDirs, ProjectDirs};

use crate::domain::{CreditNote, Invoice};

/// Database location. `TALLY_DB` overrides it (useful for testing).
pub fn database_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("TALLY_DB") {
        return Ok(PathBuf::from(p));
    }
    let dirs =
        ProjectDirs::from("", "", "tally").context("could not determine the home directory")?;
    Ok(dirs.data_dir().join("tally.db"))
}

/// Shows a path with the home directory as `~`, for compact messages.
pub fn display(path: &Path) -> String {
    if let Some(base) = BaseDirs::new()
        && let Ok(rest) = path.strip_prefix(base.home_dir())
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

/// Expands a leading `~` so users can type `~/Documents/Invoices`.
pub fn expand_home(path: &str) -> PathBuf {
    let path = path.trim();
    if let Some(rest) = path.strip_prefix("~")
        && (rest.is_empty() || rest.starts_with('/'))
        && let Some(base) = BaseDirs::new()
    {
        return base.home_dir().join(rest.trim_start_matches('/'));
    }
    PathBuf::from(path)
}

pub fn template_override(output_dir: &Path) -> PathBuf {
    output_dir.join("template.typ")
}

/// Optional tax rates that add to or replace the built-in years.
pub fn rates_override(output_dir: &Path) -> PathBuf {
    output_dir.join("rates.toml")
}

pub fn fonts_dir(output_dir: &Path) -> PathBuf {
    output_dir.join("fonts")
}

pub fn backups_dir(output_dir: &Path) -> PathBuf {
    output_dir.join("backups")
}

pub fn exports_dir(output_dir: &Path) -> PathBuf {
    output_dir.join("exports")
}

/// Copies of supplier receipts attached to expenses.
pub fn receipts_dir(output_dir: &Path) -> PathBuf {
    output_dir.join("receipts")
}

/// Draft previews go in the system temp folder, never the output folder:
/// `$TMPDIR/tally-preview/draft-12.pdf` (`draft-new.pdf` if unsaved).
pub fn preview_pdf(invoice_id: i64) -> PathBuf {
    let name = if invoice_id == 0 {
        "draft-new.pdf".to_string()
    } else {
        format!("draft-{invoice_id}.pdf")
    };
    std::env::temp_dir().join("tally-preview").join(name)
}

/// `<output>/2026/2026-0001_Acme-Ltd_receipt-1.pdf`, next to the invoice PDF.
pub fn invoice_receipt(output_dir: &Path, invoice: &Invoice, n: usize, ext: &str) -> PathBuf {
    let pdf = invoice_pdf(output_dir, invoice);
    let stem = pdf
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("invoice")
        .to_string();
    pdf.with_file_name(format!("{stem}_receipt-{n}{ext}"))
}

/// `<output>/2026/2026-0001_Acme-Ltd.pdf`
pub fn invoice_pdf(output_dir: &Path, invoice: &Invoice) -> PathBuf {
    let number = invoice.number.as_deref().unwrap_or("draft");
    let client = invoice
        .snapshot
        .as_ref()
        .map(|s| s.client.name.as_str())
        .unwrap_or("client");
    document_pdf(output_dir, invoice.issue_date.year(), number, client)
}

/// `<output>/2026/CN-2026-001_Acme-Ltd.pdf`
pub fn credit_note_pdf(output_dir: &Path, note: &CreditNote) -> PathBuf {
    document_pdf(
        output_dir,
        note.date.year(),
        &note.number,
        &note.snapshot.client.name,
    )
}

fn document_pdf(output_dir: &Path, year: i32, number: &str, client: &str) -> PathBuf {
    output_dir.join(year.to_string()).join(format!(
        "{}_{}.pdf",
        safe_component(number),
        safe_component(client)
    ))
}

/// Makes a string safe for a filename: letters, digits, `-`, `_` and `.`;
/// runs of anything else become a single `-`.
pub fn safe_component(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars() {
        if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches(['-', '.']).to_string();
    if out.is_empty() {
        "untitled".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_filenames() {
        assert_eq!(safe_component("Acme Ltd"), "Acme-Ltd");
        assert_eq!(
            safe_component("Smith & Sons (UK) Ltd."),
            "Smith-Sons-UK-Ltd"
        );
        assert_eq!(safe_component("a/b\\c:d"), "a-b-c-d");
        assert_eq!(safe_component("Café Rouge"), "Café-Rouge");
        assert_eq!(safe_component("../.."), "untitled");
        assert_eq!(safe_component("2026-0001"), "2026-0001");
    }

    #[test]
    fn pdf_paths_use_year_folders() {
        let p = document_pdf(Path::new("/out"), 2026, "2026-0001", "Acme Ltd");
        assert_eq!(p, PathBuf::from("/out/2026/2026-0001_Acme-Ltd.pdf"));
    }

    #[test]
    fn expands_home() {
        let home = BaseDirs::new().unwrap().home_dir().to_path_buf();
        assert_eq!(expand_home("~/Invoices"), home.join("Invoices"));
        assert_eq!(expand_home("~"), home);
        assert_eq!(expand_home("/abs/path"), PathBuf::from("/abs/path"));
        assert_eq!(expand_home("~other/x"), PathBuf::from("~other/x"));
    }
}
