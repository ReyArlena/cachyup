//! Every step cachyup can run. Each one checks the real end state first
//! (SKIPPED when already in place), acts, and for installs checks again, so
//! re-running is always safe. Checks run for real in a dry run; changes do not.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::catalog::{App, SetOp, Source, Tweak};
use crate::config::Config;
use crate::system::{capture_any, has_cmd, installer_present};

use super::edit::{ini_get, ini_set, json_get, json_set};
use super::plan::{
    GROUP_CHECKS, GROUP_CLEANUP, GROUP_PREFLIGHT, GROUP_PREREQ, GROUP_TWEAKS, GROUP_UPDATE, Step,
};
use super::{Cmd, Ctx, Outcome, Status};

const MIRRORLIST: &str = "/etc/pacman.d/cachyos-mirrorlist";
const PACMAN_LOCK: &str = "/var/lib/pacman/db.lck";
const FLATHUB_URL: &str = "https://flathub.org/repo/flathub.flatpakrepo";

/// cachyos-rate-mirrors rewrites these on every rating.
const RATED_MIRRORLISTS: &[&str] = &[
    "/etc/pacman.d/mirrorlist",
    "/etc/pacman.d/cachyos-mirrorlist",
    "/etc/pacman.d/cachyos-v3-mirrorlist",
    "/etc/pacman.d/cachyos-v4-mirrorlist",
];

/// Packages that only repack upstream binaries: a rebuild changes nothing.
const PREBUILT_PATTERNS: &[&str] = &[
    "*-bin",
    "*-appimage",
    "phpstorm",
    "goland",
    "rustrover",
    "clion",
    "datagrip",
    "webstorm",
    "rider",
    "pycharm*",
    "intellij*",
    "google-chrome",
    "spotify",
    "discord-ptb",
];

/// Run commands in order; the first failure fails the step.
fn act(cx: &mut Ctx, cmds: &[Cmd]) -> Outcome {
    for cmd in cmds {
        if let Err(reason) = cx.exec(cmd) {
            return Outcome::failed(reason);
        }
    }
    done(cx)
}

/// SUCCESS, or WOULD RUN in a dry run.
fn done(cx: &Ctx) -> Outcome {
    if cx.dry {
        Outcome::new(Status::Would, None)
    } else {
        Outcome::success()
    }
}

/// Pass, or FAILED with `msg`; in a dry run a failed check only needs attention.
fn require(cx: &Ctx, ok: bool, msg: &str) -> Outcome {
    match (ok, cx.dry) {
        (true, _) => Outcome::pass(),
        (false, false) => Outcome::failed(msg),
        (false, true) => Outcome::attention(msg),
    }
}

// --- preflight ---

pub fn preflight(s: &mut Vec<Step>, dry: bool) {
    let g = GROUP_PREFLIGHT;
    s.push(
        Step::new(g, "Running on CachyOS", |cx| {
            let ok = std::fs::read_to_string("/etc/os-release")
                .is_ok_and(|t| t.lines().any(|l| l == "ID=cachyos"));
            require(cx, ok, "this tool targets CachyOS only")
        })
        .required(),
    );
    s.push(
        Step::new(g, "Internet connection", |cx| {
            let ok = cx
                .query(
                    "curl",
                    &[
                        "-fsS",
                        "--max-time",
                        "8",
                        "-o",
                        "/dev/null",
                        "https://github.com",
                    ],
                )
                .is_some();
            require(cx, ok, "cannot reach github.com")
        })
        .required(),
    );
    let sudo = Step::new(g, "Sudo access", |cx| {
        let ok = cx.query("sudo", &["-n", "true"]).is_some();
        require(
            cx,
            ok,
            "no sudo ticket: restart cachyup and enter your password",
        )
    });
    s.push(if dry { sudo } else { sudo.required() });
    s.push(
        Step::new(g, "Package database free", |cx| {
            let ok = !Path::new(PACMAN_LOCK).exists();
            require(
                cx,
                ok,
                "pacman is already running (or remove a stale /var/lib/pacman/db.lck)",
            )
        })
        .required(),
    );
    s.push(Step::new(g, "~/.local/bin on PATH", |cx| {
        let bin = cx.paths.home.join(".local/bin");
        let ok =
            std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == bin));
        if ok {
            Outcome::pass()
        } else {
            Outcome::attention("add ~/.local/bin to PATH so the cachyup command is found")
        }
    }));
}

