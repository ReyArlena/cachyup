//! The app: owns the screens, routes input, applies effects, drives the runner.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{Event as TermEvent, KeyCode};
use ratatui::DefaultTerminal;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::AppState;
use crate::engine::runner::RunOptions;
use crate::engine::{Event, RunHandle, Status, start_run};
use crate::system::{InfoMap, fetch_info, load_info_cache, save_info_cache};

use super::Effect;
use super::history::HistoryScreen;
use super::input::{Action, Hit, HitMap, map_event};
use super::main_screen::MainScreen;
use super::review::{ReviewKind, ReviewResult, ReviewScreen};
use super::run_screen::RunScreen;
use super::theme;
use super::widgets::{Button, Modal, draw_help, draw_key_hints, key_hint_rows};

use crate::cli::INSTALL_CMD;
const RELEASES_URL: &str = "https://github.com/ReyArlena/cachyup/releases/latest";
const RELEASE_API: &str = "https://api.github.com/repos/ReyArlena/cachyup/releases/latest";

pub enum Exit {
    Quit,
    /// Step aside so sudo can ask for the password, then come back.
    Authenticate,
    /// Step aside so the user's editor can open apps.toml, then reload.
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Main,
    Run,
    History,
    Review,
}

pub struct App {
    state: AppState,
    screen: Screen,
    /// Where Esc from history goes back to.
    history_from: Screen,
    main: MainScreen,
    run_screen: RunScreen,
    history: HistoryScreen,
    review: Option<ReviewScreen>,
    modal: Option<Modal>,
    startup: VecDeque<Modal>,
    help: bool,
    hits: HitMap,
    toast: Option<(String, Instant)>,
    run: Option<RunHandle>,
    info: InfoMap,
    info_rx: Option<Receiver<InfoMap>>,
    /// Descriptions for the open review, fetched in the background.
    review_info_rx: Option<Receiver<InfoMap>>,
    update_rx: Option<Receiver<String>>,
    /// A newer release, once the check has answered. Shown in the header.
    latest_release: Option<String>,
    sudo: bool,
    /// Started with --dry-run: real runs are off.
    dry_only: bool,
    /// A real run waiting for sudo.
    pending_run: bool,
    /// An uninstall (app key) waiting for sudo.
    pending_uninstall: Option<String>,
    exit: Option<Exit>,
    /// Whether the terminal currently sends mouse events to us. Off while a
    /// popup is open, so its text can be selected and copied normally.
    mouse_on: bool,
}

