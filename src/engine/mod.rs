//! The engine: turns the configuration into an ordered list of steps and runs
//! them on a worker thread, streaming everything as [`Event`]s. It knows
//! nothing about terminals; the TUI and the headless CLI both consume events.

pub mod ctx;
pub mod edit;
pub mod plan;
pub mod record;
pub mod runner;
mod steps;

use std::fmt;

use serde::{Deserialize, Serialize};

pub use ctx::Ctx;
pub use plan::{Plan, build_plan};
pub use record::{RunRecord, StepRecord};
pub use runner::{RunHandle, start_run};

/// The result of one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    /// Something was installed or changed.
    Success,
    /// Already in place, or not applicable.
    Skipped,
    /// Look at the log.
    Failed,
    /// A check that changes nothing came out fine. Not counted.
    Pass,
    /// A check found something that needs a human (.pacnew, reboot, ...).
    Attention,
    /// Dry run: this is what would have been done.
    Would,
    /// Never ran: the run was stopped or aborted first.
    Cancelled,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Success => "SUCCESS",
            Status::Skipped => "SKIPPED",
            Status::Failed => "FAILED",
            Status::Pass => "PASS",
            Status::Attention => "ATTENTION",
            Status::Would => "WOULD RUN",
            Status::Cancelled => "CANCELLED",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub status: Status,
    pub reason: Option<String>,
}

impl Outcome {
    pub fn new(status: Status, reason: impl Into<Option<String>>) -> Self {
        Self {
            status,
            reason: reason.into(),
        }
    }
    pub fn success() -> Self {
        Self::new(Status::Success, None)
    }
    pub fn pass() -> Self {
        Self::new(Status::Pass, None)
    }
    pub fn skipped(reason: &str) -> Self {
        Self::new(Status::Skipped, Some(reason.to_string()))
    }
    pub fn failed(reason: impl Into<String>) -> Self {
        Self::new(Status::Failed, Some(reason.into()))
    }
    pub fn attention(reason: impl Into<String>) -> Self {
        Self::new(Status::Attention, Some(reason.into()))
    }
}

/// One line of the status recap, as `name.....STATUS (reason)`.
pub fn status_line(name: &str, outcome: &Outcome) -> String {
    const WIDTH: usize = 44;
    let pad = WIDTH.saturating_sub(name.chars().count()).max(3);
    let mut line = format!("{name}{}{}", ".".repeat(pad), outcome.status.label());
    if let Some(reason) = &outcome.reason {
        line.push_str(&format!(" ({reason})"));
    }
    line
}

/// What the worker thread reports.
#[derive(Debug, Clone)]
pub enum Event {
    /// A step began.
    StepStarted(usize),
    /// A finished line of command output.
    Line(String),
    /// The line currently being redrawn with `\r` (progress bars). Replaced by
    /// the next `Partial` or `Line`.
    Partial(String),
    /// A dim informational line, e.g. the command about to run.
    Note(String),
    /// A group of steps begins (`==> System update`).
    Heading(String),
    /// A `name.....STATUS (reason)` line.
    Status {
        text: String,
        status: Status,
    },
    StepFinished(usize, Outcome),
    /// The run is over (finished, stopped or aborted). Always the last event.
    Finished(Box<RunRecord>),
}

/// A command to run, optionally through `sudo -n`.
#[derive(Debug, Clone, PartialEq)]
pub struct Cmd {
    pub program: String,
    pub args: Vec<String>,
    pub sudo: bool,
}

impl Cmd {
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            sudo: false,
        }
    }

    pub fn sudo(program: &str, args: &[&str]) -> Self {
        Self {
            sudo: true,
            ..Self::new(program, args)
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// How the command reads in the log.
    pub fn display(&self) -> String {
        let mut parts = Vec::new();
        if self.sudo {
            parts.push("sudo".to_string());
        }
        parts.push(self.program.clone());
        for a in &self.args {
            if a.is_empty() || a.contains(char::is_whitespace) || a.contains(['|', '&', ';']) {
                parts.push(format!("'{a}'"));
            } else {
                parts.push(a.clone());
            }
        }
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines_pad_like_the_bash_version() {
        let line = status_line("discord", &Outcome::skipped("already present"));
        assert_eq!(
            line,
            "discord.....................................SKIPPED (already present)"
        );
        let long = "x".repeat(50);
        assert!(status_line(&long, &Outcome::success()).ends_with("...SUCCESS"));
    }

    #[test]
    fn commands_display_with_sudo_and_quotes() {
        let cmd = Cmd::sudo("pacman", &["-S", "--noconfirm"]).arg("a b");
        assert_eq!(cmd.display(), "sudo pacman -S --noconfirm 'a b'");
    }
}
