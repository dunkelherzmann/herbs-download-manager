#!/bin/sh
# Builds the Linux app as a Flatpak (SPECIFICATION.md §1). Run on the Linux machine from anywhere in the repo.
#   --install   installs it for this user (flatpak run org.herbsdownloadmanager.app), like the Mac's /Applications
# Needs flatpak-builder (org.flatpak.Builder), the GNOME 49 SDK and its Rust extension (flatpak install --user flathub org.gnome.Sdk//49
# org.freedesktop.Sdk.Extension.rust-stable//25.08). Ends with the app's --self-check.
set -eu
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
INSTALL=0
[ "${1:-}" = "--install" ] && INSTALL=1
cd "$ROOT/linux"
# flatpak-builder runs as the Flathub app org.flatpak.Builder (installed for this user on first use).
flatpak info --user org.flatpak.Builder >/dev/null 2>&1 || flatpak install --user -y --noninteractive flathub org.flatpak.Builder
STATE="$ROOT/linux/.flatpak-builder"
builder() { flatpak run org.flatpak.Builder --state-dir="$STATE" --disable-rofiles-fuse "$@"; }
if [ "$INSTALL" = 1 ]; then
    builder --user --install --force-clean build org.herbsdownloadmanager.app.yml
    flatpak run --command=herbs-download-manager org.herbsdownloadmanager.app --self-check
    printf 'Installed org.herbsdownloadmanager.app (%s)\n' "$(sh "$ROOT/scripts/version.sh")"
    sh "$ROOT/scripts/extension-to-downloads.sh"
else
    builder --force-clean build org.herbsdownloadmanager.app.yml
    builder --run build org.herbsdownloadmanager.app.yml herbs-download-manager --self-check
    printf 'Built linux/build (%s)\n' "$(sh "$ROOT/scripts/version.sh")"
fi
