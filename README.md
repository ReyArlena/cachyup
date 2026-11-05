# cachyup

One command to set up and update a CachyOS install: a full system update with cleanup,
your apps (pacman, AUR, flatpak, install scripts), config tweaks for those apps, and a
summary of anything that needs a human (`.pacnew` files, a pending reboot, services to
restart, AUR packages to rebuild).

It is a terminal UI. Pick what to run, press `r`, go and make tea. Every run is logged, and
the history screen shows past runs with their full output. Re-running is always safe: every
step checks the real end state first and skips what is already done.

## Install

```bash
curl -fsSL https://reyarlena.github.io/cachyup/install.sh | sh
```

This puts the `cachyup` binary in `~/.local/bin`. Run the same command again to update;
cachyup tells you at launch when a new release is out (`c` in that popup copies the
install command). Run as your normal user, not root.

Remove it with `sh install.sh --uninstall`, or `--purge` to also delete settings and
history.

## Using it

```bash
cachyup
```

At launch cachyup asks for your sudo password in the plain terminal (skipped if sudo
already has a ticket), then opens. The ticket is kept fresh in the background for the
whole session, so a long AUR build never stops at a password prompt.

```
┌ Tasks ──────────────────────┐┌ Install Apps options ─────────────────────────────────┐
│ ✓ Full System Update        ││ Pacman packages  101/102                              │
│   10 of 10 steps            ││ ✓ discord                              Installed      │
│                             ││     ✓ Disable host update              Applied        │
│ ✓ Install Apps              ││ ✓ docker                               Installed      │
│   136 selected, 2 to install││ ✗ junction (default off: dux-shell ... Not installed  │
│                             ││ ✓ zed                     new          Not installed  │
│ ✓ Install Tweaks            ││                                                       │
│   2 of 3 selected           ││ AUR packages  18/18                                   │
└─────────────────────────────┘└───────────────────────────────────────────────────────┘
 Space toggle  c options  r run  H history  ? help  q quit
```

The left pane lists the three tasks; `Space` switches one on or off. The right pane shows
the highlighted task's options. Press `c` (or `Enter`, `Tab`, `→`/`l`) to move into them,
and `Esc` (or `←`/`h`) to come back. Arrows and `h` `j` `k` `l` move everywhere, `?` lists
the keys for the current screen. Changes are saved as you make them.

| Task | Options |
|------|---------|
| Full System Update | Each step on or off: rate mirrors, keyrings, `pacman -Syu`, `yay -Sua`, `flatpak update`, and the cleanup steps. Numbers: mirror rating cooldown, package cache versions, yay cache age, runs kept in history. |
| Install Apps | Every app in the list, grouped by source, with a status column (Installed, Not installed, Unavailable). Tweaks for an app are listed under it. `i` shows the description and package details, `I` shows a description line under every entry (remembered), `/` filters, `a` switches a whole group. |
| Install Tweaks | The same tweaks as a flat list, with Applied / Not applied (`I` for descriptions). Toggling here and under the app is the same switch. Turning a tweak on also turns on its app if that app is not installed (a tweak is skipped while its app is missing). |

Ticks are green and switched on; crosses are grey and switched off. Blue entries with a
`new` badge were added in a cachyup update and start switched off. Names say what an entry
does; anything about its current state is in brackets after it, e.g. `Set as default link
handler (currently dms-open)` for a tweak not yet applied, or `(default off: dux-shell
installed)` for an app that is off by default on this machine.

`r` asks for confirmation: `Enter` runs, `d` does a dry run, `Esc` cancels. While running,
the left pane ticks off each step and the right pane shows the full terminal output.
Select a step on the left to jump to its output. `q` offers to stop after the current step
(safe) or right away. At the end a summary lists every failure and everything that needs
attention; `c` copies the commands it suggests (`pacdiff`, `systemctl restart`, `yay
--rebuild`) to the clipboard. Popups do not capture the mouse, so their text can also be
selected and copied as usual.

`H` opens the history: past runs on the left, the selected run's output on the right,
`x` deletes one.

### Dry run

