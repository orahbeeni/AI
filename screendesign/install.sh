#!/usr/bin/env bash
# Builds the latest Sketchframe from GitHub and installs it for the current user.
#
#   curl -fsSL https://raw.githubusercontent.com/orahbeeni/AI/main/screendesign/install.sh | bash
#
#   install.sh                 build the latest version from GitHub and install it
#   install.sh --source DIR    build from a local checkout (DIR holds app/Cargo.toml) instead
#   install.sh --uninstall     remove the installed app (your projects are kept)
#
# Linux:  ~/.local/bin/sketchframe, plus a menu entry and icon under ~/.local/share
# macOS:  Sketchframe.app in /Applications (or ~/Applications when that is not writable)
set -euo pipefail

REPO="https://github.com/orahbeeni/AI.git"
BRANCH="main"
SUBDIR="screendesign"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# Questions go to the terminal, so this also works when piped from curl.
ask() {
    local reply=""
    if [ -r /dev/tty ]; then
        printf '%s [y/N] ' "$1" > /dev/tty
        read -r reply < /dev/tty || true
    fi
    [[ "$reply" =~ ^[Yy] ]]
}

os=$(uname -s)
case "$os" in
    Linux | Darwin) ;;
    *) die "this script is for Linux and macOS; on Windows use the Sketchframe-Setup installer" ;;
esac

source_dir=""
uninstall=false
while [ $# -gt 0 ]; do
    case "$1" in
        --source) source_dir=$(cd "${2:?--source needs a folder}" && pwd); shift 2 ;;
        --uninstall) uninstall=true; shift ;;
        -h | --help) echo "usage: install.sh [--source DIR | --uninstall]  (see the top of this script)"; exit 0 ;;
        *) die "unknown option: $1 (try --help)" ;;
    esac
done

if [ "$os" = Darwin ]; then
    if [ -w /Applications ]; then app_dir=/Applications; else app_dir="$HOME/Applications"; fi
    cache="$HOME/Library/Caches/sketchframe"
else
    data="${XDG_DATA_HOME:-$HOME/.local/share}"
    bin_dir="$HOME/.local/bin"
    cache="${XDG_CACHE_HOME:-$HOME/.cache}/sketchframe"
fi

if $uninstall; then
    if [ "$os" = Darwin ]; then
        rm -rf /Applications/Sketchframe.app "$HOME/Applications/Sketchframe.app"
    else
        rm -f "$bin_dir/sketchframe" "$data/applications/sketchframe.desktop" \
            "$data/icons/hicolor/256x256/apps/sketchframe.png"
        command -v update-desktop-database > /dev/null && update-desktop-database "$data/applications" 2> /dev/null || true
    fi
    rm -rf "$cache"
    say "Sketchframe removed. Your projects and settings are still in the Sketchframe data folder."
    exit 0
fi

# --- tools ---------------------------------------------------------------------------------

command -v git > /dev/null || [ -n "$source_dir" ] || die "git is needed (Linux: install it with your package manager; macOS: xcode-select --install)"

if [ "$os" = Darwin ]; then
    xcode-select -p > /dev/null 2>&1 || die "the Xcode command line tools are needed: run  xcode-select --install  and then this script again"
else
    command -v cc > /dev/null || die "a C compiler is needed to link the app. Ubuntu/Debian: sudo apt install build-essential   Fedora: sudo dnf install gcc   Arch: sudo pacman -S base-devel"
fi