// --- system update ---

pub fn system_update(s: &mut Vec<Step>, config: &Config) {
    let g = GROUP_UPDATE;
    if config.step_enabled("rate_mirrors") {
        s.push(Step::new(g, "Rate mirrors", |cx| {
            if !has_cmd("cachyos-rate-mirrors") {
                return Outcome::skipped("cachyos-rate-mirrors not installed");
            }
            let cooldown = cx.settings.rate_mirrors_cooldown_days as u64;
            if let Some(age) = age_days(Path::new(MIRRORLIST))
                && cooldown > 0
                && age < cooldown
            {
                return Outcome::skipped(&format!("rated {age}d ago, cooldown {cooldown}d"));
            }
            act(cx, &[Cmd::sudo("cachyos-rate-mirrors", &[])])
        }));
    }
    if config.step_enabled("keyrings") {
        s.push(Step::new(g, "Keyrings", |cx| {
            act(
                cx,
                &[Cmd::sudo(
                    "pacman",
                    &[
                        "-Sy",
                        "--needed",
                        "--noconfirm",
                        "archlinux-keyring",
                        "cachyos-keyring",
                    ],
                )],
            )
        }));
    }
    if config.step_enabled("pacman") {
        s.push(Step::new(g, "pacman -Syu", |cx| {
            act(cx, &[Cmd::sudo("pacman", &["-Syu", "--noconfirm"])])
        }));
    }
    if config.step_enabled("aur") {
        s.push(Step::new(g, "yay -Sua", |cx| {
            if !has_cmd("yay") {
                return Outcome::skipped("yay not installed");
            }
            act(cx, &[yay(&["-Sua"])])
        }));
    }
    if config.step_enabled("flatpak") {
        s.push(Step::new(g, "flatpak update", |cx| {
            if !has_cmd("flatpak") {
                return Outcome::skipped("flatpak not installed");
            }
            act(
                cx,
                &[Cmd::new("flatpak", &["update", "-y", "--noninteractive"])],
            )
        }));
    }
    // checkrebuild runs ldd over every AUR package and takes minutes. Start it
    // now; the post-update checks collect the result.
    s.push(
        Step::new(g, "start checkrebuild", |cx| {
            if has_cmd("checkrebuild") {
                cx.rebuild = Some(std::thread::spawn(|| capture_any("checkrebuild", &[])));
            }
            Outcome::pass()
        })
        .hidden(),
    );
}

/// yay without prompts. `--sudoflags -n` makes yay's own sudo calls fail
/// rather than ask for a password over the TUI.
fn yay(args: &[&str]) -> Cmd {
    Cmd::new("yay", args).args([
        "--noconfirm",
        "--answerdiff",
        "None",
        "--answerclean",
        "None",
        "--removemake",
        "--sudoflags",
        "-n",
    ])
}

fn age_days(path: &Path) -> Option<u64> {
    let modified = path.metadata().ok()?.modified().ok()?;
    let age = SystemTime::now().duration_since(modified).ok()?;
    Some(age.as_secs() / 86_400)
}

// --- packages ---

pub fn prerequisites(s: &mut Vec<Step>, names: &[String]) {
    for name in names {
        let name = name.clone();
        s.push(Step::new(GROUP_PREREQ, name.clone(), move |cx| {
            install_package(cx, Source::Pacman, &name, None)
        }));
    }
}