A dry run changes nothing. Every check still runs for real (is it installed, is it
available, are there orphans, which `.pacnew` files exist), and every command that would
change something is logged as `would run: ...` with a `WOULD RUN` status. Use it to see what
a run will do. `cachyup --dry-run` opens the interface with real runs switched off.

### Without the interface

```bash
cachyup --run              # run the saved settings, print the output, exit
cachyup --run --dry-run    # same, changing nothing
```

For scripts and cron. The first `Ctrl-C` stops after the current step, the second stops
now. Exit code 0 means nothing failed, 1 means a step failed, 2 an error, 3 the run was
stopped or aborted early.

### Start over

```bash
cachyup --reset
```

Deletes every setting, all state and the run history (asks first, `--yes` to skip), so the
next launch behaves like a fresh install.

## What a run does

Only what is switched on, in this order:

| Group | Steps |
|-------|-------|
| Preflight | Running on CachyOS, internet, sudo, pacman not already running, `~/.local/bin` on PATH. A failure stops the run. |
| System update | Rate mirrors (skipped within the cooldown), keyrings, `pacman -Syu`, `yay -Sua`, `flatpak update`. |
| Prerequisites | Packages cachyup needs (`yay`, `base-devel`, `pacman-contrib`, ...). Always installed. |
| Pacman / AUR / Flatpaks / Installers | Each selected app on its own: checked, installed, checked again. |
| Tweaks | Each selected tweak, skipped while its app is not installed. |
| Cleanup | Orphans, package cache (keeps 2 versions of installed packages, none of uninstalled ones), yay build cache older than 30 days, unused flatpak runtimes, `.pacnew` files for mirrorlists cachyos-rate-mirrors generates. |
| Post-update checks | Report only: `.pacnew` check, reboot check, service restart check (services running replaced binaries), AUR rebuild check (packages built against removed libraries). `PASS` when all is well, `ATTENTION` with the command to run otherwise. |

Statuses: `SUCCESS` something was installed or changed, `SKIPPED` already in place or not
applicable, `FAILED` see the output, `PASS` a check came out fine, `ATTENTION` something for
you to do (the reason says what), `WOULD RUN` dry run.

Installing one package per call is a little slower than one big transaction, but every
package gets its own honest result and reason.

## Apps and tweaks

### Your selection

Every app and tweak is on or off, saved in `~/.config/cachyup/selection.toml`. Switching an
app off never uninstalls it.

- **First launch:** everything is on, except entries with a condition (`unless_cmd`).
  Junction (a link chooser) and its tweak are off when Dux Shell's `dms` command
  is installed, because Dux Shell has its own link chooser.
- **cachyup adds an app in an update:** a popup at launch lists the new entries. They start
  off, shown in blue with a `new` badge for that session. `Enter` jumps to the list to
  review them.
- **You uninstall an app cachyup installed:** at the next launch it is switched off, with a
  popup saying so, so it is not reinstalled behind your back. Switch it on to get it back.
- **An app is dropped from the list:** it is forgotten. If it comes back later it counts as
  new.

### Adding your own

Create `~/.config/cachyup/user-apps.toml`. Same layout as cachyup's own
[`catalog.toml`](catalog.toml). Everything in it is optional; a short file is fine:

```toml
pacman = ["htop", "btop"]
aur = ["pfetch"]
flatpak = ["org.gnome.Chess", { name = "com.example.App", description = "My app" }]

[[installer]]
name = "my-tool"
check = "~/.local/bin/my-tool"          # a path or a command on PATH; present = installed
install = "curl -fsSL https://example.com/install.sh | sh"

[[tweak]]
id = "htop-tree"
app = "pacman/htop"                      # shown under htop, skipped while it is missing
name = "Tree view"
set = [
    {
        ini = "~/.config/htop/htoprc",
        section = "x",
        key = "tree_view",
        value = 1,
    },
]
```

Your entries start switched on the first time they are seen. An entry that repeats one of
cachyup's own is ignored, and a popup at launch says so. So does a name listed under two
sources, a tweak whose `app` is not in the list, or a file that does not parse. The user
file only adds: it cannot change or hide a built-in entry, and `prerequisites` is ignored
in it.

#### Packages

Three lists, one per source. Each entry is a bare name or a table with extras:

