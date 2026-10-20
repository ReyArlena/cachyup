#!/usr/bin/env bash
# Entry point. Clones (or updates) the repo, then hands off to install.sh.
#
# First run on a fresh machine:
#   bash <(curl -fsSL https://reyarlena.github.io/cachyup/boot.sh)
#
# Every run after that, from the `cachyup` command install.sh puts on PATH
# (a symlink to this file inside the clone). Running from inside a clone
# updates and uses that clone; running via curl uses ~/.local/share/cachyup.
set -euo pipefail

REPO_URL="${CACHYUP_REPO:-https://github.com/ReyArlena/cachyup.git}"
INSTALL_DIR="${CACHYUP_DIR:-$HOME/.local/share/cachyup}"

# Running from a clone (via the symlink or directly)? Use that clone.
SELF_DIR="$(dirname "$(readlink -f "$0")")"
if [[ -f "$SELF_DIR/install.sh" ]]; then
    INSTALL_DIR="$SELF_DIR"
fi

if ! command -v git &>/dev/null; then
    echo "==> Installing git"
    sudo pacman -S --needed --noconfirm git
fi

if [[ -d "$INSTALL_DIR/.git" ]]; then
    echo "==> Updating $INSTALL_DIR"
    git -C "$INSTALL_DIR" pull --ff-only
else
    echo "==> Cloning into $INSTALL_DIR"
    git clone "$REPO_URL" "$INSTALL_DIR"
fi

# Reattach stdin to the terminal: when piped through curl, stdin is the script itself.
exec bash "$INSTALL_DIR/install.sh" "$@" </dev/tty
