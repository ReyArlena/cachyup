//! The app and tweak catalog: the built-in `catalog.toml` (compiled in) plus
//! the user's optional `user-apps.toml`, in the same layout.
//!
//! Every app has a key `source/name` (e.g. `pacman/discord`), used by the
//! selection and state files. Tweaks are keyed by their `id`.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub const BUILTIN: &str = include_str!("../catalog.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Source {
    Pacman,
    Aur,
    Flatpak,
    Installer,
}

impl Source {
    pub const ALL: [Source; 4] = [
        Source::Pacman,
        Source::Aur,
        Source::Flatpak,
        Source::Installer,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Source::Pacman => "pacman",
            Source::Aur => "aur",
            Source::Flatpak => "flatpak",
            Source::Installer => "installer",
        }
    }

    /// Where it comes from, for the info popup.
    pub fn label(self) -> &'static str {
        match self {
            Source::Pacman => "Repo (pacman)",
            Source::Aur => "AUR (yay)",
            Source::Flatpak => "Flathub (flatpak)",
            Source::Installer => "Install script",
        }
    }

    /// Heading used in the app list and the run.
    pub fn heading(self) -> &'static str {
        match self {
            Source::Pacman => "Pacman packages",
            Source::Aur => "AUR packages",
            Source::Flatpak => "Flatpaks",
            Source::Installer => "Installers",
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// Where an entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Builtin,
    User,
}

#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub source: Source,
    pub name: String,
    /// Display name when it differs from the package name (installers).
    pub label: Option<String>,
    pub description: Option<String>,
    /// Off by default when this command exists (e.g. junction when DMS is present).
    pub unless_cmd: Option<String>,
    pub unless_note: Option<String>,
    /// Explicit default; `None` means on (unless `unless_cmd` applies).
    pub default: Option<bool>,
    /// Only for `Source::Installer`.
    pub installer: Option<Installer>,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Installer {
    /// A command name on PATH, or a path (`~` allowed). Present means installed.
    pub check: String,
    /// Run with `bash -c`, as the user.
    pub install: String,
}

impl App {
    pub fn key(&self) -> String {
        format!("{}/{}", self.source.id(), self.name)
    }

