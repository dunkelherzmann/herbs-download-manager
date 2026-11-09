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
    APP="$ROOT/dist/Herb's Downloader.app/Contents/MacOS/HerbsDownloader"
    [ -x "$APP" ] || { echo "Build the app first: scripts/build-macos.sh" >&2; exit 1; }
    OUT="$("$APP" --self-check)"
    echo "$OUT"
    case "$OUT" in
        "core "*" connections 32 per-server 32 choices 1-32") ;;
        *) echo "Self-check gave unexpected output" >&2; exit 1 ;;
    esac
fi
printf 'All tests passed\n'
