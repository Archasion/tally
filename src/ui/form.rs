//! A reusable keyboard-driven form: text inputs, choices and toggles.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Wrap};
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

use super::theme;

#[derive(Debug, Clone)]
pub enum FieldKind {
    Text,
    Choice(Vec<String>),
    Toggle,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub key: &'static str,
    pub label: String,
    pub kind: FieldKind,
    pub input: Input,
    pub choice: usize,
    pub toggle: bool,
    pub required: bool,
    pub hint: String,
    /// For a choice: a short name and description per option, listed in a
    /// panel beside the form so every option can be seen at once.
    pub overview: Vec<(String, String)>,
}

/// Width of the overview panel, including its border and padding.
const OVERVIEW_WIDTH: u16 = 42;
/// The fields need at least this much room before the panel is shown.
const MIN_FIELDS_WIDTH: u16 = 56;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Key handled; nothing for the caller to do.
    Continue,
    /// A field's value changed.
    Changed,
    Submit,
    Cancel,
    /// Focus tried to move below the last field (only when `wraps` is false).
    LeaveDown,
}

#[derive(Debug, Clone)]
pub struct Form {
    pub fields: Vec<Field>,
    pub focus: usize,
    pub error: Option<String>,
    /// Normal forms wrap focus and submit on Enter at the last field.
    /// Embedded forms (the invoice editor header) report `LeaveDown` instead.
    pub wraps: bool,
}

impl Form {
    pub fn new() -> Self {
        Form {
            fields: Vec::new(),
            focus: 0,
            error: None,
            wraps: true,
        }
    }

    pub fn embedded(mut self) -> Self {
        self.wraps = false;
        self
    }

    fn push(mut self, key: &'static str, label: &str, kind: FieldKind) -> Self {
        self.fields.push(Field {
            key,
            label: label.to_string(),
            kind,
            input: Input::default(),
            choice: 0,
            toggle: false,
            required: false,
            hint: String::new(),
            overview: Vec::new(),
        });
        self
    }

    pub fn text(self, key: &'static str, label: &str, value: impl Into<String>) -> Self {
        let mut f = self.push(key, label, FieldKind::Text);
        f.fields.last_mut().expect("just pushed").input = Input::new(value.into());
        f
    }

    pub fn choice(
        self,
        key: &'static str,
        label: &str,
        options: Vec<String>,
        selected: usize,
    ) -> Self {
        let selected = selected.min(options.len().saturating_sub(1));
        let mut f = self.push(key, label, FieldKind::Choice(options));
        f.fields.last_mut().expect("just pushed").choice = selected;
        f
    }

    pub fn toggle(self, key: &'static str, label: &str, value: bool) -> Self {
        let mut f = self.push(key, label, FieldKind::Toggle);
        f.fields.last_mut().expect("just pushed").toggle = value;
        f
    }

    /// Marks the most recently added field as required.
    pub fn required(mut self) -> Self {
        if let Some(f) = self.fields.last_mut() {
            f.required = true;
        }
        self
    }

    /// Sets help text for the most recently added field.
    pub fn hint(mut self, hint: &str) -> Self {
        if let Some(f) = self.fields.last_mut() {
            f.hint = hint.to_string();
        }
        self
    }

    /// Lists every option of the most recent choice field beside the form,
    /// as `(short name, description)` pairs in option order.
    pub fn overview(mut self, entries: Vec<(String, String)>) -> Self {
        if let Some(f) = self.fields.last_mut() {
            f.overview = entries;
        }
        self
    }

    fn overview_field(&self) -> Option<&Field> {
        self.fields.iter().find(|f| !f.overview.is_empty())
    }

    /// Extra width the overview panel needs, or 0 if there's none.
    pub fn side_width(&self) -> u16 {
        if self.overview_field().is_some() {
            OVERVIEW_WIDTH
        } else {
            0
        }
    }

