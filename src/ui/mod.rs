//! The full-screen terminal app.
//!
//! `App` owns the database and all UI state. Each screen lives in its own
//! module and adds `draw_*` / `key_*` methods to `App`.

mod business_form;
mod clients;
mod dashboard;
mod editor;
mod form;
mod invoices;
mod modals;
mod report_view;
mod theme;

use std::path::PathBuf;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, TableState, Tabs, Wrap};
use ratatui::{DefaultTerminal, Frame};

use crate::db::Db;
use crate::domain::dates::{self, TaxYear};
use crate::domain::{Business, Client};
use crate::pdf::Renderer;
use crate::reports::{self, InvoiceRow};
use crate::{backup, paths};

use editor::Editor;
use form::Form;
use invoices::{Detail, Filter};
use modals::{Action, Purpose};
use report_view::ReportTab;

pub fn run(db: Db) -> Result<()> {
    let mut app = App::new(db)?;
    let mut terminal = ratatui::init();
    let result = app.event_loop(&mut terminal);
    ratatui::restore();
    result?;
    // Back up on the way out, after the terminal is restored so messages are visible.
    if let Some(b) = &app.business {
        match backup::run(&app.db, &paths::expand_home(&b.output_dir), b.backups_to_keep) {
            Ok(path) => println!("Backed up to {}", path.display()),
            Err(e) => eprintln!("warning: backup failed: {e:#}"),
        }
    }
    Ok(())
}

pub(crate) enum Screen {
    Wizard(Form),
    Dashboard,
    Invoices,
    Detail(i64),
    Editor(Box<Editor>),
    Clients,
    Reports,
    Settings(Form),
}

impl Screen {
    fn tab(&self) -> usize {
        match self {
            Screen::Wizard(_) | Screen::Dashboard => 0,
            Screen::Invoices | Screen::Detail(_) | Screen::Editor(_) => 1,
            Screen::Clients => 2,
            Screen::Reports => 3,
            Screen::Settings(_) => 4,
        }
    }

    /// Screens that capture typing, where global shortcuts are disabled.
    fn captures_text(&self) -> bool {
        matches!(self, Screen::Wizard(_) | Screen::Editor(_) | Screen::Settings(_))
    }
}

pub(crate) enum Modal {
    Form { title: String, form: Form, purpose: Purpose },
    Confirm { message: String, action: Action },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToastKind {
    Info,
    Success,
    Error,
}

pub(crate) struct App {
    pub db: Db,
    pub business: Option<Business>,
    renderer: Option<Renderer>,
    pub screen: Screen,
    pub modal: Option<Modal>,
    pub toast: Option<(String, ToastKind)>,
    pub quit: bool,

    pub rows: Vec<InvoiceRow>,
    pub clients: Vec<Client>,

