//! Terminal user interface built on ratatui.
//!
//! - `app.rs`          the `App`: event loop, modals, effects, the runner
//! - `main_screen.rs`  tasks and their options
//! - `run_screen.rs`   a run in progress
//! - `history.rs`      past runs
//! - `widgets.rs`      panes, modals, text input, the output view
//! - `input.rs`        key and mouse mapping, the click map
//!
//! Sudo is asked for before the TUI is drawn. If it is needed again later the
//! TUI steps aside (restores the terminal), sudo prompts normally, and the
//! TUI comes back.

mod app;
mod history;
mod input;
mod main_screen;
mod run_screen;
mod theme;
mod widgets;

use std::io::stdout;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;

use crate::app::AppState;
use crate::paths::Paths;

pub use app::App;

/// What a screen asks the app to do.
pub enum Effect {
    Modal(widgets::Modal),
    Toast(String),
    SaveConfig,
    SaveSelection,
    /// Show the run confirmation.
    AskRun,
    /// Show the stop-the-run choice.
    AskStop,
    /// Leave a finished run.
    CloseRun,
    History,
    CloseHistory,
    Quit,
}

pub fn run(paths: Paths, dry_only: bool) -> Result<i32> {
    // Before anything is drawn, so sudo can prompt in the plain terminal.
    let sudo = if dry_only {
        crate::sudo::ensure(false)
    } else {
        crate::sudo::ensure(true)
    };
    let state = AppState::load(paths)?;

    let paused = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    spawn_input_thread(tx, paused.clone());

    let mut app = App::new(state, sudo, dry_only);
    loop {
        let mut terminal = ratatui::init();
        let _ = execute!(stdout(), EnableMouseCapture);
        let result = app.run(&mut terminal, &rx);
        let _ = execute!(stdout(), DisableMouseCapture);
        ratatui::restore();
        match result? {
            app::Exit::Quit => return Ok(0),
            app::Exit::Authenticate => {
                // Stop reading keys so sudo gets the password, not us.
                paused.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(150));
                while rx.try_recv().is_ok() {}
                println!();
                let ok = crate::sudo::ensure(true);
                while rx.try_recv().is_ok() {}
                paused.store(false, Ordering::SeqCst);
                app.authenticated(ok);
            }
        }
    }
}

/// crossterm's reader blocks, so it gets its own thread. While `paused` it
/// leaves the terminal alone.
fn spawn_input_thread(tx: mpsc::Sender<crossterm::event::Event>, paused: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        loop {
            if paused.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            if crossterm::event::poll(Duration::from_millis(100)).unwrap_or(false)
                && !paused.load(Ordering::SeqCst)
                && let Ok(event) = crossterm::event::read()
                && tx.send(event).is_err()
            {
                return;
            }
        }
    });
}
