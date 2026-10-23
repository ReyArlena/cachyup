//! Shared bootstrap for the TUI and the headless run: load every file,
//! reconcile the selection with the catalog and the system, and save changes.

use std::path::Path;

use anyhow::Result;

use crate::catalog::{App, Catalog};
use crate::config::Config;
use crate::engine::{Plan, RunRecord, build_plan};
use crate::paths::Paths;
use crate::selection::{Reconciled, Selection, reconcile};
use crate::state::State;
use crate::system::{Installed, has_cmd};

pub struct AppState {
    pub paths: Paths,
    pub config: Config,
    pub selection: Selection,
    pub catalog: Catalog,
    pub state: State,
    pub installed: Installed,
    /// What changed at this launch (new apps, removed apps).
    pub launch: Reconciled,
    pub first_launch: bool,
}

impl AppState {
    pub fn load(paths: Paths) -> Result<Self> {
        let config = Config::load(&paths.config_file())?;
        let mut selection = Selection::load(&paths.selection_file())?;
        let saved = State::load(&paths.state_file())?;
        let first_launch = saved.is_none();
        let catalog = Catalog::load(&paths.user_apps_file());
        let installed = Installed::probe();
        let (state, launch) = reconcile(
            &catalog,
            &mut selection,
            saved,
            &|app: &App| installed.app(app, &paths),
            &has_cmd,
        );
        let app = Self {
            paths,
            config,
            selection,
            catalog,
            state,
            installed,
            launch,
            first_launch,
        };
        app.save_selection()?;
        app.save_state()?;
        if first_launch || !app.paths.config_file().exists() {
            app.save_config()?;
        }
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

    pub fn save_selection(&self) -> Result<()> {
        self.selection.save(&self.paths.selection_file())
    }

    pub fn save_state(&self) -> Result<()> {
        self.state.save(&self.paths.state_file())
    }

    pub fn plan(&self, dry: bool) -> Plan {
        build_plan(&self.catalog, &self.config, &self.selection, dry)
    }

    /// Flip an app. A change by hand resets "managed": turning an app back on
    /// after removing it must not count as removed again at the next launch.
    pub fn set_app(&mut self, key: &str, on: bool) {
        self.selection.apps.insert(key.to_string(), on);
        self.state.managed.remove(key);
    }

    /// Flip a tweak. Turning one on also turns its app on when that app is
    /// not installed, or the tweak would be skipped on every run. Returns the
    /// app key it switched on, if any. An installed app is left alone: it may
    /// be off on purpose, and the tweak applies either way.
    pub fn set_tweak(&mut self, id: &str, on: bool) -> Option<String> {
        self.selection.tweaks.insert(id.to_string(), on);
        if !on {
            return None;
        }
        let key = self.catalog.tweak(id)?.app.clone()?;
        let app = self.catalog.app(&key)?;
        if self.selection.app_on(&key) || self.is_installed(app) == Some(true) {
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
            paths.user_apps_file(),
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
        assert!(app.selection.app_on("installer/missing"));

        assert_eq!(app.set_tweak("t-present", true), None);
        assert!(
            !app.selection.app_on("installer/present"),
            "installed app left alone"
        );

        assert_eq!(app.set_tweak("t-missing", false), None);
        assert!(
            app.selection.app_on("installer/missing"),
            "turning off leaves the app"
        );
    }
}
