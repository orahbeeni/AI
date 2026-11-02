#!/usr/bin/env bash
# Build video and install it for the current user, on macOS or Linux.
#
#   ./install.sh              build the release binary and install it
#   ./install.sh --no-build   reuse the existing target/release/video
#   ./install.sh --uninstall  remove what the installer put in place
#
# macOS: builds Video.app and puts it in /Applications (or ~/Applications if that is not writable).
# Linux: installs ~/.local/bin/video, a menu entry and an icon (no root needed).
#
# Needs: cargo (https://rustup.rs). macOS also uses sips, iconutil and codesign, which come with the system.
#        Linux also needs the ALSA development files (Debian/Ubuntu: libasound2-dev).
# ffmpeg does not have to be installed first: the app installs it the first time it runs without it.
set -euo pipefail

cd "$(dirname "$0")"

NAME=video
APPNAME=Video
OS=${VIDEO_INSTALL_OS:-$(uname -s)}   # VIDEO_INSTALL_OS only exists so the layout can be tried on another system

BUILD=1
UNINSTALL=0
for a in "$@"; do
    case "$a" in
        --no-build) BUILD=0 ;;
        --uninstall) UNINSTALL=1 ;;
        -h|--help) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $a" >&2; exit 1 ;;
    esac
done

# --- where things go -----------------------------------------------------------------------------
if [ "$OS" = Darwin ]; then
    if [ -w /Applications ]; then APPS=/Applications; else APPS="$HOME/Applications"; fi
    APPS=${VIDEO_APP_DIR:-$APPS}
    APP="$APPS/$APPNAME.app"
else
    BIN_DIR="${VIDEO_BIN_DIR:-$HOME/.local/bin}"
    DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
    ICON="$DATA_DIR/icons/hicolor/512x512/apps/$NAME.png"
    DESKTOP="$DATA_DIR/applications/$NAME.desktop"
fi

if [ "$UNINSTALL" = 1 ]; then
    if [ "$OS" = Darwin ]; then
        rm -rf "$APP"
        echo "Removed $APP"
    else
        rm -f "$BIN_DIR/$NAME" "$ICON" "$DESKTOP"
        update-desktop-database "$DATA_DIR/applications" 2>/dev/null || true
        echo "Removed $BIN_DIR/$NAME and its menu entry"
    fi
    echo "A downloaded copy of ffmpeg, if the app made one, is left in the app data folder (video-app/ffmpeg)."
    exit 0
fi

case "$OS" in
    Darwin|Linux) ;;
    *) echo "this installer supports macOS and Linux; on Windows run: cargo build --release" >&2; exit 1 ;;
esac

# --- build ---------------------------------------------------------------------------------------
[ -d "$HOME/.cargo/bin" ] && PATH="$HOME/.cargo/bin:$PATH"
if [ "$BUILD" = 1 ]; then
    command -v cargo >/dev/null || { echo "cargo not found; install Rust from https://rustup.rs and run this again" >&2; exit 1; }
    # Playing sound needs the ALSA development files to build on Linux.
    if [ "$OS" = Linux ] && command -v pkg-config >/dev/null && ! pkg-config --exists alsa; then
        echo "The ALSA development files are missing (needed to build the sound playback)." >&2
        echo "  Debian/Ubuntu: sudo apt install libasound2-dev pkg-config" >&2
        echo "  Fedora:        sudo dnf install alsa-lib-devel" >&2
        exit 1
    fi
    cargo build --release
fi
BIN=target/release/$NAME
[ -x "$BIN" ] || { echo "$BIN not found; run without --no-build" >&2; exit 1; }
VERSION=$(sed -n '/^\[package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p;}' Cargo.toml)
[ -n "$VERSION" ] || { echo "could not read the version from Cargo.toml" >&2; exit 1; }

# --- macOS: Video.app ----------------------------------------------------------------------------
if [ "$OS" = Darwin ]; then
    BUNDLE=target/macos/$APPNAME.app
    rm -rf "$BUNDLE"
    mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"
    install -m 755 "$BIN" "$BUNDLE/Contents/MacOS/$NAME"

    # Icon: every size macOS wants, rendered from the 1024px PNG.
    ICONSET=$(mktemp -d)/$NAME.iconset
    mkdir -p "$ICONSET"
    for s in 16 32 128 256 512; do
        sips -z $s $s assets/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
        sips -z $((s * 2)) $((s * 2)) assets/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
    done
    iconutil -c icns "$ICONSET" -o "$BUNDLE/Contents/Resources/$NAME.icns"

    cat > "$BUNDLE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$APPNAME</string>
    <key>CFBundleDisplayName</key><string>$APPNAME</string>
    <key>CFBundleIdentifier</key><string>dev.video.converter</string>
    <key>CFBundleExecutable</key><string>$NAME</string>
    <key>CFBundleIconFile</key><string>$NAME</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.video</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>CFBundleDocumentTypes</key>
    <array><dict>
        <key>CFBundleTypeName</key><string>Video</string>
        <key>CFBundleTypeRole</key><string>Viewer</string>
        <key>LSItemContentTypes</key><array><string>public.movie</string></array>
    </dict></array>
</dict>
</plist>
PLIST

    # Ad hoc signature: enough to run on the Mac that built it.
    codesign --force --deep -s - "$BUNDLE" >/dev/null

    mkdir -p "$APPS"
    rm -rf "$APP"
    cp -R "$BUNDLE" "$APP"
    echo "Installed $APP"
    echo "Open it from Launchpad or Spotlight (\"Video\")."
    exit 0
fi

# --- Linux: ~/.local ------------------------------------------------------------------------------
mkdir -p "$BIN_DIR" "$(dirname "$ICON")" "$(dirname "$DESKTOP")"
install -m 755 "$BIN" "$BIN_DIR/$NAME"
install -m 644 assets/icon.png "$ICON"
cat > "$DESKTOP" <<EOT
[Desktop Entry]
Type=Application
Name=Video
GenericName=Video converter
Comment=Convert, cut and crop videos, and keep them under a file size
Exec=$BIN_DIR/$NAME %f
Icon=$NAME
Terminal=false
Categories=AudioVideo;Video;AudioVideoEditing;
MimeType=video/quicktime;video/mp4;video/x-matroska;video/webm;video/x-msvideo;
Keywords=video;convert;mp4;ffmpeg;trim;crop;
StartupWMClass=video
EOT
chmod 644 "$DESKTOP"
update-desktop-database "$DATA_DIR/applications" 2>/dev/null || true
gtk-update-icon-cache -q -t "$DATA_DIR/icons/hicolor" 2>/dev/null || true

echo "Installed $BIN_DIR/$NAME and a menu entry (\"Video\")."
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "Note: $BIN_DIR is not on your PATH; add it to run \`video\` from a terminal." ;;
esac
# The app fetches a static ffmpeg when none is installed; that needs tar with xz support.
command -v xz >/dev/null || echo "Note: 'xz' was not found. Install xz-utils (Debian/Ubuntu) so the app can unpack ffmpeg, or install ffmpeg yourself: sudo apt install ffmpeg"
exit 0
