#!/usr/bin/env bash
# Stop Discord blocking launch until it self-updates (pacman owns updates).
NAME="Discord: disable host update"

FILE="$HOME/.config/discord/settings.json"

check() { [[ "$(json_get "$FILE" '.SKIP_HOST_UPDATE')" == "true" ]]; }
run()   { json_set "$FILE" '.SKIP_HOST_UPDATE' true; }
