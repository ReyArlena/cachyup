#!/usr/bin/env bash
# Package installation. One flow for pacman, AUR (yay) and flatpak, one
# package at a time so every package gets its own honest result:
#   SKIPPED  already installed
#   FAILED   not found in any source, or the install ran and it is still absent
#   SUCCESS  installed now
#
# Each backend provides three functions: <kind>_installed, <kind>_available, <kind>_install.

install_list() {
    local kind="$1" file="$2"
    local -a pkgs=()
    mapfile -t pkgs < <(read_list "$file")

    local p
    for p in "${pkgs[@]}"; do
        if "${kind}_installed" "$p"; then
            status "$p" SKIPPED "already present"
        elif ! "${kind}_available" "$p"; then
            status "$p" FAILED "not found"
        else
            note "installing: $p"
            run_logged "${kind}_install" "$p" || true
            if "${kind}_installed" "$p"; then
                status "$p" SUCCESS
            else
                status "$p" FAILED "${RUN_LAST_LINE:-see log}"
            fi
        fi
    done
}

# --- pacman ---
pacman_installed() { pacman -Qq "$1" &>/dev/null; }
pacman_available() { pacman -Si "$1" &>/dev/null; }
pacman_install()   { sudo pacman -S --needed --noconfirm "$@"; }

# --- AUR via yay (yay itself comes from the cachyos repo) ---
aur_installed() { pacman -Qq "$1" &>/dev/null; }
aur_available() { yay -Si "$1" &>/dev/null; }
aur_install()   { yay -S --needed --noconfirm --answerdiff None --answerclean None --removemake "$@"; }

# --- flatpak (flathub) ---
flatpak_installed() { flatpak info "$1" &>/dev/null; }
flatpak_available() { flatpak remote-info flathub "$1" &>/dev/null; }
flatpak_install()   { flatpak install -y --noninteractive flathub "$@"; }

flathub_present() { flatpak remotes | grep -q '^flathub'; }
flathub_add()     { flatpak remote-add --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo; }
