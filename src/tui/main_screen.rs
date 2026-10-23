//! The main screen: tasks on the left, the highlighted task's options on the
//! right. `c` (or Enter, Tab, Right) moves into the options to change them.

use std::collections::{HashMap, HashSet};

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::AppState;
use crate::catalog::{App, Origin, Source};
use crate::config::{NUMBER_SETTINGS, UPDATE_STEPS};
use crate::engine::edit::{tweak_applied, tweak_current};
use crate::system::{InfoMap, has_cmd};

use super::Effect;
use super::input::{Action, Hit, HitMap};
use super::theme;
use super::widgets::{Button, Modal, TextInput, clamp_scroll, pane_block};

const PANE_TASKS: usize = 0;
const PANE_OPTIONS: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    Update,
    Apps,
    Tweaks,
}

impl Task {
    pub const ALL: [Task; 3] = [Task::Update, Task::Apps, Task::Tweaks];

    pub fn label(self) -> &'static str {
        match self {
            Task::Update => crate::engine::plan::TASK_UPDATE,
            Task::Apps => crate::engine::plan::TASK_APPS,
            Task::Tweaks => crate::engine::plan::TASK_TWEAKS,
        }
    }

    fn help(self) -> &'static str {
        match self {
            Task::Update => {
                "Mirrors, keyrings, pacman, AUR and flatpak updates, then cleanup and post-update checks. c to choose steps."
            }
            Task::Apps => {
                "Install the apps switched on in your list (pacman, AUR, flatpak, install scripts). Never uninstalls. c to choose apps."
            }
            Task::Tweaks => {
                "Apply config tweaks for your apps. Each is skipped while its app is not installed. c to choose tweaks."
            }
        }
    }

    fn on(self, app: &AppState) -> bool {
        let t = &app.config.tasks;
        match self {
            Task::Update => t.update,
            Task::Apps => t.apps,
            Task::Tweaks => t.tweaks,
        }
    }

    fn set(self, app: &mut AppState, on: bool) {
        let t = &mut app.config.tasks;
        match self {
            Task::Update => t.update = on,
            Task::Apps => t.apps = on,
            Task::Tweaks => t.tweaks = on,
        }
    }
}

/// One row of an options list.
#[derive(Debug, Clone, PartialEq)]
enum Row {
    Header(String),
    /// Index into `UPDATE_STEPS`.
    Step(usize),
    /// Index into `NUMBER_SETTINGS`.
    Number(usize),
    /// Always runs; shown for information.
    Fixed(&'static str, &'static str),
    App(String),
    /// A tweak; `nested` when shown under its app.
    Tweak {
        id: String,
        nested: bool,
    },
    Blank,
}

impl Row {
    fn selectable(&self) -> bool {
        !matches!(self, Row::Header(_) | Row::Blank)
    }
}

pub struct MainScreen {
    focus: usize,
    task: usize,
    /// Cursor and scroll per task.
    cursor: [usize; 3],
    offset: [usize; 3],
    filter: TextInput,
    filtering: bool,
    /// Built-in apps and tweaks new since the last launch: blue, with a badge.
    pub new_items: HashSet<String>,
    tweak_status: HashMap<String, bool>,
    /// For tweaks not in place: what the file holds now.
    tweak_current: HashMap<String, String>,
    /// `unless_cmd` -> the package providing it, for commands that exist.
    conditions: HashMap<String, String>,
    list_height: usize,
}

impl MainScreen {
    pub fn new(app: &AppState) -> Self {
        let mut s = Self {
            focus: PANE_TASKS,
            task: 0,
            cursor: [0; 3],
            offset: [0; 3],
            filter: TextInput::default(),
            filtering: false,
            new_items: app
                .launch
                .new_apps
                .iter()
                .chain(&app.launch.new_tweaks)
                .cloned()
                .collect(),
            tweak_status: HashMap::new(),
            tweak_current: HashMap::new(),
            conditions: app
                .catalog
                .apps
                .iter()
                .filter_map(|a| a.unless_cmd.clone())
                .chain(
                    app.catalog
                        .tweaks
                        .iter()
                        .filter_map(|t| t.unless_cmd.clone()),
                )
                .filter(|cmd| has_cmd(cmd))
                .map(|cmd| {
                    let package =
                        crate::system::owning_package(&cmd).unwrap_or_else(|| cmd.clone());
                    (cmd, package)
                })
                .collect(),
            list_height: 10,
        };
        s.refresh(app);
        s
    }

