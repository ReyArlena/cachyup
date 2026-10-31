//! The Dotfiles screen: GNU Stow packages on the left, the selected one's
//! links, warnings and stow preview on the right. Read-only for now.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::dotfiles::{ItemKind, Package, PkgState, Scan, Shape};
use crate::engine::Status;

use super::Effect;
use super::input::{Action, Hit, HitMap};
use super::theme;
use super::widgets::{clamp_scroll, pane_block};

const PANE_LIST: usize = 0;
const PANE_DETAIL: usize = 1;

/// What to scan: the stow folder, the home folder and packages to skip.
#[derive(Debug, Clone)]
pub struct Source {
    pub dir: PathBuf,
    pub home: PathBuf,
    pub ignore: Vec<String>,
    /// `dir` as the user wrote it (with `~`).
    pub shown: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Row {
    Header(String),
    /// Index into `scan.packages`.
    Package(usize),
}

pub struct DotfilesScreen {
    source: Option<Source>,
    scan: Option<Scan>,
    rx: Option<Receiver<Scan>>,
    rows: Vec<Row>,
    cursor: usize,
    offset: usize,
    focus: usize,
    detail_scroll: u16,
    /// Rows the details took at the last draw, and the rows shown.
    detail_rows: u16,
    detail_height: u16,
}

impl DotfilesScreen {
    pub fn new() -> Self {
        Self {
            source: None,
            scan: None,
            rx: None,
            rows: Vec::new(),
            cursor: 0,
            offset: 0,
            focus: PANE_LIST,
            detail_scroll: 0,
            detail_rows: 0,
            detail_height: 0,
        }
    }

    /// Scan in the background (stow runs once per package).
    pub fn open(&mut self, source: Source) {
        if self.source.as_ref().map(|s| &s.dir) != Some(&source.dir) {
            self.scan = None;
            self.rows.clear();
            self.cursor = 0;
        }
        self.focus = PANE_LIST;
        self.source = Some(source);
        self.rescan();
    }

