//! Your business details and settings: the first-run wizard and the Settings tab.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::form::{Form, Outcome};
use super::{App, Screen, theme};
use crate::domain::numbering::NumberPattern;
use crate::domain::{Address, BankDetails, Business};
use crate::{backup, paths};

pub fn form(b: Option<&Business>) -> Form {
    let get =
        |f: fn(&Business) -> String, default: &str| b.map(f).unwrap_or_else(|| default.to_string());
    Form::new()
        .text("name", "Your name", get(|b| b.name.clone(), ""))
        .required()
        .hint("As it should appear on invoices (your own name, as a sole trader)")
        .text("line1", "Address line 1", get(|b| b.address.line1.clone(), ""))
        .required()
        .text("line2", "Address line 2", get(|b| b.address.line2.clone(), ""))
        .text("town", "Town / city", get(|b| b.address.town.clone(), ""))
        .required()
        .text("postcode", "Postcode", get(|b| b.address.postcode.clone(), ""))
        .required()
        .text("email", "Email", get(|b| b.email.clone(), ""))
        .text("phone", "Phone", get(|b| b.phone.clone(), ""))
        .text("account_name", "Bank account name", get(|b| b.bank.account_name.clone(), ""))
        .required()
        .text("sort_code", "Sort code", get(|b| b.bank.sort_code.clone(), ""))
        .required()
        .hint("e.g. 12-34-56")
        .text("account_number", "Account number", get(|b| b.bank.account_number.clone(), ""))
        .required()
        .text("terms", "Payment terms (days)", get(|b| b.payment_terms_days.to_string(), "30"))
        .required()
        .hint("Default days between invoice date and due date (0 = due on receipt)")
        .text("notes", "Default invoice notes", get(|b| b.default_notes.clone(), "Thank you for your business."))
        .hint("Pre-filled on new invoices")
        .text("invoice_pattern", "Invoice number pattern", get(|b| b.invoice_pattern.clone(), "{YEAR}-{SEQ:4}"))
        .required()
        .hint("Tokens: {LABEL} (client label) {YEAR} {YY} {MONTH} {SEQ:n}, e.g. {LABEL}-{YEAR}-{SEQ:4}")
        .text(
            "credit_pattern",
            "Credit note pattern",
            get(|b| b.credit_note_pattern.clone(), "CN-{YEAR}-{SEQ:3}"),
        )
        .required()
        .hint("Same tokens. {YEAR}/{YY} restart numbering each 1 January; {LABEL} numbers each client separately")
        .text("output_dir", "Output folder", get(|b| b.output_dir.clone(), "~/Documents/Invoices"))
        .required()
        .hint("Your synced folder for PDFs, backups, exports and template.typ (~ allowed)")
        .text("backups", "Backups to keep", get(|b| b.backups_to_keep.to_string(), "10"))
        .required()
        .hint("A backup is written to <output folder>/backups each time you quit")
}

/// Validates the form into a `Business`, reporting the first bad field.
fn parse(form: &mut Form) -> Option<Business> {
    if let Err(msg) = form.check_required() {
        form.error = Some(msg);
        return None;
    }
    let sort_digits: String = form
        .value("sort_code")
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    if sort_digits.len() != 6
        || form
            .value("sort_code")
            .chars()
            .any(|c| !c.is_ascii_digit() && c != '-' && c != ' ')
    {
        form.fail("sort_code", "Sort code must be 6 digits, e.g. 12-34-56");
        return None;
    }
    let account = form.value("account_number").replace(' ', "");
    if !(6..=10).contains(&account.len()) || !account.chars().all(|c| c.is_ascii_digit()) {
        form.fail(
            "account_number",
            "Account number must be digits only (usually 8)",
        );
        return None;
    }
    let Ok(terms) = form.value("terms").parse::<u32>() else {
        form.fail("terms", "Payment terms must be a whole number of days");
        return None;
    };
    for key in ["invoice_pattern", "credit_pattern"] {
        if let Err(e) = NumberPattern::parse(&form.value(key)) {
            form.fail(key, e);
            return None;
        }
    }
    if form.value("invoice_pattern") == form.value("credit_pattern") {
        form.fail(
            "credit_pattern",
            "Credit notes need a different pattern from invoices",
        );
        return None;
    }
    if !paths::expand_home(&form.value("output_dir")).is_absolute() {
        form.fail(
            "output_dir",
            "Use a full path, e.g. ~/Documents/Invoices or /Users/you/Invoices",
        );
        return None;
    }
    let backups = match form.value("backups").parse::<u32>() {
        Ok(n) if n >= 1 => n,
        _ => {
            form.fail("backups", "Keep at least 1 backup");
            return None;
        }
    };
    Some(Business {
        name: form.value("name"),
        address: Address {
            line1: form.value("line1"),
            line2: form.value("line2"),
            town: form.value("town"),
            postcode: form.value("postcode").to_uppercase(),
            country: String::new(),
        },
        email: form.value("email"),
        phone: form.value("phone"),
        bank: BankDetails {
            account_name: form.value("account_name"),
            sort_code: format!(
                "{}-{}-{}",
                &sort_digits[0..2],
                &sort_digits[2..4],
                &sort_digits[4..6]
            ),
            account_number: account,
        },
        payment_terms_days: terms,
        default_notes: form.value("notes"),
        invoice_pattern: form.value("invoice_pattern"),
        credit_note_pattern: form.value("credit_pattern"),
        output_dir: form.value("output_dir"),
        backups_to_keep: backups,
    })
}