    /// Re-read what is on disk (after a run).
    pub fn refresh(&mut self, app: &AppState) {
        self.tweak_status = app
            .catalog
            .tweaks
            .iter()
            .map(|t| (t.id.clone(), tweak_applied(t, &app.paths)))
            .collect();
        self.tweak_current = app
            .catalog
            .tweaks
            .iter()
            .filter_map(|t| Some((t.id.clone(), tweak_current(t, &app.paths)?)))
            .collect();
    }

    pub fn is_filtering(&self) -> bool {
        self.filtering
    }

    fn current(&self) -> Task {
        Task::ALL[self.task]
    }

    /// Jump to the app list with the cursor on the first new app.
    pub fn review_new(&mut self, app: &AppState) {
        self.task = 1;
        self.focus = PANE_OPTIONS;
        self.filter = TextInput::default();
        let rows = self.rows(app);
        if let Some(i) = rows.iter().position(|r| match r {
            Row::App(k) => self.new_items.contains(k),
            _ => false,
        }) {
            self.cursor[1] = i;
        }
    }

    fn rows(&self, app: &AppState) -> Vec<Row> {
        let filter = self.filter.value.trim().to_lowercase();
        let matches = |text: &str| filter.is_empty() || text.to_lowercase().contains(&filter);
        match self.current() {
            Task::Update => {
                let mut rows = vec![Row::Header("Steps".into())];
                rows.extend((0..UPDATE_STEPS.len()).map(Row::Step));
                rows.push(Row::Blank);
                rows.push(Row::Header("Settings".into()));
                rows.extend((0..NUMBER_SETTINGS.len()).map(Row::Number));
                rows.push(Row::Blank);
                rows.push(Row::Header("Always run".into()));
                rows.push(Row::Fixed(
                    "Preflight checks",
                    "CachyOS, internet, sudo, pacman not busy, ~/.local/bin on PATH",
                ));
                rows.push(Row::Fixed(
                    "Prerequisites",
                    "Packages cachyup itself needs (yay, base-devel, pacman-contrib, ...), installed before anything else",
                ));
                rows.push(Row::Fixed(
                    "Post-update checks",
                    "Report only: .pacnew files, pending reboot, services to restart, AUR packages needing a rebuild",
                ));
                rows
            }
            Task::Apps => {
                let mut rows = Vec::new();
                for source in Source::ALL {
                    let apps: Vec<&App> = app
                        .catalog
                        .apps_from(source)
                        .filter(|a| {
                            matches(&a.name)
                                || matches(a.display_name())
                                || app.catalog.tweaks_for(&a.key()).any(|t| matches(&t.name))
                        })
                        .collect();
                    if apps.is_empty() {
                        continue;
                    }
                    if !rows.is_empty() {
                        rows.push(Row::Blank);
                    }
                    let on = apps
                        .iter()
                        .filter(|a| app.selection.app_on(&a.key()))
                        .count();
                    rows.push(Row::Header(format!(
                        "{}  {on}/{}",
                        source.heading(),
                        apps.len()
                    )));
                    for a in apps {
                        let key = a.key();
                        rows.push(Row::App(key.clone()));
                        for t in app.catalog.tweaks_for(&key) {
                            rows.push(Row::Tweak {
                                id: t.id.clone(),
                                nested: true,
                            });
                        }
                    }
                }
                rows
            }
            Task::Tweaks => app
                .catalog
                .tweaks
                .iter()
                .filter(|t| {
                    matches(&t.name)
                        || t.app
                            .as_deref()
                            .and_then(|k| app.catalog.app(k))
                            .is_some_and(|a| matches(a.display_name()))
                })
                .map(|t| Row::Tweak {
                    id: t.id.clone(),
                    nested: false,
                })
                .collect(),
        }
    }