    pub dash_table: TableState,
    pub invoice_filter: Filter,
    pub invoice_table: TableState,
    pub detail: Option<Detail>,
    pub client_table: TableState,
    pub show_archived: bool,
    pub report_tab: ReportTab,
    pub report_year: TaxYear,
    pub aging_table: TableState,
    pub last_export: Option<PathBuf>,
}

impl App {
    fn new(db: Db) -> Result<Self> {
        let business = db.business()?;
        let screen = match &business {
            Some(_) => Screen::Dashboard,
            None => Screen::Wizard(business_form::form(None)),
        };
        let mut app = App {
            db,
            business,
            renderer: None,
            screen,
            modal: None,
            toast: None,
            quit: false,
            rows: Vec::new(),
            clients: Vec::new(),
            dash_table: TableState::default().with_selected(0),
            invoice_filter: Filter::All,
            invoice_table: TableState::default().with_selected(0),
            detail: None,
            client_table: TableState::default().with_selected(0),
            show_archived: false,
            report_tab: ReportTab::Aging,
            report_year: reports::current_tax_year(),
            aging_table: TableState::default().with_selected(0),
            last_export: None,
        };
        app.refresh()?;
        Ok(app)
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.on_key(key);
            }
        }
        Ok(())
    }

    /// Reloads cached data after any change.
    pub fn refresh(&mut self) -> Result<()> {
        self.rows = reports::invoice_rows(&self.db, dates::today())?;
        self.clients = self.db.clients(true)?;
        if let Screen::Detail(id) = self.screen {
            self.detail = Some(Detail::load(self, id)?);
        }
        Ok(())
    }

    pub fn output_dir(&self) -> PathBuf {
        self.business
            .as_ref()
            .map(|b| paths::expand_home(&b.output_dir))
            .unwrap_or_default()
    }

    /// The PDF renderer, created on first use (font loading takes a moment).
    pub fn renderer(&mut self) -> Result<&Renderer> {
        if self.renderer.is_none() {
            self.renderer = Some(Renderer::new(&self.output_dir()).context("loading PDF template and fonts")?);
        }
        Ok(self.renderer.as_ref().expect("just set"))
    }

    /// Forces the template/fonts to be reloaded (after settings change or to pick up edits).
    pub fn reset_renderer(&mut self) {
        self.renderer = None;
    }

    pub fn info(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), ToastKind::Info));
    }

    pub fn success(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), ToastKind::Success));
    }

    pub fn error(&mut self, err: &anyhow::Error) {
        self.toast = Some((format!("{err:#}"), ToastKind::Error));
    }

    pub fn confirm(&mut self, message: impl Into<String>, action: Action) {
        self.modal = Some(Modal::Confirm { message: message.into(), action });
    }

    pub fn go(&mut self, screen: Screen) {
        self.screen = screen;
        if let Err(e) = self.refresh() {
            self.error(&e);
        }
    }

    pub fn open_detail(&mut self, id: i64) {
        self.detail = None;
        self.go(Screen::Detail(id));
    }

    fn on_key(&mut self, key: KeyEvent) {
        self.toast = None;
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.request_quit();
            return;
        }
        let result = if self.modal.is_some() {
            self.key_modal(key)
        } else {
            self.key_screen(key)
        };
        if let Err(e) = result {
            self.error(&e);
        }
    }

    fn request_quit(&mut self) {
        let dirty = matches!(&self.screen, Screen::Editor(e) if e.dirty);
        if dirty {
            self.confirm("Quit without saving this draft?", Action::Quit);
        } else {
            self.quit = true;
        }
    }

    fn key_screen(&mut self, key: KeyEvent) -> Result<()> {
        if !self.screen.captures_text() {
            match key.code {
                KeyCode::Char('q') => {
                    self.request_quit();
                    return Ok(());
                }
                KeyCode::Char('1') => {
                    self.go(Screen::Dashboard);
                    return Ok(());
                }
                KeyCode::Char('2') => {
                    self.go(Screen::Invoices);
                    return Ok(());
                }
                KeyCode::Char('3') => {
                    self.go(Screen::Clients);
                    return Ok(());
                }
                KeyCode::Char('4') => {
                    self.go(Screen::Reports);
                    return Ok(());
                }
                KeyCode::Char('5') => {
                    let form = business_form::form(self.business.as_ref());
                    self.go(Screen::Settings(form));
                    return Ok(());
                }
                _ => {}
            }
        }
        match self.screen {
            Screen::Wizard(_) | Screen::Settings(_) => self.key_business(key),
            Screen::Dashboard => self.key_dashboard(key),
            Screen::Invoices => self.key_invoices(key),
            Screen::Detail(id) => self.key_detail(id, key),
            Screen::Editor(_) => self.key_editor(key),
            Screen::Clients => self.key_clients(key),
            Screen::Reports => self.key_reports(key),
        }
    }

    // ----- drawing --------------------------------------------------------

    fn draw(&mut self, f: &mut Frame) {
        let [tabs_area, body, help_area, status_area] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(f.area());

        self.draw_tabs(f, tabs_area);
        let body = body.inner(ratatui::layout::Margin::new(1, 0));
        match self.screen {
            Screen::Wizard(_) | Screen::Settings(_) => self.draw_business(f, body),
            Screen::Dashboard => self.draw_dashboard(f, body),
            Screen::Invoices => self.draw_invoices(f, body),
            Screen::Detail(_) => self.draw_detail(f, body),
            Screen::Editor(_) => self.draw_editor(f, body),
            Screen::Clients => self.draw_clients(f, body),
            Screen::Reports => self.draw_reports(f, body),
        }
        let help = if self.modal.is_some() { self.modal_help() } else { self.help() };
        f.render_widget(Paragraph::new(help_line(&help)), help_area.inner(ratatui::layout::Margin::new(1, 0)));
        self.draw_status(f, status_area.inner(ratatui::layout::Margin::new(1, 0)));
        if self.modal.is_some() {
            self.draw_modal(f);
        }
    }

    fn draw_tabs(&self, f: &mut Frame, area: Rect) {
        let [line, _] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
        let [brand, tabs] = Layout::horizontal([Constraint::Length(8), Constraint::Min(1)]).areas(line);
        f.render_widget(Paragraph::new(Span::styled(" tally", theme::title())), brand);
        if matches!(self.screen, Screen::Wizard(_)) {
            f.render_widget(Paragraph::new(Span::styled("First-time setup", theme::bold())), tabs);
            return;
        }
        let titles = ["1 Dashboard", "2 Invoices", "3 Clients", "4 Reports", "5 Settings"];
        f.render_widget(
            Tabs::new(titles)
                .select(self.screen.tab())
                .highlight_style(theme::title().add_modifier(ratatui::style::Modifier::UNDERLINED))
                .style(theme::muted())
                .divider(" "),
            tabs,
        );
    }

    fn draw_status(&self, f: &mut Frame, area: Rect) {
        let line = match &self.toast {
            Some((msg, kind)) => {
                let style = match kind {
                    ToastKind::Info => theme::accent(),
                    ToastKind::Success => theme::success(),
                    ToastKind::Error => theme::error(),
                };
                Line::styled(msg.clone(), style)
            }
            None => Line::default(),
        };
        f.render_widget(Paragraph::new(line), area);
    }

    fn help(&self) -> Vec<(&'static str, &'static str)> {
        let mut keys = match &self.screen {
            Screen::Wizard(_) => vec![("↑↓/Tab", "move"), ("Enter", "next"), ("Ctrl+S", "save"), ("Esc", "quit")],
            Screen::Settings(_) => vec![
                ("↑↓/Tab", "move"),
                ("Ctrl+S", "save"),
                ("Ctrl+B", "back up now"),
                ("Ctrl+T", "reload template"),
                ("Esc", "back"),
            ],
            Screen::Dashboard => vec![("↑↓", "select"), ("Enter", "open"), ("n", "new invoice")],
            Screen::Invoices => vec![
                ("↑↓", "select"),
                ("Enter", "open"),
                ("n", "new"),
                ("f", "filter"),
                ("u", "duplicate"),
                ("o", "open PDF"),
            ],
            Screen::Detail(_) => self.detail_help(),
            Screen::Editor(e) => e.help(),
            Screen::Clients => vec![
                ("↑↓", "select"),
                ("n", "new"),
                ("Enter/e", "edit"),
                ("i", "invoice"),
                ("a", "archive"),
                ("h", "show archived"),
            ],
            Screen::Reports => self.reports_help(),
        };
        if !self.screen.captures_text() {
            keys.push(("q", "quit"));
        }
        keys
    }
}

