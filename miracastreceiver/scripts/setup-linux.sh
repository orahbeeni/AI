#!/usr/bin/env bash
# One-time setup for the Miracast receiver on Ubuntu/Debian.
#
#   scripts/setup-linux.sh            install (asks for your sudo password)
#   scripts/setup-linux.sh --uninstall
#
# What it does, and why:
#  1. Installs the GStreamer development headers and plugins (H.264/AAC decode).
#  2. Builds the release binary and installs it to ~/.local/bin.
#  3. Gives that binary three network capabilities (setcap), so it can put an
#     IP address on the Wi-Fi Direct group interface (cap_net_admin) and run
#     the small DHCP server on UDP 67 (cap_net_bind_service, cap_net_raw).
#     Nothing else in the app runs with extra privileges.
#  4. Adds a D-Bus policy that lets YOUR user talk to wpa_supplicant (on
#     Debian/Ubuntu only root and the "netdev" group can). Same access as
#     being in the netdev group; takes effect immediately, no re-login.
#  5. Adds a launcher entry ("Miracast Receiver") to your app menu.
#
# NetworkManager keeps running and your Wi-Fi stays connected: the app only
# uses the separate Wi-Fi Direct (P2P) device that wpa_supplicant provides.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN="$HOME/.local/bin/miracast-receiver"
POLICY=/etc/dbus-1/system.d/miracast-receiver.conf
DESKTOP="$HOME/.local/share/applications/miracast-receiver.desktop"
ICON="$HOME/.local/share/icons/hicolor/scalable/apps/miracast-receiver.svg"
USER_NAME="$(id -un)"

reload_dbus() {
    sudo systemctl reload dbus.service 2>/dev/null \
        || sudo dbus-send --system --type=method_call --dest=org.freedesktop.DBus / org.freedesktop.DBus.ReloadConfig \
        || true
}

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN" "$DESKTOP" "$ICON"
    if [ -f "$POLICY" ]; then
        sudo rm -f "$POLICY"
        reload_dbus
    fi
    echo "Removed the binary, launcher entry and D-Bus policy."
    exit 0
fi

echo "==> 1/5 Installing GStreamer packages (sudo)"
sudo apt-get update -qq
sudo apt-get install -y \
    pkg-config build-essential \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
    gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad \
    gstreamer1.0-plugins-ugly gstreamer1.0-libav gstreamer1.0-pulseaudio

echo "==> 2/5 Building the release binary"
if ! command -v cargo >/dev/null; then
    echo "cargo not found: install Rust from https://rustup.rs first" >&2
    exit 1
fi
cargo build --release -p wfd-app
install -Dm755 target/release/miracast-receiver "$BIN"

echo "==> 3/5 Granting network capabilities to $BIN (sudo)"
sudo setcap cap_net_admin,cap_net_bind_service,cap_net_raw+ep "$BIN"
getcap "$BIN"

echo "==> 4/5 Allowing $USER_NAME to use wpa_supplicant over D-Bus (sudo)"
sudo tee "$POLICY" >/dev/null <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<!-- Installed by miracast-receiver/scripts/setup-linux.sh -->
<busconfig>
  <policy user="$USER_NAME">
    <allow send_destination="fi.w1.wpa_supplicant1"/>
    <allow send_interface="fi.w1.wpa_supplicant1"/>
    <allow receive_sender="fi.w1.wpa_supplicant1" receive_type="signal"/>
  </policy>
</busconfig>
EOF
reload_dbus

echo "==> 5/5 Adding the app menu entry"
install -Dm644 assets/miracast-receiver.svg "$ICON"
mkdir -p "$(dirname "$DESKTOP")"
sed "s|@BIN@|$BIN|" assets/miracast-receiver.desktop > "$DESKTOP"

echo
echo "Done. Checking the result:"
"$BIN" doctor || true
cat <<'EOF'

Start it from the app menu ("Miracast Receiver") or run: miracast-receiver
Note: re-run this script after rebuilding, because a new binary loses its
capabilities (cargo-built copies in target/ never have them).
EOF
