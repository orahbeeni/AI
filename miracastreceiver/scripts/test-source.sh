#!/usr/bin/env bash
# Plain gst-launch-1.0 Miracast-style stream (H.264 + AAC in MPEG-TS over
# RTP/UDP), without RTSP. Use it to check the media path on its own:
#   terminal 1:  cargo run -p wfd-media --release --example rx -- 19000 30
#   terminal 2:  scripts/test-source.sh 127.0.0.1 19000
# For the full RTSP + media + window path use the app's simulator instead:
#   miracast-receiver --simulate     (or: miracast-receiver sim-source + --connect)
set -euo pipefail
HOST=${1:-127.0.0.1}
PORT=${2:-19000}
exec gst-launch-1.0 -e \
  videotestsrc is-live=true pattern=ball ! video/x-raw,width=1280,height=720,framerate=30/1 \
  ! timeoverlay font-desc="Sans 28" ! videoconvert \
  ! x264enc tune=zerolatency speed-preset=ultrafast bitrate=6000 key-int-max=60 \
  ! video/x-h264,profile=constrained-baseline ! h264parse config-interval=-1 ! queue \
  ! mpegtsmux name=mux alignment=7 ! rtpmp2tpay pt=33 ! udpsink host="$HOST" port="$PORT" sync=false async=false \
  audiotestsrc is-live=true wave=ticks volume=0.2 ! audioconvert ! audioresample \
  ! audio/x-raw,rate=48000,channels=2 ! voaacenc bitrate=128000 ! aacparse ! queue ! mux.
