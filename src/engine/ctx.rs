//! What a step gets to work with: run commands (streamed to the log and the
//! screen), ask read-only questions, and change files. In a dry run every
//! change is logged as "would run" instead of done; questions still run for real.

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::Settings;
use crate::paths::Paths;

use super::{Cmd, Event};

/// How long a force stop waits after SIGTERM before SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(10);
/// After a command exits, how long to keep reading output that something it
/// left running in the background may still hold open.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

/// Shared between the worker and whoever wants to stop it.
#[derive(Debug, Default)]
pub struct Control {
    /// Finish the current step, then stop.
    pub stop: AtomicBool,
    /// Kill the current command now.
    pub force: AtomicBool,
    /// Process group of the command running right now.
    pub child: Mutex<Option<i32>>,
}

impl Control {
    pub fn kill_child(&self, signal: nix::sys::signal::Signal) {
        if let Some(pgid) = *self.child.lock().unwrap() {
            let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pgid), signal);
        }
    }
}

pub struct Ctx {
    pub dry: bool,
    pub paths: Paths,
    pub settings: Settings,
    pub control: Arc<Control>,
    events: Sender<Event>,
    log: Option<File>,
    /// Last non-empty output line of the latest command, for FAILED reasons.
    pub last_line: String,
    /// App keys confirmed installed during this run.
    pub managed: Vec<String>,
    /// Lazily filled per run: what each source has.
    pub repo_available: Option<Option<HashSet<String>>>,
    pub flathub_available: Option<Option<HashSet<String>>>,
    /// `checkrebuild` running in the background since the AUR update.
    pub rebuild: Option<std::thread::JoinHandle<Option<String>>>,
}

impl Ctx {
    pub fn new(
        dry: bool,
        paths: Paths,
        settings: Settings,
        control: Arc<Control>,
        events: Sender<Event>,
        log: Option<File>,
    ) -> Self {
        Self {
            dry,
            paths,
            settings,
            control,
            events,
            log,
            last_line: String::new(),
            managed: Vec::new(),
            repo_available: None,
            flathub_available: None,
            rebuild: None,
        }
    }

    pub(super) fn send(&self, event: Event) {
        let _ = self.events.send(event);
    }

    /// Append a raw line to the log only.
    pub(super) fn log(&mut self, text: &str) {
        if let Some(log) = &mut self.log {
            let _ = writeln!(log, "{text}");
        }
    }

    /// A line of output, in the log and on screen.
    pub fn line(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.log(&text);
        self.send(Event::Line(text));
    }

    /// A dim informational line.
    pub fn note(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.log(&format!("    {text}"));
        self.send(Event::Note(text));
    }

    /// Run a command, streaming its output. In a dry run, only log it.
    /// `Err` carries the reason for a FAILED line (last line of output).
    pub fn exec(&mut self, cmd: &Cmd) -> Result<(), String> {
        if self.dry {
            self.note(format!("would run: {}", cmd.display()));
            return Ok(());
        }
        self.run_streamed(cmd)
    }

    /// Run a command for real even in a dry run. Only for commands that change
    /// nothing but whose output belongs in the log.
    pub fn exec_readonly(&mut self, cmd: &Cmd) -> Result<(), String> {
        self.run_streamed(cmd)
    }

