#!/usr/bin/env bash
# Logging. One file per run, $LOG_FILE, holding everything the screen showed:
# headings, command output, and a status line per step. `grep FAILED` finds a
# failure with its command output directly above it.

GREEN='\033[0;32m'
YELLOW='\033[0;33m'
RED='\033[0;31m'
BOLD='\033[1m'
DIM='\033[2m'
BANNER='\033[30;47m'   # black on white (ANSI 16, follows the terminal theme)
RESET='\033[0m'

LOG_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/cachyup"
STATUS_WIDTH=44

log_init() {
    mkdir -p "$LOG_DIR"
    LOG_FILE="$LOG_DIR/$(date +%Y%m%d-%H%M%S).log"
    : > "$LOG_FILE"
    # Private scratch file: only headings and status lines go here, so the
    # end-of-run summary never has to parse command output. Removed on exit.
    SUMMARY_FILE="$(mktemp)"
    trap cleanup EXIT
    trap on_interrupt INT
    export LOG_FILE SUMMARY_FILE
}

on_interrupt() {
    echo
    echo -e "${RED}Interrupted.${RESET} Log: $LOG_FILE"
    printf '\nINTERRUPTED (Ctrl-C)\n' >> "$LOG_FILE"
    exit 130
}

# The one EXIT trap. Anything that needs tearing down at exit goes here.
cleanup() {
    rm -f "$SUMMARY_FILE"
    [[ -n "${SUDO_KEEPALIVE_PID:-}" ]] && kill "$SUDO_KEEPALIVE_PID" 2>/dev/null
    return 0
}

# Phase heading.
heading() {
    echo
    echo -e "${BOLD}==> $1${RESET}"
    printf '\n==> %s\n' "$1" >> "$LOG_FILE"
    printf 'H\t%s\n' "$1" >> "$SUMMARY_FILE"
}

# Full-width coloured divider line, e.g. before the summary.
banner() {
    local cols
    cols="$(tput cols 2>/dev/null </dev/tty || echo 80)"
    printf "${BANNER}%-${cols}s${RESET}\n" " $1"
}

# Dim informational line.
note() {
    echo -e "${DIM}    $1${RESET}"
    printf '    %s\n' "$1" >> "$LOG_FILE"
}

# status NAME RESULT [REASON]
#   RESULT is SUCCESS | SKIPPED | FAILED | PASS
#   PASS is for checks that change nothing; it is replayed in the summary but
#   not counted, so "N succeeded" only ever means something was installed or changed.
#   Prints "NAME.........RESULT (REASON)".
status() {
    local name="$1" result="$2" reason="${3:-}"
    local pad=$(( STATUS_WIDTH - ${#name} ))
    (( pad < 3 )) && pad=3
    local dots
    dots="$(printf '%*s' "$pad" '' | tr ' ' '.')"

    local colour
    case "$result" in
        SUCCESS|PASS) colour="$GREEN" ;;
        SKIPPED) colour="$YELLOW" ;;
        FAILED)  colour="$RED" ;;
        *)       colour="$RESET" ;;
    esac

    local suffix=""
    [[ -n "$reason" ]] && suffix=" ($reason)"

    echo -e "${name}${dots}${colour}${result}${RESET}${suffix}"
    printf '%s%s%s%s\n' "$name" "$dots" "$result" "$suffix" >> "$LOG_FILE"
    printf 'S\t%s\t%s%s%s%s\n' "$result" "$name" "$dots" "$result" "$suffix" >> "$SUMMARY_FILE"
}

# run_logged CMD [ARGS...]
#   Runs the command with its output streamed to the screen and appended to the
#   log as it happens (so nothing is lost on Ctrl-C). Sets RUN_LAST_LINE to the
#   last non-empty line of output, for FAILED reasons. Returns the command's
#   exit status.
RUN_LAST_LINE=""
run_logged() {
    local out rc=0
    out="$(mktemp)"
    "$@" 2>&1 | tee -a "$out" "$LOG_FILE" || rc=$?
    RUN_LAST_LINE="$(last_line "$out")"
    rm -f "$out"
    return "$rc"
}

# step NAME CHECK_FN ACTION_FN
#   CHECK_FN and ACTION_FN are function (or command) names, called with no
#   arguments. Wrap anything that needs arguments or quoting in a function.
#   If CHECK_FN succeeds the step is SKIPPED. A check may set SKIP_REASON before
#   returning 0 to replace the default "already present" (e.g. "using DMS Open").
#   Otherwise ACTION_FN runs and the step is SUCCESS or FAILED (reason = last
#   line of output).
step() {
    local name="$1" check="$2" action="$3"

    SKIP_REASON=""
    if "$check"; then
        status "$name" SKIPPED "${SKIP_REASON:-already present}"
        return 0
    fi

    note "running: $name"
    if run_logged "$action"; then
        status "$name" SUCCESS
    else
        status "$name" FAILED "$RUN_LAST_LINE"
    fi
}

# Last non-empty line of a file, ANSI stripped, trimmed, truncated.
last_line() {
    sed 's/\x1b\[[0-9;]*m//g' "$1" | grep -v '^[[:space:]]*$' | tail -n 1 \
        | sed 's/^[[:space:]]*//' | cut -c1-80
}

# End-of-run recap: every heading and status line again, back to back with no
# command output between them, then counts. PASS lines are shown but not
# counted, so "N succeeded" only ever means something was installed or changed.
summary() {
    local ok skip fail
    ok=$(grep -c $'^S\tSUCCESS\t' "$SUMMARY_FILE" || true)
    skip=$(grep -c $'^S\tSKIPPED\t' "$SUMMARY_FILE" || true)
    fail=$(grep -c $'^S\tFAILED\t' "$SUMMARY_FILE" || true)

    echo
    banner "==> Summary"
    local kind result line
    while IFS=$'\t' read -r kind result line; do
        if [[ "$kind" == "H" ]]; then
            echo
            echo -e "${BOLD}==> ${result}${RESET}"
        else
            line="${line/.SUCCESS/.${GREEN}SUCCESS${RESET}}"
            line="${line/.PASS/.${GREEN}PASS${RESET}}"
            line="${line/.SKIPPED/.${YELLOW}SKIPPED${RESET}}"
            line="${line/.FAILED/.${RED}FAILED${RESET}}"
            echo -e "$line"
        fi
    done < "$SUMMARY_FILE"

    echo
    echo -e "${GREEN}${ok} succeeded${RESET}, ${YELLOW}${skip} skipped${RESET}, ${RED}${fail} failed${RESET}"
    printf '\n==> Summary\n%s succeeded, %s skipped, %s failed\n' "$ok" "$skip" "$fail" >> "$LOG_FILE"

    if (( fail > 0 )); then
        echo "Command output for each failure is in the log, directly above its FAILED line."
    else
        echo -e "${GREEN}Nothing failed.${RESET} No need to check the log."
    fi
}

on_unexpected_error() {
    local file="$1" line="$2" cmd="$3"
    echo
    echo -e "${RED}Unexpected error in ${file##*/} line $line: $cmd${RESET}"
    printf '\nABORTED: unexpected error in %s line %s: %s\n' "${file##*/}" "$line" "$cmd" >> "$LOG_FILE"
    echo "Log: $LOG_FILE"
    exit 1
}
