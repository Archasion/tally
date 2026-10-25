//! Pop-up forms and confirmations.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::text::Line;
use ratatui::widgets::Clear;

use super::form::{Form, Outcome};
use super::{App, Modal, Screen, centered, popup_block, render_popup_text};
use crate::db::TxKind;
use crate::domain::dates::TaxYear;
use crate::domain::expense::{BillingPeriod, Expense, ExpenseCategory};
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
    NewExpense,
    EditExpense(i64),
    /// A new expense for the next payment of a recurring one.
    NextPayment(i64),
    /// Pick a rebillable expense to add to the draft in the editor.
    Reimbursement,
    /// Regenerate an issued invoice's PDF, keeping or updating its details.
    Regenerate(i64),
    /// Open or email a credit note: (invoice id, credit note id).
    CreditNoteActions(i64, i64),
    /// Income tax + Class 4 for a tax year, from an HMRC statement.
    PriorBill(TaxYear),
}

/// What a confirmation does when accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Issue(i64),
    DeleteDraft(i64),
    /// Update an issued invoice to the current details, then regenerate it.
    UpdateDetails(i64),
    /// After opening its email in Mail: record the invoice as sent today.
    MarkSentToday(i64),
    DeleteTx(TxKind, i64),
    DeleteExpense(i64),
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
        .text("label", "Label", c.label)
        .hint("For {LABEL} in numbers: LGT → LGT-2026-0001. A–Z/0–9, locked once used")
        .text("contact", "Contact / FAO", c.contact_name)
        .hint("Optional, e.g. Accounts Payable")
        .text("email", "Email", c.email)
        .text("email_name", "Name in emails", c.email_name)
        .hint("Optional, how to greet them: \"Hi Jo,\". Defaults to the contact")
        .text("line1", "Address line 1", c.address.line1)
        .text("line2", "Address line 2", c.address.line2)
        .text("line3", "Address line 3", c.address.line3)
        .text("town", "Town / city", c.address.town)
        .hint("Include any state, province or region here, e.g. Austin, TX")
        .text("postcode", "Postcode / ZIP", c.address.postcode)
        .text("country", "Country", c.address.country)
        .required()
        .hint("Printed as the last address line on every invoice")
        .text("tax_id_label", "Tax ID name", c.tax_id_label)
        .hint("Optional, e.g. VAT ID, VAT number, ABN, EIN, GST number")
        .text("tax_id", "Tax ID", c.tax_id)
        .hint("Printed under the client's address, e.g. DE123456789")
        .toggle("business", "Business client", c.is_business)
        .hint("Statutory late-payment interest only applies to business clients")
        .toggle("late", "Late-payment notice", c.late_payment_notice)
        .hint("Print the UK statutory interest notice (business clients only)")
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

