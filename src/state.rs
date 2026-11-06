//! `state.toml`: what cachyup has seen and done, as opposed to what the user
//! chose (that is `config.toml`). Never edited by hand.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::write_file;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// cachyup version of the last launch.
    pub version: Option<String>,
    /// Installed apps (`source/name`) the user has looked at in a review,
    /// added or not. Only what is installed since then counts as new.
    pub reviewed: BTreeSet<String>,
    /// Built-in entries (`source/name`, `tweak/id`) the user chose not to
    /// add. Not offered again.
    pub dismissed: BTreeSet<String>,
    /// Apps cachyup installed, or found installed while selected at a
    /// launch or a run. If one of these goes missing it was removed on
    /// purpose.
    pub managed: BTreeSet<String>,
}

impl State {
    /// `None` when there is no state file yet (first launch).
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let state =
            toml::from_str(&text).with_context(|| format!("{} is not valid", path.display()))?;
        Ok(Some(state))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self)?;
        write_file(
            path,
            &format!("# Written by cachyup. Not meant to be edited.\n\n{body}"),
        )
        .with_context(|| format!("writing {}", path.display()))
    }
}
