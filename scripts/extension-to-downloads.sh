#!/bin/sh
# Puts the browser extension in the Downloads folder as "Herb's DM extension <version>" (Herbert, 2026-11-10: each new
# version of the extension goes there with its version in the name), for Load Unpacked. A version already there is
# left alone. Run by macos/scripts/build.sh and linux/scripts/build.sh when they install.
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
