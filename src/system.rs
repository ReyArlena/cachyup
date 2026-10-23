//! Read-only questions about the system: which commands exist, what is
//! installed, and what the package sources say about each app.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::catalog::{App, Catalog, Source};
use crate::paths::Paths;

/// Whether a command is on PATH.
pub fn has_cmd(name: &str) -> bool {
    if name.contains('/') {
        return is_executable(Path::new(name));
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| is_executable(&dir.join(name))))
}

/// Full path of a command on PATH.
pub fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| is_executable(p))
}

/// The package that owns a command, e.g. `dms` -> `dms-shell`.
pub fn owning_package(cmd: &str) -> Option<String> {
    let path = which(cmd)?;
    capture("pacman", &["-Qqo", &path.to_string_lossy()])
        .map(|o| o.trim().to_string())
        .filter(|p| !p.is_empty())
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Run a command and return its stdout, or `None` if it could not run or failed.
/// The C locale keeps field names stable for parsing.
pub fn capture(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Like `capture`, but returns stdout whatever the exit status
/// (`pacman -Si a b` exits 1 if any one name is missing).
pub fn capture_any(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Installed packages and flatpaks, read in two quick calls.
#[derive(Debug, Clone, Default)]
pub struct Installed {
    /// `None` when pacman could not be queried.
    pub packages: Option<HashSet<String>>,
    pub flatpaks: Option<HashSet<String>>,
}

impl Installed {
    pub fn probe() -> Self {
        let lines = |s: String| -> HashSet<String> {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        };
        let packages = capture("pacman", &["-Qq"]).map(lines);
        let flatpaks = if has_cmd("flatpak") {
            capture("flatpak", &["list", "--columns=application"]).map(lines)
        } else {
            Some(HashSet::new())
        };
        Self { packages, flatpaks }
    }

    /// `None` when it cannot tell.
    pub fn app(&self, app: &App, paths: &Paths) -> Option<bool> {
        match app.source {
            Source::Pacman | Source::Aur => self.packages.as_ref().map(|p| p.contains(&app.name)),
            Source::Flatpak => self.flatpaks.as_ref().map(|f| f.contains(&app.name)),
            Source::Installer => app
                .installer
                .as_ref()
                .map(|i| installer_present(&i.check, paths)),
        }
    }
}

/// An installer's `check`: a path (`~` allowed) or a command name.
pub fn installer_present(check: &str, paths: &Paths) -> bool {
    if check.starts_with('~') || check.contains('/') {
        paths.expand(check).exists()
    } else {
        has_cmd(check)
    }
}

/// What the package sources say about an app. Cached between launches.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PackageInfo {
    /// Whether the source has it. `None` when it could not be checked.
    pub available: Option<bool>,
    pub description: Option<String>,
    /// Extra facts for the info popup, in display order.
    pub details: Vec<(String, String)>,
}

pub type InfoMap = BTreeMap<String, PackageInfo>;

pub fn load_info_cache(path: &Path) -> InfoMap {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_info_cache(path: &Path, info: &InfoMap) {
    if let Ok(text) = serde_json::to_string(info) {
        let _ = crate::paths::write_file(path, &text);
    }
}

/// Ask every source about every app in the catalog. Slow-ish (network for the
/// AUR and maybe flathub), so call it off the UI thread.
pub fn fetch_info(catalog: &Catalog, installed: &Installed, paths: &Paths) -> InfoMap {
    let mut info = InfoMap::new();
    let names = |source: Source| -> Vec<&str> {
        catalog.apps_from(source).map(|a| a.name.as_str()).collect()
    };

    // Repo packages: pacman -Si covers description, version, repo, url.
    let repo = names(Source::Pacman);
    if !repo.is_empty() {
        let mut args = vec!["-Si"];
        args.extend(&repo);
        let parsed = capture_any("pacman", &args)
            .map(|out| parse_pacman_info(&out))
            .unwrap_or_default();
        let ok = !parsed.is_empty();
        for name in &repo {
            let entry = info.entry(format!("pacman/{name}")).or_default();
            if let Some(fields) = parsed.get(*name) {
                entry.available = Some(true);
                entry.description = fields.get("Description").cloned();
                for (label, key) in [
                    ("Version", "Version"),
                    ("Repository", "Repository"),
                    ("URL", "URL"),
                    ("Download size", "Download Size"),
                ] {
                    if let Some(v) = fields.get(key) {
                        entry.details.push((label.into(), v.clone()));
                    }
                }
            } else if ok {
                entry.available = Some(false);
            }
        }
    }

    // AUR: one request to the RPC interface for all names.
    let aur = names(Source::Aur);
    if !aur.is_empty() {
        let query: Vec<String> = aur.iter().map(|n| format!("arg[]={n}")).collect();
        let url = format!("https://aur.archlinux.org/rpc/v5/info?{}", query.join("&"));
        let parsed = capture("curl", &["-fsS", "--max-time", "15", &url])
            .and_then(|out| parse_aur_info(&out));
        for name in &aur {
            let entry = info.entry(format!("aur/{name}")).or_default();
            let Some(results) = &parsed else { continue };
            match results.iter().find(|r| r.name == *name) {
                Some(r) => {
                    entry.available = Some(true);
                    entry.description = r.description.clone();
                    entry.details.push(("Version".into(), r.version.clone()));
                    entry.details.push(("Repository".into(), "aur".into()));
                    if let Some(url) = &r.url {
                        entry.details.push(("URL".into(), url.clone()));
                    }
                    entry
                        .details
                        .push(("Votes".into(), r.num_votes.to_string()));
                    if r.out_of_date.is_some() {
                        entry.details.push(("Flagged".into(), "out of date".into()));
                    }
                }
                None => entry.available = Some(false),
            }
        }
    }

    // Flathub: one listing with descriptions.
    let flat = names(Source::Flatpak);
    if !flat.is_empty() && has_cmd("flatpak") {
        let listing = capture(
            "flatpak",
            &[
                "remote-ls",
                "flathub",
                "--app",
                "--columns=application,name,version,description",
            ],
        );
        let rows: HashMap<String, Vec<String>> = listing
            .as_deref()
            .map(|out| {
                out.lines()
                    .filter_map(|l| {
                        let cols: Vec<String> = l.split('\t').map(String::from).collect();
                        (cols.len() >= 4).then(|| (cols[0].clone(), cols))
                    })
                    .collect()
            })
            .unwrap_or_default();
        for name in &flat {
            let entry = info.entry(format!("flatpak/{name}")).or_default();
            if listing.is_none() {
                continue;
            }
            match rows.get(*name) {
                Some(cols) => {
                    entry.available = Some(true);
                    entry.description = Some(cols[3].clone()).filter(|d| !d.is_empty());
                    entry.details.push(("Name".into(), cols[1].clone()));
                    if !cols[2].is_empty() {
                        entry.details.push(("Version".into(), cols[2].clone()));
                    }
                    entry.details.push(("Repository".into(), "flathub".into()));
                }
                None => entry.available = Some(false),
            }
        }
    }

    // Installed packages (repo or AUR): local version, size, install date.
    let local: Vec<&str> = catalog
        .apps
        .iter()
        .filter(|a| matches!(a.source, Source::Pacman | Source::Aur))
        .filter(|a| installed.app(a, paths) == Some(true))
        .map(|a| a.name.as_str())
        .collect();
    if !local.is_empty() {
        let mut args = vec!["-Qi"];
        args.extend(&local);
        let parsed = capture_any("pacman", &args)
            .map(|out| parse_pacman_info(&out))
            .unwrap_or_default();
        for app in catalog
            .apps
            .iter()
            .filter(|a| matches!(a.source, Source::Pacman | Source::Aur))
        {
            let Some(fields) = parsed.get(&app.name) else {
                continue;
            };
            let entry = info.entry(app.key()).or_default();
            if entry.description.is_none() {
                entry.description = fields.get("Description").cloned();
            }
            for (label, key) in [
                ("Installed version", "Version"),
                ("Installed size", "Installed Size"),
                ("Install date", "Install Date"),
                ("Install reason", "Install Reason"),
            ] {
                if let Some(v) = fields.get(key) {
                    entry.details.push((label.into(), v.clone()));
                }
            }
        }
    }

    // Installers have no source to ask; their catalog description is it.
    for app in catalog.apps_from(Source::Installer) {
        let entry = info.entry(app.key()).or_default();
        entry.available = Some(true);
        if let Some(i) = &app.installer {
            entry
                .details
                .push(("Installs with".into(), i.install.clone()));
            entry.details.push(("Present when".into(), i.check.clone()));
        }
    }
    info
}

/// Parse `pacman -Si` / `-Qi` output: blank-line separated blocks of
/// `Key   : value` lines, with continuation lines indented. Keyed by `Name`.
pub fn parse_pacman_info(out: &str) -> HashMap<String, HashMap<String, String>> {
    let mut all = HashMap::new();
    for block in out.split("\n\n") {
        let mut fields: HashMap<String, String> = HashMap::new();
        let mut last: Option<String> = None;
        for line in block.lines() {
            if line.starts_with(' ') {
                if let Some(k) = &last
                    && let Some(v) = fields.get_mut(k)
                {
                    v.push(' ');
                    v.push_str(line.trim());
                }
                continue;
            }
            if let Some((k, v)) = line.split_once(" : ") {
                let k = k.trim().to_string();
                fields.insert(k.clone(), v.trim().to_string());
                last = Some(k);
            }
        }
        // pacman -Si lists a package once per repo; keep the first (highest priority).
        if let Some(name) = fields.get("Name").cloned() {
            all.entry(name).or_insert(fields);
        }
    }
    all
}

#[derive(Debug, Deserialize)]
pub struct AurResult {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Description")]
    pub description: Option<String>,
    #[serde(rename = "Version")]
    pub version: String,
    #[serde(rename = "URL")]
    pub url: Option<String>,
    #[serde(rename = "NumVotes", default)]
    pub num_votes: u64,
    #[serde(rename = "OutOfDate")]
    pub out_of_date: Option<u64>,
}

pub fn parse_aur_info(out: &str) -> Option<Vec<AurResult>> {
    #[derive(Deserialize)]
    struct Reply {
        results: Vec<AurResult>,
    }
    serde_json::from_str::<Reply>(out).ok().map(|r| r.results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pacman_info_blocks() {
        let out = "\
Repository      : extra
Name            : ark
Version         : 25.08.3-1
Description     : Archiving Tool
URL             : https://apps.kde.org/ark/
Depends On      : a  b
                  c

Repository      : cachyos
Name            : discord
Version         : 1:0.0.110-1
Description     : All-in-one voice and text chat
";
        let p = parse_pacman_info(out);
        assert_eq!(p["ark"]["Description"], "Archiving Tool");
        assert_eq!(p["ark"]["Depends On"], "a  b c");
        assert_eq!(p["discord"]["Repository"], "cachyos");
    }

    #[test]
    fn parses_aur_rpc_reply() {
        let out = r#"{"resultcount":1,"results":[{"Name":"spotify","Description":"Music","Version":"1.2-1","URL":null,"NumVotes":5,"OutOfDate":null}],"type":"multiinfo","version":5}"#;
        let r = parse_aur_info(out).unwrap();
        assert_eq!(r[0].name, "spotify");
        assert_eq!(r[0].num_votes, 5);
        assert!(parse_aur_info("not json").is_none());
    }

    #[test]
    fn finds_common_commands() {
        assert!(has_cmd("sh"));
        assert!(!has_cmd("definitely-not-a-command-xyz"));
    }
}
