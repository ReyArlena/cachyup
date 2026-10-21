#!/usr/bin/env bash
# Autostart the cachy-update tray applet (update notifier, click to run an
# interactive update). This is what `cachy-update --tray --enable` does, minus
# starting the applet right now: it copies the desktop file into autostart.
NAME="cachy-update tray autostart"

SRC="/usr/share/applications/cachy-update-tray.desktop"
DST="${XDG_CONFIG_HOME:-$HOME/.config}/autostart/cachy-update-tray.desktop"

check() {
    pacman_installed cachy-update || { SKIP_REASON="cachy-update not installed"; return 0; }
    [[ -f "$DST" ]]
}
run() { copy_file "$SRC" "$DST"; }
