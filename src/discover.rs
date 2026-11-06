//! What could be added to the user's app list: apps installed on this
//! machine that the list lacks, and entries cachyup ships that the list
//! lacks. Nothing here changes anything; the review screen decides.

use std::collections::HashSet;

use crate::catalog::{App, Catalog, Source};
use crate::state::State;
use crate::system::Explicit;

/// An installed package or flatpak app that is not in the list.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub source: Source,
    pub name: String,
    /// Set when a rule marks it as machine-specific (kernel, driver,
    /// firmware, base system). Hidden by default in the review, never dropped.
    pub hidden: Option<&'static str>,
}

impl Candidate {
    pub fn key(&self) -> String {
        format!("{}/{}", self.source.id(), self.name)
    }
}

/// An entry cachyup ships that the user's list lacks.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltinCandidate {
    /// `source/name` for an app, `tweak/id` for a tweak.
    pub key: String,
    pub label: String,
    pub is_tweak: bool,
}

/// Why a package is set aside, from its name. Kernels come from the files
/// they own (`Explicit::kernels`), everything else from a name pattern.
pub fn system_rule(name: &str, kernels: &HashSet<String>) -> Option<&'static str> {
    if kernels.contains(name) {
        return Some("kernel");
    }
    let starts = |p: &str| name.starts_with(p);
    let ends = |p: &str| name.ends_with(p);
    let eq = |p: &str| name == p;
    if starts("linux-firmware") || ends("-firmware") || ends("-ucode") || eq("sof-firmware") {
        return Some("firmware");
    }
    if starts("nvidia")
        || starts("lib32-nvidia")
        || ends("-dkms")
        || starts("xf86-video-")
        || starts("xf86-input-")
        || starts("vulkan-")
        || starts("lib32-vulkan-")
        || starts("mesa")
        || starts("lib32-mesa")
        || starts("libva-")
        || eq("intel-media-driver")
        || starts("broadcom-")
        || eq("r8168")
    {
        return Some("driver");
    }
    if eq("base")
        || eq("base-devel")
        || starts("cachyos-")
        || ends("-keyring")
        || ends("-mirrorlist")
        || ends("-settings")
        || ends("-hooks")
    {
        return Some("base system");
    }
    None
}

/// Installed apps the list lacks, not yet reviewed, with system packages
/// marked hidden. Prerequisites and anything in the list are left out.
pub fn installed_candidates(
    explicit: &Explicit,
    catalog: &Catalog,
    state: &State,
) -> Vec<Candidate> {
    // Apps, plus the packages tweaks install: those are in the list too.
    let listed: HashSet<String> = catalog
        .apps
        .iter()
        .map(App::key)
        .chain(catalog.tweaks.iter().flat_map(|t| {
            t.installs
                .iter()
                .map(|p| format!("{}/{}", p.source.id(), p.name))
        }))
        .collect();
    let mut out = Vec::new();
    for (source, names) in [
        (Source::Pacman, &explicit.pacman),
        (Source::Aur, &explicit.aur),
        (Source::Flatpak, &explicit.flatpak),
    ] {
        for name in names {
            let key = format!("{}/{name}", source.id());
            if listed.contains(&key)
                || state.reviewed.contains(&key)
                || catalog.prerequisites.contains(name)
            {
                continue;
            }
            let hidden = match source {
                Source::Flatpak => None,
                _ => system_rule(name, &explicit.kernels),
            };
            out.push(Candidate {
                source,
                name: name.clone(),
                hidden,
            });
        }
    }
    out
}

