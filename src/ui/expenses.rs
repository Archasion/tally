//! Expenses tab: business costs by tax year, with Self Assessment category totals.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Row, Table, Wrap};

use super::modals::{Action, Purpose};
use super::{App, clamp_selection, list_nav, theme};
use crate::domain::Money;
use crate::domain::dates;
use crate::domain::expense::Expense;
use crate::reports;

impl App {
    fn year_expenses(&self) -> Vec<Expense> {
        self.db
            .expenses(Some(self.expense_year))
            .unwrap_or_default()
    }

    pub(super) fn key_expenses(&mut self, key: KeyEvent) -> Result<()> {
        let list = self.year_expenses();
        if list_nav(&mut self.expense_table, list.len(), key.code) {
            return Ok(());
        }
        let selected = self.expense_table.selected().and_then(|i| list.get(i));
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => {
                self.expense_year = self.expense_year.prev();
                self.expense_table.select(Some(0));
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.expense_year = self.expense_year.next();
                self.expense_table.select(Some(0));
            }
            KeyCode::Char('n') => self.open_form(Purpose::NewExpense, None),
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(e) = selected {
                    self.open_form(Purpose::EditExpense(e.id), None);
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(e) = selected {
                    self.confirm(
                        format!(
                            "Delete the expense '{}' ({}) on {}?",
                            e.description,
                            e.amount,
                            dates::short(e.date)
                        ),
                        Action::DeleteExpense(e.id),
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn draw_expenses(&mut self, f: &mut Frame, area: Rect) {
        let year = self.expense_year;
        let list = self.year_expenses();
        let total: Money = list.iter().map(|e| e.amount).sum();

        let [title_area, _, body] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .areas(area);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("Expenses {}", year.label()), theme::title()),
                Span::styled(
                    format!(
                        "   paid {} to {}   ←/→ to change year",
                        dates::short(year.start()),
                        dates::short(year.end())
                    ),
                    theme::muted(),
                ),
            ])),
            title_area,
        );

        let [table_area, side] = Layout::horizontal([Constraint::Fill(3), Constraint::Length(44)])
            .spacing(2)
            .areas(body);

        if list.is_empty() {
            f.render_widget(
                Paragraph::new(Line::styled(
                    "No expenses this tax year. Press n to add one.",
                    theme::muted(),
                )),
                table_area,
            );
        } else {
            let rows = list.iter().map(|e| {
                Row::new(vec![
                    Line::from(dates::short(e.date)),
                    Line::from(e.category.short()),
                    Line::from(e.description.clone()),
                    Line::from(e.amount.to_string()).right_aligned(),
                ])
            });
            clamp_selection(&mut self.expense_table, list.len());
            f.render_stateful_widget(
                Table::new(
                    rows,
                    [
                        Constraint::Length(11),
                        Constraint::Length(15),
                        Constraint::Fill(1),
                        Constraint::Length(12),
                    ],
                )
                .header(super::numeric_header(
                    &["Paid", "Category", "Description"],
                    &["Amount"],
                ))
                .column_spacing(2)
                .row_highlight_style(theme::selected_row()),
                table_area,
                &mut self.expense_table,
            );
        }

        // Category totals, as they'd go on the return.
        let mut side_lines = vec![
            Line::styled("By Self Assessment category", theme::header_row()),
            Line::default(),
        ];
        for (category, sum) in reports::expense_totals(&list) {
            side_lines.push(Line::from(vec![
                Span::raw(format!("{:<30}", category.short())),
                Span::raw(format!("{:>12}", sum.to_string())),
            ]));
        }
        side_lines.push(Line::default());
        side_lines.push(Line::from(vec![
            Span::styled(format!("{:<30}", "Total"), theme::bold()),
            Span::styled(format!("{:>12}", total.to_string()), theme::bold()),
        ]));
        if let Ok(book) = &self.rates
            && let Some(rates) = book.get(year)
        {
            side_lines.push(Line::default());
            let allowance = crate::tax::whole(rates.trading_allowance);
            let note = if total >= rates.trading_allowance {
                format!(
                    "These are more than the {allowance} trading allowance, so the tax estimate deducts them."
                )
            } else {
                format!(
                    "Below the {allowance} trading allowance: the tax estimate deducts the allowance instead."
                )
            };
            side_lines.push(Line::styled(note, theme::muted()));
        }
        if let Some(e) = self.expense_table.selected().and_then(|i| list.get(i)) {
            side_lines.push(Line::default());
            side_lines.push(Line::styled(e.category.label(), theme::accent()));
            if !e.notes.is_empty() {
                side_lines.push(Line::from(e.notes.clone()));
            }
        }
        f.render_widget(Paragraph::new(side_lines).wrap(Wrap { trim: true }), side);
    }
}