    fn run_streamed(&mut self, cmd: &Cmd) -> Result<(), String> {
        if self.control.force.load(Ordering::SeqCst) {
            return Err("stopped".into());
        }
        self.note(format!("$ {}", cmd.display()));
        self.last_line.clear();

        let (reader, writer) = std::io::pipe().map_err(|e| e.to_string())?;
        let mut command = build(cmd);
        command
            .stdin(Stdio::null())
            .stdout(writer.try_clone().map_err(|e| e.to_string())?)
            .stderr(writer)
            // Own process group: a force stop can signal the whole tree, and
            // Ctrl-C in a plain terminal does not reach it mid-transaction.
            .process_group(0);
        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("{}: {e}", cmd.program);
                self.line(msg.clone());
                return Err(msg);
            }
        };
        // Drop our copies of the pipe's write end, or the read never ends.
        drop(command);
        *self.control.child.lock().unwrap() = Some(child.id() as i32);

        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            return;
                        }
                    }
                }
            }
        });

        let mut splitter = LineSplitter::default();
        let mut killed_at: Option<Instant> = None;
        let status;
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => self.pieces(splitter.push(&chunk)),
                Err(RecvTimeoutError::Disconnected) => {
                    status = child.wait().ok();
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Ok(Some(s)) = child.try_wait() {
                        status = Some(s);
                        // Read what is left, but do not wait on a daemon the
                        // command started that inherited the pipe.
                        while let Ok(chunk) = rx.recv_timeout(DRAIN_GRACE) {
                            self.pieces(splitter.push(&chunk));
                        }
                        break;
                    }
                }
            }
            if self.control.force.load(Ordering::SeqCst) {
                match killed_at {
                    None => {
                        self.control.kill_child(nix::sys::signal::Signal::SIGTERM);
                        killed_at = Some(Instant::now());
                    }
                    Some(t) if t.elapsed() > KILL_GRACE => {
                        self.control.kill_child(nix::sys::signal::Signal::SIGKILL);
                    }
                    _ => {}
                }
            }
        }
        self.pieces(splitter.finish());
        *self.control.child.lock().unwrap() = None;

        match status {
            Some(s) if s.success() => Ok(()),
            _ if killed_at.is_some() => Err("stopped".into()),
            Some(s) => Err(if self.last_line.is_empty() {
                format!(
                    "exit {}",
                    s.code().map_or("signal".into(), |c| c.to_string())
                )
            } else {
                self.last_line.clone()
            }),
            None => Err("could not wait for the command".into()),
        }
    }

    fn pieces(&mut self, pieces: Vec<Piece>) {
        for piece in pieces {
            match piece {
                Piece::Line(text) => {
                    if !text.trim().is_empty() {
                        self.last_line = truncate(text.trim(), 80);
                    }
                    self.line(text);
                }
                Piece::Partial(text) => self.send(Event::Partial(text)),
            }
        }
    }

    /// Ask a read-only question: stdout, or `None` if the command failed.
    pub fn query(&self, program: &str, args: &[&str]) -> Option<String> {
        crate::system::capture(program, args)
    }

    /// Same, through `sudo -n` (never prompts; fails without a ticket).
    pub fn query_sudo(&self, program: &str, args: &[&str]) -> Option<String> {
        let mut all = vec!["-n", program];
        all.extend_from_slice(args);
        crate::system::capture("sudo", &all)
    }

    /// Same, with text on stdin. Output is stdout and stderr together and
    /// returned whatever the exit status.
    pub fn query_with_input(&self, program: &str, args: &[&str], input: &str) -> Option<String> {
        let mut child = Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input.as_bytes());
        }
        let out = child.wait_with_output().ok()?;
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        Some(text)
    }

    /// Change something in-process (write a file, remove a directory). In a
    /// dry run only `what` is logged.
    pub fn mutate(
        &mut self,
        what: &str,
        change: impl FnOnce() -> anyhow::Result<()>,
    ) -> Result<(), String> {
        if self.dry {
            self.note(format!("would: {what}"));
            return Ok(());
        }
        self.line(what.to_string());
        change().map_err(|e| {
            let msg = format!("{e:#}");
            self.line(msg.clone());
            msg
        })
    }
}

fn build(cmd: &Cmd) -> Command {
    if cmd.sudo {
        let mut c = Command::new("sudo");
        c.arg("-n").arg(&cmd.program).args(&cmd.args);
        c
    } else {
        let mut c = Command::new(&cmd.program);
        c.args(&cmd.args);
        c
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        text.chars().take(max).collect()
    }
}

#[derive(Debug, PartialEq)]
pub enum Piece {
    Line(String),
    Partial(String),
}

/// Turns a byte stream into lines. Strips ANSI escapes; a bare `\r` makes the
/// text so far a `Partial` (the next one replaces it on screen), so progress
/// bars redraw in place and only their final state reaches the log.
#[derive(Default)]
pub struct LineSplitter {
    buf: Vec<u8>,
    /// Text of the line that was ended by a `\r`, kept in case `\n` follows.
    pending: Option<String>,
}

