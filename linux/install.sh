#!/bin/sh
# Builds Herb's Download Manager for Linux and installs it under a prefix (/app inside the Flatpak build).
# Usage: sh linux/install.sh PREFIX   (run from the repository root)
set -eu
PREFIX="${1:-/app}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/linux"
cargo build --release --locked 2>/dev/null || cargo build --release
install -Dm755 target/release/herbs-download-manager "$PREFIX/bin/herbs-download-manager"
install -Dm755 target/release/hdm "$PREFIX/bin/hdm"
install -Dm644 data/org.herbsdownloadmanager.app.desktop "$PREFIX/share/applications/org.herbsdownloadmanager.app.desktop"
install -Dm644 data/org.herbsdownloadmanager.app.metainfo.xml "$PREFIX/share/metainfo/org.herbsdownloadmanager.app.metainfo.xml"
for dir in icons/hicolor/*/apps; do
    for icon in "$dir"/*; do
        install -Dm644 "$icon" "$PREFIX/share/$dir/$(basename "$icon")"
    done
done
install -Dm644 "$ROOT/LICENSE" "$PREFIX/share/licenses/org.herbsdownloadmanager.app/LICENSE"