[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
if ! command -v cargo > /dev/null; then
    say "Rust is not installed. It is needed to build Sketchframe (about 1.5 GB in ~/.rustup and ~/.cargo)."
    ask "Install Rust now with rustup (https://rustup.rs)?" || die "install Rust from https://rustup.rs and run this script again"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    . "$HOME/.cargo/env"
fi

# --- source --------------------------------------------------------------------------------

if [ -z "$source_dir" ]; then
    src="$cache/src"
    if [ -d "$src/.git" ]; then
        say "Fetching the latest version"
    else
        say "Downloading the source"
        rm -rf "$src"
        mkdir -p "$cache"
        git clone -q --depth 1 --filter=blob:none --no-checkout --branch "$BRANCH" "$REPO" "$src"
    fi
    # Only the app's source: the repository also holds other projects and build output.
    git -C "$src" sparse-checkout set --no-cone "/$SUBDIR/app/" "!/$SUBDIR/app/target/" "!/$SUBDIR/app/dist/" "!/$SUBDIR/app/shots/"
    git -C "$src" fetch -q --depth 1 origin "$BRANCH"
    git -C "$src" reset --hard -q FETCH_HEAD
    source_dir="$src/$SUBDIR"
fi
app="$source_dir/app"
[ -f "$app/Cargo.toml" ] || die "no app/Cargo.toml in $source_dir"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$app/Cargo.toml" | head -1)

# --- build ---------------------------------------------------------------------------------

say "Building Sketchframe $version (the first build takes a few minutes)"
export CARGO_TARGET_DIR="$cache/target"
cargo build --release --manifest-path "$app/Cargo.toml"
exe="$CARGO_TARGET_DIR/release/sketchframe"

# --- install -------------------------------------------------------------------------------

if [ "$os" = Darwin ]; then
    bundle="$app_dir/Sketchframe.app"
    stage="$cache/Sketchframe.app"
    rm -rf "$stage"
    mkdir -p "$stage/Contents/MacOS" "$stage/Contents/Resources"
    cp "$exe" "$stage/Contents/MacOS/sketchframe"

    # .icns from the 256 px PNG, with the tools every Mac has.
    iconset="$cache/icon.iconset"
    rm -rf "$iconset" && mkdir -p "$iconset"
    for s in 16 32 64 128 256; do
        sips -z $s $s "$app/windows/icon.png" --out "$iconset/icon_${s}x${s}.png" > /dev/null
        if [ $s -gt 16 ]; then cp "$iconset/icon_${s}x${s}.png" "$iconset/icon_$((s / 2))x$((s / 2))@2x.png"; fi
    done
    iconutil -c icns "$iconset" -o "$stage/Contents/Resources/icon.icns"

    cat > "$stage/Contents/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Sketchframe</string>
  <key>CFBundleDisplayName</key><string>Sketchframe</string>
  <key>CFBundleIdentifier</key><string>com.orahbeeni.sketchframe</string>
  <key>CFBundleExecutable</key><string>sketchframe</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>10.15</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
    # Built on this Mac, so an ad-hoc signature is enough for Gatekeeper.
    codesign --force --deep --sign - "$stage" > /dev/null 2>&1 || true

    mkdir -p "$app_dir"
    rm -rf "$bundle"
    mv "$stage" "$bundle"
    say "Installed Sketchframe $version to $bundle"
    echo "    Open it from Launchpad or Spotlight. For AI assistants (MCP) the program is:"
    echo "    $bundle/Contents/MacOS/sketchframe --mcp"
else
    mkdir -p "$bin_dir" "$data/applications" "$data/icons/hicolor/256x256/apps"
    install -m 755 "$exe" "$bin_dir/sketchframe"
    install -m 644 "$app/windows/icon.png" "$data/icons/hicolor/256x256/apps/sketchframe.png"
    cat > "$data/applications/sketchframe.desktop" << EOF
[Desktop Entry]
Type=Application
Name=Sketchframe
Comment=Sketch-style screen designer
Exec=$bin_dir/sketchframe %f
Icon=sketchframe
Terminal=false
Categories=Graphics;Development;
StartupWMClass=sketchframe
EOF
    command -v update-desktop-database > /dev/null && update-desktop-database "$data/applications" 2> /dev/null || true
    command -v gtk-update-icon-cache > /dev/null && gtk-update-icon-cache -q "$data/icons/hicolor" 2> /dev/null || true

    say "Installed Sketchframe $version to $bin_dir/sketchframe"
    echo "    Start it from your applications menu, or run: sketchframe"
    case ":$PATH:" in
        *":$bin_dir:"*) ;;
        *) echo "    Note: $bin_dir is not on your PATH, so add it to run 'sketchframe' from a terminal." ;;
    esac
    if ! fc-list 2> /dev/null | grep -qi 'comic'; then
        echo "    Tip: the sketch lettering looks best with Comic Neue (Ubuntu: sudo apt install fonts-comic-neue)."
    fi
fi
