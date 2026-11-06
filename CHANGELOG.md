# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Tweaks can install optional packages with `installs` (bare names are pacman, or
  `aur/name`, `flatpak/id`), on their own or before their `set` values.
- Dux Files (AUR) with two tweaks: its optional extras (thumbnails, previews, mounts,
  shares) and setting it as the default file manager.
- Photocraft (AUR).
- Lerd (installer), with `lerd update` for the installer updates step.
- Sushi is now a tweak under Nautilus ("Quick file preview"), as the add-on it is, rather
  than a package of its own.
- Tweaks can run a shell command with `run`, guarded by a `check` command whose exit 0
  means it is done. For settings no file write reaches (`xdg-mime`, `gsettings`,
  `systemctl --user`). Runs after `installs` and `set`, never with sudo.
- README reference for every key in `user-apps.toml`.
- Installer updates: an update step, off by default, that runs each selected installer
  app's optional `update` command (`claude update` for Claude Code), one step per app,
  as the user. Install scripts are never re-run.

- `install.sh --apps URL|path` to start from your own apps file.
- `cachyup update` re-runs the install script over the running binary. It refuses when
  the binary is owned by a package (`pacman -Qo`), lives outside your home, or runs under
  cargo. cachyup itself is in the app list as an installer with that update command.
- `A` in the app list switches everything shown on, or off again.
- The app list says "Installed (will update)" when the full system update as configured
  would upgrade that app: the step for its source on, and for installers the Installer
  updates step, an `update` command and the app selected.
- A Settings box under the tasks: General (the cleanup numbers, moved off the update page,
  plus interface toggles and Hide credits), open the app list in your editor, add an app by
  name, review new installs and review new suggestions, with counts. Two reviews: apps installed on this machine that the list
  lacks (explicitly installed packages and flatpak apps; kernels, drivers, firmware and
  base-system packages set aside behind `s`), and entries new in cachyup. Tick, then `w`
  merges them into `apps.toml` with its layout kept (previous copy in `apps.toml.bak`)
  and switches them on; `Esc` discards. Reviewed entries are not offered again. A launch
  popup counts what is waiting. Each entry shows its description, `i` its details.
- `u` in the app list uninstalls an app: a confirmation shows the command (`pacman -Rns
  --noconfirm` or `flatpak uninstall`, `c` copies it), then it runs as a one-step run and
  the app is switched off so the next run does not reinstall it. Install-script apps
  take an optional `uninstall` command in their entry (set for cachyup and Claude Code).
- `f` in the app list groups apps by category (Developer Tools, Gaming, Internet,
  Communication, Media, Graphics, Office, Desktop, System) instead of by package source.
  The map is built in (`categories.toml`); unknown apps show under Other. Remembered.
- The header is a box of its own with the project link on its border (Hide credits in
  General removes it) and notices on the right: "Please review new apps" and a newer
  release when there is one.
- Ctrl-Up and Ctrl-Down jump to the start and end of a list, like Home and End.
- `t` and `s` jump to the Tasks and Settings boxes from anywhere on the main screen.
- Install Apps and Install Tweaks are one task, "Install Apps & Tweaks", with one switch.
  Tweaks sit under their app as before; tweaks without an app get an "Additional tweaks"
  group at the end of the list. A `tweaks` line in `[tasks]` of config.toml is ignored.
- "Add app" checks the package exists (pacman -Si, the AUR, Flathub) before adding it.
- Every list is sorted by name within its group (apps, tweaks, review entries), whatever
  order the file has. An added entry also goes into `apps.toml` in A-Z position when the
  list there is already in order.
- The help popup (`?`) names your files; `e` there opens `apps.toml` in `$VISUAL`,
  `$EDITOR` or `xdg-open`, and the list is re-read afterwards.

### Changed

- The app list is now your own file: `~/.config/cachyup/apps.toml`, created from
  cachyup's list (`apps.toml` in the repo, formerly `catalog.toml`) on the first launch,
  comments included. Edit it freely; cachyup's built-in copy only supplies the
  prerequisites and the "new in cachyup" notice for entries your file lacks.
- Everything starts switched off, including on the first launch. `default = true` opts an
  entry on.
- `selection.toml` folded into `config.toml` as `[apps]` and `[tweaks]`, so one file holds
  every setting. `user-apps.toml` is merged into `apps.toml`. Both old files are renamed
  `.migrated` at the first launch.
- AUR packages renamed for Dux Shell 2.1: `dankcalendar-bin` is now `dux-calendar-bin`,
  `dsearch-bin` is now `dux-search-bin`.

### Fixed

- An app uninstalled by hand is noticed at the next launch even when no real run ever
  touched it: every selected app found installed at launch now counts as managed.

### Removed

- `user-apps.toml` and `selection.toml` (see above).
- The blue `new` badge: entries now enter your list only through a review or by hand, so
  there is nothing to badge.

## [0.1.0] - 2026-10-23

The bash script rewritten as a Rust terminal UI.

### Added

- Terminal UI: tasks (Full System Update, Install Apps, Install Tweaks) on the left, their
  options on the right. Every update and cleanup step can be switched on or off.
- One combined app list (`catalog.toml`) for pacman, AUR, flatpak and install scripts, with
  each app's tweaks listed under it, a status column, package descriptions read from the
  sources, an info popup (`i`), and a description line under every entry (`I`,
  remembered between launches).
- `~/.config/cachyup/user-apps.toml` for your own apps and tweaks, with duplicate detection.
- Apps added in a cachyup update start off and are highlighted; apps you uninstall are
  switched off at the next launch, with a popup for each.
- Turning a tweak on also turns on its app when that app is not installed.
- Dry run (`d` in the run dialog, or `--dry-run`): every check runs, nothing changes.
- Run screen with live progress and full output; history screen for past runs.
- `--run` to run the saved settings without the interface, `--reset` to start over.
- Update check at launch; `c` copies the install command.
- `c` in the end-of-run summary copies the suggested commands (`pacdiff`, `systemctl
  restart`, `yay --rebuild`) to the clipboard.
- Mouse support (click, wheel); popups release the mouse so their text can be selected.
- Installed from a prebuilt binary (`install.sh`), like kiln.

### Changed

- Settings moved from `~/.config/cachyup/apps` to `config.toml` and `selection.toml`.
- Logs moved to `~/.local/state/cachyup/runs/<date>/`.
- Post-update findings are `ATTENTION` instead of `FAILED`, so the failure count only
  counts real failures. The checks have neutral names (`Reboot check` instead of `Reboot
  not required`), so a finding never reads as the opposite of what it means.

[Unreleased]: https://github.com/ReyArlena/cachyup/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ReyArlena/cachyup/releases/tag/v0.1.0
