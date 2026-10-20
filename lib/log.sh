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

LOG_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/linux-bootstrapper"
STATUS_WIDTH=44

# Status lines look like "name.....RESULT" with at least 3 dots; this pattern
# is what summary() counts, so command output cannot be mistaken for a status.
STATUS_RE='\.\.\.(SUCCESS|SKIPPED|FAILED|PASS)( \(|$)'

log_init() {
    mkdir -p "$LOG_DIR"
    LOG_FILE="$LOG_DIR/$(date +%Y%m%d-%H%M%S).log"
    : > "$LOG_FILE"
    export LOG_FILE
}

# Phase heading.
heading() {
    echo
    echo -e "${BOLD}==> $1${RESET}"
    printf '\n==> %s\n' "$1" >> "$LOG_FILE"
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
}

# step NAME CHECK_CMD ACTION_CMD
#   If CHECK_CMD succeeds the step is SKIPPED.
#   Otherwise ACTION_CMD runs with its output streamed to screen and log,
#   and the step is SUCCESS or FAILED (reason = last line of output).
step() {
    local name="$1" check="$2" action="$3"

    if $check; then
        status "$name" SKIPPED "already present"
        return 0
    fi

    note "running: $name"
    local out
    out="$(mktemp)"

    if $action 2>&1 | tee "$out"; then
        cat "$out" >> "$LOG_FILE"
        status "$name" SUCCESS
    else
        cat "$out" >> "$LOG_FILE"
        status "$name" FAILED "$(last_line "$out")"
    fi
    rm -f "$out"
}

# Last non-empty line of a file, ANSI stripped, trimmed, truncated.
last_line() {
    sed 's/\x1b\[[0-9;]*m//g' "$1" | grep -v '^[[:space:]]*$' | tail -n 1 \
        | sed 's/^[[:space:]]*//' | cut -c1-80
}

# End-of-run recap: every heading and status line from the log again, back to
# back with no command output between them, then counts. Tells the user whether
# the log is worth opening.
summary() {
    local ok skip fail
    ok=$(grep -cE "${STATUS_RE/(SUCCESS|SKIPPED|FAILED|PASS)/SUCCESS}" "$LOG_FILE" || true)
    skip=$(grep -cE "${STATUS_RE/(SUCCESS|SKIPPED|FAILED|PASS)/SKIPPED}" "$LOG_FILE" || true)
    fail=$(grep -cE "${STATUS_RE/(SUCCESS|SKIPPED|FAILED|PASS)/FAILED}" "$LOG_FILE" || true)

    echo
    banner "==> Summary"
    grep -E "^==> |${STATUS_RE}" "$LOG_FILE" | while IFS= read -r line; do
        if [[ "$line" == "==> "* ]]; then
            echo
            echo -e "${BOLD}${line}${RESET}"
        else
            line="${line/.SUCCESS/.${GREEN}SUCCESS${RESET}}"
            line="${line/.PASS/.${GREEN}PASS${RESET}}"
            line="${line/.SKIPPED/.${YELLOW}SKIPPED${RESET}}"
            line="${line/.FAILED/.${RED}FAILED${RESET}}"
            echo -e "$line"
        fi
    done

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
    local line="$1" cmd="$2"
    echo
    echo -e "${RED}Unexpected error at line $line: $cmd${RESET}"
    printf '\nABORTED: unexpected error at line %s: %s\n' "$line" "$cmd" >> "$LOG_FILE"
    echo "Log: $LOG_FILE"
    exit 1
}