    fn field(&self, key: &str) -> &Field {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .unwrap_or_else(|| panic!("no form field '{key}'"))
    }

    fn field_mut(&mut self, key: &str) -> &mut Field {
        self.fields
            .iter_mut()
            .find(|f| f.key == key)
            .unwrap_or_else(|| panic!("no form field '{key}'"))
    }

    pub fn value(&self, key: &str) -> String {
        self.field(key).input.value().trim().to_string()
    }

    pub fn set_value(&mut self, key: &str, value: impl Into<String>) {
        self.field_mut(key).input = Input::new(value.into());
    }

    pub fn selected(&self, key: &str) -> usize {
        self.field(key).choice
    }

    pub fn is_on(&self, key: &str) -> bool {
        self.field(key).toggle
    }

    pub fn focused_key(&self) -> &'static str {
        self.fields[self.focus].key
    }

    /// Returns the first empty required field as an error, focusing it.
    pub fn check_required(&mut self) -> Result<(), String> {
        if let Some(i) = self.fields.iter().position(|f| {
            f.required && matches!(f.kind, FieldKind::Text) && f.input.value().trim().is_empty()
        }) {
            self.focus = i;
            return Err(format!("{} is required", self.fields[i].label));
        }
        Ok(())
    }

    /// Shows an error and focuses the named field.
    pub fn fail(&mut self, key: &str, message: impl Into<String>) {
        if let Some(i) = self.fields.iter().position(|f| f.key == key) {
            self.focus = i;
        }
        self.error = Some(message.into());
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('s') {
            return Outcome::Submit;
        }
        let last = self.fields.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Tab | KeyCode::Down => return self.next(false),
            KeyCode::Enter => return self.next(true),
            KeyCode::BackTab | KeyCode::Up => {
                self.focus = if self.focus == 0 {
                    if self.wraps { last } else { 0 }
                } else {
                    self.focus - 1
                };
                return Outcome::Continue;
            }
            _ => {}
        }
        let field = &mut self.fields[self.focus];
        match &field.kind {
            FieldKind::Text => {
                let before = field.input.value().to_string();
                field.input.handle_event(&Event::Key(key));
                if field.input.value() != before {
                    self.error = None;
                    return Outcome::Changed;
                }
                Outcome::Continue
            }
            FieldKind::Choice(options) => {
                let n = options.len().max(1);
                match key.code {
                    KeyCode::Left | KeyCode::Char('h') => field.choice = (field.choice + n - 1) % n,
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
                        field.choice = (field.choice + 1) % n
                    }
                    _ => return Outcome::Continue,
                }
                self.error = None;
                Outcome::Changed
            }
            FieldKind::Toggle => match key.code {
                KeyCode::Left
                | KeyCode::Right
                | KeyCode::Char(' ')
                | KeyCode::Char('y')
                | KeyCode::Char('n') => {
                    field.toggle = match key.code {
                        KeyCode::Char('y') => true,
                        KeyCode::Char('n') => false,
                        _ => !field.toggle,
                    };
                    self.error = None;
                    Outcome::Changed
                }
                _ => Outcome::Continue,
            },
        }
    }

    fn next(&mut self, is_enter: bool) -> Outcome {
        if self.focus + 1 < self.fields.len() {
            self.focus += 1;
            Outcome::Continue
        } else if !self.wraps {
            Outcome::LeaveDown
        } else if is_enter {
            Outcome::Submit
        } else {
            self.focus = 0;
            Outcome::Continue
        }
    }

    /// Height needed to show every field plus the hint/error line, and the
    /// overview panel if there is one.
    pub fn height(&self) -> u16 {
        self.height_in(u16::MAX)
    }

    /// Height when rendered `width` wide, where the panel may not fit.
    pub fn height_in(&self, width: u16) -> u16 {
        let fields = self.fields.len() as u16 + 1 + self.hint_height();
        if width < MIN_FIELDS_WIDTH + OVERVIEW_WIDTH {
            return fields;
        }
        match self.overview_field() {
            // Heading, options, gap, then up to two lines of name and three of description.
            Some(f) => fields.max(f.overview.len() as u16 + 7),
            None => fields,
        }
    }

    /// Pop-up forms let a long hint wrap onto a second line; the embedded
    /// editor header keeps one so the lines below don't move.
    fn hint_height(&self) -> u16 {
        if self.wraps { 2 } else { 1 }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, active: bool) {
        let panel_fits = area.width >= MIN_FIELDS_WIDTH + OVERVIEW_WIDTH;
        let area = match self.overview_field() {
            Some(field) if panel_fits => {
                let [main, side] =
                    Layout::horizontal([Constraint::Fill(1), Constraint::Length(OVERVIEW_WIDTH)])
                        .areas(area);
                let focused = active && self.fields[self.focus].key == field.key;
                render_overview(frame, side, field, focused);
                main
            }
            _ => area,
        };
        // The hint sits right under the fields, even when the panel is taller.
        let [fields_area, _, hint_area, _] = Layout::vertical([
            Constraint::Max(self.fields.len() as u16),
            Constraint::Length(1),
            Constraint::Length(self.hint_height()),
            Constraint::Min(0),
        ])
        .areas(area);
        let label_width = self
            .fields
            .iter()
            .map(|f| f.label.chars().count())
            .max()
            .unwrap_or(0) as u16
            + 2;
        let visible = fields_area.height as usize;
        // Scroll so the focused field is always visible.
        let start = if self.focus >= visible {
            self.focus + 1 - visible
        } else {
            0
        };

        for (row, (i, field)) in self
            .fields
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .enumerate()
        {
            let y = fields_area.y + row as u16;
            let focused = active && i == self.focus;
            let label_area = Rect::new(fields_area.x, y, label_width.min(fields_area.width), 1);
            let value_area = Rect::new(
                fields_area.x + label_width,
                y,
                fields_area.width.saturating_sub(label_width),
                1,
            );
            let marker = if field.required { "*" } else { " " };
            let label_style = if focused {
                theme::focused_label()
            } else {
                theme::label()
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(field.label.clone(), label_style),
                    Span::styled(marker, theme::muted()),
                ])),
                label_area,
            );
            let value_style = if focused {
                theme::focused_value()
            } else {
                theme::value()
            };
            match &field.kind {
                FieldKind::Text => {
                    let width = value_area.width.max(1) as usize;
                    let scroll = field.input.visual_scroll(width.saturating_sub(1));
                    frame.render_widget(
                        Paragraph::new(field.input.value())
                            .style(value_style)
                            .scroll((0, scroll as u16)),
                        value_area,
                    );
                    if focused {
                        let x = field.input.visual_cursor().saturating_sub(scroll) as u16;
                        frame.set_cursor_position((
                            value_area.x + x.min(value_area.width.saturating_sub(1)),
                            y,
                        ));
                    }
                }
                FieldKind::Choice(options) => {
                    let current = options.get(field.choice).cloned().unwrap_or_default();
                    let text = if focused {
                        format!("◀ {current} ▶")
                    } else {
                        current
                    };
                    frame.render_widget(Paragraph::new(text).style(value_style), value_area);
                }
                FieldKind::Toggle => {
                    let text = if field.toggle { "[x] Yes" } else { "[ ] No" };
                    frame.render_widget(Paragraph::new(text).style(value_style), value_area);
                }
            }
        }

        let hint_line = if let Some(err) = &self.error {
            Line::styled(err.clone(), theme::error())
        } else if active {
            let field = &self.fields[self.focus];
            // Without room for the panel, describe the selected option here instead.
            let mut hint = match field.overview.get(field.choice) {
                Some((_, description)) if !panel_fits => description.clone(),
                _ => field.hint.clone(),
            };
            let extra = match field.kind {
                FieldKind::Choice(_) => "←/→ to change",
                FieldKind::Toggle => "space to toggle",
                FieldKind::Text => "",
            };
            if !extra.is_empty() {
                if !hint.is_empty() {
                    hint.push_str(" · ");
                }
                hint.push_str(extra);
            }
            Line::styled(hint, theme::muted())
        } else {
            Line::default()
        };
        frame.render_widget(
            Paragraph::new(hint_line).wrap(Wrap { trim: true }),
            hint_area,
        );
    }
}

