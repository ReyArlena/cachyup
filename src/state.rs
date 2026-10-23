//! `state.toml`: what cachyup has seen and done, as opposed to what the user
//! chose (that is `selection.toml`). Never edited by hand.

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
    /// App keys known at the last launch. Anything else is new.
    pub seen_apps: BTreeSet<String>,
    pub seen_tweaks: BTreeSet<String>,
    /// Apps cachyup installed, or found installed while selected, at least
    /// once. If one of these goes missing it was removed on purpose.
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
