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

# yay keeps every downloaded source and built package. Prune by age rather
# than wiping, so a rebuild or retry right after a run still has its files.
YAY_CACHE="$HOME/.cache/yay"
YAY_CACHE_KEEP_DAYS=30
yay_cache_tidy() {
    [[ -d "$YAY_CACHE" ]] || { SKIP_REASON="empty"; return 0; }
    [[ -z "$(find "$YAY_CACHE" -type f -mtime +"$YAY_CACHE_KEEP_DAYS" -print -quit)" ]] \
        && { SKIP_REASON="nothing older than ${YAY_CACHE_KEEP_DAYS}d"; return 0; }
    return 1
}
yay_cache_prune() {
    du -sh "$YAY_CACHE"
    find "$YAY_CACHE" -type f -mtime +"$YAY_CACHE_KEEP_DAYS" -delete
    find "$YAY_CACHE" -mindepth 1 -type d -empty -delete
    du -sh "$YAY_CACHE"
}
step "yay build cache (older than ${YAY_CACHE_KEEP_DAYS}d)" yay_cache_tidy yay_cache_prune

# flatpak has no dry run; answering "n" to the prompt lists what it would remove.
flatpak_unused_none()   { echo n | flatpak uninstall --unused 2>&1 | grep -q 'Nothing unused' && { SKIP_REASON="none"; return 0; }; return 1; }
flatpak_unused_remove() { flatpak uninstall --unused -y --noninteractive; }
if has_cmd flatpak; then
    step "Unused flatpak runtimes" flatpak_unused_none flatpak_unused_remove
fi

heading "Post-update checks"

no_pacnew() {
    local -a files=()
    mapfile -t files < <(pacdiff -o 2>/dev/null)
    [[ ${#files[@]} -eq 0 ]] && return 0
    CHECK_REASON="${#files[@]} to merge, see command above"
    local f; for f in "${files[@]}"; do note "$f"; done
    note "merge them with:  sudo DIFFPROG=meld pacdiff"
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