    pub fn handle(
        &mut self,
        action: Action,
        app: &mut AppState,
        info: &InfoMap,
        hits: &HitMap,
    ) -> Vec<Effect> {
        let action = match action {
            Action::Click { x, y } => match hits.find(x, y) {
                Some(Hit::Row { pane, index }) => {
                    let (pane, index) = (*pane, *index);
                    if pane == PANE_TASKS {
                        let again = self.focus == PANE_TASKS && self.task == index;
                        self.focus = PANE_TASKS;
                        self.task = index;
                        if !again {
                            return Vec::new();
                        }
                    } else {
                        let again = self.focus == PANE_OPTIONS && self.cursor[self.task] == index;
                        self.focus = PANE_OPTIONS;
                        self.cursor[self.task] = index;
                        if !again {
                            return Vec::new();
                        }
                    }
                    Action::Toggle
                }
                Some(Hit::Pane(p)) => {
                    self.focus = *p;
                    return Vec::new();
                }
                _ => return Vec::new(),
            },
            Action::Scroll { x, y, up } => {
                let pane = hits.pane_at(x, y).unwrap_or(self.focus);
                self.focus = pane;
                if up { Action::Up } else { Action::Down }
            }
            other => other,
        };

        match action {
            Action::Char('r') => return vec![Effect::AskRun],
            Action::Char('H') => return vec![Effect::History],
            _ => {}
        }
        if self.focus == PANE_TASKS {
            self.handle_tasks(action, app)
        } else {
            self.handle_options(action, app, info)
        }
    }

    fn handle_tasks(&mut self, action: Action, app: &mut AppState) -> Vec<Effect> {
        match action {
            Action::Up => self.task = self.task.saturating_sub(1),
            Action::Down => self.task = (self.task + 1).min(Task::ALL.len() - 1),
            Action::Home => self.task = 0,
            Action::End => self.task = Task::ALL.len() - 1,
            Action::Toggle => {
                let t = self.current();
                t.set(app, !t.on(app));
                return vec![Effect::SaveConfig];
            }
            Action::Char('c') | Action::Confirm | Action::Right | Action::NextPane => {
                self.focus = PANE_OPTIONS;
                self.snap_cursor(app);
            }
            Action::Back => return vec![Effect::Quit],
            _ => {}
        }
        Vec::new()
    }

    /// Put the cursor on a selectable row.
    fn snap_cursor(&mut self, app: &AppState) {
        let rows = self.rows(app);
        let c = &mut self.cursor[self.task];
        if rows.is_empty() {
            *c = 0;
            return;
        }
        *c = (*c).min(rows.len() - 1);
        if !rows[*c].selectable() {
            *c = rows
                .iter()
                .enumerate()
                .skip(*c)
                .chain(rows.iter().enumerate())
                .find(|(_, r)| r.selectable())
                .map_or(0, |(i, _)| i);
        }
    }

    fn move_cursor(&mut self, rows: &[Row], delta: isize) {
        if rows.is_empty() {
            return;
        }
        let mut c = self.cursor[self.task] as isize;
        let step = delta.signum();
        let mut remaining = delta.abs();
        let last = rows.len() as isize - 1;
        while remaining > 0 {
            let mut next = c + step;
            while (0..=last).contains(&next) && !rows[next as usize].selectable() {
                next += step;
            }
            if !(0..=last).contains(&next) {
                break;
            }
            c = next;
            remaining -= 1;
        }
        self.cursor[self.task] = c as usize;
    }

    fn handle_options(
        &mut self,
        action: Action,
        app: &mut AppState,
        info: &InfoMap,
    ) -> Vec<Effect> {
        let rows = self.rows(app);
        let page = self.list_height.saturating_sub(1).max(1) as isize;
        match action {
            Action::Up => self.move_cursor(&rows, -1),
            Action::Down => self.move_cursor(&rows, 1),
            Action::PageUp => self.move_cursor(&rows, -page),
            Action::PageDown => self.move_cursor(&rows, page),
            Action::Home => {
                self.cursor[self.task] = 0;
                self.snap_cursor(app);
            }
            Action::End => {
                self.cursor[self.task] = rows.len().saturating_sub(1);
                self.move_cursor(&rows, -1);
                self.move_cursor(&rows, 1);
            }
            Action::Back | Action::Left | Action::NextPane | Action::PrevPane => {
                if !self.filter.value.is_empty() && action == Action::Back {
                    self.filter = TextInput::default();
                    self.snap_cursor(app);
                } else {
                    self.focus = PANE_TASKS;
                }
            }
            Action::Char('/') if self.current() != Task::Update => {
                self.filtering = true;
            }
            Action::Char('i') => {
                if let Some(row) = rows.get(self.cursor[self.task]) {
                    return self
                        .info_modal(row, app, info)
                        .map(Effect::Modal)
                        .into_iter()
                        .collect();
                }
            }
            Action::Char('a') if self.current() == Task::Apps => {
                return self.toggle_group(&rows, app);
            }
            Action::Toggle | Action::Confirm => {
                if let Some(row) = rows.get(self.cursor[self.task]).cloned() {
                    return self.toggle(&row, app);
                }
            }
            _ => {}
        }
        Vec::new()
    }

