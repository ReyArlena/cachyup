#!/usr/bin/env bash
# spotify-url-helper, own installer. Installs ~/.local/bin/spotify-url-helper
# and a .desktop file in ~/.local/share/applications.
NAME="Spotify Helper"

check() { [[ -x "$HOME/.local/bin/spotify-url-helper" ]]; }
run()   { curl -fsSL https://raw.githubusercontent.com/ReyArlena/spotify-url-helper/main/install.sh | sh; }
