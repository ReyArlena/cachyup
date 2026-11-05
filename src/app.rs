//! Shared bootstrap for the TUI and the headless run: load every file,
//! reconcile the selection with the catalog and the system, and save changes.

use std::path::Path;

use anyhow::{Context, Result};

use crate::catalog::{App, Catalog};
use crate::config::Config;
use crate::discover::{BuiltinCandidate, Candidate, builtin_candidates, installed_candidates};
use crate::engine::{Plan, RunRecord, build_plan};
use crate::paths::Paths;
use crate::selection::{Reconciled, Selection, reconcile};
use crate::state::State;
use crate::system::{Explicit, Installed, has_cmd};

pub struct AppState {
    pub paths: Paths,
    /// Settings and the on/off selection (`config.selection`).
    pub config: Config,
    pub catalog: Catalog,
    pub state: State,
    pub installed: Installed,
    /// Explicitly installed packages and flatpak apps, for the review.
    pub explicit: Explicit,
    /// What changed at this launch (new apps, removed apps).
    pub launch: Reconciled,
    pub first_launch: bool,
}

impl AppState {
    pub fn load(paths: Paths) -> Result<Self> {
        let mut config = Config::load(&paths.config_file())?;
        let mut notes = migrate_selection(&paths, &mut config)?;
        let saved = State::load(&paths.state_file())?;
        let first_launch = saved.is_none();
        notes.extend(Catalog::ensure_file(&paths.config_dir));
        let mut catalog = Catalog::load(&paths.apps_file());
        notes.append(&mut catalog.warnings);
        catalog.warnings = notes;
        let installed = Installed::probe();
        let explicit = Explicit::probe();
        let (state, launch) = reconcile(
            &catalog,
            &mut config.selection,
            saved,
            &|app: &App| installed.app(app, &paths),
            &has_cmd,
        );
        let app = Self {
            paths,
            config,
            catalog,
            state,
            installed,
            explicit,
            launch,
            first_launch,
        };
        app.save_config()?;
        app.save_state()?;
        Ok(app)
    }

    /// Everything in a scratch directory, for tests. Nothing is probed from
    /// the system except what is installed.
    pub fn sandboxed(base: &Path) -> Result<Self> {
        Self::load(Paths::under(base))
    }

    pub fn save_config(&self) -> Result<()> {
        self.config.save(&self.paths.config_file())
    }

    /// The selection lives in config.toml.
    pub fn save_selection(&self) -> Result<()> {
        self.save_config()
    }

    pub fn save_state(&self) -> Result<()> {
        self.state.save(&self.paths.state_file())
    }

    /// Installed apps the list lacks and the user has not reviewed.
    pub fn installed_candidates(&self) -> Vec<Candidate> {
        installed_candidates(&self.explicit, &self.catalog, &self.state)
    }

    /// Entries cachyup ships that the list lacks and the user has not dismissed.
    pub fn builtin_candidates(&self) -> Vec<BuiltinCandidate> {
        builtin_candidates(&Catalog::builtin(), &self.catalog, &self.state)
    }

    /// Add entries from `extra` (TOML in the apps.toml layout) whose keys
    /// `keep` accepts to the user's apps.toml, keeping its layout. The old
    /// file is kept as `apps.toml.bak`. The caller reloads afterwards.
    pub fn merge_into_apps(&self, extra: &str, keep: impl Fn(&str) -> bool) -> Result<()> {
        let file = self.paths.apps_file();
        let current = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        let merged = crate::apps_edit::append_where(&current, extra, keep)?;
        if merged != current {
            crate::paths::write_file(&file.with_extension("toml.bak"), &current)?;
            crate::paths::write_file(&file, &merged)
                .with_context(|| format!("writing {}", file.display()))?;
        }
        Ok(())
    }

    pub fn plan(&self, dry: bool) -> Plan {
        build_plan(&self.catalog, &self.config, dry)
    }

