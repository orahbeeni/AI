#!/usr/bin/env bash
# Cross-builds sketchframe.exe for Windows and wraps it in dist/Sketchframe-Setup-<version>.exe.
# Needs: rustup target x86_64-pc-windows-gnu, x86_64-w64-mingw32-gcc/windres, makensis.
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cargo build --release --target x86_64-pc-windows-gnu
mkdir -p dist
out="$PWD/dist/Sketchframe-Setup-$version.exe"
makensis -V2 -DVERSION="$version" -DOUTFILE="$out" \
  -DEXE_PATH="$PWD/target/x86_64-pc-windows-gnu/release/sketchframe.exe" \
  windows/installer.nsi
echo "built $out"
