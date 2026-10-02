//! PDF output via an embedded Typst compiler.
//!
//! The document is described by [`DocumentData`] (serialised to `/data.json`
//! inside a virtual file system) and laid out by a Typst template — the
//! built-in `templates/invoice.typ`, or `template.typ` in the output folder.

mod world;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use typst::text::Font;

use crate::domain::{CreditNote, Invoice, LineItem, Snapshot, dates};
use crate::paths;

pub const BUILTIN_TEMPLATE: &str = include_str!("../../templates/invoice.typ");

/// Everything a template needs, pre-formatted as display strings.
#[derive(Debug, Clone, Serialize)]
pub struct DocumentData {
    /// `"invoice"` or `"credit_note"`.
    pub kind: &'static str,
    /// `"INVOICE"` or `"CREDIT NOTE"`.
    pub title: &'static str,
    pub number: String,
    pub business: Party,
    /// `"Bill to"` or `"Credit to"`.
    pub client_heading: &'static str,
    pub client: Party,
    /// Label/value pairs shown beside the client: dates and references.
    pub meta: Vec<(String, String)>,
    pub reason: String,
    pub lines: Vec<LineData>,
    pub total_label: &'static str,
    pub total: String,
    pub payment: Option<PaymentData>,
    pub notes: String,
    /// Empty unless the client is a business with the notice switched on.
    pub late_payment_notice: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Party {
    pub name: String,
    pub contact: String,
    /// Address lines, ending with the country for clients.
    pub address: Vec<String>,
    /// e.g. `VAT ID: DE123456789`; empty if none.
    pub tax_id: String,
    pub email: String,
    pub phone: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LineData {
    pub description: String,
    pub quantity: String,
    pub rate: String,
    pub amount: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaymentData {
    pub account_name: String,
    pub sort_code: String,
    pub account_number: String,
    pub reference: String,
    pub terms: String,
}

pub const LATE_PAYMENT_NOTICE: &str = "This is a business-to-business transaction. If payment is \
not received by the due date, statutory interest may be charged at 8% above the Bank of England \
base rate, together with fixed debt recovery costs, under the Late Payment of Commercial Debts \
(Interest) Act 1998.";

impl DocumentData {
    pub fn for_invoice(inv: &Invoice) -> Result<Self> {
        let snap = inv
            .snapshot
            .as_ref()
            .ok_or_else(|| anyhow!("only issued invoices can be rendered"))?;
        let number = inv.number.clone().unwrap_or_default();
        let b = &snap.business;
        let terms = match b.payment_terms_days {
            0 => format!(
                "Payment is due on receipt, by {}.",
                dates::long(inv.due_date)
            ),
            n => format!(
                "Payment is due within {n} days of the invoice date, by {}.",
                dates::long(inv.due_date)
            ),
        };
        Ok(DocumentData {
            kind: "invoice",
            title: "INVOICE",
            number: number.clone(),
            business: business_party(snap),
            client_heading: "Bill to",
            client: client_party(snap),
            meta: vec![
                ("Invoice date".into(), dates::long(inv.issue_date)),
                (
                    supply_label(inv.supply_end.is_some()),
                    dates::supply_period(inv.supply_start, inv.supply_end),
                ),
                ("Due date".into(), dates::long(inv.due_date)),
            ],
            reason: String::new(),
            lines: inv.lines.iter().map(line_data).collect(),
            total_label: "Total due",
            total: inv.total().to_string(),
            payment: Some(PaymentData {
                account_name: b.bank.account_name.clone(),
                sort_code: b.bank.sort_code.clone(),
                account_number: b.bank.account_number.clone(),
                reference: number,
                terms,
            }),
            notes: inv.notes.trim().to_string(),
            late_payment_notice: late_payment_notice(snap),
        })
    }

    pub fn for_credit_note(note: &CreditNote, invoice: &Invoice) -> Self {
        let snap = &note.snapshot;
        DocumentData {
            kind: "credit_note",
            title: "CREDIT NOTE",
            number: note.number.clone(),
            business: business_party(snap),
            client_heading: "Credit to",
            client: client_party(snap),
            meta: vec![
                ("Credit note date".into(), dates::long(note.date)),
                (
                    "Against invoice".into(),
                    invoice.number.clone().unwrap_or_default(),
                ),
                ("Invoice date".into(), dates::long(invoice.issue_date)),
                (
                    supply_label(invoice.supply_end.is_some()),
                    dates::supply_period(invoice.supply_start, invoice.supply_end),
                ),
            ],
            reason: note.reason.clone(),
            lines: note.lines.iter().map(line_data).collect(),
            total_label: "Total credited",
            total: note.total().to_string(),
            payment: None,
            notes: String::new(),
            late_payment_notice: String::new(),
        }
    }
}

fn supply_label(is_period: bool) -> String {
    if is_period {
        "Supply period"
    } else {
        "Supply date"
    }
    .into()
}

fn business_party(snap: &Snapshot) -> Party {
    let b = &snap.business;
    Party {
        name: b.name.clone(),
        contact: String::new(),
        address: b.address.lines(),
        tax_id: String::new(),
        email: b.email.clone(),
        phone: b.phone.clone(),
    }
}

fn client_party(snap: &Snapshot) -> Party {
    let c = &snap.client;
    Party {
        name: c.name.clone(),
        contact: c.contact_name.clone(),
        address: c.address.lines(),
        tax_id: match (c.tax_id_label.trim(), c.tax_id.trim()) {
            (_, "") => String::new(),
            ("", id) => id.to_string(),
            (label, id) => format!("{label}: {id}"),
        },
        email: c.email.clone(),
        phone: String::new(),
    }
}

fn late_payment_notice(snap: &Snapshot) -> String {
    if snap.client.is_business && snap.client.late_payment_notice {
        LATE_PAYMENT_NOTICE.into()
    } else {
        String::new()
    }
}

fn line_data(l: &LineItem) -> LineData {
    let unit = l.kind.unit(l.quantity);
    LineData {
        description: l.description.clone(),
        quantity: if unit.is_empty() {
            l.quantity.to_string()
        } else {
            format!("{} {unit}", l.quantity)
        },
        rate: match l.kind {
            crate::domain::LineKind::Hours => format!("{}/hr", l.unit_price),
            crate::domain::LineKind::Days => format!("{}/day", l.unit_price),
            crate::domain::LineKind::Fixed => l.unit_price.to_string(),
        },
        amount: l.amount().to_string(),
    }
}

/// Compiles documents to PDF. Font loading is the slow part, so build one
/// `Renderer` and reuse it.
pub struct Renderer {
    fonts: Vec<Font>,
    template: String,
    template_is_override: bool,
}

impl Renderer {
    /// Loads the bundled fonts plus any in `<output>/fonts/`, and the template
    /// override `<output>/template.typ` if present.
    pub fn new(output_dir: &Path) -> Result<Self> {
        let mut fonts = world::embedded_fonts();
        let font_dir = paths::fonts_dir(output_dir);
        if font_dir.is_dir() {
            for entry in std::fs::read_dir(&font_dir)? {
                let path = entry?.path();
                let is_font = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    matches!(
                        e.to_ascii_lowercase().as_str(),
                        "ttf" | "otf" | "ttc" | "otc"
                    )
                });
                if is_font {
                    let data = std::fs::read(&path)
                        .with_context(|| format!("reading {}", path.display()))?;
                    fonts.extend(world::fonts_from(data));
                }
            }
        }
        let override_path = paths::template_override(output_dir);
        let (template, template_is_override) = if override_path.is_file() {
            let src = std::fs::read_to_string(&override_path)
                .with_context(|| format!("reading {}", override_path.display()))?;
            (src, true)
        } else {
            (BUILTIN_TEMPLATE.to_string(), false)
        };
        Ok(Renderer {
            fonts,
            template,
            template_is_override,
        })
    }

    /// A renderer using only built-in fonts and the built-in template.
    pub fn builtin() -> Self {
        Renderer {
            fonts: world::embedded_fonts(),
            template: BUILTIN_TEMPLATE.into(),
            template_is_override: false,
        }
    }

    pub fn uses_template_override(&self) -> bool {
        self.template_is_override
    }

    pub fn render(&self, data: &DocumentData) -> Result<Vec<u8>> {
        let json = serde_json::to_vec(data)?;
        world::compile_pdf(&self.template, json, &self.fonts).map_err(|e| {
            anyhow!(
                "{e}{}",
                if self.template_is_override {
                    " (in your template.typ override)"
                } else {
                    ""
                }
            )
        })
    }

    /// Renders an issued invoice to `<output>/<year>/<number>_<client>.pdf`.
    pub fn write_invoice(&self, output_dir: &Path, inv: &Invoice) -> Result<PathBuf> {
        let path = paths::invoice_pdf(output_dir, inv);
        write_atomically(&path, &self.render(&DocumentData::for_invoice(inv)?)?)?;
        Ok(path)
    }

    pub fn write_credit_note(
        &self,
        output_dir: &Path,
        note: &CreditNote,
        invoice: &Invoice,
    ) -> Result<PathBuf> {
        let path = paths::credit_note_pdf(output_dir, note);
        write_atomically(
            &path,
            &self.render(&DocumentData::for_credit_note(note, invoice))?,
        )?;
        Ok(path)
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("PDF path has no parent folder")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = path.with_extension("pdf.partial");
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// A realistic sample invoice for previewing templates (`tally sample-pdf`).
pub fn sample_invoice() -> Invoice {
    use crate::domain::*;
    use chrono::{Datelike, Days};
    let issue = dates::today();
    let start = issue.with_day(1).unwrap_or(issue);
    let business = Business {
        name: "Your Name".into(),
        address: Address {
            line1: "1 Example Street".into(),
            line2: String::new(),
            town: "Bristol".into(),
            postcode: "BS1 1AA".into(),
            country: String::new(),
        },
        email: "you@example.com".into(),
        phone: "07700 900000".into(),
        bank: BankDetails {
            account_name: "Your Name".into(),
            sort_code: "00-00-00".into(),
            account_number: "00000000".into(),
        },
        payment_terms_days: 30,
        default_notes: String::new(),
        invoice_pattern: "{YEAR}-{SEQ:4}".into(),
        credit_note_pattern: "CN-{YEAR}-{SEQ:3}".into(),
        output_dir: String::new(),
        backups_to_keep: 10,
    };
    let mut client = Client::new_empty();
    client.name = "Example Client Ltd".into();
    client.contact_name = "Accounts Payable".into();
    client.address = Address {
        line1: "2 Market Square".into(),
        line2: "Suite 4".into(),
        town: "Bath".into(),
        postcode: "BA1 1AA".into(),
        country: DEFAULT_COUNTRY.into(),
    };
    client.label = "EXC".into();
    client.tax_id_label = "VAT number".into();
    client.tax_id = "GB123456789".into();
    client.late_payment_notice = true;
    Invoice {
        id: 0,
        number: Some(format!("EXC-{}-0001", issue.format("%Y"))),
        client_id: 0,
        issue_date: issue,
        due_date: issue + Days::new(30),
        supply_start: start,
        supply_end: Some(issue),
        notes: "Thank you for your business.".into(),
        issued_at: Some(issue.and_hms_opt(9, 0, 0).expect("valid time")),
        sent_on: None,
        snapshot: Some(Snapshot { business, client }),
        lines: vec![
            LineItem {
                kind: LineKind::Days,
                description: "Consultancy — discovery workshop".into(),
                quantity: Quantity::from_hundredths(200),
                unit_price: Money::from_pence(45_000),
            },
            LineItem {
                kind: LineKind::Hours,
                description: "Development and testing".into(),
                quantity: Quantity::from_hundredths(1_250),
                unit_price: Money::from_pence(6_000),
            },
            LineItem {
                kind: LineKind::Fixed,
                description: "Domain and hosting setup".into(),
                quantity: Quantity::ONE,
                unit_price: Money::from_pence(15_000),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_pdf_with_the_builtin_template() {
        let inv = sample_invoice();
        let pdf = Renderer::builtin()
            .render(&DocumentData::for_invoice(&inv).unwrap())
            .unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.len() > 1_000);
    }

    #[test]
    fn renders_a_credit_note() {
        let inv = sample_invoice();
        let note = CreditNote {
            id: 1,
            number: "CN-2026-001".into(),
            invoice_id: 0,
            date: inv.issue_date,
            reason: "Hosting no longer required".into(),
            lines: vec![inv.lines[2].clone()],
            snapshot: inv.snapshot.clone().unwrap(),
        };
        let data = DocumentData::for_credit_note(&note, &inv);
        assert!(data.payment.is_none());
        assert!(
            Renderer::builtin()
                .render(&data)
                .unwrap()
                .starts_with(b"%PDF-")
        );
    }

    #[test]
    fn template_errors_are_reported() {
        let r = Renderer {
            fonts: world::embedded_fonts(),
            template: "#let x = (".into(),
            template_is_override: true,
        };
        let err = r
            .render(&DocumentData::for_invoice(&sample_invoice()).unwrap())
            .unwrap_err();
        assert!(err.to_string().contains("template.typ"), "{err}");
    }

    #[test]
    fn late_payment_notice_only_for_flagged_business_clients() {
        let mut inv = sample_invoice();
        assert!(
            !DocumentData::for_invoice(&inv)
                .unwrap()
                .late_payment_notice
                .is_empty()
        );
        inv.snapshot.as_mut().unwrap().client.is_business = false;
        assert!(
            DocumentData::for_invoice(&inv)
                .unwrap()
                .late_payment_notice
                .is_empty()
        );
    }
}
