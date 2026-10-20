#!/usr/bin/env bash
# kiln, own installer. As a normal user it installs to ~/.local/bin/kiln.
NAME="kiln"

check() { has_cmd kiln || [[ -x "$HOME/.local/bin/kiln" ]]; }
run()   { curl -fsSL https://raw.githubusercontent.com/ReyArlena/kiln/main/install.sh | sh; }
