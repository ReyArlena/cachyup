//! Pieces shared by several screens: panes, key hints, modals, text input
//! and the scrolling output view.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::engine::Status;

use super::input::{Hit, HitMap};
use super::theme;

/// A bordered block. Only the focused pane gets a coloured border and title.
pub fn pane_block(title: &str, focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(if focused {
            theme::focused_border()
        } else {
            theme::border()
        })
        .title(Span::styled(
            format!(" {title} "),
            if focused {
                theme::title()
            } else {
                Style::default()
            },
        ))
}

/// Centre a box of the given size inside `area`.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [v] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    let [h] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(v);
    h
}

/// Bottom line of key hints. Each one is clickable and acts like its key.
pub fn draw_key_hints(frame: &mut Frame, area: Rect, hints: &[(&str, &str)], hits: &mut HitMap) {
    let mut spans: Vec<Span> = Vec::new();
    let mut x = area.x;
    for (key, label) in hints {
        let text = format!(" {key} ");
        let width = (text.chars().count() + label.chars().count() + 1) as u16;
        if let Some(c) = hint_char(key) {
            hits.add(Rect::new(x, area.y, width, 1), Hit::Key(c));
        }
        spans.push(Span::styled(text, theme::key()));
        spans.push(Span::styled(format!("{label} "), theme::help()));
        x = x.saturating_add(width);
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn hint_char(key: &str) -> Option<char> {
    match key {
        "Enter" => Some('\n'),
        "Space" => Some(' '),
        "Esc" => Some('\x1b'),
        "Tab" => Some('\t'),
        k if k.chars().count() == 1 => k.chars().next(),
        _ => None,
    }
}

/// Keep a cursor visible: the scroll offset for a list of `len` rows shown
/// `visible` at a time.
pub fn clamp_scroll(selected: usize, len: usize, visible: usize, offset: usize) -> usize {
    if len == 0 || visible == 0 {
        return 0;
    }
    let selected = selected.min(len - 1);
    if selected < offset {
        selected
    } else if selected >= offset + visible {
        selected + 1 - visible
    } else {
        offset.min(len.saturating_sub(visible))
    }
}

// --- modal ---

/// A key in a modal and what it means. `tag: None` just closes.
#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub keys: Vec<KeyCode>,
    pub hint: String,
    pub label: String,
    pub tag: Option<String>,
}

impl Button {
    pub fn new(keys: &[KeyCode], hint: &str, label: &str, tag: Option<&str>) -> Self {
        Self {
            keys: keys.to_vec(),
            hint: hint.into(),
            label: label.into(),
            tag: tag.map(String::from),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Modal {
    pub title: String,
    pub body: Vec<Line<'static>>,
    pub buttons: Vec<Button>,
    /// A text field, for modals that ask for a value.
    pub input: Option<TextInput>,
    /// Vertical scroll for long bodies.
    pub scroll: u16,
    /// Text `c` copies to the clipboard (a command to run).
    pub copy: Option<String>,
    /// Set when `c` was pressed; the app does the copy and clears it.
    pub copy_requested: bool,
}

impl Modal {
    pub fn new(title: &str, body: Vec<Line<'static>>, buttons: Vec<Button>) -> Self {
        Self {
            title: title.into(),
            body,
            buttons,
            input: None,
            scroll: 0,
            copy: None,
            copy_requested: false,
        }
    }

    pub fn text(text: &str) -> Vec<Line<'static>> {
        text.lines().map(|l| Line::from(l.to_string())).collect()
    }

    /// Just a message. Enter or Esc closes.
    pub fn message(title: &str, text: &str) -> Self {
        Self::new(
            title,
            Self::text(text),
            vec![Button::new(
                &[KeyCode::Enter, KeyCode::Esc, KeyCode::Char('q')],
                "Enter",
                "close",
                None,
            )],
        )
    }

    /// Yes/No. Enter or `y` confirms with `tag`.
    pub fn confirm(title: &str, text: &str, tag: &str) -> Self {
        Self::new(
            title,
            Self::text(text),
            vec![
                Button::new(
                    &[KeyCode::Enter, KeyCode::Char('y')],
                    "Enter",
                    "yes",
                    Some(tag),
                ),
                Button::new(&[KeyCode::Esc, KeyCode::Char('n')], "Esc", "no", None),
            ],
        )
    }

    /// Ask for a value. Enter confirms with `tag`; read it from `input`.
    pub fn input(title: &str, text: &str, initial: &str, tag: &str) -> Self {
        let mut m = Self::new(
            title,
            Self::text(text),
            vec![
                Button::new(&[KeyCode::Enter], "Enter", "save", Some(tag)),
                Button::new(&[KeyCode::Esc], "Esc", "cancel", None),
            ],
        );
        m.input = Some(TextInput::with_value(initial));
        m
    }

    pub fn input_value(&self) -> Option<String> {
        self.input.as_ref().map(|i| i.value.clone())
    }

    /// `Some(tag)` when a button was pressed (the modal closes), `None` to stay open.
    /// Let `c` copy `text`.
    pub fn with_copy(mut self, text: String) -> Self {
        self.copy = Some(text);
        self
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Option<String>> {
        if key.code == KeyCode::Char('c') && self.copy.is_some() && self.input.is_none() {
            self.copy_requested = true;
            return None;
        }
        if let Some(button) = self.buttons.iter().find(|b| b.keys.contains(&key.code)) {
            return Some(button.tag.clone());
        }
        if let Some(input) = &mut self.input {
            input.handle_key(key);
            return None;
        }
        match key.code {
            KeyCode::Esc => Some(None),
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll = self.scroll.saturating_sub(1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll = self.scroll.saturating_add(1);
                None
            }
            _ => None,
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let max_width = area.width.saturating_sub(4).max(1);
        let content = self
            .body
            .iter()
            .map(|l| l.width())
            .max()
            .unwrap_or(0)
            .max(self.title.len() + 4)
            .max(self.hint_line().width()) as u16;
        let width = (content + 4).clamp(36, max_width.max(36)).min(max_width);
        let inner_width = width.saturating_sub(4).max(1);
        // Rows the body takes once wrapped.
        let body_rows: u16 = self
            .body
            .iter()
            .map(|l| (l.width() as u16).div_ceil(inner_width).max(1))
            .sum();
        let extra = if self.input.is_some() { 2 } else { 0 };
        let wanted = body_rows + extra + 2 + 2;
        let height = wanted.min(area.height.saturating_sub(2).max(3));
        let rect = centered(area, width, height);
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::focused_border())
            .title(Span::styled(format!(" {} ", self.title), theme::title()));
        let inner = block.inner(rect).inner(ratatui::layout::Margin::new(1, 0));
        frame.render_widget(block, rect);

        let [body_area, input_area, hint_area] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(extra),
            Constraint::Length(2),
        ])
        .areas(inner);
        let max_scroll = body_rows.saturating_sub(body_area.height);
        self.scroll = self.scroll.min(max_scroll);
        frame.render_widget(
            Paragraph::new(self.body.clone())
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0)),
            body_area,
        );
        if let Some(input) = &self.input {
            frame.render_widget(
                Paragraph::new(vec![Line::from(""), input.line(true)]),
                input_area,
            );
        }
        let mut hint = vec![Line::from("")];
        let mut line = self.hint_line();
        if max_scroll > 0 {
            line.spans.push(Span::styled("  j/k scroll", theme::dim()));
        }
        hint.push(line);
        frame.render_widget(Paragraph::new(hint), hint_area);
    }

    fn hint_line(&self) -> Line<'static> {
        let mut spans = Vec::new();
        for (i, b) in self.buttons.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("   "));
            }
            spans.push(Span::styled(b.hint.clone(), theme::key()));
            spans.push(Span::raw(format!(" {}", b.label)));
        }
        if let Some(text) = &self.copy {
            spans.push(Span::raw("   "));
            spans.push(Span::styled("c", theme::key()));
            spans.push(Span::raw(if text.contains('\n') {
                " copy commands"
            } else {
                " copy command"
            }));
        }
        Line::from(spans)
    }
}

