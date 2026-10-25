//! The draft invoice editor: header fields plus a list of lines.

use anyhow::Result;
use chrono::{Days, NaiveDate};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Row, Table, TableState};

use super::form::{Form, Outcome};
use super::modals::{Action, Purpose};
use super::{App, Screen, clamp_selection, list_nav, theme};
use crate::domain::expense::Expense;
use crate::domain::{Business, Client, Invoice, LineItem, dates};

pub struct Editor {
    pub invoice: Invoice,
    pub header: Form,
    /// Client ids in the same order as the client choice options.
    client_ids: Vec<i64>,
    pub in_lines: bool,
    pub lines: TableState,
    pub dirty: bool,
    terms: u32,
    /// Keep the due date at issue date + terms until the user edits it.
    due_follows_issue: bool,
    /// Keep the supply date equal to the issue date until the user edits it.
    supply_follows_issue: bool,
}

impl Editor {
    pub fn new_draft(business: &Business, clients: &[Client], client_id: Option<i64>) -> Self {
        let today = dates::today();
        let client_id = client_id
            .or_else(|| clients.iter().find(|c| !c.archived).map(|c| c.id))
            .unwrap_or_default();
        let invoice = Invoice {
            id: 0,
            number: None,
            client_id,
            issue_date: today,
            due_date: today + Days::new(business.payment_terms_days.into()),
            supply_start: today,
            supply_end: None,
            notes: business.default_notes.clone(),
            issued_at: None,
            sent_on: None,
            snapshot: None,
            details_updated_on: None,
            lines: Vec::new(),
        };
        let mut e = Self::edit(invoice, business, clients);
        e.due_follows_issue = true;
        e.supply_follows_issue = true;
        e
    }

    pub fn edit(invoice: Invoice, business: &Business, clients: &[Client]) -> Self {
        // Archived clients are hidden unless this invoice already uses one.
        let choices: Vec<&Client> = clients
            .iter()
            .filter(|c| !c.archived || c.id == invoice.client_id)
            .collect();
        let client_ids: Vec<i64> = choices.iter().map(|c| c.id).collect();
        let selected = client_ids
            .iter()
            .position(|id| *id == invoice.client_id)
            .unwrap_or(0);
        let terms = business.payment_terms_days;
        let header = Form::new()
            .choice(
                "client",
                "Client",
                choices.iter().map(|c| c.name.clone()).collect(),
                selected,
            )
            .text("issue", "Invoice date", invoice.issue_date.to_string())
            .required()
            .hint("YYYY-MM-DD or DD/MM/YYYY")
            .text(
                "supply_start",
                "Supply date / from",
                invoice.supply_start.to_string(),
            )
            .required()
            .hint("When the work was done (required on UK invoices). Fill 'Supply to' for a period")
            .text(
                "supply_end",
                "Supply to",
                invoice
                    .supply_end
                    .map(|d| d.to_string())
                    .unwrap_or_default(),
            )
            .hint("Optional end of the supply period, e.g. the last day of the month")
            .text("due", "Due date", invoice.due_date.to_string())
            .required()
            .hint(&format!("Defaults to {terms} days after the invoice date"))
            .text("notes", "Notes", invoice.notes.clone())
            .hint("Printed under the payment details")
            .embedded();
        Editor {
            due_follows_issue: invoice.due_date == invoice.issue_date + Days::new(terms.into()),
            supply_follows_issue: invoice.supply_start == invoice.issue_date
                && invoice.supply_end.is_none(),
            invoice,
            header,
            client_ids,
            in_lines: false,
            lines: TableState::default().with_selected(0),
            dirty: false,
            terms,
        }
    }

    pub fn is_new(&self) -> bool {
        self.invoice.id == 0
    }

    pub fn client_id(&self) -> i64 {
        self.client_ids
            .get(self.header.selected("client"))
            .copied()
            .unwrap_or_default()
    }

    pub fn help(&self) -> Vec<(&'static str, &'static str)> {
        if self.in_lines {
            vec![
                ("a", "add line"),
                ("r", "add reimbursement"),
                ("Enter/e", "edit"),
                ("d", "delete"),
                ("K/J", "move"),
                ("↑", "to header"),
                ("Ctrl+P", "preview"),
                ("Ctrl+S", "save"),
                ("Esc", "close"),
            ]
        } else {
            vec![
                ("↑↓/Tab", "move"),
                ("↓ past notes", "lines"),
                ("Ctrl+P", "preview"),
                ("Ctrl+S", "save"),
                ("Esc", "close"),
            ]
        }
    }

