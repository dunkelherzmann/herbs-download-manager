#!/bin/sh
# Puts the browser extension in the Downloads folder as "Herb's DM extension <version>" (Herbert, 2026-11-10: each new
# version of the extension goes there with its version in the name), for Load Unpacked. A version already there is
# left alone. On the Mac, each version's Edge Add-ons package goes there too, as "Herb's DM extension <version> - Edge
# store.zip", ready to upload as the listing's update (Herbert, 2026-11-26). Run by macos/scripts/build.sh and
# linux/scripts/build.sh when they install.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(sed -n 's/^ *"version": *"\([^"]*\)".*/\1/p' "$ROOT/extension/manifest.json" | head -1)"
DOWNLOADS="$(xdg-user-dir DOWNLOAD 2>/dev/null || true)"
[ -n "$DOWNLOADS" ] && [ "$DOWNLOADS" != "$HOME" ] || DOWNLOADS="$HOME/Downloads"
DEST="$DOWNLOADS/Herb's DM extension $VERSION"
if [ -d "$DEST" ]; then
    printf 'Extension %s already in %s\n' "$VERSION" "$DOWNLOADS"
else
    mkdir -p "$DOWNLOADS"
    cp -R "$ROOT/extension" "$DEST"
    printf 'Extension %s copied to %s\n' "$VERSION" "$DEST"
fi
if [ "$(uname)" = Darwin ]; then
    PACKAGE="$DOWNLOADS/Herb's DM extension $VERSION - Edge store.zip"
    if [ -f "$PACKAGE" ]; then
        printf 'Edge store zip %s already in %s\n' "$VERSION" "$DOWNLOADS"
    else
        PACKAGE="$PACKAGE" SOURCE="$ROOT/extension" sh "$ROOT/scripts/edge-store-package.sh" --zip-only >/dev/null
        printf 'Edge store zip %s written to %s\n' "$VERSION" "$PACKAGE"
    fi
fi