// --- text input ---

/// Minimal single-line editor.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextInput {
    pub value: String,
    pub cursor: usize,
}

impl TextInput {
    pub fn with_value(value: &str) -> Self {
        Self {
            value: value.to_string(),
            cursor: value.chars().count(),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c) => {
                let i = self.byte_index();
                self.value.insert(i, c);
                self.cursor += 1;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let i = self.byte_index();
                self.value.remove(i);
            }
            KeyCode::Delete if self.cursor < self.value.chars().count() => {
                let i = self.byte_index();
                self.value.remove(i);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.value.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.value.chars().count(),
            _ => {}
        }
    }

    fn byte_index(&self) -> usize {
        self.value
            .char_indices()
            .nth(self.cursor)
            .map_or(self.value.len(), |(i, _)| i)
    }

    pub fn line(&self, active: bool) -> Line<'static> {
        if !active {
            return Line::from(self.value.clone());
        }
        let chars: Vec<char> = self.value.chars().collect();
        let at = self.cursor.min(chars.len());
        let before: String = chars[..at].iter().collect();
        let under: String = chars.get(at).map_or(" ".into(), |c| c.to_string());
        let after: String = chars
            .get(at + 1..)
            .map_or(String::new(), |c| c.iter().collect());
        Line::from(vec![
            Span::raw(before),
            Span::styled(under, Style::default().reversed()),
            Span::raw(after),
        ])
    }
}