impl App {
    pub fn new(state: AppState, sudo: bool, dry_only: bool) -> Self {
        let mut startup = VecDeque::new();
        startup.extend(startup_modals(&state));

        let info = load_info_cache(&state.paths.info_cache());
        let info_rx = spawn_info_fetch(&state);
        let update_rx = state.config.settings.update_check.then(|| {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                if let Some(v) = latest_release() {
                    let _ = tx.send(v);
                }
            });
            rx
        });

        let main = MainScreen::new(&state);
        Self {
            screen: Screen::Main,
            history_from: Screen::Main,
            main,
            run_screen: RunScreen::empty(),
            history: HistoryScreen::new(),
            review: None,
            modal: startup.pop_front(),
            startup,
            help: false,
            hits: HitMap::default(),
            toast: None,
            run: None,
            info,
            info_rx: Some(info_rx),
            review_info_rx: None,
            latest_release: None,
            update_rx,
            sudo,
            dry_only,
            pending_run: false,
            pending_uninstall: None,
            exit: None,
            mouse_on: true,
            state,
        }
    }

    pub fn run(
        &mut self,
        terminal: &mut DefaultTerminal,
        input: &Receiver<TermEvent>,
    ) -> Result<Exit> {
        // The caller turns mouse capture on before every (re)start.
        self.mouse_on = true;
        loop {
            terminal.draw(|f| self.draw(f))?;
            let want_mouse = self.modal.is_none();
            if want_mouse != self.mouse_on {
                use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
                let _ = if want_mouse {
                    crossterm::execute!(std::io::stdout(), EnableMouseCapture)
                } else {
                    crossterm::execute!(std::io::stdout(), DisableMouseCapture)
                };
                self.mouse_on = want_mouse;
            }
            if let Some(exit) = self.exit.take() {
                return Ok(exit);
            }
            match input.recv_timeout(Duration::from_millis(80)) {
                Ok(event) => {
                    self.on_terminal_event(event);
                    // Handle a burst (paste, key repeat) before redrawing.
                    while let Ok(event) = input.try_recv() {
                        self.on_terminal_event(event);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(Exit::Quit),
            }
            self.poll_background();
        }
    }

    /// Back from the editor: everything re-read from disk. Any problem with
    /// the edited file comes up as a popup, like at launch.
    pub fn reload(&mut self, state: AppState) {
        self.startup.extend(startup_modals(&state));
        self.info_rx = Some(spawn_info_fetch(&state));
        self.main = MainScreen::new(&state);
        self.state = state;
        if self.modal.is_none() {
            self.modal = self.startup.pop_front();
        }
    }

    /// A short message in the corner, from outside the event loop.
    pub fn notify(&mut self, text: &str) {
        self.toast(text);
    }

    /// `w` on the review screen: merge the ticked entries into apps.toml,
    /// remember what was shown, re-read everything, switch the added on.
    fn save_review(&mut self, result: &ReviewResult) -> Result<()> {
        match result.kind {
            ReviewKind::Installed => {
                let snippet = list_snippet(&result.add);
                self.state.merge_into_apps(&snippet, |_| true)?;
                self.state
                    .state
                    .reviewed
                    .extend(result.seen.iter().cloned());
            }
            ReviewKind::Builtin => {
                let add: std::collections::HashSet<&str> =
                    result.add.iter().map(String::as_str).collect();
                self.state
                    .merge_into_apps(crate::catalog::BUILTIN, |k| add.contains(k))?;
                self.state.state.dismissed.extend(
                    result
                        .seen
                        .iter()
                        .filter(|k| !add.contains(k.as_str()))
                        .cloned(),
                );
            }
        }
        self.state.save_state()?;
        self.reload_and_switch_on(&result.add)
    }

    /// "Add an app by name": one package, checked against its source,
    /// appended and switched on.
    fn add_app(&mut self, spec: &str) -> Result<()> {
        let package = crate::catalog::Package::parse(spec)?;
        let key = format!("{}/{}", package.source.id(), package.name);
        if self.state.catalog.app(&key).is_some() {
            anyhow::bail!("{key} is already in the list");
        }
        // Ask the source the same way the list does: pacman -Si, the AUR
        // RPC, or the Flathub listing. A name nobody has is not added.
        let probe = crate::catalog::Catalog {
            apps: vec![crate::catalog::App {
                source: package.source,
                name: package.name.clone(),
                label: None,
                description: None,
                unless_cmd: None,
                default: None,
                installer: None,
                origin: crate::catalog::Origin::User,
            }],
            ..Default::default()
        };
        let info = fetch_info(&probe, &self.state.installed, &self.state.paths);
        match info.get(&key).and_then(|i| i.available) {
            Some(true) => {}
            Some(false) => anyhow::bail!(
                "{} has no package called {}",
                package.source.label(),
                package.name
            ),
            None => anyhow::bail!(
                "could not ask {} about {}; try again",
                package.source.label(),
                package.name
            ),
        }
        self.state
            .merge_into_apps(&list_snippet(std::slice::from_ref(&key)), |_| true)?;
        self.reload_and_switch_on(std::slice::from_ref(&key))?;
        self.toast(&format!("added {key}, switched on"));
        Ok(())
    }

    /// Drop an entry from apps.toml and re-read everything.
    fn remove_app(&mut self, key: &str) -> Result<()> {
        self.state.remove_from_apps(key)?;
        self.reload_and_switch_on(&[])
    }

    /// Re-read apps.toml and everything else, then switch the given app
    /// keys on (a reload gives new entries their default, which is off).
    fn reload_and_switch_on(&mut self, keys: &[String]) -> Result<()> {
        let fresh = AppState::load(self.state.paths.clone())?;
        self.reload(fresh);
        for key in keys {
            if let Some(id) = key.strip_prefix("tweak/") {
                self.state
                    .config
                    .selection
                    .tweaks
                    .insert(id.to_string(), true);
            } else if self.state.catalog.app(key).is_some() {
                self.state.set_app(key, true);
            }
        }
        self.state.save_config()?;
        self.main.refresh(&self.state);
        Ok(())
    }

    /// Back from the sudo prompt.
    pub fn authenticated(&mut self, ok: bool) {
        self.sudo = ok;
        if self.pending_run {
            self.pending_run = false;
            if ok {
                self.start(false);
            } else {
                self.toast("sudo failed: run cancelled");
            }
        }
        if let Some(key) = self.pending_uninstall.take() {
            if ok {
                self.start_uninstall(&key);
            } else {
                self.toast("sudo failed: nothing uninstalled");
            }
        }
    }

    fn poll_background(&mut self) {
        // Runner events: drain plenty per frame so output keeps up.
        let mut finished = None;
        if let Some(run) = &self.run {
            for _ in 0..5000 {
                match run.events.try_recv() {
                    Ok(Event::Finished(record)) => {
                        finished = Some(record);
                        break;
                    }
                    Ok(event) => self.run_screen.apply(event),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break,
                }
            }
        }
        if let Some(record) = finished {
            self.run = None;
            if let Err(e) = self.state.record_run(&record) {
                self.toast(&format!("could not save state: {e:#}"));
            }
            self.main.refresh(&self.state);
            let modal = summary_modal(&record);
            self.run_screen.apply(Event::Finished(record));
            self.modal = Some(modal);
        }

        if let Some(rx) = &self.info_rx
            && let Ok(info) = rx.try_recv()
        {
            self.info = info;
            self.info_rx = None;
        }
        if let Some(rx) = &self.review_info_rx
            && let Ok(info) = rx.try_recv()
        {
            self.review_info_rx = None;
            if let Some(review) = &mut self.review {
                review.set_info(info);
            }
        }
        if let Some(rx) = &self.update_rx
            && let Ok(latest) = rx.try_recv()
        {
            self.update_rx = None;
            self.latest_release = Some(latest.clone());
            let modal = Modal::new(
                "Update available",
                Modal::text(&format!(
                    "cachyup v{latest} is out (you have v{}).\n\nTo update, run:\n  cachyup update\n\nor the install command again:\n  {INSTALL_CMD}",
                    env!("CARGO_PKG_VERSION")
                )),
                vec![
                    Button::new(
                        &[KeyCode::Enter],
                        "Enter",
                        "open release page",
                        Some("open-release"),
                    ),
                    Button::new(&[KeyCode::Esc, KeyCode::Char('q')], "Esc", "later", None),
                ],
            )
            .with_copy(INSTALL_CMD.to_string());
            if self.modal.is_none() && self.run.is_none() {
                self.modal = Some(modal);
            } else {
                self.startup.push_back(modal);
            }
        }
    }

    fn on_terminal_event(&mut self, event: TermEvent) {
        if let TermEvent::Key(key) = &event {
            if key.kind != crossterm::event::KeyEventKind::Press {
                return;
            }
            if let Some(modal) = &mut self.modal {
                let outcome = modal.handle_key(*key);
                if modal.copy_requested {
                    modal.copy_requested = false;
                    let text = modal.copy.clone().unwrap_or_default();
                    self.copy_to_clipboard(&text);
                    return;
                }
                if let Some(tag) = outcome {
                    let value = modal.input_value();
                    self.modal = None;
                    if let Some(tag) = tag {
                        self.on_modal(&tag, value);
                    }
                    if self.modal.is_none() && self.run.is_none() {
                        self.modal = self.startup.pop_front();
                    }
                }
                return;
            }
            if self.help {
                self.help = false;
                if key.code == KeyCode::Char('e') {
                    if self.run.is_some() {
                        self.toast("wait for the run to finish before editing");
                    } else {
                        self.exit = Some(Exit::Edit);
                    }
                }
                return;
            }
            if self.screen == Screen::Main && self.main.is_filtering() {
                self.main.filter_key(*key, &self.state);
                return;
            }
            if self.screen == Screen::Review
                && let Some(review) = &mut self.review
                && review.is_filtering()
            {
                review.filter_key(*key);
                return;
            }
        }
        if self.modal.is_some() || self.help {
            return;
        }
        let Some(mut action) = map_event(event) else {
            return;
        };
        // Clicking a key hint sends the same action as the key.
        if let Action::Click { x, y } = action
            && let Some(Hit::Key(c)) = self.hits.find(x, y)
        {
            action = match c {
                '\n' => Action::Confirm,
                ' ' => Action::Toggle,
                '\x1b' => Action::Back,
                '\t' => Action::NextPane,
                'q' => Action::Quit,
                '?' => Action::Help,
                c => Action::Char(*c),
            };
        }
        if action == Action::Help {
            self.help = true;
            return;
        }
        let effects = match self.screen {
            Screen::Main => {
                if action == Action::Quit {
                    vec![Effect::Quit]
                } else {
                    self.main
                        .handle(action, &mut self.state, &self.info, &self.hits)
                }
            }
            Screen::Run => self.run_screen.handle(action, &self.hits),
            Screen::History => {
                if action == Action::Quit {
                    vec![Effect::Quit]
                } else {
                    self.history.handle(action, &self.hits)
                }
            }
            Screen::Review => {
                if action == Action::Quit {
                    vec![Effect::Quit]
                } else if let Some(review) = &mut self.review {
                    review.handle(action, &self.hits)
                } else {
                    vec![Effect::CloseReview]
                }
            }
        };
        self.apply(effects);
    }

    fn on_modal(&mut self, tag: &str, value: Option<String>) {
        match tag {
            "run" => self.start(false),
            "dry" => self.start(true),
            "stop-after" => {
                if let Some(run) = &self.run {
                    run.stop_after_step();
                    self.run_screen.stopping = true;
                }
            }
            "stop-now" => {
                if let Some(run) = &self.run {
                    run.stop_now();
                    self.run_screen.stopping = true;
                }
            }
            "open-settings" => {
                self.screen = Screen::Main;
                self.main.open_settings(&self.state);
            }
            "discard-review" => {
                self.review = None;
                self.screen = Screen::Main;
            }
            "add-app" => {
                let spec = value.unwrap_or_default();
                if let Err(e) = self.add_app(spec.trim()) {
                    self.toast(&format!("not added: {e:#}"));
                }
            }
            "open-release" => open_url(RELEASES_URL),
            tag if tag.starts_with("uninstall:") => {
                let key = tag["uninstall:".len()..].to_string();
                self.start_uninstall(&key);
            }
            tag if tag.starts_with("remove-app:") => {
                let key = tag["remove-app:".len()..].to_string();
                match self.remove_app(&key) {
                    Ok(()) => self.toast(&format!("removed {key} from apps.toml")),
                    Err(e) => self.toast(&format!("not removed: {e:#}")),
                }
            }
            "delete-run" => {
                let effects = self.history.delete_current(&self.state.paths.runs_dir());
                self.apply(effects);
            }
            tag if tag.starts_with("number:") => {
                let index = tag["number:".len()..].parse().unwrap_or(0);
                let effects =
                    self.main
                        .set_number(index, &value.unwrap_or_default(), &mut self.state);
                self.apply(effects);
            }
            _ => {}
        }
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Modal(m) => self.modal = Some(m),
                Effect::Toast(t) => self.toast(&t),
                Effect::SaveConfig => {
                    if let Err(e) = self.state.save_config() {
                        self.toast(&format!("could not save: {e:#}"));
                    }
                }
                Effect::SaveSelection => {
                    if let Err(e) = self.state.save_selection().and(self.state.save_state()) {
                        self.toast(&format!("could not save: {e:#}"));
                    }
                }
                Effect::AskRun => self.ask_run(),
                Effect::AskUninstall(key) => self.ask_uninstall(&key),
                Effect::AskStop => {
                    self.modal = Some(Modal::new(
                        "Stop the run?",
                        Modal::text(
                            "Stopping after the current step is safe.\n\nStopping now kills the running command. Interrupting pacman\nmid-transaction can leave packages half-installed.",
                        ),
                        vec![
                            Button::new(
                                &[KeyCode::Enter],
                                "Enter",
                                "stop after this step",
                                Some("stop-after"),
                            ),
                            Button::new(&[KeyCode::Char('f')], "f", "stop now", Some("stop-now")),
                            Button::new(
                                &[KeyCode::Esc, KeyCode::Char('n')],
                                "Esc",
                                "keep going",
                                None,
                            ),
                        ],
                    ));
                }
                Effect::CloseRun => {
                    self.screen = Screen::Main;
                    if self.modal.is_none() {
                        self.modal = self.startup.pop_front();
                    }
                }
                Effect::History => {
                    self.history_from = self.screen;
                    self.history.refresh(&self.state.paths.runs_dir());
                    self.screen = Screen::History;
                }
                Effect::CloseHistory => self.screen = self.history_from,
                Effect::EditApps => {
                    if self.run.is_some() {
                        self.toast("wait for the run to finish before editing");
                    } else {
                        self.exit = Some(Exit::Edit);
                    }
                }
                Effect::Review(kind) => {
                    let review = ReviewScreen::new(kind, &self.state);
                    self.review_info_rx = Some(spawn_review_info(&review, &self.state));
                    self.review = Some(review);
                    self.screen = Screen::Review;
                }
                Effect::CloseReview => {
                    self.review = None;
                    self.screen = Screen::Main;
                }
                Effect::SaveReview(result) => {
                    self.review = None;
                    self.screen = Screen::Main;
                    match self.save_review(&result) {
                        Ok(()) => self.toast(&format!(
                            "added {} to apps.toml, {} reviewed",
                            result.add.len(),
                            result.seen.len()
                        )),
                        Err(e) => self.toast(&format!("could not save: {e:#}")),
                    }
                }
                Effect::Quit => self.exit = Some(Exit::Quit),
            }
        }
    }

    fn ask_run(&mut self) {
        if self.run.is_some() {
            self.screen = Screen::Run;
            return;
        }
        let plan = self.state.plan(true);
        if plan.steps.is_empty() {
            self.toast("nothing to run: switch a task on (Space)");
            return;
        }
        let state = &self.state;
        let mut body: Vec<Line<'static>> = Vec::new();
        let label = |s: &str| Span::styled(format!("{s:<10}"), theme::dim());
        body.push(Line::from(vec![
            label("Tasks"),
            Span::raw(plan.tasks.join(", ")),
        ]));
        let groups = |name: &str| plan.visible().filter(|s| s.group == name).count();
        if state.config.tasks.update {
            body.push(Line::from(vec![
                label("Update"),
                Span::raw(format!(
                    "{} update steps, {} cleanup steps",
                    groups(crate::engine::plan::GROUP_UPDATE),
                    groups(crate::engine::plan::GROUP_CLEANUP)
                )),
            ]));
        }
        if state.config.tasks.apps {
            let selected: Vec<_> = state
                .catalog
                .apps
                .iter()
                .filter(|a| state.config.selection.app_on(&a.key()))
                .collect();
            let missing = selected
                .iter()
                .filter(|a| state.is_installed(a) == Some(false))
                .count();
            body.push(Line::from(vec![
                label("Apps"),
                Span::raw(format!("{} selected, {missing} to install", selected.len())),
            ]));
        }
        if state.config.tasks.apps {
            let n = state
                .catalog
                .tweaks
                .iter()
                .filter(|t| state.config.selection.tweak_on(&t.id))
                .count();
            body.push(Line::from(vec![
                label("Tweaks"),
                Span::raw(format!("{n} selected")),
            ]));
        }
        let mut warnings = Vec::new();
        if state.config.tasks.apps
            && state.config.tasks.update
            && !state.config.step_enabled("pacman")
        {
            warnings.push("pacman -Syu is off: new packages install against an old database.");
        }
        if state.config.tasks.apps && !state.config.tasks.update {
            warnings
                .push("Full System Update is off: packages install against the current database.");
        }
        if !self.sudo && !self.dry_only {
            warnings.push("No sudo ticket: a real run asks for your password first.");
        }
        if !warnings.is_empty() {
            body.push(Line::from(""));
            for w in warnings {
                body.push(Line::from(Span::styled(
                    format!("! {w}"),
                    theme::status(Status::Attention),
                )));
            }
        }
        body.push(Line::from(""));
        body.push(Line::from(Span::styled(
            "A dry run makes no changes: it runs every check and logs what would be done.",
            theme::dim(),
        )));
        let buttons = if self.dry_only {
            body.push(Line::from(Span::styled(
                "Started with --dry-run: real runs are off.",
                theme::dim(),
            )));
            vec![
                Button::new(
                    &[KeyCode::Enter, KeyCode::Char('d'), KeyCode::Char('r')],
                    "d",
                    "dry run",
                    Some("dry"),
                ),
                Button::new(&[KeyCode::Esc, KeyCode::Char('q')], "Esc", "cancel", None),
            ]
        } else {
            vec![
                Button::new(
                    &[KeyCode::Enter, KeyCode::Char('r')],
                    "Enter",
                    "run",
                    Some("run"),
                ),
                Button::new(&[KeyCode::Char('d')], "d", "dry run", Some("dry")),
                Button::new(&[KeyCode::Esc, KeyCode::Char('q')], "Esc", "cancel", None),
            ]
        };
        self.modal = Some(Modal::new("Run", body, buttons));
    }

    /// `u`: the confirmation, with the command it would run.
    fn ask_uninstall(&mut self, key: &str) {
        let Some(app) = self.state.catalog.app(key) else {
            return;
        };
        if self.dry_only {
            self.toast("started with --dry-run: nothing is uninstalled");
            return;
        }
        if self.run.is_some() {
            self.toast("wait for the run to finish");
            return;
        }
        let name = app.display_name().to_string();
        let Some(cmd) = crate::engine::plan::uninstall_cmd(app) else {
            self.toast(&format!(
                "{name}'s entry has no uninstall command; switch it off and remove it by hand"
            ));
            return;
        };
        if self.state.is_installed(app) != Some(true) {
            // Nothing to uninstall. A name its source does not know is
            // dead weight in the list: offer to drop it.
            if self.info.get(key).and_then(|i| i.available) == Some(false) {
                self.modal = Some(Modal::new(
                    &format!("Remove {name} from the list?"),
                    Modal::text(&format!(
                        "{} has no package called {}, so it can never be installed.\n\nRemove its line from apps.toml?",
                        app.source.label(),
                        app.name
                    )),
                    vec![
                        Button::new(
                            &[KeyCode::Enter],
                            "Enter",
                            "remove",
                            Some(&format!("remove-app:{key}")),
                        ),
                        Button::new(&[KeyCode::Esc, KeyCode::Char('n')], "Esc", "keep", None),
                    ],
                ));
            } else {
                self.toast(&format!("{name} is not installed"));
            }
            return;
        }
        let cmd = cmd.display();
        let text = format!(
            "This runs:\n\n  {cmd}\n\nand switches {name} off in cachyup, so the next run does not put it back.\n\nTo do it by hand instead, run the command above (c copies it)."
        );
        self.modal = Some(
            Modal::new(
                &format!("Uninstall {name}?"),
                Modal::text(&text),
                vec![
                    Button::new(
                        &[KeyCode::Enter],
                        "Enter",
                        "uninstall",
                        Some(&format!("uninstall:{key}")),
                    ),
                    Button::new(&[KeyCode::Esc, KeyCode::Char('n')], "Esc", "cancel", None),
                ],
            )
            .with_copy(cmd),
        );
    }

    /// Confirmed: switch the app off, then run the one-step plan like any
    /// other run (streamed, logged, in the history).
    fn start_uninstall(&mut self, key: &str) {
        let Some(app) = self.state.catalog.app(key).cloned() else {
            return;
        };
        let needs_sudo = crate::engine::plan::uninstall_cmd(&app).is_some_and(|c| c.sudo);
        if needs_sudo && !self.sudo {
            if crate::sudo::has_ticket() {
                self.sudo = true;
                crate::sudo::keep_alive();
            } else {
                self.pending_uninstall = Some(key.to_string());
                self.exit = Some(Exit::Authenticate);
                return;
            }
        }
        self.state.set_app(key, false);
        if let Err(e) = self.state.save_selection().and(self.state.save_state()) {
            self.toast(&format!("could not save: {e:#}"));
        }
        let plan = crate::engine::plan::uninstall_plan(&app);
        self.run_screen = RunScreen::new(&plan, false);
        self.run = Some(start_run(
            plan,
            RunOptions {
                dry: false,
                paths: self.state.paths.clone(),
                settings: self.state.config.settings.clone(),
            },
        ));
        self.screen = Screen::Run;
    }

    fn start(&mut self, dry: bool) {
        if !dry && !self.sudo {
            if crate::sudo::has_ticket() {
                self.sudo = true;
                crate::sudo::keep_alive();
            } else {
                self.pending_run = true;
                self.exit = Some(Exit::Authenticate);
                return;
            }
        }
        let plan = self.state.plan(dry);
        self.run_screen = RunScreen::new(&plan, dry);
        self.run = Some(start_run(
            plan,
            RunOptions {
                dry,
                paths: self.state.paths.clone(),
                settings: self.state.config.settings.clone(),
            },
        ));
        self.screen = Screen::Run;
    }

    /// wl-copy (Wayland) or xclip when present, otherwise the terminal's
    /// OSC 52 clipboard escape.
    fn copy_to_clipboard(&mut self, text: &str) {
        let piped = |program: &str, args: &[&str]| -> bool {
            use std::io::Write;
            use std::process::{Command, Stdio};
            let Ok(mut child) = Command::new(program)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                return false;
            };
            let wrote = child
                .stdin
                .take()
                .is_some_and(|mut s| s.write_all(text.as_bytes()).is_ok());
            wrote && child.wait().is_ok_and(|s| s.success())
        };
        let copied = (std::env::var_os("WAYLAND_DISPLAY").is_some() && piped("wl-copy", &[]))
            || (std::env::var_os("DISPLAY").is_some()
                && piped("xclip", &["-selection", "clipboard"]))
            || {
                use crossterm::clipboard::CopyToClipboard;
                crossterm::execute!(std::io::stdout(), CopyToClipboard::to_clipboard_from(text))
                    .is_ok()
            };
        self.toast(if copied {
            "copied to clipboard"
        } else {
            "could not copy"
        });
    }

    fn toast(&mut self, text: &str) {
        self.toast = Some((text.to_string(), Instant::now()));
    }

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        self.hits.clear();
        let area = frame.area();
        let keys = self.key_hints();
        let [bar, body, hints] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(key_hint_rows(&keys, area.width) + 2),
        ])
        .areas(area);

        self.draw_status_bar(frame, bar);
        match self.screen {
            Screen::Main => self
                .main
                .draw(frame, body, &self.state, &self.info, &mut self.hits),
            Screen::Run => self.run_screen.draw(frame, body, &mut self.hits),
            Screen::History => self.history.draw(frame, body, &mut self.hits),
            Screen::Review => {
                if let Some(review) = &mut self.review {
                    review.draw(frame, body, &mut self.hits);
                }
            }
        }
        draw_key_hints(frame, hints, &keys, &mut self.hits);

        if let Some((text, when)) = &self.toast {
            if when.elapsed() < Duration::from_secs(4) {
                let width = (text.chars().count() as u16 + 2).min(area.width);
                let rect = Rect::new(
                    area.x + area.width.saturating_sub(width),
                    hints.y.saturating_sub(1),
                    width,
                    1,
                );
                frame.render_widget(Clear, rect);
                frame.render_widget(
                    Paragraph::new(Span::styled(format!(" {text} "), theme::toast())),
                    rect,
                );
            } else {
                self.toast = None;
            }
        }
        if self.help {
            let apps_file = self.state.paths.tilde(&self.state.paths.apps_file());
            draw_help(frame, area, &keys, &apps_file);
        }
        if let Some(modal) = &mut self.modal {
            modal.draw(frame, area);
        }
    }

    fn key_hints(&self) -> Vec<(&'static str, &'static str)> {
        match self.screen {
            Screen::Main => self.main.key_hints(),
            Screen::Run => self.run_screen.key_hints(),
            Screen::History => self.history.key_hints(),
            Screen::Review => self
                .review
                .as_ref()
                .map(ReviewScreen::key_hints)
                .unwrap_or_default(),
        }
    }

    /// The header box: name and version, sudo state, then notices on the
    /// right. The project link sits on the bottom border unless hidden.
    fn draw_status_bar(&self, frame: &mut ratatui::Frame, area: Rect) {
        use ratatui::widgets::{Block, Borders};
        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border());
        if !self.state.config.settings.hide_credits {
            block = block.title_bottom(
                Line::from(Span::styled(format!(" {CREDITS} "), theme::dim())).right_aligned(),
            );
        }
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let mut spans = vec![
            Span::styled(
                format!(" cachyup v{} ", env!("CARGO_PKG_VERSION")),
                theme::title(),
            ),
            Span::raw(" "),
        ];
        if self.dry_only {
            spans.push(Span::styled(" DRY RUN ONLY ", theme::badge_warn()));
            spans.push(Span::raw(" "));
        } else if self.sudo {
            spans.push(Span::styled(" sudo ", theme::badge_ok()));
            spans.push(Span::raw(" "));
        } else {
            spans.push(Span::styled(" no sudo ", theme::badge_bad()));
            spans.push(Span::raw(" "));
        }
        if self.run.is_some() && self.screen != Screen::Run {
            spans.push(Span::styled(" running ", theme::badge_new()));
            spans.push(Span::raw(" "));
        }
        if self.info_rx.is_some() {
            spans.push(Span::styled("reading package info...", theme::dim()));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), inner);

        // Notices, right-aligned, pipe-separated.
        let mut notices: Vec<Span> = Vec::new();
        let (installed, builtin) = self.main.review_counts();
        if installed + builtin > 0 {
            notices.push(Span::styled("Please review new apps", theme::attention()));
        }
        if let Some(latest) = &self.latest_release {
            notices.push(Span::styled(
                format!("v{latest} available: cachyup update"),
                theme::attention(),
            ));
        }
        if notices.is_empty() {
            return;
        }
        let mut line: Vec<Span> = Vec::new();
        for (i, n) in notices.into_iter().enumerate() {
            if i > 0 {
                line.push(Span::styled(" │ ", theme::dim()));
            }
            line.push(n);
        }
        line.push(Span::raw(" "));
        frame.render_widget(Paragraph::new(Line::from(line)).right_aligned(), inner);
    }
}

