#!/usr/bin/env bash
# Type-check the whole workspace for Windows and macOS from a Linux machine.
# `cargo check` doesn't link, so the GStreamer/GLib -sys crates only need to
# be told the library names instead of asking pkg-config for the target OS.
# Usage: scripts/cross-check.sh [target...]
set -euo pipefail
cd "$(dirname "$0")/.."
targets=("$@")
[ ${#targets[@]} -eq 0 ] && targets=(x86_64-pc-windows-gnu aarch64-apple-darwin)
for lib in glib-2.0 gobject-2.0 gio-2.0 gmodule-2.0 gstreamer-1.0 gstreamer-base-1.0 gstreamer-app-1.0 \
           gstreamer-video-1.0 gstreamer-audio-1.0; do
    var=$(echo "$lib" | tr 'a-z.-' 'A-Z__')
    export "SYSTEM_DEPS_${var}_NO_PKG_CONFIG=1"
    export "SYSTEM_DEPS_${var}_LIB=${lib%-*}"
done
for t in "${targets[@]}"; do
    echo "== cargo check --workspace --target $t"
    cargo check --workspace --all-targets --target "$t"
done
