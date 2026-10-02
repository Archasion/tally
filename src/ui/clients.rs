use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Row, Table};

use super::modals::Purpose;
use super::{App, clamp_selection, list_nav, theme};
use crate::domain::{Client, LineKind, Money};

impl App {
    fn visible_clients(&self) -> Vec<&Client> {
        self.clients
            .iter()
            .filter(|c| self.show_archived || !c.archived)
            .collect()
    }

    pub(super) fn select_client(&mut self, id: i64) {
        let index = self.visible_clients().iter().position(|c| c.id == id);
        self.client_table.select(index.or(Some(0)));
    }

    pub(super) fn key_clients(&mut self, key: KeyEvent) -> Result<()> {
        let ids: Vec<i64> = self.visible_clients().iter().map(|c| c.id).collect();
        if list_nav(&mut self.client_table, ids.len(), key.code) {
            return Ok(());
        }
        let selected = self
            .client_table
            .selected()
            .and_then(|i| ids.get(i).copied());
        match key.code {
            KeyCode::Char('n') => self.open_form(Purpose::NewClient, None),
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(id) = selected {
                    self.open_form(Purpose::EditClient(id), None);
                }
            }
            KeyCode::Char('i') => {
                if let Some(id) = selected {
                    if self.clients.iter().any(|c| c.id == id && c.archived) {
                        anyhow::bail!("Unarchive this client (a) before invoicing them");
                    }
                    self.new_invoice(Some(id))?;
                }
            }
            KeyCode::Char('a') => {
                if let Some(client) = selected
                    .and_then(|id| self.clients.iter().find(|c| c.id == id))
                    .cloned()
                {
                    self.db.set_client_archived(client.id, !client.archived)?;
                    self.refresh()?;
                    self.success(if client.archived {
                        format!("Restored {}", client.name)
                    } else {
                        format!("Archived {} (h shows archived clients)", client.name)
                    });
                }
            }
            KeyCode::Char('h') => {
                self.show_archived = !self.show_archived;
                self.client_table.select(Some(0));
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn draw_clients(&mut self, f: &mut Frame, area: Rect) {
        let [title_area, _, table_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .areas(area);
        let archived = self.clients.iter().filter(|c| c.archived).count();
        let mut title = vec![Span::styled("Clients", theme::title())];
        if archived > 0 {
            let note = if self.show_archived {
                format!("  (showing {archived} archived)")
            } else {
                format!("  ({archived} archived hidden)")
            };
            title.push(Span::styled(note, theme::muted()));
        }
        f.render_widget(Paragraph::new(Line::from(title)), title_area);

        // Outstanding balance per client.
        let owed = |id: i64| -> Money {
            self.rows
                .iter()
                .filter(|r| r.invoice.client_id == id && r.status.is_outstanding())
                .map(|r| r.balance())
                .sum()
        };
        let clients = self.visible_clients();
        if clients.is_empty() {
            f.render_widget(
                Paragraph::new(Line::styled(
                    "No clients yet. Press n to add one.",
                    theme::muted(),
                )),
                table_area,
            );
            return;
        }
        let rows: Vec<Row> = clients
            .iter()
            .map(|c| {
                let rate = match c.default_rate {
                    Some(r) => match c.default_rate_kind {
                        LineKind::Hours => format!("{r}/hr"),
                        LineKind::Days => format!("{r}/day"),
                        LineKind::Fixed => r.to_string(),
                    },
                    None => String::new(),
                };
                let kind = match (c.is_business, c.late_payment_notice) {
                    (true, true) => "Business · late notice",
                    (true, false) => "Business",
                    (false, _) => "Individual",
                };
                let owed = owed(c.id);
                let name_style = if c.archived {
                    theme::muted()
                } else {
                    ratatui::style::Style::new()
                };
                Row::new(vec![
                    Line::styled(c.label.clone(), theme::accent()),
                    Line::styled(c.name.clone(), name_style),
                    Line::from(
                        [c.address.town.as_str(), c.address.country.as_str()]
                            .into_iter()
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    Line::from(kind),
                    Line::from(rate).right_aligned(),
                    Line::from(if owed.is_zero() {
                        String::new()
                    } else {
                        owed.to_string()
                    })
                    .right_aligned(),
                ])
            })
            .collect();
        let len = rows.len();
        let table = Table::new(
            rows,
            [
                Constraint::Length(10),
                Constraint::Fill(2),
                Constraint::Fill(2),
                Constraint::Length(22),
                Constraint::Length(14),
                Constraint::Length(12),
            ],
        )
        .header(super::numeric_header(
            &["Label", "Name", "Location", "Type"],
            &["Default rate", "Owed"],
        ))
        .column_spacing(2)
        .row_highlight_style(theme::selected_row());
        clamp_selection(&mut self.client_table, len);
        f.render_stateful_widget(table, table_area, &mut self.client_table);
    }
}
