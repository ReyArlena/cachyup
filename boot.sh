#!/usr/bin/env bash
# Remote entry point. Clones (or updates) the repo, then hands off to install.sh.
#
#   bash <(curl -fsSL https://reyarlena.github.io/linux-bootstrapper/boot.sh)
#
# Also works with `curl ... | bash`: stdin is re-attached to the terminal before
# install.sh runs so prompts (sudo password, key presses) still work.
set -euo pipefail

REPO_URL="${LINUX_BOOTSTRAPPER_REPO:-https://github.com/ReyArlena/linux-bootstrapper.git}"
INSTALL_DIR="${LINUX_BOOTSTRAPPER_DIR:-$HOME/.local/share/linux-bootstrapper}"

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

exec bash "$INSTALL_DIR/install.sh" "$@" </dev/tty
