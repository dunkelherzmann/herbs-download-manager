#!/bin/sh
# Builds "Herb's Download Manager.app" (Apple silicon) in dist/: the Rust core as a static library, its Swift bindings,
# then the AppKit app. Needs the Xcode Command Line Tools and rustup's stable toolchain.
# Usage: scripts/build-macos.sh [--install] [--zip]
#   --install  copies the app to /Applications (quitting a running copy first)
#   --zip      also writes dist/Herbs-Download-Manager-<version>-macOS.zip
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(sh "$ROOT/scripts/version.sh")"
NAME="Herb's Download Manager"
SHORT="Herb's DM"   # the menu bar and anywhere macOS needs a short name
APP="$ROOT/dist/$NAME.app"
BUILD="$ROOT/.build"
TARGET=aarch64-apple-darwin
INSTALL=0; ZIP=0
for a in "$@"; do
    case "$a" in
        --install) INSTALL=1 ;;
        --zip) ZIP=1 ;;
        *) echo "usage: $0 [--install] [--zip]" >&2; exit 2 ;;
    esac
done

cd "$ROOT"
mkdir -p "$BUILD/rust" "$BUILD/uniffi"

printf 'Building the Rust core…\n'
MACOSX_DEPLOYMENT_TARGET=15.0 cargo build --quiet --release --target "$TARGET" -p hd-ffi
cp "target/$TARGET/release/libhd_ffi.a" "$BUILD/rust/libhd_ffi.a"

printf 'Generating the Swift bindings…\n'
cargo run --quiet --release -p uniffi-bindgen -- generate --library "$BUILD/rust/libhd_ffi.a" \
    --language swift --out-dir "$BUILD/uniffi" >/dev/null
mkdir -p macos/Sources/hd_ffiFFI macos/Sources/HDCore
cp "$BUILD/uniffi/hd_ffiFFI.h" macos/Sources/hd_ffiFFI/
cp "$BUILD/uniffi/hd_ffiFFI.modulemap" macos/Sources/hd_ffiFFI/module.modulemap
cp "$BUILD/uniffi/hd_ffi.swift" macos/Sources/HDCore/

printf 'Building the app…\n'
STATUS=0
swift build -c release --arch arm64 --scratch-path "$BUILD/swift" >"$BUILD/build-swift.log" 2>&1 || STATUS=$?
grep -E 'error|warning: ' "$BUILD/build-swift.log" | grep -v 'search path .* not found' | head -40 || true
[ "$STATUS" = 0 ] || { echo "Build failed; full log: $BUILD/build-swift.log" >&2; exit 1; }

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
BIN="$(swift build -c release --arch arm64 --scratch-path "$BUILD/swift" --show-bin-path)"
cp "$BIN/HerbsDownloadManager" "$APP/Contents/MacOS/HerbsDownloadManager"
strip -x "$APP/Contents/MacOS/HerbsDownloadManager"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$SHORT</string>
    <key>CFBundleDisplayName</key><string>$NAME</string>
    <key>CFBundleIdentifier</key><string>com.herbertcisneros.HerbsDownloadManager</string>
    <key>CFBundleExecutable</key><string>HerbsDownloadManager</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>15.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
    <!-- Text and HTML files dropped on the Dock icon: their links open Add Links (never the default app). -->
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key><string>Text with links</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>LSItemContentTypes</key><array><string>public.plain-text</string><string>public.html</string></array>
        </dict>
    </array>
    <!-- The service that takes text and links: also what lets them be dropped on the Dock icon. -->
    <key>NSServices</key>
    <array>
        <dict>
            <key>NSMenuItem</key><dict><key>default</key><string>Download Links with $SHORT</string></dict>
            <key>NSMessage</key><string>addLinksFromService</string>
            <key>NSPortName</key><string>$NAME</string>
            <key>NSRequiredContext</key><dict/>
            <key>NSSendTypes</key>
            <array><string>public.utf8-plain-text</string><string>public.url</string><string>public.html</string></array>
        </dict>
    </array>
</dict>
</plist>
PLIST

# Ad-hoc signature: runs on this Mac. Use a Developer ID + notarization to distribute.
codesign --force --sign - --options runtime --timestamp=none "$APP"
codesign --verify --strict "$APP"
printf 'Built %s (%s)\n' "$APP" "$VERSION"

if [ "$ZIP" = 1 ]; then
    (cd "$ROOT/dist" && ditto -c -k --keepParent "$NAME.app" "Herbs-Download-Manager-$VERSION-macOS.zip")
    printf 'Zipped dist/Herbs-Download-Manager-%s-macOS.zip\n' "$VERSION"
fi

if [ "$INSTALL" = 1 ]; then
    osascript -e "tell application id \"com.herbertcisneros.HerbsDownloadManager\" to quit" >/dev/null 2>&1 || true
    rm -rf "/Applications/$NAME.app"
    ditto "$APP" "/Applications/$NAME.app"
    printf 'Installed /Applications/%s.app\n' "$NAME"
fi
