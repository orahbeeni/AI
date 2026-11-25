#!/bin/bash
# Runs the app's built-in self-test in a virtual display with a throwaway data folder.
# It drives the real app (nesting, undo, a file drop, saving, real file moves on disk, board rules).
set -e
cd "$(dirname "$0")/.."
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
env -u WAYLAND_DISPLAY GDK_BACKEND=x11 xvfb-run -a -s "-screen 0 1440x900x24" \
  env XDG_DATA_HOME="$T/xdg-data" XDG_CACHE_HOME="$T/xdg-cache" ORGANISE_DIR="$T/Organise" ORGANISE_SELFTEST=1 ${SELFTEST_LOG:+ORGANISE_DEBUG=1} target/release/organise 2>"${SELFTEST_LOG:-/dev/null}" | grep -E "^(ok|FAIL|SELFTEST)"