    fn rescan(&mut self) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let scan = crate::dotfiles::scan(&source.dir, &source.home, &source.ignore, true);
            let _ = tx.send(scan);
        });
        self.rx = Some(rx);
    }

    pub fn scanning(&self) -> bool {
        self.rx.is_some()
    }

    /// Pick up a finished scan, keeping the cursor on the same package.
    pub fn poll(&mut self) {
        let Some(rx) = &self.rx else {
            return;
        };
        let Ok(scan) = rx.try_recv() else {
            return;
        };
        self.rx = None;
        let current = self.current().map(|p| p.name.clone());
        self.rows = build_rows(&scan);
        self.scan = Some(scan);
        self.cursor = current
            .and_then(|name| {
                self.rows.iter().position(|r| match r {
                    Row::Package(i) => self.package(*i).is_some_and(|p| p.name == name),
                    _ => false,
                })
            })
            .unwrap_or(0);
        self.snap(1);
    }

    fn package(&self, index: usize) -> Option<&Package> {
        self.scan.as_ref()?.packages.get(index)
    }

    fn current(&self) -> Option<&Package> {
        match self.rows.get(self.cursor)? {
            Row::Package(i) => self.package(*i),
            Row::Header(_) => None,
        }
    }

    /// Move off a header, in the direction of travel.
    fn snap(&mut self, dir: isize) {
        let len = self.rows.len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = self.cursor.min(len - 1);
        let mut c = self.cursor as isize;
        while (0..len as isize).contains(&c) {
            if matches!(self.rows[c as usize], Row::Package(_)) {
                self.cursor = c as usize;
                return;
            }
            c += dir;
        }
        // Nothing that way: try the other.
        if dir > 0 {
            self.snap(-1);
        } else if let Some(i) = self.rows.iter().position(|r| matches!(r, Row::Package(_))) {
            self.cursor = i;
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let before = self.cursor;
        let len = self.rows.len() as isize;
        if len == 0 {
            return;
        }
        let mut c = self.cursor as isize;
        let mut left = delta.abs();
        while left > 0 {
            let next = c + delta.signum();
            if !(0..len).contains(&next) {
                break;
            }
            c = next;
            if matches!(self.rows[c as usize], Row::Package(_)) {
                left -= 1;
                self.cursor = c as usize;
            }
        }
        if self.cursor != before {
            self.detail_scroll = 0;
        }
    }

    fn scroll_detail(&mut self, delta: i32) {
        let max = self.detail_rows.saturating_sub(self.detail_height) as i32;
        self.detail_scroll = (self.detail_scroll as i32 + delta).clamp(0, max.max(0)) as u16;
    }

    pub fn handle(&mut self, action: Action, hits: &HitMap) -> Vec<Effect> {
        let action = match action {
            Action::Click { x, y } => {
                match hits.find(x, y) {
                    Some(Hit::Row {
                        pane: PANE_LIST,
                        index,
                    }) => {
                        self.focus = PANE_LIST;
                        if *index != self.cursor {
                            self.cursor = *index;
                            self.detail_scroll = 0;
                        }
                    }
                    Some(Hit::Pane(p)) => self.focus = *p,
                    _ => {}
                }
                return Vec::new();
            }
            Action::Scroll { x, y, up } => {
                if hits.pane_at(x, y) == Some(PANE_DETAIL) {
                    self.scroll_detail(if up { -3 } else { 3 });
                    return Vec::new();
                }
                if up { Action::Up } else { Action::Down }
            }
            other => other,
        };
        match action {
            Action::Back if self.focus == PANE_DETAIL => {
                self.focus = PANE_LIST;
                return Vec::new();
            }
            Action::Back | Action::Char('D') => return vec![Effect::CloseDotfiles],
            Action::NextPane | Action::PrevPane => {
                self.focus = 1 - self.focus;
                return Vec::new();
            }
            Action::Char('r') => {
                self.rescan();
                return vec![Effect::Toast("rescanning".into())];
            }
            Action::Char('p') => return vec![Effect::AskDotfilesDir],
            _ => {}
        }
        if self.focus == PANE_LIST {
            match action {
                Action::Up => self.move_cursor(-1),
                Action::Down => self.move_cursor(1),
                Action::PageUp => self.move_cursor(-10),
                Action::PageDown => self.move_cursor(10),
                Action::Home => {
                    self.cursor = 0;
                    self.snap(1);
                    self.detail_scroll = 0;
                }
                Action::End => {
                    self.cursor = self.rows.len().saturating_sub(1);
                    self.snap(-1);
                    self.detail_scroll = 0;
                }
                Action::Confirm | Action::Right => self.focus = PANE_DETAIL,
                _ => {}
            }
        } else {
            let page = self.detail_height.max(1) as i32;
            match action {
                Action::Up => self.scroll_detail(-1),
                Action::Down => self.scroll_detail(1),
                Action::PageUp => self.scroll_detail(-page),
                Action::PageDown => self.scroll_detail(page),
                Action::Home => self.detail_scroll = 0,
                Action::End => self.scroll_detail(i32::MAX / 2),
                Action::Left => self.focus = PANE_LIST,
                _ => {}
            }
        }
        Vec::new()
    }

    pub fn key_hints(&self) -> Vec<(&'static str, &'static str)> {
        let mut hints = vec![("Tab", "switch pane")];
        if self.focus == PANE_LIST {
            hints.push(("Enter", "details"));
        }
        hints.extend([
            ("r", "rescan"),
            ("p", "folder"),
            ("Esc", "back"),
            ("?", "help"),
            ("q", "quit"),
        ]);
        hints
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, hits: &mut HitMap) {
        let [left, right] =
            Layout::horizontal([Constraint::Length(58), Constraint::Min(20)]).areas(area);
        self.draw_list(frame, left, hits);
        self.draw_detail(frame, right, hits);
    }

    fn draw_list(&mut self, frame: &mut Frame, area: Rect, hits: &mut HitMap) {
        let focused = self.focus == PANE_LIST;
        let title = match &self.source {
            Some(s) => format!("Dotfiles {}", s.shown),
            None => "Dotfiles".into(),
        };
        let block = pane_block(&title, focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        hits.add(area, Hit::Pane(PANE_LIST));

        let Some(scan) = &self.scan else {
            frame.render_widget(
                Paragraph::new(Span::styled(" Scanning...", theme::dim())),
                inner,
            );
            return;
        };
        if let Some(err) = &scan.error {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(
                        format!(" {err}"),
                        theme::status(Status::Failed),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        " Press p to choose another folder.",
                        theme::dim(),
                    )),
                ])
                .wrap(Wrap { trim: false }),
                inner,
            );
            return;
        }
        if scan.packages.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(" No packages in this folder", theme::dim())),
                inner,
            );
            return;
        }

        let name_width = scan
            .packages
            .iter()
            .map(|p| p.name.chars().count())
            .max()
            .unwrap_or(0)
            .min(30);
        let height = inner.height as usize;
        self.offset = clamp_scroll(self.cursor, self.rows.len(), height, self.offset);
        let mut lines = Vec::new();
        for (i, row) in self.rows.iter().enumerate().skip(self.offset).take(height) {
            let y = inner.y + (i - self.offset) as u16;
            let mut line = match row {
                Row::Header(text) => Line::from(Span::styled(format!(" {text}"), theme::heading())),
                Row::Package(index) => {
                    hits.add(
                        Rect::new(inner.x, y, inner.width, 1),
                        Hit::Row {
                            pane: PANE_LIST,
                            index: i,
                        },
                    );
                    let p = &scan.packages[*index];
                    let (mark, style) = state_mark(&p.state);
                    let mut spans = vec![
                        Span::styled(format!("  {mark} "), style),
                        Span::raw(format!("{:<name_width$}  ", p.name)),
                        Span::styled(state_label(&p.state), style),
                    ];
                    if let Some(w) = warn_mark(p) {
                        spans.push(Span::raw(" "));
                        spans.push(w);
                    }
                    Line::from(spans)
                }
            };
            if i == self.cursor {
                line = line.style(if focused {
                    theme::selected()
                } else {
                    theme::selected_inactive()
                });
            }
            lines.push(line);
        }
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn draw_detail(&mut self, frame: &mut Frame, area: Rect, hits: &mut HitMap) {
        hits.add(area, Hit::Pane(PANE_DETAIL));
        let focused = self.focus == PANE_DETAIL;
        let title = self.current().map_or("Details".into(), |p| p.name.clone());
        let block = pane_block(&title, focused);
        let inner = block.inner(area).inner(ratatui::layout::Margin::new(1, 0));
        frame.render_widget(block, area);

        let mut lines = Vec::new();
        if let Some(scan) = &self.scan {
            if scan.error.is_none() && !scan.stow {
                lines.push(Line::from(Span::styled(
                    "! stow is not installed, so there is no preview of what it would do. Install it with: sudo pacman -S stow",
                    theme::status(Status::Attention),
                )));
                lines.push(Line::from(""));
            }
            if let (Some(p), Some(source)) = (self.current(), &self.source) {
                lines.extend(detail_lines(
                    p,
                    source,
                    scan.stow,
                    scan.git,
                    inner.width as usize,
                ));
            }
        }
        if self.scanning() && self.scan.is_some() {
            lines.insert(0, Line::from(Span::styled("Rescanning...", theme::dim())));
        }

        let width = inner.width.max(1);
        // Most lines are wrapped already; this counts any that are not.
        self.detail_rows = lines
            .iter()
            .map(|l| (l.width() as u16).div_ceil(width).max(1))
            .sum();
        self.detail_height = inner.height;
        let max = self.detail_rows.saturating_sub(inner.height);
        self.detail_scroll = self.detail_scroll.min(max);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((self.detail_scroll, 0)),
            inner,
        );
    }
}

