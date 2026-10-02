#!/bin/bash
gnome-extensions disable screenshot-annotate@omar.local 2>/dev/null || true
rm -rf ~/.local/share/gnome-shell/extensions/screenshot-annotate@omar.local \
  ~/.local/bin/screenshot-annotator ~/.local/share/applications/com.omar.ScreenshotAnnotator.desktop \
  ~/.config/screenshot-annotator
echo "Removed."
