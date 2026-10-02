//! Pop-up forms and confirmations.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::text::Line;
use ratatui::widgets::Clear;

use super::form::{Form, Outcome};
use super::{App, Modal, Screen, centered, popup_block, render_popup_text};
use crate::db::TxKind;
use crate::domain::{Address, Client, LineItem, LineKind, Money, Quantity, Transaction, dates};

/// What a pop-up form is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    NewClient,
    EditClient(i64),
    Payment(i64),
    Refund(i64),
    CreditNote(i64),
    MarkSent(i64),
    /// A line in the editor: `None` adds, `Some(i)` replaces line `i`.
    Line(Option<usize>),
}

/// What a confirmation does when accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Issue(i64),
    DeleteDraft(i64),
    Regenerate(i64),
    DeleteTx(TxKind, i64),
    DiscardEditor,
    Quit,
}

const METHODS: [&str; 5] = ["Bank transfer", "Card", "Cash", "Cheque", "Other"];

pub fn client_form(c: Option<&Client>) -> Form {
    let c = c.cloned().unwrap_or_else(Client::new_empty);
    let kind_index = LineKind::ALL
        .iter()
        .position(|k| *k == c.default_rate_kind)
        .unwrap_or(0);
    Form::new()
        .text("name", "Name", c.name)
        .required()
        .hint("Company or person, as it should appear on invoices")
        .text("contact", "Contact / FAO", c.contact_name)
        .hint("Optional, e.g. Accounts Payable")
        .text("email", "Email", c.email)
        .text("line1", "Address line 1", c.address.line1)
        .text("line2", "Address line 2", c.address.line2)
        .text("town", "Town / city", c.address.town)
        .text("postcode", "Postcode", c.address.postcode)
        .toggle("business", "Business client", c.is_business)
        .hint("Statutory late-payment interest only applies to business clients")
        .toggle("late", "Late-payment notice", c.late_payment_notice)
        .hint(
            "Print the statutory interest notice on this client's invoices (business clients only)",
        )
        .text(
            "rate",
            "Default rate (£)",
            c.default_rate
                .map(|m| m.to_decimal_string())
                .unwrap_or_default(),
        )
        .hint("Optional; pre-filled when adding lines")
        .choice(
            "rate_kind",
            "Default rate is",
            vec!["per hour".into(), "per day".into(), "fixed price".into()],
            kind_index,
        )
}

pub fn line_form(
    index: Option<usize>,
    existing: Option<&LineItem>,
    client: Option<&Client>,
) -> Modal {
    let kind = existing
        .map(|l| l.kind)
        .or(client.map(|c| c.default_rate_kind))
        .unwrap_or(LineKind::Hours);
    let kind_index = LineKind::ALL.iter().position(|k| *k == kind).unwrap_or(0);
    let rate = existing
        .map(|l| l.unit_price.to_decimal_string())
        .or_else(|| {
            client
                .and_then(|c| c.default_rate)
                .map(|m| m.to_decimal_string())
        })
        .unwrap_or_default();
    let form = Form::new()
        .choice(
            "kind",
            "Type",
            LineKind::ALL
                .iter()
                .map(|k| k.label().to_string())
                .collect(),
            kind_index,
        )
        .text(
            "description",
            "Description",
            existing.map(|l| l.description.clone()).unwrap_or_default(),
        )
        .required()
        .text(
            "quantity",
            "Quantity",
            existing
                .map(|l| l.quantity.to_string())
                .unwrap_or_else(|| "1".into()),
        )
        .required()
        .hint("Hours, days or units; up to 2 decimals (e.g. 7.5)")
        .text("rate", "Rate (£)", rate)
        .required()
        .hint("Per hour, per day or per unit");
    let title = if index.is_some() {
        "Edit line"
    } else {
        "Add line"
    };
    Modal::Form {
        title: title.into(),
        form,
        purpose: Purpose::Line(index),
    }
}

fn money_field(form: &mut Form, key: &str, allow_zero: bool) -> Option<Money> {
    match Money::parse(&form.value(key)) {
        Ok(m) if m.is_positive() || (allow_zero && m.is_zero()) => Some(m),
        Ok(_) => {
            form.fail(key, "Enter an amount greater than £0.00");
            None
        }
        Err(e) => {
            form.fail(key, e);
            None
        }
    }
}

