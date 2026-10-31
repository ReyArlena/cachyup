//! GNU Stow dotfiles: which packages in the dotfiles folder are linked into
//! the home directory, and how (one link for a whole folder, or one link per
//! file inside a real folder).
//!
//! Read-only. The link state comes from reading the links themselves; what
//! stow would do next comes from `stow --simulate`, so stow's own rules
//! (ignore lists, tree folding, conflicts) decide, not a copy of them.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

/// One scan of the dotfiles folder.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub dir: PathBuf,
    pub packages: Vec<Package>,
    /// `stow` was found and the previews ran.
    pub stow: bool,
    /// The folder is a git repo, so git-ignored files could be checked.
    pub git: bool,
    /// The folder could not be read.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    pub name: String,
    pub state: PkgState,
    /// Where stow stops: each path is linked, missing or in the way.
    pub items: Vec<Item>,
    pub warnings: Vec<Warning>,
    /// `stow --simulate --verbose` output, minus its simulation notice.
    pub preview: Vec<String>,
    /// Files in the package (links in it count as files).
    pub files: usize,
    /// .gitignore rules for this package: root lines that name it, and every
    /// rule that ignores something in it.
    pub gitignore: Vec<Rule>,
    /// What git ignores in it, relative to the package (folders end in `/`).
    pub git_ignored: Vec<String>,
}

/// One line of a .gitignore file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rule {
    /// The ignore file, relative to the dotfiles folder (usually `.gitignore`).
    pub source: String,
    pub line: usize,
    pub pattern: String,
}

/// What git ignores in the dotfiles repo.
#[derive(Debug, Default)]
struct GitInfo {
    /// Relative to the dotfiles folder, folders collapsed.
    ignored: Vec<PathBuf>,
    /// The ones that are folders.
    dirs: HashSet<PathBuf>,
    /// The rule that ignores each path.
    rule_for: std::collections::HashMap<PathBuf, Rule>,
    /// Every rule in the root .gitignore.
    root: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Relative to the home directory.
    pub path: PathBuf,
    pub kind: ItemKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    /// The whole folder is one link into the package (stow "folded" it).
    Folder,
    /// A single file linked into the package, inside a real folder.
    File,
    /// Nothing there yet.
    Missing,
    /// Something that is not ours is in the way.
    Blocked(String),
    /// Linked from another package.
    Owned(String),
}

impl ItemKind {
    pub fn linked(&self) -> bool {
        matches!(self, ItemKind::Folder | ItemKind::File)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PkgState {
    /// Every path is linked.
    Linked(Shape),
    Partial {
        linked: usize,
        total: usize,
    },
    NotLinked,
    /// Its folder is already linked from this other package.
    TakenBy(String),
    /// Real files are in the way.
    Blocked,
    /// Nothing at its top level starts with a dot.
    NotPackage,
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Folders,
    Files,
    Mixed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    /// Data could be lost or leaked, rather than just a surprise.
    pub severe: bool,
    pub text: String,
}

impl Warning {
    fn note(text: String) -> Self {
        Self {
            severe: false,
            text,
        }
    }

    fn severe(text: String) -> Self {
        Self { severe: true, text }
    }
}

/// Scan every package. `use_stow: false` skips the `stow --simulate` previews
/// (tests, or when stow is missing).
pub fn scan(dir: &Path, home: &Path, ignore: &[String], use_stow: bool) -> Scan {
    let mut out = Scan {
        dir: dir.to_path_buf(),
        ..Scan::default()
    };
    let names = match package_names(dir, ignore) {
        Ok(names) => names,
        Err(e) => {
            out.error = Some(format!("cannot read {}: {e}", dir.display()));
            return out;
        }
    };
    out.stow = use_stow && crate::system::has_cmd("stow");
    let git = git_info(dir);
    out.git = git.is_some();
    let git = git.unwrap_or_default();

    let previews: Vec<Option<Vec<String>>> = if out.stow {
        std::thread::scope(|s| {
            let handles: Vec<_> = names
                .iter()
                .map(|n| s.spawn(move || stow_preview(dir, home, n)))
                .collect();
            handles.into_iter().map(|h| h.join().ok()).collect()
        })
    } else {
        vec![None; names.len()]
    };

    // Every file path each package would link, to spot packages that clash.
    let files: Vec<HashSet<PathBuf>> = names.iter().map(|n| package_files(&dir.join(n))).collect();

    for (i, name) in names.iter().enumerate() {
        out.packages.push(build_package(
            dir,
            home,
            name,
            &files,
            &names,
            i,
            previews[i].clone(),
            &git,
        ));
    }
    out
}

/// Top-level folders, minus hidden ones and the user's ignore list.
fn package_names(dir: &Path, ignore: &[String]) -> std::io::Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.') && !n.starts_with('-'))
        .filter(|n| !ignore.iter().any(|i| i == n))
        .collect();
    names.sort();
    Ok(names)
}

#[allow(clippy::too_many_arguments)]
fn build_package(
    dir: &Path,
    home: &Path,
    name: &str,
    all_files: &[HashSet<PathBuf>],
    names: &[String],
    index: usize,
    preview: Option<Vec<String>>,
    git: &GitInfo,
) -> Package {
    let ignored = &git.ignored;
    let pkg_dir = dir.join(name);
    let dot = canonical(dir);
    let ignores = IgnoreList::for_package(&pkg_dir);
    let mut items = Vec::new();
    walk(&pkg_dir, Path::new(""), home, &dot, &ignores, &mut items);

    // Missing paths stow would not link (its ignore list) are not missing.
    let planned: Option<HashSet<PathBuf>> = preview.as_ref().map(|p| {
        p.iter()
            .filter_map(|l| l.strip_prefix("LINK: "))
            .filter_map(|l| l.split(" => ").next())
            .map(PathBuf::from)
            .collect()
    });
    if let Some(planned) = &planned {
        items.retain(|i| i.kind != ItemKind::Missing || planned.contains(&i.path));
    }

    let top: Vec<String> = std::fs::read_dir(&pkg_dir)
        .map(|rd| {
            let mut v: Vec<String> = rd
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| !ignores.ignored(n, true))
                .collect();
            v.sort();
            v
        })
        .unwrap_or_default();

