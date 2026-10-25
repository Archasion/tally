//! Reports tab: aging, tax-year income and CSV export.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Row, Table, Wrap};

use super::modals::Purpose;
use super::{App, clamp_selection, list_nav, open_path, theme};
use crate::domain::dates;
use crate::reports::{self, AgingBucket, InvoiceRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportTab {
    Aging,
    Income,
    Tax,
    Export,
}

impl ReportTab {
    const ALL: [ReportTab; 4] = [
        ReportTab::Aging,
        ReportTab::Income,
        ReportTab::Tax,
        ReportTab::Export,
    ];

    fn label(self) -> &'static str {
        match self {
            ReportTab::Aging => "Outstanding & overdue",
            ReportTab::Income => "Tax-year income",
            ReportTab::Tax => "Tax estimate",
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
            ReportTab::Tax => keys.extend([
                ("←/→", "tax year"),
                ("p", "to date / projected"),
                ("b", "last year's bill"),
                ("↑↓", "scroll"),
                ("R", "reload rates"),
            ]),
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
            ReportTab::Tax => match key.code {
                KeyCode::Left | KeyCode::Char('h') => {
                    self.report_year = self.report_year.prev();
                    self.tax_scroll = 0;
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    self.report_year = self.report_year.next();
                    self.tax_scroll = 0;
                }
                KeyCode::Char('p') => self.tax_projected = !self.tax_projected,
                KeyCode::Char('b') => {
                    self.open_form(Purpose::PriorBill(self.report_year.prev()), None)
                }
                KeyCode::Char('R') => {
                    self.reload_rates();
                    match &self.rates {
                        Ok(_) => self.success("Reloaded tax rates"),
                        Err(e) => self.toast = Some((e.clone(), super::ToastKind::Error)),
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.tax_scroll = self.tax_scroll.saturating_add(1)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.tax_scroll = self.tax_scroll.saturating_sub(1)
                }
                KeyCode::PageDown => self.tax_scroll = self.tax_scroll.saturating_add(10),
                KeyCode::PageUp => self.tax_scroll = self.tax_scroll.saturating_sub(10),
                KeyCode::Home | KeyCode::Char('g') => self.tax_scroll = 0,
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
            ReportTab::Tax => self.draw_tax(f, body),
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
            Line::from("Writes five CSV files for your spreadsheet or accountant:"),
            Line::default(),
            Line::from(
                "  invoices.csv       every invoice with totals, payments, credits and balance",
            ),
            Line::from("  payments.csv       money received, with the tax year of each payment"),
            Line::from("  refunds.csv        money paid back to clients"),
            Line::from("  credit_notes.csv   credit notes and what they were against"),
            Line::from(
                "  expenses.csv       expenses with their Self Assessment category and tax year",
            ),
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

    fn draw_tax(&mut self, f: &mut Frame, area: Rect) {
        let today = dates::today();
        let book = match &self.rates {
            Ok(book) => book,
            Err(e) => {
                let text = vec![
                    Line::styled("Your rates.toml couldn't be read", theme::error()),
                    Line::from(e.clone()),
                    Line::default(),
                    Line::styled("Fix the file and press R to reload.", theme::muted()),
                ];
                f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), area);
                return;
            }
        };
        let view = match reports::tax_report(&self.db, book, self.report_year, today) {
            Ok(reports::TaxReport::Ready(view)) => view,
            Ok(reports::TaxReport::NoRates(y)) => {
                let path = crate::paths::rates_override(&self.output_dir());
                let years: Vec<String> = book.years().map(|y| y.label()).collect();
                let text = vec![
                    Line::styled(format!("No tax rates for {}", y.label()), theme::title()),
                    Line::default(),
                    Line::from(format!("tally has rates for: {}.", years.join(", "))),
                    Line::from(format!(
                        "To estimate {}, add a [years.{}] block to {} — run `tally rates` to see the format — then press R.",
                        y.label(),
                        y.0,
                        crate::paths::display(&path)
                    )),
                    Line::styled(
                        "tally doesn't guess rates it hasn't been given.",
                        theme::muted(),
                    ),
                    Line::default(),
                    Line::styled("←/→ to change year", theme::muted()),
                ];
                f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), area);
                return;
            }
            Err(e) => {
                f.render_widget(
                    Paragraph::new(Line::styled(format!("{e:#}"), theme::error())),
                    area,
                );
                return;
            }
        };

        let (est, is_projected) = view.shown(self.tax_projected);
        let year = est.year;
        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(vec![
            Span::styled(format!("Tax estimate {}", year.label()), theme::title()),
            Span::styled(
                format!(
                    "   {} to {}",
                    dates::long(year.start()),
                    dates::long(year.end())
                ),
                theme::muted(),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Rates    ", theme::muted()),
            Span::raw(view.rates.provenance()),
        ]));
        let mut basis = vec![
            Span::styled("Figures  ", theme::muted()),
            Span::raw(view.basis_line(self.tax_projected)),
        ];
        if let Some((_, projected)) = &view.projected {
            let (label, other) = if is_projected {
                ("year to date", &view.actual)
            } else {
                ("projected full year", projected)
            };
            basis.push(Span::styled(
                format!("   ({label}: {}, p to switch)", other.total()),
                theme::muted(),
            ));
        }
        lines.push(Line::from(basis));

        for line in crate::tax::breakdown(est, &view.rates) {
            let amount = line.amount.map(|m| m.to_string()).unwrap_or_default();
            match line.kind {
                crate::tax::LineKind::Heading => {
                    lines.push(Line::default());
                    let mut spans =
                        vec![Span::styled(format!("{:<52}", line.label), theme::bold())];
                    if let Some(o) = line.obligation {
                        let style = match o {
                            crate::tax::Obligation::Required => theme::warning(),
                            crate::tax::Obligation::Optional => theme::accent(),
                            crate::tax::Obligation::NotPayable => theme::muted(),
                        };
                        spans.push(Span::styled(format!("{:>14}", o.label()), style));
                    }
                    lines.push(Line::from(spans));
                }
                crate::tax::LineKind::Note => {
                    lines.push(Line::styled(format!("  {}", line.label), theme::muted()))
                }
                crate::tax::LineKind::Item
                | crate::tax::LineKind::Subtotal
                | crate::tax::LineKind::Total => {
                    let style = match line.kind {
                        crate::tax::LineKind::Item => ratatui::style::Style::new(),
                        crate::tax::LineKind::Total => theme::title(),
                        _ => theme::bold(),
                    };
                    if line.kind == crate::tax::LineKind::Total {
                        lines.push(Line::default());
                    }
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {:<50}", line.label), style),
                        Span::styled(format!("{amount:>14}"), style),
                    ]));
                    if !line.detail.is_empty() {
                        lines.push(Line::styled(
                            format!("      {}", line.detail),
                            theme::muted(),
                        ));
                    }
                }
            }
        }

        let (payments, notes) = view.schedule(self.tax_projected);
        lines.push(Line::default());
        lines.push(Line::styled("Payments due to HMRC", theme::bold()));
        for p in &payments {
            let style = if p.due < today {
                theme::muted()
            } else {
                ratatui::style::Style::new()
            };
            lines.push(Line::styled(
                format!(
                    "  {}  {:<62}{:>14}",
                    dates::short(p.due),
                    p.what,
                    p.amount.to_string()
                ),
                style,
            ));
        }
        for n in notes {
            lines.push(Line::styled(format!("  {n}"), theme::muted()));
        }
        if let Some(n) = view.mtd_notice(self.tax_projected) {
            lines.push(Line::default());
            lines.push(Line::styled(n, theme::warning()));
        }
        lines.push(Line::default());
        lines.push(Line::styled("Sources", theme::bold()));
        for src in &view.rates.sources {
            lines.push(Line::styled(format!("  {src}"), theme::muted()));
        }
        lines.push(Line::default());
        lines.push(Line::styled(reports::TAX_DISCLAIMER, theme::muted()));

        let max_scroll = (lines.len() as u16).saturating_sub(area.height);
        self.tax_scroll = self.tax_scroll.min(max_scroll);
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((self.tax_scroll, 0)),
            area,
        );
    }
}
