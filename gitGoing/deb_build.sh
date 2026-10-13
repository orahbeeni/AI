#!/usr/bin/env bash
# Build a Debian package of gitgoing: target/deb/gitgoing_<version>_<arch>.deb
#
#   ./deb_build.sh            build the release binary, then the .deb
#   ./deb_build.sh --no-build reuse the existing target/release/gitgoing
#
# Needs: cargo, dpkg-deb (package "dpkg-dev" or "dpkg").
set -euo pipefail

cd "$(dirname "$0")"

NAME=gitgoing
MAINTAINER="Omar Rahbeeni <omar.rahbeeni@gmail.com>"
VERSION=$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' Cargo.toml)
[ -n "$VERSION" ] || { echo "could not read the version from Cargo.toml" >&2; exit 1; }
ARCH=$(dpkg --print-architecture)

PKG="target/deb/${NAME}_${VERSION}_${ARCH}"
DEB="$PKG.deb"

if [ "${1:-}" != "--no-build" ]; then
    cargo build --release -p gitgoing
fi
BIN=target/release/$NAME
[ -x "$BIN" ] || { echo "$BIN not found; run without --no-build" >&2; exit 1; }

# Start from an empty staging tree so nothing stale ends up in the package.
rm -rf "$PKG" "$DEB"
install -d "$PKG/DEBIAN" "$PKG/usr/bin" "$PKG/usr/share/applications" \
    "$PKG/usr/share/doc/$NAME" "$PKG/usr/share/icons/hicolor/512x512/apps"

install -m 755 "$BIN" "$PKG/usr/bin/$NAME"
install -m 644 crates/app/assets/icon.png "$PKG/usr/share/icons/hicolor/512x512/apps/$NAME.png"
install -m 644 LICENSE "$PKG/usr/share/doc/$NAME/copyright"

cat > "$PKG/usr/share/applications/$NAME.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=gitgoing
GenericName=Git Client
Comment=Fast, graph-first desktop git client
Exec=gitgoing %F
Icon=gitgoing
Terminal=false
Categories=Development;RevisionControl;
Keywords=git;vcs;commit;branch;
StartupWMClass=gitgoing
EOF
chmod 644 "$PKG/usr/share/applications/$NAME.desktop"

SIZE_KB=$(du -sk --exclude=DEBIAN "$PKG" | cut -f1)
cat > "$PKG/DEBIAN/control" <<EOF
Package: $NAME
Version: $VERSION
Architecture: $ARCH
Maintainer: $MAINTAINER
Installed-Size: $SIZE_KB
Depends: libc6, libgcc-s1, zlib1g, git
Recommends: libgl1, libxkbcommon0, libwayland-client0, libgtk-3-0
Section: devel
Priority: optional
Description: Fast, memory-frugal, graph-first desktop git client
 gitgoing is a cross-platform desktop git client written in Rust (egui).
 It offers a streaming commit graph, line-level staging, side-by-side
 diffs, a conflict editor, interactive rebase and more.
EOF

dpkg-deb --build --root-owner-group "$PKG" "$DEB"

echo
echo "Built $DEB"
echo "Install with: sudo dpkg -i $DEB"