    /// Applies the header form to the invoice. Returns an error message for the form.
    fn apply_header(&mut self) -> Result<(), (String, &'static str)> {
        let parse = |form: &Form, key: &'static str| -> Result<NaiveDate, (String, &'static str)> {
            dates::parse_date(&form.value(key)).map_err(|e| (e, key))
        };
        self.header
            .check_required()
            .map_err(|e| (e, self.header.focused_key()))?;
        self.invoice.client_id = self.client_id();
        self.invoice.issue_date = parse(&self.header, "issue")?;
        self.invoice.supply_start = parse(&self.header, "supply_start")?;
        self.invoice.supply_end = match self.header.value("supply_end").as_str() {
            "" => None,
            _ => Some(parse(&self.header, "supply_end")?),
        };
        self.invoice.due_date = parse(&self.header, "due")?;
        self.invoice.notes = self.header.value("notes");
        if self.invoice.due_date < self.invoice.issue_date {
            return Err((
                "The due date can't be before the invoice date".into(),
                "due",
            ));
        }
        if let Some(end) = self.invoice.supply_end
            && end < self.invoice.supply_start
        {
            return Err((
                "'Supply to' is before 'Supply date / from'".into(),
                "supply_end",
            ));
        }
        Ok(())
    }

    /// Keeps dependent dates in step while the user types the invoice date.
    fn on_header_change(&mut self) {
        self.dirty = true;
        match self.header.focused_key() {
            "issue" => {
                if let Ok(issue) = dates::parse_date(&self.header.value("issue")) {
                    if self.due_follows_issue {
                        self.header
                            .set_value("due", (issue + Days::new(self.terms.into())).to_string());
                    }
                    if self.supply_follows_issue {
                        self.header.set_value("supply_start", issue.to_string());
                    }
                }
            }
            "due" => self.due_follows_issue = false,
            "supply_start" | "supply_end" => self.supply_follows_issue = false,
            _ => {}
        }
    }
}

impl App {
    fn editor(&mut self) -> Option<&mut Editor> {
        match &mut self.screen {
            Screen::Editor(e) => Some(e),
            _ => None,
        }
    }

    /// Validates and saves the draft, then shows it.
    pub(super) fn save_editor(&mut self) -> Result<()> {
        // Match the field directly so `self.db` can be borrowed alongside it.
        let Screen::Editor(e) = &mut self.screen else {
            return Ok(());
        };
        if let Err((msg, key)) = e.apply_header() {
            e.in_lines = false;
            e.header.fail(key, msg);
            return Ok(());
        }
        let id = if e.is_new() {
            self.db.create_draft(&e.invoice)?
        } else {
            self.db.update_draft(&e.invoice)?;
            e.invoice.id
        };
        self.refresh()?;
        self.open_detail(id);
        self.success("Draft saved. Press i to issue it when ready.");
        Ok(())
    }

    /// Previews the draft as currently edited, without saving it.
    fn preview_editor(&mut self) -> Result<()> {
        let Screen::Editor(e) = &mut self.screen else {
            return Ok(());
        };
        if let Err((msg, key)) = e.apply_header() {
            e.in_lines = false;
            e.header.fail(key, msg);
            return Ok(());
        }
        let inv = e.invoice.clone();
        self.preview_draft(&inv)
    }

    pub(super) fn close_editor(&mut self) {
        let Some(e) = self.editor() else { return };
        let back_to = if e.is_new() { None } else { Some(e.invoice.id) };
        match back_to {
            Some(id) => self.open_detail(id),
            None => self.go(Screen::Invoices),
        }
    }

    pub(super) fn add_or_replace_line(&mut self, index: Option<usize>, line: LineItem) {
        let Some(e) = self.editor() else { return };
        match index {
            Some(i) if i < e.invoice.lines.len() => e.invoice.lines[i] = line,
            _ => {
                e.invoice.lines.push(line);
                e.lines.select(Some(e.invoice.lines.len() - 1));
            }
        }
        e.dirty = true;
        e.in_lines = true;
    }

    pub(super) fn key_editor(&mut self, key: KeyEvent) -> Result<()> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('s') {
            return self.save_editor();
        }
        if ctrl && key.code == KeyCode::Char('p') {
            return self.preview_editor();
        }
        let Some(e) = self.editor() else {
            return Ok(());
        };
        if key.code == KeyCode::Esc {
            if e.dirty {
                self.confirm("Discard your changes to this draft?", Action::DiscardEditor);
            } else {
                self.close_editor();
            }
            return Ok(());
        }

        if !e.in_lines {
            match e.header.handle_key(key) {
                Outcome::Changed => e.on_header_change(),
                Outcome::LeaveDown => e.in_lines = true,
                Outcome::Submit => return self.save_editor(),
                _ => {}
            }
            return Ok(());
        }

