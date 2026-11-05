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
- Dotfiles screen (`D`) for a GNU Stow folder, asked for the first time it opens: each
  package with how it is linked (one folder link or single-file links), partial, used by
  another package, blocked or not linked, every link, its `.gitignore` rules and what they
  ignore, what `stow --simulate` would do, and warnings for clashing packages, real files
  in the way, and git-ignored files (including private keys) kept inside the repo by a
  folder link. Read-only, and only opens when stow is installed.

### Changed

- AUR packages renamed for Dux Shell 2.1: `dankcalendar-bin` is now `dux-calendar-bin`,
  `dsearch-bin` is now `dux-search-bin`.

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