/// Built-in entries the list lacks and the user has not dismissed. A tweak
/// is offered only once its app is in the list, so merging it never leaves
/// a dangling `app`.
pub fn builtin_candidates(
    builtin: &Catalog,
    catalog: &Catalog,
    state: &State,
) -> Vec<BuiltinCandidate> {
    let mut out = Vec::new();
    for app in &builtin.apps {
        let key = app.key();
        if catalog.app(&key).is_none() && !state.dismissed.contains(&key) {
            out.push(BuiltinCandidate {
                label: format!("{} ({})", app.display_name(), app.source),
                key,
                is_tweak: false,
            });
        }
    }
    for tweak in &builtin.tweaks {
        let key = format!("tweak/{}", tweak.id);
        let app_listed = tweak
            .app
            .as_deref()
            .is_none_or(|a| catalog.app(a).is_some());
        if catalog.tweak(&tweak.id).is_none() && app_listed && !state.dismissed.contains(&key) {
            let for_app = tweak
                .app
                .as_deref()
                .and_then(|a| builtin.app(a))
                .map(|a| format!(" for {}", a.display_name()))
                .unwrap_or_default();
            out.push(BuiltinCandidate {
                label: format!("tweak: {}{for_app}", tweak.name),
                key,
                is_tweak: true,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_set_aside_machine_specific_packages() {
        let kernels: HashSet<String> = ["linux-cachyos", "linux-cachyos-headers"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(system_rule("linux-cachyos", &kernels), Some("kernel"));
        assert_eq!(system_rule("linux-firmware", &kernels), Some("firmware"));
        assert_eq!(system_rule("intel-ucode", &kernels), Some("firmware"));
        assert_eq!(system_rule("nvidia-open-dkms", &kernels), Some("driver"));
        assert_eq!(system_rule("vulkan-radeon", &kernels), Some("driver"));
        assert_eq!(
            system_rule("cachyos-settings", &kernels),
            Some("base system")
        );
        assert_eq!(system_rule("base-devel", &kernels), Some("base system"));
        assert_eq!(system_rule("htop", &kernels), None);
        assert_eq!(system_rule("discord", &kernels), None);
    }

    #[test]
    fn candidates_skip_listed_reviewed_and_prerequisites() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("apps.toml");
        std::fs::write(
            &file,
            "pacman = [\"htop\"]\nflatpak = [\"org.gnome.Chess\"]\n\n[[tweak]]\nid = \"t\"\napp = \"pacman/htop\"\nname = \"T\"\ninstalls = [\"yay\", \"aur/extra\"]\n",
        )
        .unwrap();
        let catalog = Catalog::load(&file);
        let mut state = State::default();
        state.reviewed.insert("pacman/btop".into());
        let explicit = Explicit {
            pacman: ["htop", "btop", "yay", "discord", "linux-cachyos"]
                .into_iter()
                .map(String::from)
                .collect(),
            aur: vec!["spotify".into(), "extra".into()],
            flatpak: vec!["org.gnome.Chess".into(), "com.example.App".into()],
            kernels: ["linux-cachyos".to_string()].into_iter().collect(),
        };
        let c = installed_candidates(&explicit, &catalog, &state);
        let keys: Vec<String> = c.iter().map(Candidate::key).collect();
        assert_eq!(
            keys,
            [
                "pacman/discord",
                "pacman/linux-cachyos",
                "aur/spotify",
                "flatpak/com.example.App"
            ]
        );
        assert_eq!(c[1].hidden, Some("kernel"));
        assert_eq!(c[0].hidden, None);
    }

    #[test]
    fn builtin_candidates_follow_the_list_and_dismissals() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("apps.toml");
        std::fs::write(&file, "pacman = [\"discord\"]\n").unwrap();
        let catalog = Catalog::load(&file);
        let builtin = Catalog::builtin();
        let mut state = State::default();
        state.dismissed.insert("pacman/ark".into());
        let c = builtin_candidates(&builtin, &catalog, &state);
        let keys: Vec<&str> = c.iter().map(|c| c.key.as_str()).collect();
        assert!(!keys.contains(&"pacman/discord"), "already listed");
        assert!(!keys.contains(&"pacman/ark"), "dismissed");
        assert!(keys.contains(&"aur/spotify"));
        assert!(
            keys.contains(&"tweak/discord-skip-host-update"),
            "its app is listed: {keys:?}"
        );
        assert!(
            !keys.contains(&"tweak/dux-files-default"),
            "its app is not listed yet"
        );
    }
}
