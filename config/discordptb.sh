#!/usr/bin/env bash
# Same as discord.sh, for the PTB build.
NAME="Discord PTB: disable host update"

FILE="$HOME/.config/discordptb/settings.json"

check() { [[ "$(json_get "$FILE" '.SKIP_HOST_UPDATE')" == "true" ]]; }
run()   { json_set "$FILE" '.SKIP_HOST_UPDATE' true; }
