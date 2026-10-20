#!/usr/bin/env bash
# Junction: a chooser for which app opens a link or file. DankMaterialShell
# ships its own (DMS Open), so only install Junction when DMS is absent.
NAME="junction (pacman)"

check() {
    pacman_installed junction && return 0
    has_cmd dms && { SKIP_REASON="using DMS Open"; return 0; }
    return 1
}
run() { pacman_install junction; }