```toml
pacman = ["htop", { name = "junction", unless_cmd = "dms" }]
aur = ["pfetch", { name = "foo-git", default = false, description = "Not for everyone" }]
flatpak = ["org.gnome.Chess"]
```

| Key | Meaning |
|-----|---------|
| `name` | Package name (`pacman`, `aur`) or application id (`flatpak`). Required. |
| `description` | One line under the entry. Otherwise read from the package source. |
| `default` | `true` or `false`: on or off the first time it is seen. Default on. |
| `unless_cmd` | Off by default while this command exists (`"dms"`, `"/usr/bin/foo"`). |

Pacman names are checked against the repos, AUR names against the AUR, flatpak ids
against Flathub. An unknown name shows as Unavailable and is skipped in a run.

#### Installers

Apps with their own install script. Each one is checked, installed, and checked again:

```toml
[[installer]]
name = "claude"
label = "Claude Code"
description = "Anthropic's coding agent (~/.local/bin/claude)"
check = "~/.local/bin/claude"
install = "curl -fsSL https://claude.ai/install.sh | bash"
```

| Key | Meaning |
|-----|---------|
| `name` | Short id, used in the selection file as `installer/name`. Required. |
| `label` | Display name when it should differ from `name`. |
| `description` | One line under the entry. |
| `check` | A path (`~` allowed) or a command name on PATH. Present means installed. Required. |
| `install` | Shell command run with `bash -c` as you, only while `check` fails. Required. |
| `default`, `unless_cmd` | As for packages. |

Installers never get sudo. A script that needs it fails rather than prompts.

#### Tweaks

A tweak is a small change tied to an app: optional packages, values written into config
files, a command, or any mix. It needs at least one of `installs`, `set`, or `run`. The
three happen in that order, and each part is skipped while it is already in place:

```toml
[[tweak]]
id = "dux-files-default"
app = "aur/dux-files-git"
name = "Set Dux Files as default"
description = "Make Dux Files the default file manager."
installs = ["gvfs", "aur/foo-git", "flatpak/org.example.App"]
set = [
    {
        ini = "~/.config/mimeapps.list",
        section = "Default Applications",
        key = "inode/directory",
        value = "dev.duxlabs.files.desktop",
    },
    {
        json = "~/.config/foo/settings.json",
        key = "ui.theme",
        value = "dark",
    },
]
check = "test \"$(xdg-mime query default inode/directory)\" = dev.duxlabs.files.desktop"
run = "xdg-mime default dev.duxlabs.files.desktop inode/directory"
```

| Key | Meaning |
|-----|---------|
| `id` | Unique id, used in the selection file. Required. |
| `name` | Shown in the list. Required. |
| `app` | Key of the app it belongs to: `pacman/htop`, `aur/foo`, `flatpak/org.x.App`, `installer/my-tool`. Shown under that app and skipped while it is not installed. Without it the tweak stands alone. |
| `description` | One line under the entry. |
| `installs` | Packages installed first. Bare names are pacman; `aur/name` and `flatpak/id` for the others. |
| `set` | Values written into config files, each a table with `json` or `ini` (see below). |
| `check` | Shell command, exit 0 means `run` has been done. Required with `run`. |
| `run` | Shell command run with `bash -c` as you, only while `check` fails. |
| `default`, `unless_cmd` | As for packages. |

Each `set` entry writes one value and is skipped when the file already holds it:

| Shape | Writes |
|-------|--------|
| `{ json = FILE, key = "a.b", value = V }` | A JSON key, dotted path for nesting. `value` keeps its type (`true`, `3`, `"text"`). The file is created if missing and other keys are left alone. |
| `{ ini = FILE, section = "S", key = "k", value = "v" }` | A `key=value` line under `[S]` in an ini-style file (`mimeapps.list`, `htoprc`). Replaces the line if it exists, otherwise appends it. |

Files are written through symlinks, so a stow-managed dotfile stays a symlink and the
change lands in your dotfiles repo. Tables can span lines with a trailing comma (TOML 1.1).

`check` and `run` cover what no file write can: `gsettings set`, `systemctl --user enable`,
`flatpak override`, `xdg-mime default`. `check` runs for real in a dry run and at launch
(for the Applied / Not applied status), so it must change nothing and finish quickly. A
run reports `FAILED` when `run` exits non-zero or `check` still fails afterwards. Neither
gets sudo.

