#!/bin/sh
# Feature tests on the Mac: tests/features.scenario run by the built app (a test copy with its own data folder, never
# Herbert's), every step through the app's own command code. Prints one PASS or FAIL line per feature.
#   macos/scripts/features.sh        (after macos/scripts/build.sh)
set -u
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APP="$ROOT/dist/Herb's Download Manager.app/Contents/MacOS/HerbsDownloadManager"
WORK="$(mktemp -d)"
PORT=$((20000 + $$ % 20000))
cargo build -q --manifest-path "$ROOT/Cargo.toml" -p hd-mock
"$ROOT/target/debug/hd-mock" --features --port "$PORT" > "$WORK/mock.txt" 2>&1 &
MOCK=$!
trap 'kill $MOCK 2>/dev/null; wait $MOCK 2>/dev/null; rm -rf "$WORK"' EXIT
i=0; while ! grep -q '^sha256' "$WORK/mock.txt" && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
SUM="$(awk '/^sha256/ {print $3}' "$WORK/mock.txt")"
HD_DATA_DIR="$WORK/data" HD_DOWNLOADS_DIR="$WORK/dl" "$APP" --features "$ROOT/tests/features.scenario" "$WORK/report.txt" \
    "L=http://127.0.0.1:$PORT" "M1=http://127.0.0.1:$((PORT + 1))" "M2=http://127.0.0.1:$((PORT + 2))" \
    "DL=$WORK/dl" "SMALL_SHA256=$SUM" >/dev/null 2>&1
STATUS=$?
cat "$WORK/report.txt" 2>/dev/null || echo "the app wrote no report"
exit $STATUS