impl App {
    pub(super) fn key_business(&mut self, key: KeyEvent) -> Result<()> {
        let is_wizard = matches!(self.screen, Screen::Wizard(_));
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if !is_wizard && ctrl && key.code == KeyCode::Char('b') {
            let b = self.db.require_business()?;
            let path = backup::run(
                &self.db,
                &paths::expand_home(&b.output_dir),
                b.backups_to_keep,
            )?;
            self.success(format!("Backed up to {}", paths::display(&path)));
            return Ok(());
        }
        if !is_wizard && ctrl && key.code == KeyCode::Char('t') {
            self.reset_renderer();
            let uses_override = self.renderer()?.uses_template_override();
            self.success(if uses_override {
                "Reloaded fonts and your template.typ override"
            } else {
                "Reloaded fonts (using the built-in template)"
            });
            return Ok(());
        }
        let (Screen::Wizard(form) | Screen::Settings(form)) = &mut self.screen else {
            return Ok(());
        };
        match form.handle_key(key) {
            Outcome::Cancel => {
                if is_wizard {
                    self.quit = true;
                } else {
                    self.go(Screen::Dashboard);
                }
            }
            Outcome::Submit => {
                let Some(business) = parse(form) else {
                    return Ok(());
                };
                let dir = paths::expand_home(&business.output_dir);
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    form.fail(
                        "output_dir",
                        format!("Can't create {}: {e}", paths::display(&dir)),
                    );
                    return Ok(());
                }
                if let Err(e) = self.db.save_business(&business) {
                    form.error = Some(format!("{e:#}"));
                    return Ok(());
                }
                self.business = Some(business);
                self.reset_renderer();
                if is_wizard {
                    self.go(Screen::Clients);
                    self.success("You're set up. Add your first client with n.");
                } else {
                    self.success("Settings saved. Already-issued invoices keep the details they were issued with.");
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn draw_business(&mut self, f: &mut Frame, area: Rect) {
        let is_wizard = matches!(self.screen, Screen::Wizard(_));
        let intro: Vec<Line> = if is_wizard {
            vec![
                Line::styled("Welcome to tally", theme::title()),
                Line::from(
                    "Enter your details once; they appear on every invoice. You can change them later in Settings.",
                ),
                Line::from(vec![
                    Span::raw("Fields marked "),
                    Span::styled("*", theme::muted()),
                    Span::raw(" are required. Ctrl+S saves."),
                ]),
            ]
        } else {
            let db_path = paths::database_path()
                .map(|p| paths::display(&p))
                .unwrap_or_default();
            let override_path = paths::template_override(&self.output_dir());
            let template = if override_path.is_file() {
                format!("{} (override)", paths::display(&override_path))
            } else {
                "built-in".to_string()
            };
            vec![
                Line::styled("Your business details and settings", theme::title()),
                Line::from(vec![
                    Span::styled("Database: ", theme::muted()),
                    Span::raw(db_path),
                ]),
                Line::from(vec![
                    Span::styled("Template: ", theme::muted()),
                    Span::raw(template),
                ]),
            ]
        };
        let [intro_area, _, form_area] = Layout::vertical([
            Constraint::Length(intro.len() as u16),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .areas(area);
        f.render_widget(Paragraph::new(intro).wrap(Wrap { trim: true }), intro_area);
        let active = self.modal.is_none();
        if let Screen::Wizard(form) | Screen::Settings(form) = &self.screen {
            let width = form_area.width.min(100);
            let height = form_area.height.min(form.height());
            form.render(
                f,
                Rect {
                    width,
                    height,
                    ..form_area
                },
                active,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled() -> Form {
        let mut f = form(None);
        for (k, v) in [
            ("name", "Nick"),
            ("line1", "1 High St"),
            ("town", "Bristol"),
            ("postcode", "bs1 1aa"),
            ("account_name", "N Reiners"),
            ("sort_code", "123456"),
            ("account_number", "12345678"),
        ] {
            f.set_value(k, v);
        }
        f
    }

    #[test]
    fn parses_and_normalises() {
        let b = parse(&mut filled()).unwrap();
        assert_eq!(b.bank.sort_code, "12-34-56");
        assert_eq!(b.address.postcode, "BS1 1AA");
        assert_eq!(b.payment_terms_days, 30);
    }

    #[test]
    fn rejects_bad_values() {
        let mut f = filled();
        f.set_value("sort_code", "12-34-5");
        assert!(parse(&mut f).is_none());
        assert_eq!(f.focused_key(), "sort_code");

        let mut f = filled();
        f.set_value("credit_pattern", "{YEAR}-{SEQ:4}");
        assert!(parse(&mut f).is_none());

        let mut f = filled();
        f.set_value("output_dir", "{LABEL}-{YEAR}");
        assert!(parse(&mut f).is_none());
        assert_eq!(f.focused_key(), "output_dir");

        let mut f = filled();
        f.set_value("name", "");
        assert!(parse(&mut f).is_none());
        assert_eq!(f.focused_key(), "name");
    }
}