/// Packages grouped: linked first, then not linked, then folders that are
/// not packages.
fn build_rows(scan: &Scan) -> Vec<Row> {
    let group = |s: &PkgState| match s {
        PkgState::Linked(_) | PkgState::Partial { .. } => 0,
        PkgState::NotLinked | PkgState::TakenBy(_) | PkgState::Blocked => 1,
        PkgState::NotPackage | PkgState::Empty => 2,
    };
    let titles = ["Linked", "Not linked", "Not stow packages"];
    let mut rows = Vec::new();
    for (g, title) in titles.iter().enumerate() {
        let members: Vec<usize> = scan
            .packages
            .iter()
            .enumerate()
            .filter(|(_, p)| group(&p.state) == g)
            .map(|(i, _)| i)
            .collect();
        if members.is_empty() {
            continue;
        }
        if !rows.is_empty() {
            rows.push(Row::Header(String::new()));
        }
        rows.push(Row::Header(format!("{title} ({})", members.len())));
        rows.extend(members.into_iter().map(Row::Package));
    }
    rows
}

fn state_mark(state: &PkgState) -> (&'static str, Style) {
    match state {
        PkgState::Linked(_) => ("✓", theme::on()),
        PkgState::Partial { .. } => ("~", theme::status(Status::Attention)),
        PkgState::Blocked => ("✗", theme::status(Status::Failed)),
        PkgState::NotLinked | PkgState::TakenBy(_) => ("·", theme::dim()),
        PkgState::NotPackage | PkgState::Empty => ("?", theme::dim()),
    }
}

