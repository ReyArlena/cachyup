//! The review screen: tick what to add to the app list from apps installed
//! on this machine, or from what cachyup ships. Nothing is written until
//! `w`; `Esc` throws the draft away.

use std::collections::HashSet;

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::AppState;
use crate::catalog::{Catalog, Source};
use crate::system::InfoMap;

use super::Effect;
use super::input::{Action, Hit, HitMap};
use super::theme;
use super::widgets::{Button, Modal, TextInput, clamp_scroll, pane_block};

const PANE: usize = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewKind {
    /// Explicitly installed packages and flatpaks the list lacks.
    Installed,
    /// Entries in cachyup's own list that the user's lacks.
    Builtin,
}

impl ReviewKind {
    fn title(self) -> &'static str {
        match self {
            ReviewKind::Installed => "Review: installed on this machine, not in your list",
            ReviewKind::Builtin => "Review: new in cachyup, not in your list",
        }
    }
}

/// One thing that can be added.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// `source/name` or `tweak/id`, as the merge code expects.
    pub key: String,
    pub label: String,
    pub group: &'static str,
    /// Set aside as machine-specific; shown only with `s`.
    pub hidden: Option<&'static str>,
}

/// What `w` hands back: what to add, and everything that was on screen,
/// so it is not offered again.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewResult {
    pub kind: ReviewKind,
    pub add: Vec<String>,
    pub seen: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum Row {
    Header(&'static str),
    /// Index into `items`.
    Item(usize),
}

pub struct ReviewScreen {
    kind: ReviewKind,
    items: Vec<Item>,
    /// Descriptions and details per key, filled in from a background fetch.
    info: InfoMap,
    /// Catalog descriptions for built-in entries, ready at once.
    builtin: Option<Catalog>,
    ticked: HashSet<String>,
    show_hidden: bool,
    cursor: usize,
    offset: usize,
    filter: TextInput,
    filtering: bool,
    height: usize,
}

impl ReviewScreen {
    pub fn new(kind: ReviewKind, app: &AppState) -> Self {
        let items = match kind {
            ReviewKind::Installed => app
                .installed_candidates()
                .into_iter()
                .map(|c| Item {
                    key: c.key(),
                    label: c.name.clone(),
                    group: c.source.heading(),
                    hidden: c.hidden,
                })
                .collect(),
            ReviewKind::Builtin => app
                .builtin_candidates()
                .into_iter()
                .map(|c| Item {
                    group: if c.is_tweak {
                        "Tweaks"
                    } else {
                        c.key
                            .split_once('/')
                            .and_then(|(s, _)| Source::from_id(s))
                            .map_or("Apps", Source::heading)
                    },
                    key: c.key,
                    label: c.label,
                    hidden: None,
                })
                .collect(),
        };
        let mut s = Self::from_items(kind, items);
        if kind == ReviewKind::Builtin {
            s.builtin = Some(Catalog::builtin());
        }
        s
    }

    pub fn from_items(kind: ReviewKind, items: Vec<Item>) -> Self {
        Self {
            kind,
            items,
            info: InfoMap::new(),
            builtin: None,
            ticked: HashSet::new(),
            show_hidden: false,
            cursor: 0,
            offset: 0,
            filter: TextInput::default(),
            filtering: false,
            height: 10,
        }
    }

    pub fn is_filtering(&self) -> bool {
        self.filtering
    }

    pub fn kind(&self) -> ReviewKind {
        self.kind
    }

    /// Every key on the list, for the info fetch.
    pub fn keys(&self) -> Vec<String> {
        self.items.iter().map(|i| i.key.clone()).collect()
    }

    pub fn set_info(&mut self, info: InfoMap) {
        self.info = info;
    }

    /// Catalog text first (built-in entries), then what the source says.
    fn describe(&self, item: &Item) -> Option<String> {
        let from_catalog = self.builtin.as_ref().and_then(|b| {
            if let Some(id) = item.key.strip_prefix("tweak/") {
                b.tweak(id).and_then(|t| t.description.clone())
            } else {
                b.app(&item.key).and_then(|a| a.description.clone())
            }
        });
        from_catalog.or_else(|| self.info.get(&item.key).and_then(|i| i.description.clone()))
    }

    fn info_modal(&self, item: &Item) -> Modal {
        let label = |k: &str| Span::styled(format!("{k:<18}"), theme::dim());
        let mut body = vec![
            Line::from(self.describe(item).unwrap_or_else(|| {
                if self.info.is_empty() {
                    "Reading package info...".into()
                } else {
                    "No description".into()
                }
            })),
            Line::from(""),
            Line::from(vec![label("Entry"), Span::raw(item.key.clone())]),
        ];
        if let Some(i) = self.info.get(&item.key) {
            for (k, v) in &i.details {
                body.push(Line::from(vec![label(k), Span::raw(v.clone())]));
            }
        }
        if let Some(why) = item.hidden {
            body.push(Line::from(vec![
                label("Set aside"),
                Span::raw(format!("{why}: machine-specific, shown with s")),
            ]));
        }
        Modal::new(
            &item.label,
            body,
            vec![Button::new(
                &[
                    KeyCode::Enter,
                    KeyCode::Esc,
                    KeyCode::Char('i'),
                    KeyCode::Char('q'),
                ],
                "Enter",
                "close",
                None,
            )],
        )
    }

    pub fn hidden_count(&self) -> usize {
        self.items.iter().filter(|i| i.hidden.is_some()).count()
    }

    /// Visible rows: a header per group, items under it, honouring the
    /// filter and the hidden toggle.
    fn rows(&self) -> Vec<Row> {
        let filter = self.filter.value.trim().to_lowercase();
        let mut rows = Vec::new();
        let mut group: Option<&'static str> = None;
        for (i, item) in self.items.iter().enumerate() {
            if item.hidden.is_some() && !self.show_hidden {
                continue;
            }
            if !filter.is_empty() && !item.label.to_lowercase().contains(&filter) {
                continue;
            }
            if group != Some(item.group) {
                rows.push(Row::Header(item.group));
                group = Some(item.group);
            }
            rows.push(Row::Item(i));
        }
        rows
    }

    fn visible_keys(&self) -> Vec<String> {
        self.rows()
            .iter()
            .filter_map(|r| match r {
                Row::Item(i) => Some(self.items[*i].key.clone()),
                _ => None,
            })
            .collect()
    }

    fn snap(&mut self, rows: &[Row]) {
        if rows.is_empty() {
            self.cursor = 0;
            return;
        }
        self.cursor = self.cursor.min(rows.len() - 1);
        if matches!(rows[self.cursor], Row::Header(_)) {
            self.cursor = rows
                .iter()
                .enumerate()
                .skip(self.cursor)
                .chain(rows.iter().enumerate())
                .find(|(_, r)| matches!(r, Row::Item(_)))
                .map_or(0, |(i, _)| i);
        }
    }

    fn step(&mut self, rows: &[Row], delta: isize) {
        if rows.is_empty() {
            return;
        }
        let last = rows.len() as isize - 1;
        let mut c = self.cursor as isize;
        let dir = delta.signum();
        let mut remaining = delta.abs();
        while remaining > 0 {
            let mut next = c + dir;
            while (0..=last).contains(&next) && matches!(rows[next as usize], Row::Header(_)) {
                next += dir;
            }
            if !(0..=last).contains(&next) {
                break;
            }
            c = next;
            remaining -= 1;
        }
        self.cursor = c as usize;
    }

    fn tick(&mut self, key: &str) {
        if !self.ticked.remove(key) {
            self.ticked.insert(key.to_string());
        }
    }

    /// Feed a key while the filter is being typed.
    pub fn filter_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Enter => self.filtering = false,
            KeyCode::Esc => {
                self.filtering = false;
                self.filter = TextInput::default();
            }
            KeyCode::Up | KeyCode::Down => self.filtering = false,
            _ => self.filter.handle_key(key),
        }
        self.cursor = 0;
        self.offset = 0;
    }

    pub fn handle(&mut self, action: Action, hits: &HitMap) -> Vec<Effect> {
        let action = match action {
            Action::Click { x, y } => match hits.find(x, y) {
                Some(Hit::Row { pane: PANE, index }) => {
                    self.cursor = *index;
                    Action::Toggle
                }
                _ => return Vec::new(),
            },
            Action::Scroll { up, .. } => {
                if up {
                    Action::Up
                } else {
                    Action::Down
                }
            }
            other => other,
        };
        let rows = self.rows();
        self.snap(&rows);
        let page = self.height.saturating_sub(1).max(1) as isize;
        match action {
            Action::Up => self.step(&rows, -1),
            Action::Down => self.step(&rows, 1),
            Action::PageUp => self.step(&rows, -page),
            Action::PageDown => self.step(&rows, page),
            Action::Home => {
                self.cursor = 0;
                self.snap(&rows);
            }
            Action::End => {
                self.cursor = rows.len().saturating_sub(1);
                self.step(&rows, -1);
                self.step(&rows, 1);
            }
            Action::Toggle | Action::Confirm => {
                if let Some(Row::Item(i)) = rows.get(self.cursor) {
                    let key = self.items[*i].key.clone();
                    self.tick(&key);
                }
            }
            Action::Char('a') => {
                // The cursor's group.
                let start = rows[..=self.cursor.min(rows.len().saturating_sub(1))]
                    .iter()
                    .rposition(|r| matches!(r, Row::Header(_)))
                    .unwrap_or(0);
                let keys: Vec<String> = rows[start..]
                    .iter()
                    .skip(1)
                    .take_while(|r| !matches!(r, Row::Header(_)))
                    .filter_map(|r| match r {
                        Row::Item(i) => Some(self.items[*i].key.clone()),
                        _ => None,
                    })
                    .collect();
                self.tick_all(&keys);
            }
            Action::Char('A') => {
                let keys = self.visible_keys();
                self.tick_all(&keys);
            }
            Action::Char('i') => {
                if let Some(Row::Item(i)) = rows.get(self.cursor) {
                    return vec![Effect::Modal(self.info_modal(&self.items[*i]))];
                }
            }
            Action::Char('s') => {
                self.show_hidden = !self.show_hidden;
                let rows = self.rows();
                self.snap(&rows);
            }
            Action::Char('/') => {
                self.filtering = true;
            }
            Action::Char('w') => {
                let seen = self.visible_keys();
                let mut add: Vec<String> = self
                    .items
                    .iter()
                    .filter(|i| self.ticked.contains(&i.key))
                    .map(|i| i.key.clone())
                    .collect();
                add.sort();
                return vec![Effect::SaveReview(ReviewResult {
                    kind: self.kind,
                    add,
                    seen,
                })];
            }
            Action::Back => {
                if !self.filter.value.is_empty() {
                    self.filter = TextInput::default();
                    self.cursor = 0;
                } else if !self.ticked.is_empty() {
                    return vec![Effect::Modal(Modal::confirm(
                        "Discard?",
                        &format!(
                            "{} ticked. Leave without adding them?\n\nThey will be offered again next time.",
                            self.ticked.len()
                        ),
                        "discard-review",
                    ))];
                } else {
                    return vec![Effect::CloseReview];
                }
            }
            _ => {}
        }
        Vec::new()
    }

    /// All on, or all off when they already are.
    fn tick_all(&mut self, keys: &[String]) {
        let all_on = !keys.is_empty() && keys.iter().all(|k| self.ticked.contains(k));
        for k in keys {
            if all_on {
                self.ticked.remove(k);
            } else {
                self.ticked.insert(k.clone());
            }
        }
    }

    pub fn key_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.filtering {
            return vec![("Enter", "done"), ("Esc", "clear")];
        }
        let mut hints = vec![
            ("Space", "tick"),
            ("i", "info"),
            ("a", "group"),
            ("A", "all shown"),
            ("/", "filter"),
        ];
        if self.hidden_count() > 0 {
            hints.push(("s", "system packages"));
        }
        hints.extend([
            ("w", "save and merge"),
            ("Esc", "back"),
            ("?", "help"),
            ("q", "quit"),
        ]);
        hints
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, hits: &mut HitMap) {
        let block = pane_block(self.kind.title(), true);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        hits.add(area, Hit::Pane(PANE));

        let filter_rows = if self.filtering || !self.filter.value.is_empty() {
            1
        } else {
            0
        };
        let [filter_area, list_area, help_area] = Layout::vertical([
            Constraint::Length(filter_rows),
            Constraint::Min(1),
            Constraint::Length(4),
        ])
        .areas(inner);
        if filter_rows > 0 {
            let mut spans = vec![Span::styled(" / ", theme::key())];
            spans.extend(self.filter.line(self.filtering).spans);
            frame.render_widget(Paragraph::new(Line::from(spans)), filter_area);
        }

        let rows = self.rows();
        self.snap(&rows);
        let height = list_area.height as usize;
        self.height = height;
        self.offset = clamp_scroll(self.cursor, rows.len(), height, self.offset);
        let mut lines = Vec::new();
        for (i, row) in rows.iter().enumerate().skip(self.offset).take(height) {
            let mut line = match row {
                Row::Header(h) => Line::from(Span::styled(h.to_string(), theme::heading())),
                Row::Item(idx) => {
                    let item = &self.items[*idx];
                    let on = self.ticked.contains(&item.key);
                    let (mark, style) = if on {
                        ("✓", theme::on())
                    } else {
                        ("·", theme::dim())
                    };
                    let mut spans = vec![
                        Span::styled(format!(" {mark} "), style),
                        Span::raw(item.label.clone()),
                    ];
                    if let Some(why) = item.hidden {
                        spans.push(Span::styled(format!("  ({why})"), theme::dim()));
                    }
                    Line::from(spans)
                }
            };
            if i == self.cursor && matches!(row, Row::Item(_)) {
                line = line.style(theme::selected());
            }
            let y = list_area.y + (i - self.offset) as u16;
            if matches!(row, Row::Item(_)) {
                hits.add(
                    Rect::new(list_area.x, y, list_area.width, 1),
                    Hit::Row {
                        pane: PANE,
                        index: i,
                    },
                );
            }
            lines.push(line);
        }
        if rows.is_empty() {
            let text = if !self.filter.value.is_empty() {
                "  nothing matches the filter"
            } else if self.hidden_count() > 0 {
                "  nothing to review (s shows the system packages set aside)"
            } else {
                "  nothing to review"
            };
            lines.push(Line::from(Span::styled(text, theme::dim())));
        }
        frame.render_widget(Paragraph::new(lines), list_area);

        // The cursor's description, then the standing summary.
        let shown = self.visible_keys().len();
        let mut lines = Vec::new();
        if let Some(Row::Item(i)) = rows.get(self.cursor) {
            let text = match self.describe(&self.items[*i]) {
                Some(d) => format!("{d}  (i for details)"),
                None if self.info.is_empty() => "Reading package info...".into(),
                None => "No description  (i for details)".into(),
            };
            lines.push(Line::from(Span::styled(text, theme::help())));
        }
        let mut summary = format!(
            "{} ticked of {shown} shown. w adds the ticked ones to apps.toml and marks all shown as reviewed; Esc leaves everything as it was.",
            self.ticked.len()
        );
        if let (ReviewKind::Installed, n) = (self.kind, self.hidden_count())
            && n > 0
        {
            summary.push_str(&format!(
                " {n} machine-specific packages (kernels, drivers, firmware, base system) are set aside; s shows them."
            ));
        }
        lines.push(Line::from(Span::styled(summary, theme::dim())));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), help_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, group: &'static str, hidden: Option<&'static str>) -> Item {
        Item {
            key: key.into(),
            label: key.split('/').nth(1).unwrap_or(key).into(),
            group,
            hidden,
        }
    }

    fn screen() -> ReviewScreen {
        ReviewScreen::from_items(
            ReviewKind::Installed,
            vec![
                item("pacman/htop", "Pacman packages", None),
                item("pacman/linux-cachyos", "Pacman packages", Some("kernel")),
                item("aur/spotify", "AUR packages", None),
            ],
        )
    }

    #[test]
    fn hidden_items_stay_out_until_shown() {
        let mut s = screen();
        assert_eq!(s.visible_keys(), ["pacman/htop", "aur/spotify"]);
        s.handle(Action::Char('s'), &HitMap::default());
        assert_eq!(
            s.visible_keys(),
            ["pacman/htop", "pacman/linux-cachyos", "aur/spotify"]
        );
    }

    #[test]
    fn save_reports_ticked_and_seen_and_esc_asks_when_ticked() {
        let hits = HitMap::default();
        let mut s = screen();
        assert!(matches!(
            s.handle(Action::Back, &hits)[..],
            [Effect::CloseReview]
        ));
        s.handle(Action::Toggle, &hits); // htop (first item under the header)
        assert!(matches!(
            s.handle(Action::Back, &hits)[..],
            [Effect::Modal(_)]
        ));
        let effects = s.handle(Action::Char('w'), &hits);
        let [Effect::SaveReview(r)] = &effects[..] else {
            panic!("expected SaveReview");
        };
        assert_eq!(r.add, ["pacman/htop"]);
        assert_eq!(r.seen, ["pacman/htop", "aur/spotify"], "hidden not seen");
    }

    #[test]
    fn info_shows_in_the_popup_once_fetched() {
        let hits = HitMap::default();
        let mut s = screen();
        let mut info = InfoMap::new();
        info.insert(
            "pacman/htop".into(),
            crate::system::PackageInfo {
                available: Some(true),
                description: Some("Interactive process viewer".into()),
                details: vec![("Installed version".into(), "3.4.1-1".into())],
            },
        );
        s.set_info(info);
        assert_eq!(
            s.describe(&s.items[0]).as_deref(),
            Some("Interactive process viewer")
        );
        assert!(matches!(
            s.handle(Action::Char('i'), &hits)[..],
            [Effect::Modal(_)]
        ));
    }

    #[test]
    fn group_and_all_toggles() {
        let hits = HitMap::default();
        let mut s = screen();
        s.handle(Action::Char('A'), &hits);
        assert_eq!(s.ticked.len(), 2);
        s.handle(Action::Char('A'), &hits);
        assert!(s.ticked.is_empty());
        s.handle(Action::Char('a'), &hits);
        assert_eq!(s.ticked.len(), 1, "cursor's group only");
    }
}
