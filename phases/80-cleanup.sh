#!/usr/bin/env bash
# Reclaim space after the updates, then report things that need a human:
# .pacnew files, a pending reboot, AUR packages built against old libraries.
# Nothing here is destructive beyond caches and packages nothing depends on.

heading "Cleanup"

orphans_none()   { [[ -z "$(pacman -Qtdq)" ]] && { SKIP_REASON="none"; return 0; }; return 1; }
orphans_remove() { pacman -Qtdq | sudo pacman -Rns --noconfirm -; }
step "Orphaned packages" orphans_none orphans_remove

cache_tidy()  { paccache -dk2 2>/dev/null | grep -q 'no candidate' && { SKIP_REASON="nothing to prune"; return 0; }; return 1; }
cache_prune() { sudo paccache -rk2; }
step "Package cache (keep 2 versions)" cache_tidy cache_prune

yay_cache_empty() { [[ -z "$(ls -A "$HOME/.cache/yay" 2>/dev/null)" ]] && { SKIP_REASON="empty"; return 0; }; return 1; }
yay_cache_clear() { du -sh "$HOME/.cache/yay" && rm -rf "$HOME/.cache/yay"/*; }
step "yay build cache" yay_cache_empty yay_cache_clear

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
    CHECK_REASON="${#files[@]} to merge, run: sudo pacdiff"
    local f; for f in "${files[@]}"; do note "$f"; done
    return 1
}
check ".pacnew files" no_pacnew "see above"

# The modules dir of the running kernel is removed when the kernel package is
# upgraded, so its absence means the running kernel is no longer installed.
kernel_current() { [[ -d "/usr/lib/modules/$(uname -r)" ]]; }
check "Reboot not required" kernel_current "kernel was updated, reboot when convenient"

# Started in the background by 10-preflight.sh after the AUR update.
no_rebuilds() {
    [[ -n "${REBUILD_PID:-}" ]] || { CHECK_REASON="checkrebuild not installed"; return 1; }
    if kill -0 "$REBUILD_PID" 2>/dev/null; then
        note "waiting for checkrebuild to finish"
        wait "$REBUILD_PID" || true
    fi
    local -a pkgs=()
    mapfile -t pkgs < <(awk '{print $2}' "$REBUILD_OUT")
    rm -f "$REBUILD_OUT"
    [[ ${#pkgs[@]} -eq 0 ]] && return 0
    CHECK_REASON="rebuild with: yay -S --rebuild ${pkgs[*]}"
    return 1
}
check "AUR packages up to date with libraries" no_rebuilds "see above"
