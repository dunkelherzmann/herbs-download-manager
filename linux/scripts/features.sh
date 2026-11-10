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
# The app-observer skill watches the run when it's installed (HD_NO_OBSERVE=1 skips it): a crash, a write outside
# the test folder, or a change to the real app's data or the browsers' native-host folders fails the run.
OBSERVER="$HOME/.agents/library/app-observer/scripts/observer.py"
set --
if [ -f "$OBSERVER" ] && [ -z "${HD_NO_OBSERVE:-}" ]; then
    # Judged: the app and what it starts, not the KWin session around it. Its Flatpak cache folder is allowed.
    set -- python3 "$OBSERVER" run --out "$WORK/observer" --name features --app /app/bin/herbs-download-manager \
        --allow "$WORK/" --allow "$HOME/.var/app/org.herbsdownloadmanager.app/cache/" \
        --guard "$HOME/.var/app/org.herbsdownloadmanager.app/data" --guard "$HOME/.var/app/org.herbsdownloadmanager.app/config" \
        --guard "$HOME/.local/share/herbs-download-manager" \
        --guard "$HOME/.config/google-chrome/NativeMessagingHosts" --guard "$HOME/.config/chromium/NativeMessagingHosts" \
        --guard "$HOME/.mozilla/native-messaging-hosts" --
fi
"$@" timeout 300 kwin_wayland --virtual --no-lockscreen --width 1300 --height 800 --socket hdm-features --exit-with-session \
    "flatpak run --env=HD_DATA_DIR=$WORK/data --env=HD_DOWNLOADS_DIR=$WORK/dl org.herbsdownloadmanager.app \
     --features $WORK/features.scenario $WORK/report.txt L=http://127.0.0.1:$PORT M1=http://127.0.0.1:$((PORT + 1)) \
     M2=http://127.0.0.1:$((PORT + 2)) DL=$WORK/dl SMALL_SHA256=$SUM" > "$WORK/session.log" 2>&1
cat "$WORK/report.txt" 2>/dev/null || { echo "the app wrote no report (see $WORK/session.log)"; exit 1; }
grep '^observer: ' "$WORK/session.log"
! grep -q '^observer: PROBLEM' "$WORK/session.log" && grep -q ', 0 failed' "$WORK/report.txt"
