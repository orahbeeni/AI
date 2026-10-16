#!/usr/bin/env bash
# Build a Debian package of term: target/deb/term_<version>_<arch>.deb
#
#   ./deb_build.sh            build the release binary, then the .deb
#   ./deb_build.sh --no-build reuse the existing target/release/term
#
# Needs: cargo, dpkg-deb (package "dpkg-dev" or "dpkg").
set -euo pipefail

cd "$(dirname "$0")"

NAME=term
MAINTAINER="Omar Rahbeeni <omar.rahbeeni@gmail.com>"
VERSION=$(sed -n '/^\[package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' Cargo.toml)
[ -n "$VERSION" ] || { echo "could not read the version from Cargo.toml" >&2; exit 1; }
ARCH=$(dpkg --print-architecture)

PKG="target/deb/${NAME}_${VERSION}_${ARCH}"
DEB="$PKG.deb"

if [ "${1:-}" != "--no-build" ]; then
    cargo build --release
fi
BIN=target/release/$NAME
[ -x "$BIN" ] || { echo "$BIN not found; run without --no-build" >&2; exit 1; }

# Start from an empty staging tree so nothing stale ends up in the package.
rm -rf "$PKG" "$DEB"
install -d "$PKG/DEBIAN" "$PKG/usr/bin" "$PKG/usr/share/applications" \
    "$PKG/usr/share/doc/$NAME" "$PKG/usr/share/icons/hicolor/512x512/apps"

install -m 755 "$BIN" "$PKG/usr/bin/$NAME"
install -m 644 packaging/term.png "$PKG/usr/share/icons/hicolor/512x512/apps/$NAME.png"
install -m 644 LICENSE "$PKG/usr/share/doc/$NAME/copyright"

cat > "$PKG/usr/share/applications/$NAME.desktop" <<EOT
[Desktop Entry]
Type=Application
Name=Term
GenericName=Terminal
Comment=Fast split-pane terminal
Exec=term
Icon=term
Terminal=false
Categories=System;TerminalEmulator;
Keywords=terminal;shell;console;command;
StartupWMClass=term
EOT
chmod 644 "$PKG/usr/share/applications/$NAME.desktop"

SIZE_KB=$(du -sk --exclude=DEBIAN "$PKG" | cut -f1)
cat > "$PKG/DEBIAN/control" <<EOT
Package: $NAME
Version: $VERSION
Architecture: $ARCH
Maintainer: $MAINTAINER
Installed-Size: $SIZE_KB
Depends: libc6, libgcc-s1
Recommends: libgl1, libxkbcommon0, libwayland-client0, libgtk-3-0, xdg-desktop-portal-gtk
Section: utils
Priority: optional
Description: Fast cross-platform split-pane terminal
 term is a terminal written in Rust (egui) with many resizable panes in one
 window, tabs, saved layouts with per-terminal folders and commands, and
 selection that stays put while a log is streaming.
EOT

dpkg-deb --build --root-owner-group "$PKG" "$DEB"

echo
echo "Built $DEB"
echo "Install with: sudo dpkg -i $DEB"
