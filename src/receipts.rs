//! Supplier receipts for expenses. Attaching one copies it into
//! `<output>/receipts/<year>/`, so it's synced and backed up with your other
//! files; issuing an invoice copies the receipts of the purchases it bills
//! next to its PDF, ready to attach to the same email.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::Datelike;

use crate::db::Db;
use crate::domain::Invoice;
use crate::domain::expense::Expense;
use crate::paths;

/// Longest name part of a stored receipt, so long descriptions stay readable.
const MAX_NAME: usize = 40;

/// Copies `source` into the receipts folder for `expense` and returns its
/// path relative to the output folder (always with `/`). The file is named
/// after the expense, not the original file: the date paid, then the
/// supplier's invoice number or else the description, e.g.
/// `receipts/2026/2026-10-03_JB-4471.pdf`. Existing receipts are never overwritten.
pub fn import(output_dir: &Path, source: &Path, expense: &Expense) -> Result<String> {
    if !source.is_file() {
        bail!("No file at {}", paths::display(source));
    }
    let paid = expense.date;
    let year = paid.year().to_string();
    let dir = paths::receipts_dir(output_dir).join(&year);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let stem = stored_name(expense);
    let ext = extension(source);
    let mut name = format!("{paid}_{stem}{ext}");
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("{paid}_{stem}-{n}{ext}");
        n += 1;
    }
    let target = dir.join(&name);
    std::fs::copy(source, &target).with_context(|| {
        format!(
            "copying {} to {}",
            paths::display(source),
            paths::display(&target)
        )
    })?;
    Ok(format!("receipts/{year}/{name}"))
}

/// The supplier's invoice number, or else the description, made safe for a
/// filename and shortened to at most [`MAX_NAME`] characters, at a word break.
fn stored_name(expense: &Expense) -> String {
    let basis = if expense.receipt_ref.trim().is_empty() {
        &expense.description
    } else {
        &expense.receipt_ref
    };
    if !basis.chars().any(char::is_alphanumeric) {
        return "receipt".into();
    }
    let full = paths::safe_component(basis);
    let mut name: String = full.chars().take(MAX_NAME).collect();
    // Cut at the last word break rather than mid-word.
    if full.chars().nth(MAX_NAME).is_some_and(|c| c != '-')
        && let Some(i) = name.rfind('-')
    {
        name.truncate(i);
    }
    match name.trim_end_matches(['-', '.']) {
        "" => "receipt".into(),
        trimmed => trimmed.into(),
    }
}

/// Where a stored receipt is on disk. Only paths inside the output folder
/// are accepted, whatever the database says.
pub fn resolve(output_dir: &Path, stored: &str) -> Result<PathBuf> {
    let rel = Path::new(stored);
    if !rel
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        bail!("receipt path {stored} is outside the output folder");
    }
    Ok(output_dir.join(rel))
}

/// Receipts copied next to an invoice's PDF, and stored receipts that
/// couldn't be found.
#[derive(Debug, Default)]
pub struct Enclosures {
    pub copied: Vec<PathBuf>,
    pub missing: Vec<String>,
}

/// Copies the receipts of every reimbursement on `inv` next to its PDF as
/// `<pdf name>_receipt-N.<ext>`, in line order. Earlier copies are removed
/// first, so re-running leaves exactly the current set.
pub fn copy_for_invoice(db: &Db, output_dir: &Path, inv: &Invoice) -> Result<Enclosures> {
    let mut out = Enclosures::default();
    remove_copies(output_dir, inv)?;
    for expense_id in inv.lines.iter().filter_map(|l| l.expense_id) {
        let stored = db.expense(expense_id)?.receipt_file;
        if stored.is_empty() {
            continue;
        }
        let source = resolve(output_dir, &stored)?;
        if !source.is_file() {
            out.missing.push(stored);
            continue;
        }
        let n = out.copied.len() + 1;
        let target = paths::invoice_receipt(output_dir, inv, n, &extension(&source));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::copy(&source, &target)
            .with_context(|| format!("copying {}", paths::display(&source)))?;
        out.copied.push(target);
    }
    Ok(out)
}

/// Deletes `<pdf name>_receipt-*` files left by an earlier run.
fn remove_copies(output_dir: &Path, inv: &Invoice) -> Result<()> {
    let pdf = paths::invoice_pdf(output_dir, inv);
    let (Some(dir), Some(stem)) = (pdf.parent(), pdf.file_stem().and_then(|s| s.to_str())) else {
        return Ok(());
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    let prefix = format!("{stem}_receipt-");
    for entry in entries {
        let path = entry?.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(&prefix))
        {
            std::fs::remove_file(&path)
                .with_context(|| format!("removing {}", paths::display(&path)))?;
        }
    }
    Ok(())
}

/// `.pdf`, lower-cased, or empty if the file has no extension.
fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", paths::safe_component(e).to_ascii_lowercase()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_without_overwriting() {
        let tmp = std::env::temp_dir().join(format!("tally-receipts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let src = tmp.join("Order 113-4471 Nick's stuff (1).PDF");
        std::fs::write(&src, b"%PDF").unwrap();
        let out = tmp.join("out");
        let mut e = Expense::new(chrono::NaiveDate::from_ymd_opt(2026, 10, 3).unwrap());
        e.description = "Samsung T7 500GB portable SSD".into();
        e.receipt_ref = "INV/2026/4471".into();

        // Named after the supplier's invoice number, never the original file.
        let a = import(&out, &src, &e).unwrap();
        assert_eq!(a, "receipts/2026/2026-10-03_INV-2026-4471.pdf");
        let b = import(&out, &src, &e).unwrap();
        assert_eq!(b, "receipts/2026/2026-10-03_INV-2026-4471-2.pdf");
        // Without one, after the description, shortened.
        e.receipt_ref.clear();
        e.description = "PRO USB-C 3.2 Gen2 Male Cable 10Gbps Data 20V 5A 100W Charging".into();
        let c = import(&out, &src, &e).unwrap();
        assert_eq!(
            c,
            "receipts/2026/2026-10-03_PRO-USB-C-3.2-Gen2-Male-Cable-10Gbps.pdf"
        );
        e.description = "???".into();
        assert_eq!(stored_name(&e), "receipt");
        assert!(resolve(&out, &a).unwrap().is_file() && resolve(&out, &b).unwrap().is_file());
        assert!(resolve(&out, "../secrets.pdf").is_err());
        assert!(resolve(&out, "/etc/passwd").is_err());
        assert!(import(&out, &tmp.join("missing.pdf"), &e).is_err());
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