    let linked = items.iter().filter(|i| i.kind.linked()).count();
    let total = items.len();
    let owner = items.iter().find_map(|i| match &i.kind {
        ItemKind::Owned(o) => Some(o.clone()),
        _ => None,
    });
    let blocked = items.iter().any(|i| matches!(i.kind, ItemKind::Blocked(_)));
    let state = if top.is_empty() {
        PkgState::Empty
    } else if !top.iter().any(|n| n.starts_with('.')) {
        PkgState::NotPackage
    } else if total > 0 && linked == total {
        let folders = items.iter().any(|i| i.kind == ItemKind::Folder);
        let files = items.iter().any(|i| i.kind == ItemKind::File);
        PkgState::Linked(match (folders, files) {
            (true, true) => Shape::Mixed,
            (true, false) => Shape::Folders,
            _ => Shape::Files,
        })
    } else if linked > 0 {
        PkgState::Partial { linked, total }
    } else if let Some(o) = owner.clone() {
        PkgState::TakenBy(o)
    } else if blocked {
        PkgState::Blocked
    } else if total == 0 {
        PkgState::Empty
    } else {
        PkgState::NotLinked
    };

    let mut warnings = Vec::new();
    let tilde = |p: &Path| format!("~/{}", p.display());

    if state == PkgState::NotPackage {
        warnings.push(Warning::note(format!(
            "Nothing at its top level starts with a dot, so stow would link its contents straight into ~ (for example ~/{}). If it is not a stow package, add it to `ignore` under [dotfiles] in config.toml.",
            top[0]
        )));
    } else {
        for n in top.iter().filter(|n| !n.starts_with('.')) {
            warnings.push(Warning::note(format!(
                "{n} is at the top of the package, so stow would link it as ~/{n}."
            )));
        }
    }

    for item in &items {
        match &item.kind {
            ItemKind::Owned(other) => warnings.push(Warning::note(format!(
                "{} is linked from {other}. Linking {name} as well would split that folder link into single-file links and mix the two packages.",
                tilde(&item.path)
            ))),
            ItemKind::Blocked(why) => warnings.push(Warning::severe(format!(
                "{} is {why}, so stow will not link over it. Move it into the package or out of the way first.",
                tilde(&item.path)
            ))),
            _ => {}
        }
    }

