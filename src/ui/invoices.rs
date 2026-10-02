//! Invoice list and invoice detail screens.

use std::path::PathBuf;

use anyhow::{Result, bail};
use chrono::NaiveDate;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Row, Table, TableState};

use super::editor::Editor;
use super::modals::{Action, Purpose};
use super::{App, Screen, clamp_selection, list_nav, open_path, theme};
use crate::db::TxKind;
use crate::domain::{CreditNote, Status, Transaction, dates};
use crate::paths;
use crate::reports::InvoiceRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Outstanding,
    Overdue,
    Drafts,
    Settled,
}

impl Filter {
    const ALL: [Filter; 5] = [
        Filter::All,
        Filter::Outstanding,
        Filter::Overdue,
        Filter::Drafts,
        Filter::Settled,
    ];

    fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Outstanding => "Outstanding",
            Filter::Overdue => "Overdue",
            Filter::Drafts => "Drafts",
            Filter::Settled => "Paid & credited",
        }
    }

    fn matches(self, s: Status) -> bool {
        match self {
            Filter::All => true,
            Filter::Outstanding => s.is_outstanding(),
            Filter::Overdue => s == Status::Overdue,
            Filter::Drafts => s == Status::Draft,
            Filter::Settled => matches!(s, Status::Paid | Status::Credited),
        }
    }

    fn next(self) -> Self {
        let i = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

pub(super) fn invoice_table(rows: &[&InvoiceRow], today: NaiveDate) -> Table<'static> {
    let header = Row::new(vec![
        Line::from("Number"),
        Line::from("Client"),
        Line::from("Issued"),
        Line::from("Due"),
        Line::from("Total").right_aligned(),
        Line::from("Balance").right_aligned(),
        Line::from("Status"),
    ])
    .style(theme::header_row());
    let body = rows.iter().map(|r| {
        let inv = &r.invoice;
        let due = if r.status == Status::Overdue {
            format!(
                "{} ({}d late)",
                dates::short(inv.due_date),
                r.days_overdue(today)
            )
        } else {
            dates::short(inv.due_date)
        };
        Row::new(vec![
            Line::from(inv.display_number()),
            Line::from(r.client_name.clone()),
            Line::from(dates::short(inv.issue_date)),
            Line::from(due),
            Line::from(r.totals.total.to_string()).right_aligned(),
            Line::from(if inv.is_draft() {
                String::new()
            } else {
                r.balance().to_string()
            })
            .right_aligned(),
            Line::styled(r.status.label(), theme::status(r.status)),
        ])
    });
    Table::new(
        body,
        [
            Constraint::Length(14),
            Constraint::Fill(1),
            Constraint::Length(11),
            Constraint::Length(22),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(9),
        ],
    )
    .header(header)
    .column_spacing(2)
    .row_highlight_style(theme::selected_row())
}

/// One money movement on an invoice.
pub enum Entry {
    Payment(Transaction),
    Refund(Transaction),
    Credit(Box<CreditNote>),
}

impl Entry {
    fn date(&self) -> NaiveDate {
        match self {
            Entry::Payment(t) | Entry::Refund(t) => t.date,
            Entry::Credit(c) => c.date,
        }
    }
}

pub struct Detail {
    pub row: InvoiceRow,
    pub entries: Vec<Entry>,
    pub sel: TableState,
    pub pdf: Option<PathBuf>,
}

impl Detail {
    pub fn load(app: &App, id: i64) -> Result<Detail> {
        let Some(row) = app.rows.iter().find(|r| r.invoice.id == id).cloned() else {
            bail!("invoice {id} no longer exists");
        };
        let mut entries: Vec<Entry> = Vec::new();
        entries.extend(
            app.db
                .transactions(TxKind::Payment, Some(id))?
                .into_iter()
                .map(Entry::Payment),
        );
        entries.extend(
            app.db
                .transactions(TxKind::Refund, Some(id))?
                .into_iter()
                .map(Entry::Refund),
        );
        entries.extend(
            app.db
                .credit_notes(Some(id))?
                .into_iter()
                .map(|c| Entry::Credit(Box::new(c))),
        );
        entries.sort_by_key(Entry::date);
        let pdf =
            (!row.invoice.is_draft()).then(|| paths::invoice_pdf(&app.output_dir(), &row.invoice));
        let selected = app.detail.as_ref().and_then(|d| d.sel.selected());
        let mut sel = TableState::default().with_selected(selected.or(Some(0)));
        clamp_selection(&mut sel, entries.len());
        Ok(Detail {
            row,
            entries,
            sel,
            pdf,
        })
    }
}

