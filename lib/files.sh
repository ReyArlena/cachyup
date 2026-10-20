#!/usr/bin/env bash
# File and config helpers.
#
# Symlink rule: this tool never replaces a symlink with a regular file. If a
# target path is a symlink (e.g. a stow-managed dotfile), the write goes THROUGH
# the link into the real file. Always write with write_file, copy_file or
# json_set; never mv, install or ln -sf over an existing path.

# write_file FILE   (content on stdin)
#   Creates parent dirs, writes through symlinks, keeps existing permissions.
write_file() {
    mkdir -p "$(dirname "$1")"
    cat > "$1"
}

# copy_file SRC DST
copy_file() { write_file "$2" < "$1"; }

# json_get FILE JQ_PATH   -> prints value, or "null" when missing/invalid
json_get() {
    [[ -f "$1" ]] || { echo null; return; }
    jq -r "$2 // null" "$1" 2>/dev/null || echo null
}

# json_set FILE JQ_PATH JSON_VALUE
#   Sets a key in a JSON file, creating the file if missing.
#   jq runs to completion first; the file is only touched if jq succeeded.
json_set() {
    local file="$1" path="$2" value="$3" tmp rc=0
    tmp="$(mktemp)"
    if [[ -f "$file" ]]; then
        jq "$path = $value" "$file" > "$tmp" || rc=$?
    else
        jq -n "$path = $value" > "$tmp" || rc=$?
    fi
    (( rc == 0 )) && write_file "$file" < "$tmp"
    rm -f "$tmp"
    return "$rc"
}