fn date_field(form: &mut Form, key: &str) -> Option<chrono::NaiveDate> {
    match dates::parse_date(&form.value(key)) {
        Ok(d) => Some(d),
        Err(e) => {
            form.fail(key, e);
            None
        }
    }
}

impl App {
    /// Opens a pop-up form. `amount` pre-fills the amount field where relevant.
    pub(super) fn open_form(&mut self, purpose: Purpose, amount: Option<Money>) {
        let today = dates::today().to_string();
        let amount = amount.map(|m| m.to_decimal_string()).unwrap_or_default();
        let number = |app: &App, id: i64| {
            app.rows
                .iter()
                .find(|r| r.invoice.id == id)
                .map(|r| r.invoice.display_number())
                .unwrap_or_default()
        };
        let (title, form) = match purpose {
            Purpose::NewClient => ("New client".to_string(), client_form(None)),
            Purpose::EditClient(id) => {
                let client = self.clients.iter().find(|c| c.id == id);
                ("Edit client".to_string(), client_form(client))
            }
            Purpose::Payment(id) => (
                format!("Record payment on {}", number(self, id)),
                Form::new()
                    .text("date", "Date received", today)
                    .required()
                    .text("amount", "Amount (£)", amount)
                    .required()
                    .hint("Pre-filled with the balance. Overpayments are allowed and can be refunded")
                    .choice("method", "Method", METHODS.iter().map(|s| s.to_string()).collect(), 0)
                    .text("reference", "Reference", "")
                    .hint("Optional, e.g. the bank statement reference"),
            ),
            Purpose::Refund(id) => (
                format!("Record refund on {}", number(self, id)),
                Form::new()
                    .text("date", "Date paid back", today)
                    .required()
                    .hint("Counts against income in the tax year it's paid")
                    .text("amount", "Amount (£)", amount)
                    .required()
                    .choice("method", "Method", METHODS.iter().map(|s| s.to_string()).collect(), 0)
                    .text("reference", "Reference", ""),
            ),
            Purpose::CreditNote(id) => (
                format!("Credit note against {}", number(self, id)),
                Form::new()
                    .text("date", "Date", today)
                    .required()
                    .text("reason", "Reason", "")
                    .required()
                    .hint("Printed on the credit note, e.g. 'Hosting cancelled' or 'Invoiced in error'")
                    .text("description", "Line description", format!("Credit against invoice {}", number(self, id)))
                    .required()
                    .text("amount", "Amount (£)", amount)
                    .required()
                    .hint("Pre-filled with the full uncredited amount"),
            ),
            Purpose::MarkSent(id) => {
                let current = self
                    .rows
                    .iter()
                    .find(|r| r.invoice.id == id)
                    .and_then(|r| r.invoice.sent_on)
                    .map(|d| d.to_string())
                    .unwrap_or(today);
                (
                    format!("Mark {} as sent", number(self, id)),
                    Form::new()
                        .text("date", "Date sent", current)
                        .hint("Clear the date and save to mark it as not sent"),
                )
            }
            Purpose::Line(_) => unreachable!("line forms are opened with line_form"),
        };
        self.modal = Some(Modal::Form {
            title,
            form,
            purpose,
        });
    }