impl LineSplitter {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Piece> {
        let mut out = Vec::new();
        for &b in bytes {
            match b {
                b'\n' => {
                    let text = self.take();
                    let pending = self.pending.take();
                    // "progress 100%\r\n": the line is the redrawn text.
                    let line = if text.is_empty() {
                        pending.unwrap_or_default()
                    } else {
                        text
                    };
                    out.push(Piece::Line(line));
                }
                b'\r' => {
                    let text = self.take();
                    if !text.is_empty() {
                        out.push(Piece::Partial(text.clone()));
                        self.pending = Some(text);
                    }
                }
                _ => self.buf.push(b),
            }
        }
        out
    }

    /// Whatever is left when the stream ends.
    pub fn finish(&mut self) -> Vec<Piece> {
        let text = self.take();
        let last = if text.is_empty() {
            self.pending.take()
        } else {
            Some(text)
        };
        last.map(Piece::Line).into_iter().collect()
    }

    fn take(&mut self) -> String {
        let raw = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        strip_ansi(&raw)
    }
}

/// Remove ANSI escape sequences (colours, cursor movement) and other control
/// characters except tab.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.next() {
                // CSI: parameters, then one final byte in @..~
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: until BEL or ESC \
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if c == '\t' || !c.is_control() {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colours_and_osc() {
        assert_eq!(strip_ansi("\x1b[1;32mok\x1b[0m done"), "ok done");
        assert_eq!(strip_ansi("\x1b]0;title\x07text"), "text");
        assert_eq!(strip_ansi("a\x08b"), "ab");
    }

    #[test]
    fn splits_lines_and_progress() {
        let mut s = LineSplitter::default();
        let p = s.push(b"one\ntw");
        assert_eq!(p, vec![Piece::Line("one".into())]);
        let p = s.push(b"o\n 10%\r 50%\r100%\r\nend");
        assert_eq!(
            p,
            vec![
                Piece::Line("two".into()),
                Piece::Partial(" 10%".into()),
                Piece::Partial(" 50%".into()),
                Piece::Partial("100%".into()),
                Piece::Line("100%".into()),
            ]
        );
        assert_eq!(s.finish(), vec![Piece::Line("end".into())]);
    }

    fn ctx(dry: bool) -> (Ctx, mpsc::Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        let dir = std::env::temp_dir();
        let ctx = Ctx::new(
            dry,
            Paths::under(&dir),
            Settings::default(),
            Arc::new(Control::default()),
            tx,
            None,
        );
        (ctx, rx)
    }

    #[test]
    fn exec_streams_and_reports_last_line() {
        let (mut cx, rx) = ctx(false);
        let ok = cx.exec(&Cmd::new(
            "sh",
            &["-c", "echo hello; echo oops >&2; exit 3"],
        ));
        assert_eq!(ok, Err("oops".into()));
        let lines: Vec<String> = rx
            .try_iter()
            .filter_map(|e| match e {
                Event::Line(l) => Some(l),
                _ => None,
            })
            .collect();
        assert_eq!(lines, ["hello", "oops"]);
    }

    #[test]
    fn exec_does_not_hang_on_background_children() {
        let (mut cx, _rx) = ctx(false);
        let start = Instant::now();
        let ok = cx.exec(&Cmd::new("sh", &["-c", "sleep 5 & echo started"]));
        assert_eq!(ok, Ok(()));
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn dry_run_only_logs() {
        let (mut cx, rx) = ctx(true);
        let marker = std::env::temp_dir().join("cachyup-dry-run-marker");
        let _ = std::fs::remove_file(&marker);
        let cmd = Cmd::new("touch", &[marker.to_str().unwrap()]);
        assert_eq!(cx.exec(&cmd), Ok(()));
        assert!(!marker.exists());
        assert!(matches!(rx.try_recv(), Ok(Event::Note(n)) if n.starts_with("would run: touch")));
        assert_eq!(cx.mutate("write x", || panic!("must not run")), Ok(()));
    }
}
