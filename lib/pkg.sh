#!/usr/bin/env bash
# Package installation. One flow for pacman, AUR (yay) and flatpak:
#   1. per package: SKIPPED if installed, FAILED if not found in any source
#   2. one bulk install of everything remaining (one long wait, not many)
#   3. per package: SUCCESS or FAILED based on what is actually installed now
#
# Each backend provides three functions: <kind>_installed, <kind>_available, <kind>_install.

install_list() {
    local kind="$1" file="$2"
    local -a pkgs=() want=()
    mapfile -t pkgs < <(read_list "$file")

    local p
    for p in "${pkgs[@]}"; do
        if "${kind}_installed" "$p"; then
            status "$p" SKIPPED "already present"
        elif ! "${kind}_available" "$p"; then
            status "$p" FAILED "not found"
        else
            want+=("$p")
        fi
    done

    [[ ${#want[@]} -gt 0 ]] || return 0

    note "installing: ${want[*]}"
    local out reason
    out="$(mktemp)"
    "${kind}_install" "${want[@]}" 2>&1 | tee "$out" || true
    cat "$out" >> "$LOG_FILE"
    reason="$(grep -iE 'error|fail|not found' "$out" | tail -n 1 | sed 's/\x1b\[[0-9;]*m//g' | cut -c1-80 || true)"
    rm -f "$out"

    for p in "${want[@]}"; do
        if "${kind}_installed" "$p"; then
            status "$p" SUCCESS
        else
            status "$p" FAILED "${reason:-see log}"
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
