#!/bin/bash
# Starts the app in a virtual display, clicks "close" like a window manager does, and checks it quits.
cd "$(dirname "$0")/.."
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
env -u WAYLAND_DISPLAY GDK_BACKEND=x11 xvfb-run -a -s "-screen 0 1280x820x24" bash -c "
  XDG_DATA_HOME='$T/xdg-data' XDG_CACHE_HOME='$T/xdg-cache' ORGANISE_DIR='$T/Organise' target/release/organise >/dev/null 2>&1 & P=\$!
  sleep 5
  python3 tools/xclose.py Organise
  for i in \$(seq 1 50); do kill -0 \$P 2>/dev/null || { echo 'CLOSE PASSED (window closed)'; exit 0; }; sleep 0.1; done
  echo 'CLOSE FAILED (still open after 5 s)'; kill \$P; exit 1"
