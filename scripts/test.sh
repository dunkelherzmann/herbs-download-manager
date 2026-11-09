#!/bin/sh
# Runs every automatic test on this machine: the Rust workspace (core, bindings, mock server), then on the Mac
# the built app's --self-check, which calls the core through the Swift bindings. Build first on the Mac.
# Usage: scripts/test.sh
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

printf 'Rust tests…\n'
cargo test --quiet --workspace 2>&1 | grep -E '^test result|FAILED|panicked|error' || true
cargo test --quiet --workspace >/dev/null 2>&1 || { echo "Rust tests failed (run: cargo test --workspace)" >&2; exit 1; }

if [ "$(uname)" = Darwin ]; then
    printf 'Mac app self-check…\n'
    APP="$ROOT/dist/Herb's Download Manager.app/Contents/MacOS/HerbsDownloadManager"
    [ -x "$APP" ] || { echo "Build the app first: macos/scripts/build.sh" >&2; exit 1; }
    OUT="$("$APP" --self-check)"
    echo "$OUT"
    case "$OUT" in
        "core "*" connections 32 per-server 32 choices 1-32") ;;
        *) echo "Self-check gave unexpected output" >&2; exit 1 ;;
    esac

    printf 'Mac app menus…\n'
    WORK="$(mktemp -d)"
    trap 'rm -rf "$WORK"' EXIT
    # A private data folder; the window is never shown. The run ends with ⌘Q sent through the in-window menu.
    HD_DATA_DIR="$WORK/data" "$APP" --menu-report "$WORK/menus.txt" >/dev/null 2>&1 &
    PID=$!
    i=0; while kill -0 "$PID" 2>/dev/null && [ "$i" -lt 30 ]; do sleep 0.5; i=$((i + 1)); done
    if kill -0 "$PID" 2>/dev/null; then kill "$PID"; echo "Menus: ⌘Q through the in-window menu didn't quit the app" >&2; exit 1; fi
    grep -qx "menu bar: Herb's DM" "$WORK/menus.txt" || { echo "Menus: the macOS menu bar holds more than the app menu" >&2; exit 1; }
    grep -qx "main menu: File, Edit, Tasks, View, Window, Help, Settings…, About Herb's Download Manager, Hide Herb's DM, Quit Herb's DM" "$WORK/menus.txt" \
        || { echo "Menus: the Main Menu (☰) is incomplete" >&2; cat "$WORK/menus.txt" >&2; exit 1; }
    mkdir -p "$ROOT/.build/test" && cp "$WORK/menus.txt.png" "$ROOT/.build/test/macos-window.png"
    echo "menus in the Main Menu (☰); ⌘Q reached it"
    # Every control explains itself: hover text and a VoiceOver name in every window (A11yAudit.swift).
    HD_DATA_DIR="$WORK/a11y" "$APP" --queue http://127.0.0.1:9/a.bin --a11y-report "$WORK/a11y.txt" >/dev/null 2>&1
    for window in "Herb's Download Manager" Settings Sign-ins "New Download" "Add 2 Links" "Batch Download" a.bin; do
        grep -q "^# checked:.*$window" "$WORK/a11y.txt" || { echo "Accessibility audit: the $window window didn't open" >&2; cat "$WORK/a11y.txt" >&2; exit 1; }
    done
    if grep -v '^#' "$WORK/a11y.txt" | grep -q .; then
        echo "Controls without hover text or a VoiceOver name:" >&2; grep -v '^#' "$WORK/a11y.txt" >&2; exit 1
    fi
    echo "every control has hover text and a VoiceOver name"
    # Feature tests: tests/features.scenario through the app's own commands (macos/scripts/features.sh).
    echo "Mac app features…"
    "$ROOT/macos/scripts/features.sh" || { echo "Feature tests failed" >&2; exit 1; }
fi
printf 'All tests passed\n'