    /// Feed a key while the filter is being typed.
    pub fn filter_key(&mut self, key: crossterm::event::KeyEvent, app: &AppState) {
        match key.code {
            KeyCode::Enter => self.filtering = false,
            KeyCode::Esc => {
                self.filtering = false;
                self.filter = TextInput::default();
            }
            KeyCode::Up | KeyCode::Down => self.filtering = false,
            _ => self.filter.handle_key(key),
        }
        self.cursor[self.task] = 0;
        self.offset[self.task] = 0;
        self.snap_cursor(app);
    }

    fn toggle(&mut self, row: &Row, app: &mut AppState) -> Vec<Effect> {
        match row {
            Row::Step(i) => {
                let step = &UPDATE_STEPS[*i];
                if let Some(needs) = step.needs
                    && !app.config.step_enabled(needs)
                {
                    let label = UPDATE_STEPS
                        .iter()
                        .find(|s| s.id == needs)
                        .map_or(needs, |s| s.label);
                    return vec![Effect::Toast(format!("{} needs {label}", step.label))];
                }
                let on = app.config.step_on(step.id);
                app.config.update.insert(step.id.to_string(), !on);
                vec![Effect::SaveConfig]
            }
            Row::Number(i) => {
                let n = &NUMBER_SETTINGS[*i];
                let current = (n.get)(&app.config.settings);
                vec![Effect::Modal(Modal::input(
                    n.label,
                    &format!("{}\nBetween {} and {}.", n.help, n.min, n.max),
                    &current.to_string(),
                    &format!("number:{i}"),
                ))]
            }
            Row::Fixed(..) => vec![Effect::Toast("always runs".into())],
            Row::App(key) => {
                let on = app.selection.app_on(key);
                app.set_app(key, !on);
                vec![Effect::SaveSelection]
            }
            Row::Tweak { id, .. } => {
                let on = app.selection.tweak_on(id);
                app.set_tweak(id, !on);
                vec![Effect::SaveSelection]
            }
            Row::Header(_) | Row::Blank => Vec::new(),
        }
    }

    /// `a`: all apps in the cursor's group on, or all off if they already are.
    fn toggle_group(&mut self, rows: &[Row], app: &mut AppState) -> Vec<Effect> {
        let c = self.cursor[self.task];
        let start = rows[..=c.min(rows.len().saturating_sub(1))]
            .iter()
            .rposition(|r| matches!(r, Row::Header(_)))
            .unwrap_or(0);
        let keys: Vec<String> = rows[start + 1..]
            .iter()
            .take_while(|r| !matches!(r, Row::Header(_)))
            .filter_map(|r| match r {
                Row::App(k) => Some(k.clone()),
                _ => None,
            })
            .collect();
        let all_on = keys.iter().all(|k| app.selection.app_on(k));
        for k in &keys {
            app.set_app(k, !all_on);
        }
        vec![
            Effect::SaveSelection,
            Effect::Toast(format!(
                "{} {} apps",
                if all_on {
                    "switched off"
                } else {
                    "switched on"
                },
                keys.len()
            )),
        ]
    }

    /// Apply an `Input` modal's value.
    pub fn set_number(&mut self, index: usize, value: &str, app: &mut AppState) -> Vec<Effect> {
        let n = &NUMBER_SETTINGS[index];
        match value.trim().parse::<u32>() {
            Ok(v) if (n.min..=n.max).contains(&v) => {
                (n.set)(&mut app.config.settings, v);
                vec![Effect::SaveConfig]
            }
            _ => vec![Effect::Toast(format!(
                "{}: enter a number from {} to {}",
                n.label, n.min, n.max
            ))],
        }
    }

