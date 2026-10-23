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
use super::run_screen::RunScreen;
use super::theme;
use super::widgets::{Button, Modal, draw_help, draw_key_hints};

pub const INSTALL_CMD: &str = "curl -fsSL https://reyarlena.github.io/cachyup/install.sh | sh";
const RELEASES_URL: &str = "https://github.com/ReyArlena/cachyup/releases/latest";
const RELEASE_API: &str = "https://api.github.com/repos/ReyArlena/cachyup/releases/latest";

pub enum Exit {
    Quit,
    /// Step aside so sudo can ask for the password, then come back.
    Authenticate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Main,
    Run,
    History,
}

pub struct App {
    state: AppState,
    screen: Screen,
    /// Where Esc from history goes back to.
    history_from: Screen,
    main: MainScreen,
    run_screen: RunScreen,
    history: HistoryScreen,
    modal: Option<Modal>,
    startup: VecDeque<Modal>,
    help: bool,
    hits: HitMap,
    toast: Option<(String, Instant)>,
    run: Option<RunHandle>,
    info: InfoMap,
    info_rx: Option<Receiver<InfoMap>>,
    update_rx: Option<Receiver<String>>,
    sudo: bool,
    /// Started with --dry-run: real runs are off.
    dry_only: bool,
    /// A real run waiting for sudo.
    pending_run: bool,
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
        let (info_tx, info_rx) = mpsc::channel();
        {
            let catalog = state.catalog.clone();
            let installed = state.installed.clone();
            let paths = state.paths.clone();
            std::thread::spawn(move || {
                let info = fetch_info(&catalog, &installed, &paths);
                save_info_cache(&paths.info_cache(), &info);
                let _ = info_tx.send(info);
            });
        }
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
            modal: startup.pop_front(),
            startup,
            help: false,
            hits: HitMap::default(),
            toast: None,
            run: None,
            info,
            info_rx: Some(info_rx),
            update_rx,
            sudo,
            dry_only,
            pending_run: false,
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
        if let Some(rx) = &self.update_rx
            && let Ok(latest) = rx.try_recv()
        {
            self.update_rx = None;
            let modal = Modal::new(
                "Update available",
                Modal::text(&format!(
                    "cachyup {latest} is out (you have {}).\n\nTo update, run the install command again:\n  {INSTALL_CMD}",
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
                return;
            }
            if self.screen == Screen::Main && self.main.is_filtering() {
                self.main.filter_key(*key, &self.state);
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
            "review-new" => {
                self.screen = Screen::Main;
                self.main.review_new(&self.state);
            }
            "open-release" => open_url(RELEASES_URL),
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
                .filter(|a| state.selection.app_on(&a.key()))
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
        if state.config.tasks.tweaks {
            let n = state
                .catalog
                .tweaks
                .iter()
                .filter(|t| state.selection.tweak_on(&t.id))
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
        let [bar, body, hints] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .areas(area);

        self.draw_status_bar(frame, bar);
        match self.screen {
            Screen::Main => self
                .main
                .draw(frame, body, &self.state, &self.info, &mut self.hits),
            Screen::Run => self.run_screen.draw(frame, body, &mut self.hits),
            Screen::History => self.history.draw(frame, body, &mut self.hits),
        }
        let keys = self.key_hints();
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
            draw_help(frame, area, &keys);
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
        }
    }

    fn draw_status_bar(&self, frame: &mut ratatui::Frame, area: Rect) {
        let mut spans = vec![
            Span::styled(
                format!(" cachyup {} ", env!("CARGO_PKG_VERSION")),
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
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}

/// Popups for the start of a session, shown one after another.
fn startup_modals(state: &AppState) -> Vec<Modal> {
    let mut out = Vec::new();
    if !state.catalog.warnings.is_empty() {
        let mut text = String::from("Problems in ~/.config/cachyup/user-apps.toml:\n\n");
        for w in &state.catalog.warnings {
            text.push_str(&format!(
                "  • {}\n",
                w.trim_start_matches("user-apps.toml: ")
            ));
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
    let new_count = state.launch.new_apps.len() + state.launch.new_tweaks.len();
    if new_count > 0 {
        let mut text =
            String::from("cachyup was updated and has new entries. They start switched off:\n\n");
        for key in &state.launch.new_apps {
            text.push_str(&format!("  • {}\n", name(key)));
        }
        for id in &state.launch.new_tweaks {
            if let Some(t) = state.catalog.tweak(id) {
                text.push_str(&format!("  • tweak: {}\n", t.name));
            }
        }
        text.push_str("\nReview them now?");
        out.push(Modal::new(
            "New in cachyup",
            Modal::text(&text),
            vec![
                Button::new(
                    &[KeyCode::Enter, KeyCode::Char('y')],
                    "Enter",
                    "review",
                    Some("review-new"),
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
