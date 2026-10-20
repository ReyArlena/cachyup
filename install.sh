#!/usr/bin/env bash
# Orchestrator. Sources lib/*.sh then runs phases/*.sh in filename order.
# Safe to re-run: every step checks before acting and skips what is already done.
set -Eeuo pipefail  # -E: ERR trap fires inside functions and subshells too

BOOTSTRAP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export BOOTSTRAP_DIR

for lib in "$BOOTSTRAP_DIR"/lib/*.sh; do
    source "$lib"
done

log_init
trap 'on_unexpected_error $LINENO "$BASH_COMMAND"' ERR

for phase in "$BOOTSTRAP_DIR"/phases/*.sh; do
    source "$phase"
done
