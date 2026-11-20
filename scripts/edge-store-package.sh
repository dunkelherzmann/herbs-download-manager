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
# Edge Add-ons refuses two fields the other ways of loading the extension need: `key` (it fixes the id when loaded
# unpacked; the store gives its own) and `background.scripts` (Firefox's background page). The package leaves them out.
cp -R extension "$TMP/extension"
python3 -I - "$TMP/extension/manifest.json" <<'PY'
import json, sys
path = sys.argv[1]
manifest = json.load(open(path, encoding="utf-8"))
manifest.pop("key", None)
manifest["background"].pop("scripts", None)
json.dump(manifest, open(path, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
PY
(cd "$TMP/extension" && zip -q -r -X "$OLDPWD/$OUT/herbs-dm-extension-$VERSION.zip" . -x '.*')
swiftc -O -o "$TMP/render" icon/render.swift
"$TMP/render" icon/icon.svg "$OUT/logo-300.png" 300 > /dev/null
echo "$OUT/herbs-dm-extension-$VERSION.zip"
echo "$OUT/logo-300.png"
