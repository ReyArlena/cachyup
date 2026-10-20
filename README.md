# linux-bootstrapper

One script to set up a fresh CachyOS install: packages, app installers, config tweaks.
Idempotent, so re-running it is always safe. All the slow parts run first with no prompts,
so you can start it, go and make tea, and come back to a summary.

## Usage

Fresh machine:

```bash
bash <(curl -fsSL https://reyarlena.github.io/linux-bootstrapper/boot.sh)
```

This clones the repo to `~/.local/share/linux-bootstrapper` (or updates it if present) and
runs `install.sh`.

You are asked for your sudo password once at the start. Nothing else needs input until the end.

Re-run later from the clone:

```bash
~/.local/share/linux-bootstrapper/install.sh
```

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

==> Done
2 succeeded, 2 skipped, 1 failed

Failed steps:
spotify.....................................FAILED (not found)

Command output for each failure is in the log, directly above its FAILED line.

Log: ~/.local/state/linux-bootstrapper/20261020-221500.log

Press o to open the log, any other key to finish
```

The log holds everything the screen showed: headings, full command output, and a status line
per step. Search it for `FAILED` and the command output sits directly above.

## Phases

Phases live in `phases/` and run in filename order.

| Phase | What it does |
|-------|--------------|
| `10-preflight.sh` | Not root, CachyOS, internet, sudo. Aborts on the first failure. Then `pacman -Syu` and the tools the script itself needs. |
| `20-packages.sh` | Pacman, then AUR (yay), then Flatpak (flathub). All the long waits. |
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
source has it), then everything remaining is installed in one call, then each package is
checked again for its final `SUCCESS` or `FAILED`.

### An installer or a config tweak

Add a file to `installers/` or `config/`. It is sourced in a subshell and must define:

```bash
NAME="Thing"            # label for the status line
check() { ...; }        # exit 0 if already done  ->  SKIPPED
run()   { ...; }        # do it; non-zero exit    ->  FAILED (last line of output)
```

Everything in `lib/` is available: `has_cmd`, `json_get`, `json_set`, `write_file`,
`copy_file`, `read_list`, `status`, `note`. See `installers/claude.sh` and `config/discord.sh`.

Writing files: use `write_file FILE` (content on stdin), `copy_file SRC DST` or `json_set`.
They create parent directories and write *through* symlinks, so a stow-managed file stays a
symlink and the change lands in the dotfiles repo. Never `mv`, `install` or `ln -sf` over an
existing path; those replace the link with a plain file. No backups are made: this tool is
meant for fresh systems.

Rules for `check()`: test the actual end state (binary exists, key has the right value), not
whether the script ran before. That is what makes re-runs safe.

### A new phase

Add `phases/NN-name.sh`. It is sourced by `install.sh` with all of `lib/` loaded. Use
`heading` once at the top and `step`, `install_list` or `run_scripts` for the work.

## Unattended install notes

- `--noconfirm` answers the default to pacman prompts. On a package conflict the default is
  "do not remove", so the install fails. Remove the conflicting package in an earlier step.
- Some AUR packages verify PGP signatures. Import the key in a config script that runs before
  the AUR phase if one is ever needed.
- The sudo ticket is refreshed in the background for the whole run, so a long AUR build does
  not leave a password prompt waiting for you.

## Layout

```
boot.sh          curl entry point: clone/update then exec install.sh
install.sh       loads lib/, runs phases/ in order
lib/log.sh       status lines, step(), two log files
lib/utils.sh     checks, sudo keepalive, read_list, run_scripts
lib/files.sh     write_file, copy_file, json_get, json_set
lib/pkg.sh       install_list + pacman/aur/flatpak backends
phases/          ordered stages
packages/        package list files
installers/      NAME/check/run scripts for curl-style installers
config/          NAME/check/run scripts for config tweaks
```
