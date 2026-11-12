#!/bin/sh
# Makes a release of Herb's Download Manager from the commit origin/main is at (Herbert, 2026-11-10: "Create
# releases for HDM"). Run on the Mac; Linux and Windows build over ssh on the Linux and Windows machines.
#   scripts/release.sh             builds the four files into dist/release-<version>/ and checks each one
#   scripts/release.sh --publish   then tags v<version> on GitHub and creates the release with them, marked Latest
# Files: Herbs-Download-Manager-<version>-macOS.zip (the .app, ad-hoc signed, not notarized),
# …-Linux.flatpak (a bundle that installs the GNOME runtime from Flathub), …-Windows.zip (the app and hdm.exe, which
# must stay side by side: hdm is the browser's native host), and Herbs-DM-extension-<extension version>.zip.
# Every build runs in a throwaway worktree of that commit, so no checkout changes and Herbert's running copies are
# never touched. The notes are scripts/release-notes.md with VERSION and EXTENSION filled in.
# HDM_RELEASE_REF=origin/BRANCH builds a pushed branch instead, to try the files before merging (never with --publish).
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# The Linux and Windows machines, by the names ssh knows them under: from the environment, or from an untracked
# scripts/hosts.env (HDM_LINUX_HOST=name, HDM_WINDOWS_HOST=name) in this checkout or in the main one.
for hosts in "$ROOT/scripts/hosts.env" "$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)/../scripts/hosts.env"; do
    if [ -f "$hosts" ]; then . "$hosts"; break; fi
done
LINUX_HOST="${HDM_LINUX_HOST:-linux}"
WINDOWS_HOST="${HDM_WINDOWS_HOST:-windows}"
PUBLISH=0
[ "${1:-}" = "--publish" ] && PUBLISH=1
REF="${HDM_RELEASE_REF:-origin/main}"
if [ "$PUBLISH" = 1 ] && [ "$REF" != "origin/main" ]; then echo "release.sh: only main is published" >&2; exit 2; fi
git -C "$ROOT" fetch -q origin
COMMIT="$(git -C "$ROOT" rev-parse "$REF")"
WT="$(mktemp -d -t hdm-release)/wt"
git -C "$ROOT" worktree add -q --detach "$WT" "$COMMIT"
cleanup() { git -C "$ROOT" worktree remove --force "$WT" 2>/dev/null || true; }
trap cleanup EXIT
VERSION="$(sh "$WT/scripts/version.sh")"
EXT="$(sed -n 's/^ *"version": *"\([^"]*\)".*/\1/p' "$WT/extension/manifest.json" | head -1)"
OUT="$ROOT/dist/release-$VERSION"
LOGS="$OUT/logs"
mkdir -p "$LOGS"
MAC="Herbs-Download-Manager-$VERSION-macOS.zip"
LIN="Herbs-Download-Manager-$VERSION-Linux.flatpak"
WIN="Herbs-Download-Manager-$VERSION-Windows.zip"
EXTZIP="Herbs-DM-extension-$EXT.zip"
echo "Release $VERSION from $(git -C "$ROOT" log -1 --format='%h %s' "$COMMIT")"

# Linux and Windows build at the same time as the Mac.
ssh "$LINUX_HOST" "set -e; cd ~/Projects/herbs-download-manager && git fetch -q origin && rm -rf ~/Projects/wt-hdm-release && git worktree prune && git worktree add -q --detach ~/Projects/wt-hdm-release $COMMIT && cd ~/Projects/wt-hdm-release && sh linux/scripts/build.sh --bundle ~/$LIN && cd ~/Projects/herbs-download-manager && git worktree remove --force ~/Projects/wt-hdm-release" >"$LOGS/linux.log" 2>&1 &
LINUX=$!
scp -q "$WT/windows/scripts/release.ps1" "$WINDOWS_HOST:hdm-release.ps1"
ssh "$WINDOWS_HOST" "powershell -NoProfile -ExecutionPolicy Bypass -File hdm-release.ps1 -Commit $COMMIT -Zip $WIN" >"$LOGS/windows.log" 2>&1 &
WINDOWS=$!

failed=0
if "$WT/macos/scripts/build.sh" --zip >"$LOGS/mac.log" 2>&1; then
    cp "$WT/dist/$MAC" "$OUT/"
    # The zip opens to the app, and the app inside starts and passes its self-check.
    CHECK="$(mktemp -d -t hdm-release-check)"
    ditto -x -k "$OUT/$MAC" "$CHECK"
    codesign --verify --strict "$CHECK/Herb's Download Manager.app" \
        && "$CHECK/Herb's Download Manager.app/Contents/MacOS/HerbsDownloadManager" --self-check >>"$LOGS/mac.log" 2>&1 \
        && echo "mac: $MAC" || { echo "mac: the zipped app failed its check (see $LOGS/mac.log)"; failed=1; }
    rm -rf "$CHECK"
else
    echo "mac: build failed (see $LOGS/mac.log)"; failed=1
fi
if wait $LINUX && scp -q "$LINUX_HOST:$LIN" "$OUT/"; then
    echo "linux: $LIN"
else
    echo "linux: build failed (see $LOGS/linux.log)"; failed=1
fi
if wait $WINDOWS && scp -q "$WINDOWS_HOST:$WIN" "$OUT/"; then
    echo "windows: $WIN"
else
    echo "windows: build failed (see $LOGS/windows.log)"; failed=1
fi
git -C "$ROOT" archive --format=zip --prefix="Herbs-DM-extension-$EXT/" -o "$OUT/$EXTZIP" "$COMMIT:extension"
echo "extension: $EXTZIP"
sed -e "s/VERSION/$VERSION/g" -e "s/EXTENSION/$EXT/g" "$WT/scripts/release-notes.md" >"$OUT/notes.md"
[ "$failed" = 0 ] || exit 1
(cd "$OUT" && shasum -a 256 "$MAC" "$LIN" "$WIN" "$EXTZIP" >SHA256SUMS.txt)

if [ "$PUBLISH" = 1 ]; then
    gh release create "v$VERSION" --repo dunkelherzmann/herbs-download-manager --target "$COMMIT" --latest \
        --title "Herb's Download Manager $VERSION" --notes-file "$OUT/notes.md" \
        "$OUT/$MAC" "$OUT/$LIN" "$OUT/$WIN" "$OUT/$EXTZIP" "$OUT/SHA256SUMS.txt"
fi