    fn info_modal(&self, row: &Row, app: &AppState, info: &InfoMap) -> Option<Modal> {
        let label = |k: &str| Span::styled(format!("{k:<18}"), theme::dim());
        match row {
            Row::App(key) => {
                let a = app.catalog.app(key)?;
                let i = info.get(key);
                let mut body = Vec::new();
                let desc = a
                    .description
                    .clone()
                    .or_else(|| i.and_then(|i| i.description.clone()))
                    .unwrap_or_else(|| {
                        if info.is_empty() {
                            "Reading package info...".into()
                        } else {
                            "No description".into()
                        }
                    });
                body.push(Line::from(desc));
                body.push(Line::from(""));
                body.push(Line::from(vec![
                    label("Package"),
                    Span::raw(a.name.clone()),
                ]));
                body.push(Line::from(vec![
                    label("Source"),
                    Span::raw(a.source.label()),
                ]));
                let (status, style) = app_status(a, app, info);
                body.push(Line::from(vec![
                    label("Status"),
                    Span::styled(status, style),
                ]));
                body.push(Line::from(vec![
                    label("In your list"),
                    if app.selection.app_on(key) {
                        Span::styled("on", theme::on())
                    } else {
                        Span::styled("off", theme::off())
                    },
                ]));
                if a.origin == Origin::User {
                    body.push(Line::from(vec![
                        label("Added by"),
                        Span::raw("user-apps.toml"),
                    ]));
                }
                if let Some(package) = a.unless_cmd.as_ref().and_then(|c| self.conditions.get(c)) {
                    body.push(Line::from(vec![
                        label("Default off"),
                        Span::raw(format!("{package} is installed")),
                    ]));
                }
                for (k, v) in i.map(|i| i.details.as_slice()).unwrap_or_default() {
                    body.push(Line::from(vec![label(k), Span::raw(v.clone())]));
                }
                let tweaks: Vec<String> = app
                    .catalog
                    .tweaks_for(key)
                    .map(|t| t.name.clone())
                    .collect();
                if !tweaks.is_empty() {
                    body.push(Line::from(vec![
                        label("Tweaks"),
                        Span::raw(tweaks.join(", ")),
                    ]));
                }
                Some(Modal::new(
                    a.display_name(),
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
                ))
            }
            Row::Tweak { id, .. } => {
                let t = app.catalog.tweak(id)?;
                let mut body = vec![
                    Line::from(
                        t.description
                            .clone()
                            .unwrap_or_else(|| "No description".into()),
                    ),
                    Line::from(""),
                ];
                if let Some(a) = t.app.as_deref().and_then(|k| app.catalog.app(k)) {
                    body.push(Line::from(vec![
                        label("For"),
                        Span::raw(a.display_name().to_string()),
                    ]));
                }
                let applied = self.tweak_status.get(id).copied().unwrap_or(false);
                body.push(Line::from(vec![
                    label("Status"),
                    if applied {
                        Span::styled("Applied", theme::on())
                    } else {
                        Span::styled("Not applied", Style::default().fg(theme::WARN))
                    },
                ]));
                body.push(Line::from(""));
                body.push(Line::from(Span::styled("Changes", theme::heading())));
                for op in &t.sets {
                    body.push(Line::from(match op {
                        crate::catalog::SetOp::Json { file, key, value } => {
                            format!("  {file}: {key} = {value}")
                        }
                        crate::catalog::SetOp::Ini {
                            file,
                            section,
                            key,
                            value,
                        } => {
                            format!("  {file}: [{section}] {key}={value}")
                        }
                    }));
                }
                Some(Modal::new(
                    &t.name,
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
                ))
            }
            Row::Step(i) => Some(Modal::message(
                UPDATE_STEPS[*i].label,
                UPDATE_STEPS[*i].help,
            )),
            Row::Number(i) => Some(Modal::message(
                NUMBER_SETTINGS[*i].label,
                NUMBER_SETTINGS[*i].help,
            )),
            Row::Fixed(name, help) => Some(Modal::message(name, help)),
            _ => None,
        }
    }

