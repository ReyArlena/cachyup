#!/usr/bin/env bash
# Reclaim space after the updates, then report things that need a human:
# .pacnew files, a pending reboot, AUR packages built against old libraries.
# Nothing here is destructive beyond caches and packages nothing depends on.

heading "Cleanup"

orphans_none()   { [[ -z "$(pacman -Qtdq)" ]] && { SKIP_REASON="none"; return 0; }; return 1; }
orphans_remove() { pacman -Qtdq | sudo pacman -Rns --noconfirm -; }
step "Orphaned packages" orphans_none orphans_remove

# Keep 2 versions of installed packages, none of uninstalled ones (same policy
# as cachy-update, which keeps 3 and 0).
cache_candidates() { { paccache -dk2; paccache -duk0; } 2>/dev/null | sed -n 's/.*: \([0-9]*\) candidate.*/\1/p' | awk '{ n += $1 } END { print n + 0 }'; }
cache_tidy()  { [[ "$(cache_candidates)" -eq 0 ]] && { SKIP_REASON="nothing to prune"; return 0; }; return 1; }
cache_prune() { sudo paccache -rk2 && sudo paccache -ruk0; }
step "Package cache" cache_tidy cache_prune

# yay keeps a git clone per AUR package under ~/.cache/yay, with sources and
# built packages inside. Prune whole package dirs, never single files: deleting
# old files out of a clone leaves a dir without .git, or with a hollow .git
# (HEAD and objects gone), and yay's next fetch of that package fails. A dir
# goes when nothing in it was touched for KEEP_DAYS (so a rebuild or retry
# right after a run still has its files), or when its clone is already broken.
YAY_CACHE="$HOME/.cache/yay"
YAY_CACHE_KEEP_DAYS=30
# Is DIR itself a usable git repo (work tree or bare)? A gutted repo is not
# recognised by git, which then walks up and reports an enclosing one, so
# compare what git found against DIR rather than trusting the exit status.
git_repo_is() {
    local dir found
    dir=$(realpath "$1")
    found=$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null) && [[ "$found" == "$dir" ]] && return 0
    found=$(git -C "$dir" rev-parse --absolute-git-dir 2>/dev/null) && [[ "$found" == "$dir" ]]
}
# A package dir is broken when its AUR clone is gone, or when a bare mirror of
# a VCS source inside it (makepkg keeps these next to the PKGBUILD) is gone:
# makepkg then stops with "<dir> is not a clone of <url>".
yay_cache_dir_broken() {
    local d="$1" sub
    git_repo_is "$d" || return 0
    for sub in "$d"/*/; do
        sub="${sub%/}"
        [[ -f "$sub/config" && -f "$sub/description" ]] || continue
        git_repo_is "$sub" || return 0
    done
    return 1
}
yay_cache_stale_dirs() {
    local d
    for d in "$YAY_CACHE"/*/; do
        d="${d%/}"
        [[ -d "$d" ]] || continue
        if yay_cache_dir_broken "$d"; then
            echo "$d"
        elif [[ -z "$(find "$d" -mtime -"$YAY_CACHE_KEEP_DAYS" -print -quit)" ]]; then
            echo "$d"
        fi
    done
    return 0
}
yay_cache_tidy() {
    [[ -d "$YAY_CACHE" ]] || { SKIP_REASON="empty"; return 0; }
    [[ -z "$(yay_cache_stale_dirs)" ]] \
        && { SKIP_REASON="nothing older than ${YAY_CACHE_KEEP_DAYS}d"; return 0; }
    return 1
}
yay_cache_prune() {
    du -sh "$YAY_CACHE"
    local d
    while IFS= read -r d; do
        if yay_cache_dir_broken "$d"; then
            echo "broken clone: ${d##*/}"
        else
            echo "stale: ${d##*/}"
        fi
        rm -rf -- "$d"
    done < <(yay_cache_stale_dirs)
    du -sh "$YAY_CACHE"
}
step "yay build cache (older than ${YAY_CACHE_KEEP_DAYS}d)" yay_cache_tidy yay_cache_prune

# flatpak has no dry run; answering "n" to the prompt lists what it would remove.
flatpak_unused_none()   { echo n | flatpak uninstall --unused 2>&1 | grep -q 'Nothing unused' && { SKIP_REASON="none"; return 0; }; return 1; }
flatpak_unused_remove() { flatpak uninstall --unused -y --noninteractive; }
if has_cmd flatpak; then
    step "Unused flatpak runtimes" flatpak_unused_none flatpak_unused_remove
fi