/// On the header's bottom border, unless hidden in General.
const CREDITS: &str = "github.com/ReyArlena";

/// Popups for the start of a session, shown one after another.
fn startup_modals(state: &AppState) -> Vec<Modal> {
    let mut out = Vec::new();
    if !state.catalog.warnings.is_empty() {
        let mut text = String::from("About ~/.config/cachyup:\n\n");
        for w in &state.catalog.warnings {
            text.push_str(&format!("  • {}\n", w.trim_start_matches("apps.toml: ")));
        }
        out.push(Modal::message("Your app list", text.trim_end()));
    }
    let name = |key: &String| {
        state.catalog.app(key).map_or(key.clone(), |a| {
            format!("{} ({})", a.display_name(), a.source)
        })
    };
    if !state.launch.removed.is_empty() {
        let mut text = String::from(
            "These apps were uninstalled since cachyup installed them,\nso they have been switched off in your app list:\n\n",
        );
        for key in &state.launch.removed {
            text.push_str(&format!("  • {}\n", name(key)));
        }
        text.push_str("\nSwitch one back on to reinstall it.");
        out.push(Modal::message("Apps removed", &text));
    }
    let installed = state
        .installed_candidates()
        .iter()
        .filter(|c| c.hidden.is_none())
        .count();
    let builtin = state.builtin_candidates().len();
    if installed + builtin > 0 {
        let mut parts = Vec::new();
        if installed > 0 {
            parts.push(format!(
                "{installed} app{} installed on this machine since your last review",
                if installed == 1 { "" } else { "s" }
            ));
        }
        if builtin > 0 {
            parts.push(format!(
                "{builtin} entr{} new in cachyup",
                if builtin == 1 { "y" } else { "ies" }
            ));
        }
        let text = format!(
            "Not in your app list yet:\n\n  • {}\n\nReview them under Settings and tick what to add.",
            parts.join("\n  • ")
        );
        out.push(Modal::new(
            "To review",
            Modal::text(&text),
            vec![
                Button::new(
                    &[KeyCode::Enter, KeyCode::Char('y')],
                    "Enter",
                    "open Settings",
                    Some("open-settings"),
                ),
                Button::new(&[KeyCode::Esc, KeyCode::Char('n')], "Esc", "later", None),
            ],
        ));
    }
    out
}

