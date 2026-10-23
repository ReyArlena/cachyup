# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0]

The bash script rewritten as a Rust terminal UI.

### Added

- Terminal UI: tasks (Full System Update, Install Apps, Install Tweaks) on the left, their
  options on the right. Every update and cleanup step can be switched on or off.
- One combined app list (`catalog.toml`) for pacman, AUR, flatpak and install scripts, with
  each app's tweaks listed under it, a status column, package descriptions read from the
  sources, and an info popup.
- `~/.config/cachyup/user-apps.toml` for your own apps and tweaks, with duplicate detection.
- Apps added in a cachyup update start off and are highlighted; apps you uninstall are
  switched off at the next launch, with a popup for each.
- Dry run (`d` in the run dialog, or `--dry-run`): every check runs, nothing changes.
- Run screen with live progress and full output; history screen for past runs.
- `--run` to run the saved settings without the interface, `--reset` to start over.
- Update check at launch.
- Installed from a prebuilt binary (`install.sh`), like kiln.

### Changed

- Settings moved from `~/.config/cachyup/apps` to `config.toml` and `selection.toml`.
- Logs moved to `~/.local/state/cachyup/runs/<date>/`.
- Post-update findings are `ATTENTION` instead of `FAILED`, so the failure count only
  counts real failures.

[Unreleased]: https://github.com/ReyArlena/cachyup/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ReyArlena/cachyup/releases/tag/v0.1.0