    pub(super) fn key_modal(&mut self, key: KeyEvent) -> Result<()> {
        match self.modal.take() {
            Some(Modal::Confirm { message, action }) => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => self.run_action(action),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Ok(()),
                _ => {
                    self.modal = Some(Modal::Confirm { message, action });
                    Ok(())
                }
            },
            Some(Modal::Form {
                title,
                mut form,
                purpose,
            }) => match form.handle_key(key) {
                Outcome::Cancel => Ok(()),
                Outcome::Submit => {
                    if let Err(msg) = form.check_required() {
                        form.error = Some(msg);
                        self.modal = Some(Modal::Form {
                            title,
                            form,
                            purpose,
                        });
                        return Ok(());
                    }
                    match self.submit(purpose, &mut form) {
                        // Validation failed inside `submit`; keep the form open.
                        Ok(false) => {
                            self.modal = Some(Modal::Form {
                                title,
                                form,
                                purpose,
                            })
                        }
                        Ok(true) => {}
                        Err(e) => {
                            form.error = Some(format!("{e:#}"));
                            self.modal = Some(Modal::Form {
                                title,
                                form,
                                purpose,
                            });
                        }
                    }
                    Ok(())
                }
                _ => {
                    self.modal = Some(Modal::Form {
                        title,
                        form,
                        purpose,
                    });
                    Ok(())
                }
            },
            None => Ok(()),
        }
    }

    /// Applies a submitted form. Returns `Ok(false)` if a field was invalid.
    fn submit(&mut self, purpose: Purpose, form: &mut Form) -> Result<bool> {
        match purpose {
            Purpose::NewClient | Purpose::EditClient(_) => {
                let rate = match form.value("rate").as_str() {
                    "" => None,
                    _ => match money_field(form, "rate", true) {
                        Some(m) => Some(m),
                        None => return Ok(false),
                    },
                };
                let existing = match purpose {
                    Purpose::EditClient(id) => self.clients.iter().find(|c| c.id == id).cloned(),
                    _ => None,
                };
                let client = Client {
                    id: existing.as_ref().map(|c| c.id).unwrap_or(0),
                    name: form.value("name"),
                    contact_name: form.value("contact"),
                    email: form.value("email"),
                    address: Address {
                        line1: form.value("line1"),
                        line2: form.value("line2"),
                        town: form.value("town"),
                        postcode: form.value("postcode").to_uppercase(),
                    },
                    is_business: form.is_on("business"),
                    late_payment_notice: form.is_on("late"),
                    default_rate: rate,
                    default_rate_kind: LineKind::ALL[form.selected("rate_kind")],
                    archived: existing.map(|c| c.archived).unwrap_or(false),
                };
                if client.late_payment_notice && !client.is_business {
                    form.fail(
                        "late",
                        "The statutory late-payment notice only applies to business clients",
                    );
                    return Ok(false);
                }
                if client.address.lines().is_empty() {
                    form.fail(
                        "line1",
                        "Enter the client's address (required on UK invoices)",
                    );
                    return Ok(false);
                }
                if client.id == 0 {
                    let id = self.db.insert_client(&client)?;
                    self.refresh()?;
                    self.select_client(id);
                    self.success(format!("Added {}. Press i to invoice them.", client.name));
                } else {
                    self.db.update_client(&client)?;
                    self.refresh()?;
                    self.success(format!(
                        "Saved {}. Already-issued invoices keep the details they were issued with.",
                        client.name
                    ));
                }
            }
            Purpose::Payment(id) | Purpose::Refund(id) => {
                let Some(date) = date_field(form, "date") else {
                    return Ok(false);
                };
                let Some(amount) = money_field(form, "amount", false) else {
                    return Ok(false);
                };
                let inv = self.db.invoice(id)?;
                if date < inv.issue_date {
                    form.fail("date", "That's before the invoice date");
                    return Ok(false);
                }
                let tx = Transaction {
                    id: 0,
                    invoice_id: id,
                    date,
                    amount,
                    method: METHODS[form.selected("method")].to_string(),
                    reference: form.value("reference"),
                };
                if matches!(purpose, Purpose::Payment(_)) {
                    self.db.add_payment(&tx)?;
                    self.refresh()?;
                    let balance = self.db.totals(id)?.balance();
                    if balance.is_negative() {
                        self.info(format!("Payment recorded. The client has overpaid by {} — record a refund with r.", -balance));
                    } else if balance.is_zero() {
                        self.success(format!(
                            "Payment recorded. {} is paid in full.",
                            inv.display_number()
                        ));
                    } else {
                        self.success(format!("Payment recorded. {balance} still owed."));
                    }
                } else {
                    self.db.add_refund(&tx)?;
                    self.refresh()?;
                    self.success(format!("Refund of {amount} recorded."));
                }
            }
            Purpose::CreditNote(id) => {
                let Some(date) = date_field(form, "date") else {
                    return Ok(false);
                };
                let Some(amount) = money_field(form, "amount", false) else {
                    return Ok(false);
                };
                let line = LineItem {
                    kind: LineKind::Fixed,
                    description: form.value("description"),
                    quantity: Quantity::ONE,
                    unit_price: amount,
                };
                let out = self.output_dir();
                self.renderer()?;
                let renderer = self.renderer.as_ref().expect("initialised above");
                let mut written = std::path::PathBuf::new();
                let note = self.db.create_credit_note(
                    id,
                    date,
                    &form.value("reason"),
                    vec![line],
                    |note, inv| {
                        written = renderer.write_credit_note(&out, note, inv)?;
                        Ok(())
                    },
                )?;
                self.refresh()?;
                let balance = self.db.totals(id)?.balance();
                let follow_up = if balance.is_negative() {
                    format!(
                        " {} is owed back — record it with r once refunded.",
                        -balance
                    )
                } else {
                    String::new()
                };
                self.success(format!(
                    "Credit note {} → {}.{follow_up}",
                    note.number,
                    crate::paths::display(&written)
                ));
            }
            Purpose::MarkSent(id) => {
                let sent = match form.value("date").as_str() {
                    "" => None,
                    _ => match date_field(form, "date") {
                        Some(d) => Some(d),
                        None => return Ok(false),
                    },
                };
                self.db.set_sent(id, sent)?;
                self.refresh()?;
                self.success(match sent {
                    Some(d) => format!("Marked as sent on {}", dates::long(d)),
                    None => "Marked as not sent".to_string(),
                });
            }
            Purpose::Line(index) => {
                let quantity = match Quantity::parse(&form.value("quantity")) {
                    Ok(q) => q,
                    Err(e) => {
                        form.fail("quantity", e);
                        return Ok(false);
                    }
                };
                let Some(unit_price) = money_field(form, "rate", false) else {
                    return Ok(false);
                };
                let line = LineItem {
                    kind: LineKind::ALL[form.selected("kind")],
                    description: form.value("description"),
                    quantity,
                    unit_price,
                };
                self.add_or_replace_line(index, line);
            }
        }
        Ok(true)
    }

    fn run_action(&mut self, action: Action) -> Result<()> {
        match action {
            Action::Quit => self.quit = true,
            Action::DiscardEditor => self.close_editor(),
            Action::Issue(id) => self.issue(id)?,
            Action::DeleteDraft(id) => {
                self.db.delete_draft(id)?;
                self.go(Screen::Invoices);
                self.success("Draft deleted");
            }
            Action::Regenerate(id) => {
                let path = self.regenerate(id)?;
                self.success(format!("Regenerated {}", crate::paths::display(&path)));
            }
            Action::DeleteTx(kind, tx_id) => {
                self.db.delete_transaction(kind, tx_id)?;
                self.refresh()?;
                self.success("Deleted");
            }
        }
        Ok(())
    }

    pub(super) fn modal_help(&self) -> Vec<(&'static str, &'static str)> {
        match &self.modal {
            Some(Modal::Confirm { .. }) => vec![("y/Enter", "yes"), ("n/Esc", "no")],
            _ => vec![
                ("↑↓/Tab", "move"),
                ("Enter", "next/save"),
                ("Ctrl+S", "save"),
                ("Esc", "cancel"),
            ],
        }
    }

    pub(super) fn draw_modal(&self, f: &mut Frame) {
        match &self.modal {
            Some(Modal::Confirm { message, .. }) => {
                let mut lines: Vec<Line> =
                    message.lines().map(|l| Line::from(l.to_string())).collect();
                lines.push(Line::default());
                lines.push(super::help_line(&[("y", "yes"), ("n", "no")]));
                render_popup_text(f, "Confirm", lines, 64);
            }
            Some(Modal::Form { title, form, .. }) => {
                let label_width = form
                    .fields
                    .iter()
                    .map(|f| f.label.len())
                    .max()
                    .unwrap_or(10) as u16;
                let width = (label_width + 50).max(60);
                let area = centered(f.area(), width, form.height() + 3);
                f.render_widget(Clear, area);
                let block = popup_block(title).padding(ratatui::widgets::Padding::new(1, 1, 1, 0));
                let inner = block.inner(area);
                f.render_widget(block, area);
                form.render(f, inner, true);
            }
            None => {}
        }
    }
}
