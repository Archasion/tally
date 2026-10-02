use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use super::invoices::invoice_table;
use super::{App, list_nav, theme};
use crate::domain::dates;
use crate::reports::{self, InvoiceRow};

const RECENT: usize = 12;

fn recent(rows: &[InvoiceRow]) -> Vec<&InvoiceRow> {
    let mut rows: Vec<&InvoiceRow> = rows.iter().collect();
    // Overdue first, then drafts, then newest.
    rows.sort_by_key(|r| {
        (
            r.status != crate::domain::Status::Overdue,
            !r.invoice.is_draft(),
            std::cmp::Reverse(r.invoice.issue_date),
        )
    });
    rows.truncate(RECENT);
    rows
}

impl App {
    pub(super) fn key_dashboard(&mut self, key: KeyEvent) -> Result<()> {
        let recent: Vec<i64> = recent(&self.rows).iter().map(|r| r.invoice.id).collect();
        if list_nav(&mut self.dash_table, recent.len(), key.code) {
            return Ok(());
        }
        match key.code {
            KeyCode::Enter => {
                if let Some(&id) = self.dash_table.selected().and_then(|i| recent.get(i)) {
                    self.open_detail(id);
                }
            }
            KeyCode::Char('n') => self.new_invoice(None)?,
            _ => {}
        }
        Ok(())
    }

    pub(super) fn draw_dashboard(&mut self, f: &mut Frame, area: Rect) {
        let today = dates::today();
        let o = reports::overview(&self.rows);
        let year = reports::current_tax_year();
        let income = reports::income(&self.db, year).map(|r| r.net()).unwrap_or_default();

        let [cards, _, table_title, table_area] = Layout::vertical([
            Constraint::Length(4),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .areas(area);

        let card_areas = Layout::horizontal([Constraint::Fill(1); 4]).spacing(1).split(cards);
        let overdue_style = if o.overdue_count > 0 { theme::error() } else { theme::bold() };
        let card = |title: &str, value: String, sub: String, style| {
            Paragraph::new(vec![Line::styled(value, style), Line::styled(sub, theme::muted())])
                .block(Block::bordered().title(Span::styled(format!(" {title} "), theme::muted())).border_style(theme::muted()))
        };
        f.render_widget(
            card(
                "Outstanding",
                o.outstanding.to_string(),
                format!("{} invoice{}", o.outstanding_count, plural(o.outstanding_count)),
                theme::bold(),
            ),
            card_areas[0],
        );
        f.render_widget(
            card(
                "Overdue",
                o.overdue.to_string(),
                format!("{} invoice{}", o.overdue_count, plural(o.overdue_count)),
                overdue_style,
            ),
            card_areas[1],
        );
        f.render_widget(
            card(&format!("Income {}", year.label()), income.to_string(), "received, cash basis".into(), theme::bold()),
            card_areas[2],
        );
        let (drafts_value, drafts_sub, style) = if o.refunds_owed.is_positive() {
            (o.refunds_owed.to_string(), "refunds owed to clients".to_string(), theme::warning())
        } else {
            (o.drafts.to_string(), format!("draft{}", plural(o.drafts)), theme::bold())
        };
        f.render_widget(
            card(if o.refunds_owed.is_positive() { "Refunds due" } else { "Drafts" }, drafts_value, drafts_sub, style),
            card_areas[3],
        );

        let recent = recent(&self.rows);
        f.render_widget(Paragraph::new(Line::styled("Needs attention & recent", theme::title())), table_title);
        if recent.is_empty() {
            let msg = if self.clients.is_empty() {
                "No invoices yet. Press 3 to add a client, then n to create an invoice."
            } else {
                "No invoices yet. Press n to create one."
            };
            f.render_widget(Paragraph::new(Line::styled(msg, theme::muted())), table_area);
            return;
        }
        super::clamp_selection(&mut self.dash_table, recent.len());
        let table = invoice_table(&recent, today);
        f.render_stateful_widget(table, table_area, &mut self.dash_table);
    }
}

pub(super) fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
