#!/usr/bin/env bash
# App selection: which packages from packages/*.txt this machine wants.
#
# The lists in the repo say what exists. ~/.config/cachyup/apps says what this
# machine decided, one line per package:
#
#   pacman/discord          done
#   aur/spotify             install
#   flatpak/org.kde.ktouch  skip
#
#   install  wanted; cachyup has not managed to install it yet (retried each run)
#   done     cachyup installed it, or found it present, at least once. Hands off:
#            if it goes missing later you removed it on purpose, not reinstalled
#   skip     unchecked in the selection screen
#
# The file records what cachyup did and what you chose, never what the system
# has right now. Any list line not in the file is "new" and gets asked about at
# the start of the run; on the first run that is the whole list.

APPS_FILE="${XDG_CONFIG_HOME:-$HOME/.config}/cachyup/apps"
APP_KINDS=(pacman aur flatpak)   # each has packages/<kind>.txt and <kind>_* backends
declare -A APPS=()               # APPS[kind/name]=state

app_list_file() { echo "$BOOTSTRAP_DIR/packages/$1.txt"; }

# Package names in a list file, one per line (first word; the rest is ignored).
app_names() { read_list "$(app_list_file "$1")" | awk '{print $1}'; }

app_state() { echo "${APPS[$1/$2]:-}"; }
app_set()   { APPS["$1/$2"]="$3"; }

# Read the file. Entries for packages no longer in any list are dropped, so a
# package removed from a list and added back later is asked about again.
apps_load() {
    APPS=()
    [[ -f "$APPS_FILE" ]] || return 0
    local -A known=()
    local kind name
    for kind in "${APP_KINDS[@]}"; do
        while read -r name; do known["$kind/$name"]=1; done < <(app_names "$kind")
    done
    local key state
    while read -r key state; do
        [[ -n "$key" && -n "$state" && -n "${known[$key]:-}" ]] || continue
        APPS["$key"]="$state"
    done < <(read_list "$APPS_FILE")
}

apps_save() {
    local key
    for key in "${!APPS[@]}"; do
        printf '%-40s %s\n' "$key" "${APPS[$key]}"
    done | sort | write_file "$APPS_FILE"
}

# Record a decision for every name: chosen ones install, the rest skip.
# A chosen package that is already present is turned into `done` by install_list.
apps_decide() {
    local kind="$1" chosen="$2"; shift 2
    local name
    for name in "$@"; do
        if [[ "$chosen" == *" $name "* ]]; then
            app_set "$kind" "$name" install
        else
            app_set "$kind" "$name" skip
        fi
    done
}

# apps_choose HEADER PRESELECTED NAME...
#   gum multi-select. PRESELECTED is comma separated. Sets APPS_CHOSEN to the
#   chosen names padded with spaces, for `[[ "$APPS_CHOSEN" == *" $name "* ]]`.
#   gum draws on stderr (the terminal) and prints the selection on stdout.
#   Esc or Ctrl-C aborts the whole run: a cancelled screen must not be read as
#   "skip everything". Not called in a subshell, so die() ends the run.
APPS_CHOSEN=""
apps_choose() {
    local header="$1" preselected="$2"; shift 2
    local -a args=(--no-limit --height 20 --header "$header")
    [[ -n "$preselected" ]] && args+=(--selected="$preselected")
    local out rc=0
    out="$(gum choose "${args[@]}" "$@")" || rc=$?
    (( rc == 0 )) || die "app selection cancelled"
    APPS_CHOSEN=" $(echo "$out" | tr '\n' ' ') "
}

# New apps: list lines with no decision yet. One screen per kind, everything
# pre-checked. Sets APPS_NEW, APPS_NEW_INSTALL, APPS_NEW_SKIP for the status line.
APPS_NEW=0; APPS_NEW_INSTALL=0; APPS_NEW_SKIP=0
apps_ask_new() {
    local kind name
    local -a names
    for kind in "${APP_KINDS[@]}"; do
        names=()
        while read -r name; do
            [[ -n "$(app_state "$kind" "$name")" ]] || names+=("$name")
        done < <(app_names "$kind")
        [[ ${#names[@]} -gt 0 ]] || continue

        APPS_NEW=$(( APPS_NEW + ${#names[@]} ))
        echo
        apps_choose "New $kind apps added to cachyup. Install which? (x toggles, enter confirms)" '*' "${names[@]}"
        apps_decide "$kind" "$APPS_CHOSEN" "${names[@]}"
        for name in "${names[@]}"; do
            if [[ "$APPS_CHOSEN" == *" $name "* ]]; then APPS_NEW_INSTALL=$(( APPS_NEW_INSTALL + 1 ))
            else APPS_NEW_SKIP=$(( APPS_NEW_SKIP + 1 )); fi
        done
    done
    (( APPS_NEW > 0 )) && apps_save
    return 0
}

# Full selection: every list line, with `install` and `done` pre-checked.
# Chosen -> install (a present one becomes `done` again in install_list,
# a manually removed one gets reinstalled). Unchecked -> skip.
apps_ask_all() {
    local kind name state pre
    local -a names
    for kind in "${APP_KINDS[@]}"; do
        mapfile -t names < <(app_names "$kind")
        [[ ${#names[@]} -gt 0 ]] || continue
        pre=""
        for name in "${names[@]}"; do
            state="$(app_state "$kind" "$name")"
            [[ "$state" == install || "$state" == "done" ]] && pre+="${pre:+,}$name"
        done
        echo
        apps_choose "All $kind apps. Install which? (x toggles, enter confirms)" "$pre" "${names[@]}"
        apps_decide "$kind" "$APPS_CHOSEN" "${names[@]}"
    done
    apps_save
    return 0
}

# The one question asked every run. Default No, so Enter just carries on.
apps_offer_customise() {
    echo
    local rc=0
    gum confirm --default=false "Customise the full app selection?" || rc=$?
    case "$rc" in
        0) apps_ask_all; APPS_CUSTOMISED=1 ;;
        1) ;;
        *) die "cancelled" ;;
    esac
    return 0
}
APPS_CUSTOMISED=0

apps_summary() {
    local s
    if (( APPS_NEW == 0 )); then
        s="nothing new"
    else
        s="$APPS_NEW new: $APPS_NEW_INSTALL install, $APPS_NEW_SKIP skip"
    fi
    (( APPS_CUSTOMISED )) && s+=", customised"
    echo "$s"
}