impl App {
    fn filtered(&self) -> Vec<&InvoiceRow> {
        self.rows
            .iter()
            .filter(|r| self.invoice_filter.matches(r.status))
            .collect()
    }

    /// Opens the editor on a new draft, optionally for a given client.
    pub(super) fn new_invoice(&mut self, client_id: Option<i64>) -> Result<()> {
        let business = self.db.require_business()?;
        if !self.clients.iter().any(|c| !c.archived) {
            bail!("Add a client first: press 3, then n");
        }
        let editor = Editor::new_draft(&business, &self.clients, client_id);
        self.go(Screen::Editor(Box::new(editor)));
        Ok(())
    }

    pub(super) fn key_invoices(&mut self, key: KeyEvent) -> Result<()> {
        let ids: Vec<i64> = self.filtered().iter().map(|r| r.invoice.id).collect();
        if list_nav(&mut self.invoice_table, ids.len(), key.code) {
            return Ok(());
        }
        let selected = self
            .invoice_table
            .selected()
            .and_then(|i| ids.get(i).copied());
        match key.code {
            KeyCode::Char('f') | KeyCode::Tab => {
                self.invoice_filter = self.invoice_filter.next();
                self.invoice_table.select(Some(0));
            }
            KeyCode::Char('n') => self.new_invoice(None)?,
            KeyCode::Enter => {
                if let Some(id) = selected {
                    self.open_detail(id);
                }
            }
            KeyCode::Char('u') => {
                if let Some(id) = selected {
                    self.duplicate(id)?;
                }
            }
            KeyCode::Char('o') => {
                if let Some(id) = selected {
                    self.open_invoice_pdf(id)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn draw_invoices(&mut self, f: &mut Frame, area: Rect) {
        let [filter_area, _, table_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .areas(area);
        let mut spans = vec![Span::styled("Show: ", theme::muted())];
        for (i, filter) in Filter::ALL.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" · ", theme::muted()));
            }
            let count = self
                .rows
                .iter()
                .filter(|r| filter.matches(r.status))
                .count();
            let text = format!("{} ({count})", filter.label());
            spans.push(if *filter == self.invoice_filter {
                Span::styled(text, theme::title())
            } else {
                Span::styled(text, theme::muted())
            });
        }
        f.render_widget(Paragraph::new(Line::from(spans)), filter_area);

        let today = dates::today();
        let rows: Vec<&InvoiceRow> = self
            .rows
            .iter()
            .filter(|r| self.invoice_filter.matches(r.status))
            .collect();
        if rows.is_empty() {
            f.render_widget(
                Paragraph::new(Line::styled(
                    "Nothing here. Press n for a new invoice.",
                    theme::muted(),
                )),
                table_area,
            );
            return;
        }
        clamp_selection(&mut self.invoice_table, rows.len());
        f.render_stateful_widget(
            invoice_table(&rows, today),
            table_area,
            &mut self.invoice_table,
        );
    }

    // ----- detail ---------------------------------------------------------

    fn duplicate(&mut self, id: i64) -> Result<()> {
        let new_id = self.db.duplicate_invoice(id, dates::today())?;
        self.refresh()?;
        let inv = self.db.invoice(new_id)?;
        let business = self.db.require_business()?;
        self.go(Screen::Editor(Box::new(Editor::edit(
            inv,
            &business,
            &self.clients,
        ))));
        self.success("Duplicated into a new draft. Check the dates and lines, then Ctrl+S.");
        Ok(())
    }

    pub(super) fn issue(&mut self, id: i64) -> Result<()> {
        let out = self.output_dir();
        let now = chrono::Local::now().naive_local();
        self.renderer()?;
        let renderer = self.renderer.as_ref().expect("initialised above");
        let mut written = PathBuf::new();
        let inv = self.db.issue_invoice(id, now, |inv| {
            written = renderer.write_invoice(&out, inv)?;
            Ok(())
        })?;
        self.refresh()?;
        self.success(format!(
            "Issued {} → {}",
            inv.display_number(),
            paths::display(&written)
        ));
        Ok(())
    }

    pub(super) fn regenerate(&mut self, id: i64) -> Result<PathBuf> {
        let inv = self.db.invoice(id)?;
        let out = self.output_dir();
        let path = self.renderer()?.write_invoice(&out, &inv)?;
        Ok(path)
    }

    fn open_invoice_pdf(&mut self, id: i64) -> Result<()> {
        let inv = self.db.invoice(id)?;
        if inv.is_draft() {
            bail!("Drafts don't have a PDF yet. Issue the invoice first (i).");
        }
        let path = paths::invoice_pdf(&self.output_dir(), &inv);
        if !path.exists() {
            self.regenerate(id)?;
            self.info(format!(
                "The PDF was missing, so it was recreated at {}",
                paths::display(&path)
            ));
        }
        open_path(&path)
    }

    fn open_credit_note_pdf(&mut self, note: &CreditNote) -> Result<()> {
        let path = paths::credit_note_pdf(&self.output_dir(), note);
        if !path.exists() {
            let inv = self.db.invoice(note.invoice_id)?;
            let out = self.output_dir();
            self.renderer()?.write_credit_note(&out, note, &inv)?;
            self.info(format!(
                "The PDF was missing, so it was recreated at {}",
                paths::display(&path)
            ));
        }
        open_path(&path)
    }

    pub(super) fn detail_help(&self) -> Vec<(&'static str, &'static str)> {
        let Some(d) = &self.detail else {
            return vec![("Esc", "back")];
        };
        if d.row.invoice.is_draft() {
            return vec![
                ("e", "edit"),
                ("i", "issue"),
                ("u", "duplicate"),
                ("x", "delete draft"),
                ("Esc", "back"),
            ];
        }
        let mut keys = vec![("p", "payment"), ("s", "sent")];
        if d.row.balance().is_negative() {
            keys.push(("r", "refund"));
        }
        if d.row.totals.creditable().is_positive() {
            keys.push(("c", "credit note"));
        }
        keys.extend([("o", "PDF"), ("R", "regenerate"), ("u", "duplicate")]);
        match d.sel.selected().and_then(|i| d.entries.get(i)) {
            Some(Entry::Credit(_)) => keys.push(("Enter", "open credit note")),
            Some(_) => keys.push(("X", "delete entry")),
            None => {}
        }
        keys.push(("Esc", "back"));
        keys
    }

    pub(super) fn key_detail(&mut self, id: i64, key: KeyEvent) -> Result<()> {
        let Some(d) = &mut self.detail else {
            self.go(Screen::Invoices);
            return Ok(());
        };
        let len = d.entries.len();
        if list_nav(&mut d.sel, len, key.code) {
            return Ok(());
        }
        let is_draft = d.row.invoice.is_draft();
        let balance = d.row.balance();
        let creditable = d.row.totals.creditable();
        let number = d.row.invoice.display_number();
        let selected = d.sel.selected().and_then(|i| d.entries.get(i));
        let selected_tx = match selected {
            Some(Entry::Payment(t)) => Some((TxKind::Payment, t.clone())),
            Some(Entry::Refund(t)) => Some((TxKind::Refund, t.clone())),
            _ => None,
        };
        let selected_cn = match selected {
            Some(Entry::Credit(c)) => Some((**c).clone()),
            _ => None,
        };

        match key.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left => self.go(Screen::Invoices),
            KeyCode::Char('u') => self.duplicate(id)?,
            // Drafts
            KeyCode::Char('e') | KeyCode::Char('E') if is_draft => {
                let inv = self.db.invoice(id)?;
                let business = self.db.require_business()?;
                self.go(Screen::Editor(Box::new(Editor::edit(
                    inv,
                    &business,
                    &self.clients,
                ))));
            }
            KeyCode::Char('i') if is_draft => {
                let inv = self.db.invoice(id)?;
                if inv.lines.is_empty() {
                    bail!("Add at least one line before issuing (e to edit)");
                }
                self.confirm(
                    format!(
                        "Issue this invoice for {}?\n\nIt gets the next invoice number, the PDF is written, and the invoice is locked. Later corrections need a credit note.",
                        inv.total()
                    ),
                    Action::Issue(id),
                );
            }
            KeyCode::Char('e') | KeyCode::Char('x') if !is_draft => {
                bail!(
                    "{number} is issued and locked. To correct it, raise a credit note (c) and issue a new invoice (u duplicates it)."
                )
            }
            KeyCode::Char('x') if is_draft => self.confirm(
                "Delete this draft? This can't be undone.",
                Action::DeleteDraft(id),
            ),
            // Issued
            KeyCode::Char('p') if !is_draft => self.open_form(
                Purpose::Payment(id),
                Some(balance.max(crate::domain::Money::ZERO)),
            ),
            KeyCode::Char('r') if !is_draft => {
                if !balance.is_negative() {
                    bail!(
                        "Nothing is owed back on {number}. Refunds settle overpayments or credit notes on paid invoices."
                    );
                }
                self.open_form(Purpose::Refund(id), Some(-balance));
            }
            KeyCode::Char('c') if !is_draft => {
                if !creditable.is_positive() {
                    bail!("{number} has already been fully credited");
                }
                self.open_form(Purpose::CreditNote(id), Some(creditable));
            }
            KeyCode::Char('s') if !is_draft => self.open_form(Purpose::MarkSent(id), None),
            KeyCode::Char('o') if !is_draft => self.open_invoice_pdf(id)?,
            KeyCode::Char('R') if !is_draft => {
                let warning = if paths::template_override(&self.output_dir()).is_file() {
                    "\n\nYou have a template.typ override, so the regenerated PDF may look different from the one you sent."
                } else {
                    ""
                };
                self.confirm(
                    format!("Regenerate the PDF for {number}? It uses the details frozen at issue time.{warning}"),
                    Action::Regenerate(id),
                );
            }
            KeyCode::Enter => {
                if let Some(cn) = selected_cn {
                    self.open_credit_note_pdf(&cn)?;
                }
            }
            KeyCode::Char('X') => {
                if let Some((kind, t)) = selected_tx {
                    let what = if kind == TxKind::Payment {
                        "payment"
                    } else {
                        "refund"
                    };
                    self.confirm(
                        format!(
                            "Delete the {what} of {} on {}? Only do this for a mistaken entry.",
                            t.amount,
                            dates::short(t.date)
                        ),
                        Action::DeleteTx(kind, t.id),
                    );
                } else if selected_cn.is_some() {
                    bail!("Credit notes are permanent records and can't be deleted");
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn draw_detail(&mut self, f: &mut Frame, area: Rect) {
        let Some(d) = &mut self.detail else { return };
        let inv = &d.row.invoice;
        let today = dates::today();

        let mut info = vec![Line::from(vec![
            Span::styled(
                if inv.is_draft() {
                    format!("Draft #{}", inv.id)
                } else {
                    format!("Invoice {}", inv.display_number())
                },
                theme::title(),
            ),
            Span::raw("  "),
            Span::styled(d.row.status.label(), theme::status(d.row.status)),
        ])];
        let kv = |k: &str, v: String| {
            Line::from(vec![
                Span::styled(format!("{k:<14}"), theme::muted()),
                Span::raw(v),
            ])
        };
        info.push(kv("Client", d.row.client_name.clone()));
        info.push(kv("Invoice date", dates::long(inv.issue_date)));
        info.push(kv(
            if inv.supply_end.is_some() {
                "Supply period"
            } else {
                "Supply date"
            },
            dates::supply_period(inv.supply_start, inv.supply_end),
        ));
        let mut due = dates::long(inv.due_date);
        if d.row.status == Status::Overdue {
            due.push_str(&format!("  ({} days overdue)", d.row.days_overdue(today)));
        }
        info.push(kv("Due date", due));
        if !inv.is_draft() {
            info.push(kv(
                "Sent",
                inv.sent_on
                    .map(dates::long)
                    .unwrap_or_else(|| "not marked as sent".into()),
            ));
        }
        if let Some(pdf) = &d.pdf {
            let exists = if pdf.exists() {
                ""
            } else {
                "  (missing — o recreates it)"
            };
            info.push(kv("PDF", format!("{}{exists}", paths::display(pdf))));
        }
        if !inv.notes.is_empty() {
            info.push(kv("Notes", inv.notes.clone()));
        }

        let lines_height = inv.lines.len() as u16 + 1;
        let [
            info_area,
            _,
            lines_area,
            totals_area,
            _,
            ledger_title,
            ledger_area,
        ] = Layout::vertical([
            Constraint::Length(info.len() as u16),
            Constraint::Length(1),
            Constraint::Length(lines_height.max(2)),
            Constraint::Length(5),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .areas(area);
        f.render_widget(Paragraph::new(info), info_area);

        let line_rows = inv.lines.iter().map(|l| {
            let unit = l.kind.unit(l.quantity);
            Row::new(vec![
                Line::from(l.description.clone()),
                Line::from(format!("{} {unit}", l.quantity).trim_end().to_string()).right_aligned(),
                Line::from(l.unit_price.to_string()).right_aligned(),
                Line::from(l.amount().to_string()).right_aligned(),
            ])
        });
        f.render_widget(
            Table::new(
                line_rows,
                [
                    Constraint::Fill(1),
                    Constraint::Length(12),
                    Constraint::Length(12),
                    Constraint::Length(12),
                ],
            )
            .header(super::numeric_header(
                &["Description"],
                &["Qty", "Rate", "Amount"],
            ))
            .column_spacing(2),
            lines_area,
        );

        let t = d.row.totals;
        // Fixed-width label/value pairs so the amounts line up when right-aligned.
        let total_line = |label: &str, value: crate::domain::Money, style| {
            Line::from(vec![
                Span::styled(format!("{label:<10}"), theme::muted()),
                Span::styled(format!("{:>12}", value.to_string()), style),
            ])
        };
        let plain = ratatui::style::Style::new();
        let mut totals = vec![total_line("Total", t.total, theme::bold())];
        if !inv.is_draft() {
            totals.push(total_line("Paid", t.paid, plain));
            if !t.credited.is_zero() {
                totals.push(total_line("Credited", t.credited, plain));
            }
            if !t.refunded.is_zero() {
                totals.push(total_line("Refunded", t.refunded, plain));
            }
            let balance = t.balance();
            totals.push(if balance.is_negative() {
                total_line("Owed back", -balance, theme::warning())
            } else if balance.is_zero() {
                total_line("Balance", balance, theme::success())
            } else {
                total_line("Balance", balance, theme::bold())
            });
        }
        f.render_widget(Paragraph::new(totals).right_aligned(), totals_area);

        if inv.is_draft() {
            return;
        }
        f.render_widget(
            Paragraph::new(Line::styled(
                "Payments, refunds & credit notes",
                theme::title(),
            )),
            ledger_title,
        );
        if d.entries.is_empty() {
            f.render_widget(
                Paragraph::new(Line::styled(
                    "None yet. Press p to record a payment.",
                    theme::muted(),
                )),
                ledger_area,
            );
            return;
        }
        let rows = d.entries.iter().map(|e| match e {
            Entry::Payment(t) => Row::new(vec![
                Line::from(dates::short(t.date)),
                Line::styled("Payment", theme::success()),
                Line::from(t.amount.to_string()).right_aligned(),
                Line::from(join_nonempty(&[&t.method, &t.reference])),
            ]),
            Entry::Refund(t) => Row::new(vec![
                Line::from(dates::short(t.date)),
                Line::styled("Refund", theme::warning()),
                Line::from((-t.amount).to_string()).right_aligned(),
                Line::from(join_nonempty(&[&t.method, &t.reference])),
            ]),
            Entry::Credit(c) => Row::new(vec![
                Line::from(dates::short(c.date)),
                Line::styled(
                    format!("Credit {}", c.number),
                    theme::status(Status::Credited),
                ),
                Line::from((-c.total()).to_string()).right_aligned(),
                Line::from(c.reason.clone()),
            ]),
        });
        f.render_stateful_widget(
            Table::new(
                rows,
                [
                    Constraint::Length(11),
                    Constraint::Length(18),
                    Constraint::Length(12),
                    Constraint::Fill(1),
                ],
            )
            .column_spacing(2)
            .row_highlight_style(theme::selected_row()),
            ledger_area,
            &mut d.sel,
        );
    }
}

fn join_nonempty(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" · ")
}