fn state_label(state: &PkgState) -> String {
    match state {
        PkgState::Linked(Shape::Folders) => "folder link".into(),
        PkgState::Linked(Shape::Files) => "file links".into(),
        PkgState::Linked(Shape::Mixed) => "folder + file links".into(),
        PkgState::Partial { linked, total } => format!("partial {linked}/{total}"),
        PkgState::NotLinked => "not linked".into(),
        PkgState::TakenBy(other) => format!("used by {other}"),
        PkgState::Blocked => "blocked".into(),
        PkgState::NotPackage => "not a package".into(),
        PkgState::Empty => "empty".into(),
    }
}

/// `!` after a package with warnings: red when one is severe.
fn warn_mark(p: &Package) -> Option<Span<'static>> {
    if p.warnings.is_empty() {
        return None;
    }
    let severe = p.warnings.iter().any(|w| w.severe);
    Some(Span::styled(
        "!",
        theme::status(if severe {
            Status::Failed
        } else {
            Status::Attention
        }),
    ))
}

fn state_sentence(state: &PkgState) -> String {
    match state {
        PkgState::Linked(Shape::Folders) => {
            "Linked. Each folder below is a single link into the package, so new files in it are picked up without restowing.".into()
        }
        PkgState::Linked(Shape::Files) => {
            "Linked. Each file is its own link inside a real folder, so new files in the package need a restow.".into()
        }
        PkgState::Linked(Shape::Mixed) => {
            "Linked, partly as whole-folder links and partly as single-file links.".into()
        }
        PkgState::Partial { linked, total } => {
            format!("Partly linked: {linked} of {total} paths.")
        }
        PkgState::NotLinked => "Not linked. Nothing is in the way.".into(),
        PkgState::TakenBy(other) => format!("Not linked: its folder is linked from {other}."),
        PkgState::Blocked => "Not linked: files that are not links are in the way.".into(),
        PkgState::NotPackage => "Not a stow package.".into(),
        PkgState::Empty => "Empty: nothing to link.".into(),
    }
}