pub fn apps(s: &mut Vec<Step>, source: Source, apps: Vec<App>) {
    let g = source.heading();
    if source == Source::Flatpak {
        s.push(Step::new(g, "Flathub remote", |cx| {
            if !has_cmd("flatpak") {
                return Outcome::failed("flatpak not installed");
            }
            let present = cx
                .query("flatpak", &["remotes", "--columns=name"])
                .is_some_and(|o| o.lines().any(|l| l.trim() == "flathub"));
            if present {
                return Outcome::skipped("already present");
            }
            act(
                cx,
                &[Cmd::new(
                    "flatpak",
                    &["remote-add", "--if-not-exists", "flathub", FLATHUB_URL],
                )],
            )
        }));
    }
    for app in apps {
        let label = app.display_name().to_string();
        s.push(Step::new(g, label, move |cx| match source {
            Source::Installer => run_installer(cx, &app),
            _ => install_package(cx, source, &app.name, Some(app.key())),
        }));
    }
}

/// Whether a package or flatpak is installed right now.
fn package_installed(cx: &Ctx, source: Source, name: &str) -> bool {
    match source {
        Source::Pacman | Source::Aur => cx.query("pacman", &["-Qq", name]).is_some(),
        Source::Flatpak => has_cmd("flatpak") && cx.query("flatpak", &["info", name]).is_some(),
        Source::Installer => false,
    }
}

/// Whether the source has it. `None` when that could not be checked; the
/// install is tried anyway and its own error explains.
fn available(cx: &mut Ctx, source: Source, name: &str) -> Option<bool> {
    match source {
        Source::Pacman => {
            let set = cx.repo_available.get_or_insert_with(|| {
                crate::system::capture("pacman", &["-Slq"]).map(|o| lines(&o))
            });
            set.as_ref().map(|s| s.contains(name))
        }
        Source::Aur => {
            let url = format!("https://aur.archlinux.org/rpc/v5/info?arg[]={name}");
            let out = cx.query("curl", &["-fsS", "--max-time", "15", &url])?;
            crate::system::parse_aur_info(&out).map(|r| r.iter().any(|r| r.name == name))
        }
        Source::Flatpak => {
            let set = cx.flathub_available.get_or_insert_with(|| {
                crate::system::capture(
                    "flatpak",
                    &["remote-ls", "flathub", "--app", "--columns=application"],
                )
                .map(|o| lines(&o))
            });
            set.as_ref().map(|s| s.contains(name))
        }
        Source::Installer => Some(true),
    }
}

fn lines(out: &str) -> HashSet<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// Check, install one package on its own, check again. `key` is set for
/// catalog apps (tracked as managed), `None` for prerequisites.
fn install_package(cx: &mut Ctx, source: Source, name: &str, key: Option<String>) -> Outcome {
    if package_installed(cx, source, name) {
        cx.managed.extend(key);
        return Outcome::skipped("already present");
    }
    let tool = match source {
        Source::Aur => Some("yay"),
        Source::Flatpak => Some("flatpak"),
        _ => None,
    };
    if let Some(tool) = tool
        && !has_cmd(tool)
        && !cx.dry
    {
        return Outcome::failed(format!("{tool} not installed"));
    }
    if available(cx, source, name) == Some(false) {
        return Outcome::failed("not found");
    }
    cx.note(format!("installing: {name}"));
    let cmd = match source {
        Source::Pacman => Cmd::sudo("pacman", &["-S", "--needed", "--noconfirm", name]),
        Source::Aur => yay(&["-S", "--needed", name]),
        Source::Flatpak => Cmd::new(
            "flatpak",
            &["install", "-y", "--noninteractive", "flathub", name],
        ),
        Source::Installer => unreachable!("installers use run_installer"),
    };
    let result = cx.exec(&cmd);
    if cx.dry {
        return done(cx);
    }
    if package_installed(cx, source, name) {
        cx.managed.extend(key);
        Outcome::success()
    } else {
        Outcome::failed(result.err().unwrap_or_else(|| "see log".into()))
    }
}

