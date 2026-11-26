#!/bin/sh
# Checks a pushed branch on Herbert's three machines before merging, as well as GitHub Actions: the repo is public,
# and Actions checks every pull request on GitHub's runners again (since 2026-11-10, .github/workflows/build.yml).
# Merge only on this script's exit status 0 and a "passed" line per machine, never judged through `| tail` (#89).
#
#   scripts/check.sh [BRANCH]    (default: the current branch, which must be pushed)
#
# - Mac (here): macos/scripts/build.sh and scripts/test.sh, as the old macOS job did, on the current checkout.
# - Linux and Windows, through Master Control (mc-exec): `cargo test --workspace` in a throwaway worktree of
#   origin/BRANCH (/tmp/hdm-check-BRANCH, %TEMP%\hdm-check-BRANCH), so their own checkouts stay on main; builds are cached in
#   ~/.cache/hdm-check-target.
# - On every machine, after the tests: `cargo clippy --workspace --all-targets`. It fails only on clippy's deny-level
#   lints (real bugs); warnings are left alone, so there is no `-D warnings`.
# - First, without running anything: if the branch changed extension/, it must raise "version" in
#   extension/manifest.json too (docs/agents/extension.md), or the install steps never copy the new extension to his Downloads.
# Prints one line per machine and exits non-zero if any failed. Logs go to .build/check/<machine>.log.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# The Linux and Windows machines, by their Master Control names (`mctl machines`): linux and windows, or others from the
# environment or an untracked scripts/hosts.env (HDM_LINUX_HOST=name, HDM_WINDOWS_HOST=name) in this checkout or the main one.
for hosts in "$ROOT/scripts/hosts.env" "$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)/../scripts/hosts.env"; do
    if [ -f "$hosts" ]; then . "$hosts"; break; fi
done
LINUX_MACHINE="${HDM_LINUX_HOST:-linux}"
WINDOWS_MACHINE="${HDM_WINDOWS_HOST:-windows}"
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

# Every change to extension/ raises its manifest's version: compared with where the branch left main.
git -C "$ROOT" fetch -q origin main || { echo "check.sh: can't fetch origin/main" >&2; exit 2; }
BASE="$(git -C "$ROOT" merge-base HEAD origin/main)"
if ! git -C "$ROOT" diff --quiet "$BASE" HEAD -- extension/; then
    ext_version() { git -C "$ROOT" show "${1}:extension/manifest.json" 2>/dev/null | sed -n 's/^ *"version" *: *"\([^"]*\)".*/\1/p'; }
    OLD_EXT="$(ext_version "$BASE")"
    NEW_EXT="$(ext_version HEAD)"
    if [ "$OLD_EXT" = "$NEW_EXT" ]; then
        echo "extension: FAILED ($COMMIT), extension/ changed but its manifest's version is still ${NEW_EXT:-unknown}: raise it (docs/agents/extension.md)"
        exit 1
    fi
    # A new version only for a real change: with the version left out (the manifest's and SCRIPT_VERSION in
    # background.js), the extension must differ from main's, or the new version would be an empty update (Herbert, 2026-11-26).
    unversioned() {
        git -C "$ROOT" ls-tree -r "$1" extension | grep -v -E '	extension/(manifest\.json|background\.js)$'
        git -C "$ROOT" show "${1}:extension/manifest.json" | grep -v -E '^ *"version" *:'
        git -C "$ROOT" show "${1}:extension/background.js" | grep -v -E '^const SCRIPT_VERSION = '
    }
    if [ "$(unversioned "$BASE" | shasum)" = "$(unversioned HEAD | shasum)" ]; then
        echo "extension: FAILED ($COMMIT), only the version changed ($OLD_EXT to $NEW_EXT): leave it at $OLD_EXT; a new version needs a real change (docs/agents/extension.md)"
        exit 1
    fi
fi
# Each branch checks in its own folder: sessions check their branches at the same time, and one run deleting the
# shared folder under another's tests failed them (2026-11-10).
DIR="hdm-check-$(printf '%s' "$BRANCH" | tr -c 'A-Za-z0-9' '-')"

# The worktree stays after the run, so cargo's exit code is the command's. Branches are checked once or twice and
# then merged, so each run first removes check folders over a day old (any branch's): they were piling up, one a
# branch (2026-11-11). A day leaves alone the checks other sessions are running now.
(mc-exec "$LINUX_MACHINE" "cd ~/Projects/herbs-download-manager && git fetch -q origin $BRANCH \
    && (find /tmp -maxdepth 1 -name 'hdm-check-*' -mmin +1440 -exec rm -rf {} + || true) && rm -rf /tmp/$DIR \
    && git worktree prune && git worktree add -q --detach /tmp/$DIR FETCH_HEAD && cd /tmp/$DIR \
    && PATH=\$HOME/.cargo/bin:\$PATH CARGO_TARGET_DIR=\$HOME/.cache/hdm-check-target cargo test --workspace 2>&1 \
    && PATH=\$HOME/.cargo/bin:\$PATH CARGO_TARGET_DIR=\$HOME/.cache/hdm-check-target cargo clippy --workspace --all-targets 2>&1" \
    >"$LOGS/linux.log" 2>&1; echo $? >"$LOGS/linux.status") &

# Master Control runs Windows commands in cmd. Rust goes in with rustup, into %USERPROFILE%\.cargo\bin.
(mc-exec "$WINDOWS_MACHINE" "cd /d %USERPROFILE%\\Projects\\herbs-download-manager && git fetch -q origin $BRANCH \
    && (forfiles /p %TEMP% /m hdm-check-* /d -1 /c \"cmd /c if @isdir==TRUE rmdir /s /q @path\" >nul 2>&1) \
    & (if exist %TEMP%\\$DIR rmdir /s /q %TEMP%\\$DIR) & git worktree prune \
    && git worktree add -q --detach %TEMP%\\$DIR FETCH_HEAD && cd /d %TEMP%\\$DIR \
    && set PATH=%USERPROFILE%\\.cargo\\bin;%PATH%&& set CARGO_TARGET_DIR=%USERPROFILE%\\.cache\\hdm-check-target&& cargo test --workspace 2>&1 \
    && cargo clippy --workspace --all-targets 2>&1" \
    >"$LOGS/windows.log" 2>&1; echo $? >"$LOGS/windows.status") &

(cd "$ROOT" && macos/scripts/build.sh && scripts/test.sh && cargo clippy --workspace --all-targets 2>&1) >"$LOGS/mac.log" 2>&1
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
