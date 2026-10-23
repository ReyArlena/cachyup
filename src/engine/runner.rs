//! Runs a plan on a worker thread. Writes `output.log` as it goes (so nothing
//! is lost on a crash or Ctrl-C), rewrites `run.json` after every step, and
//! streams [`Event`]s to whoever is watching.

use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};

use chrono::Local;

use crate::config::Settings;
use crate::paths::Paths;

use super::ctx::Control;
use super::plan::Plan;
use super::record::{LOG_FILE, RunRecord, StepRecord};
use super::{Ctx, Event, Outcome, Status, status_line};

pub struct RunHandle {
    pub events: Receiver<Event>,
    pub control: Arc<Control>,
    pub dir: PathBuf,
}

impl RunHandle {
    /// Finish the current step, then stop.
    pub fn stop_after_step(&self) {
        self.control.stop.store(true, Ordering::SeqCst);
    }

    /// Stop now: the running command gets SIGTERM, then SIGKILL.
    pub fn stop_now(&self) {
        self.control.stop.store(true, Ordering::SeqCst);
        self.control.force.store(true, Ordering::SeqCst);
    }
}

pub struct RunOptions {
    pub dry: bool,
    pub paths: Paths,
    pub settings: Settings,
}

pub fn start_run(plan: Plan, opts: RunOptions) -> RunHandle {
    let (tx, rx) = mpsc::channel();
    let control = Arc::new(Control::default());
    let started = Local::now();
    let id = RunRecord::new_id(started);
    let dir = opts.paths.runs_dir().join(&id);
    let handle = RunHandle {
        events: rx,
        control: control.clone(),
        dir: dir.clone(),
    };

    std::thread::spawn(move || {
        let _ = std::fs::create_dir_all(&dir);
        let log = File::create(dir.join(LOG_FILE)).ok();
        let mut record = RunRecord {
            id,
            version: env!("CARGO_PKG_VERSION").into(),
            started,
            finished: None,
            dry_run: opts.dry,
            tasks: plan.tasks.clone(),
            steps: plan
                .visible()
                .map(|s| StepRecord {
                    group: s.group.clone(),
                    name: s.name.clone(),
                    outcome: None,
                })
                .collect(),
            ended_early: None,
            managed: Vec::new(),
        };
        let _ = record.save(&dir);

        let mut cx = Ctx::new(
            opts.dry,
            opts.paths.clone(),
            opts.settings.clone(),
            control.clone(),
            tx.clone(),
            log,
        );
        cx.log(&format!(
            "cachyup {}{}, started {}",
            record.version,
            if opts.dry { " (dry run)" } else { "" },
            started.format("%Y-%m-%d %H:%M:%S")
        ));

        let mut group: Option<String> = None;
        let mut visible = 0usize;
        for step in plan.steps {
            if control.stop.load(Ordering::SeqCst) {
                record.ended_early = Some("stopped by the user".into());
                break;
            }
            if step.hidden {
                let _ = (step.body)(&mut cx);
                continue;
            }
            if group.as_deref() != Some(step.group.as_str()) {
                cx.log(&format!("\n==> {}", step.group));
                cx.send(Event::Heading(step.group.clone()));
                group = Some(step.group.clone());
            }
            let index = visible;
            visible += 1;
            cx.send(Event::StepStarted(index));
            let body = step.body;
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&mut cx)))
                .unwrap_or_else(|_| Outcome::failed("internal error (see log)"));
            let line = status_line(&step.name, &outcome);
            cx.log(&line);
            cx.send(Event::Status {
                text: line,
                status: outcome.status,
            });
            record.steps[index].outcome = Some(outcome.clone());
            let _ = record.save(&dir);
            cx.send(Event::StepFinished(index, outcome.clone()));

            if step.required && outcome.status == Status::Failed {
                record.ended_early = Some(format!("aborted: {} failed", step.name));
                break;
            }
        }

        if record.ended_early.is_some() {
            for step in record.steps.iter_mut().filter(|s| s.outcome.is_none()) {
                step.outcome = Some(Outcome::new(Status::Cancelled, None));
            }
        }
        record.managed = std::mem::take(&mut cx.managed);
        record.managed.sort();
        record.managed.dedup();
        record.finished = Some(Local::now());
        summary(&mut cx, &record);
        let _ = record.save(&dir);
        RunRecord::prune(
            &opts.paths.runs_dir(),
            opts.settings.runs_to_keep.max(1) as usize,
        );
        let _ = tx.send(Event::Finished(Box::new(record)));
    });
    handle
}

/// End-of-run recap: every heading and status line again with no command
/// output between them, then the counts. Goes to the log and the screen.
fn summary(cx: &mut Ctx, record: &RunRecord) {
    cx.log("\n==> Summary");
    cx.send(Event::Heading("Summary".into()));
    let mut group: Option<&str> = None;
    for step in &record.steps {
        let Some(outcome) = &step.outcome else {
            continue;
        };
        if group != Some(step.group.as_str()) {
            group = Some(&step.group);
            cx.log(&format!("\n{}", step.group));
            cx.send(Event::Line(String::new()));
            cx.send(Event::Line(step.group.clone()));
        }
        let line = status_line(&step.name, outcome);
        cx.log(&line);
        cx.send(Event::Status {
            text: line,
            status: outcome.status,
        });
    }
    cx.line("");
    if let Some(why) = &record.ended_early {
        cx.line(format!("Run ended early: {why}"));
    }
    cx.line(record.counts_line());
    if record.counts().failed > 0 {
        cx.line("Command output for each failure is in the log, directly above its FAILED line.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::plan::Step;

    #[test]
    fn runs_steps_and_records_them() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let plan = Plan {
            tasks: vec!["t".into()],
            steps: vec![
                Step::new("G", "one", |_| Outcome::success()),
                Step::new("G", "hidden", |_| Outcome::failed("x")).hidden(),
                Step::new("G", "two", |cx| {
                    cx.managed.push("pacman/x".into());
                    Outcome::failed("broke")
                })
                .required(),
                Step::new("H", "three", |_| Outcome::success()),
            ],
        };
        let handle = start_run(
            plan,
            RunOptions {
                dry: false,
                paths: paths.clone(),
                settings: Settings::default(),
            },
        );
        let record = loop {
            if let Event::Finished(r) = handle.events.recv().unwrap() {
                break r;
            }
        };
        let statuses: Vec<Status> = record
            .steps
            .iter()
            .map(|s| s.outcome.as_ref().unwrap().status)
            .collect();
        assert_eq!(
            statuses,
            [Status::Success, Status::Failed, Status::Cancelled]
        );
        assert_eq!(record.managed, ["pacman/x"]);
        assert!(record.ended_early.as_deref().unwrap().contains("two"));
        let log = std::fs::read_to_string(handle.dir.join(LOG_FILE)).unwrap();
        assert!(log.contains("==> G"));
        assert!(log.contains("two........"));
        assert!(log.contains("==> Summary"));
        assert_eq!(RunRecord::load(&handle.dir).unwrap(), *record);
    }
}
