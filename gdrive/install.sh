#!/usr/bin/env bash
# Installs Google Drive for Linux for the current user (no sudo needed).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
bin="$HOME/.local/bin/gdrive"
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/mime/packages" \
         "$HOME/.config/autostart"

[ -d "$here/.venv" ] || python3 -m venv --system-site-packages "$here/.venv"
"$here/.venv/bin/pip" install -q google-api-python-client google-auth-oauthlib watchdog

cat > "$bin" <<SH
#!/usr/bin/env bash
cd "$here" && exec "$here/.venv/bin/python" -m gdrive "\$@"
SH
chmod +x "$bin"

# double-click .gdoc/.gsheet/... opens them in the browser
sed "s|@BIN@|$bin|" "$here/packaging/gdrive-open.desktop" > "$HOME/.local/share/applications/gdrive-open.desktop"
cp "$here/packaging/gdrive-mime.xml" "$HOME/.local/share/mime/packages/gdrive.xml"
update-mime-database "$HOME/.local/share/mime" 2>/dev/null || true
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
xdg-mime default gdrive-open.desktop application/x-gdrive-link 2>/dev/null || true

# app launcher + start at login
cat > "$HOME/.local/share/applications/gdrive.desktop" <<D
[Desktop Entry]
Type=Application
Name=Google Drive
Comment=Sync your Google Drive
Exec=$bin gui
Icon=folder-remote
Categories=Network;FileTransfer;
StartupWMClass=io.github.gdrive_linux
D
sed "s|gui\$|gui --background|" "$HOME/.local/share/applications/gdrive.desktop" > "$HOME/.config/autostart/gdrive.desktop"

echo "Installed. Next: follow README step 1, then run:  open "Google Drive" from your app menu"