    pub fn key_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.filtering {
            return vec![("Enter", "done"), ("Esc", "clear")];
        }
        let mut hints = Vec::new();
        if self.focus == PANE_TASKS {
            hints.extend([("Space", "toggle"), ("c", "options")]);
        } else {
            hints.push(("Space", "toggle"));
            match self.current() {
                Task::Update => hints.push(("i", "info")),
                Task::Apps => hints.extend([("i", "info"), ("a", "all in group"), ("/", "filter")]),
                Task::Tweaks => hints.extend([("i", "info"), ("/", "filter")]),
            }
            hints.push(("Esc", "back"));
        }
        hints.extend([("r", "run"), ("H", "history"), ("?", "help"), ("q", "quit")]);
        hints
    }

    pub fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        app: &AppState,
        info: &InfoMap,
        hits: &mut HitMap,
    ) {
        let [left, right] =
            Layout::horizontal([Constraint::Length(34), Constraint::Min(20)]).areas(area);
        self.draw_tasks(frame, left, app, hits);
        self.draw_options(frame, right, app, info, hits);
    }

    fn draw_tasks(&self, frame: &mut Frame, area: Rect, app: &AppState, hits: &mut HitMap) {
        let focused = self.focus == PANE_TASKS;
        let block = pane_block("Tasks", focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        hits.add(area, Hit::Pane(PANE_TASKS));

        let mut lines = Vec::new();
        for (i, task) in Task::ALL.iter().enumerate() {
            let on = task.on(app);
            let (mark, style) = if on {
                ("✓", theme::on())
            } else {
                ("✗", theme::off())
            };
            let mut line = Line::from(vec![
                Span::styled(format!(" {mark} "), style),
                Span::styled(task.label(), style),
            ]);
            if i == self.task {
                line = line.style(if focused {
                    theme::selected()
                } else {
                    theme::selected_inactive()
                });
            }
            let y = inner.y + lines.len() as u16;
            if y < inner.y + inner.height {
                hits.add(
                    Rect::new(inner.x, y, inner.width, 2),
                    Hit::Row {
                        pane: PANE_TASKS,
                        index: i,
                    },
                );
            }
            lines.push(line);
            lines.push(Line::from(Span::styled(
                format!("   {}", task_summary(*task, app)),
                theme::dim(),
            )));
            lines.push(Line::from(""));
        }
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn draw_options(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        app: &AppState,
        info: &InfoMap,
        hits: &mut HitMap,
    ) {
        let focused = self.focus == PANE_OPTIONS;
        let title = format!("{} options", self.current().label());
        let block = pane_block(&title, focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        hits.add(area, Hit::Pane(PANE_OPTIONS));

        let filter_rows = if self.filtering || !self.filter.value.is_empty() {
            1
        } else {
            0
        };
        let [filter_area, list_area, help_area] = Layout::vertical([
            Constraint::Length(filter_rows),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .areas(inner);
        if filter_rows > 0 {
            let mut spans = vec![Span::styled(" / ", theme::key())];
            spans.extend(self.filter.line(self.filtering).spans);
            frame.render_widget(Paragraph::new(Line::from(spans)), filter_area);
        }

        let rows = self.rows(app);
        let t = self.task;
        if self.cursor[t] >= rows.len() {
            self.cursor[t] = rows.len().saturating_sub(1);
        }
        let height = list_area.height as usize;
        self.list_height = height;
        self.offset[t] = clamp_scroll(self.cursor[t], rows.len(), height, self.offset[t]);
        let width = list_area.width as usize;
        let mut lines = Vec::new();
        for (i, row) in rows.iter().enumerate().skip(self.offset[t]).take(height) {
            let mut line = self.row_line(row, app, info, width);
            if i == self.cursor[t] && row.selectable() {
                line = line.style(if focused {
                    theme::selected()
                } else {
                    theme::selected_inactive()
                });
            }
            let y = list_area.y + (i - self.offset[t]) as u16;
            if row.selectable() {
                hits.add(
                    Rect::new(list_area.x, y, list_area.width, 1),
                    Hit::Row {
                        pane: PANE_OPTIONS,
                        index: i,
                    },
                );
            }
            lines.push(line);
        }
        if rows.is_empty() {
            lines.push(Line::from(Span::styled(
                if self.filter.value.is_empty() {
                    "  nothing here"
                } else {
                    "  nothing matches the filter"
                },
                theme::dim(),
            )));
        }
        frame.render_widget(Paragraph::new(lines), list_area);

        let help = if focused {
            rows.get(self.cursor[t])
                .map(|r| self.row_help(r, app, info))
                .unwrap_or_default()
        } else {
            self.current().help().to_string()
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "─".repeat(help_area.width as usize),
                    theme::border(),
                )),
                Line::from(Span::styled(help, theme::help())),
            ])
            .wrap(Wrap { trim: true }),
            help_area,
        );
    }

    fn row_line(&self, row: &Row, app: &AppState, info: &InfoMap, width: usize) -> Line<'static> {
        let mark = |on: bool, enabled: bool| -> (Span<'static>, Style) {
            let style = if on && enabled {
                theme::on()
            } else {
                theme::off()
            };
            (Span::styled(if on { " ✓ " } else { " ✗ " }, style), style)
        };
        match row {
            Row::Header(text) => Line::from(Span::styled(format!(" {text}"), theme::heading())),
            Row::Blank => Line::from(""),
            Row::Step(i) => {
                let step = &UPDATE_STEPS[*i];
                let enabled = step.needs.is_none_or(|n| app.config.step_enabled(n));
                let (m, style) = mark(app.config.step_on(step.id), enabled);
                let mut spans = vec![m, Span::styled(step.label, style)];
                if !enabled {
                    let needs = UPDATE_STEPS
                        .iter()
                        .find(|s| Some(s.id) == step.needs)
                        .map_or("", |s| s.label);
                    spans.push(Span::styled(format!("  (needs {needs})"), theme::dim()));
                }
                Line::from(spans)
            }
            Row::Number(i) => {
                let n = &NUMBER_SETTINGS[*i];
                let value = (n.get)(&app.config.settings).to_string();
                let pad = width.saturating_sub(n.label.len() + value.len() + 5);
                Line::from(vec![
                    Span::raw(format!("   {}", n.label)),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(value, theme::title()),
                    Span::raw(" "),
                ])
            }
            Row::Fixed(name, _) => Line::from(vec![
                Span::styled(" ✓ ", theme::dim()),
                Span::styled(name.to_string(), theme::dim()),
                Span::styled("  (always)", theme::dim()),
            ]),
            Row::App(key) => {
                let Some(a) = app.catalog.app(key) else {
                    return Line::from(key.clone());
                };
                let on = app.selection.app_on(key);
                let new = self.new_items.contains(key);
                let (m, mut style) = mark(on, true);
                if new && !on {
                    style = theme::new_item();
                }
                let (status, status_style) = app_status(a, app, info);
                let mut badge = Vec::new();
                if new {
                    badge.push(Span::styled(" new ", theme::badge_new()));
                }
                if a.origin == Origin::User {
                    badge.push(Span::styled(" user", theme::dim()));
                }
                if let Some(package) = a.unless_cmd.as_ref().and_then(|c| self.conditions.get(c)) {
                    badge.push(Span::styled(
                        format!(" (default off: {package} installed)"),
                        theme::dim(),
                    ));
                }
                table_line(
                    m,
                    a.display_name(),
                    style,
                    &status,
                    status_style,
                    badge,
                    width,
                    0,
                )
            }
            Row::Tweak { id, nested } => {
                let Some(t) = app.catalog.tweak(id) else {
                    return Line::from(id.clone());
                };
                let on = app.selection.tweak_on(id);
                let new = self.new_items.contains(id);
                let (m, mut style) = mark(on, true);
                if new && !on {
                    style = theme::new_item();
                }
                let name = if *nested {
                    t.name.clone()
                } else {
                    match t.app.as_deref().and_then(|k| app.catalog.app(k)) {
                        Some(a) => format!("{}: {}", a.display_name(), t.name),
                        None => t.name.clone(),
                    }
                };
                let applied = self.tweak_status.get(id).copied().unwrap_or(false);
                let app_missing = t
                    .app
                    .as_deref()
                    .and_then(|k| app.catalog.app(k))
                    .is_some_and(|a| app.is_installed(a) == Some(false));
                let (status, status_style) = if applied {
                    ("Applied".to_string(), theme::on())
                } else if app_missing {
                    ("App missing".to_string(), theme::dim())
                } else if on {
                    ("Not applied".to_string(), Style::default().fg(theme::WARN))
                } else {
                    ("Not applied".to_string(), theme::dim())
                };
                let mut badge = Vec::new();
                if new {
                    badge.push(Span::styled(" new ", theme::badge_new()));
                }
                if let Some(current) = self.tweak_current.get(id) {
                    badge.push(Span::styled(
                        format!(" (currently {current})"),
                        theme::dim(),
                    ));
                }
                table_line(
                    m,
                    &name,
                    style,
                    &status,
                    status_style,
                    badge,
                    width,
                    if *nested { 4 } else { 0 },
                )
            }
        }
    }

    fn row_help(&self, row: &Row, app: &AppState, info: &InfoMap) -> String {
        match row {
            Row::Step(i) => UPDATE_STEPS[*i].help.to_string(),
            Row::Number(i) => format!("{} (Enter to change)", NUMBER_SETTINGS[*i].help),
            Row::Fixed(_, help) => help.to_string(),
            Row::App(key) => {
                let desc = app
                    .catalog
                    .app(key)
                    .and_then(|a| a.description.clone())
                    .or_else(|| info.get(key).and_then(|i| i.description.clone()));
                match desc {
                    Some(d) => format!("{d}  (i for details)"),
                    None if info.is_empty() => "Reading package info...".into(),
                    None => "No description  (i for details)".into(),
                }
            }
            Row::Tweak { id, .. } => app
                .catalog
                .tweak(id)
                .and_then(|t| t.description.clone())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }
}

