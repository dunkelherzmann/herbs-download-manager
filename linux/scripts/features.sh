#!/bin/sh
# Feature tests on Linux: tests/features.scenario run by the installed Flatpak (a test copy with its own data folder)
# in a virtual KWin session, every step through the app's own command code. Prints one PASS or FAIL line per feature.
#   sh linux/scripts/features.sh     (after linux/scripts/build.sh --install)
set -u
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="$HOME/.cache/hdm-features"   # under home: the Flatpak has its own /tmp
rm -rf "$WORK"; mkdir -p "$WORK"
PORT=$((20000 + $$ % 20000))
PATH="$HOME/.cargo/bin:$PATH" cargo build -q --manifest-path "$ROOT/Cargo.toml" -p hd-mock
"$ROOT/target/debug/hd-mock" --features --port "$PORT" > "$WORK/mock.txt" 2>&1 &
MOCK=$!
trap 'kill $MOCK 2>/dev/null; wait $MOCK 2>/dev/null' EXIT
i=0; while ! grep -q '^sha256' "$WORK/mock.txt" && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
SUM="$(awk '/^sha256/ {print $3}' "$WORK/mock.txt")"
cp "$ROOT/tests/features.scenario" "$WORK/features.scenario"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
timeout 300 kwin_wayland --virtual --no-lockscreen --width 1300 --height 800 --socket hdm-features --exit-with-session \
    "flatpak run --env=HD_DATA_DIR=$WORK/data --env=HD_DOWNLOADS_DIR=$WORK/dl org.herbsdownloadmanager.app \
     --features $WORK/features.scenario $WORK/report.txt L=http://127.0.0.1:$PORT M1=http://127.0.0.1:$((PORT + 1)) \
     M2=http://127.0.0.1:$((PORT + 2)) DL=$WORK/dl SMALL_SHA256=$SUM" > "$WORK/session.log" 2>&1
cat "$WORK/report.txt" 2>/dev/null || { echo "the app wrote no report (see $WORK/session.log)"; exit 1; }
grep -q ', 0 failed' "$WORK/report.txt"
