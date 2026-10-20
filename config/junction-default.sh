#!/usr/bin/env bash
# Make Junction the handler for http/https links, so clicking a link asks which
# app to use. Only when Junction is installed and DMS (which has DMS Open for
# the same job) is not.
NAME="Junction: default link handler"

APP="re.sol.junction.desktop"
MIMEAPPS="${XDG_CONFIG_HOME:-$HOME/.config}/mimeapps.list"

check() {
    has_cmd dms && { SKIP_REASON="using DMS Open"; return 0; }
    pacman_installed junction || { SKIP_REASON="junction not installed"; return 0; }
    [[ "$(ini_get "$MIMEAPPS" "Default Applications" "x-scheme-handler/http")"  == "$APP" ]] \
        && [[ "$(ini_get "$MIMEAPPS" "Default Applications" "x-scheme-handler/https")" == "$APP" ]]
}

run() {
    ini_set "$MIMEAPPS" "Default Applications" "x-scheme-handler/http"  "$APP"
    ini_set "$MIMEAPPS" "Default Applications" "x-scheme-handler/https" "$APP"
}