// --- output view ---

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutKind {
    Plain,
    Note,
    Heading,
    Status(Status),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutLine {
    pub text: String,
    pub kind: OutKind,
}

/// A scrolling terminal dump. Follows the end until the user scrolls up;
/// End (or G) follows again.
#[derive(Debug, Default)]
pub struct OutputView {
    pub lines: Vec<OutLine>,
    /// The line being redrawn with `\r` right now.
    pub partial: Option<String>,
    pub offset: usize,
    pub follow: bool,
    /// Rows shown at the last draw, for paging.
    height: usize,
}

/// Lines kept in memory; older ones are still in the log file.
const MAX_LINES: usize = 50_000;

impl OutputView {
    pub fn new() -> Self {
        Self {
            follow: true,
            ..Self::default()
        }
    }

    pub fn push(&mut self, kind: OutKind, text: String) {
        self.partial = None;
        if kind == OutKind::Heading && !self.lines.is_empty() {
            self.lines.push(OutLine {
                text: String::new(),
                kind: OutKind::Plain,
            });
        }
        self.lines.push(OutLine { text, kind });
        if self.lines.len() > MAX_LINES {
            let cut = self.lines.len() - MAX_LINES;
            self.lines.drain(..cut);
            self.offset = self.offset.saturating_sub(cut);
        }
    }

    /// Parse a saved log back into coloured lines.
    pub fn from_log(text: &str) -> Self {
        let mut view = Self::new();
        view.follow = false;
        for line in text.lines() {
            let kind = if let Some(h) = line.strip_prefix("==> ") {
                view.lines.push(OutLine {
                    text: format!("==> {h}"),
                    kind: OutKind::Heading,
                });
                continue;
            } else if let Some(status) = status_in_line(line) {
                OutKind::Status(status)
            } else if line.starts_with("    ") {
                OutKind::Note
            } else {
                OutKind::Plain
            };
            view.lines.push(OutLine {
                text: line.to_string(),
                kind,
            });
        }
        view
    }

    fn total(&self) -> usize {
        self.lines.len() + usize::from(self.partial.is_some())
    }

    fn max_offset(&self) -> usize {
        self.total().saturating_sub(self.height.max(1))
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let next = (self.offset as isize + delta).clamp(0, self.max_offset() as isize) as usize;
        self.offset = next;
        self.follow = self.offset >= self.max_offset();
    }

    pub fn page(&mut self, down: bool) {
        let step = self.height.saturating_sub(2).max(1) as isize;
        self.scroll_by(if down { step } else { -step });
    }

    pub fn scroll_to_start(&mut self) {
        self.offset = 0;
        self.follow = false;
    }

    pub fn scroll_to_end(&mut self) {
        self.follow = true;
        self.offset = self.max_offset();
    }

    /// Scroll so `line` is at the top.
    pub fn show_line(&mut self, line: usize) {
        self.offset = line.min(self.max_offset());
        self.follow = false;
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, block: Block<'static>) {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.height = inner.height as usize;
        if self.follow {
            self.offset = self.max_offset();
        }
        self.offset = self.offset.min(self.max_offset());
        let width = inner.width as usize;
        let mut rows: Vec<Line> = self
            .lines
            .iter()
            .skip(self.offset)
            .take(self.height)
            .map(|l| render_line(l, width))
            .collect();
        if let Some(p) = &self.partial
            && rows.len() < self.height
        {
            rows.push(Line::from(clip(p, width)));
        }
        frame.render_widget(Paragraph::new(rows), inner);
        if !self.follow && self.total() > self.height {
            let pos = format!(
                " {}/{} ",
                self.offset + self.height.min(self.total()),
                self.total()
            );
            let x = area.x + area.width.saturating_sub(pos.len() as u16 + 1);
            frame.render_widget(
                Paragraph::new(Span::styled(pos, theme::dim())),
                Rect::new(
                    x,
                    area.y + area.height.saturating_sub(1),
                    area.width.min(12),
                    1,
                ),
            );
        }
    }
}

fn clip(text: &str, width: usize) -> String {
    let text = text.replace('\t', "    ");
    if text.chars().count() <= width {
        text
    } else {
        text.chars().take(width).collect()
    }
}

fn render_line(line: &OutLine, width: usize) -> Line<'static> {
    let text = clip(&line.text, width);
    match line.kind {
        OutKind::Plain => Line::from(text),
        OutKind::Note => Line::from(Span::styled(text, theme::dim())),
        OutKind::Heading => Line::from(Span::styled(text, theme::heading())),
        OutKind::Status(status) => {
            let label = format!(".{}", status.label());
            match text.find(&label) {
                Some(at) => {
                    let at = at + 1;
                    let end = at + status.label().len();
                    Line::from(vec![
                        Span::raw(text[..at].to_string()),
                        Span::styled(text[at..end].to_string(), theme::status(status)),
                        Span::raw(text[end..].to_string()),
                    ])
                }
                None => Line::from(text),
            }
        }
    }
}