Descriptions come from the package sources: `pacman -Si` for repo packages, the AUR web
API, and the flathub listing. They are cached in `~/.cache/cachyup/info.json`, so the list
shows them straight away next time.

## Files

| Path | What |
|------|------|
| `~/.config/cachyup/config.toml` | Tasks, update steps, cleanup numbers |
| `~/.config/cachyup/selection.toml` | On/off for every app and tweak |
| `~/.config/cachyup/user-apps.toml` | Your own apps and tweaks (optional) |
| `~/.local/state/cachyup/state.toml` | What cachyup has seen and installed (not for editing) |
| `~/.local/state/cachyup/runs/<date>/output.log` | Everything a run printed |
| `~/.local/state/cachyup/runs/<date>/run.json` | The run's summary, rewritten after every step |
| `~/.cache/cachyup/info.json` | Package descriptions |

All three settings files are plain TOML and fine to edit by hand while cachyup is closed.

## Notes

- **Sudo.** Only commands that need root use it: pacman, paccache, checkservices, rate
  mirrors. yay, flatpak and install scripts run as you (yay calls sudo itself). Every
  elevated call uses `sudo -n`, so nothing can ever ask for a password mid-run; without a
  ticket the step fails instead of hanging.
- **No prompts.** Commands get no input. `--noconfirm` answers pacman's default, so on a
  package conflict (default "do not remove") the install fails rather than waiting.
- **Stopping.** Commands run in their own process group, so `Ctrl-C` never reaches pacman
  mid-transaction. Output streams into the log as it happens; an interrupted run is still
  in the history.
- **Flatpaks** are installed system-wide. Polkit allows that without a password from an
  active desktop session; over SSH or from a bare TTY it would ask. Run from a terminal on
  the desktop.
- **Post-update checks never act.** `.pacnew` files are listed for `pacdiff --sudo`, a
  kernel update is reported for you to reboot, services come with the `systemctl restart`
  line, AUR rebuilds with the `yay` line. Packages that only repack upstream binaries
  (`-bin`, AppImages, the JetBrains IDEs, Chrome, Spotify, Discord PTB) are listed but not
  advised: rebuilding them changes nothing.
- **cachy-update.** The update and cleanup steps mirror what CachyOS's own `cachy-update`
  does interactively, minus the questions. `cachy-update` is in the app list for
  day-to-day updates between cachyup runs; the two do not conflict.

## Develop

```bash
cargo build
cargo run                       # the interface
cargo run -- --run --dry-run    # headless dry run
cargo test                      # unit tests and tests/tui_render.rs
cargo clippy --all-targets
cargo fmt
```

To try it on a scratch config without touching your own, point the XDG directories
somewhere else:

```bash
XDG_CONFIG_HOME=/tmp/c XDG_STATE_HOME=/tmp/s XDG_CACHE_HOME=/tmp/k cargo run -- --dry-run
```

Install a local build with `cargo build --release && install -m755 target/release/cachyup ~/.local/bin/`.

Releases: tag `vX.Y.Z` and push the tag. GitHub Actions builds a static binary and attaches
it to the release, which is what `install.sh` downloads.

### Adding to the built-in list

Edit [`catalog.toml`](catalog.toml) (compiled into the binary). A unit test fails if an
entry is listed twice or a tweak points at an app that does not exist. Use real package
names: groups (`gnome`) and virtual names (`jack`) show as Unavailable.

## Layout

```
catalog.toml          the built-in app and tweak list
install.sh            downloads the release binary
src/catalog.rs        catalog + user-apps.toml parsing, duplicate detection
src/config.rs         config.toml, the update steps and their dependencies
src/selection.rs      selection.toml and launch-time reconciliation (new, removed)
src/state.rs          state.toml
src/system.rs         read-only probes: installed packages, package info
src/sudo.rs           ask once, keep the ticket alive
src/app.rs            loads everything, shared by the TUI and --run
src/cli.rs            command line, headless run
src/engine/           the plan, every step, the runner, run records
src/tui/              the interface
```

## License

MIT
