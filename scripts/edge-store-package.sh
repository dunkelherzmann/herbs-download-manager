#!/bin/sh
# Builds what the Edge Add-ons listing needs (SPECIFICATION.md §13): the extension as a zip, and the 300 × 300 store
# logo from icon/icon.svg. Needs a Mac (AppKit draws the SVG). Writes to dist/edge-store/ (OUT to change it).
set -eu
cd "$(dirname "$0")/.."
OUT=${OUT:-dist/edge-store}
VERSION=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' extension/manifest.json | head -1)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$OUT"
rm -f "$OUT/herbs-dm-extension-$VERSION.zip"
(cd extension && zip -q -r -X "$OLDPWD/$OUT/herbs-dm-extension-$VERSION.zip" . -x '.*')
swiftc -O -o "$TMP/render" icon/render.swift
"$TMP/render" icon/icon.svg "$OUT/logo-300.png" 300 > /dev/null
echo "$OUT/herbs-dm-extension-$VERSION.zip"
echo "$OUT/logo-300.png"