    if let PkgState::Partial { .. } = state {
        let missing: Vec<String> = items
            .iter()
            .filter(|i| i.kind == ItemKind::Missing)
            .map(|i| tilde(&i.path))
            .collect();
        if !missing.is_empty() {
            warnings.push(Warning::note(format!(
                "Not linked yet: {}.",
                list_some(&missing, 4)
            )));
        }
    }

    // Packages that ship the same files as this one.
    for (j, other) in names.iter().enumerate() {
        if j == index || owner.as_deref() == Some(other.as_str()) {
            continue;
        }
        let mut shared: Vec<&PathBuf> = all_files[index].intersection(&all_files[j]).collect();
        if shared.is_empty() {
            continue;
        }
        shared.sort();
        warnings.push(Warning::note(format!(
            "Shares {} with {other} (for example {}): only one of them can be linked.",
            plural(shared.len(), "file"),
            tilde(shared[0])
        )));
    }

    // Folder links put everything the app writes there inside the repo.
    for item in items.iter().filter(|i| i.kind == ItemKind::Folder) {
        let inside = Path::new(name).join(&item.path);
        let under: Vec<&PathBuf> = ignored.iter().filter(|p| p.starts_with(&inside)).collect();
        if under.is_empty() {
            continue;
        }
        let rel: Vec<String> = under
            .iter()
            .map(|p| p.strip_prefix(&inside).unwrap_or(p).display().to_string())
            .collect();
        let secrets: Vec<String> = under
            .iter()
            .filter(|p| looks_secret(&dir.join(p)))
            .filter_map(|p| p.file_name()?.to_str().map(String::from))
            .collect();
        if !secrets.is_empty() {
            warnings.push(Warning::severe(format!(
                "Private keys or secrets live inside the repo through the {} folder link ({}). They are only kept out of git by .gitignore, and `git clean -fdX` (or -fdx) in the repo would delete them.",
                tilde(&item.path),
                list_some(&secrets, 4)
            )));
        }
        warnings.push(Warning::note(format!(
            "{} git-ignored {} inside the repo through the {} folder link (for example {}). Anything the app writes there lands in the repo; `git clean -fdX` would delete it.",
            under.len(),
            if under.len() == 1 { "entry lives" } else { "entries live" },
            tilde(&item.path),
            list_some(&rel, 3)
        )));
    }

    // Already explained by the owner warning when another package has it.
    let unlinks = preview
        .iter()
        .flatten()
        .filter(|l| l.starts_with("UNLINK: "))
        .count();
    if unlinks > 0 && owner.is_none() {
        warnings.push(Warning::note(format!(
            "Linking it now would first remove {}: stow splits a folder link so two packages can share the folder.",
            plural(unlinks, "existing link")
        )));
    }

    let (gitignore, git_ignored) = package_git(git, name);
    Package {
        name: name.to_string(),
        state,
        items,
        warnings,
        preview: preview.unwrap_or_default(),
        files: all_files[index].len(),
        gitignore,
        git_ignored,
    }
}

/// The .gitignore rules that touch a package, and what they ignore in it.
fn package_git(git: &GitInfo, name: &str) -> (Vec<Rule>, Vec<String>) {
    let mut rules: Vec<Rule> = git
        .root
        .iter()
        .filter(|r| {
            let p = r.pattern.trim_start_matches('!').trim_start_matches('/');
            p.strip_prefix(name)
                .is_some_and(|rest| rest.starts_with('/'))
        })
        .cloned()
        .collect();
    let mut ignored = Vec::new();
    for path in git.ignored.iter().filter(|p| p.starts_with(name)) {
        if let Some(rule) = git.rule_for.get(path) {
            rules.push(rule.clone());
        }
        let rel = path
            .strip_prefix(name)
            .unwrap_or(path)
            .display()
            .to_string();
        ignored.push(if git.dirs.contains(path) {
            format!("{rel}/")
        } else {
            rel
        });
    }
    // Root .gitignore first, in file order.
    rules.sort_by(|a, b| {
        (a.source != ".gitignore", &a.source, a.line).cmp(&(
            b.source != ".gitignore",
            &b.source,
            b.line,
        ))
    });
    rules.dedup();
    (rules, ignored)
}

