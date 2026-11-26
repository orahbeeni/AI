#!/usr/bin/env bash
# Builds the Windows installer on Linux:
#
#   ./tools/build-windows.sh
#
# Result: target/x86_64-pc-windows-gnu/release/bundle/nsis/Organise_<version>_x64-setup.exe
# (also copied to dist/). Works on Windows 10 and 11, 64-bit. It installs for the current user, so it needs
# no admin rights; if the PC lacks Microsoft's WebView2 engine (Windows 11 and up-to-date Windows 10 have it),
# the installer downloads and installs it.
#
# Needs the mingw-w64 cross compiler and NSIS. If they aren't installed and you can't use sudo, on Debian or
# Ubuntu this script downloads the packages and unpacks them into ~/.cache/organise-windows-tools (no root).
set -euo pipefail

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

here=$(cd "$(dirname "$0")/.." && pwd)
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
TARGET=x86_64-pc-windows-gnu

rustup target list --installed | grep -qx $TARGET || { say "Adding the Windows target to Rust"; rustup target add $TARGET; }

# --- cross compiler and NSIS: the system's, or unpacked into a cache folder ---------------------------------
TOOLS=${ORGANISE_WINDOWS_TOOLS:-$HOME/.cache/organise-windows-tools}
if [ -d "$TOOLS/root/usr/bin" ]; then
    export PATH="$TOOLS/root/usr/bin:$PATH"
    export NSISDIR="$TOOLS/root/usr/share/nsis"
fi
if ! command -v x86_64-w64-mingw32-gcc > /dev/null || ! command -v makensis > /dev/null; then
    command -v apt-get > /dev/null || die "install the mingw-w64 cross compiler and NSIS (makensis), then run this again"
    say "Fetching mingw-w64 and NSIS into $TOOLS (no root needed)"
    mkdir -p "$TOOLS/debs" "$TOOLS/root"
    (cd "$TOOLS/debs" && apt-get download \
        gcc-mingw-w64-x86-64-posix gcc-mingw-w64-x86-64-posix-runtime gcc-mingw-w64-base \
        binutils-mingw-w64-x86-64 mingw-w64-x86-64-dev mingw-w64-common nsis nsis-common)
    for d in "$TOOLS"/debs/*.deb; do dpkg -x "$d" "$TOOLS/root"; done
    # the posix-threads compiler is the one Rust's Windows target expects under the plain name
    for t in gcc gcc-ar gcc-nm gcc-ranlib; do
        [ -e "$TOOLS/root/usr/bin/x86_64-w64-mingw32-$t" ] || ln -sf "x86_64-w64-mingw32-$t-posix" "$TOOLS/root/usr/bin/x86_64-w64-mingw32-$t"
    done
    export PATH="$TOOLS/root/usr/bin:$PATH"
    export NSISDIR="$TOOLS/root/usr/share/nsis"
    command -v x86_64-w64-mingw32-gcc > /dev/null && command -v makensis > /dev/null || die "the unpacked tools didn't work; install mingw-w64 and nsis with your package manager"
fi
# Tauri's CLI sets NSISDIR to /usr/share/nsis when it runs makensis; unpacked NSIS lives elsewhere, so a small
# wrapper earlier on the PATH points it back
if [ "$(command -v makensis)" = "$TOOLS/root/usr/bin/makensis" ]; then
    mkdir -p "$TOOLS/bin"
    printf '#!/bin/sh\nNSISDIR="%s" exec "%s" "$@"\n' "$TOOLS/root/usr/share/nsis" "$TOOLS/root/usr/bin/makensis" > "$TOOLS/bin/makensis"
    chmod +x "$TOOLS/bin/makensis"
    export PATH="$TOOLS/bin:$PATH"
fi
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc

# --- build -----------------------------------------------------------------------------------------------------
version=$(python3 -c "import json;print(json.load(open('$here/app/src-tauri/tauri.conf.json'))['version'])")
say "Building Organise $version for Windows (the first build takes several minutes)"
cd "$here/app"
[ -d node_modules ] || npm install
npx tauri build --target $TARGET --bundles nsis

out=$(ls -t "$here"/target/$TARGET/release/bundle/nsis/*-setup.exe | head -1)
mkdir -p "$here/dist"
cp "$out" "$here/dist/"
say "Done: $here/dist/$(basename "$out") ($(du -h "$out" | cut -f1))"
