//! Colours and styles in one place. Only the 16 named terminal colours, so
//! the user's terminal theme decides what they look like.

use ratatui::style::{Color, Modifier, Style};

use crate::engine::Status;

pub const ACCENT: Color = Color::Cyan;
pub const DIM: Color = Color::DarkGray;
pub const ON: Color = Color::Green;
pub const OK: Color = Color::Green;
pub const WARN: Color = Color::Yellow;
pub const BAD: Color = Color::Red;
pub const NEW: Color = Color::Blue;

pub fn title() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn heading() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// The cursor row in the focused pane.
pub fn selected() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// The cursor row in a pane without focus.
pub fn selected_inactive() -> Style {
    Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

pub fn focused_border() -> Style {
    Style::default().fg(ACCENT)
}

pub fn border() -> Style {
    Style::default().fg(DIM)
}

pub fn dim() -> Style {
    Style::default().fg(DIM)
}

pub fn on() -> Style {
    Style::default().fg(ON)
}

pub fn off() -> Style {
    Style::default().fg(DIM)
}

pub fn new_item() -> Style {
    Style::default().fg(NEW).add_modifier(Modifier::BOLD)
}

pub fn badge_new() -> Style {
    Style::default()
        .fg(Color::White)
        .bg(NEW)
        .add_modifier(Modifier::BOLD)
}

pub fn badge_bad() -> Style {
    Style::default()
        .fg(Color::White)
        .bg(BAD)
        .add_modifier(Modifier::BOLD)
}

pub fn badge_ok() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(OK)
        .add_modifier(Modifier::BOLD)
}

pub fn badge_warn() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(WARN)
        .add_modifier(Modifier::BOLD)
}

pub fn toast() -> Style {
    Style::default().fg(Color::Black).bg(ACCENT)
}

/// Secondary text that still has to be readable.
pub fn help() -> Style {
    Style::default().fg(Color::Gray)
}

pub fn key() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn status(status: Status) -> Style {
    let s = Style::default().add_modifier(Modifier::BOLD);
    match status {
        Status::Success | Status::Pass => s.fg(OK),
        Status::Skipped | Status::Attention => s.fg(WARN),
        Status::Failed => s.fg(BAD),
        Status::Would => s.fg(ACCENT),
        Status::Cancelled => Style::default().fg(DIM),
    }
}

/// One-character mark for a step in the progress list.
pub fn status_mark(status: Status) -> &'static str {
    match status {
        Status::Success | Status::Pass => "✓",
        Status::Skipped => "–",
        Status::Failed => "✗",
        Status::Attention => "!",
        Status::Would => "~",
        Status::Cancelled => "·",
    }
}
