#!/bin/sh
# Turns a video into the intro animation embedded in the app
# (crates/app/assets/intro.bin): a bundle of JPEG frames played back by the app.
# Audio is not included.
#
#   scripts/make-intro.sh video.mp4 [fps=24] [width=1152] [quality=7]
#
# Needs ffmpeg and python3. Quality is ffmpeg's JPEG scale: 2 (best) to 31 (smallest).
set -eu

VIDEO="${1:?usage: make-intro.sh video.mp4 [fps] [width] [quality]}"
FPS="${2:-24}"
WIDTH="${3:-1152}"
QUALITY="${4:-7}"
OUT="$(cd "$(dirname "$0")/.." && pwd)/crates/app/assets/intro.bin"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
ffmpeg -v error -y -i "$VIDEO" -an -vf "fps=$FPS,scale=$WIDTH:-2:flags=lanczos" -q:v "$QUALITY" -huffman optimal "$TMP/f_%04d.jpg"

python3 - "$TMP" "$OUT" "$FPS" <<'PY'
import glob, struct, sys

tmp, out, fps = sys.argv[1], sys.argv[2], int(sys.argv[3])
frames = [open(p, "rb").read() for p in sorted(glob.glob(tmp + "/f_*.jpg"))]
assert frames, "ffmpeg produced no frames"

# Image size from the first JPEG's start-of-frame marker.
def jpeg_size(data):
    i = 2
    while i < len(data):
        assert data[i] == 0xFF
        marker, length = data[i + 1], struct.unpack(">H", data[i + 2:i + 4])[0]
        if 0xC0 <= marker <= 0xCF and marker not in (0xC4, 0xC8, 0xCC):
            h, w = struct.unpack(">HH", data[i + 5:i + 9])
            return w, h
        i += 2 + length
    raise SystemExit("no size found")

w, h = jpeg_size(frames[0])
with open(out, "wb") as f:
    # "GGIV", version, fps, width, height, frame count, then each frame's length, then the frames.
    f.write(b"GGIV" + struct.pack("<HHHHI", 1, fps, w, h, len(frames)))
    f.write(b"".join(struct.pack("<I", len(fr)) for fr in frames))
    f.write(b"".join(frames))
print(f"{len(frames)} frames, {w}x{h} at {fps} fps, {sum(map(len, frames)) / 1e6:.2f} MB -> {out}")
PY
