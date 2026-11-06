//! The main screen: tasks on the left, the highlighted task's options on the
//! right. `c` (or Enter, Tab, Right) moves into the options to change them.

use std::collections::HashMap;

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::AppState;
use crate::catalog::{App, Source, categories, category_of};
use crate::config::{NUMBER_SETTINGS, UPDATE_STEPS};
use crate::engine::edit::{tweak_applied, tweak_current};
use crate::system::{InfoMap, has_cmd};

use super::Effect;
use super::input::{Action, Hit, HitMap};
use super::review::ReviewKind;
use super::theme;
use super::widgets::{Button, Modal, TextInput, clamp_scroll, pane_block};

const PANE_TASKS: usize = 0;
const PANE_OPTIONS: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    Update,
    Apps,
    Tweaks,
    /// Not a task: the app list itself (edit, add, review).
    Settings,
}

impl Task {
    pub const ALL: [Task; 4] = [Task::Update, Task::Apps, Task::Tweaks, Task::Settings];

    pub fn label(self) -> &'static str {
        match self {
            Task::Update => crate::engine::plan::TASK_UPDATE,
            Task::Apps => crate::engine::plan::TASK_APPS,
            Task::Tweaks => crate::engine::plan::TASK_TWEAKS,
            Task::Settings => "Settings",
        }
    }

    fn is_task(self) -> bool {
        self != Task::Settings
    }

    /// Apps and Tweaks: the panes with filter, descriptions and toggle-all.
    fn lists_apps(self) -> bool {
        matches!(self, Task::Apps | Task::Tweaks)
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
            Task::Settings => {
                "Your app list (~/.config/cachyup/apps.toml): edit it, add an app, or review what could be added. c to open."
            }
        }
    }

    fn on(self, app: &AppState) -> bool {
        let t = &app.config.tasks;
        match self {
            Task::Update => t.update,
            Task::Apps => t.apps,
            Task::Tweaks => t.tweaks,
            Task::Settings => true,
        }
    }

    fn set(self, app: &mut AppState, on: bool) {
        let t = &mut app.config.tasks;
        match self {
            Task::Update => t.update = on,
            Task::Apps => t.apps = on,
            Task::Tweaks => t.tweaks = on,
            Task::Settings => {}
        }
    }
}

/// The rows of the Settings pane.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Setting {
    Edit,
    Add,
    ReviewInstalled,
    ReviewBuiltin,
}

impl Setting {
    const ALL: [Setting; 4] = [
        Setting::Edit,
        Setting::Add,
        Setting::ReviewInstalled,
        Setting::ReviewBuiltin,
    ];

    fn label(self) -> &'static str {
        match self {
            Setting::Edit => "Edit the app list in your editor",
            Setting::Add => "Add an app by name",
            Setting::ReviewInstalled => "Review apps installed on this machine",
            Setting::ReviewBuiltin => "Review what is new in cachyup",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Setting::Edit => {
                "Opens ~/.config/cachyup/apps.toml in $VISUAL, $EDITOR or xdg-open, and re-reads it when you are done"
            }
            Setting::Add => {
                "A pacman name, aur/name or flatpak/id. Appended to apps.toml and switched on"
            }
            Setting::ReviewInstalled => {
                "Explicitly installed packages and flatpaks that are not in your list: tick the ones to add. Kernels, drivers and firmware are set aside"
            }
            Setting::ReviewBuiltin => {
                "Entries cachyup ships that your list lacks: tick the ones to add, the rest are not offered again"
            }
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
    Setting(Setting),
    Blank,
    /// The description line under an app or tweak (`I`).
    Desc(Box<Row>),
}

impl Row {
    fn selectable(&self) -> bool {
        !matches!(self, Row::Header(_) | Row::Blank | Row::Desc(_))
    }
}

pub struct MainScreen {
    focus: usize,
    task: usize,
    /// Cursor and scroll per task.
    cursor: [usize; 4],
    offset: [usize; 4],
    filter: TextInput,
    filtering: bool,
    /// Show a description line under every app and tweak.
    show_desc: bool,
    /// Group apps by category (`f`) rather than by package source.
    by_category: bool,
    /// (installed, new in cachyup) waiting for a review; shown in Settings.
    review_counts: (usize, usize),
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
            cursor: [0; 4],
            offset: [0; 4],
            filter: TextInput::default(),
            filtering: false,
            show_desc: app.config.settings.show_descriptions,
            by_category: app.config.settings.group_by_category,
            review_counts: (0, 0),
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