fn run_installer(cx: &mut Ctx, app: &App) -> Outcome {
    let Some(installer) = &app.installer else {
        return Outcome::failed("no installer defined");
    };
    if installer_present(&installer.check, &cx.paths) {
        cx.managed.push(app.key());
        return Outcome::skipped("already present");
    }
    let result = cx.exec(&Cmd::new("bash", &["-c", &installer.install]));
    if cx.dry {
        return done(cx);
    }
    if installer_present(&installer.check, &cx.paths) {
        cx.managed.push(app.key());
        Outcome::success()
    } else {
        Outcome::failed(
            result
                .err()
                .unwrap_or_else(|| format!("{} still missing", installer.check)),
        )
    }
}

// --- tweaks ---

/// `(tweak, its app, whether this run installs that app first)`.
pub fn tweaks(s: &mut Vec<Step>, tweaks: Vec<(Tweak, Option<App>, bool)>) {
    for (tweak, app, app_queued) in tweaks {
        let label = match &app {
            Some(a) => format!("{}: {}", a.display_name(), tweak.name),
            None => tweak.name.clone(),
        };
        s.push(Step::new(GROUP_TWEAKS, label, move |cx| {
            apply_tweak(cx, &tweak, app.as_ref(), app_queued)
        }));
    }
}

fn apply_tweak(cx: &mut Ctx, tweak: &Tweak, app: Option<&App>, app_queued: bool) -> Outcome {
    if let Some(app) = app {
        let installed = match app.source {
            Source::Installer => app
                .installer
                .as_ref()
                .is_some_and(|i| installer_present(&i.check, &cx.paths)),
            source => package_installed(cx, source, &app.name),
        };
        if !installed {
            if cx.dry && app_queued {
                return Outcome::new(
                    Status::Would,
                    Some(format!("after installing {}", app.display_name())),
                );
            }
            return Outcome::skipped(&format!("{} not installed", app.display_name()));
        }
    }
    let paths = cx.paths.clone();
    if tweak.sets.iter().all(|op| op_applied(op, &paths)) {
        return Outcome::skipped("already applied");
    }
    for op in &tweak.sets {
        if op_applied(op, &paths) {
            continue;
        }
        let what = match op {
            SetOp::Json { file, key, value } => format!("set {key} = {value} in {file}"),
            SetOp::Ini {
                file,
                section,
                key,
                value,
            } => format!("set [{section}] {key}={value} in {file}"),
        };
        if let Err(reason) = cx.mutate(&what, || apply_op(op, &paths)) {
            return Outcome::failed(format!("{what}: {reason}"));
        }
    }
    if cx.dry {
        return done(cx);
    }
    if tweak.sets.iter().all(|op| op_applied(op, &paths)) {
        Outcome::success()
    } else {
        Outcome::failed("value did not stick")
    }
}

fn apply_op(op: &SetOp, paths: &crate::paths::Paths) -> anyhow::Result<()> {
    match op {
        SetOp::Json { file, key, value } => json_set(&paths.expand(file), key, value),
        SetOp::Ini {
            file,
            section,
            key,
            value,
        } => ini_set(&paths.expand(file), section, key, value),
    }
}

pub fn op_applied(op: &SetOp, paths: &crate::paths::Paths) -> bool {
    match op {
        SetOp::Json { file, key, value } => {
            json_get(&paths.expand(file), key).as_ref() == Some(value)
        }
        SetOp::Ini {
            file,
            section,
            key,
            value,
        } => ini_get(&paths.expand(file), section, key).as_deref() == Some(value.as_str()),
    }
}

// --- cleanup ---

