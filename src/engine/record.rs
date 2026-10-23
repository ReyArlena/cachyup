//! `run.json`: the summary of one run, rewritten after every step so an
//! interrupted run still leaves a readable record. Its directory also holds
//! `output.log`, the full terminal dump.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use super::{Outcome, Status};

pub const RECORD_FILE: &str = "run.json";
pub const LOG_FILE: &str = "output.log";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRecord {
    pub group: String,
    pub name: String,
    /// `None` until the step has run.
    pub outcome: Option<Outcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub version: String,
    pub started: DateTime<Local>,
    pub finished: Option<DateTime<Local>>,
    pub dry_run: bool,
    /// Task labels that were switched on.
    pub tasks: Vec<String>,
    pub steps: Vec<StepRecord>,
    /// Set when the run ended early: why.
    pub ended_early: Option<String>,
    /// App keys found installed or installed by this run (for state.managed).
    #[serde(default)]
    pub managed: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub succeeded: usize,
    pub skipped: usize,
    pub failed: usize,
    pub attention: usize,
    pub would: usize,
}

impl RunRecord {
    pub fn new_id(at: DateTime<Local>) -> String {
        at.format("%Y-%m-%d_%H-%M-%S").to_string()
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for s in self.steps.iter().filter_map(|s| s.outcome.as_ref()) {
            match s.status {
                Status::Success => c.succeeded += 1,
                Status::Skipped => c.skipped += 1,
                Status::Failed => c.failed += 1,
                Status::Attention => c.attention += 1,
                Status::Would => c.would += 1,
                Status::Pass | Status::Cancelled => {}
            }
        }
        c
    }

    /// "3 succeeded, 2 skipped, 1 failed" (dry runs say "would run").
    pub fn counts_line(&self) -> String {
        let c = self.counts();
        let mut parts = Vec::new();
        if self.dry_run {
            parts.push(format!("{} would run", c.would));
        } else {
            parts.push(format!("{} succeeded", c.succeeded));
        }
        parts.push(format!("{} skipped", c.skipped));
        parts.push(format!("{} failed", c.failed));
        if c.attention > 0 {
            parts.push(format!("{} need attention", c.attention));
        }
        parts.join(", ")
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        let text = serde_json::to_string_pretty(self)?;
        let tmp = dir.join(format!("{RECORD_FILE}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, dir.join(RECORD_FILE))?;
        Ok(())
    }

    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(RECORD_FILE);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("{} is not valid", path.display()))
    }

    /// Every run under `runs_dir`, newest first. Unreadable ones are skipped.
    pub fn list(runs_dir: &Path) -> Vec<(PathBuf, RunRecord)> {
        let Ok(entries) = std::fs::read_dir(runs_dir) else {
            return Vec::new();
        };
        let mut runs: Vec<(PathBuf, RunRecord)> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .filter_map(|p| Self::load(&p).ok().map(|r| (p, r)))
            .collect();
        runs.sort_by_key(|r| std::cmp::Reverse(r.1.started));
        runs
    }

    /// Delete all but the newest `keep` runs.
    pub fn prune(runs_dir: &Path, keep: usize) {
        for (dir, _) in Self::list(runs_dir).into_iter().skip(keep) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, minutes: i64) -> RunRecord {
        RunRecord {
            id: id.into(),
            version: "0".into(),
            started: Local::now() + chrono::Duration::minutes(minutes),
            finished: None,
            dry_run: false,
            tasks: vec![],
            steps: vec![
                StepRecord {
                    group: "g".into(),
                    name: "a".into(),
                    outcome: Some(Outcome::success()),
                },
                StepRecord {
                    group: "g".into(),
                    name: "b".into(),
                    outcome: Some(Outcome::failed("x")),
                },
                StepRecord {
                    group: "g".into(),
                    name: "c".into(),
                    outcome: None,
                },
            ],
            ended_early: None,
            managed: vec![],
        }
    }

    #[test]
    fn saves_lists_and_prunes() {
        let dir = tempfile::tempdir().unwrap();
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            record(id, i as i64).save(&dir.path().join(id)).unwrap();
        }
        let ids: Vec<String> = RunRecord::list(dir.path())
            .into_iter()
            .map(|(_, r)| r.id)
            .collect();
        assert_eq!(ids, ["c", "b", "a"]);
        RunRecord::prune(dir.path(), 2);
        assert_eq!(RunRecord::list(dir.path()).len(), 2);
        assert!(!dir.path().join("a").exists());
    }

    #[test]
    fn counts_ignore_unrun_steps() {
        let r = record("x", 0);
        assert_eq!(r.counts_line(), "1 succeeded, 0 skipped, 1 failed");
    }
}