/// `a`, `a and b`, `a, b and c`.
fn join_phrases(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn reimbursement_label(e: &Expense) -> String {
    let flag = if e.is_approved() {
        ""
    } else {
        "  (not approved)"
    };
    format!(
        "{}  {}  {}{flag}",
        dates::short(e.date),
        e.description,
        e.amount
    )
}

fn reimbursement_hint(e: &Expense) -> String {
    if e.is_approved() {
        e.invoice_detail()
    } else {
        "Not approved yet: record who approved it and when under Expenses (4)".into()
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
            Purpose::NewExpense | Purpose::EditExpense(_) | Purpose::NextPayment(_) => {
                let (title, e) = match purpose {
                    Purpose::EditExpense(id) => match self.db.expense(id) {
                        Ok(e) => ("Edit expense".to_string(), e),
                        Err(e) => return self.error(&e),
                    },
                    Purpose::NextPayment(id) => match self.db.expense(id).map(|e| e.next_period()) {
                        Ok(Some(e)) => (format!("Next {} payment", e.billing.label().to_lowercase()), e),
                        Ok(None) => return self.info("That's a one-off purchase. Set a billing period to repeat it"),
                        Err(e) => return self.error(&e),
                    },
                    _ => ("New expense".to_string(), Expense::new(dates::today())),
                };
                let index = ExpenseCategory::ALL.iter().position(|c| *c == e.category).unwrap_or(0);
                let billing = BillingPeriod::ALL.iter().position(|b| *b == e.billing).unwrap_or(0);
                let rebill = self.rebill_choices(e.rebill_client_id);
                let rebill_index = rebill.iter().position(|(id, _)| *id == e.rebill_client_id).unwrap_or(0);
                let amount = if e.amount.is_zero() { String::new() } else { e.amount.to_decimal_string() };
                (
                    title,
                    Form::new()
                        .text("date", "Date paid", e.date.to_string())
                        .required()
                        .hint("The date the money left your account (cash basis)")
                        .text("description", "Description", e.description)
                        .required()
                        .choice(
                            "category",
                            "Category",
                            ExpenseCategory::ALL.iter().map(|c| c.label().to_string()).collect(),
                            index,
                        )
                        .overview(
                            ExpenseCategory::ALL
                                .iter()
                                .map(|c| (c.short().to_string(), c.hint().to_string()))
                                .collect(),
                        )
                        .hint("Self Assessment category; all of them are listed on the right")
                        .text("amount", "Amount (£)", amount)
                        .required()
                        .hint("The business part only, if it was partly personal")
                        .choice(
                            "billing",
                            "Billing period",
                            BillingPeriod::ALL.iter().map(|b| b.label().to_string()).collect(),
                            billing,
                        )
                        .hint("How often you pay. For subscriptions, u on the list adds the next payment")
                        .choice("rebill", "Rebill to", rebill.into_iter().map(|(_, name)| name).collect(), rebill_index)
                        .hint("A client who reimburses this. It stays your expense; their payment is income")
                        .text("approved_by", "Approved by", e.approved_by)
                        .hint("Who agreed to it at the client, e.g. Jo Smith (Engineering Manager)")
                        .text("approved_on", "Approved on", e.approved_on.map(|d| d.to_string()).unwrap_or_default())
                        .hint("When they agreed. Needed before it can go on an invoice")
                        .text("receipt_ref", "Supplier invoice no.", e.receipt_ref)
                        .hint("The supplier's invoice or receipt number; printed on your invoice")
                        .text("receipt_file", "Receipt file", e.receipt_file)
                        .hint("Optional path, e.g. ~/Downloads/receipt.pdf. Copied into receipts/ in your output folder")
                        .text("notes", "Notes", e.notes)
                        .hint("Optional; for you only, never printed"),
                )
            }
            Purpose::Reimbursement => {
                let options = match self.reimbursement_options() {
                    Ok(o) => o,
                    Err(e) => return self.error(&e),
                };
                let Some(first) = options.first() else {
                    let client = self.editor_client_name();
                    return self.info(format!(
                        "Nothing to rebill to {client}. Add the purchase under Expenses (4) with 'Rebill to' set to them"
                    ));
                };
                let hint = reimbursement_hint(first);
                (
                    "Add reimbursement".to_string(),
                    Form::new()
                        .choice("expense", "Purchase", options.iter().map(reimbursement_label).collect(), 0)
                        .hint(&hint),
                )
            }
            Purpose::Regenerate(id) => {
                let warning = if crate::paths::template_override(&self.output_dir()).is_file() {
                    ". Your template.typ override is used either way"
                } else {
                    ""
                };
                (
                    format!("Regenerate the PDF for {}", number(self, id)),
                    Form::new()
                        .choice(
                            "how",
                            "Details",
                            vec![
                                "Keep them as issued".into(),
                                "Update to my current business and client details".into(),
                            ],
                            0,
                        )
                        .hint(&format!(
                            "The number, dates, lines and amounts never change{warning}"
                        )),
                )
            }
            Purpose::CreditNoteActions(invoice_id, note_id) => {
                let title = match self.credit_note(invoice_id, note_id) {
                    Ok(n) => format!("Credit note {}", n.number),
                    Err(e) => return self.error(&e),
                };
                (
                    title,
                    Form::new()
                        .choice(
                            "action",
                            "Do",
                            vec!["Open the PDF".into(), "Email it to the client".into()],
                            0,
                        )
                        .hint("Emailing opens a new message in Mail; nothing is sent until you press Send"),
                )
            }
            Purpose::PriorBill(year) => {
                let current = self.db.entered_bill(year).ok().flatten().map(|m| m.to_decimal_string()).unwrap_or_default();
                (
                    format!("Your {} bill (income tax + Class 4)", year.label()),
                    Form::new()
                        .text("amount", "Amount (£)", current)
                        .hint("From your HMRC tax calculation (SA302). Leave empty to use tally's estimate"),
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
            Some(Modal::EmailPreview {
                invoice_id,
                title,
                lines,
            }) => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('m') => Ok(()),
                KeyCode::Char('o') => {
                    let inv = self.db.invoice(invoice_id)?;
                    self.preview_draft(&inv)
                }
                _ => {
                    self.modal = Some(Modal::EmailPreview {
                        invoice_id,
                        title,
                        lines,
                    });
                    Ok(())
                }
            },
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
                outcome => {
                    // Keep the picker's hint in step with the selected purchase.
                    if outcome == Outcome::Changed
                        && purpose == Purpose::Reimbursement
                        && let Some(hint) = self
                            .reimbursement_options()
                            .ok()
                            .and_then(|o| o.get(form.selected("expense")).map(reimbursement_hint))
                    {
                        let focus = form.focus;
                        form.fields[focus].hint = hint;
                    }
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
                    email_name: form.value("email_name"),
                    email: form.value("email"),
                    address: Address {
                        line1: form.value("line1"),
                        line2: form.value("line2"),
                        line3: form.value("line3"),
                        town: form.value("town"),
                        postcode: form.value("postcode"),
                        country: form.value("country"),
                    },
                    label: form.value("label"),
                    tax_id_label: form.value("tax_id_label"),
                    tax_id: form.value("tax_id"),
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
                if client.address.line1.is_empty() && client.address.town.is_empty() {
                    form.fail(
                        "line1",
                        "Enter the client's address (required on UK invoices)",
                    );
                    return Ok(false);
                }
                if let Err(e) = crate::domain::numbering::normalize_label(&client.label) {
                    form.fail("label", e);
                    return Ok(false);
                }
                match (client.tax_id_label.is_empty(), client.tax_id.is_empty()) {
                    (true, false) => {
                        form.fail("tax_id_label", "Say what the tax ID is called, e.g. VAT ID");
                        return Ok(false);
                    }
                    (false, true) => {
                        form.fail("tax_id", "Enter the tax ID, or clear its name");
                        return Ok(false);
                    }
                    _ => {}
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
                let line = LineItem::new(
                    LineKind::Fixed,
                    form.value("description"),
                    Quantity::ONE,
                    amount,
                );
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
            Purpose::NewExpense | Purpose::EditExpense(_) | Purpose::NextPayment(_) => {
                let Some(date) = date_field(form, "date") else {
                    return Ok(false);
                };
                let Some(amount) = money_field(form, "amount", false) else {
                    return Ok(false);
                };
                let approved_on = match form.value("approved_on").as_str() {
                    "" => None,
                    _ => match date_field(form, "approved_on") {
                        Some(d) => Some(d),
                        None => return Ok(false),
                    },
                };
                let approved_by = form.value("approved_by");
                match (approved_by.is_empty(), approved_on.is_none()) {
                    (false, true) => {
                        form.fail("approved_on", "Enter when it was approved");
                        return Ok(false);
                    }
                    (true, false) => {
                        form.fail("approved_by", "Enter who approved it");
                        return Ok(false);
                    }
                    _ => {}
                }
                let existing = match purpose {
                    Purpose::EditExpense(id) => Some(self.db.expense(id)?),
                    _ => None,
                };
                let stored_receipt = existing
                    .as_ref()
                    .map(|e| e.receipt_file.clone())
                    .unwrap_or_default();
                // The client the form was opened with, so the choices match.
                let opened_with = match purpose {
                    Purpose::EditExpense(id) | Purpose::NextPayment(id) => {
                        self.db.expense(id)?.rebill_client_id
                    }
                    _ => None,
                };
                let rebill_client_id = self
                    .rebill_choices(opened_with)
                    .get(form.selected("rebill"))
                    .and_then(|(id, _)| *id);
                let mut expense = Expense {
                    id: existing.as_ref().map(|e| e.id).unwrap_or(0),
                    date,
                    description: form.value("description"),
                    category: ExpenseCategory::ALL[form.selected("category")],
                    amount,
                    notes: form.value("notes"),
                    billing: BillingPeriod::ALL[form.selected("billing")],
                    rebill_client_id,
                    approved_by,
                    approved_on,
                    receipt_ref: form.value("receipt_ref"),
                    receipt_file: stored_receipt.clone(),
                };
                // A new path is copied in; the stored one (or blank) is kept as is.
                let receipt = form.value("receipt_file");
                if receipt.is_empty() {
                    expense.receipt_file.clear();
                } else if receipt != stored_receipt {
                    match crate::receipts::import(
                        &self.output_dir(),
                        &crate::paths::expand_home(&receipt),
                        &expense,
                    ) {
                        Ok(stored) => expense.receipt_file = stored,
                        Err(e) => {
                            form.fail("receipt_file", format!("{e:#}"));
                            return Ok(false);
                        }
                    }
                }
                if let Err(e) = self.db.save_expense(&expense) {
                    // Don't leave a copy behind for a save that didn't happen.
                    if expense.receipt_file != stored_receipt
                        && let Ok(copy) =
                            crate::receipts::resolve(&self.output_dir(), &expense.receipt_file)
                    {
                        let _ = std::fs::remove_file(copy);
                    }
                    return Err(e);
                }
                // Show the year the expense landed in.
                self.expense_year = TaxYear::containing(date);
                let follow_up = if expense.rebill_client_id.is_some() && !expense.is_approved() {
                    " Add who approved it before invoicing."
                } else {
                    ""
                };
                self.success(format!(
                    "Saved {} expense in {}.{follow_up}",
                    amount,
                    self.expense_year.label()
                ));
            }
            Purpose::CreditNoteActions(invoice_id, note_id) => {
                if form.selected("action") == 1 {
                    self.email_credit_note(invoice_id, note_id)?;
                } else {
                    let note = self.credit_note(invoice_id, note_id)?;
                    self.open_credit_note_pdf(&note)?;
                }
            }
            Purpose::Regenerate(id) => {
                let changes = if form.selected("how") == 1 {
                    self.detail_changes(id)?
                } else {
                    Vec::new()
                };
                if changes.is_empty() {
                    let (path, receipts) = self.regenerate(id)?;
                    let note = if form.selected("how") == 1 {
                        "Nothing printed had changed, so the details were kept. "
                    } else {
                        ""
                    };
                    self.success(format!(
                        "{note}Regenerated{receipts}: {}",
                        crate::paths::display(&path)
                    ));
                } else {
                    self.confirm(
                        format!(
                            "Update {} to your current details?\n\nThis changes {}.\n\nThe new PDF will differ from the one you sent, so send the client the new copy. The number, dates, lines and amounts don't change, and the invoice records that its details were updated.",
                            self.db.invoice(id)?.display_number(),
                            join_phrases(&changes)
                        ),
                        Action::UpdateDetails(id),
                    );
                }
            }
            Purpose::Reimbursement => {
                let options = self.reimbursement_options()?;
                let Some(e) = options.get(form.selected("expense")) else {
                    return Ok(true);
                };
                if !e.is_approved() {
                    form.fail(
                        "expense",
                        "Not approved yet. Record who approved it and when under Expenses (4)",
                    );
                    return Ok(false);
                }
                let line = LineItem {
                    detail: e.invoice_detail(),
                    expense_id: Some(e.id),
                    ..LineItem::new(
                        LineKind::Fixed,
                        format!("Reimbursement: {}", e.description),
                        Quantity::ONE,
                        e.amount,
                    )
                };
                self.add_or_replace_line(None, line);
            }
            Purpose::PriorBill(year) => {
                let amount = match form.value("amount").as_str() {
                    "" => None,
                    _ => match money_field(form, "amount", true) {
                        Some(m) => Some(m),
                        None => return Ok(false),
                    },
                };
                self.db.set_entered_bill(year, amount)?;
                self.success(match amount {
                    Some(m) => format!(
                        "Using {m} as your {} bill for payments on account",
                        year.label()
                    ),
                    None => format!("Using tally's estimate for {}", year.label()),
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
                let line = LineItem::new(
                    LineKind::ALL[form.selected("kind")],
                    form.value("description"),
                    quantity,
                    unit_price,
                );
                self.add_or_replace_line(index, line);
            }
        }
        Ok(true)
    }

    /// "Rebill to" options: nobody, then active clients (plus `current`, even
    /// if archived). The same list is rebuilt on submit to map the choice back.
    fn rebill_choices(&self, current: Option<i64>) -> Vec<(Option<i64>, String)> {
        let mut out = vec![(None, "No one (my own cost)".to_string())];
        out.extend(
            self.clients
                .iter()
                .filter(|c| !c.archived || Some(c.id) == current)
                .map(|c| (Some(c.id), c.name.clone())),
        );
        out
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
            Action::MarkSentToday(id) => {
                self.db.set_sent(id, Some(dates::today()))?;
                self.refresh()?;
                self.success("Marked as sent today");
            }
            Action::UpdateDetails(id) => {
                self.db.update_issued_details(id, dates::today())?;
                self.refresh()?;
                let (path, receipts) = self.regenerate(id)?;
                self.success(format!(
                    "Updated the details and regenerated{receipts}: {}",
                    crate::paths::display(&path)
                ));
            }
            Action::DeleteExpense(id) => {
                self.db.delete_expense(id)?;
                self.success("Expense deleted");
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
            Some(Modal::EmailPreview { .. }) => vec![("o", "PDF preview"), ("Esc", "close")],
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
            Some(Modal::EmailPreview { title, lines, .. }) => {
                let width = 84.min(f.area().width.saturating_sub(4));
                render_popup_text(f, title, lines.clone(), width);
            }
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
                // Wide enough for the longest hint or choice, plus any overview
                // panel, within the screen. Choice hints gain " · ←/→ to change".
                let content = form
                    .fields
                    .iter()
                    .map(|f| match &f.kind {
                        super::form::FieldKind::Choice(opts) => (label_width as usize
                            + 8
                            + opts.iter().map(|o| o.chars().count()).max().unwrap_or(0))
                        .max(f.hint.chars().count() + 21),
                        _ => f.hint.chars().count() + 4,
                    })
                    .max()
                    .unwrap_or(0) as u16;
                let width = ((label_width + 50).max(60).max(content) + form.side_width())
                    .min(f.area().width.saturating_sub(4));
                // Borders and padding take 4 columns.
                let area = centered(f.area(), width, form.height_in(width.saturating_sub(4)) + 3);
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
