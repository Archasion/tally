//! Reports tab: aging, tax-year income and CSV export.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Row, Table, Wrap};

use super::{App, clamp_selection, list_nav, open_path, theme};
use crate::domain::dates;
use crate::reports::{self, AgingBucket, InvoiceRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportTab {
    Aging,
    Income,
    Export,
}

impl ReportTab {
    const ALL: [ReportTab; 3] = [ReportTab::Aging, ReportTab::Income, ReportTab::Export];

    fn label(self) -> &'static str {
        match self {
            ReportTab::Aging => "Outstanding & overdue",
            ReportTab::Income => "Tax-year income",
            ReportTab::Export => "CSV export",
        }
    }
}

impl App {
    pub(super) fn reports_help(&self) -> Vec<(&'static str, &'static str)> {
        let mut keys = vec![("Tab", "next report")];
        match self.report_tab {
            ReportTab::Aging => keys.extend([("↑↓", "select"), ("Enter", "open invoice")]),
            ReportTab::Income => keys.push(("←/→", "tax year")),
            ReportTab::Export => {
                keys.push(("x", "export now"));
                if self.last_export.is_some() {
                    keys.push(("o", "open folder"));
                }
            }
        }
        keys
    }

    pub(super) fn key_reports(&mut self, key: KeyEvent) -> Result<()> {
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            let i = ReportTab::ALL
                .iter()
                .position(|t| *t == self.report_tab)
                .unwrap_or(0);
            let n = ReportTab::ALL.len();
            let next = if key.code == KeyCode::Tab {
                (i + 1) % n
            } else {
                (i + n - 1) % n
            };
            self.report_tab = ReportTab::ALL[next];
            return Ok(());
        }
        match self.report_tab {
            ReportTab::Aging => {
                let ids: Vec<i64> = reports::aging(&self.rows, dates::today())
                    .rows
                    .iter()
                    .map(|r| r.invoice.id)
                    .collect();
                if list_nav(&mut self.aging_table, ids.len(), key.code) {
                    return Ok(());
                }
                if key.code == KeyCode::Enter
                    && let Some(&id) = self.aging_table.selected().and_then(|i| ids.get(i))
                {
                    self.open_detail(id);
                }
            }
            ReportTab::Income => match key.code {
                KeyCode::Left | KeyCode::Char('h') => self.report_year = self.report_year.prev(),
                KeyCode::Right | KeyCode::Char('l') => self.report_year = self.report_year.next(),
                _ => {}
            },
            ReportTab::Export => match key.code {
                KeyCode::Char('x') | KeyCode::Enter => {
                    let dir = reports::export_csv(&self.db, &self.output_dir(), dates::today())?;
                    self.success(format!("Exported to {}", crate::paths::display(&dir)));
                    self.last_export = Some(dir);
                }
                KeyCode::Char('o') => {
                    if let Some(dir) = &self.last_export {
                        open_path(dir)?;
                    }
                }
                _ => {}
            },
        }
        Ok(())
    }

    pub(super) fn draw_reports(&mut self, f: &mut Frame, area: Rect) {
        let [tabs_area, _, body] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .areas(area);
        let mut spans = Vec::new();
        for (i, tab) in ReportTab::ALL.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" · ", theme::muted()));
            }
            let style = if *tab == self.report_tab {
                theme::title()
            } else {
                theme::muted()
            };
            spans.push(Span::styled(tab.label(), style));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), tabs_area);
        match self.report_tab {
            ReportTab::Aging => self.draw_aging(f, body),
            ReportTab::Income => self.draw_income(f, body),
            ReportTab::Export => self.draw_export(f, body),
        }
    }

    fn draw_aging(&mut self, f: &mut Frame, area: Rect) {
        let today = dates::today();
        let report = reports::aging(&self.rows, today);
        let [summary_area, _, table_area] = Layout::vertical([
            Constraint::Length(AgingBucket::ALL.len() as u16 + 2),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .areas(area);
        let mut summary: Vec<Row> = report
            .buckets
            .iter()
            .map(|(bucket, sum, count)| {
                let style = if *bucket != AgingBucket::NotYetDue && *count > 0 {
                    theme::error()
                } else {
                    ratatui::style::Style::new()
                };
                Row::new(vec![
                    Line::from(bucket.label()),
                    Line::from(count.to_string()).right_aligned(),
                    Line::styled(sum.to_string(), style).right_aligned(),
                ])
            })
            .collect();
        summary.push(Row::new(vec![
            Line::styled("Total outstanding", theme::bold()),
            Line::from(report.rows.len().to_string()).right_aligned(),
            Line::styled(report.total.to_string(), theme::bold()).right_aligned(),
        ]));
        f.render_widget(
            Table::new(
                summary,
                [
                    Constraint::Length(22),
                    Constraint::Length(9),
                    Constraint::Length(14),
                ],
            )
            .header(super::numeric_header(&[""], &["Invoices", "Owed"]))
            .column_spacing(2),
            summary_area,
        );
        if report.rows.is_empty() {
            f.render_widget(
                Paragraph::new(Line::styled("Nothing outstanding. 🎉", theme::success())),
                table_area,
            );
            return;
        }
        let rows: Vec<&InvoiceRow> = report.rows.iter().collect();
        clamp_selection(&mut self.aging_table, rows.len());
        f.render_stateful_widget(
            super::invoices::invoice_table(&rows, today),
            table_area,
            &mut self.aging_table,
        );
    }

    fn draw_income(&mut self, f: &mut Frame, area: Rect) {
        let report = match reports::income(&self.db, self.report_year) {
            Ok(r) => r,
            Err(e) => {
                f.render_widget(
                    Paragraph::new(Line::styled(format!("{e:#}"), theme::error())),
                    area,
                );
                return;
            }
        };
        let y = report.year;
        let header = vec![
            Line::from(vec![
                Span::styled(format!("Tax year {}", y.label()), theme::title()),
                Span::styled(
                    format!(
                        "   {} to {}   ←/→ to change",
                        dates::long(y.start()),
                        dates::long(y.end())
                    ),
                    theme::muted(),
                ),
            ]),
            Line::styled(
                "Cash basis: money received minus refunds paid, by the date the money moved.",
                theme::muted(),
            ),
        ];
        let [header_area, _, table_area] = Layout::vertical([
            Constraint::Length(header.len() as u16),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .areas(area);
        f.render_widget(
            Paragraph::new(header).wrap(Wrap { trim: true }),
            header_area,
        );

        let blank = |m: crate::domain::Money| {
            if m.is_zero() {
                String::new()
            } else {
                m.to_string()
            }
        };
        let mut rows: Vec<Row> = report
            .months
            .iter()
            .map(|(label, received, refunded)| {
                Row::new(vec![
                    Line::from(label.clone()),
                    Line::from(blank(*received)).right_aligned(),
                    Line::from(blank(*refunded)).right_aligned(),
                    Line::from(blank(*received - *refunded)).right_aligned(),
                ])
            })
            .collect();
        rows.push(Row::new(vec![
            Line::styled("Total", theme::bold()),
            Line::styled(report.received.to_string(), theme::bold()).right_aligned(),
            Line::styled(report.refunded.to_string(), theme::bold()).right_aligned(),
            Line::styled(report.net().to_string(), theme::bold()).right_aligned(),
        ]));
        f.render_widget(
            Table::new(
                rows,
                [
                    Constraint::Length(20),
                    Constraint::Length(14),
                    Constraint::Length(14),
                    Constraint::Length(14),
                ],
            )
            .header(super::numeric_header(
                &["Month"],
                &["Received", "Refunded", "Net"],
            ))
            .column_spacing(2),
            table_area,
        );
    }

    fn draw_export(&mut self, f: &mut Frame, area: Rect) {
        let target = crate::paths::exports_dir(&self.output_dir());
        let mut text = vec![
            Line::from("Writes four CSV files for your spreadsheet or accountant:"),
            Line::default(),
            Line::from(
                "  invoices.csv       every invoice with totals, payments, credits and balance",
            ),
            Line::from("  payments.csv       money received, with the tax year of each payment"),
            Line::from("  refunds.csv        money paid back to clients"),
            Line::from("  credit_notes.csv   credit notes and what they were against"),
            Line::default(),
            Line::from(vec![
                Span::styled("Into: ", theme::muted()),
                Span::raw(format!("{}/<date-time>/", crate::paths::display(&target))),
            ]),
            Line::default(),
            super::help_line(&[("x", "export now")]),
        ];
        if let Some(dir) = &self.last_export {
            text.push(Line::default());
            text.push(Line::from(vec![
                Span::styled("Last export: ", theme::success()),
                Span::raw(crate::paths::display(dir)),
            ]));
        }
        f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), area);
    }
}