pub fn cleanup(s: &mut Vec<Step>, config: &Config) {
    let g = GROUP_CLEANUP;
    if config.step_enabled("orphans") {
        s.push(Step::new(g, "Orphaned packages", |cx| {
            // -Qtdq exits 1 when there are none.
            let orphans: Vec<String> = cx
                .query("pacman", &["-Qtdq"])
                .map(|o| {
                    o.lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            if orphans.is_empty() {
                return Outcome::skipped("none");
            }
            act(
                cx,
                &[Cmd::sudo("pacman", &["-Rns", "--noconfirm"]).args(orphans)],
            )
        }));
    }
    if config.step_enabled("package_cache") {
        let keep = config.settings.package_cache_keep;
        s.push(Step::new(g, "Package cache", move |cx| {
            if !has_cmd("paccache") {
                return Outcome::skipped("paccache not installed");
            }
            // Keep N versions of installed packages, none of uninstalled ones.
            let installed = format!("-dk{keep}");
            let candidates: u64 = [installed.as_str(), "-duk0"]
                .iter()
                .filter_map(|flag| capture_any("paccache", &[flag]))
                .map(|o| paccache_candidates(&o))
                .sum();
            if candidates == 0 {
                return Outcome::skipped("nothing to prune");
            }
            act(
                cx,
                &[
                    Cmd::sudo("paccache", &[&format!("-rk{keep}")]),
                    Cmd::sudo("paccache", &["-ruk0"]),
                ],
            )
        }));
    }
    if config.step_enabled("yay_cache") {
        let days = config.settings.yay_cache_keep_days;
        s.push(Step::new(
            g,
            format!("yay build cache (older than {days}d)"),
            move |cx| prune_yay_cache(cx, days),
        ));
    }
    if config.step_enabled("flatpak_unused") {
        s.push(Step::new(g, "Unused flatpak runtimes", |cx| {
            if !has_cmd("flatpak") {
                return Outcome::skipped("flatpak not installed");
            }
            // No dry run in flatpak; answering "n" lists what it would remove.
            let out = cx
                .query_with_input("flatpak", &["uninstall", "--unused"], "n\n")
                .unwrap_or_default();
            if out.contains("Nothing unused") {
                return Outcome::skipped("none");
            }
            act(
                cx,
                &[Cmd::new(
                    "flatpak",
                    &["uninstall", "--unused", "-y", "--noninteractive"],
                )],
            )
        }));
    }
    if config.step_enabled("mirrorlist_pacnew") {
        s.push(Step::new(g, "Rated mirrorlist .pacnew files", |cx| {
            let files = rated_mirrorlist_pacnews();
            if files.is_empty() {
                return Outcome::skipped("none");
            }
            for f in &files {
                cx.line(format!("dropping {f}: live file is rate-mirrors output"));
                if let Err(reason) = cx.exec(&Cmd::sudo("rm", &["--", f])) {
                    return Outcome::failed(reason);
                }
            }
            done(cx)
        }));
    }
}

/// Sum of "N candidates" in `paccache -d` output.
fn paccache_candidates(out: &str) -> u64 {
    out.lines()
        .filter_map(|l| {
            let idx = l.find(" candidate")?;
            l[..idx]
                .rsplit(|c: char| !c.is_ascii_digit())
                .next()?
                .parse::<u64>()
                .ok()
        })
        .sum()
}

/// A .pacnew for a mirrorlist is only dropped when the live file carries the
/// rate-mirrors header: proof it is generated, not hand-edited.
fn rated_mirrorlist_pacnews() -> Vec<String> {
    if !has_cmd("cachyos-rate-mirrors") {
        return Vec::new();
    }
    RATED_MIRRORLISTS
        .iter()
        .filter(|f| Path::new(&format!("{f}.pacnew")).exists())
        .filter(|f| {
            std::fs::read_to_string(f)
                .is_ok_and(|t| t.lines().any(|l| l.starts_with("# ARGS: rate-mirrors ")))
        })
        .map(|f| format!("{f}.pacnew"))
        .collect()
}

/// yay keeps a git clone per AUR package. Prune whole package dirs, never
/// single files: a dir goes when nothing in it was touched for `days`, or when
/// its clone is already broken (yay's next fetch of it would fail).
fn prune_yay_cache(cx: &mut Ctx, days: u32) -> Outcome {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| cx.paths.home.join(".cache"))
        .join("yay");
    if !cache.is_dir() {
        return Outcome::skipped("empty");
    }
    let max_age = Duration::from_secs(days as u64 * 86_400);
    let git = has_cmd("git");
    let mut stale: Vec<(PathBuf, bool)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&cache) else {
        return Outcome::skipped("empty");
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for dir in dirs {
        if git && yay_dir_broken(&dir) {
            stale.push((dir, true));
        } else if !touched_within(&dir, max_age) {
            stale.push((dir, false));
        }
    }
    if stale.is_empty() {
        return Outcome::skipped(&format!("nothing older than {days}d"));
    }
    let size = |cx: &Ctx| {
        cx.query("du", &["-sh", &cache.to_string_lossy()])
            .and_then(|o| o.split_whitespace().next().map(String::from))
            .unwrap_or_default()
    };
    let before = size(cx);
    for (dir, broken) in &stale {
        let name = dir.file_name().unwrap_or_default().to_string_lossy();
        let what = if *broken {
            format!("remove broken clone: {name}")
        } else {
            format!("remove stale: {name}")
        };
        let dir = dir.clone();
        if let Err(reason) = cx.mutate(&what, || Ok(std::fs::remove_dir_all(&dir)?)) {
            return Outcome::failed(reason);
        }
    }
    if !cx.dry {
        cx.line(format!("yay cache: {before} -> {}", size(cx)));
    }
    done(cx)
}

/// Is `dir` itself a usable git repo? A gutted repo is not recognised by git,
/// which then walks up and reports an enclosing one, so compare paths.
fn git_repo_is(dir: &Path) -> bool {
    let Ok(real) = dir.canonicalize() else {
        return false;
    };
    let real = real.to_string_lossy().to_string();
    let d = dir.to_string_lossy();
    let found = |args: &[&str]| -> Option<String> {
        let mut all = vec!["-C", &d];
        all.extend_from_slice(args);
        crate::system::capture("git", &all).map(|o| o.trim().to_string())
    };
    found(&["rev-parse", "--show-toplevel"]).is_some_and(|f| f == real)
        || found(&["rev-parse", "--absolute-git-dir"]).is_some_and(|f| f == real)
}

/// The AUR clone is gone, or a bare mirror of a VCS source inside it is
/// (makepkg then stops with "<dir> is not a clone of <url>").
fn yay_dir_broken(dir: &Path) -> bool {
    if !git_repo_is(dir) {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("config").is_file() && p.join("description").is_file())
        .any(|sub| !git_repo_is(&sub))
}

/// Anything under `dir` (itself included) modified within `max_age`.
fn touched_within(dir: &Path, max_age: Duration) -> bool {
    let recent = |p: &Path| {
        p.symlink_metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age < max_age)
    };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if recent(&d) {
            return true;
        }
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.filter_map(|e| e.ok()) {
            let p = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(p);
            } else if recent(&p) {
                return true;
            }
        }
    }
    false
}

