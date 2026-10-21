#!/usr/bin/env bash
# Small helpers shared by every phase and script.

die() {
    echo -e "${RED}ERROR: $1${RESET}" >&2
    printf 'ABORTED: %s\n' "$1" >> "${LOG_FILE:-/dev/null}"
    exit 1
}

has_cmd() { command -v "$1" &>/dev/null; }

# require NAME CHECK_CMD MESSAGE_ON_FAILURE
#   Logs the check as PASS (not counted as a change); aborts the run if it fails.
require() {
    local name="$1" check="$2" msg="$3"
    if $check; then
        status "$name" PASS
    else
        status "$name" FAILED "$msg"
        die "$msg"
    fi
}

# check NAME CHECK_CMD MESSAGE_ON_FAILURE
#   Like require, but a failure is logged and the run continues. The check
#   function may set CHECK_REASON to replace MESSAGE_ON_FAILURE.
check() {
    local name="$1" check="$2" msg="$3"
    CHECK_REASON=""
    if $check; then
        status "$name" PASS
    else
        status "$name" FAILED "${CHECK_REASON:-$msg}"
    fi
}

is_cachyos() { grep -q '^ID=cachyos' /etc/os-release; }
not_root()   { [[ $EUID -ne 0 ]]; }
has_internet() { curl -fsS --max-time 8 -o /dev/null https://github.com; }

# Ask for sudo once, then keep the ticket alive in the background until exit
# (killed by cleanup() in lib/log.sh).
sudo_keepalive() {
    sudo -v || return 1
    ( while true; do sudo -n true; sleep 50; kill -0 "$$" 2>/dev/null || exit; done ) &
    SUDO_KEEPALIVE_PID=$!
}

# Read a list file: one entry per line, blank lines and # comments ignored.
# Single sed so an empty result is exit 0 (grep would exit 1 and trip pipefail/ERR).
read_list() { sed -E '/^[[:space:]]*(#|$)/d; s/[[:space:]]*#.*$//' "$1"; }

# run_scripts DIR
#   Each DIR/*.sh defines: NAME, check(), run()  (check() shadows the helper
#   above inside the subshell, which is fine: scripts never need it).
#   Sourced in a subshell so scripts cannot leak functions or variables.
run_scripts() {
    local dir="$1" f
    for f in "$dir"/*.sh; do
        [[ -e "$f" ]] || continue
        (
            source "$f"
            [[ -n "${NAME:-}" ]] || die "$f does not set NAME"
            step "$NAME" check run
        )
    done
}
