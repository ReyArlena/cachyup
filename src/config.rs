//! `config.toml`: which tasks run, which update steps run, and the numbers
//! the cleanup uses. Missing keys fall back to defaults, so old files keep working.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::write_file;

/// One toggleable step of the Full System Update task.
pub struct UpdateStep {
    pub id: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    /// Another step this one cannot run without.
    pub needs: Option<&'static str>,
}

/// Display order of the update options, and their defaults (all on).
pub const UPDATE_STEPS: &[UpdateStep] = &[
    UpdateStep {
        id: "rate_mirrors",
        label: "Rate mirrors",
        help: "cachyos-rate-mirrors, at most once per cooldown period",
        needs: None,
    },
    UpdateStep {
        id: "pacman",
        label: "pacman -Syu",
        help: "Full upgrade of repo packages",
        needs: None,
    },
    UpdateStep {
        id: "keyrings",
        label: "Keyrings",
        help: "Refresh archlinux-keyring and cachyos-keyring before the upgrade, so signatures verify on a machine that has not updated in a while",
        needs: Some("pacman"),
    },
    UpdateStep {
        id: "aur",
        label: "yay -Sua",
        help: "Upgrade AUR packages",
        needs: None,
    },
    UpdateStep {
        id: "flatpak",
        label: "flatpak update",
        help: "Upgrade flatpaks and runtimes",
        needs: None,
    },
    UpdateStep {
        id: "orphans",
        label: "Remove orphans",
        help: "pacman -Rns packages nothing depends on any more",
        needs: None,
    },
    UpdateStep {
        id: "package_cache",
        label: "Prune package cache",
        help: "paccache: keep N versions of installed packages, none of uninstalled ones",
        needs: None,
    },
    UpdateStep {
        id: "yay_cache",
        label: "Prune yay build cache",
        help: "Remove AUR build dirs untouched for N days, and broken clones",
        needs: None,
    },
    UpdateStep {
        id: "flatpak_unused",
        label: "Remove unused flatpak runtimes",
        help: "flatpak uninstall --unused",
        needs: None,
    },
    UpdateStep {
        id: "mirrorlist_pacnew",
        label: "Drop rated mirrorlist .pacnew",
        help: "A .pacnew for a mirrorlist that cachyos-rate-mirrors generates is just the stock list; nothing to merge",
        needs: None,
    },
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub tasks: Tasks,
    /// Update step id -> on/off. Unknown ids are kept but ignored.
    pub update: BTreeMap<String, bool>,
    pub settings: Settings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tasks {
    pub update: bool,
    pub apps: bool,
    pub tweaks: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub rate_mirrors_cooldown_days: u32,
    pub package_cache_keep: u32,
    pub yay_cache_keep_days: u32,
    pub runs_to_keep: u32,
    /// Check GitHub for a newer release at launch.
    pub update_check: bool,
    /// A description line under every app and tweak (`I` in the app).
    pub show_descriptions: bool,
}

impl Default for Tasks {
    fn default() -> Self {
        Self {
            update: true,
            apps: true,
            tweaks: true,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            rate_mirrors_cooldown_days: 7,
            package_cache_keep: 2,
            yay_cache_keep_days: 30,
            runs_to_keep: 30,
            update_check: true,
            show_descriptions: false,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tasks: Tasks::default(),
            update: UPDATE_STEPS
                .iter()
                .map(|s| (s.id.to_string(), true))
                .collect(),
            settings: Settings::default(),
        }
    }
}

/// One editable number in the update options.
pub struct NumberSetting {
    pub label: &'static str,
    pub help: &'static str,
    pub get: fn(&Settings) -> u32,
    pub set: fn(&mut Settings, u32),
    pub min: u32,
    pub max: u32,
}

pub const NUMBER_SETTINGS: &[NumberSetting] = &[
    NumberSetting {
        label: "Mirror rating cooldown (days)",
        help: "Skip rating if the mirrorlist is newer than this. 0 rates every run",
        get: |s| s.rate_mirrors_cooldown_days,
        set: |s, v| s.rate_mirrors_cooldown_days = v,
        min: 0,
        max: 365,
    },
    NumberSetting {
        label: "Package cache versions to keep",
        help: "paccache -rk N for installed packages",
        get: |s| s.package_cache_keep,
        set: |s, v| s.package_cache_keep = v,
        min: 0,
        max: 20,
    },
    NumberSetting {
        label: "yay cache age to keep (days)",
        help: "Build dirs untouched for longer are removed",
        get: |s| s.yay_cache_keep_days,
        set: |s, v| s.yay_cache_keep_days = v,
        min: 1,
        max: 3650,
    },
    NumberSetting {
        label: "Runs kept in history",
        help: "Older run logs are deleted after each run",
        get: |s| s.runs_to_keep,
        set: |s, v| s.runs_to_keep = v,
        min: 1,
        max: 1000,
    },
];

const HEADER: &str = "\
# cachyup settings. Change them in the app (c on a task) or here.
# Delete a line to get its default back.

";

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut config: Config =
            toml::from_str(&text).with_context(|| format!("{} is not valid", path.display()))?;
        // Steps added in a newer version start with their default.
        for step in UPDATE_STEPS {
            config.update.entry(step.id.to_string()).or_insert(true);
        }
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self)?;
        write_file(path, &format!("{HEADER}{body}"))
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Whether an update step is set to on (ignoring dependencies).
    pub fn step_on(&self, id: &str) -> bool {
        self.update.get(id).copied().unwrap_or(true)
    }

    /// Whether an update step will actually run: on, and whatever it needs is on.
    pub fn step_enabled(&self, id: &str) -> bool {
        let Some(step) = UPDATE_STEPS.iter().find(|s| s.id == id) else {
            return false;
        };
        self.step_on(id) && step.needs.is_none_or(|n| self.step_enabled(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_fills_new_steps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::default();
        c.tasks.tweaks = false;
        c.update.insert("orphans".into(), false);
        c.settings.yay_cache_keep_days = 10;
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), c);

        std::fs::write(&path, "[update]\norphans = false\n").unwrap();
        let c = Config::load(&path).unwrap();
        assert!(!c.step_on("orphans"));
        assert!(c.step_on("pacman"));
        assert!(c.tasks.update);
    }

    #[test]
    fn keyrings_follow_pacman() {
        let mut c = Config::default();
        assert!(c.step_enabled("keyrings"));
        c.update.insert("pacman".into(), false);
        assert!(c.step_on("keyrings"));
        assert!(!c.step_enabled("keyrings"));
    }
}