// --- post-update checks (report only, never act) ---

pub fn post_checks(s: &mut Vec<Step>) {
    let g = GROUP_CHECKS;
    s.push(Step::new(g, ".pacnew files", |cx| {
        if !has_cmd("pacdiff") {
            return Outcome::attention("pacdiff not installed");
        }
        let files: Vec<String> = cx
            .query("pacdiff", &["-o"])
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();
        if files.is_empty() {
            return Outcome::pass();
        }
        for f in &files {
            cx.note(f.clone());
        }
        Outcome::attention(format!(
            "{} to merge, run: DIFFPROG=meld pacdiff --sudo",
            files.len()
        ))
    }));
    s.push(Step::new(g, "Reboot not required", |cx| {
        // The running kernel's modules dir goes when its package is upgraded.
        // kernel-modules-hook keeps it, so then this cannot tell.
        if cx
            .query("pacman", &["-Qq", "kernel-modules-hook"])
            .is_some()
        {
            return Outcome::pass();
        }
        let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
        if Path::new("/usr/lib/modules").join(release.trim()).is_dir() {
            Outcome::pass()
        } else {
            Outcome::attention("kernel was updated, reboot when convenient")
        }
    }));
    s.push(Step::new(g, "No services need a restart", |cx| {
        if !has_cmd("checkservices") {
            return Outcome::attention("checkservices not installed");
        }
        // -F -P -R: no failed-service list, no pacdiff, no restarting.
        let out = cx
            .query_with_input("sudo", &["-n", "checkservices", "-F", "-P", "-R"], "")
            .unwrap_or_default();
        if out.contains("password is required") {
            return Outcome::attention("needs sudo");
        }
        let services = restart_services(&out);
        if services.is_empty() {
            Outcome::pass()
        } else {
            Outcome::attention(format!(
                "restart with: sudo systemctl restart {}",
                services.join(" ")
            ))
        }
    }));
    s.push(Step::new(
        g,
        "AUR packages up to date with libraries",
        |cx| {
            let Some(handle) = cx.rebuild.take() else {
                return if has_cmd("checkrebuild") {
                    // Not started (update steps off): run it now.
                    rebuild_outcome(cx, capture_any("checkrebuild", &[]))
                } else {
                    Outcome::attention("checkrebuild not installed")
                };
            };
            if !handle.is_finished() {
                cx.note("waiting for checkrebuild to finish");
            }
            let out = handle.join().ok().flatten();
            rebuild_outcome(cx, out)
        },
    ));
}

