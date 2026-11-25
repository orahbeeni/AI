#!/bin/bash
# Adds Organise to the app launcher (Activities / app grid) for this user, so the dock and taskbar show its icon.
# Starts it through run.sh, which rebuilds first if the sources changed.
#   ./tools/install-desktop.sh            add or update
#   ./tools/install-desktop.sh --remove   take it out again
#
# The window's Wayland app id (and X11 WM_CLASS instance) is the program name, "organise", so the launcher file
# must be called organise.desktop for GNOME to match the window to it.
set -e
DIR=$(cd "$(dirname "$0")/.." && pwd)
APPS=${XDG_DATA_HOME:-$HOME/.local/share}/applications
FILE=$APPS/organise.desktop

if [ "$1" = "--remove" ]; then
  rm -f "$FILE"
  echo "Removed $FILE"
else
  mkdir -p "$APPS"
  cat > "$FILE" <<EOF
[Desktop Entry]
Type=Application
Name=Organise
Comment=Lists, a board, and links to files that follow them when they move
Exec="$DIR/run.sh"
Icon=$DIR/app/src-tauri/icons/icon.png
Terminal=false
Categories=Office;ProjectManagement;
StartupWMClass=Organise
EOF
  echo "Added $FILE"
fi
command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" 2>/dev/null || true
