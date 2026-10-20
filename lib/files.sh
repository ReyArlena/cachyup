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

# ini_set FILE SECTION KEY VALUE
#   Sets KEY=VALUE under [SECTION] in an ini-style file (mimeapps.list, .desktop
#   files, ...). Replaces the key if present in that section, appends it to the
#   section otherwise, creates the section (and file) if missing. Writes via
#   write_file so symlinks are honoured. Comments and other sections untouched.
ini_set() {
    local file="$1" section="$2" key="$3" value="$4" tmp
    tmp="$(mktemp)"
    if [[ -f "$file" ]]; then
        awk -v s="[$section]" -v k="$key" -v v="$value" '
            BEGIN { insec = 0; done = 0 }
            /^\[/ {
                if (insec && !done) { print k "=" v; done = 1 }
                insec = ($0 == s)
                print; next
            }
            insec && substr($0, 1, length(k) + 1) == k "=" {
                if (!done) { print k "=" v; done = 1 }
                next
            }
            { print }
            END {
                if (!done) {
                    if (!insec) { if (NR > 0) print ""; print s }
                    print k "=" v
                }
            }
        ' "$file" > "$tmp"
    else
        printf '[%s]\n%s=%s\n' "$section" "$key" "$value" > "$tmp"
    fi
    write_file "$file" < "$tmp"
    rm -f "$tmp"
}

# ini_get FILE SECTION KEY   -> prints the value, or nothing if absent
ini_get() {
    local file="$1" section="$2" key="$3"
    [[ -f "$file" ]] || return 0
    awk -v s="[$section]" -v k="$key" '
        /^\[/ { insec = ($0 == s); next }
        insec && substr($0, 1, length(k) + 1) == k "=" { print substr($0, length(k) + 2); exit }
    ' "$file"
}
