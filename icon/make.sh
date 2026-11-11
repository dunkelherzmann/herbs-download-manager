#!/bin/sh
# Builds every platform's app icon from icon.svg (needs a Mac: AppKit draws the SVG; ImageMagick packs the .ico).
#   macos/Resources/AppIcon.icns, linux/icons/hicolor/<size>/apps/…, windows/icons/app.ico
set -eu
cd "$(dirname "$0")"
ROOT=..
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
swiftc -O -o "$TMP/render" render.swift
render() { "$TMP/render" icon.svg "$1" "$2" > /dev/null; }

# macOS
SET="$TMP/AppIcon.iconset"
mkdir -p "$SET" "$ROOT/macos/Resources"
for s in 16 32 128 256 512; do
  render "$SET/icon_${s}x${s}.png" $s
  render "$SET/icon_${s}x${s}@2x.png" $((s * 2))
done
iconutil -c icns -o "$ROOT/macos/Resources/AppIcon.icns" "$SET"

# Linux (Flatpak id org.herbsdownloadmanager.app)
for s in 16 24 32 48 64 128 256 512; do
  d="$ROOT/linux/icons/hicolor/${s}x${s}/apps"
  mkdir -p "$d"
  render "$d/org.herbsdownloadmanager.app.png" $s
done
mkdir -p "$ROOT/linux/icons/hicolor/scalable/apps"
cp icon.svg "$ROOT/linux/icons/hicolor/scalable/apps/org.herbsdownloadmanager.app.svg"

# Windows
mkdir -p "$ROOT/windows/icons"
for s in 16 20 24 32 40 48 64 256; do render "$TMP/w$s.png" $s; done
magick "$TMP"/w16.png "$TMP"/w20.png "$TMP"/w24.png "$TMP"/w32.png "$TMP"/w40.png "$TMP"/w48.png "$TMP"/w64.png \
  "$TMP"/w256.png "$ROOT/windows/icons/app.ico"
# The browser extension's toolbar button: the square without the macOS margin around it, so it is as big as the
# other extensions' buttons (Herbert, 2026-11-11: it looked smaller). The Extensions page keeps the full icon.
sed 's/viewBox="0 0 1024 1024"/viewBox="96 96 832 832"/' icon.svg > "$TMP/toolbar.svg"
for s in 16 24 32; do "$TMP/render" "$TMP/toolbar.svg" "$ROOT/extension/icons/toolbar-$s.png" $s > /dev/null; done
echo "Icons written."
