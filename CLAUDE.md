# cachyup

Sets up and updates a CachyOS install: a Rust binary with a ratatui TUI and a headless `--run` mode. It wraps pacman, yay, flatpak and the CachyOS/Arch maintenance tools, one step at a time, and logs every run.

## Layout

- `catalog.toml` the built-in app and tweak list, compiled in. `src/catalog.rs` parses it plus `~/.config/cachyup/user-apps.toml`.
- `src/config.rs` (config.toml, `UPDATE_STEPS`, `NUMBER_SETTINGS`), `src/selection.rs` (on/off per app/tweak, launch reconciliation), `src/state.rs` (seen and managed apps).
- `src/engine/` the core: `plan.rs` builds the ordered steps, `steps.rs` holds every step body, `ctx.rs` runs commands (streamed, dry-run aware), `runner.rs` the worker thread, `record.rs` run.json.
- `src/tui/` the interface: `app.rs` event loop and effects, `main_screen.rs`, `run_screen.rs`, `history.rs`, `widgets.rs`, `input.rs`, `theme.rs`.
- `src/app.rs` shared bootstrap (`AppState`) for the TUI and the CLI.

## Rules

- Never run as root. Elevated commands use `Cmd::sudo` (always `sudo -n`); yay, flatpak and installers never use sudo.
- Every step checks the real end state before acting and after installing. Checks run for real in a dry run; changes go through `Ctx::exec` or `Ctx::mutate` so a dry run only logs them.
- Never replace a user's file: write through symlinks with `paths::write_file`.
- Engine must not know about terminals. New UI needs go through `Event`/`Effect`.
- Only the 16 named terminal colours (`tui/theme.rs`). Only the focused pane gets a coloured border.
- Arrows and h/j/k/l are navigation only. Never bind them to actions (history is `H`).
- Before every commit, grep for personal data (passwords, IPs, hostnames, emails).
- Update `CHANGELOG.md` (Keep a Changelog) with user-visible changes.
- Comment concisely for humans; prefer readable code over clever shorthand.

## Commands

```
cargo build
cargo run -- --run --dry-run      # headless dry run against the real system
cargo test                        # unit tests + tests/tui_render.rs
cargo clippy --all-targets
cargo fmt
```

The TUI cannot be driven without a TTY. Use `tests/tui_render.rs` (TestBackend; `CACHYUP_DUMP=dir` writes each rendered screen to a file) and the headless dry run. Point `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` at a scratch directory to keep the user's own settings untouched.
