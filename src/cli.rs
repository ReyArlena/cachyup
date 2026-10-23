//! Command line. No arguments opens the TUI; `--run` runs the saved settings
//! without it, for scripts and cron.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Result, bail};
use clap::Parser;

use crate::app::{AppState, is_root};
use crate::engine::runner::RunOptions;
use crate::engine::{Event, Status, start_run};
use crate::paths::Paths;

#[derive(Debug, Parser)]
#[command(
    name = "cachyup",
    version,
    long_version = concat!(env!("CARGO_PKG_VERSION"), "\n", env!("CARGO_PKG_REPOSITORY")),
    about = "Set up and update a CachyOS install: system update, apps, tweaks and cleanup.",
    after_help = "Without options, opens the interactive interface.\n\
                  Settings: ~/.config/cachyup   Logs: ~/.local/state/cachyup/runs"
)]
pub struct Cli {
    /// Run the saved settings without the interface, then exit.
    #[arg(long)]
    pub run: bool,

    /// Do not change anything: run every check and log what would be done.
    /// With --run, or on its own to open the interface in dry-run mode.
    #[arg(long, short = 'n')]
    pub dry_run: bool,

    /// Delete all settings, state and history, so the next launch is like a
    /// fresh install. Asks first.
    #[arg(long)]
    pub reset: bool,

    /// With --reset: do not ask.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

/// Exit codes: 0 fine, 1 a step failed, 2 error, 3 stopped or aborted early.
pub fn run_cli(cli: Cli) -> Result<i32> {
    let paths = Paths::from_env();
    if cli.reset {
        return reset(&paths, cli.yes);
    }
    if is_root() {
        bail!(
            "run cachyup as your normal user, not root. AUR builds and home-directory \
             config must be owned by you; sudo is asked for when needed"
        );
    }
    if cli.run {
        return headless(paths, cli.dry_run);
    }
    crate::tui::run(paths, cli.dry_run)
}

fn reset(paths: &Paths, yes: bool) -> Result<i32> {
    println!("This deletes all cachyup settings, state and run history:");
    for dir in [&paths.config_dir, &paths.state_dir, &paths.cache_dir] {
        println!("  {}", paths.tilde(dir));
    }
    if !yes {
        if !std::io::stdin().is_terminal() {
            bail!("refusing to reset without a terminal; pass --yes");
        }
        print!("Continue? [y/N] ");
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("Nothing deleted.");
            return Ok(0);
        }
    }
    AppState::reset(paths)?;
    println!("Done. The next launch starts fresh.");
    Ok(0)
}

static INTERRUPTS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn on_sigint(_: nix::libc::c_int) {
    INTERRUPTS.fetch_add(1, Ordering::SeqCst);
}

fn headless(paths: Paths, dry: bool) -> Result<i32> {
    let mut app = AppState::load(paths)?;
    for w in &app.catalog.warnings {
        eprintln!("warning: {w}");
    }
    if !app.launch.removed.is_empty() {
        eprintln!(
            "Uninstalled since cachyup installed them, now switched off: {}",
            app.launch.removed.join(", ")
        );
    }
    if !app.launch.new_apps.is_empty() {
        eprintln!(
            "New in cachyup (off until you switch them on): {}",
            app.launch.new_apps.join(", ")
        );
    }
    if !crate::sudo::ensure(std::io::stdin().is_terminal()) && !dry {
        bail!("sudo is needed for a real run (try again from a terminal, or use --dry-run)");
    }

    let plan = app.plan(dry);
    if plan.steps.is_empty() {
        println!("Nothing to do: every task is switched off (see ~/.config/cachyup/config.toml).");
        return Ok(0);
    }

    // First Ctrl-C: finish the current step, then stop. Second: stop now.
    // The commands run in their own process group, so the terminal's SIGINT
    // never reaches pacman mid-transaction.
    let action = nix::sys::signal::SigAction::new(
        nix::sys::signal::SigHandler::Handler(on_sigint),
        nix::sys::signal::SaFlags::SA_RESTART,
        nix::sys::signal::SigSet::empty(),
    );
    // SAFETY: the handler only touches an atomic.
    unsafe {
        let _ = nix::sys::signal::sigaction(nix::sys::signal::Signal::SIGINT, &action);
    }

    let colour = std::io::stdout().is_terminal();
    let handle = start_run(
        plan,
        RunOptions {
            dry,
            paths: app.paths.clone(),
            settings: app.config.settings.clone(),
        },
    );
    let mut seen_interrupts = 0;
    let mut partial = false;
    let record = loop {
        let n = INTERRUPTS.load(Ordering::SeqCst);
        if n != seen_interrupts {
            seen_interrupts = n;
            if n == 1 {
                handle.stop_after_step();
                eprintln!("\nStopping after the current step (Ctrl-C again to stop now)");
            } else {
                handle.stop_now();
                eprintln!("\nStopping now");
            }
        }
        let event = match handle.events.recv_timeout(Duration::from_millis(200)) {
            Ok(e) => e,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => bail!("the run ended unexpectedly"),
        };
        if partial && !matches!(event, Event::Partial(_)) {
            print!("\r\x1b[K");
            partial = false;
        }
        match event {
            Event::Heading(h) => println!("\n{}", paint(colour, "1", &format!("==> {h}"))),
            Event::Line(l) => println!("{l}"),
            Event::Note(n) => println!("{}", paint(colour, "2", &format!("    {n}"))),
            Event::Partial(p) => {
                if colour {
                    print!("\r\x1b[K{}", truncate(&p, 200));
                    let _ = std::io::stdout().flush();
                    partial = true;
                }
            }
            Event::Status { text, status } => println!("{}", status_text(colour, &text, status)),
            Event::StepStarted(_) | Event::StepFinished(..) => {}
            Event::Finished(record) => break record,
        }
    };
    app.record_run(&record)?;
    println!("\nLog: {}", app.paths.tilde(&handle.dir.join("output.log")));
    Ok(if record.ended_early.is_some() {
        3
    } else if record.counts().failed > 0 {
        1
    } else {
        0
    })
}

fn paint(colour: bool, code: &str, text: &str) -> String {
    if colour {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Colour only the status word, as the bash version did.
fn status_text(colour: bool, text: &str, status: Status) -> String {
    if !colour {
        return text.to_string();
    }
    let label = status.label();
    let Some(at) = text.find(&format!(".{label}")) else {
        return text.to_string();
    };
    let at = at + 1;
    format!(
        "{}{}{}",
        &text[..at],
        paint(true, status_code(status), label),
        &text[at + label.len()..]
    )
}

pub fn status_code(status: Status) -> &'static str {
    match status {
        Status::Success | Status::Pass => "32",
        Status::Skipped | Status::Attention => "33",
        Status::Failed => "31",
        Status::Would => "36",
        Status::Cancelled => "2",
    }
}

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}