fn summary_modal(record: &crate::engine::RunRecord) -> Modal {
    let mut body = vec![Line::from(record.counts_line())];
    if let Some(why) = &record.ended_early {
        body.push(Line::from(Span::styled(
            format!("Ended early: {why}"),
            theme::status(Status::Failed),
        )));
    }
    let notable: Vec<_> = record
        .steps
        .iter()
        .filter_map(|s| s.outcome.as_ref().map(|o| (s, o)))
        .filter(|(_, o)| matches!(o.status, Status::Failed | Status::Attention))
        .collect();
    if !notable.is_empty() {
        body.push(Line::from(""));
        for (step, o) in notable {
            body.push(Line::from(vec![
                Span::styled(
                    format!("{} ", theme::status_mark(o.status)),
                    theme::status(o.status),
                ),
                Span::raw(step.name.clone()),
                Span::styled(
                    o.reason
                        .as_ref()
                        .map(|r| format!("  {r}"))
                        .unwrap_or_default(),
                    theme::dim(),
                ),
            ]));
        }
    }
    if record.counts().failed > 0 {
        body.push(Line::from(""));
        body.push(Line::from(Span::styled(
            "Command output for each failure is in the output pane and the log.",
            theme::dim(),
        )));
    }
    let title = if record.dry_run {
        "Dry run finished"
    } else {
        "Run finished"
    };
    let modal = Modal::new(
        title,
        body,
        vec![Button::new(
            &[KeyCode::Enter, KeyCode::Esc, KeyCode::Char('q')],
            "Enter",
            "close",
            None,
        )],
    );
    // Steps that need a human end their reason with "run: <command>".
    let commands: Vec<String> = record
        .steps
        .iter()
        .filter_map(|s| s.outcome.as_ref()?.reason.as_ref())
        .filter_map(|r| r.split_once("run: ").map(|(_, cmd)| cmd.to_string()))
        .collect();
    if commands.is_empty() {
        modal
    } else {
        modal.with_copy(commands.join("\n"))
    }
}