    pub fn display_name(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tweak {
    pub id: String,
    /// Key of the app this belongs to, e.g. `pacman/discord`.
    pub app: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub unless_cmd: Option<String>,
    pub unless_note: Option<String>,
    pub default: Option<bool>,
    pub sets: Vec<SetOp>,
    pub origin: Origin,
}

/// One value a tweak writes.
#[derive(Debug, Clone, PartialEq)]
pub enum SetOp {
    Json {
        file: String,
        key: String,
        value: serde_json::Value,
    },
    Ini {
        file: String,
        section: String,
        key: String,
        value: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub prerequisites: Vec<String>,
    pub apps: Vec<App>,
    pub tweaks: Vec<Tweak>,
    /// Problems with the user file worth telling the user about at launch.
    pub warnings: Vec<String>,
}

impl Catalog {
    /// The built-in catalog only.
    pub fn builtin() -> Self {
        let file: CatalogFile = toml::from_str(BUILTIN).expect("built-in catalog.toml is valid");
        let mut catalog = Catalog::default();
        catalog
            .merge(file, Origin::Builtin)
            .expect("built-in catalog.toml is consistent");
        catalog
    }

    /// Built-in plus the user's file if it exists. Problems with the user file
    /// never stop the app: they end up in `warnings`.
    pub fn load(user_file: &Path) -> Self {
        let mut catalog = Self::builtin();
        if !user_file.exists() {
            return catalog;
        }
        let parsed = std::fs::read_to_string(user_file)
            .with_context(|| format!("reading {}", user_file.display()))
            .and_then(|text| {
                toml::from_str::<CatalogFile>(&text)
                    .with_context(|| format!("{} is not valid", user_file.display()))
            });
        match parsed {
            Ok(file) => {
                if let Err(e) = catalog.merge(file, Origin::User) {
                    catalog.warnings.push(format!("{e:#}"));
                }
            }
            Err(e) => catalog.warnings.push(format!("{e:#}")),
        }
        catalog
    }

    pub fn app(&self, key: &str) -> Option<&App> {
        self.apps.iter().find(|a| a.key() == key)
    }

    pub fn tweak(&self, id: &str) -> Option<&Tweak> {
        self.tweaks.iter().find(|t| t.id == id)
    }

    /// Tweaks that belong to an app, in catalog order.
    pub fn tweaks_for<'a>(&'a self, app_key: &'a str) -> impl Iterator<Item = &'a Tweak> + 'a {
        self.tweaks
            .iter()
            .filter(move |t| t.app.as_deref() == Some(app_key))
    }

    pub fn apps_from(&self, source: Source) -> impl Iterator<Item = &App> {
        self.apps.iter().filter(move |a| a.source == source)
    }

    /// Add a parsed file. Built-in duplicates are a bug (error); user entries
    /// that repeat an existing one are skipped with a warning.
    fn merge(&mut self, file: CatalogFile, origin: Origin) -> Result<()> {
        let user = origin == Origin::User;
        if user && !file.prerequisites.is_empty() {
            self.warnings
                .push("user-apps.toml: `prerequisites` is ignored; use `pacman` instead".into());
        }
        if !user {
            self.prerequisites = file.prerequisites;
        }

        let mut keys: HashSet<String> = self.apps.iter().map(App::key).collect();
        let mut incoming: Vec<App> = Vec::new();
        for (source, entries) in [
            (Source::Pacman, file.pacman),
            (Source::Aur, file.aur),
            (Source::Flatpak, file.flatpak),
        ] {
            for entry in entries {
                incoming.push(entry.into_app(source, origin));
            }
        }
        for def in file.installer {
            incoming.push(App {
                source: Source::Installer,
                name: def.name,
                label: def.label,
                description: def.description,
                unless_cmd: def.unless_cmd,
                unless_note: def.unless_note,
                default: def.default,
                installer: Some(Installer {
                    check: def.check,
                    install: def.install,
                }),
                origin,
            });
        }

        for app in incoming {
            if app.name.trim().is_empty() || app.name.contains(char::is_whitespace) {
                let msg = format!("{}: invalid name {:?}", app.source, app.name);
                if user {
                    self.warnings.push(format!("user-apps.toml: {msg}"));
                    continue;
                }
                bail!(msg);
            }
            let key = app.key();
            if !keys.insert(key.clone()) {
                if user {
                    let already = match self.app(&key).map(|a| a.origin) {
                        Some(Origin::Builtin) => "already in cachyup's list",
                        _ => "listed twice",
                    };
                    self.warnings
                        .push(format!("user-apps.toml: {key} is {already}, ignored"));
                    continue;
                }
                bail!("{key} is listed twice");
            }
            // Same name under another source: allowed, but probably a mistake.
            if user
                && let Some(other) = self
                    .apps
                    .iter()
                    .find(|a| a.name == app.name && a.source != app.source)
            {
                self.warnings.push(format!(
                    "user-apps.toml: {key} is also listed as {}",
                    other.key()
                ));
            }
            self.apps.push(app);
        }

        let mut ids: HashSet<String> = self.tweaks.iter().map(|t| t.id.clone()).collect();
        for def in file.tweak {
            let id = def.id.clone();
            let tweak = match def.into_tweak(origin) {
                Ok(t) => t,
                Err(e) if user => {
                    self.warnings.push(format!("user-apps.toml: {e:#}"));
                    continue;
                }
                Err(e) => return Err(e),
            };
            if let Some(app) = &tweak.app
                && self.app(app).is_none()
            {
                let msg = format!("tweak {id}: app {app} is not in the list");
                if user {
                    self.warnings.push(format!("user-apps.toml: {msg}"));
                    continue;
                }
                bail!(msg);
            }
            if !ids.insert(id.clone()) {
                if user {
                    self.warnings.push(format!(
                        "user-apps.toml: tweak {id} already exists, ignored"
                    ));
                    continue;
                }
                bail!("tweak {id} is listed twice");
            }
            self.tweaks.push(tweak);
        }
        Ok(())
    }
}

// --- file format ---

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CatalogFile {
    prerequisites: Vec<String>,
    pacman: Vec<Entry>,
    aur: Vec<Entry>,
    flatpak: Vec<Entry>,
    installer: Vec<InstallerDef>,
    tweak: Vec<TweakDef>,
}

/// A package list line: a bare name or a table with extras.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Entry {
    Name(String),
    Full(EntryDef),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryDef {
    name: String,
    description: Option<String>,
    unless_cmd: Option<String>,
    unless_note: Option<String>,
    default: Option<bool>,
}

impl Entry {
    fn into_app(self, source: Source, origin: Origin) -> App {
        let def = match self {
            Entry::Name(name) => EntryDef {
                name,
                description: None,
                unless_cmd: None,
                unless_note: None,
                default: None,
            },
            Entry::Full(def) => def,
        };
        App {
            source,
            name: def.name.trim().to_string(),
            label: None,
            description: def.description,
            unless_cmd: def.unless_cmd,
            unless_note: def.unless_note,
            default: def.default,
            installer: None,
            origin,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallerDef {
    name: String,
    label: Option<String>,
    description: Option<String>,
    check: String,
    install: String,
    unless_cmd: Option<String>,
    unless_note: Option<String>,
    default: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TweakDef {
    id: String,
    app: Option<String>,
    name: String,
    description: Option<String>,
    unless_cmd: Option<String>,
    unless_note: Option<String>,
    default: Option<bool>,
    set: Vec<SetDef>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetDef {
    json: Option<String>,
    ini: Option<String>,
    section: Option<String>,
    key: String,
    value: toml::Value,
}

impl TweakDef {
    fn into_tweak(self, origin: Origin) -> Result<Tweak> {
        if self.set.is_empty() {
            bail!("tweak {}: needs at least one `set`", self.id);
        }
        let mut sets = Vec::new();
        for s in self.set {
            sets.push(match (s.json, s.ini) {
                (Some(file), None) => SetOp::Json {
                    file,
                    key: s.key,
                    value: serde_json::to_value(&s.value)?,
                },
                (None, Some(file)) => SetOp::Ini {
                    file,
                    section: s
                        .section
                        .with_context(|| format!("tweak {}: ini needs `section`", self.id))?,
                    key: s.key,
                    value: match s.value {
                        toml::Value::String(v) => v,
                        other => other.to_string(),
                    },
                },
                _ => bail!(
                    "tweak {}: each `set` needs exactly one of `json` or `ini`",
                    self.id
                ),
            });
        }
        Ok(Tweak {
            id: self.id,
            app: self.app,
            name: self.name,
            description: self.description,
            unless_cmd: self.unless_cmd,
            unless_note: self.unless_note,
            default: self.default,
            sets,
            origin,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_parses_without_duplicates() {
        let c = Catalog::builtin();
        assert!(c.app("pacman/discord").is_some());
        assert!(c.app("aur/spotify").is_some());
        assert!(c.app("flatpak/org.inkscape.Inkscape").is_some());
        assert_eq!(
            c.app("installer/claude").unwrap().display_name(),
            "Claude Code"
        );
        assert!(c.prerequisites.contains(&"yay".to_string()));
        let junction = c.app("pacman/junction").unwrap();
        assert_eq!(junction.unless_cmd.as_deref(), Some("dms"));
        assert_eq!(c.tweaks_for("pacman/discord").count(), 1);
        assert!(c.warnings.is_empty());
    }

    #[test]
    fn user_file_adds_and_flags_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("user-apps.toml");
        std::fs::write(
            &file,
            r#"
pacman = ["htop", "discord", "htop"]
aur = ["obsidian"]
flatpak = [{ name = "com.example.App", description = "mine" }]

[[tweak]]
id = "htop-tree"
app = "pacman/htop"
name = "Tree view"
set = [{ ini = "~/.config/htop/htoprc", section = "x", key = "tree_view", value = 1 }]
"#,
        )
        .unwrap();
        let c = Catalog::load(&file);
        assert_eq!(c.app("pacman/htop").unwrap().origin, Origin::User);
        assert_eq!(
            c.app("flatpak/com.example.App")
                .unwrap()
                .description
                .as_deref(),
            Some("mine")
        );
        assert_eq!(c.apps.iter().filter(|a| a.name == "htop").count(), 1);
        let w = c.warnings.join("\n");
        assert!(
            w.contains("pacman/discord is already in cachyup's list"),
            "{w}"
        );
        assert!(w.contains("pacman/htop is listed twice"), "{w}");
        assert!(
            w.contains("aur/obsidian is also listed as pacman/obsidian"),
            "{w}"
        );
        match &c.tweak("htop-tree").unwrap().sets[0] {
            SetOp::Ini { value, .. } => assert_eq!(value, "1"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn broken_user_file_is_a_warning_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("user-apps.toml");
        std::fs::write(&file, "pacman = [1, 2").unwrap();
        let c = Catalog::load(&file);
        assert!(c.app("pacman/discord").is_some());
        assert_eq!(c.warnings.len(), 1);
    }
}
