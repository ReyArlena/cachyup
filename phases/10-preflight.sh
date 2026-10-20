#!/usr/bin/env bash
# Checks that must pass before anything is installed. Aborts on the first failure.
# Then: sudo ticket, system update, tools the later phases depend on.

heading "Preflight"

require "Running as normal user" not_root     "run as your user, not root (sudo is requested when needed)"
require "Running on CachyOS"      is_cachyos   "this script targets CachyOS only"
require "Internet connection"     has_internet "cannot reach github.com"
require "Sudo access"             sudo_keepalive "sudo authentication failed"

heading "System update"
step "pacman -Syu" false "sudo pacman -Syu --noconfirm"

heading "Prerequisites"
install_list pacman "$BOOTSTRAP_DIR/packages/prerequisites.txt"