/// The newest release tag if it is newer than this build.
fn latest_release() -> Option<String> {
    let out = crate::system::capture(
        "curl",
        &[
            "-fsSL",
            "--max-time",
            "5",
            "-H",
            "Accept: application/vnd.github+json",
            RELEASE_API,
        ],
    )?;
    let json: serde_json::Value = serde_json::from_str(&out).ok()?;
    let tag = json
        .get("tag_name")?
        .as_str()?
        .trim_start_matches('v')
        .to_string();
    newer(&tag, env!("CARGO_PKG_VERSION")).then_some(tag)
}

/// Compare dotted version numbers.
fn newer(candidate: &str, current: &str) -> bool {
    let parse =
        |v: &str| -> Vec<u64> { v.split(['.', '-']).map_while(|p| p.parse().ok()).collect() };
    parse(candidate) > parse(current)
}

/// Descriptions for a review: the local package database for installed
/// apps, the package sources (network) for cachyup's own entries.
fn spawn_review_info(review: &ReviewScreen, state: &AppState) -> Receiver<InfoMap> {
    let (tx, rx) = mpsc::channel();
    let keys = review.keys();
    let kind = review.kind();
    let installed = state.installed.clone();
    let paths = state.paths.clone();
    std::thread::spawn(move || {
        let info = match kind {
            ReviewKind::Installed => crate::system::installed_info(&keys),
            ReviewKind::Builtin => {
                // Only the offered apps, so one AUR request covers them.
                let builtin = crate::catalog::Catalog::builtin();
                let subset = crate::catalog::Catalog {
                    apps: builtin
                        .apps
                        .iter()
                        .filter(|a| keys.contains(&a.key()))
                        .cloned()
                        .collect(),
                    ..Default::default()
                };
                fetch_info(&subset, &installed, &paths)
            }
        };
        let _ = tx.send(info);
    });
    rx
}