/// Find the status word of a `name.....STATUS (reason)` line.
fn status_in_line(line: &str) -> Option<Status> {
    use Status::*;
    [Success, Skipped, Failed, Pass, Attention, Would, Cancelled]
        .into_iter()
        .find(|s| {
            let label = format!("...{}", s.label());
            line.find(&label).is_some_and(|i| {
                let rest = &line[i + label.len()..];
                rest.is_empty() || rest.starts_with(" (")
            })
        })
}

/// Full-screen help overlay.
pub fn draw_help(frame: &mut Frame, area: Rect, screen_keys: &[(&str, &str)]) {
    let mut lines = vec![
        Line::from(Span::styled("Moving around", theme::title())),
        Line::from("  Arrows / h j k l    move, scroll"),
        Line::from("  PgUp PgDn g G       jump"),
        Line::from("  Tab                 other pane"),
        Line::from("  Mouse               click to select, wheel to scroll"),
        Line::from(""),
        Line::from(Span::styled("Everywhere", theme::title())),
        Line::from("  Space  toggle    Enter  confirm / open    Esc  back"),
        Line::from("  q      quit      ?      this help"),
        Line::from(""),
        Line::from(Span::styled("This screen", theme::title())),
    ];
    for (k, label) in screen_keys {
        lines.push(Line::from(format!("  {k:<7} {label}")));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press any key to close",
        theme::dim(),
    )));
    let rect = centered(area, 64, lines.len() as u16 + 2);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::focused_border())
        .title(Span::styled(" Help ", theme::title()))
        .title_alignment(Alignment::Center);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn modal_buttons_close_with_their_tag() {
        let mut m = Modal::confirm("t", "body", "go");
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        assert_eq!(m.handle_key(key(KeyCode::Char('x'))), None);
        assert_eq!(m.handle_key(key(KeyCode::Enter)), Some(Some("go".into())));
        assert_eq!(m.handle_key(key(KeyCode::Esc)), Some(None));
    }

    #[test]
    fn output_follows_until_scrolled() {
        let mut v = OutputView::new();
        v.height = 3;
        for i in 0..10 {
            v.push(OutKind::Plain, i.to_string());
        }
        v.scroll_to_end();
        assert_eq!(v.offset, 7);
        v.scroll_by(-2);
        assert!(!v.follow);
        assert_eq!(v.offset, 5);
        v.scroll_by(5);
        assert!(v.follow);
    }

    #[test]
    fn log_lines_get_their_kind_back() {
        let v = OutputView::from_log(
            "==> Cleanup\nOrphaned packages....SKIPPED (none)\n    $ ls\nplain\nx...FAILED\n",
        );
        let kinds: Vec<OutKind> = v.lines.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            [
                OutKind::Heading,
                OutKind::Status(Status::Skipped),
                OutKind::Note,
                OutKind::Plain,
                OutKind::Status(Status::Failed),
            ]
        );
    }

    #[test]
    fn scroll_offset_follows_selection() {
        assert_eq!(clamp_scroll(0, 10, 5, 0), 0);
        assert_eq!(clamp_scroll(7, 10, 5, 0), 3);
        assert_eq!(clamp_scroll(2, 10, 5, 4), 2);
    }
}
