#!/usr/bin/env bash
# Claude Code, native installer. Installs to ~/.local/bin/claude.
NAME="Claude Code"

check() { has_cmd claude || [[ -x "$HOME/.local/bin/claude" ]]; }
run()   { curl -fsSL https://claude.ai/install.sh | bash; }