/// Package descriptions, read in the background and cached.
fn spawn_info_fetch(state: &AppState) -> Receiver<InfoMap> {
    let (tx, rx) = mpsc::channel();
    let catalog = state.catalog.clone();
    let installed = state.installed.clone();
    let paths = state.paths.clone();
    std::thread::spawn(move || {
        let info = fetch_info(&catalog, &installed, &paths);
        save_info_cache(&paths.info_cache(), &info);
        let _ = tx.send(info);
    });
    rx
}

/// `pacman = [...]`, `aur = [...]`, `flatpak = [...]` for the given keys,
/// the text the merge code appends from.
fn list_snippet(keys: &[String]) -> String {
    let mut lists: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
    for key in keys {
        if let Some((source, name)) = key.split_once('/')
            && matches!(source, "pacman" | "aur" | "flatpak")
        {
            lists.entry(source).or_default().push(name.to_string());
        }
    }
    toml::to_string(&lists).unwrap_or_default()
}

fn open_url(url: &str) {
    let _ = std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Hooks for the render tests in `tests/tui_render.rs`.
impl App {
    pub fn draw_for_test(&mut self, frame: &mut ratatui::Frame) {
        self.draw(frame);
    }

    pub fn key_for_test(&mut self, key: crossterm::event::KeyEvent) {
        self.on_terminal_event(TermEvent::Key(key));
    }

    /// Run a dry run to the end, feeding every event to the run screen.
    pub fn dry_run_for_test(&mut self) {
        self.start(true);
        while self.run.is_some() {
            self.poll_background();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn screen_name_for_test(&self) -> &'static str {
        match self.screen {
            Screen::Main => "main",
            Screen::Run => "run",
            Screen::History => "history",
            Screen::Review => "review",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_copies_the_commands_to_run() {
        use crate::engine::{Outcome, RunRecord, StepRecord};
        let step = |name: &str, outcome: Outcome| StepRecord {
            group: "Post-update checks".into(),
            name: name.into(),
            outcome: Some(outcome),
        };
        let record = RunRecord {
            id: "x".into(),
            version: "0".into(),
            started: chrono::Local::now(),
            finished: None,
            dry_run: false,
            tasks: vec![],
            steps: vec![
                step(
                    ".pacnew check",
                    Outcome::attention("4 to merge, run: DIFFPROG=meld pacdiff --sudo"),
                ),
                step("Reboot check", Outcome::attention("kernel was updated")),
                step("AUR rebuild check", Outcome::pass()),
            ],
            ended_early: None,
            managed: vec![],
        };
        let modal = summary_modal(&record);
        assert_eq!(modal.copy.as_deref(), Some("DIFFPROG=meld pacdiff --sudo"));
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(newer("0.10.0", "0.9.3"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
    }
}
