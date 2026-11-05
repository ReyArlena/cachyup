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
- Tweaks can run a shell command with `run`, guarded by a `check` command whose exit 0
  means it is done. For settings no file write reaches (`xdg-mime`, `gsettings`,
  `systemctl --user`). Runs after `installs` and `set`, never with sudo.
- README reference for every key in `user-apps.toml`.
- Installer updates: an update step, off by default, that runs each selected installer
  app's optional `update` command (`claude update` for Claude Code), one step per app,
  as the user. Install scripts are never re-run.

- `install.sh --apps URL|path` to start from your own apps file.
- `A` in the app list switches everything shown on, or off again.

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

### Removed

- `user-apps.toml` and `selection.toml` (see above).

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
