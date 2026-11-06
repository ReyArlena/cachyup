//! The app and tweak catalog. The user's `~/.config/cachyup/apps.toml` is
//! the list; it starts as a copy of the built-in `apps.toml` (compiled in),
//! which stays around as the reference for prerequisites and new entries.
//!
//! Every app has a key `source/name` (e.g. `pacman/discord`), used by the
//! selection and state files. Tweaks are keyed by their `id`.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::paths::write_file;
use serde::Deserialize;

pub const BUILTIN: &str = include_str!("../apps.toml");

/// Name of the user's copy, in the config directory.
pub const FILE_NAME: &str = "apps.toml";

/// Category per package name, for grouping the app list. Built in; the
/// user's file knows nothing about it.
const CATEGORIES: &str = include_str!("../categories.toml");

pub const OTHER_CATEGORY: &str = "Other";

/// The categories in display order, each with its package names.
pub fn categories() -> Vec<(String, Vec<String>)> {
    let table: toml::Table = toml::from_str(CATEGORIES).expect("categories.toml is valid");
    // `toml::Table` sorts keys; the file order is kept by re-reading headers.
    let order: Vec<String> = CATEGORIES
        .lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix('"')?
                .split_once('"')
                .map(|(k, _)| k.to_string())
        })
        .filter(|k| table.contains_key(k))
        .collect();
    order
        .into_iter()
        .map(|name| {
            let names = table[&name]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            (name, names)
        })
        .collect()
}

/// The category of a package name, or "Other".
pub fn category_of(name: &str) -> String {
    categories()
        .into_iter()
        .find(|(_, names)| names.iter().any(|n| n == name))
        .map_or(OTHER_CATEGORY.to_string(), |(c, _)| c)
}

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

    pub fn from_id(id: &str) -> Option<Self> {
        Source::ALL.into_iter().find(|s| s.id() == id)
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

/// Which file an entry was parsed from. The built-in list must be
/// consistent (errors); the user's file gets warnings instead.
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
    /// Optional update command, run by the "Installer updates" step.
    pub update: Option<String>,
    /// Optional removal command, for `u` in the app list.
    pub uninstall: Option<String>,
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
    pub default: Option<bool>,
    /// Packages installed before `sets` are written (optional dependencies).
    pub installs: Vec<Package>,
    pub sets: Vec<SetOp>,
    /// A shell command run after `sets`, guarded by its own check.
    pub run: Option<RunOp>,
    pub origin: Origin,
}

/// A tweak step done by a shell command. `check` decides whether it is
/// already in place; `run` applies it. Both run with `bash -c`, as the user.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOp {
    /// Exit 0 means applied. Runs for real in a dry run, so it must change nothing.
    pub check: String,
    pub run: String,
}

/// A package a tweak installs, from any package source.
#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    pub source: Source,
    pub name: String,
}

impl Package {
    /// `name` (pacman) or `source/name`; installers are not packages.
    pub fn parse(spec: &str) -> Result<Self> {
        let (source, name) = match spec.split_once('/') {
            Some((s, n)) => (
                Source::from_id(s)
                    .filter(|s| *s != Source::Installer)
                    .with_context(|| format!("{spec}: unknown source {s:?}"))?,
                n,
            ),
            None => (Source::Pacman, spec),
        };
        let name = name.trim();
        if name.is_empty() || name.contains(char::is_whitespace) {
            bail!("invalid package name {spec:?}");
        }
        Ok(Package {
            source,
            name: name.to_string(),
        })
    }
}

impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.source {
            Source::Pacman => f.write_str(&self.name),
            source => write!(f, "{source}/{}", self.name),
        }
    }
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
        let file: CatalogFile = toml::from_str(BUILTIN).expect("built-in apps.toml is valid");
        let mut catalog = Catalog::default();
        catalog
            .merge(file, Origin::Builtin)
            .expect("built-in apps.toml is consistent");
        catalog
    }

    /// Create the user's file from the built-in list when it is missing.
    /// An old `user-apps.toml` is merged into the new file and renamed.
    /// Returns anything worth telling the user; never fails the launch.
    pub fn ensure_file(dir: &Path) -> Vec<String> {
        let mut warnings = Vec::new();
        let file = dir.join(FILE_NAME);
        if file.exists() {
            return warnings;
        }
        let legacy = dir.join("user-apps.toml");
        let mut text = BUILTIN.to_string();
        if legacy.exists() {
            match std::fs::read_to_string(&legacy)
                .map_err(anyhow::Error::from)
                .and_then(|extra| crate::apps_edit::append(BUILTIN, &extra))
            {
                Ok(merged) => text = merged,
                Err(e) => warnings.push(format!(
                    "user-apps.toml could not be merged into {FILE_NAME}: {e:#}"
                )),
            }
        }
        if let Err(e) = write_file(&file, &text) {
            warnings.push(format!("could not create {}: {e}", file.display()));
            return warnings;
        }
        if legacy.exists() {
            let moved = dir.join("user-apps.toml.migrated");
            if let Err(e) = std::fs::rename(&legacy, &moved) {
                warnings.push(format!("could not rename {FILE_NAME}: {e}"));
            } else {
                warnings.push(format!(
                    "user-apps.toml merged into {FILE_NAME} and renamed to user-apps.toml.migrated"
                ));
            }
        }
        warnings
    }

    /// The user's file. Falls back to the built-in list, with a warning, when
    /// it is missing or does not parse. Built-in prerequisites are always
    /// included, so a trimmed file cannot break a run.
    pub fn load(user_file: &Path) -> Self {
        let parsed = std::fs::read_to_string(user_file)
            .with_context(|| format!("reading {}", user_file.display()))
            .and_then(|text| {
                toml::from_str::<CatalogFile>(&text)
                    .with_context(|| format!("{} is not valid", user_file.display()))
            });
        let mut catalog = match parsed {
            Ok(file) => {
                let mut catalog = Catalog::default();
                if let Err(e) = catalog.merge(file, Origin::User) {
                    catalog.warnings.push(format!("{e:#}"));
                }
                catalog
            }
            Err(e) => {
                let mut catalog = Self::builtin();
                catalog
                    .warnings
                    .push(format!("{e:#}. Using the built-in list for now."));
                catalog
            }
        };
        for p in Self::builtin().prerequisites {
            if !catalog.prerequisites.contains(&p) {
                catalog.prerequisites.push(p);
            }
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

    /// Add a parsed file. Built-in duplicates are a bug (error); duplicates
    /// in the user's file are skipped with a warning.
    fn merge(&mut self, file: CatalogFile, origin: Origin) -> Result<()> {
        let user = origin == Origin::User;
        self.prerequisites = file.prerequisites;

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
                default: def.default,
                installer: Some(Installer {
                    check: def.check,
                    install: def.install,
                    update: def.update.filter(|u| !u.trim().is_empty()),
                    uninstall: def.uninstall.filter(|u| !u.trim().is_empty()),
                }),
                origin,
            });
        }

        for app in incoming {
            if app.name.trim().is_empty() || app.name.contains(char::is_whitespace) {
                let msg = format!("{}: invalid name {:?}", app.source, app.name);
                if user {
                    self.warnings.push(format!("{FILE_NAME}: {msg}"));
                    continue;
                }
                bail!(msg);
            }
            let key = app.key();
            if !keys.insert(key.clone()) {
                if user {
                    self.warnings
                        .push(format!("{FILE_NAME}: {key} is listed twice, ignored"));
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
                    "{FILE_NAME}: {key} is also listed as {}",
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
                    self.warnings.push(format!("{FILE_NAME}: {e:#}"));
                    continue;
                }
                Err(e) => return Err(e),
            };
            if let Some(app) = &tweak.app
                && self.app(app).is_none()
            {
                let msg = format!("tweak {id}: app {app} is not in the list");
                if user {
                    self.warnings.push(format!("{FILE_NAME}: {msg}"));
                    continue;
                }
                bail!(msg);
            }
            if !ids.insert(id.clone()) {
                if user {
                    self.warnings
                        .push(format!("{FILE_NAME}: tweak {id} is listed twice, ignored"));
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
    default: Option<bool>,
}

impl Entry {
    fn into_app(self, source: Source, origin: Origin) -> App {
        let def = match self {
            Entry::Name(name) => EntryDef {
                name,
                description: None,
                unless_cmd: None,
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
    update: Option<String>,
    uninstall: Option<String>,
    unless_cmd: Option<String>,
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
    default: Option<bool>,
    #[serde(default)]
    installs: Vec<String>,
    #[serde(default)]
    set: Vec<SetDef>,
    check: Option<String>,
    run: Option<String>,
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
        let run = match (self.check, self.run) {
            (Some(check), Some(run)) if !check.trim().is_empty() && !run.trim().is_empty() => {
                Some(RunOp { check, run })
            }
            (None, None) => None,
            _ => bail!("tweak {}: `run` and `check` go together", self.id),
        };
        if self.set.is_empty() && self.installs.is_empty() && run.is_none() {
            bail!(
                "tweak {}: needs `installs`, at least one `set`, or `run` with `check`",
                self.id
            );
        }
        let installs = self
            .installs
            .iter()
            .map(|spec| Package::parse(spec))
            .collect::<Result<Vec<_>>>()
            .with_context(|| format!("tweak {}", self.id))?;
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
            default: self.default,
            installs,
            sets,
            run,
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
        let claude = c.app("installer/claude").unwrap();
        assert_eq!(claude.display_name(), "Claude Code");
        assert_eq!(
            claude.installer.as_ref().unwrap().update.as_deref(),
            Some("claude update")
        );
        assert!(c.prerequisites.contains(&"yay".to_string()));
        let junction = c.app("pacman/junction").unwrap();
        assert_eq!(junction.unless_cmd.as_deref(), Some("dms"));
        assert_eq!(c.tweaks_for("pacman/discord").count(), 1);
        let extras = c.tweak("dux-files-extras").unwrap();
        assert_eq!(extras.installs.len(), 5);
        assert!(extras.sets.is_empty());
        assert!(c.warnings.is_empty());
    }

    #[test]
    fn categories_cover_the_builtin_list_once() {
        let cats = categories();
        assert_eq!(cats[0].0, "Developer Tools");
        assert!(cats.iter().any(|(c, _)| c == "Gaming"));
        let mut seen = HashSet::new();
        for (_, names) in &cats {
            for n in names {
                assert!(seen.insert(n.clone()), "{n} is in two categories");
            }
        }
        let c = Catalog::builtin();
        let uncategorised: Vec<String> = c
            .apps
            .iter()
            .filter(|a| category_of(&a.name) == OTHER_CATEGORY)
            .map(App::key)
            .collect();
        assert!(uncategorised.is_empty(), "{uncategorised:?}");
        let listed: HashSet<&str> = c.apps.iter().map(|a| a.name.as_str()).collect();
        let stale: Vec<&String> = seen
            .iter()
            .filter(|n| !listed.contains(n.as_str()))
            .collect();
        assert!(stale.is_empty(), "not in apps.toml: {stale:?}");
        assert_eq!(category_of("nothing-like-this"), OTHER_CATEGORY);
    }

    #[test]
    fn installs_parse_sources_and_reject_bad_specs() {
        let p = Package::parse("poppler").unwrap();
        assert_eq!((p.source, p.name.as_str()), (Source::Pacman, "poppler"));
        let p = Package::parse("aur/foo-git").unwrap();
        assert_eq!((p.source, p.name.as_str()), (Source::Aur, "foo-git"));
        assert_eq!(p.to_string(), "aur/foo-git");
        let p = Package::parse("flatpak/org.example.App").unwrap();
        assert_eq!(p.source, Source::Flatpak);
        assert!(Package::parse("installer/foo").is_err());
        assert!(Package::parse("snap/foo").is_err());
        assert!(Package::parse("aur/").is_err());
        assert!(Package::parse("two words").is_err());

        let file: CatalogFile = toml::from_str(
            r#"
            [[tweak]]
            id = "empty"
            name = "nothing"
            "#,
        )
        .unwrap();
        let def = file.tweak.into_iter().next().unwrap();
        assert!(def.into_tweak(Origin::Builtin).is_err());
    }

    #[test]
    fn run_tweaks_need_a_check() {
        let parse = |body: &str| -> Result<Tweak> {
            let file: CatalogFile =
                toml::from_str(&format!("[[tweak]]\nid = \"t\"\nname = \"t\"\n{body}")).unwrap();
            file.tweak
                .into_iter()
                .next()
                .unwrap()
                .into_tweak(Origin::User)
        };
        let t = parse("check = \"true\"\nrun = \"echo hi\"").unwrap();
        assert_eq!(t.run.as_ref().map(|r| r.run.as_str()), Some("echo hi"));
        assert!(t.sets.is_empty());
        assert!(parse("run = \"echo hi\"").is_err());
        assert!(parse("check = \"true\"").is_err());
        assert!(parse("check = \"\"\nrun = \"echo hi\"").is_err());
        let t = parse("installs = [\"htop\"]").unwrap();
        assert!(t.run.is_none());
    }

    #[test]
    fn user_file_is_the_list_and_flags_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE_NAME);
        std::fs::write(
            &file,
            r#"
pacman = ["htop", "htop"]
aur = ["htop"]
flatpak = [{ name = "com.example.App", description = "mine" }]

[[tweak]]
id = "htop-tree"
app = "pacman/htop"
name = "Tree view"
set = [{ ini = "~/.config/htop/htoprc", section = "x", key = "tree_view", value = 1 }]

[[tweak]]
id = "orphan"
app = "pacman/nothing"
name = "No such app"
set = [{ ini = "~/x", section = "x", key = "k", value = 1 }]
"#,
        )
        .unwrap();
        let c = Catalog::load(&file);
        assert_eq!(c.app("pacman/htop").unwrap().origin, Origin::User);
        assert!(
            c.app("pacman/discord").is_none(),
            "built-in apps are not added"
        );
        assert!(
            c.prerequisites.contains(&"yay".to_string()),
            "built-in prerequisites always apply"
        );
        assert_eq!(
            c.app("flatpak/com.example.App")
                .unwrap()
                .description
                .as_deref(),
            Some("mine")
        );
        assert_eq!(c.apps.iter().filter(|a| a.name == "htop").count(), 2);
        let w = c.warnings.join("\n");
        assert!(w.contains("pacman/htop is listed twice"), "{w}");
        assert!(w.contains("aur/htop is also listed as pacman/htop"), "{w}");
        assert!(w.contains("app pacman/nothing is not in the list"), "{w}");
        assert!(c.tweak("orphan").is_none());
        match &c.tweak("htop-tree").unwrap().sets[0] {
            SetOp::Ini { value, .. } => assert_eq!(value, "1"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ensure_file_copies_the_builtin_and_migrates_user_apps() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Catalog::ensure_file(dir.path()).is_empty());
        let file = dir.path().join(FILE_NAME);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), BUILTIN);
        // A second call leaves the file alone.
        std::fs::write(&file, "pacman = [\"htop\"]\n").unwrap();
        assert!(Catalog::ensure_file(dir.path()).is_empty());
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "pacman = [\"htop\"]\n"
        );

        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("user-apps.toml");
        std::fs::write(&legacy, "pacman = [\"htop\"]\n").unwrap();
        let w = Catalog::ensure_file(dir.path());
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(!legacy.exists());
        assert!(dir.path().join("user-apps.toml.migrated").exists());
        let c = Catalog::load(&dir.path().join(FILE_NAME));
        assert!(c.app("pacman/htop").is_some());
        assert!(c.app("pacman/discord").is_some());
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    }

    #[test]
    fn broken_or_missing_user_file_falls_back_to_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE_NAME);
        std::fs::write(&file, "pacman = [1, 2").unwrap();
        let c = Catalog::load(&file);
        assert!(c.app("pacman/discord").is_some());
        assert_eq!(c.warnings.len(), 1);
        assert!(c.warnings[0].contains("Using the built-in list"));
        let c = Catalog::load(&dir.path().join("missing.toml"));
        assert!(c.app("pacman/discord").is_some());
        assert_eq!(c.warnings.len(), 1);
    }
}