/// The folder every path sits in, ending in `/` (empty when none).
fn common_folder(paths: &[String]) -> String {
    let parents = |p: &str| -> Vec<String> {
        let parts: Vec<&str> = p.trim_end_matches('/').split('/').collect();
        parts[..parts.len().saturating_sub(1)]
            .iter()
            .map(|s| s.to_string())
            .collect()
    };
    let Some(first) = paths.first() else {
        return String::new();
    };
    let mut common = parents(first);
    for p in &paths[1..] {
        let other = parents(p);
        let same = common
            .iter()
            .zip(&other)
            .take_while(|(a, b)| a == b)
            .count();
        common.truncate(same);
    }
    if common.is_empty() {
        String::new()
    } else {
        format!("{}/", common.join("/"))
    }
}

/// Word-wrap `text` after `lead`, continuing lines indented by `indent`.
/// Words longer than a line (paths) are split.
fn hanging(
    lead: Vec<Span<'static>>,
    text: &str,
    style: Style,
    width: usize,
    indent: usize,
) -> Vec<Line<'static>> {
    let lead_width: usize = lead.iter().map(|s| s.width()).sum();
    let mut lines = Vec::new();
    let mut spans = lead;
    let mut room = width.saturating_sub(lead_width).max(1);
    let mut current = String::new();
    let flush =
        |spans: &mut Vec<Span<'static>>, current: &mut String, lines: &mut Vec<Line<'static>>| {
            spans.push(Span::styled(std::mem::take(current), style));
            lines.push(Line::from(std::mem::take(spans)));
            spans.push(Span::raw(" ".repeat(indent)));
        };
    for word in text.split(' ') {
        let mut word: Vec<char> = word.chars().collect();
        loop {
            let used = current.chars().count();
            let gap = usize::from(used > 0);
            if used + gap + word.len() <= room {
                if gap == 1 {
                    current.push(' ');
                }
                current.extend(word.iter());
                break;
            }
            if used > 0 {
                flush(&mut spans, &mut current, &mut lines);
                room = width.saturating_sub(indent).max(1);
                continue;
            }
            // Longer than a whole line.
            let rest = word.split_off(room.min(word.len()));
            current.extend(word.iter());
            flush(&mut spans, &mut current, &mut lines);
            room = width.saturating_sub(indent).max(1);
            word = rest;
            if word.is_empty() {
                break;
            }
        }
    }
    spans.push(Span::styled(current, style));
    lines.push(Line::from(spans));
    lines
}

