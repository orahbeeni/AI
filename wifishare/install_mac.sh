#!/usr/bin/env bash
# Build wifishare and install it as /Applications/wifishare.app, with `wifishare` on the PATH.
#
#   ./install_mac.sh             build the release binary, then install
#   ./install_mac.sh --no-build  reuse the existing target/release/wifishare
#
# A running copy is quit and started again. The app is signed ad hoc, which is enough to run it
# on the Mac that built it, but every rebuild gets a new signature: if the cursor will not cross
# to the other computer afterwards, remove wifishare under System Settings > Privacy & Security >
# Accessibility and add it again.
set -euo pipefail

cd "$(dirname "$0")"

NAME=wifishare
APP=/Applications/$NAME.app
LINK=/usr/local/bin/$NAME
VERSION=$(sed -n '/^\[package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p;}' Cargo.toml)
[ -n "$VERSION" ] || { echo "could not read the version from Cargo.toml" >&2; exit 1; }

BUILD=1
for a in "$@"; do
    case "$a" in
        --no-build) BUILD=0 ;;
        *) echo "unknown option: $a" >&2; exit 1 ;;
    esac
done

[ "$BUILD" = 1 ] && cargo build --release
BIN=target/release/$NAME
[ -x "$BIN" ] || { echo "$BIN not found; run without --no-build" >&2; exit 1; }

# Quit the running copy so the new one can start in its place.
WAS_RUNNING=0
if pgrep -f "$APP/Contents/MacOS/$NAME" >/dev/null; then
    WAS_RUNNING=1
    pkill -TERM -f "$APP/Contents/MacOS/$NAME" || true
    for _ in $(seq 40); do
        pgrep -f "$APP/Contents/MacOS/$NAME" >/dev/null || break
        sleep 0.25
    done
    pkill -KILL -f "$APP/Contents/MacOS/$NAME" 2>/dev/null || true
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
install -m 755 "$BIN" "$APP/Contents/MacOS/$NAME"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$NAME</string>
    <key>CFBundleDisplayName</key><string>$NAME</string>
    <key>CFBundleIdentifier</key><string>com.wifishare.app</string>
    <key>CFBundleExecutable</key><string>$NAME</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSLocalNetworkUsageDescription</key><string>wifishare finds and connects to paired computers on your local network.</string>
    <key>NSBonjourServices</key><array><string>_wifishare._tcp</string></array>
</dict>
</plist>
PLIST

codesign --force --deep -s - --identifier com.wifishare.app "$APP" >/dev/null
echo "Installed $APP ($VERSION)"

if ln -sf "$APP/Contents/MacOS/$NAME" "$LINK" 2>/dev/null; then
    echo "Linked $LINK"
else
    echo "Could not link $LINK; for the command line run: sudo ln -sf $APP/Contents/MacOS/$NAME $LINK"
fi

if [ "$WAS_RUNNING" = 1 ]; then
    open "$APP"
    echo "Restarted wifishare"
fi