    /// Flip an app. A change by hand resets "managed": turning an app back on
    /// after removing it must not count as removed again at the next launch.
    pub fn set_app(&mut self, key: &str, on: bool) {
        self.config.selection.apps.insert(key.to_string(), on);
        self.state.managed.remove(key);
    }

    /// Flip a tweak. Turning one on also turns its app on when that app is
    /// not installed, or the tweak would be skipped on every run. Returns the
    /// app key it switched on, if any. An installed app is left alone: it may
    /// be off on purpose, and the tweak applies either way.
    pub fn set_tweak(&mut self, id: &str, on: bool) -> Option<String> {
        self.config.selection.tweaks.insert(id.to_string(), on);
        if !on {
            return None;
        }
        let key = self.catalog.tweak(id)?.app.clone()?;
        let app = self.catalog.app(&key)?;
        if self.config.selection.app_on(&key) || self.is_installed(app) == Some(true) {
            return None;
        }
        self.set_app(&key, true);
        Some(key)
    }

    /// After a real run: remember what is now installed, and re-read the system.
    pub fn record_run(&mut self, record: &RunRecord) -> Result<()> {
        if !record.dry_run {
            self.state.managed.extend(record.managed.iter().cloned());
            self.save_state()?;
        }
        self.installed = Installed::probe();
        Ok(())
    }

    pub fn is_installed(&self, app: &App) -> Option<bool> {
        self.installed.app(app, &self.paths)
    }

    /// Remove every config, state and cache file (`--reset`).
    pub fn reset(paths: &Paths) -> Result<()> {
        for dir in [&paths.config_dir, &paths.state_dir, &paths.cache_dir] {
            if dir.exists() {
                std::fs::remove_dir_all(dir)?;
            }
        }
        Ok(())
    }
}

/// Fold a pre-0.2.0 `selection.toml` into config.toml, once. The old file
/// is renamed so it is not read again. Returns a note for the user.
fn migrate_selection(paths: &Paths, config: &mut Config) -> Result<Vec<String>> {
    let legacy = paths.legacy_selection_file();
    if !legacy.exists() {
        return Ok(Vec::new());
    }
    if config.selection.apps.is_empty() && config.selection.tweaks.is_empty() {
        config.selection = Selection::load_legacy(&legacy)?;
    }
    let moved = legacy.with_extension("toml.migrated");
    std::fs::rename(&legacy, &moved).with_context(|| format!("renaming {}", legacy.display()))?;
    Ok(vec![
        "selection.toml moved into config.toml ([apps] and [tweaks]) and renamed to selection.toml.migrated".into(),
    ])
}

pub fn is_root() -> bool {
    nix::unistd::Uid::effective().is_root()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_a_tweak_on_turns_on_its_missing_app() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::create_dir_all(paths.home.join("bin")).unwrap();
        std::fs::write(paths.home.join("bin/present"), "").unwrap();
        std::fs::write(
            paths.apps_file(),
            r#"
[[installer]]
name = "missing"
check = "~/bin/missing"
install = "true"

[[installer]]
name = "present"
check = "~/bin/present"
install = "true"

[[tweak]]
id = "t-missing"
app = "installer/missing"
name = "t"
default = false
set = [{ json = "~/a.json", key = "k", value = 1 }]

[[tweak]]
id = "t-present"
app = "installer/present"
name = "t"
default = false
set = [{ json = "~/a.json", key = "k", value = 1 }]
"#,
        )
        .unwrap();
        let mut app = AppState::load(paths).unwrap();
        app.set_app("installer/missing", false);
        app.set_app("installer/present", false);

        assert_eq!(
            app.set_tweak("t-missing", true).as_deref(),
            Some("installer/missing")
        );
        assert!(app.config.selection.app_on("installer/missing"));

        assert_eq!(app.set_tweak("t-present", true), None);
        assert!(
            !app.config.selection.app_on("installer/present"),
            "installed app left alone"
        );

        assert_eq!(app.set_tweak("t-missing", false), None);
        assert!(
            app.config.selection.app_on("installer/missing"),
            "turning off leaves the app"
        );
    }
}
