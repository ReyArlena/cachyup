//! On/off for every app and tweak (the `[apps]` and `[tweaks]` tables of
//! config.toml), plus the launch-time reconciliation that keeps it in step
//! with the app list and the system.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::catalog::{App, Catalog, Tweak};
use crate::state::State;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Selection {
    /// App key (`pacman/discord`) -> on/off.
    pub apps: BTreeMap<String, bool>,
    /// Tweak id -> on/off.
    pub tweaks: BTreeMap<String, bool>,
}

impl Selection {
    /// Read a `selection.toml` from before 0.2.0, to fold into config.toml.
    pub fn load_legacy(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("{} is not valid", path.display()))
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
    /// Built-in entries that are not in the user's apps.toml, first noticed
    /// at this launch. Keys and tweak ids of the built-in list.
    pub new_apps: Vec<String>,
    pub new_tweaks: Vec<String>,
    /// Apps cachyup had installed that are gone now; switched off.
    pub removed: Vec<String>,
}

/// The default for an entry seen for the first time: off, unless the entry
/// says `default = true` and its `unless_cmd` command is absent.
pub fn app_default(app: &App, has_cmd: &dyn Fn(&str) -> bool) -> bool {
    app.default.unwrap_or(false) && !app.unless_cmd.as_deref().is_some_and(has_cmd)
}

pub fn tweak_default(tweak: &Tweak, has_cmd: &dyn Fn(&str) -> bool) -> bool {
    tweak.default.unwrap_or(false) && !tweak.unless_cmd.as_deref().is_some_and(has_cmd)
}

/// Bring the selection and state up to date with the app list and the system.
///
/// - Entries in the list but not in the selection get their default.
/// - Entries no longer in the list are forgotten.
/// - Built-in entries missing from the user's list are reported once, so a
///   cachyup update can say what it added.
/// - A managed app that is selected but no longer installed was removed by
///   hand: it is switched off and reported.
///
/// `installed` returns `None` when it cannot tell (the probe failed).
pub fn reconcile(
    catalog: &Catalog,
    builtin: &Catalog,
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
    state.managed.retain(|k| app_keys.contains(k));
    for app in &catalog.apps {
        selection
            .apps
            .entry(app.key())
            .or_insert_with(|| app_default(app, has_cmd));
    }

    let tweak_ids: Vec<&str> = catalog.tweaks.iter().map(|t| t.id.as_str()).collect();
    selection
        .tweaks
        .retain(|k, _| tweak_ids.contains(&k.as_str()));
    for tweak in &catalog.tweaks {
        selection
            .tweaks
            .entry(tweak.id.clone())
            .or_insert_with(|| tweak_default(tweak, has_cmd));
    }

    // Built-in entries the user's list lacks: announce each once. On the
    // first launch the list is a fresh copy, so there is nothing to say.
    let builtin_keys: Vec<String> = builtin.apps.iter().map(App::key).collect();
    state.seen_apps.retain(|k| builtin_keys.contains(k));
    for key in builtin_keys {
        if state.seen_apps.insert(key.clone()) && !first_launch && !app_keys.contains(&key) {
            out.new_apps.push(key);
        }
    }
    let builtin_tweaks: Vec<String> = builtin.tweaks.iter().map(|t| t.id.clone()).collect();
    state.seen_tweaks.retain(|k| builtin_tweaks.contains(k));
    for id in builtin_tweaks {
        if state.seen_tweaks.insert(id.clone())
            && !first_launch
            && !tweak_ids.contains(&id.as_str())
        {
            out.new_tweaks.push(id);
        }
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
        let file = dir.path().join("apps.toml");
        std::fs::write(&file, user).unwrap();
        Catalog::load(&file)
    }

    #[test]
    fn first_launch_everything_off_unless_the_entry_opts_in() {
        let c = catalog(
            "pacman = [\"htop\", { name = \"junction\", default = true, unless_cmd = \"dms\" }]\n",
        );
        let b = Catalog::builtin();
        let mut sel = Selection::default();
        let dms = |cmd: &str| cmd == "dms";
        let (state, out) = reconcile(&c, &b, &mut sel, None, &|_| Some(true), &dms);
        assert_eq!(
            out,
            Reconciled::default(),
            "first launch: nothing to report"
        );
        assert!(!sel.app_on("pacman/htop"));
        assert!(!sel.app_on("pacman/junction"), "DMS present: junction off");
        assert_eq!(sel.apps.len(), 2);
        assert_eq!(state.seen_apps.len(), b.apps.len());

        let mut sel = Selection::default();
        reconcile(&c, &b, &mut sel, None, &|_| Some(true), &|_| false);
        assert!(
            sel.app_on("pacman/junction"),
            "no DMS: default = true applies"
        );
    }

    #[test]
    fn builtin_entries_missing_from_the_list_are_reported_once() {
        let c = catalog("pacman = [\"htop\"]\n");
        let b = Catalog::builtin();
        let mut sel = Selection::default();
        let mut state = State::default();
        for app in &b.apps {
            if app.name != "discord" {
                state.seen_apps.insert(app.key());
            }
        }
        state.seen_tweaks = b.tweaks.iter().map(|t| t.id.clone()).collect();
        let (state, out) = reconcile(&c, &b, &mut sel, Some(state), &|_| Some(true), &|_| false);
        assert_eq!(out.new_apps, vec!["pacman/discord".to_string()]);
        assert!(out.new_tweaks.is_empty());
        assert!(sel.apps.contains_key("pacman/htop"));
        assert!(!sel.apps.contains_key("pacman/discord"), "not in the list");
        assert!(state.seen_apps.contains("pacman/discord"));
        let (_, out) = reconcile(&c, &b, &mut sel, Some(state), &|_| Some(true), &|_| false);
        assert!(out.new_apps.is_empty(), "announced once");
    }

    #[test]
    fn removed_managed_apps_are_switched_off() {
        let c = Catalog::builtin();
        let mut sel = Selection::default();
        let (mut state, _) = reconcile(&c, &c, &mut sel, None, &|_| Some(true), &|_| false);
        sel.apps.insert("pacman/discord".into(), true);
        sel.apps.insert("pacman/ark".into(), true);
        state.managed.insert("pacman/discord".into());
        state.managed.insert("pacman/ark".into());
        state.managed.insert("pacman/gone-from-catalog".into());
        let installed = |a: &App| Some(a.name != "discord");
        let (state, out) = reconcile(&c, &c, &mut sel, Some(state), &installed, &|_| false);
        assert_eq!(out.removed, vec!["pacman/discord".to_string()]);
        assert!(!sel.app_on("pacman/discord"));
        assert!(sel.app_on("pacman/ark"));
        assert!(!state.managed.contains("pacman/discord"));
        assert!(!state.managed.contains("pacman/gone-from-catalog"));
    }

    #[test]
    fn legacy_selection_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("selection.toml");
        std::fs::write(
            &path,
            "# old header\n[apps]\n\"pacman/discord\" = true\n[tweaks]\nx = false\n",
        )
        .unwrap();
        let sel = Selection::load_legacy(&path).unwrap();
        assert!(sel.app_on("pacman/discord"));
        assert_eq!(sel.tweaks.get("x"), Some(&false));
    }
}