# cachyos-rate-mirrors rewrites these four files on every rating, so a .pacnew
# for one of them is just the package's static default list: nothing to merge,
# the rating already tests the live mirror list from the CachyOS API. Only a
# .pacnew whose live file carries the rate-mirrors header is dropped, proof the
# file is generated and not hand-edited; anything else stays for pacdiff.
RATED_MIRRORLISTS=(
    /etc/pacman.d/mirrorlist
    /etc/pacman.d/cachyos-mirrorlist
    /etc/pacman.d/cachyos-v3-mirrorlist
    /etc/pacman.d/cachyos-v4-mirrorlist
)
rated_mirrorlist_pacnews() {
    has_cmd cachyos-rate-mirrors || return 0
    local f
    for f in "${RATED_MIRRORLISTS[@]}"; do
        [[ -f "$f.pacnew" ]] || continue
        grep -q '^# ARGS: rate-mirrors ' "$f" 2>/dev/null && echo "$f.pacnew"
    done
    return 0
}
mirrorlist_pacnew_none() {
    [[ -z "$(rated_mirrorlist_pacnews)" ]] && { SKIP_REASON="none"; return 0; }
    return 1
}
mirrorlist_pacnew_remove() {
    local f
    while IFS= read -r f; do
        echo "dropping $f: live file is rate-mirrors output"
        sudo rm -- "$f"
    done < <(rated_mirrorlist_pacnews)
}
step "Rated mirrorlist .pacnew files" mirrorlist_pacnew_none mirrorlist_pacnew_remove

heading "Post-update checks"

no_pacnew() {
    local -a files=()
    mapfile -t files < <(pacdiff -o 2>/dev/null)
    [[ ${#files[@]} -eq 0 ]] && return 0
    CHECK_REASON="${#files[@]} to merge, run: DIFFPROG=meld pacdiff --sudo"
    local f; for f in "${files[@]}"; do note "$f"; done
    return 1
}
check ".pacnew files" no_pacnew "see above"

# The modules dir of the running kernel is removed when the kernel package is
# upgraded, so its absence means the running kernel is no longer installed.
# kernel-modules-hook keeps those modules around, so with it installed this
# test cannot tell; same guard as cachy-update.
kernel_current() {
    pacman -Qq kernel-modules-hook &>/dev/null && return 0
    [[ -d "/usr/lib/modules/$(uname -r)" ]]
}
check "Reboot not required" kernel_current "kernel was updated, reboot when convenient"

# Running services whose binaries or libraries were replaced by the update.
# checkservices (archlinux-contrib) needs root; -F -P -R = no failed-service
# list, no pacdiff, no restarting: report only.
no_stale_services() {
    has_cmd checkservices || { CHECK_REASON="checkservices not installed"; return 1; }
    local out
    out="$(sudo checkservices -F -P -R 2>/dev/null || true)"
    local -a svcs=()
    mapfile -t svcs < <(grep -oE "restart '[^']+'" <<< "$out" | sed "s/restart '//; s/'//" || true)
    [[ ${#svcs[@]} -eq 0 ]] && return 0
    CHECK_REASON="restart with: sudo systemctl restart ${svcs[*]}"
    return 1
}
check "No services need a restart" no_stale_services "see above"

# Started in the background by 10-preflight.sh after the AUR update.
# Packages that just repack upstream binaries cannot be fixed by a rebuild
# (same tarball in, same package out); they are listed but not advised.
PREBUILT_PATTERNS=( '*-bin' '*-appimage' 'phpstorm' 'goland' 'rustrover' 'clion' 'datagrip' 'webstorm' 'rider' 'pycharm*' 'intellij*' 'google-chrome' 'spotify' 'discord-ptb' )
is_prebuilt() {
    local pat
    for pat in "${PREBUILT_PATTERNS[@]}"; do [[ "$1" == $pat ]] && return 0; done
    return 1
}
no_rebuilds() {
    [[ -n "${REBUILD_PID:-}" ]] || { CHECK_REASON="checkrebuild not installed"; return 1; }
    if kill -0 "$REBUILD_PID" 2>/dev/null; then
        note "waiting for checkrebuild to finish"
        wait "$REBUILD_PID" || true
    fi
    local -a flagged=() rebuild=() prebuilt=()
    mapfile -t flagged < <(awk '{print $2}' "$REBUILD_OUT")
    rm -f "$REBUILD_OUT"
    local p
    for p in "${flagged[@]}"; do
        if is_prebuilt "$p"; then prebuilt+=("$p"); else rebuild+=("$p"); fi
    done
    [[ ${#prebuilt[@]} -gt 0 ]] && note "flagged but prebuilt, a rebuild cannot help, wait for upstream: ${prebuilt[*]}"
    [[ ${#rebuild[@]} -eq 0 ]] && return 0
    CHECK_REASON="rebuild with: yay -S --rebuild ${rebuild[*]}"
    return 1
}
check "AUR packages up to date with libraries" no_rebuilds "see above"