/// The overview panel: every option, the selected one marked, then its full
/// name and description, wrapped.
fn render_overview(frame: &mut Frame, area: Rect, field: &Field, focused: bool) {
    let FieldKind::Choice(options) = &field.kind else {
        return;
    };
    let mut lines = vec![Line::styled(field.label.clone(), theme::header_row())];
    for (i, (short, _)) in field.overview.iter().enumerate() {
        lines.push(if i == field.choice {
            let style = if focused {
                theme::selected_row()
            } else {
                theme::bold()
            };
            Line::styled(format!("▸ {short}"), style)
        } else {
            Line::styled(format!("  {short}"), theme::muted())
        });
    }
    lines.push(Line::default());
    if let Some(name) = options.get(field.choice) {
        lines.push(Line::styled(name.clone(), theme::accent()));
    }
    if let Some((_, description)) = field.overview.get(field.choice) {
        lines.push(Line::styled(description.clone(), theme::muted()));
    }
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(theme::muted())
        .padding(Padding::left(1));
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        area,
    );
}

impl Default for Form {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_navigation_and_submit() {
        let mut f = Form::new().text("a", "A", "").required().choice(
            "b",
            "B",
            vec!["x".into(), "y".into()],
            0,
        );
        f.handle_key(key(KeyCode::Char('h')));
        f.handle_key(key(KeyCode::Char('i')));
        assert_eq!(f.value("a"), "hi");
        assert_eq!(f.handle_key(key(KeyCode::Enter)), Outcome::Continue);
        assert_eq!(f.focused_key(), "b");
        f.handle_key(key(KeyCode::Right));
        assert_eq!(f.selected("b"), 1);
        assert_eq!(f.handle_key(key(KeyCode::Enter)), Outcome::Submit);
        assert_eq!(f.handle_key(key(KeyCode::Tab)), Outcome::Continue);
        assert_eq!(f.focused_key(), "a");
    }

