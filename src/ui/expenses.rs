//! Expenses tab: business costs by tax year, with Self Assessment category
//! totals, and which purchases are rebilled to clients.

use std::collections::HashMap;

use anyhow::{Result, bail};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Row, Table, Wrap};

use super::modals::{Action, Purpose};
use super::{App, clamp_selection, list_nav, open_path, theme};
use crate::db::Billed;
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
            KeyCode::Char('u') => {
                if let Some(e) = selected {
                    self.open_form(Purpose::NextPayment(e.id), None);
                }
            }
            KeyCode::Char('o') => {
                if let Some(e) = selected {
                    if e.receipt_file.is_empty() {
                        bail!("No receipt file attached. Add one with e");
                    }
                    let path = crate::receipts::resolve(&self.output_dir(), &e.receipt_file)?;
                    if !path.is_file() {
                        bail!(
                            "The receipt is missing from {}",
                            crate::paths::display(&path)
                        );
                    }
                    open_path(&path)?;
                }
            }
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

    /// e.g. `Acme · 2026-0004`, `Acme · to bill` or `Acme · needs approval`.
    fn rebill_status(&self, e: &Expense, billed: &HashMap<i64, Billed>) -> Option<String> {
        let client = e.rebill_client_id?;
        let name = self
            .clients
            .iter()
            .find(|c| c.id == client)
            .map(|c| {
                if c.label.is_empty() {
                    c.name.clone()
                } else {
                    c.label.clone()
                }
            })
            .unwrap_or_default();
        let state = match billed.get(&e.id) {
            Some(b) => b.display(),
            None if e.is_approved() => "to bill".into(),
            None => "needs approval".into(),
        };
        Some(format!("{name} · {state}"))
    }

    pub(super) fn draw_expenses(&mut self, f: &mut Frame, area: Rect) {
        let year = self.expense_year;
        let list = self.year_expenses();
        let total: Money = list.iter().map(|e| e.amount).sum();
        let billed = self.db.billed_expenses().unwrap_or_default();
        let to_bill: Vec<&Expense> = list
            .iter()
            .filter(|e| e.rebill_client_id.is_some() && !billed.contains_key(&e.id))
            .collect();

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
                Span::styled(
                    match to_bill.len() {
                        0 => String::new(),
                        n => format!(
                            "   {n} to rebill ({})",
                            to_bill.iter().map(|e| e.amount).sum::<Money>()
                        ),
                    },
                    theme::warning(),
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
            let statuses: Vec<String> = list
                .iter()
                .map(|e| self.rebill_status(e, &billed).unwrap_or_default())
                .collect();
            let rows = list.iter().zip(statuses).map(|(e, status)| {
                Row::new(vec![
                    Line::from(dates::short(e.date)),
                    Line::from(e.category.short()),
                    Line::from(e.description.clone()),
                    Line::from(status),
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
                        Constraint::Length(20),
                        Constraint::Length(12),
                    ],
                )
                .header(super::numeric_header(
                    &["Paid", "Category", "Description", "Rebill"],
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
            let kv = |k: &str, v: String| {
                Line::from(vec![
                    Span::styled(format!("{k:<14}"), theme::muted()),
                    Span::raw(v),
                ])
            };
            side_lines.push(kv("Billing", e.billing.label().to_string()));
            if let Some(status) = self.rebill_status(e, &billed) {
                side_lines.push(kv("Rebill", status));
                side_lines.push(kv(
                    "Approved",
                    match e.approved_on {
                        Some(on) => format!("{}, {}", e.approved_by, dates::short(on)),
                        None => "not yet".into(),
                    },
                ));
            }
            if !e.receipt_ref.is_empty() {
                side_lines.push(kv("Supplier inv.", e.receipt_ref.clone()));
            }
            if !e.receipt_file.is_empty() {
                side_lines.push(kv("Receipt", format!("{}  (o opens)", e.receipt_file)));
            }
            if !e.notes.is_empty() {
                side_lines.push(Line::default());
                side_lines.push(Line::from(e.notes.clone()));
            }
        }
        f.render_widget(Paragraph::new(side_lines).wrap(Wrap { trim: true }), side);
    }
}