/// Follow stow's tree: stop at each link, or where the home side is not a
/// real folder; recurse where both sides are real folders.
fn walk(
    src_dir: &Path,
    rel: &Path,
    home: &Path,
    dot: &Path,
    ignores: &IgnoreList,
    out: &mut Vec<Item>,
) {
    let Ok(rd) = std::fs::read_dir(src_dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if ignores.ignored(name_str, rel.as_os_str().is_empty()) {
            continue;
        }
        let src = entry.path();
        let path = rel.join(&name);
        let target = home.join(&path);
        let src_real_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        let kind = match target.symlink_metadata() {
            Err(_) => ItemKind::Missing,
            Ok(meta) if meta.file_type().is_symlink() => {
                let dest = link_destination(&target);
                if dest.is_some() && dest == Some(resolve_parent(&src)) {
                    if src.is_dir() {
                        ItemKind::Folder
                    } else {
                        ItemKind::File
                    }
                } else if let Some(other) = dest
                    .as_ref()
                    .and_then(|d| d.strip_prefix(dot).ok())
                    .and_then(|r| r.components().next())
                    .and_then(|c| c.as_os_str().to_str())
                {
                    ItemKind::Owned(other.to_string())
                } else {
                    let to = std::fs::read_link(&target)
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    ItemKind::Blocked(format!("a link to {to}, outside the dotfiles folder"))
                }
            }
            Ok(meta) if meta.is_dir() && src_real_dir => {
                walk(&src, &path, home, dot, ignores, out);
                continue;
            }
            Ok(meta) if meta.is_dir() => {
                ItemKind::Blocked("a real folder where the package has a file".into())
            }
            Ok(_) if src_real_dir => {
                ItemKind::Blocked("a real file where the package has a folder".into())
            }
            Ok(_) => ItemKind::Blocked("a real file".into()),
        };
        out.push(Item { path, kind });
    }
}

/// Where a link points, with every folder on the way resolved but the last
/// name kept, so a link to a link still matches the path stow wrote.
fn link_destination(link: &Path) -> Option<PathBuf> {
    let dest = std::fs::read_link(link).ok()?;
    let abs = link.parent()?.join(dest);
    Some(resolve_parent(&normalize(&abs)))
}

fn resolve_parent(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => canonical(parent).join(name),
        _ => path.to_path_buf(),
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Drop `.` and fold `..` without touching the filesystem.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Every file stow would link from a package, relative to its root.
fn package_files(pkg_dir: &Path) -> HashSet<PathBuf> {
    let ignores = IgnoreList::for_package(pkg_dir);
    let mut out = HashSet::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(pkg_dir.join(&rel)) else {
            continue;
        };
        for entry in rd.filter_map(|e| e.ok()) {
            let name = entry.file_name();
            if name
                .to_str()
                .is_none_or(|n| ignores.ignored(n, rel.as_os_str().is_empty()))
            {
                continue;
            }
            let path = rel.join(&name);
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(path);
            } else {
                out.insert(path);
            }
        }
    }
    out
}

/// Stow's default ignore list (used when a package has no
/// `.stow-local-ignore`), as plain name checks. Only used to keep the file
/// counts honest; the stow preview has the final say on what is missing.
struct IgnoreList {
    custom: Option<Vec<String>>,
}

impl IgnoreList {
    fn for_package(pkg_dir: &Path) -> Self {
        let custom = std::fs::read_to_string(pkg_dir.join(".stow-local-ignore"))
            .ok()
            .map(|text| {
                text.lines()
                    .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect()
            });
        Self { custom }
    }

    fn ignored(&self, name: &str, at_root: bool) -> bool {
        if name == ".stow-local-ignore" {
            return true;
        }
        if let Some(list) = &self.custom {
            return list.iter().any(|p| p.trim_start_matches('^') == name);
        }
        const NAMES: &[&str] = &[
            "RCS",
            "CVS",
            ".cvsignore",
            ".svn",
            "_darcs",
            ".hg",
            ".git",
            ".gitignore",
            ".gitmodules",
        ];
        NAMES.contains(&name)
            || name.ends_with(",v")
            || name.ends_with('~')
            || name.starts_with(".#")
            || (name.len() > 1 && name.starts_with('#') && name.ends_with('#'))
            || (at_root
                && (name.starts_with("README") || name.starts_with("LICENSE") || name == "COPYING"))
    }
}

