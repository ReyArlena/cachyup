#!/usr/bin/env bash
# Checks that must pass before anything is installed. Aborts on the first failure.
# Then: sudo ticket, system update, tools the later phases depend on.

heading "Preflight"

local_bin_on_path() { [[ ":$PATH:" == *":$HOME/.local/bin:"* ]]; }

require "Running as normal user" not_root       "run as your user, not root (sudo is requested when needed)"
require "Running on CachyOS"      is_cachyos     "this script targets CachyOS only"
require "Internet connection"     has_internet   "cannot reach github.com"
require "Sudo access"             sudo_keepalive "sudo authentication failed"
check   "~/.local/bin on PATH"    local_bin_on_path "add ~/.local/bin to PATH so the cachyup command is found"

heading "System update"
system_update() { sudo pacman -Syu --noconfirm; }
step "pacman -Syu" false system_update

heading "Prerequisites"
install_list pacman "$BOOTSTRAP_DIR/packages/prerequisites.txt"

# `cachyup` on PATH: a symlink to boot.sh inside this clone, so every later run
# is just `cachyup` (boot.sh pulls the latest and runs install.sh).
CACHYUP_LINK="$HOME/.local/bin/cachyup"
cachyup_linked() { [[ "$(readlink -f "$CACHYUP_LINK" 2>/dev/null)" == "$BOOTSTRAP_DIR/boot.sh" ]]; }
cachyup_link() {
    if [[ -e "$CACHYUP_LINK" && ! -L "$CACHYUP_LINK" ]]; then
        echo "$CACHYUP_LINK exists and is not a symlink, not replacing it"
        return 1
    fi
    mkdir -p "$HOME/.local/bin" && ln -sfn "$BOOTSTRAP_DIR/boot.sh" "$CACHYUP_LINK"
}
step "Install global cachyup command" cachyup_linked cachyup_link
