//! `selection.toml`: on/off for every app and tweak, plus the launch-time
//! reconciliation that keeps it in step with the catalog and the system.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::catalog::{App, Catalog, Origin, Tweak};
use crate::paths::write_file;
use crate::state::State;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Selection {
    /// App key (`pacman/discord`) -> on/off.
    pub apps: BTreeMap<String, bool>,
    /// Tweak id -> on/off.
    pub tweaks: BTreeMap<String, bool>,
}

const HEADER: &str = "\
# Which apps and tweaks cachyup installs. Toggle them in the app, or here.
# Turning an app off never uninstalls it.

";

impl Selection {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("{} is not valid", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self)?;
        write_file(path, &format!("{HEADER}{body}"))
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn app_on(&self, key: &str) -> bool {
        self.apps.get(key).copied().unwrap_or(false)
    }

    pub fn tweak_on(&self, id: &str) -> bool {
        self.tweaks.get(id).copied().unwrap_or(false)
    }
}

/// What changed at launch, for the startup popups.
#[derive(Debug, Default, PartialEq)]
pub struct Reconciled {
    /// Built-in apps added since the last launch (off, shown with a badge).
    pub new_apps: Vec<String>,
    pub new_tweaks: Vec<String>,
    /// Apps cachyup had installed that are gone now; switched off.
    pub removed: Vec<String>,
}

/// The default for an entry seen for the first time.
pub fn app_default(app: &App, has_cmd: &dyn Fn(&str) -> bool) -> bool {
    app.default.unwrap_or(true) && !app.unless_cmd.as_deref().is_some_and(has_cmd)
}

pub fn tweak_default(tweak: &Tweak, has_cmd: &dyn Fn(&str) -> bool) -> bool {
    tweak.default.unwrap_or(true) && !tweak.unless_cmd.as_deref().is_some_and(has_cmd)
}

