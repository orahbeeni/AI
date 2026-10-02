#!/bin/bash
# Installs the annotator app and the GNOME Shell extension for the current user.
set -e
here="$(cd "$(dirname "$0")" && pwd)"
uuid=screenshot-annotate@omar.local

install -Dm755 "$here/annotator/screenshot-annotator" ~/.local/bin/screenshot-annotator
install -Dm644 "$here/annotator/com.omar.ScreenshotAnnotator.desktop" ~/.local/share/applications/com.omar.ScreenshotAnnotator.desktop
update-desktop-database ~/.local/share/applications 2>/dev/null || true

dest=~/.local/share/gnome-shell/extensions/$uuid
mkdir -p "$dest"
cp "$here"/extension/{metadata.json,extension.js,stylesheet.css} "$dest/"
gnome-extensions enable $uuid 2>/dev/null || true

echo "Installed. On Wayland, log out and back in once so GNOME Shell loads the extension."
echo "Then press Print Screen and switch on the 'Annotate' button."
