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

# gum is needed for the app questions, which must come before the long system
# update. -Sy (not -Syu) so it is quick; the full -Syu follows straight after.
has_gum()     { has_cmd gum; }
install_gum() { sudo pacman -Sy --needed --noconfirm gum; }
step "gum" has_gum install_gum

# All the questions, together, then nothing until the end.
apps_load
apps_ask_new
apps_offer_customise
status "Apps" PASS "$(apps_summary)"

heading "System update"
# Order matters: fast mirrors first, fresh keyrings so signatures verify on a
# machine that has not updated in a while, then the full upgrade, then the
# sources pacman does not cover.
rate_mirrors()   { sudo cachyos-rate-mirrors; }
keyrings()       { sudo pacman -Sy --needed --noconfirm archlinux-keyring cachyos-keyring; }
system_update()  { sudo pacman -Syu --noconfirm; }
aur_update()     { yay -Sua --noconfirm --answerdiff None --answerclean None --removemake; }
flatpak_update() { flatpak update -y --noninteractive; }
# Rating takes a while and rankings barely move week to week, so skip if the
# mirrorlist (rewritten by cachyos-rate-mirrors) is newer than the cooldown.
RATE_MIRRORS_COOLDOWN_DAYS=7
MIRRORLIST=/etc/pacman.d/cachyos-mirrorlist
no_rate_mirrors() {
    has_cmd cachyos-rate-mirrors || { SKIP_REASON="cachyos-rate-mirrors not installed"; return 0; }
    [[ -f "$MIRRORLIST" ]] || return 1
    local age_days=$(( ($(date +%s) - $(stat -c %Y "$MIRRORLIST")) / 86400 ))
    (( age_days < RATE_MIRRORS_COOLDOWN_DAYS )) || return 1
    SKIP_REASON="rated ${age_days}d ago, cooldown ${RATE_MIRRORS_COOLDOWN_DAYS}d"
    return 0
}
no_yay()          { ! has_cmd yay && { SKIP_REASON="yay not installed"; return 0; }; return 1; }
no_flatpak()      { ! has_cmd flatpak && { SKIP_REASON="flatpak not installed"; return 0; }; return 1; }

step "Rate mirrors"    no_rate_mirrors rate_mirrors
step "Keyrings"        false           keyrings
step "pacman -Syu"     false           system_update
step "yay -Sua"        no_yay          aur_update
step "flatpak update"  no_flatpak      flatpak_update

# checkrebuild (AUR packages linked against removed libraries) runs ldd over
# every file of every AUR package and takes minutes. Start it now in the
# background; the post-update checks in 80-cleanup.sh collect the result.
REBUILD_OUT="$(mktemp)"
if has_cmd checkrebuild; then
    checkrebuild > "$REBUILD_OUT" 2>/dev/null &
    REBUILD_PID=$!
fi

heading "Prerequisites"
install_list pacman "$BOOTSTRAP_DIR/packages/prerequisites.txt" always

# `cachyup` on PATH: a symlink to install.sh inside this clone, so every later run
# is just `cachyup` (install.sh pulls the latest and runs run.sh).
CACHYUP_LINK="$HOME/.local/bin/cachyup"
cachyup_linked() { [[ "$(readlink -f "$CACHYUP_LINK" 2>/dev/null)" == "$BOOTSTRAP_DIR/install.sh" ]]; }
cachyup_link() {
    if [[ -e "$CACHYUP_LINK" && ! -L "$CACHYUP_LINK" ]]; then
        echo "$CACHYUP_LINK exists and is not a symlink, not replacing it"
        return 1
    fi
    mkdir -p "$HOME/.local/bin" && ln -sfn "$BOOTSTRAP_DIR/install.sh" "$CACHYUP_LINK"
}
step "Install global cachyup command" cachyup_linked cachyup_link