/// `stow --simulate --verbose` for one package. Stow prints its plan on stderr.
fn stow_preview(dir: &Path, home: &Path, package: &str) -> Vec<String> {
    let output = Command::new("stow")
        .arg("--simulate")
        .arg("--verbose=1")
        .arg(format!("--dir={}", dir.display()))
        .arg(format!("--target={}", home.display()))
        .arg(package)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output();
    let Ok(output) = output else {
        return vec!["could not run stow".into()];
    };
    let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    parse_preview(&text)
}

fn parse_preview(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty() && !l.starts_with("WARNING: in simulation mode"))
        .map(String::from)
        .collect()
}

/// What git ignores in the repo and why. `None` when it is not a git repo.
/// Read-only: `git ls-files` and `git check-ignore`.
fn git_info(dir: &Path) -> Option<GitInfo> {
    if !dir.join(".git").exists() || !crate::system::has_cmd("git") {
        return None;
    }
    // With the trailing `/` git gives folders, which check-ignore needs.
    let raw = git_ls_ignored(dir)?;
    let mut info = GitInfo::default();
    for r in &raw {
        let path = PathBuf::from(r.trim_end_matches('/'));
        if r.ends_with('/') {
            info.dirs.insert(path.clone());
        }
        info.ignored.push(path);
    }
    for (path, rule) in git_check_ignore(dir, &raw) {
        info.rule_for
            .insert(PathBuf::from(path.trim_end_matches('/')), rule);
    }
    info.root = std::fs::read_to_string(dir.join(".gitignore"))
        .map(|text| parse_rules(".gitignore", &text))
        .unwrap_or_default();
    Some(info)
}

fn parse_rules(source: &str, text: &str) -> Vec<Rule> {
    text.lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim()))
        .filter(|(_, l)| !l.is_empty() && !l.starts_with('#'))
        .map(|(line, l)| Rule {
            source: source.to_string(),
            line,
            pattern: l.to_string(),
        })
        .collect()
}

/// `git check-ignore -v`: the rule that ignores each path.
fn git_check_ignore(dir: &Path, paths: &[String]) -> Vec<(String, Rule)> {
    use std::io::Write;
    if paths.is_empty() {
        return Vec::new();
    }
    let Ok(mut child) = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["check-ignore", "--verbose", "-z", "--stdin"])
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    // Written from another thread so a full stdout pipe cannot block us.
    let mut input = Vec::new();
    for p in paths {
        input.extend_from_slice(p.as_bytes());
        input.push(0);
    }
    let stdin = child.stdin.take();
    let writer = std::thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(&input);
        }
    });
    let output = child.wait_with_output();
    let _ = writer.join();
    let Ok(output) = output else {
        return Vec::new();
    };
    // source NUL line NUL pattern NUL path NUL, per path.
    let fields: Vec<String> = output
        .stdout
        .split(|b| *b == 0)
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    fields
        .chunks_exact(4)
        .filter(|c| !c[0].is_empty())
        .map(|c| {
            let source = Path::new(&c[0])
                .strip_prefix(dir)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| c[0].clone());
            (
                c[3].clone(),
                Rule {
                    source,
                    line: c[1].parse().unwrap_or(0),
                    pattern: c[2].clone(),
                },
            )
        })
        .collect()
}

/// `git ls-files --others --ignored`, folders collapsed and ending in `/`.
fn git_ls_ignored(dir: &Path) -> Option<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "-z",
        ])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        output
            .stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect(),
    )
}