        let len = e.invoice.lines.len();
        let sel = e.lines.selected().filter(|i| *i < len);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if sel.unwrap_or(0) == 0 => e.in_lines = false,
            KeyCode::BackTab => e.in_lines = false,
            KeyCode::Char('K') => {
                if let Some(i) = sel.filter(|i| *i > 0) {
                    e.invoice.lines.swap(i, i - 1);
                    e.lines.select(Some(i - 1));
                    e.dirty = true;
                }
            }
            KeyCode::Char('J') => {
                if let Some(i) = sel.filter(|i| i + 1 < len) {
                    e.invoice.lines.swap(i, i + 1);
                    e.lines.select(Some(i + 1));
                    e.dirty = true;
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(i) = sel {
                    e.invoice.lines.remove(i);
                    clamp_selection(&mut e.lines, e.invoice.lines.len());
                    e.dirty = true;
                }
            }
            KeyCode::Char('a') | KeyCode::Char('n') => {
                let client = e.client_id();
                self.open_line_form(None, client);
            }
            KeyCode::Char('r') => self.open_form(Purpose::Reimbursement, None),
            KeyCode::Enter | KeyCode::Char('e') => {
                let client = e.client_id();
                if sel.is_some_and(|i| e.invoice.lines[i].is_reimbursement()) {
                    self.info("Reimbursement lines come from Expenses and can't be edited here. d removes one; r adds one");
                } else if sel.is_some() {
                    self.open_line_form(sel, client);
                } else {
                    self.open_line_form(None, client);
                }
            }
            code => {
                list_nav(&mut e.lines, len, code);
            }
        }
        Ok(())
    }

    pub(super) fn draw_editor(&mut self, f: &mut Frame, area: Rect) {
        let modal_open = self.modal.is_some();
        let Some(e) = self.editor() else { return };
        let title = if e.is_new() {
            "New invoice (draft)".to_string()
        } else {
            format!("Edit draft #{}", e.invoice.id)
        };
        let dirty = if e.dirty { "  • unsaved" } else { "" };
        let [
            title_area,
            _,
            header_area,
            lines_title,
            lines_area,
            total_area,
        ] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(e.header.height()),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .areas(area);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(title, theme::title()),
                Span::styled(dirty, theme::warning()),
            ])),
            title_area,
        );
        let header_width = header_area.width.min(100);
        e.header.render(
            f,
            Rect {
                width: header_width,
                ..header_area
            },
            !e.in_lines && !modal_open,
        );

        let lines_heading = if e.in_lines {
            theme::title()
        } else {
            theme::muted()
        };
        f.render_widget(
            Paragraph::new(Line::styled("Lines", lines_heading)),
            lines_title,
        );
        if e.invoice.lines.is_empty() {
            let msg = if e.in_lines {
                "No lines yet. Press a to add one, or r for a reimbursement."
            } else {
                "No lines yet. Move down here and press a."
            };
            f.render_widget(
                Paragraph::new(Line::styled(msg, theme::muted())),
                lines_area,
            );
        } else {
            let rows = e.invoice.lines.iter().map(|l| {
                let unit = l.kind.unit(l.quantity);
                Row::new(vec![
                    Line::from(if l.is_reimbursement() {
                        "Reimburse"
                    } else {
                        l.kind.label()
                    }),
                    Line::from(l.description.clone()),
                    Line::from(format!("{} {unit}", l.quantity).trim_end().to_string())
                        .right_aligned(),
                    Line::from(l.unit_price.to_string()).right_aligned(),
                    Line::from(l.amount().to_string()).right_aligned(),
                ])
            });
            let highlight = if e.in_lines && !modal_open {
                theme::selected_row()
            } else {
                ratatui::style::Style::new()
            };
            clamp_selection(&mut e.lines, e.invoice.lines.len());
            f.render_stateful_widget(
                Table::new(
                    rows,
                    [
                        Constraint::Length(11),
                        Constraint::Fill(1),
                        Constraint::Length(10),
                        Constraint::Length(12),
                        Constraint::Length(12),
                    ],
                )
                .header(super::numeric_header(
                    &["Type", "Description"],
                    &["Qty", "Rate", "Amount"],
                ))
                .column_spacing(2)
                .row_highlight_style(highlight),
                lines_area,
                &mut e.lines,
            );
        }
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Total  ", theme::muted()),
                Span::styled(e.invoice.total().to_string(), theme::bold()),
            ]))
            .right_aligned(),
            total_area,
        );
    }

    /// Expenses that can be added to the draft being edited: rebilled to its
    /// client, not on any saved invoice, and not already among its lines.
    pub(super) fn reimbursement_options(&self) -> Result<Vec<Expense>> {
        let Screen::Editor(e) = &self.screen else {
            return Ok(Vec::new());
        };
        let mut list = self
            .db
            .unbilled_reimbursements(e.client_id(), e.invoice.id)?;
        list.retain(|x| !e.invoice.lines.iter().any(|l| l.expense_id == Some(x.id)));
        Ok(list)
    }

    pub(super) fn editor_client_name(&self) -> String {
        let Screen::Editor(e) = &self.screen else {
            return String::new();
        };
        let id = e.client_id();
        self.clients
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
            .unwrap_or_default()
    }

    fn open_line_form(&mut self, index: Option<usize>, client_id: i64) {
        let client = self.clients.iter().find(|c| c.id == client_id).cloned();
        let existing =
            index.and_then(|i| self.editor().and_then(|e| e.invoice.lines.get(i).cloned()));
        self.modal = Some(super::modals::line_form(
            index,
            existing.as_ref(),
            client.as_ref(),
        ));
    }
}
