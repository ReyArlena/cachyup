#!/bin/sh
# Installs (or removes) cachyup from the latest GitHub release.
#
#   curl -fsSL https://reyarlena.github.io/cachyup/install.sh | sh     # install or update
#   sh install.sh --uninstall     # remove the binary
#   sh install.sh --purge         # remove the binary, settings, state and run history
#
# Installs to ~/.local/bin/cachyup (override with CACHYUP_BIN_DIR). Run it
# again at any time to update. cachyup runs as your normal user and asks for
# sudo itself, so this script refuses to run as root.
# To build from source instead, see the Develop section of the README.
set -e

REPO="ReyArlena/cachyup"
BIN_DIR="${CACHYUP_BIN_DIR:-$HOME/.local/bin}"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/cachyup"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/cachyup"
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/cachyup"

if [ "$(id -u)" = "0" ]; then
    echo "Run this as your normal user, not root: cachyup asks for sudo when it needs it." >&2
    exit 1
fi

case "$1" in
    --uninstall|--purge)
        if [ -e "$BIN_DIR/cachyup" ] || [ -L "$BIN_DIR/cachyup" ]; then
            rm -f "$BIN_DIR/cachyup"
            echo "Removed $BIN_DIR/cachyup"
        else
            echo "cachyup was not installed in $BIN_DIR"
        fi
        if [ "$1" = "--purge" ]; then
            rm -rf "$CONFIG_DIR" "$STATE_DIR" "$CACHE_DIR"
            echo "Removed $CONFIG_DIR, $STATE_DIR and $CACHE_DIR"
        else
            echo "Settings and run history kept in $CONFIG_DIR and $STATE_DIR (use --purge to remove)"
        fi
        exit 0
        ;;
    "") ;;
    *)
        echo "Usage: install.sh [--uninstall | --purge]" >&2
        exit 1
        ;;
esac

if ! command -v curl >/dev/null 2>&1; then
    echo "curl is required" >&2
    exit 1
fi

# CachyOS is x86_64 only.
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) ASSET="cachyup-linux-x86_64" ;;
    *)
        echo "No prebuilt binary for $(uname -s) $(uname -m)." >&2
        echo "Build from source instead: see the Develop section of the README." >&2
        exit 1
        ;;
esac
url="https://github.com/$REPO/releases/latest/download/$ASSET"

mkdir -p "$BIN_DIR"
# Download to a temp file first, then move into place, so a failed download
# never leaves a truncated binary behind. The move also replaces the symlink
# that older (bash) versions of cachyup put here.
tmp=$(mktemp "$BIN_DIR/.cachyup.XXXXXX")
trap 'rm -f "$tmp"' EXIT

echo "Downloading $url"
if ! curl -fsSL "$url" -o "$tmp"; then
    echo "Download failed. Check https://github.com/$REPO/releases, or build from source (see the README)." >&2
    exit 1
fi
chmod 755 "$tmp"
mv -f "$tmp" "$BIN_DIR/cachyup"
trap - EXIT

echo
echo "Installed $BIN_DIR/cachyup ($("$BIN_DIR/cachyup" --version 2>/dev/null || echo 'version unknown'))"
case ":$PATH:" in
    *":$BIN_DIR:"*)
        echo "Run: cachyup"
        ;;
    *)
        echo "Note: $BIN_DIR is not on your PATH. Add it, or run: $BIN_DIR/cachyup"
        ;;
esac
if [ -d "$HOME/.local/share/cachyup/.git" ]; then
    echo
    echo "The old bash version's clone is still at ~/.local/share/cachyup; it is no longer used and can be deleted."
fi
