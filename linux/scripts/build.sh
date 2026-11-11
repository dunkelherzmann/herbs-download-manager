#!/bin/sh
# Builds the Linux app as a Flatpak (SPECIFICATION.md §1). Run on the Linux machine from anywhere in the repo.
#   --install   installs it for this user (flatpak run org.herbsdownloadmanager.app), like the Mac's /Applications
#   --bundle F  writes a single-file bundle F instead, for a release (scripts/release.sh)
# Needs flatpak-builder (org.flatpak.Builder), the GNOME 49 SDK and its Rust extension (flatpak install --user flathub org.gnome.Sdk//49
# org.freedesktop.Sdk.Extension.rust-stable//25.08). Ends with the app's --self-check.
set -eu
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
INSTALL=0
BUNDLE=""
case "${1:-}" in
    --install) INSTALL=1 ;;
    --bundle) BUNDLE="${2:?--bundle needs a file to write}" ;;
esac
# The app's version, for About (core/build.rs reads it; the Flatpak build has no git history).
sh "$ROOT/scripts/version.sh" > "$ROOT/version.txt"
cd "$ROOT/linux"
# flatpak-builder runs as the Flathub app org.flatpak.Builder (installed for this user on first use).
flatpak info --user org.flatpak.Builder >/dev/null 2>&1 || flatpak install --user -y --noninteractive flathub org.flatpak.Builder
STATE="$ROOT/linux/.flatpak-builder"
builder() { flatpak run org.flatpak.Builder --state-dir="$STATE" --disable-rofiles-fuse "$@"; }
# The built app's --self-check. `--run` takes no --disable-rofiles-fuse, and its own exit status is 1 even for
# `true`, so the check is judged by what the app prints when it passes.
self_check() {
    out="$(flatpak run org.flatpak.Builder --state-dir="$STATE" --run build org.herbsdownloadmanager.app.yml \
        sh -c 'herbs-download-manager --self-check && echo SELF-CHECK-PASSED' 2>&1 || true)"
    printf '%s\n' "$out" | grep -v SELF-CHECK-PASSED
    printf '%s\n' "$out" | grep -q SELF-CHECK-PASSED || { echo "build.sh: the self-check failed" >&2; exit 1; }
}
if [ "$INSTALL" = 1 ]; then
    builder --user --install --force-clean build org.herbsdownloadmanager.app.yml
    flatpak run --command=herbs-download-manager org.herbsdownloadmanager.app --self-check
    printf 'Installed org.herbsdownloadmanager.app (%s)\n' "$(sh "$ROOT/scripts/version.sh")"
    sh "$ROOT/scripts/extension-to-downloads.sh"
elif [ -n "$BUNDLE" ]; then
    # A single-file bundle for a release (scripts/release.sh): it installs the GNOME runtime from Flathub.
    rm -rf release-repo
    builder --force-clean --repo=release-repo build org.herbsdownloadmanager.app.yml
    self_check
    flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo release-repo "$BUNDLE" org.herbsdownloadmanager.app
    rm -rf release-repo
    printf 'Bundled %s (%s)\n' "$BUNDLE" "$(sh "$ROOT/scripts/version.sh")"
else
    builder --force-clean build org.herbsdownloadmanager.app.yml
    self_check
    printf 'Built linux/build (%s)\n' "$(sh "$ROOT/scripts/version.sh")"
fi