pub(crate) fn help_line(keys: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (k, v)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", theme::muted()));
        }
        spans.push(Span::styled(k.to_string(), theme::key()));
        spans.push(Span::styled(format!(" {v}"), theme::muted()));
    }
    Line::from(spans)
}

/// A centred rectangle for popups.
pub(crate) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [h] = Layout::horizontal([Constraint::Length(width.min(area.width))]).flex(Flex::Center).areas(area);
    let [v] = Layout::vertical([Constraint::Length(height.min(area.height))]).flex(Flex::Center).areas(h);
    v
}

pub(crate) fn popup_block(title: &str) -> Block<'static> {
    Block::bordered()
        .title(Span::styled(format!(" {title} "), theme::title()))
        .border_style(theme::accent())
}

pub(crate) fn render_popup_text(f: &mut Frame, title: &str, text: Vec<Line<'static>>, width: u16) {
    let height = text.len() as u16 + 4;
    let area = centered(f.area(), width, height);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(popup_block(title).padding(ratatui::widgets::Padding::new(1, 1, 1, 0))),
        area,
    );
}

/// Moves a table selection by `delta`, clamped to `len`.
pub(crate) fn step(state: &mut TableState, len: usize, delta: isize) {
    if len == 0 {
        state.select(None);
        return;
    }
    let current = state.selected().unwrap_or(0) as isize;
    let next = (current + delta).clamp(0, len as isize - 1);
    state.select(Some(next as usize));
}

/// Common list navigation keys. Returns true if handled.
pub(crate) fn list_nav(state: &mut TableState, len: usize, key: KeyCode) -> bool {
    match key {
        KeyCode::Up | KeyCode::Char('k') => step(state, len, -1),
        KeyCode::Down | KeyCode::Char('j') => step(state, len, 1),
        KeyCode::PageUp => step(state, len, -10),
        KeyCode::PageDown => step(state, len, 10),
        KeyCode::Home | KeyCode::Char('g') => step(state, len, -(len as isize)),
        KeyCode::End | KeyCode::Char('G') => step(state, len, len as isize),
        _ => return false,
    }
    true
}

/// Clamps a selection after the underlying list changed size.
pub(crate) fn clamp_selection(state: &mut TableState, len: usize) {
    match (state.selected(), len) {
        (_, 0) => state.select(None),
        (None, _) => state.select(Some(0)),
        (Some(i), n) if i >= n => state.select(Some(n - 1)),
        _ => {}
    }
}

pub(crate) fn open_path(path: &std::path::Path) -> Result<()> {
    opener::open(path).with_context(|| format!("opening {}", path.display()))
}
