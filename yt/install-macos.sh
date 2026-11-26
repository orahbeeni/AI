#!/usr/bin/env bash
# Build YT Grab and install it on this Mac.
#
#   ./install-macos.sh              build "YT Grab.app" and put it in /Applications
#   ./install-macos.sh --no-build   reuse the app from the last build
#   ./install-macos.sh --dmg        also leave a "YT Grab.dmg" next to this script, to copy to another Mac
#   ./install-macos.sh --uninstall  remove the app (your settings and downloads are kept)
#
# Installs to /Applications, or ~/Applications if that is not writable.
# Needs: Node 20+ and Rust (https://rustup.rs). Xcode command line tools provide the rest (xcode-select --install).
# yt-dlp, ffmpeg and deno do not have to be installed: the app downloads its own on first run.
set -euo pipefail

cd "$(dirname "$0")"
ROOT=$PWD

APPNAME="YT Grab"
BUILD=1
DMG=0
UNINSTALL=0
for a in "$@"; do
    case "$a" in
        --no-build) BUILD=0 ;;
        --dmg) DMG=1 ;;
        --uninstall) UNINSTALL=1 ;;
        -h|--help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $a" >&2; exit 1 ;;
    esac
done

[ "$(uname -s)" = Darwin ] || { echo "this installer is for macOS; on Linux/Windows use the release downloads or: cd app && npm run tauri build" >&2; exit 1; }

if [ -w /Applications ]; then APPS=/Applications; else APPS="$HOME/Applications"; fi
APPS=${YTGRAB_APP_DIR:-$APPS}
APP="$APPS/$APPNAME.app"

if [ "$UNINSTALL" = 1 ]; then
    rm -rf "$APP"
    echo "Removed $APP"
    echo "Settings, the queue and downloaded tools are left in ~/Library/Application Support/app.ytgrab.desktop."
    exit 0
fi

# --- build ---------------------------------------------------------------------------------------
[ -d "$HOME/.cargo/bin" ] && PATH="$HOME/.cargo/bin:$PATH"
BUNDLE_DIR=app/src-tauri/target/release/bundle
if [ "$BUILD" = 1 ]; then
    command -v cargo >/dev/null || { echo "cargo not found; install Rust from https://rustup.rs and run this again" >&2; exit 1; }
    command -v npm >/dev/null || { echo "npm not found; install Node 20+ (https://nodejs.org) and run this again" >&2; exit 1; }
    cd app
    npm ci
    if [ "$DMG" = 1 ]; then BUNDLES=app,dmg; else BUNDLES=app; fi
    npx tauri build --bundles "$BUNDLES"
    cd "$ROOT"
fi

BUNDLE="$BUNDLE_DIR/macos/$APPNAME.app"
[ -d "$BUNDLE" ] || { echo "$BUNDLE not found; run without --no-build" >&2; exit 1; }

# --- install -------------------------------------------------------------------------------------
# Quit a running copy first so the replacement isn't half-swapped under it.
osascript -e "tell application \"$APPNAME\" to quit" >/dev/null 2>&1 || true

mkdir -p "$APPS"
rm -rf "$APP"
cp -R "$BUNDLE" "$APP"
# Tauri only leaves the binary's own linker signature; sign the whole bundle ad hoc so it verifies and runs here.
codesign --force --deep -s - "$APP" >/dev/null
# Built on this Mac, so there is nothing to quarantine; clear the flag in case the bundle was copied in.
xattr -dr com.apple.quarantine "$APP" 2>/dev/null || true

if [ "$DMG" = 1 ]; then
    DMG_FILE=$(ls "$BUNDLE_DIR"/dmg/*.dmg 2>/dev/null | head -n 1 || true)
    if [ -n "$DMG_FILE" ]; then
        cp "$DMG_FILE" "$ROOT/$APPNAME.dmg"
        echo "Disk image: $ROOT/$APPNAME.dmg (unsigned: on another Mac, right-click the app → Open the first time)"
    else
        echo "No .dmg was produced; see the build output above." >&2
    fi
fi

echo "Installed $APP"
echo "Open it from Launchpad or Spotlight (\"YT Grab\")."
