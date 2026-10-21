#!/usr/bin/env bash
# Tags: optional groups of packages and tweaks, chosen per machine.
#
#   packages/*.txt     obs-studio   @streaming @full
#   scripts            check() { has_tag dev || { SKIP_REASON="tag: dev"; return 0; }; ... }
#
# A tagged line installs when any of its tags is active. Untagged lines always
# install. Active tags come from, in order: --tags on the command line (saved),
# the saved file ~/.config/cachyup/tags, or a one-time question in Preflight.
# No registry: the available tags are whatever the lists and scripts mention.

TAGS_FILE="${XDG_CONFIG_HOME:-$HOME/.config}/cachyup/tags"
declare -a CACHYUP_TAGS=()
CACHYUP_TAGS_ARG=""   # set by run.sh from --tags

has_tag() { [[ " ${CACHYUP_TAGS[*]} " == *" $1 "* ]]; }

# Every tag mentioned anywhere, one per line, sorted.
tags_available() {
    {
        for f in "$BOOTSTRAP_DIR"/packages/*.txt; do
            read_list "$f" | grep -oE '@[A-Za-z0-9_-]+' || true
        done
        grep -ohE 'has_tag +[A-Za-z0-9_-]+' "$BOOTSTRAP_DIR"/{conditional,installers,config}/*.sh 2>/dev/null \
            | awk '{print "@" $2}' || true
    } | tr -d '@' | sort -u
}

# Package names carrying TAG, comma separated (for the question).
tag_members() {
    local tag="$1"
    for f in "$BOOTSTRAP_DIR"/packages/*.txt; do
        read_list "$f" | awk -v t="@$tag" '{ for (i = 2; i <= NF; i++) if ($i == t) { print $1; break } }'
    done | sort -u | paste -sd ',' | sed 's/,/, /g'
}

# Split a list line into name and tags. Sets LINE_NAME and LINE_TAGS (array).
parse_list_line() {
    local -a words
    read -ra words <<< "$1"
    LINE_NAME="${words[0]}"
    LINE_TAGS=()
    local w
    for w in "${words[@]:1}"; do
        [[ "$w" == @* ]] && LINE_TAGS+=("${w#@}")
    done
}

# Does the current tag selection want this line? Sets LINE_SKIP_REASON if not.
line_wanted() {
    LINE_SKIP_REASON=""
    [[ ${#LINE_TAGS[@]} -eq 0 ]] && return 0
    local t
    for t in "${LINE_TAGS[@]}"; do
        has_tag "$t" && return 0
    done
    LINE_SKIP_REASON="tag: $(IFS=/; echo "${LINE_TAGS[*]}")"
    return 1
}

tags_save() {
    printf '%s\n' "$@" | write_file "$TAGS_FILE"
}

# Interactive one-time question (gum multi-select, arrows + space, Enter to confirm).
tags_ask() {
    local -a avail=()
    mapfile -t avail < <(tags_available)
    [[ ${#avail[@]} -gt 0 ]] || return 0

    local -a items=() chosen=()
    local t
    for t in "${avail[@]}"; do
        items+=("$(printf '%-12s %s' "$t" "$(tag_members "$t")")")
    done

    # gum draws its UI on stderr (the terminal) and prints the selection on
    # stdout, so capturing stdout is all that is needed.
    echo
    mapfile -t chosen < <(gum choose --no-limit \
        --header "Optional tags for this machine (x to toggle, enter to confirm):" \
        "${items[@]}" || true)

    CACHYUP_TAGS=()
    local line
    for line in "${chosen[@]}"; do
        [[ -n "$line" ]] || continue
        CACHYUP_TAGS+=("${line%% *}")
    done
    return 0
}

# Decide the active tags. Returns 0; the status line is printed by the caller.
tags_load() {
    if [[ "$CACHYUP_TAGS_ARG" == "ask" ]]; then
        tags_ask
        tags_save "${CACHYUP_TAGS[@]}"
    elif [[ -n "$CACHYUP_TAGS_ARG" ]]; then
        if [[ "$CACHYUP_TAGS_ARG" == "none" ]]; then
            CACHYUP_TAGS=()
        else
            IFS=', ' read -ra CACHYUP_TAGS <<< "$CACHYUP_TAGS_ARG"
        fi
        tags_save "${CACHYUP_TAGS[@]}"
    elif [[ -f "$TAGS_FILE" ]]; then
        mapfile -t CACHYUP_TAGS < <(read_list "$TAGS_FILE")
    else
        tags_ask
        tags_save "${CACHYUP_TAGS[@]}"
    fi
    return 0
}

tags_summary() {
    if [[ ${#CACHYUP_TAGS[@]} -eq 0 ]]; then echo "none"; else echo "${CACHYUP_TAGS[*]}"; fi
}