/// A row of the app and tweak tables: mark, name, status column, badges.
#[allow(clippy::too_many_arguments)]
fn table_line(
    mark: Span<'static>,
    name: &str,
    name_style: Style,
    status: &str,
    status_style: Style,
    badge: Vec<Span<'static>>,
    width: usize,
    indent: usize,
) -> Line<'static> {
    const STATUS_WIDTH: usize = 14;
    let name_width = width.saturating_sub(3 + STATUS_WIDTH + 2 + indent).max(8);
    let badge_width: usize = badge.iter().map(|s| s.width()).sum();
    let room = name_width.saturating_sub(badge_width);
    let mut shown: String = name.chars().take(room.max(4)).collect();
    if name.chars().count() > room.max(4) {
        shown.pop();
        shown.push('…');
    }
    let pad = name_width.saturating_sub(shown.chars().count() + badge_width);
    let mut spans = vec![
        Span::raw(" ".repeat(indent)),
        mark,
        Span::styled(shown, name_style),
    ];
    spans.extend(badge);
    spans.push(Span::raw(" ".repeat(pad + 2)));
    spans.push(Span::styled(
        format!("{status:<STATUS_WIDTH$}"),
        status_style,
    ));
    Line::from(spans)
}

/// Status column text for an app.
pub fn app_status(a: &App, app: &AppState, info: &InfoMap) -> (String, Style) {
    let on = app.selection.app_on(&a.key());
    let available = info.get(&a.key()).and_then(|i| i.available);
    match app.is_installed(a) {
        Some(true) => ("Installed".into(), theme::on()),
        Some(false) if available == Some(false) => {
            ("Unavailable".into(), Style::default().fg(theme::BAD))
        }
        Some(false) if on => ("Not installed".into(), Style::default().fg(theme::WARN)),
        Some(false) => ("Not installed".into(), theme::dim()),
        None => ("Unknown".into(), theme::dim()),
    }
}

/// The dim line under each task in the left pane.
fn task_summary(task: Task, app: &AppState) -> String {
    match task {
        Task::Update => {
            let on = UPDATE_STEPS
                .iter()
                .filter(|s| app.config.step_enabled(s.id))
                .count();
            format!("{on} of {} steps", UPDATE_STEPS.len())
        }
        Task::Apps => {
            let selected: Vec<&App> = app
                .catalog
                .apps
                .iter()
                .filter(|a| app.selection.app_on(&a.key()))
                .collect();
            let missing = selected
                .iter()
                .filter(|a| app.is_installed(a) == Some(false))
                .count();
            if missing > 0 {
                format!("{} selected, {missing} to install", selected.len())
            } else {
                format!("{} selected, all installed", selected.len())
            }
        }
        Task::Tweaks => {
            let on = app
                .catalog
                .tweaks
                .iter()
                .filter(|t| app.selection.tweak_on(&t.id))
                .count();
            format!("{on} of {} selected", app.catalog.tweaks.len())
        }
    }
}
