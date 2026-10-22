#!/usr/bin/env bash
# Package installation. One flow for pacman, AUR (yay) and flatpak, one
# package at a time so every package gets its own honest result:
#   SKIPPED  already present, deselected, or removed manually after cachyup
#            installed it (see lib/apps.sh)
#   FAILED   not found in any source, or the install ran and it is still absent
#   SUCCESS  installed now
#
# Each backend provides three functions: <kind>_installed, <kind>_available, <kind>_install.

# install_list KIND FILE [always]
#   Honours the app selection unless the third argument is `always`
#   (prerequisites: never asked about, always installed).
install_list() {
    local kind="$1" file="$2" mode="${3:-selected}"
    local -a names=()
    mapfile -t names < <(read_list "$file" | awk '{print $1}')

    local p state
    for p in "${names[@]}"; do
        state=install
        [[ "$mode" == always ]] || state="$(app_state "$kind" "$p")"
        case "$state" in
            skip)
                status "$p" SKIPPED "deselected"
                continue ;;
            done)
                if "${kind}_installed" "$p"; then
                    status "$p" SKIPPED "already present"
                else
                    status "$p" SKIPPED "removed manually"
                fi
                continue ;;
        esac

        # install, or no decision recorded (treated as install)
        if "${kind}_installed" "$p"; then
            status "$p" SKIPPED "already present"
            [[ "$mode" == always ]] || app_set "$kind" "$p" "done"
        elif ! "${kind}_available" "$p"; then
            status "$p" FAILED "not found"
        else
            note "installing: $p"
            run_logged "${kind}_install" "$p" || true
            if "${kind}_installed" "$p"; then
                status "$p" SUCCESS
                [[ "$mode" == always ]] || app_set "$kind" "$p" "done"
            else
                status "$p" FAILED "${RUN_LAST_LINE:-see log}"
            fi
        fi
    done
    [[ "$mode" == always ]] || apps_save
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
