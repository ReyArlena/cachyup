#!/usr/bin/env bash
# Entry point. Clones (or updates) the repo, then hands off to run.sh.
#
# First run on a fresh machine (works from fish, bash or zsh):
#   curl -fsSL https://reyarlena.github.io/cachyup/install.sh | bash
# with options for run.sh:
#   curl -fsSL https://reyarlena.github.io/cachyup/install.sh | bash -s -- --tags dev
#
# Every run after that, from the `cachyup` command run.sh puts on PATH
# (a symlink to this file inside the clone). Running from inside a clone
# updates and uses that clone; running via curl uses ~/.local/share/cachyup.
#
# The whole script is a function called on the last line, so bash parses all
# of it before running any of it: a download cut short runs nothing.
set -euo pipefail

main() {
    local repo_url="${CACHYUP_REPO:-https://github.com/ReyArlena/cachyup.git}"
    local install_dir="${CACHYUP_DIR:-$HOME/.local/share/cachyup}"

    # Running from a clone (via the symlink or directly)? Use that clone.
    # Piped from curl, $0 is just "bash" and must not be resolved against cwd.
    local self_dir
    if [[ "$0" == */* && -f "$0" ]]; then
        self_dir="$(dirname "$(readlink -f "$0")")"
        if [[ -f "$self_dir/run.sh" ]]; then
            install_dir="$self_dir"
        fi
    fi

    if ! command -v git &>/dev/null; then
        echo "==> Installing git"
        sudo pacman -S --needed --noconfirm git
    fi

    if [[ -d "$install_dir/.git" ]]; then
        echo "==> Updating $install_dir"
        git -C "$install_dir" pull --ff-only
    else
        echo "==> Cloning into $install_dir"
        git clone "$repo_url" "$install_dir"
    fi

    # When piped from curl, stdin is the script itself. Give run.sh the terminal.
    exec bash "$install_dir/run.sh" "$@" </dev/tty
}

main "$@"