    /// Re-read what is on disk (after a run, an edit or a review).
    pub fn refresh(&mut self, app: &AppState) {
        self.review_counts = review_counts(app);
        self.tweak_status = app
            .catalog
            .tweaks
            .iter()
            .map(|t| (t.id.clone(), tweak_applied(t, &app.paths, &app.installed)))
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

    /// Jump to the Settings pane.
    pub fn open_settings(&mut self, app: &AppState) {
        self.task = Task::ALL.len() - 1;
        self.focus = PANE_OPTIONS;
        self.snap_cursor(app);
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
            Task::Settings => {
                let mut rows = vec![Row::Header("Your app list".into())];
                rows.extend(Setting::ALL.iter().map(|s| Row::Setting(*s)));
                rows
            }
            Task::Apps => {
                let mut rows = Vec::new();
                for (heading, apps) in self.groups(app) {
                    let apps: Vec<&App> = apps
                        .into_iter()
                        .filter(|a| {
                            matches(&a.name)
                                || matches(a.display_name())
                                || matches(&heading)
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
                        .filter(|a| app.config.selection.app_on(&a.key()))
                        .count();
                    rows.push(Row::Header(format!("{heading}  {on}/{}", apps.len())));
                    for a in apps {
                        let key = a.key();
                        self.push_row(&mut rows, Row::App(key.clone()));
                        for t in app.catalog.tweaks_for(&key) {
                            self.push_row(
                                &mut rows,
                                Row::Tweak {
                                    id: t.id.clone(),
                                    nested: true,
                                },
                            );
                        }
                    }
                }
                rows
            }
            Task::Tweaks => {
                let mut rows = Vec::new();
                for t in app.catalog.tweaks.iter().filter(|t| {
                    matches(&t.name)
                        || t.app
                            .as_deref()
                            .and_then(|k| app.catalog.app(k))
                            .is_some_and(|a| matches(a.display_name()))
                }) {
                    self.push_row(
                        &mut rows,
                        Row::Tweak {
                            id: t.id.clone(),
                            nested: false,
                        },
                    );
                }
                rows
            }
        }
    }

    /// The app list's groups in display order: one per package source, or
    /// one per category (apps sorted by name, "Other" last) with `f`.
    fn groups<'a>(&self, app: &'a AppState) -> Vec<(String, Vec<&'a App>)> {
        if !self.by_category {
            return Source::ALL
                .iter()
                .map(|s| (s.heading().to_string(), app.catalog.apps_from(*s).collect()))
                .collect();
        }
        let mut groups: Vec<(String, Vec<&App>)> = categories()
            .into_iter()
            .map(|(c, _)| (c, Vec::new()))
            .chain(std::iter::once((
                crate::catalog::OTHER_CATEGORY.to_string(),
                Vec::new(),
            )))
            .collect();
        for a in &app.catalog.apps {
            let c = category_of(&a.name);
            if let Some((_, apps)) = groups.iter_mut().find(|(name, _)| *name == c) {
                apps.push(a);
            }
        }
        for (_, apps) in &mut groups {
            apps.sort_by_key(|a| a.display_name().to_lowercase());
        }
        groups
    }

    /// `f`: group by category or by source, keeping the cursor on its row.
    fn toggle_grouping(&mut self, app: &mut AppState) -> Vec<Effect> {
        let rows = self.rows(app);
        let current = rows.get(self.cursor[self.task]).cloned();
        self.by_category = !self.by_category;
        let rows = self.rows(app);
        if let Some(i) = current.and_then(|c| rows.iter().position(|r| *r == c)) {
            self.cursor[self.task] = i;
        }
        self.snap_cursor(app);
        app.config.settings.group_by_category = self.by_category;
        vec![Effect::SaveConfig]
    }

    /// Add an app or tweak row, with its description line when those are shown.
    fn push_row(&self, rows: &mut Vec<Row>, row: Row) {
        rows.push(row.clone());
        if self.show_desc {
            rows.push(Row::Desc(Box::new(row)));
        }
    }

    /// `I`: show or hide the description lines, keeping the cursor on its row.
    fn toggle_descriptions(&mut self, app: &mut AppState) -> Vec<Effect> {
        let rows = self.rows(app);
        let current = rows.get(self.cursor[self.task]).cloned();
        self.show_desc = !self.show_desc;
        let rows = self.rows(app);
        if let Some(i) = current.and_then(|c| rows.iter().position(|r| *r == c)) {
            self.cursor[self.task] = i;
        }
        self.snap_cursor(app);
        app.config.settings.show_descriptions = self.show_desc;
        vec![Effect::SaveConfig]
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
            Action::Toggle if self.current().is_task() => {
                let t = self.current();
                t.set(app, !t.on(app));
                return vec![Effect::SaveConfig];
            }
            Action::Char('c')
            | Action::Confirm
            | Action::Right
            | Action::NextPane
            | Action::Toggle => {
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
            Action::Char('I') if self.current().lists_apps() => {
                return self.toggle_descriptions(app);
            }
            Action::Char('/') if self.current().lists_apps() => {
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
            Action::Char('A') if self.current().lists_apps() => {
                return self.toggle_all(&rows, app);
            }
            Action::Char('f') if self.current() == Task::Apps => {
                return self.toggle_grouping(app);
            }
            Action::Char('u') if self.current() == Task::Apps => {
                if let Some(Row::App(key)) = rows.get(self.cursor[self.task]) {
                    return vec![Effect::AskUninstall(key.clone())];
                }
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
            Row::Setting(Setting::Edit) => vec![Effect::EditApps],
            Row::Setting(Setting::Add) => vec![Effect::Modal(Modal::input(
                "Add an app",
                "A pacman package name, aur/name or flatpak/id.\nIt is appended to apps.toml and switched on.",
                "",
                "add-app",
            ))],
            Row::Setting(Setting::ReviewInstalled) => {
                vec![Effect::Review(ReviewKind::Installed)]
            }
            Row::Setting(Setting::ReviewBuiltin) => vec![Effect::Review(ReviewKind::Builtin)],
            Row::App(key) => {
                let on = app.config.selection.app_on(key);
                app.set_app(key, !on);
                vec![Effect::SaveSelection]
            }
            Row::Tweak { id, .. } => {
                let on = app.config.selection.tweak_on(id);
                let mut effects = vec![Effect::SaveSelection];
                if let Some(key) = app.set_tweak(id, !on) {
                    let name = app
                        .catalog
                        .app(&key)
                        .map_or(key.clone(), |a| a.display_name().to_string());
                    effects.push(Effect::Toast(format!(
                        "also switched on {name} (this tweak needs it)"
                    )));
                }
                effects
            }
            Row::Header(_) | Row::Blank | Row::Desc(_) => Vec::new(),
        }
    }

    /// `A`: every app and tweak shown (so a filter narrows it) on, or all
    /// off if they already are.
    fn toggle_all(&mut self, rows: &[Row], app: &mut AppState) -> Vec<Effect> {
        let keys: Vec<String> = rows
            .iter()
            .filter_map(|r| match r {
                Row::App(k) => Some(k.clone()),
                _ => None,
            })
            .collect();
        let ids: Vec<String> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Tweak { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        if keys.is_empty() && ids.is_empty() {
            return Vec::new();
        }
        let all_on = keys.iter().all(|k| app.config.selection.app_on(k))
            && ids.iter().all(|id| app.config.selection.tweak_on(id));
        for k in &keys {
            app.set_app(k, !all_on);
        }
        for id in &ids {
            app.config.selection.tweaks.insert(id.clone(), !all_on);
        }
        let what = match (keys.len(), ids.len()) {
            (n, 0) => format!("{n} apps"),
            (0, m) => format!("{m} tweaks"),
            (n, m) => format!("{n} apps and {m} tweaks"),
        };
        vec![
            Effect::SaveSelection,
            Effect::Toast(format!(
                "{} {what}",
                if all_on {
                    "switched off"
                } else {
                    "switched on"
                }
            )),
        ]
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
        let all_on = keys.iter().all(|k| app.config.selection.app_on(k));
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
                body.push(Line::from(vec![
                    label("Category"),
                    Span::raw(category_of(&a.name)),
                ]));
                let (status, style) = app_status(a, app, info);
                body.push(Line::from(vec![
                    label("Status"),
                    Span::styled(status, style),
                ]));
                body.push(Line::from(vec![
                    label("In your list"),
                    if app.config.selection.app_on(key) {
                        Span::styled("on", theme::on())
                    } else {
                        Span::styled("off", theme::off())
                    },
                ]));
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
                for p in &t.installs {
                    body.push(Line::from(format!("  install {p}")));
                }
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
                if let Some(r) = &t.run {
                    body.push(Line::from(format!("  run: {}", r.run)));
                    body.push(Line::from(format!("  unless: {}", r.check)));
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
                Task::Apps => hints.extend([
                    ("i", "info"),
                    ("I", "descriptions"),
                    ("a", "select group"),
                    ("A", "select all"),
                    (
                        "f",
                        if self.by_category {
                            "by source"
                        } else {
                            "by category"
                        },
                    ),
                    ("u", "uninstall"),
                    ("/", "filter"),
                ]),
                Task::Tweaks => hints.extend([
                    ("i", "info"),
                    ("I", "descriptions"),
                    ("A", "select all"),
                    ("/", "filter"),
                ]),
                Task::Settings => hints.push(("Enter", "open")),
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
            let (mark, style) = if !task.is_task() {
                ("▸", theme::heading())
            } else if on {
                ("✓", theme::on())
            } else {
                ("✗", theme::off())
            };
            let mut line = Line::from(vec![
                Span::styled(format!(" {mark} "), style),
                Span::styled(format!("{} ", task.label()), style),
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
        let title = if self.current().is_task() {
            format!("{} options", self.current().label())
        } else {
            self.current().label().to_string()
        };
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
        // Keep a group's heading in view when the cursor is on its first row.
        if self.offset[t] == self.cursor[t]
            && self.cursor[t] > 0
            && !rows[self.cursor[t] - 1].selectable()
        {
            self.offset[t] -= 1;
        }
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
            // A description line clicks as the row above it.
            let target = match row {
                Row::Desc(_) => Some(i.saturating_sub(1)),
                r if r.selectable() => Some(i),
                _ => None,
            };
            if let Some(index) = target {
                hits.add(
                    Rect::new(list_area.x, y, list_area.width, 1),
                    Hit::Row {
                        pane: PANE_OPTIONS,
                        index,
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
            Row::Desc(of) => {
                // Starts in the same column as the name above it.
                let indent = match of.as_ref() {
                    Row::Tweak { nested: true, .. } => 7,
                    _ => 3,
                };
                let text = describe(of, app, info).unwrap_or_else(|| {
                    if info.is_empty() {
                        "reading package info...".into()
                    } else {
                        "no description".into()
                    }
                });
                let room = width.saturating_sub(indent + 1).max(1);
                let mut shown: String = text.chars().take(room).collect();
                if text.chars().count() > room {
                    shown.pop();
                    shown.push('…');
                }
                Line::from(vec![
                    Span::raw(" ".repeat(indent)),
                    Span::styled(shown, theme::dim()),
                ])
            }
            Row::Step(i) => {
                let step = &UPDATE_STEPS[*i];
                let enabled = step.needs.is_none_or(|n| app.config.step_enabled(n));
                let (m, style) = mark(app.config.step_on(step.id), enabled);
                let mut spans = vec![m, Span::styled(format!("{} ", step.label), style)];
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
            Row::Setting(s) => {
                let (installed, builtin) = self.review_counts;
                let count = match s {
                    Setting::ReviewInstalled => Some(installed),
                    Setting::ReviewBuiltin => Some(builtin),
                    _ => None,
                };
                let mut spans = vec![Span::raw(" ▸ "), Span::raw(format!("{} ", s.label()))];
                match count {
                    Some(0) => spans.push(Span::styled("  (nothing new)", theme::dim())),
                    Some(n) => spans.push(Span::styled(format!("  ({n})"), theme::on())),
                    None => {}
                }
                Line::from(spans)
            }
            Row::App(key) => {
                let Some(a) = app.catalog.app(key) else {
                    return Line::from(key.clone());
                };
                let on = app.config.selection.app_on(key);
                let (m, style) = mark(on, true);
                let (status, status_style) = app_status(a, app, info);
                let mut badge = Vec::new();
                if self.by_category {
                    badge.push(Span::styled(format!(" {}", a.source), theme::dim()));
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
                let on = app.config.selection.tweak_on(id);
                let (m, style) = mark(on, true);
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
            Row::Setting(s) => format!("{} (Enter to open)", s.help()),
            Row::App(_) => match describe(row, app, info) {
                Some(d) => format!("{d}  (i for details)"),
                None if info.is_empty() => "Reading package info...".into(),
                None => "No description  (i for details)".into(),
            },
            Row::Tweak { id, .. } => app
                .catalog
                .tweak(id)
                .and_then(|t| t.description.clone())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }
}

/// The description of an app (catalog text first, then the package source)
/// or a tweak.
fn describe(row: &Row, app: &AppState, info: &InfoMap) -> Option<String> {
    match row {
        Row::App(key) => app
            .catalog
            .app(key)
            .and_then(|a| a.description.clone())
            .or_else(|| info.get(key).and_then(|i| i.description.clone())),
        Row::Tweak { id, .. } => app.catalog.tweak(id).and_then(|t| t.description.clone()),
        _ => None,
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
    const STATUS_WIDTH: usize = 23;
    // 3 for the mark, 2 around the leader, 1 clear of the right border.
    let name_width = width
        .saturating_sub(3 + STATUS_WIDTH + 2 + 1 + indent)
        .max(8);
    let badge_width: usize = badge.iter().map(|s| s.width()).sum();
    let room = name_width.saturating_sub(badge_width);
    let mut shown: String = name.chars().take(room.max(4)).collect();
    if name.chars().count() > room.max(4) {
        shown.pop();
        shown.push('…');
    }
    // One space of padding inside the name's own style.
    let pad = name_width.saturating_sub(shown.chars().count() + 1 + badge_width);
    let mut spans = vec![
        Span::raw(" ".repeat(indent)),
        mark,
        Span::styled(format!("{shown} "), name_style),
    ];
    spans.extend(badge);
    // Dotted leader to the status column, so the eye can follow a long row.
    spans.push(Span::styled(
        format!(" {} ", ".".repeat(pad)),
        theme::leader(),
    ));
    spans.push(Span::styled(
        format!("{status:<STATUS_WIDTH$} "),
        status_style,
    ));
    Line::from(spans)
}

/// Status column text for an app.
pub fn app_status(a: &App, app: &AppState, info: &InfoMap) -> (String, Style) {
    let on = app.config.selection.app_on(&a.key());
    let available = info.get(&a.key()).and_then(|i| i.available);
    match app.is_installed(a) {
        Some(true) if will_update(a, app) => ("Installed (will update)".into(), theme::on()),
        Some(true) => ("Installed".into(), theme::on()),
        Some(false) if available == Some(false) => {
            ("Unavailable".into(), Style::default().fg(theme::BAD))
        }
        Some(false) if on => ("Not installed".into(), Style::default().fg(theme::WARN)),
        Some(false) => ("Not installed".into(), theme::dim()),
        None => ("Unknown".into(), theme::dim()),
    }
}

/// Whether a full system update, as configured now, would upgrade this
/// installed app: the update task on, the step for its source on, and for
/// an installer an `update` command plus the app selected (the package
/// managers upgrade everything they know, installers only what is chosen).
fn will_update(a: &App, app: &AppState) -> bool {
    if !app.config.tasks.update {
        return false;
    }
    match a.source {
        Source::Pacman => app.config.step_enabled("pacman"),
        Source::Aur => app.config.step_enabled("aur"),
        Source::Flatpak => app.config.step_enabled("flatpak"),
        Source::Installer => {
            app.config.step_enabled("installers")
                && app.config.selection.app_on(&a.key())
                && a.installer.as_ref().is_some_and(|i| i.update.is_some())
        }
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
                .filter(|a| app.config.selection.app_on(&a.key()))
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
                .filter(|t| app.config.selection.tweak_on(&t.id))
                .count();
            format!("{on} of {} selected", app.catalog.tweaks.len())
        }
        Task::Settings => {
            let (installed, builtin) = review_counts(app);
            match installed + builtin {
                0 => "nothing to review".into(),
                n => format!("{n} to review"),
            }
        }
    }
}

/// (installed and not reviewed, new in cachyup), the counts Settings shows.
/// Packages set aside as machine-specific are not counted.
fn review_counts(app: &AppState) -> (usize, usize) {
    let installed = app
        .installed_candidates()
        .iter()
        .filter(|c| c.hidden.is_none())
        .count();
    (installed, app.builtin_candidates().len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;

    /// A sandbox with two installer apps: one present (with an update
    /// command), one present without.
    fn sandbox() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::create_dir_all(paths.home.join("bin")).unwrap();
        std::fs::write(paths.home.join("bin/tool"), "").unwrap();
        std::fs::write(paths.home.join("bin/plain"), "").unwrap();
        std::fs::write(
            paths.apps_file(),
            r#"
[[installer]]
name = "tool"
check = "~/bin/tool"
install = "true"
update = "tool update"

[[installer]]
name = "plain"
check = "~/bin/plain"
install = "true"
"#,
        )
        .unwrap();
        let app = AppState::load(paths).unwrap();
        (dir, app)
    }

    #[test]
    fn installed_apps_say_when_an_update_will_run() {
        let (_dir, mut app) = sandbox();
        let info = InfoMap::default();
        let status =
            |app: &AppState, key: &str| app_status(app.catalog.app(key).unwrap(), app, &info).0;
        assert_eq!(status(&app, "installer/tool"), "Installed");

        // Needs the step, the update task and the app selected.
        app.config.update.insert("installers".into(), true);
        assert_eq!(status(&app, "installer/tool"), "Installed", "not selected");
        app.set_app("installer/tool", true);
        app.set_app("installer/plain", true);
        assert_eq!(status(&app, "installer/tool"), "Installed (will update)");
        assert_eq!(
            status(&app, "installer/plain"),
            "Installed",
            "no update command"
        );
        app.config.tasks.update = false;
        assert_eq!(status(&app, "installer/tool"), "Installed");
    }
}