/// Bring the selection and state up to date with the catalog and the system.
///
/// - First launch (`state` is `None`): everything gets its default, no badges.
/// - Later launches: new built-in entries start off and are reported as new;
///   new entries from the user's own file start with their default (on).
/// - Entries no longer in the catalog are forgotten, so adding one back later
///   counts as new again.
/// - A managed app that is selected but no longer installed was removed by
///   hand: it is switched off and reported.
///
/// `installed` returns `None` when it cannot tell (the probe failed).
pub fn reconcile(
    catalog: &Catalog,
    selection: &mut Selection,
    state: Option<State>,
    installed: &dyn Fn(&App) -> Option<bool>,
    has_cmd: &dyn Fn(&str) -> bool,
) -> (State, Reconciled) {
    let first_launch = state.is_none();
    let mut state = state.unwrap_or_default();
    let mut out = Reconciled::default();

    let app_keys: Vec<String> = catalog.apps.iter().map(App::key).collect();
    selection.apps.retain(|k, _| app_keys.contains(k));
    state.seen_apps.retain(|k| app_keys.contains(k));
    state.managed.retain(|k| app_keys.contains(k));
    for app in &catalog.apps {
        let key = app.key();
        if !state.seen_apps.contains(&key) && !selection.apps.contains_key(&key) {
            let on = if first_launch || app.origin == Origin::User {
                app_default(app, has_cmd)
            } else {
                out.new_apps.push(key.clone());
                false
            };
            selection.apps.insert(key.clone(), on);
        }
        state.seen_apps.insert(key);
    }

    let tweak_ids: Vec<&str> = catalog.tweaks.iter().map(|t| t.id.as_str()).collect();
    selection
        .tweaks
        .retain(|k, _| tweak_ids.contains(&k.as_str()));
    state
        .seen_tweaks
        .retain(|k| tweak_ids.contains(&k.as_str()));
    for tweak in &catalog.tweaks {
        if !state.seen_tweaks.contains(&tweak.id) && !selection.tweaks.contains_key(&tweak.id) {
            let on = if first_launch || tweak.origin == Origin::User {
                tweak_default(tweak, has_cmd)
            } else {
                out.new_tweaks.push(tweak.id.clone());
                false
            };
            selection.tweaks.insert(tweak.id.clone(), on);
        }
        state.seen_tweaks.insert(tweak.id.clone());
    }

    let managed: Vec<String> = state.managed.iter().cloned().collect();
    for key in managed {
        let Some(app) = catalog.app(&key) else {
            continue;
        };
        if selection.app_on(&key) && installed(app) == Some(false) {
            selection.apps.insert(key.clone(), false);
            state.managed.remove(&key);
            out.removed.push(key);
        }
    }

    state.version = Some(env!("CARGO_PKG_VERSION").to_string());
    (state, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(user: &str) -> Catalog {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("user-apps.toml");
        std::fs::write(&file, user).unwrap();
        Catalog::load(&file)
    }

    #[test]
    fn first_launch_uses_defaults_and_conditions() {
        let c = Catalog::builtin();
        let mut sel = Selection::default();
        let dms = |cmd: &str| cmd == "dms";
        let (state, out) = reconcile(&c, &mut sel, None, &|_| Some(true), &dms);
        assert_eq!(out, Reconciled::default());
        assert!(sel.app_on("pacman/discord"));
        assert!(!sel.app_on("pacman/junction"), "DMS present: junction off");
        assert!(!sel.tweak_on("junction-default-browser"));
        assert!(sel.tweak_on("discord-skip-host-update"));
        assert_eq!(state.seen_apps.len(), c.apps.len());

        let mut sel = Selection::default();
        reconcile(&c, &mut sel, None, &|_| Some(true), &|_| false);
        assert!(sel.app_on("pacman/junction"), "no DMS: junction on");
    }

    #[test]
    fn new_builtin_entries_start_off_user_entries_on() {
        let c = catalog("pacman = [\"htop\"]\n");
        let mut sel = Selection::default();
        let mut state = State::default();
        for app in &c.apps {
            if app.name != "htop" && app.name != "discord" {
                state.seen_apps.insert(app.key());
            }
        }
        state.seen_tweaks = c.tweaks.iter().map(|t| t.id.clone()).collect();
        let (state, out) = reconcile(&c, &mut sel, Some(state), &|_| Some(true), &|_| false);
        assert_eq!(out.new_apps, vec!["pacman/discord".to_string()]);
        assert!(!sel.app_on("pacman/discord"));
        assert!(sel.app_on("pacman/htop"));
        assert!(state.seen_apps.contains("pacman/discord"));
    }

    #[test]
    fn removed_managed_apps_are_switched_off() {
        let c = Catalog::builtin();
        let mut sel = Selection::default();
        let (mut state, _) = reconcile(&c, &mut sel, None, &|_| Some(true), &|_| false);
        state.managed.insert("pacman/discord".into());
        state.managed.insert("pacman/ark".into());
        state.managed.insert("pacman/gone-from-catalog".into());
        let installed = |a: &App| Some(a.name != "discord");
        let (state, out) = reconcile(&c, &mut sel, Some(state), &installed, &|_| false);
        assert_eq!(out.removed, vec!["pacman/discord".to_string()]);
        assert!(!sel.app_on("pacman/discord"));
        assert!(sel.app_on("pacman/ark"));
        assert!(!state.managed.contains("pacman/discord"));
        assert!(!state.managed.contains("pacman/gone-from-catalog"));
    }

    #[test]
    fn selection_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("selection.toml");
        let mut sel = Selection::default();
        sel.apps.insert("pacman/discord".into(), true);
        sel.apps.insert("flatpak/org.kde.ktouch".into(), false);
        sel.tweaks.insert("x".into(), true);
        sel.save(&path).unwrap();
        assert_eq!(Selection::load(&path).unwrap(), sel);
    }
}
