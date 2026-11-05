#!/usr/bin/env bash
# Builds the Windows installer from this checkout, on Linux or macOS:
#
#   ./build-windows-installer.sh
#
# The result is app/dist/Sketchframe-Setup-<version>.exe (the version comes from app/Cargo.toml).
# Give that one file to anyone; it needs no admin rights to install.
#
# Needs Rust (rustup), the mingw-w64 cross compiler and NSIS. Anything missing is listed, and the
# script offers to install it with your package manager (apt, dnf, pacman or Homebrew).
set -euo pipefail

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# Questions go to the terminal, so this also works when the script's input is piped.
ask() {
    local reply=""
    if [ -r /dev/tty ]; then
        printf '%s [y/N] ' "$1" > /dev/tty
        read -r reply < /dev/tty || true
    fi
    [[ "$reply" =~ ^[Yy] ]]
}

here=$(cd "$(dirname "$0")" && pwd)
[ -f "$here/app/Cargo.toml" ] || die "run this from the screendesign folder (app/Cargo.toml not found next to it)"
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

os=$(uname -s)
case "$os" in
    Linux | Darwin) ;;
    *) die "this script is for Linux and macOS" ;;
esac

# --- Rust ------------------------------------------------------------------------------------

if ! command -v rustup > /dev/null; then
    say "Rust is not installed."
    ask "Install Rust now with rustup (https://rustup.rs)?" || die "install Rust from https://rustup.rs and run this script again"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    . "$HOME/.cargo/env"
fi
if ! rustup target list --installed | grep -qx x86_64-pc-windows-gnu; then
    say "Adding the Windows target to Rust"
    rustup target add x86_64-pc-windows-gnu
fi

# --- cross compiler and installer maker -------------------------------------------------------

missing=""
command -v x86_64-w64-mingw32-gcc > /dev/null || missing="$missing mingw-w64"
command -v x86_64-w64-mingw32-windres > /dev/null || missing="$missing mingw-w64"
command -v makensis > /dev/null || missing="$missing nsis"

if [ -n "$missing" ]; then
    if [ "$os" = Darwin ]; then
        install="brew install mingw-w64 nsis"
        command -v brew > /dev/null || die "Homebrew is needed to install mingw-w64 and NSIS: see https://brew.sh, then run: $install"
    elif command -v apt-get > /dev/null; then
        install="sudo apt-get install -y mingw-w64 nsis"
    elif command -v dnf > /dev/null; then
        install="sudo dnf install -y mingw64-gcc mingw64-binutils mingw64-winpthreads-static nsis"
    elif command -v pacman > /dev/null; then
        install="sudo pacman -S --needed mingw-w64-gcc nsis"
    else
        die "install the mingw-w64 cross compiler (x86_64-w64-mingw32-gcc) and NSIS (makensis), then run this script again"
    fi
    say "Missing:$(printf '%s\n' $missing | sort -u | tr '\n' ' ')"
    echo "    This installs them: $install"
    ask "Run it now?" || die "install them yourself and run this script again"
    $install
    command -v x86_64-w64-mingw32-gcc > /dev/null || die "x86_64-w64-mingw32-gcc is still not on the PATH after installing"
    command -v makensis > /dev/null || die "makensis is still not on the PATH after installing"
fi

# --- build -----------------------------------------------------------------------------------

version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/app/Cargo.toml" | head -1)
say "Building the Windows installer for Sketchframe $version (the first build takes several minutes)"
"$here/app/windows/build-installer.sh"

installer="$here/app/dist/Sketchframe-Setup-$version.exe"
[ -f "$installer" ] || die "the build finished but $installer is missing"
size=$(du -h "$installer" | cut -f1)
say "Done: $installer ($size)"
echo "    Send that file to the person; they double-click it to install."
echo "    Windows may warn \"Windows protected your PC\" because the installer is not code-signed:"
echo "    they click \"More info\", then \"Run anyway\"."
