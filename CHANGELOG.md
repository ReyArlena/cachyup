# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- fd (pacman).

## [0.2.0] - 2026-11-06

The app list becomes your own file, with reviews to grow it, and the interface gets a
Settings box, categories, uninstall, and a cleaner look. Config files from 0.1.0 are
migrated at the first launch.

### Added

- Settings box under the tasks: General (the cleanup numbers, moved off the update page,
  plus toggles for descriptions, grouping by category and hiding the credits), open the
  app list in your editor, add an app by name (checked against pacman, the AUR or Flathub
  first), review new installs and review new suggestions, with counts.
- Reviews: a tick list of what could join your app list. Installed apps the list lacks
  (explicitly installed packages and flatpak apps; kernels, drivers, firmware and
  base-system packages set aside behind `s`), and entries cachyup ships that your file
  lacks. Each entry shows its description, `i` its details. `w` merges the ticked ones
  into `apps.toml` with its layout kept (previous copy in `apps.toml.bak`) and switches
  them on; `Esc` discards. Reviewed entries are not offered again. The launch popup and
  the header say when something is waiting.
- `u` in the app list uninstalls the app under the cursor: a confirmation shows the exact
  command (`pacman -Rns --noconfirm` or `flatpak uninstall`, `c` copies it), it runs as a
  one-step run, and the app is switched off so the next run does not reinstall it.
  Install-script apps take an optional `uninstall` command (set for cachyup and Claude
  Code). On an Unavailable entry, `u` offers to remove its line from `apps.toml`.
- `f` groups the app list by category (Developer Tools, Gaming, Internet, Communication,
  Media, Graphics, Office, Desktop, System) instead of by package source. The map is
  built in (`categories.toml`); unknown apps show under Other. Remembered.
- `A` selects everything shown in the app list, or clears it; `a` does a group.
- "Installed (will update)" in the status column when the full system update as
  configured would upgrade that app.
- Installer updates: an update step, off by default, that runs each selected installer
  app's optional `update` command (`claude update`, `lerd update`, `cachyup update`), one
  step per app, as the user. Install scripts are never re-run.
- `cachyup update` re-runs the install script over the running binary. It refuses when
  the binary is owned by a package (`pacman -Qo`), lives outside your home, or is a cargo
  build. cachyup itself is in the app list as an installer.
- `install.sh --apps URL|path` to start from your own apps file.
- Tweaks can install optional packages with `installs` (bare names are pacman, or
  `aur/name`, `flatpak/id`), and run a shell command with `run` guarded by a `check`
  whose exit 0 means it is done, for settings no file write reaches (`xdg-mime`,
  `gsettings`, `systemctl --user`). Order: `installs`, `set`, `run`. Never with sudo.
- Header box with the project link on its border and notices on the right ("Please
  review new apps", a newer release). Key hints in their own box that wraps on narrow
  terminals. Help popup (`?`) names your files; `e` there opens `apps.toml` in your
  editor and the list is re-read afterwards.
- Keys: `t` and `s` jump to the Tasks and Settings boxes; Ctrl-Up and Ctrl-Down jump to
  the start and end of a list.
- Apps: Dux Files (AUR) with two tweaks (optional extras, default file manager),
  Photocraft (AUR), Lerd (installer), cachyup (installer).
- README reference for every key in `apps.toml`.

### Changed

- The app list is your own file: `~/.config/cachyup/apps.toml`, created from cachyup's
  list (`apps.toml` in the repo, formerly `catalog.toml`) on the first launch, comments
  included. Edit it freely; the built-in copy only supplies the prerequisites and the
  suggestions review. An app uninstalled by hand is noticed at the next launch even when
  no run ever touched it: every switched-on app found installed at launch is tracked.
- Everything starts switched off, including on the first launch. `default = true` opts an
  entry on.
- `selection.toml` folded into `config.toml` as `[apps]` and `[tweaks]`, so one file holds
  every setting. `user-apps.toml` is merged into `apps.toml`. Both old files are renamed
  `.migrated` at the first launch.
- Install Apps and Install Tweaks are one task, "Install Apps & Tweaks", with one switch.
  Tweaks sit under their app; tweaks without an app get an "Additional tweaks" group at
  the end. A `tweaks` line under `[tasks]` in config.toml is ignored.
- Every list is sorted A-Z within its group, whatever order the file has. An added entry
  goes into `apps.toml` in A-Z position when the list there is already in order.
- Look: a quiet grey cursor row instead of reversed video, secondary text in the
  terminal's faint rendering, dotted leaders between names and their status, padding
  inside highlights and before the right border, `v` before the version.
- Sushi is a tweak under Nautilus ("Quick file preview") rather than a package of its own.
- AUR packages renamed for Dux Shell 2.1: `dankcalendar-bin` is now `dux-calendar-bin`,
  `dsearch-bin` is now `dux-search-bin`.

### Removed

- `user-apps.toml` and `selection.toml` (migrated, see above).
- The blue `new` badge: entries enter your list only through a review or by hand.
- The separate Install Tweaks task and pane.

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

[Unreleased]: https://github.com/ReyArlena/cachyup/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/ReyArlena/cachyup/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ReyArlena/cachyup/releases/tag/v0.1.0