fn restart_services(out: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = out;
    while let Some(i) = rest.find("restart '") {
        rest = &rest[i + "restart '".len()..];
        if let Some(end) = rest.find('\'') {
            found.push(rest[..end].to_string());
            rest = &rest[end..];
        }
    }
    found
}

fn rebuild_outcome(cx: &mut Ctx, out: Option<String>) -> Outcome {
    let Some(out) = out else {
        return Outcome::attention("checkrebuild failed");
    };
    let flagged: Vec<&str> = out
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .collect();
    let (prebuilt, rebuild): (Vec<&str>, Vec<&str>) =
        flagged.into_iter().partition(|p| is_prebuilt(p));
    if !prebuilt.is_empty() {
        cx.note(format!(
            "flagged but prebuilt, a rebuild cannot help, wait for upstream: {}",
            prebuilt.join(" ")
        ));
    }
    if rebuild.is_empty() {
        Outcome::pass()
    } else {
        Outcome::attention(format!(
            "rebuild with: yay -S --rebuild {}",
            rebuild.join(" ")
        ))
    }
}

fn is_prebuilt(name: &str) -> bool {
    PREBUILT_PATTERNS
        .iter()
        .any(|pat| match (pat.strip_prefix('*'), pat.strip_suffix('*')) {
            (Some(suffix), _) => name.ends_with(suffix),
            (_, Some(prefix)) => name.starts_with(prefix),
            _ => name == *pat,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_paccache_dry_runs() {
        let out = "==> finished dry run: 12 candidates (disk space saved: 1.2 GiB)\n";
        assert_eq!(paccache_candidates(out), 12);
        assert_eq!(
            paccache_candidates("==> no candidate packages found for pruning\n"),
            0
        );
    }

    #[test]
    fn finds_services_to_restart() {
        let out =
            "Found 2 services:\n  ssh: run restart 'sshd.service'\n  restart 'cups.service'\n";
        assert_eq!(restart_services(out), ["sshd.service", "cups.service"]);
    }

    #[test]
    fn prebuilt_patterns_match() {
        assert!(is_prebuilt("visual-studio-code-bin"));
        assert!(is_prebuilt("pycharm-professional"));
        assert!(is_prebuilt("spotify"));
        assert!(!is_prebuilt("openttd-jgrpp"));
    }

    #[test]
    fn stale_dirs_by_age() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), "x").unwrap();
        assert!(touched_within(dir.path(), Duration::from_secs(3600)));
        assert!(!touched_within(dir.path(), Duration::ZERO));
    }
}
