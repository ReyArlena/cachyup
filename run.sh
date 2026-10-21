#!/usr/bin/env bash
# Orchestrator, run by install.sh (and so by the `cachyup` command).
# Sources lib/*.sh then runs phases/*.sh in filename order.
# Safe to re-run: every step checks before acting and skips what is already done.
set -Eeuo pipefail  # -E: ERR trap fires inside functions and subshells too

BOOTSTRAP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export BOOTSTRAP_DIR

for lib in "$BOOTSTRAP_DIR"/lib/*.sh; do
    source "$lib"
done

usage() {
    cat <<'USAGE'
Usage: cachyup [--tags LIST | --tags ask | --tags none]

  --tags LIST   activate these tags (comma separated) and remember them
  --tags ask    ask the tag question again
  --tags none   no optional tags
With no option the saved tags are used, or you are asked once on first run.
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --tags)   [[ $# -ge 2 ]] || { usage; exit 2; }; CACHYUP_TAGS_ARG="$2"; shift 2 ;;
        --tags=*) CACHYUP_TAGS_ARG="${1#--tags=}"; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown option: $1" >&2; usage; exit 2 ;;
    esac
done

log_init
trap 'on_unexpected_error "${BASH_SOURCE[0]}" "$LINENO" "$BASH_COMMAND"' ERR

for phase in "$BOOTSTRAP_DIR"/phases/*.sh; do
    source "$phase"
done