fn detail_lines(
    p: &Package,
    source: &Source,
    stow: bool,
    git: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let label = |s: &str| Span::styled(format!("{s:<9}"), theme::dim());
    let (_, style) = state_mark(&p.state);
    let mut lines = vec![
        Line::from(vec![
            label("Package"),
            Span::raw(format!("{}/{}", source.shown, p.name)),
        ]),
        Line::from(vec![label("Files"), Span::raw(p.files.to_string())]),
    ];
    lines.extend(hanging(
        vec![label("Status")],
        &state_sentence(&p.state),
        style,
        width,
        9,
    ));

    if !p.items.is_empty() && p.state != PkgState::NotPackage {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Links in ~", theme::heading())));
        for item in &p.items {
            let (tag, style, note) = match &item.kind {
                ItemKind::Folder => ("folder", theme::on(), String::new()),
                ItemKind::File => ("file", theme::on(), String::new()),
                ItemKind::Missing => ("missing", theme::dim(), String::new()),
                ItemKind::Blocked(why) => {
                    ("blocked", theme::status(Status::Failed), format!("  {why}"))
                }
                ItemKind::Owned(other) => (
                    "in use",
                    theme::status(Status::Attention),
                    format!("  linked from {other}"),
                ),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {tag:<8}"), style),
                Span::raw(format!("~/{}", item.path.display())),
                Span::styled(note, theme::dim()),
            ]));
        }
    }

    if git {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(".gitignore", theme::heading())));
        if p.gitignore.is_empty() && p.git_ignored.is_empty() {
            lines.push(Line::from(Span::styled(
                "  no rules: git tracks everything in this package",
                theme::dim(),
            )));
        }
        for rule in &p.gitignore {
            // Root .gitignore lines are the usual case; name any other file.
            let from = if rule.source == ".gitignore" {
                format!("  line {}", rule.line)
            } else {
                format!("  {}:{}", rule.source, rule.line)
            };
            let style = if rule.pattern.starts_with('!') {
                theme::on()
            } else {
                Style::default()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {}", rule.pattern), style),
                Span::styled(from, theme::dim()),
            ]));
        }
        if !p.git_ignored.is_empty() {
            // Name the folder they share once instead of on every entry.
            const SHOWN: usize = 12;
            let common = common_folder(&p.git_ignored);
            let names: Vec<&str> = p
                .git_ignored
                .iter()
                .map(|e| e.strip_prefix(common.as_str()).unwrap_or(e))
                .collect();
            let mut text = names[..names.len().min(SHOWN)].join(", ");
            if names.len() > SHOWN {
                text.push_str(&format!(" and {} more", names.len() - SHOWN));
            }
            let lead = if common.is_empty() {
                format!("  Ignored ({}): ", names.len())
            } else {
                format!("  Ignored ({}) in {common}: ", names.len())
            };
            lines.extend(hanging(
                vec![Span::styled(lead, theme::dim())],
                &text,
                theme::help(),
                width,
                4,
            ));
        }
    }

    if !p.warnings.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Warnings", theme::heading())));
        for w in &p.warnings {
            let style = theme::status(if w.severe {
                Status::Failed
            } else {
                Status::Attention
            });
            lines.extend(hanging(
                vec![Span::styled("  ! ", style)],
                &w.text,
                Style::default(),
                width,
                4,
            ));
        }
    }

    if stow {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("What stow would do now", theme::heading()),
            Span::styled(
                format!("  stow --simulate -d {} -t ~ {}", source.shown, p.name),
                theme::dim(),
            ),
        ]));
        if p.preview.is_empty() {
            lines.push(Line::from(Span::styled(
                "  nothing: already linked",
                theme::dim(),
            )));
        }
        for l in &p.preview {
            let style = if l.starts_with("WARNING!")
                || l.starts_with("  *")
                || l.starts_with("All operations aborted")
            {
                theme::status(Status::Failed)
            } else if l.starts_with("UNLINK:") {
                theme::status(Status::Attention)
            } else {
                theme::help()
            };
            let indent = 2 + l.len() - l.trim_start().len();
            lines.extend(hanging(
                vec![Span::raw(" ".repeat(indent))],
                l.trim_start(),
                style,
                width,
                indent + 2,
            ));
        }
    }
    lines
}

/// Hooks for the render tests.
impl DotfilesScreen {
    /// Wait for the background scan.
    pub fn wait_for_test(&mut self) {
        while self.scanning() {
            self.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn common_folder_is_shared_by_every_path() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            common_folder(&v(&[".config/d/a", ".config/d/b/"])),
            ".config/d/"
        );
        assert_eq!(common_folder(&v(&[".config/d/", ".config/e"])), ".config/");
        assert_eq!(common_folder(&v(&[".ssh/work"])), ".ssh/");
        assert_eq!(common_folder(&v(&["a", ".ssh/x"])), "");
    }

    #[test]
    fn hanging_indent_wraps_words_and_splits_long_ones() {
        let lines = hanging(
            vec![Span::raw("  ! ")],
            "one two three four",
            Style::default(),
            14,
            4,
        );
        assert_eq!(text(&lines), ["  ! one two", "    three four"]);
        let lines = hanging(
            vec![Span::raw("  ")],
            "abcdefghijkl",
            Style::default(),
            8,
            4,
        );
        assert_eq!(text(&lines), ["  abcdef", "    ghij", "    kl"]);
        assert!(text(&lines).iter().all(|l| l.chars().count() <= 8));
    }
}