/// Usually private: SSH and TLS keys (by name or by their PEM header) and
/// files named as secrets.
fn looks_secret(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    let name = name.to_lowercase();
    if name.ends_with(".pub") {
        return false;
    }
    if name.starts_with("id_")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with("_key")
        || name.contains("secret")
        || name.contains("credentials")
    {
        return true;
    }
    // Keys can have any name (~/.ssh/work). Only small files are read.
    use std::io::Read;
    let small = path
        .symlink_metadata()
        .is_ok_and(|m| m.is_file() && m.len() < 64 * 1024);
    let mut head = [0u8; 4096];
    small
        && std::fs::File::open(path)
            .and_then(|mut f| f.read(&mut head))
            .is_ok_and(|n| String::from_utf8_lossy(&head[..n]).contains("PRIVATE KEY-----"))
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// "a, b, c and 4 more".
fn list_some(items: &[String], max: usize) -> String {
    let shown = items[..items.len().min(max)].join(", ");
    if items.len() > max {
        format!("{shown} and {} more", items.len() - max)
    } else {
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A home and a dotfiles folder covering every state.
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let dot = home.join("dotfiles");
        std::fs::create_dir_all(&dot).unwrap();
        let link = |to: &str, at: &str| {
            let at = home.join(at);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            symlink(to, at).unwrap();
        };

        // Folder link.
        write(&dot.join("fish/.config/fish/config.fish"), "");
        link("../dotfiles/fish/.config/fish", ".config/fish");
        // File links inside a real folder, one missing.
        write(&dot.join("units/.config/systemd/user/a.service"), "");
        write(&dot.join("units/.config/systemd/user/b.service"), "");
        link(
            "../../../dotfiles/units/.config/systemd/user/a.service",
            ".config/systemd/user/a.service",
        );
        // A single file at the home root.
        write(&dot.join("git/.gitconfig"), "");
        link("dotfiles/git/.gitconfig", ".gitconfig");
        // Two packages for the same folder; the other one is linked.
        write(&dot.join("hypr/.config/hypr/hyprland.conf"), "");
        write(&dot.join("hypr-dms/.config/hypr/hyprland.conf"), "");
        link("../dotfiles/hypr-dms/.config/hypr", ".config/hypr");
        // A real file in the way.
        write(&dot.join("smb/.smb-nas"), "");
        write(&home.join(".smb-nas"), "user=me");
        // Not linked at all.
        write(&dot.join("niri/.config/niri/config.kdl"), "");
        // Not a package: no dotted entries.
        write(&dot.join("helpers/setup.sh"), "");
        // Ignored by stow.
        write(&dot.join("niri/README.md"), "");
        (tmp, home, dot)
    }

    fn find<'a>(scan: &'a Scan, name: &str) -> &'a Package {
        scan.packages.iter().find(|p| p.name == name).unwrap()
    }

    #[test]
    fn states_cover_folders_files_and_problems() {
        let (_tmp, home, dot) = fixture();
        let scan = scan(&dot, &home, &[], false);
        assert_eq!(find(&scan, "fish").state, PkgState::Linked(Shape::Folders));
        assert_eq!(find(&scan, "git").state, PkgState::Linked(Shape::Files));
        assert_eq!(
            find(&scan, "units").state,
            PkgState::Partial {
                linked: 1,
                total: 2
            }
        );
        assert_eq!(
            find(&scan, "hypr").state,
            PkgState::TakenBy("hypr-dms".into())
        );
        assert_eq!(
            find(&scan, "hypr-dms").state,
            PkgState::Linked(Shape::Folders)
        );
        assert_eq!(find(&scan, "smb").state, PkgState::Blocked);
        assert_eq!(find(&scan, "niri").state, PkgState::NotLinked);
        assert_eq!(find(&scan, "helpers").state, PkgState::NotPackage);

        // README at the package root is ignored, as stow does.
        let niri = find(&scan, "niri");
        assert_eq!(niri.items.len(), 1);
        assert_eq!(niri.items[0].path, PathBuf::from(".config/niri"));
        assert!(niri.warnings.is_empty(), "{:?}", niri.warnings);
    }

    #[test]
    fn warnings_explain_clashes_and_blockers() {
        let (_tmp, home, dot) = fixture();
        let scan = scan(&dot, &home, &[], false);
        let text = |name: &str| -> String {
            find(&scan, name)
                .warnings
                .iter()
                .map(|w| w.text.clone())
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(text("hypr").contains("linked from hypr-dms"));
        assert!(text("hypr-dms").contains("Shares 1 file with hypr"));
        assert!(text("smb").contains("~/.smb-nas is a real file"));
        assert!(text("units").contains("b.service"));
        assert!(text("helpers").contains("~/setup.sh"));
        assert!(find(&scan, "smb").warnings.iter().any(|w| w.severe));
    }

    #[test]
    fn ignore_list_hides_packages() {
        let (_tmp, home, dot) = fixture();
        let scan = scan(&dot, &home, &["helpers".into()], false);
        assert!(!scan.packages.iter().any(|p| p.name == "helpers"));
    }

    #[test]
    fn link_to_a_link_in_the_package_still_counts() {
        let (_tmp, home, dot) = fixture();
        // The package holds a link to a folder elsewhere (like a vault).
        let vault = home.join("vault");
        write(&vault.join("note.md"), "");
        std::fs::create_dir_all(dot.join("claude/.claude")).unwrap();
        symlink(&vault, dot.join("claude/.claude/skills")).unwrap();
        write(&dot.join("claude/.claude/CLAUDE.md"), "");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        symlink(
            "../dotfiles/claude/.claude/skills",
            home.join(".claude/skills"),
        )
        .unwrap();
        symlink(
            "../dotfiles/claude/.claude/CLAUDE.md",
            home.join(".claude/CLAUDE.md"),
        )
        .unwrap();
        let scan = scan(&dot, &home, &[], false);
        assert_eq!(find(&scan, "claude").state, PkgState::Linked(Shape::Mixed));
    }

    #[test]
    fn foreign_link_blocks() {
        let (_tmp, home, dot) = fixture();
        write(&dot.join("waybar/.config/waybar/config"), "");
        symlink("/nowhere/waybar", home.join(".config/waybar")).unwrap();
        let scan = scan(&dot, &home, &[], false);
        let waybar = find(&scan, "waybar");
        assert_eq!(waybar.state, PkgState::Blocked);
        assert!(waybar.warnings[0].text.contains("/nowhere/waybar"));
    }

    #[test]
    fn secrets_are_spotted_by_name_or_content() {
        assert!(looks_secret(Path::new("ssh/.ssh/id_ed25519")));
        assert!(!looks_secret(Path::new("ssh/.ssh/id_ed25519.pub")));
        assert!(looks_secret(Path::new("x/secrets.fish")));
        assert!(!looks_secret(Path::new("x/config")));
        let tmp = tempfile::tempdir().unwrap();
        let key = tmp.path().join("work");
        write(&key, "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n");
        assert!(looks_secret(&key));
    }

    #[test]
    fn ignored_keys_behind_a_folder_link_are_severe() {
        if !crate::system::has_cmd("git") {
            return;
        }
        let (_tmp, home, dot) = fixture();
        write(&dot.join(".gitignore"), "ssh/.ssh/*\n!ssh/.ssh/config\n");
        write(&dot.join("ssh/.ssh/config"), "");
        write(
            &dot.join("ssh/.ssh/work"),
            "-----BEGIN OPENSSH PRIVATE KEY-----\n",
        );
        symlink("dotfiles/ssh/.ssh", home.join(".ssh")).unwrap();
        let ok = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dot)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return;
        }
        let scan = scan(&dot, &home, &[], false);
        let ssh = find(&scan, "ssh");
        assert_eq!(ssh.state, PkgState::Linked(Shape::Folders));
        assert!(
            ssh.warnings
                .iter()
                .any(|w| w.severe && w.text.contains("work")),
            "{:?}",
            ssh.warnings
        );
        let patterns: Vec<&str> = ssh.gitignore.iter().map(|r| r.pattern.as_str()).collect();
        assert_eq!(patterns, ["ssh/.ssh/*", "!ssh/.ssh/config"]);
        assert_eq!(ssh.git_ignored, [".ssh/work"]);
        // Packages the .gitignore never mentions have no rules.
        assert!(find(&scan, "fish").gitignore.is_empty());
    }

    #[test]
    fn preview_drops_the_simulation_notice() {
        let text = "LINK: .config/niri => ../dotfiles/niri/.config/niri\nWARNING: in simulation mode so not modifying filesystem.\n";
        assert_eq!(
            parse_preview(text),
            vec!["LINK: .config/niri => ../dotfiles/niri/.config/niri"]
        );
    }

    /// Against the real stow, when it is installed.
    #[test]
    fn stow_preview_matches_the_walk() {
        if !crate::system::has_cmd("stow") {
            return;
        }
        let (_tmp, home, dot) = fixture();
        let scan = scan(&dot, &home, &[], true);
        assert!(scan.stow);
        let niri = find(&scan, "niri");
        assert!(
            niri.preview
                .iter()
                .any(|l| l.starts_with("LINK: .config/niri")),
            "{:?}",
            niri.preview
        );
        // Already linked: stow has nothing to do.
        assert!(
            find(&scan, "fish").preview.is_empty(),
            "{:?}",
            find(&scan, "fish").preview
        );
        let smb = find(&scan, "smb");
        assert!(smb.preview.iter().any(|l| l.contains("conflicts")));
    }
}
