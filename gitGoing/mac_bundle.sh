#!/usr/bin/env bash
# Build a macOS app bundle: target/macos/gitgoing.app
#
#   ./mac_bundle.sh              build the release binary, then the bundle
#   ./mac_bundle.sh --no-build   reuse the existing target/release/gitgoing
#   ./mac_bundle.sh --install    also copy the bundle into /Applications
#
# Needs: cargo, and the macOS tools sips, iconutil and codesign.
# The bundle is signed ad hoc, which is enough to run it on the Mac that built it.
set -euo pipefail

cd "$(dirname "$0")"

NAME=gitgoing
VERSION=$(awk -F'"' '/^\[workspace.package\]/{f=1;next} /^\[/{f=0} f && /^version *=/{print $2}' Cargo.toml)
[ -n "$VERSION" ] || { echo "could not read the version from Cargo.toml" >&2; exit 1; }

BUILD=1
INSTALL=0
for a in "$@"; do
    case "$a" in
        --no-build) BUILD=0 ;;
        --install) INSTALL=1 ;;
        *) echo "unknown option: $a" >&2; exit 1 ;;
    esac
done

[ "$BUILD" = 1 ] && cargo build --release -p gitgoing
BIN=target/release/$NAME
[ -x "$BIN" ] || { echo "$BIN not found; run without --no-build" >&2; exit 1; }

APP=target/macos/$NAME.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
install -m 755 "$BIN" "$APP/Contents/MacOS/$NAME"

# Icon: render every size macOS wants from the 512px PNG.
ICONSET=$(mktemp -d)/$NAME.iconset
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    sips -z $s $s crates/app/assets/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2))
    if [ $d -le 512 ]; then
        sips -z $d $d crates/app/assets/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
    fi
done
cp crates/app/assets/icon.png "$ICONSET/icon_256x256@2x.png"
cp crates/app/assets/icon.png "$ICONSET/icon_512x512.png"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/$NAME.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$NAME</string>
    <key>CFBundleDisplayName</key><string>$NAME</string>
    <key>CFBundleIdentifier</key><string>com.omarrahbeeni.$NAME</string>
    <key>CFBundleExecutable</key><string>$NAME</string>
    <key>CFBundleIconFile</key><string>$NAME</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

codesign --force --deep -s - "$APP" >/dev/null

echo "Built $APP"
if [ "$INSTALL" = 1 ]; then
    rm -rf "/Applications/$NAME.app"
    cp -R "$APP" /Applications/
    echo "Installed /Applications/$NAME.app"
else
    echo "Install with: cp -R $APP /Applications/   (or run with --install)"
fi
