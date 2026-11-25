#!/bin/bash
# screenshot.sh <data-folder-parent> <view> <out.png>
# view: list | board | links | settings | details:<item id>. Runs in a virtual display on a copy of the data.
set -e
SRC=$1; VIEW=$2; OUT=$(realpath -m "$3")
BIN=$(realpath "$(dirname "$0")/../target/release/organise")
W=$(mktemp -d); cp -a "$SRC/." "$W/"
env -u WAYLAND_DISPLAY GDK_BACKEND=x11 xvfb-run -a -s "-screen 0 1280x820x24" bash -c "
  XDG_DATA_HOME='$W/xdg-data' XDG_CACHE_HOME='$W/xdg-cache' ORGANISE_DIR='$W/Organise' ORGANISE_VIEW='$VIEW' '$BIN' >/dev/null 2>&1 &
  P=\$!; sleep 6
  ffmpeg -loglevel error -y -f x11grab -video_size 1280x820 -i \$DISPLAY -frames:v 1 '$OUT'
  kill \$P"
rm -rf "$W"
