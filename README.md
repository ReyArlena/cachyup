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
cachyup tells you at launch when a new release is out. Run as your normal user, not root.

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
│   136 selected, 2 to install││ ✗ junction  using DMS Open             Not installed  │
│                             ││ ✓ zed                     new          Not installed  │
│ ✓ Install Tweaks            ││                                                       │
│   2 of 3 selected           ││ AUR packages  18/18                                   │
└─────────────────────────────┘└───────────────────────────────────────────────────────┘
 Space toggle  c options  r run  h history  ? help  q quit
```

The left pane lists the three tasks; `Space` switches one on or off. The right pane shows
the highlighted task's options. Press `c` (or `Enter`, `Tab`, `→`) to move into them, and
`Esc` to come back. Changes are saved as you make them.

| Task | Options |
|------|---------|
| Full System Update | Each step on or off: rate mirrors, keyrings, `pacman -Syu`, `yay -Sua`, `flatpak update`, and the cleanup steps. Numbers: mirror rating cooldown, package cache versions, yay cache age, runs kept in history. |
| Install Apps | Every app in the list, grouped by source, with a status column (Installed, Not installed, Unavailable). Tweaks for an app are listed under it. `i` shows the description and package details, `/` filters, `a` switches a whole group. |
| Install Tweaks | The same tweaks as a flat list, with Applied / Not applied. Toggling here and under the app is the same switch. |

Ticks are green and switched on; crosses are grey and switched off. Blue entries with a
`new` badge were added in a cachyup update and start switched off.

`r` asks for confirmation: `Enter` runs, `d` does a dry run, `Esc` cancels. While running,
the left pane ticks off each step and the right pane shows the full terminal output.
Select a step on the left to jump to its output. `q` offers to stop after the current step
(safe) or right away. At the end a summary lists every failure and everything that needs
attention.

`h` opens the history: past runs on the left, the selected run's output on the right,
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
| Post-update checks | Report only: `.pacnew` files to merge, reboot needed, services running replaced binaries, AUR packages built against removed libraries. |

Statuses: `SUCCESS` something was installed or changed, `SKIPPED` already in place or not
applicable, `FAILED` see the output, `PASS` a check came out fine, `ATTENTION` something for
you to do (the reason says what), `WOULD RUN` dry run.

Installing one package per call is a little slower than one big transaction, but every
package gets its own honest result and reason.

## Apps and tweaks

### Your selection

Every app and tweak is on or off, saved in `~/.config/cachyup/selection.toml`. Switching an
app off never uninstalls it.

- **First launch:** everything is on, except entries with a condition. Junction (a link
  chooser) is off when DankMaterialShell is installed, because DMS Open does the same job.
- **cachyup adds an app in an update:** a popup at launch lists the new entries. They start
  off, shown in blue with a `new` badge for that session. `Enter` jumps to the list to
  review them.
- **You uninstall an app cachyup installed:** at the next launch it is switched off, with a
  popup saying so, so it is not reinstalled behind your back. Switch it on to get it back.
- **An app is dropped from the list:** it is forgotten. If it comes back later it counts as
  new.

### Adding your own

Create `~/.config/cachyup/user-apps.toml`. Same layout as cachyup's own
[`catalog.toml`](catalog.toml):

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
set = [{ ini = "~/.config/htop/htoprc", section = "x", key = "tree_view", value = 1 }]
```

Your entries start switched on the first time they are seen. An entry that repeats one of
cachyup's own is ignored, and a popup at launch says so. So does a name listed under two
sources, or a file that does not parse.

Tweak values are written with `{ json = FILE, key = "a.b", value = ... }` (a dotted JSON
key) or `{ ini = FILE, section = "S", key = "k", value = "v" }`. Files are written through
symlinks, so a stow-managed dotfile stays a symlink and the change lands in your dotfiles
repo.

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
