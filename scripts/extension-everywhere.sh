#!/bin/sh
# Puts the merged browser extension (origin/main) in the Downloads folder of all three machines as "Herb's DM
# extension <version>", the way each install does (Herbert, 2026-11-10: each new version goes there). Run it after
# merging any change to extension/: merges don't install, so without it new versions never got there (bugs.md).
# A version already there is left alone. Prints what each machine has now.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
git -C "$ROOT" fetch -q origin main || { echo "extension-everywhere: can't fetch origin/main" >&2; exit 1; }
VERSION="$(git -C "$ROOT" show origin/main:extension/manifest.json | sed -n 's/^ *"version": *"\([^"]*\)".*/\1/p' | head -1)"
[ -n "$VERSION" ] || { echo "extension-everywhere: no version in extension/manifest.json" >&2; exit 1; }
NAME="Herb's DM extension $VERSION"
TAR="$(mktemp -t hdm-extension).tar"
git -C "$ROOT" archive --format=tar origin/main extension >"$TAR"
failed=0

# Mac (here).
DEST="$HOME/Downloads/$NAME"
if [ -d "$DEST" ]; then
    echo "mac: $VERSION already there"
else
    WORK="$(mktemp -d)"
    tar -xf "$TAR" -C "$WORK" && mkdir -p "$HOME/Downloads" && mv "$WORK/extension" "$DEST" && echo "mac: $VERSION copied" || { echo "mac: FAILED"; failed=1; }
    rm -rf "$WORK"
fi

# Linux: its Downloads folder by xdg-user-dir.
scp -q "$TAR" linux:/tmp/hdm-extension.tar &&
ssh linux "D=\"\$(xdg-user-dir DOWNLOAD 2>/dev/null)\"; [ -n \"\$D\" ] && [ \"\$D\" != \"\$HOME\" ] || D=\"\$HOME/Downloads\";
    T=\"\$D/$NAME\"; if [ -d \"\$T\" ]; then echo 'linux: $VERSION already there'; else W=\$(mktemp -d) && tar -xf /tmp/hdm-extension.tar -C \$W \
    && mkdir -p \"\$D\" && mv \$W/extension \"\$T\" && rm -rf \$W && echo 'linux: $VERSION copied'; fi; rm -f /tmp/hdm-extension.tar" \
    || { echo "linux: FAILED"; failed=1; }

# Windows: the shell's Downloads folder. The name has an apostrophe, so the steps go in a .ps1 (CLAUDE.md).
PS1="$(mktemp -t hdm-extension).ps1"
cat >"$PS1" <<PS
\$downloads = (New-Object -ComObject Shell.Application).NameSpace('shell:Downloads').Self.Path
\$dest = Join-Path \$downloads "$NAME"
if (Test-Path \$dest) { "windows: $VERSION already there" } else {
    \$work = Join-Path \$env:TEMP ("hdm-extension-" + [guid]::NewGuid())
    New-Item -ItemType Directory \$work | Out-Null
    tar -xf (Join-Path \$env:USERPROFILE 'hdm-extension.tar') -C \$work
    Move-Item (Join-Path \$work 'extension') \$dest
    Remove-Item -Recurse -Force \$work
    "windows: $VERSION copied"
}
Remove-Item (Join-Path \$env:USERPROFILE 'hdm-extension.tar'), (Join-Path \$env:USERPROFILE 'hdm-extension.ps1') -ErrorAction SilentlyContinue
PS
scp -q "$TAR" windows:hdm-extension.tar && scp -q "$PS1" windows:hdm-extension.ps1 &&
ssh windows "powershell -NoProfile -ExecutionPolicy Bypass -File hdm-extension.ps1" | tr -d '\r' || { echo "windows: FAILED"; failed=1; }
rm -f "$TAR" "$PS1"
exit $failed
