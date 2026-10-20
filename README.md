# cachyup

One command to set up a fresh CachyOS install: packages, app installers, config tweaks.
Idempotent, so re-running it is always safe, and re-running is how you update: it does a
full system update, installs anything new in the lists, and skips everything already done.
All the slow parts run first with no prompts, so you can start it, go and make tea, and come
back to a summary.

## Usage

Fresh machine:

```bash
bash <(curl -fsSL https://reyarlena.github.io/cachyup/boot.sh)
```

This clones the repo to `~/.local/share/cachyup`, runs `install.sh`, and puts a `cachyup`
command on your PATH. You are asked for your sudo password once at the start. Nothing else
needs input until the end.

Every run after that:

```bash
cachyup
```

`cachyup` is a symlink at `~/.local/bin/cachyup` pointing at `boot.sh` inside the clone.
`boot.sh` pulls the latest version of the repo and runs `install.sh`, so a plain `cachyup`
is "update the system and apply whatever changed in the lists". Preflight checks that
`~/.local/bin` is on your PATH and reports FAILED if not; the link is still created.

Run as your normal user, not root. The script refuses to run as root because AUR builds and
home-directory config must be owned by you.

## What a run looks like

```
==> Preflight
Running as normal user......................PASS
Running on CachyOS..........................PASS
Internet connection.........................PASS
Sudo access.................................PASS

==> Pacman packages
discord.....................................SKIPPED (already present)
flatpak.....................................SUCCESS

==> AUR packages
spotify.....................................FAILED (not found)

==> Installers
Claude Code.................................SKIPPED (already present)

 ==> Summary
   (every heading and status line again, with nothing in between)

2 succeeded, 2 skipped, 1 failed
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
| `10-preflight.sh` | Not root, CachyOS, internet, sudo, PATH. Then `pacman -Syu`, the tools the script itself needs, and the `cachyup` command. |
| `20-packages.sh` | Pacman, then AUR (yay), then Flatpak (flathub), then conditional packages. All the long waits. |
| `30-installers.sh` | Apps with their own install script, e.g. Claude Code. |
| `40-config.sh` | Static config tweaks, e.g. Discord no self-update. |
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

Everything in `lib/` is available: `has_cmd`, `json_get`, `json_set`, `ini_get`, `ini_set`,
`write_file`, `copy_file`, `read_list`, `status`, `note`. See `installers/claude.sh` and `config/discord.sh`.

Writing files: use `write_file FILE` (content on stdin), `copy_file SRC DST`, `json_set` or
`ini_set`.
They create parent directories and write *through* symlinks, so a stow-managed file stays a
symlink and the change lands in the dotfiles repo. Never `mv`, `install` or `ln -sf` over an
existing path; those replace the link with a plain file. No backups are made: this tool is
meant for fresh systems.

Rules for `check()`: test the actual end state (binary exists, key has the right value), not
whether the script ran before. That is what makes re-runs safe.

### A new phase

Add `phases/NN-name.sh`. It is sourced by `install.sh` with all of `lib/` loaded. Use
`heading` once at the top, then:

- `step NAME CHECK_FN ACTION_FN` for one thing. Both arguments are function names called with
  no arguments; wrap anything needing arguments or quotes in a small function.
- `install_list KIND FILE` for a package list.
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

## Layout

```
boot.sh          entry point: clone or update the repo, then exec install.sh
install.sh       loads lib/, runs phases/ in order
lib/log.sh       status lines, step(), log file, summary
lib/utils.sh     require/check, sudo keepalive, read_list, run_scripts
lib/files.sh     write_file, copy_file, json_get, json_set, ini_get, ini_set
lib/pkg.sh       install_list + pacman/aur/flatpak backends
phases/          ordered stages
packages/        package list files
conditional/     NAME/check/run scripts for packages with an install condition
installers/      NAME/check/run scripts for curl-style installers
config/          NAME/check/run scripts for config tweaks
```
