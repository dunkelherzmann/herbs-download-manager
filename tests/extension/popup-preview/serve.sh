#!/bin/sh
# Shows the extension's list outside the browser, with stand-in items (stub.js), to check its look and its keys in
# the app's built-in browser: copies extension/ to a scratch folder, loads stub.js before popup.js, and serves it on
# http://127.0.0.1:8765/popup.html (a file:// page can't be scripted there). Ctrl+C stops it.
set -eu
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
DIR="${TMPDIR:-/tmp}/hdm-popup-preview"
rm -rf "$DIR"
cp -R "$ROOT/extension" "$DIR"
cp "$ROOT/tests/extension/popup-preview/stub.js" "$DIR/stub.js"
sed -i.bak 's#<script src="popup.js"></script>#<script src="stub.js"></script><script src="popup.js"></script>#' "$DIR/popup.html"
echo "http://127.0.0.1:8765/popup.html"
cd "$DIR" && exec python3 -m http.server 8765 --bind 127.0.0.1
