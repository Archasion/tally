//! Colours and styles. Uses the terminal's 16-colour palette so it follows
//! your terminal theme (light or dark).

use ratatui::style::{Color, Modifier, Style};

use crate::domain::Status;

pub fn accent() -> Style {
    Style::new().fg(Color::Cyan)
}

pub fn title() -> Style {
    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
}

pub fn muted() -> Style {
    Style::new().fg(Color::DarkGray)
}

pub fn label() -> Style {
    Style::new()
}

pub fn focused_label() -> Style {
    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
}

pub fn value() -> Style {
    Style::new()
}

pub fn focused_value() -> Style {
    Style::new().add_modifier(Modifier::REVERSED)
}

pub fn selected_row() -> Style {
    Style::new().add_modifier(Modifier::REVERSED)
}

pub fn header_row() -> Style {
    Style::new()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::BOLD)
}

pub fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

pub fn error() -> Style {
    Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
}

pub fn success() -> Style {
    Style::new().fg(Color::Green)
}

pub fn warning() -> Style {
    Style::new().fg(Color::Yellow)
}

pub fn key() -> Style {
    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
}

pub fn status(s: Status) -> Style {
    match s {
        Status::Draft => Style::new().fg(Color::DarkGray),
        Status::Issued => Style::new().fg(Color::Blue),
        Status::Sent => Style::new().fg(Color::Cyan),
        Status::PartPaid => Style::new().fg(Color::Yellow),
        Status::Overdue => Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        Status::Paid => Style::new().fg(Color::Green),
        Status::Credited => Style::new().fg(Color::Magenta),
    }
}
