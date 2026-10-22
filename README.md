# cachyup

One command to set up a fresh CachyOS install: packages, app installers, config tweaks.
Idempotent, so re-running it is always safe, and re-running is how you update: it rates
mirrors, updates repo, AUR and flatpak packages, installs anything new in the lists, skips
everything already done, cleans up, and reports what needs a human (`.pacnew` files, a
pending reboot, AUR packages needing a rebuild).
All the slow parts run first with no prompts, so you can start it, go and make tea, and come
back to a summary.

## Usage

Fresh machine:

```bash
curl -fsSL https://reyarlena.github.io/cachyup/install.sh | bash
```

This clones the repo to `~/.local/share/cachyup`, runs `run.sh`, and puts a `cachyup`
command on your PATH. At the start you are asked for your sudo password and which apps this
machine wants (see [App selection](#app-selection)). Nothing else needs input until the end.

Every run after that:

```bash
cachyup
```

`cachyup` is a symlink at `~/.local/bin/cachyup` pointing at `install.sh` inside the clone.
`install.sh` pulls the latest version of the repo and runs `run.sh`, so a plain `cachyup`
is "update the system and apply whatever changed in the lists". Preflight checks that
`~/.local/bin` is on your PATH and reports FAILED if not; the link is still created.

There are no options. Apps added to the lists since your last run are asked about at the
start, and every run offers to customise the full selection.

Run as your normal user, not root. The script refuses to run as root because AUR builds and
home-directory config must be owned by you.

## What a run looks like

```
==> Preflight
Running as normal user......................PASS
Running on CachyOS..........................PASS
Internet connection.........................PASS
Sudo access.................................PASS
~/.local/bin on PATH........................PASS
gum.........................................SKIPPED (already present)
Apps........................................PASS (2 new: 1 install, 1 skip)

==> Pacman packages
discord.....................................SKIPPED (already present)
flatpak.....................................SUCCESS

==> AUR packages
phpstorm....................................SKIPPED (deselected)
spotify.....................................SUCCESS
spotfiy.....................................FAILED (not found)

==> Installers
Claude Code.................................SKIPPED (already present)

 ==> Summary
   (every heading and status line again, with nothing in between)

3 succeeded, 3 skipped, 1 failed
Command output for each failure is in the log, directly above its FAILED line.

Log: ~/.local/state/cachyup/20261020-221500.log

Press o to open the log, any other key to finish
```

The log holds everything the screen showed: headings, full command output, and a status line
per step. Search it for `FAILED` and the command output sits directly above.

Status meanings: `SUCCESS` something was installed or changed, `SKIPPED` already in place,
`FAILED` look at the log, `PASS` a check that changes nothing (not counted).

## Phases

Phases live in `phases/` and run in filename order.

| Phase | What it does |
|-------|--------------|
| `10-preflight.sh` | Not root, CachyOS, internet, sudo, PATH, gum, the app questions. Then rate mirrors, keyrings, `pacman -Syu`, `yay -Sua`, `flatpak update`, the tools the script itself needs, and the `cachyup` command. |
| `20-packages.sh` | Pacman, then AUR (yay), then Flatpak (flathub), then conditional packages. All the long waits. |
| `30-installers.sh` | Apps with their own install script, e.g. Claude Code. |
| `40-config.sh` | Static config tweaks, e.g. Discord no self-update. |
| `80-cleanup.sh` | Orphans, package cache (keeps 2 versions of installed, none of uninstalled), yay build cache older than 30 days, unused flatpak runtimes. Then checks: `.pacnew` files, reboot needed, services needing a restart, AUR rebuilds. |
| `90-finish.sh` | Summary, log path, the only prompt. |

## Adding things

### A package

Add one line to the right file in `packages/`. Comments with `#` are fine.

| File | Installed with |
|------|----------------|
| `prerequisites.txt` | pacman, before everything else |
| `pacman.txt` | `pacman -S --needed --noconfirm` |
| `aur.txt` | `yay -S --needed --noconfirm` |
| `flatpak.txt` | `flatpak install -y --noninteractive flathub` |

Each package is checked before the install (`SKIPPED` if present, `FAILED (not found)` if no
source has it), installed on its own, then checked again for its final `SUCCESS` or `FAILED`.
One package per call is a little slower than one big transaction but every package gets its
own honest result and reason.

Real package names only. Groups (`gnome`) and virtual names (`jack`) are not resolved and
show as not found.

### App selection

The lists say what exists. Which of it this machine wants is decided in two gum screens at the
start of the run, before the long update, and saved to `~/.config/cachyup/apps`:

1. **New apps.** Any list line with no decision saved yet is shown, one multi-select per
   source (pacman, AUR, flatpak), everything pre-checked. Untick what you do not want, enter
   confirms. On the first run that is the whole list. Skipped entirely when nothing is new,
   so adding a package to a list is what makes the question appear on every machine.
2. **Customise the full app selection?** Asked every run, default No, so Enter carries on.
   Yes shows every package with the current selection pre-checked.

Esc or Ctrl-C on a screen aborts the run rather than counting as "skip everything".

`~/.config/cachyup/apps` has one line per package, `kind/name` and a state:

| State | Meaning |
|-------|---------|
| `install` | Wanted. Installed when absent, retried each run until it succeeds. |
| `done` | cachyup installed it, or found it already present, at least once. Hands off from then on: if it goes missing later you removed it on purpose, and it is not reinstalled. |
| `skip` | Unticked. Never installed. |

The file records what cachyup did and what you chose, never what the system has right now,
so it does not go stale. The status line says which: `SKIPPED (already present)`,
`SKIPPED (deselected)`, `SKIPPED (removed manually)`. To get a removed package back, say Yes
to the customise question and tick it again. Packages dropped from a list are forgotten, so
adding them back asks again. `prerequisites.txt` is never asked about.

### A conditional package

A package that should only be installed in some situations gets a script in `conditional/`
instead of a line in a list. Same shape as an installer (below), using the package helpers
from `lib/pkg.sh`. Set `SKIP_REASON` in `check()` to say why it was skipped:

```bash
NAME="junction (pacman)"
check() {
    pacman_installed junction && return 0
    has_cmd dms && { SKIP_REASON="using DMS Open"; return 0; }
    return 1
}
run() { pacman_install junction; }
```

```
junction (pacman)...........................SKIPPED (using DMS Open)
```

### An installer or a config tweak

Add a file to `installers/` or `config/`. It is sourced in a subshell and must define:

```bash
NAME="Thing"            # label for the status line
check() { ...; }        # exit 0 if already done  ->  SKIPPED
                        #   set SKIP_REASON first to replace "already present"
run()   { ...; }        # do it; non-zero exit    ->  FAILED (last line of output)
```

Everything in `lib/` is available: `has_cmd`, `json_get`, `json_set`, `ini_get`,
`ini_set`, `write_file`, `copy_file`, `read_list`, `status`, `note`. See `installers/claude.sh`,
`config/discord.sh` and `config/junction-default.sh`.

Writing files: use `write_file FILE` (content on stdin), `copy_file SRC DST`, `json_set` or
`ini_set`. They create parent directories and write *through* symlinks, so a stow-managed
file stays a symlink and the change lands in the dotfiles repo. Never `mv`, `install` or
`ln -sf` over an existing path; those replace the link with a plain file. No backups are
made: this tool is meant for fresh systems.

Rules for `check()`: test the actual end state (binary exists, key has the right value), not
whether the script ran before. That is what makes re-runs safe.

### A new phase

Add `phases/NN-name.sh`. It is sourced by `run.sh` with all of `lib/` loaded. Use
`heading` once at the top, then:

- `step NAME CHECK_FN ACTION_FN` for one thing. `CHECK_FN` and `ACTION_FN` are function names
  called with no arguments; wrap anything needing arguments or quotes in a small function.
- `install_list KIND FILE` for a package list honouring the app selection, `install_list KIND
  FILE always` for one that is always installed.
- `run_scripts DIR` for a directory of `NAME`/`check`/`run` scripts.
- `require NAME CHECK_FN MSG` for a check that must pass (aborts), `check NAME CHECK_FN MSG`
  for one that just reports.

## Unattended install notes

- `--noconfirm` answers the default to pacman prompts. On a package conflict the default is
  "do not remove", so the install fails. Remove the conflicting package in an earlier step.
- Flatpaks are installed system-wide, the flatpak default. Polkit allows that without a
  password from an active desktop session; over SSH or from a bare TTY it would ask for admin
  auth. Run from a terminal on the desktop.
- Ctrl-C stops the run; output streams into the log as it happens, so nothing is lost, and the
  log ends with `INTERRUPTED`. Re-run to carry on.
- Some AUR packages verify PGP signatures. Import the key in a config script that runs before
  the AUR phase if one is ever needed.
- The sudo ticket is refreshed in the background for the whole run, so a long AUR build does
  not leave a password prompt waiting for you.
- The post-update checks never act on their own. `.pacnew` files are listed for `pacdiff --sudo`,
  a kernel update is reported for you to reboot, services running replaced binaries come with
  the `systemctl restart` line, and AUR rebuilds with the `yay` line. `checkrebuild` is slow,
  so it starts in the background after the AUR update and its result is collected at the end.
  Packages that only repack upstream binaries (`-bin`, AppImages, the JetBrains IDEs, Chrome,
  Spotify, Discord PTB) are listed but not advised: rebuilding them changes nothing. The list
  of such names is `PREBUILT_PATTERNS` in `phases/80-cleanup.sh`.
- The update and cleanup steps mirror what CachyOS's own `cachy-update` does interactively
  (a fork of Arch-Update), minus the questions. `cachy-update` itself is in `pacman.txt`; its
  package autostarts the tray applet system-wide, for day-to-day updates between `cachyup`
  runs. The two do not conflict.

## Layout

```
install.sh       entry point: clone or update the repo, then exec run.sh
run.sh           loads lib/, runs phases/ in order
lib/log.sh       status lines, step(), log file, summary
lib/utils.sh     require/check, sudo keepalive, read_list, run_scripts
lib/files.sh     write_file, copy_file, json_get, json_set, ini_get, ini_set
lib/pkg.sh       install_list + pacman/aur/flatpak backends
lib/apps.sh      app selection: the two questions and ~/.config/cachyup/apps
phases/          ordered stages
packages/        package list files
conditional/     NAME/check/run scripts for packages with an install condition
installers/      NAME/check/run scripts for curl-style installers
config/          NAME/check/run scripts for config tweaks
```
