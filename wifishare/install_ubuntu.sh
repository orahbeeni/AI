#!/usr/bin/env bash
# Build wifishare and install it for this user as ~/.local/bin/wifishare, listed in the
# applications menu.
#
#   ./install_ubuntu.sh             build the release binary, then install
#   ./install_ubuntu.sh --no-build  reuse the existing target/release/wifishare
#
# A running copy (from the window, a terminal or the systemd user unit in packaging/) is
# restarted on the new binary. The applications menu entry, and the start-at-login one if
# Settings > Startup turned it on, point at the installed binary. If /dev/uinput is not writable
# yet, the udev rule the client needs is installed (asks for sudo); log out and back in after that.
set -euo pipefail

cd "$(dirname "$0")"

NAME=wifishare
DEST=$HOME/.local/bin/$NAME
DATA=${XDG_DATA_HOME:-$HOME/.local/share}
CONFIG=${XDG_CONFIG_HOME:-$HOME/.config}
LAUNCHER=$DATA/applications/$NAME.desktop
AUTOSTART=$CONFIG/autostart/$NAME.desktop
UDEV_RULE=/etc/udev/rules.d/60-wifishare-uinput.rules

BUILD=1
for a in "$@"; do
    case "$a" in
        --no-build) BUILD=0 ;;
        *) echo "unknown option: $a" >&2; exit 1 ;;
    esac
done

[ "$BUILD" = 1 ] && cargo build --release
BIN=target/release/$NAME
[ -x "$BIN" ] || { echo "$BIN not found; run without --no-build" >&2; exit 1; }

# Remember how running copies were started, then stop them.
SERVICE=0
if systemctl --user is-active --quiet wifishare-client 2>/dev/null; then
    SERVICE=1
fi
RESTART=()
for pid in $(pgrep -x "$NAME" || true); do
    [ "$SERVICE" = 1 ] && [ "$pid" = "$(systemctl --user show -p MainPID --value wifishare-client)" ] && continue
    args=$(tr '\0' '\n' <"/proc/$pid/cmdline" | tail -n +2 | paste -sd ' ' -)
    RESTART+=("$args")
    kill -TERM "$pid" 2>/dev/null || true
done
for _ in $(seq 40); do
    [ ${#RESTART[@]} = 0 ] && break
    pgrep -x "$NAME" >/dev/null || break
    [ "$SERVICE" = 1 ] && [ "$(pgrep -x "$NAME" | wc -l)" -le 1 ] && break
    sleep 0.25
done

# install unlinks the old file first, so this works even while the old binary is still running.
install -D -m 755 "$BIN" "$DEST"
echo "Installed $DEST ($("$DEST" --version 2>/dev/null || echo "$NAME"))"

# Menu entry and icon, as the window's "Add to the applications menu" makes them; a start-at-login
# entry from Settings > Startup is pointed at the installed binary too.
"$DEST" install-launcher
echo "Added $LAUNCHER"
[ -f "$AUTOSTART" ] && echo "Pointed $AUTOSTART at $DEST"
command -v update-desktop-database >/dev/null && update-desktop-database "$(dirname "$LAUNCHER")" 2>/dev/null || true

# The client injects input through uinput.
if [ ! -w /dev/uinput ] && [ ! -f "$UDEV_RULE" ]; then
    echo "Installing $UDEV_RULE so wifishare can use /dev/uinput (needs sudo)"
    echo 'KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"' | sudo tee "$UDEV_RULE" >/dev/null
    sudo udevadm control --reload && sudo udevadm trigger --sysname-match=uinput
    echo "Log out and back in so the change applies to your session."
fi

if [ "$SERVICE" = 1 ]; then
    systemctl --user restart wifishare-client
    echo "Restarted the wifishare-client service"
fi
for args in "${RESTART[@]}"; do
    # shellcheck disable=SC2086 # the original arguments, split as they were given
    setsid -f "$DEST" $args >/dev/null 2>&1 </dev/null
    echo "Restarted: wifishare $args"
done

case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *) echo "Note: ~/.local/bin is not on your PATH yet; log out and back in, or run $DEST directly." ;;
esac
