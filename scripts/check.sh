#!/bin/sh
# Checks a pushed branch on Herbert's three machines, in place of GitHub Actions: the repo is private, and GitHub
# charges for private repos' minutes (asked 2026-11-09: "If we need to pay to use CI then stop using it").
#
#   scripts/check.sh [BRANCH]    (default: the current branch, which must be pushed)
#
# - Mac (here): macos/scripts/build.sh and scripts/test.sh, as the old macOS job did, on the current checkout.
# - Linux (linux) and Windows (windows), over ssh: `cargo test --workspace` in a throwaway worktree of
#   origin/BRANCH (/tmp/hdm-check-BRANCH, %TEMP%\hdm-check-BRANCH), so their own checkouts stay on main; builds are cached in
#   ~/.cache/hdm-check-target.
# Prints one line per machine and exits non-zero if any failed. Logs go to .build/check/<machine>.log.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BRANCH="${1:-$(git -C "$ROOT" branch --show-current)}"
LOGS="$ROOT/.build/check"
mkdir -p "$LOGS"
rm -f "$LOGS"/*.status

if [ "$(git -C "$ROOT" branch --show-current)" != "$BRANCH" ]; then
    echo "check.sh: the Mac check runs on the current checkout; switch to $BRANCH first" >&2
    exit 2
fi
git -C "$ROOT" fetch -q origin "$BRANCH" || { echo "check.sh: $BRANCH isn't pushed" >&2; exit 2; }
if [ "$(git -C "$ROOT" rev-parse HEAD)" != "$(git -C "$ROOT" rev-parse "origin/$BRANCH")" ]; then
    echo "check.sh: push $BRANCH first; the other machines test what's on GitHub" >&2
    exit 2
fi
COMMIT="$(git -C "$ROOT" rev-parse --short HEAD)"
# Each branch checks in its own folder: sessions check their branches at the same time, and one run deleting the
# shared folder under another's tests failed them (2026-11-10).
DIR="hdm-check-$(printf '%s' "$BRANCH" | tr -c 'A-Za-z0-9' '-')"

# The worktree stays until the next run removes it, so cargo's exit code is the command's.
(ssh linux "cd ~/Projects/herbs-download-manager && git fetch -q origin $BRANCH && rm -rf /tmp/$DIR \
    && git worktree prune && git worktree add -q --detach /tmp/$DIR FETCH_HEAD && cd /tmp/$DIR \
    && PATH=\$HOME/.cargo/bin:\$PATH CARGO_TARGET_DIR=\$HOME/.cache/hdm-check-target cargo test --workspace 2>&1" \
    >"$LOGS/linux.log" 2>&1; echo $? >"$LOGS/linux.status") &

# Windows' ssh shell is cmd. Rust goes in with rustup, into %USERPROFILE%\.cargo\bin.
(ssh windows "cd /d %USERPROFILE%\\Projects\\herbs-download-manager && git fetch -q origin $BRANCH \
    && (if exist %TEMP%\\$DIR rmdir /s /q %TEMP%\\$DIR) & git worktree prune \
    && git worktree add -q --detach %TEMP%\\$DIR FETCH_HEAD && cd /d %TEMP%\\$DIR \
    && set PATH=%USERPROFILE%\\.cargo\\bin;%PATH%&& set CARGO_TARGET_DIR=%USERPROFILE%\\.cache\\hdm-check-target&& cargo test --workspace 2>&1" \
    >"$LOGS/windows.log" 2>&1; echo $? >"$LOGS/windows.status") &

(cd "$ROOT" && macos/scripts/build.sh && scripts/test.sh) >"$LOGS/mac.log" 2>&1
echo $? >"$LOGS/mac.status"
wait

failed=0
for m in mac linux windows; do
    s="$(cat "$LOGS/$m.status" 2>/dev/null || echo 1)"
    if [ "$s" = 0 ]; then
        echo "$m: passed ($COMMIT)"
    else
        failed=1
        echo "$m: FAILED ($COMMIT), see $LOGS/$m.log"
        grep -E "panicked|FAILED|error(\[|:)" "$LOGS/$m.log" | head -5 | sed 's/^/    /'
    fi
done
exit $failed
