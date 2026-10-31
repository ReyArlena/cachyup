# Dotfiles: phase 2 plan

Phase 1 (done, uncommitted): the read-only Dotfiles screen (`D`). It checks that stow is
installed, asks for the stow folder the first time, and lists every package with how it
is linked, its links, its `.gitignore` rules, warnings and a `stow --simulate` preview.

Phase 2 turns that screen into a stow manager, so packages can be linked, unlinked and
reshaped without the command line or the fish functions (`stow-apply`, `stow-yoink`).

## Preferences

- **Folder links are the preferred shape.** A whole folder linked into the repo picks up
  every new file the app creates, so it shows up in the repo and can be committed or
  excluded by hand.
- **Blocklist `.gitignore` is the default.** New files are tracked unless excluded. An
  allowlist (`pkg/.config/app/*` plus `!pkg/.config/app/keep.json`) is never the default.
  It is only mentioned as an option where it helps, for example next to a secrets
  warning.
- **File links where folder links cannot work.** Folders that hold symlinks the system
  manages (systemd's `*.wants/`) stay as single-file links.

## Safety rules

- Native stow only: `stow`, `stow -R`, `stow -D`, `stow --no-folding`. Never `--adopt`
  (it overwrites the repo copy with the system file) and never `--override`.
- No sudo. Everything is in the home folder and the user's own repo.
- Every action shows its plan first: the exact commands, the `stow --simulate` output and
  any warnings. If stow reports a conflict, the action cannot run.
- Every action checks the real state before acting and rescans afterwards to confirm the
  result (same rule as the rest of cachyup). Running an action twice does nothing the
  second time.
- Nothing is deleted. Replacing a folder moves it to a backup next to it.
- Protected folders are never replaced or unlinked as a whole: `~`, `~/.config`,
  `~/.local`, `~/.local/share`, `~/.local/state`, `~/.cache`.
- Actions run through the engine (`Ctx::exec` / `Ctx::mutate`) as a short plan of steps
  in a `Dotfiles` group, so a dry run only logs them and every real action is recorded in
  History. The engine stays free of terminal code; the screen talks to it through
  `Effect`s.

## Actions

Proposed keys, all letters (arrows and h/j/k/l stay navigation only):

| Key | Action | Commands |
|-----|--------|----------|
| `s` | Link (stow) | `stow -d <dir> -t ~ <pkg>`, with the existing-folder choice below |
| `F` | Link as file links only | `stow --no-folding -d <dir> -t ~ <pkg>` |
| `R` | Relink | `stow -R ...`: picks up new files in file-link packages, fixes partial ones |
| `u` | Unlink | `stow -D ...`: removes links only, never real files |
| `O` | Convert file links to one folder link | unlink, replace folder (backup), link |
| `I` | Convert folder link to file links | `stow -D`, `mkdir -p` the folder, `stow --no-folding` |
| `w` | Switch to this alternative | unlink the package that owns the folder (`hypr-dms`), link this one (`hypr`) |

Each opens a confirmation popup like the run popup: `Enter` run, `d` dry run, `Esc`
cancel.

### Link when the folder already exists

Stow decides the shape from what is already in `~`: a missing folder becomes one folder
link, an existing real folder gets links inside it. So when the package's folder already
exists as a real folder, the popup offers:

1. **Replace folder** (default when allowed): move the folder to a backup, then `stow`
   creates the folder link.
2. **Link files inside**: plain `stow`, which links into the existing folder.
3. **Cancel.**

Which folder gets replaced: the shallowest folder of the package that is not protected
(`~/.config/fish` for `fish`, `~/.claude` for `claude`). The popup lets you step to a
deeper folder instead (for example `~/.claude/skills` rather than all of `~/.claude`).

Before replacing, the popup shows:

- **Data loss warning** with specifics: files only on this machine (they leave active
  use and stay only in the backup), files that differ from the repo, and how many are
  identical. Up to a dozen names each, then a count.
- **Loud warning for app state folders** like `~/.claude`, which hold sessions and
  credentials, not just config.
- **Symlink block**: if the folder contains symlinks, replace is not allowed and the
  popup lists them. Symlinks that are this package's own stow links are left out, since
  they are replaced anyway (`~/.config/systemd/user` has 18 symlinks, 9 of them stow
  links). Chromium-style `SingletonLock` / `SingletonSocket` / `SingletonCookie` links
  only exist while the app runs, so the popup says to close the app and try again.
  "Link files inside" stays available.

Replace steps:

1. Rename `<folder>` to `<folder>.cachyup-backup-<YYYYmmdd-HHMMSS>` in the same parent.
   A rename on the same filesystem: instant, nothing copied, nothing deleted.
2. `stow -d <dir> -t ~ <pkg>`.
3. Rescan. If stow failed, rename the backup back so the system is as it was.

Backups are never removed by cachyup. The screen could list them later with an option to
delete one after confirming.

### Unlink

`stow -D` removes only links. For a folder link that means the app folder is gone
afterwards, so the popup says so ("~/.config/fish will not exist; fish starts with no
config"). A second option, **Unlink and keep a copy**, unlinks and then copies the
package's files into a real folder in their place (not the git-ignored ones unless asked).

## New scan warnings

- **Symlinks inside a folder link**: links the system or an app created inside a
  folder-linked package now live in the repo. Tracked ones are serious; git-ignored ones
  are a note. Catches a systemd folder being folder-linked by mistake. (Today: herdr
  sockets, discordptb `Singleton*`, `local-bin/.local/bin/claude`.)
- **New files not yet decided**: files in a folder-linked package that git neither
  tracks nor ignores (`git status --porcelain --untracked-files=all -- <pkg>`). This is
  the review list for the blocklist workflow: commit them or add them to `.gitignore`.
- **Allowlist note**: a package whose `.gitignore` rules are allowlist style gets an
  informational note that new files the app creates are not tracked. Not a nudge to
  change it.

## Phase 3 (later)

Creating new packages from the system, replacing `stow-yoink` and `stow-adopt`:

- Pick a folder in `~`, name the package (default: the folder name).
- Choose how: **whole folder** (folder link, blocklist `.gitignore`), **whole folder with
  exclusions** (pick files or folders to add to `.gitignore`), or **individual files**
  (file links; the only choice for folders with symlinks, such as systemd).
- Symlinks are never copied into the repo.
- Copy into the package, show the diff, then replace the folder through the same backup
  flow and link it.
- Optional: offer allowlist rules for folders that look like app data (caches, sockets,
  databases), never as the default.

## Decisions

- **Backups go next to the original folder** (`<folder>.cachyup-backup-<date>`): same
  filesystem, so the rename is instant and nothing is copied.
- **No git actions in the app.** Commit, push and pull stay with the user, outside
  cachyup. The app only reads git (`ls-files`, `check-ignore`, `status`) to show rules,
  ignored files and new files. Writing `.gitignore` is fine (a file edit, not a git
  command), so phase 3's "whole folder with exclusions" appends its lines there.
- **Allowlist note** is informational only: shown when a package already uses an
  allowlist, never suggesting one.
