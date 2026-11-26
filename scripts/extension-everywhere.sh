#!/bin/sh
# Puts the merged browser extension (origin/main) in the Downloads folder of all three machines as "Herb's DM
# extension <version>", the way each install does (Herbert, 2026-11-10: each new version goes there). Run it after
# merging any change to extension/: merges don't install, so without it new versions never got there (bugs.md).
# A version already there is left alone. Prints what each machine has now.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# The Linux and Windows machines, by their Master Control names (`mctl machines`): linux and windows, or others from the
# environment or an untracked scripts/hosts.env (HDM_LINUX_HOST=name, HDM_WINDOWS_HOST=name) in this checkout or the main one.
for hosts in "$ROOT/scripts/hosts.env" "$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)/../scripts/hosts.env"; do
    if [ -f "$hosts" ]; then . "$hosts"; break; fi
done
LINUX_MACHINE="${HDM_LINUX_HOST:-linux}"
WINDOWS_MACHINE="${HDM_WINDOWS_HOST:-windows}"
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
# The Edge Add-ons package beside it (what Partner Center takes: no `key`, no Firefox `background.scripts`, files at
# the zip's top level). A zip made by hand from the folder fails the store's check (2026-11-25).
STORE="$HOME/Downloads/$NAME - Edge store.zip"
if [ -f "$STORE" ]; then
    echo "mac: $VERSION Edge store zip already there"
else
    WORK="$(mktemp -d)"
    tar -xf "$TAR" -C "$WORK" && python3 -I - "$WORK/extension/manifest.json" <<'PY' && (cd "$WORK/extension" && zip -q -r -X "$STORE" . -x '.*') && echo "mac: $VERSION Edge store zip copied" || { echo "mac: Edge store zip FAILED"; failed=1; }
import json, sys
path = sys.argv[1]
manifest = json.load(open(path, encoding="utf-8"))
manifest.pop("key", None)
manifest["background"].pop("scripts", None)
json.dump(manifest, open(path, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
PY
    rm -rf "$WORK"
fi

# Linux: its Downloads folder by xdg-user-dir.
mc-exec "$LINUX_MACHINE" --put "$TAR" /tmp/hdm-extension.tar &&
mc-exec "$LINUX_MACHINE" "D=\"\$(xdg-user-dir DOWNLOAD 2>/dev/null)\"; [ -n \"\$D\" ] && [ \"\$D\" != \"\$HOME\" ] || D=\"\$HOME/Downloads\";
    T=\"\$D/$NAME\"; if [ -d \"\$T\" ]; then echo 'linux: $VERSION already there'; else W=\$(mktemp -d) && tar -xf /tmp/hdm-extension.tar -C \$W \
    && mkdir -p \"\$D\" && mv \$W/extension \"\$T\" && rm -rf \$W && echo 'linux: $VERSION copied'; fi; rm -f /tmp/hdm-extension.tar" \
    || { echo "linux: FAILED"; failed=1; }

# Windows: the shell's Downloads folder. The name has an apostrophe, so the steps go in a .ps1 (docs/agents/linux-windows.md).
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
# Master Control copies to absolute paths, so the home folder comes first.
WIN_HOME="$(mc-exec "$WINDOWS_MACHINE" "echo %USERPROFILE%" | tr -d '\r\n')" && [ -n "$WIN_HOME" ] &&
mc-exec "$WINDOWS_MACHINE" --put "$TAR" "$WIN_HOME\\hdm-extension.tar" && mc-exec "$WINDOWS_MACHINE" --put "$PS1" "$WIN_HOME\\hdm-extension.ps1" &&
mc-exec "$WINDOWS_MACHINE" "powershell -NoProfile -ExecutionPolicy Bypass -File \"%USERPROFILE%\\hdm-extension.ps1\"" | tr -d '\r' || { echo "windows: FAILED"; failed=1; }
rm -f "$TAR" "$PS1"
exit $failed