    #[test]
    fn required_fields_are_checked() {
        let mut f = Form::new()
            .text("a", "Name", "")
            .required()
            .text("b", "Other", "x");
        f.focus = 1;
        assert_eq!(f.check_required().unwrap_err(), "Name is required");
        assert_eq!(f.focus, 0);
    }

    #[test]
    fn overview_adds_width_and_height() {
        let plain = Form::new().choice("c", "C", vec!["a".into(), "b".into()], 0);
        assert_eq!(plain.side_width(), 0);
        let entries = (0..12).map(|i| (format!("{i}"), String::new())).collect();
        let listed = Form::new()
            .choice("c", "C", (0..12).map(|i| format!("{i}")).collect(), 0)
            .overview(entries);
        assert_eq!(listed.side_width(), OVERVIEW_WIDTH);
        assert_eq!(listed.height(), 19);
        assert_eq!(listed.height_in(80), 4);
        // Pop-ups leave room for a two-line hint; the embedded editor header doesn't.
        assert_eq!(plain.height(), 4);
        assert_eq!(plain.embedded().height(), 3);
    }

    #[test]
    fn embedded_forms_report_leaving() {
        let mut f = Form::new().text("a", "A", "").embedded();
        assert_eq!(f.handle_key(key(KeyCode::Down)), Outcome::LeaveDown);
        assert_eq!(f.handle_key(key(KeyCode::Enter)), Outcome::LeaveDown);
    }
}
