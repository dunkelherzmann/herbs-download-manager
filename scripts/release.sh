#!/bin/sh
# Makes a release of Herb's Download Manager from the commit origin/main is at (Herbert, 2026-11-10: "Create
# releases for HDM"). Run on the Mac; Linux builds on the Linux machine, driven through Master Control (mc-exec), and
# Windows on GitHub's Windows runner (.github/workflows/release-windows.yml, started here for the same commit), so that
# SignPath Foundation can sign its files: SignPath signs only what a GitHub-hosted workflow built. Once signing is set
# up, the run waits until Herbert approves its two signing requests in SignPath.
#   scripts/release.sh             builds the five files into dist/release-<version>/ and checks each one
#   scripts/release.sh --publish   then tags v<version> on GitHub and creates the release with them, marked Latest
# Files: Herbs-Download-Manager-<version>-macOS.zip (the .app, ad-hoc signed, not notarized),
# …-Linux.flatpak (a bundle that installs the GNOME runtime from Flathub), …-Windows-Setup.exe (the installer, signed
# through SignPath once that's set up), …-Windows.zip (the same app and hdm.exe to run without installing; they
# must stay side by side: hdm is the browser's native host), and Herbs-DM-extension-<extension version>.zip.
# Every build runs in a throwaway worktree of that commit, so no checkout changes and Herbert's running copies are
# never touched. The notes are scripts/release-notes.md with VERSION and EXTENSION filled in.
# HDM_RELEASE_REF=origin/BRANCH builds a pushed branch instead, to try the files before merging (never with --publish).
# Needs gh signed in to GitHub with rights to start workflows.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# The Linux machine, by its Master Control name (`mctl machines`): linux, or another from the environment or an
# untracked scripts/hosts.env (HDM_LINUX_HOST=name) in this checkout or the main one.
for hosts in "$ROOT/scripts/hosts.env" "$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)/../scripts/hosts.env"; do
    if [ -f "$hosts" ]; then . "$hosts"; break; fi
done
LINUX_MACHINE="${HDM_LINUX_HOST:-linux}"
REPO=dunkelherzmann/herbs-download-manager
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
SETUP="Herbs-Download-Manager-$VERSION-Windows-Setup.exe"
EXTZIP="Herbs-DM-extension-$EXT.zip"
echo "Release $VERSION from $(git -C "$ROOT" log -1 --format='%h %s' "$COMMIT")"

# Linux and Windows build at the same time as the Mac.
# Master Control copies files by absolute path, so first the Linux machine's home folder.
LIN_HOME="$(mc-exec "$LINUX_MACHINE" 'echo $HOME')"
[ -n "$LIN_HOME" ] || { echo "release.sh: Master Control can't reach $LINUX_MACHINE" >&2; exit 1; }
mc-exec "$LINUX_MACHINE" "set -e; cd ~/Projects/herbs-download-manager && git fetch -q origin && rm -rf ~/Projects/wt-hdm-release && git worktree prune && git worktree add -q --detach ~/Projects/wt-hdm-release $COMMIT && cd ~/Projects/wt-hdm-release && sh linux/scripts/build.sh --bundle ~/$LIN && cd ~/Projects/herbs-download-manager && git worktree remove --force ~/Projects/wt-hdm-release" >"$LOGS/linux.log" 2>&1 &
LINUX=$!
# The workflow runs from the branch's copy of it and checks out COMMIT; the run is found by the name it's given.
REQUEST="$VERSION-$(date +%Y%m%d-%H%M%S)"
windows_release() {
    gh workflow run release-windows.yml --repo "$REPO" --ref "${REF#origin/}" -f commit="$COMMIT" -f request="$REQUEST" || return 1
    run=""
    for _ in $(seq 60); do
        run="$(gh run list --repo "$REPO" --workflow release-windows.yml --event workflow_dispatch --limit 20 \
            --json databaseId,displayTitle --jq ".[] | select(.displayTitle == \"Windows release $REQUEST\") | .databaseId" | head -1)"
        [ -n "$run" ] && break
        sleep 5
    done
    [ -n "$run" ] || { echo "the workflow run didn't appear"; return 1; }
    echo "https://github.com/$REPO/actions/runs/$run"
    gh run watch "$run" --repo "$REPO" --exit-status --interval 30 >/dev/null || return 1
    gh run download "$run" --repo "$REPO" -n windows-release -D "$OUT"
}
rm -f "$OUT/$WIN" "$OUT/$SETUP"
windows_release >"$LOGS/windows.log" 2>&1 &
WINDOWS=$!
echo "windows: building on GitHub Actions (see $LOGS/windows.log for the run's address)"

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
if wait $LINUX && mc-exec "$LINUX_MACHINE" --get "$LIN_HOME/$LIN" "$OUT/$LIN"; then
    echo "linux: $LIN"
else
    echo "linux: build failed (see $LOGS/linux.log)"; failed=1
fi
if wait $WINDOWS && [ -f "$OUT/$WIN" ] && [ -f "$OUT/$SETUP" ]; then
    echo "windows: $WIN, $SETUP"
else
    echo "windows: build failed (see $LOGS/windows.log)"; failed=1
fi
git -C "$ROOT" archive --format=zip --prefix="Herbs-DM-extension-$EXT/" -o "$OUT/$EXTZIP" "$COMMIT:extension"
echo "extension: $EXTZIP"
sed -e "s/VERSION/$VERSION/g" -e "s/EXTENSION/$EXT/g" "$WT/scripts/release-notes.md" >"$OUT/notes.md"
[ "$failed" = 0 ] || exit 1
(cd "$OUT" && shasum -a 256 "$MAC" "$LIN" "$SETUP" "$WIN" "$EXTZIP" >SHA256SUMS.txt)

if [ "$PUBLISH" = 1 ]; then
    gh release create "v$VERSION" --repo dunkelherzmann/herbs-download-manager --target "$COMMIT" --latest \
        --title "Herb's Download Manager $VERSION" --notes-file "$OUT/notes.md" \
        "$OUT/$MAC" "$OUT/$LIN" "$OUT/$SETUP" "$OUT/$WIN" "$OUT/$EXTZIP" "$OUT/SHA256SUMS.txt"
fi
