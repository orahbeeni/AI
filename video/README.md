# video

A small video tool for **macOS, Windows and Linux**: convert a video to MP4, cut out the part you want, crop it,
and keep the file under a size (for example 20 MB). It is a front end for ffmpeg, written in Rust with egui/eframe.

## Running

```sh
cargo run --release                  # then open a video, or drop one on the window
cargo run --release -- clip.mov      # open a file straight away
cargo build --release                # target/release/video
```

To install it (macOS and Linux):

```sh
./install.sh              # builds, then installs Video.app (macOS) or ~/.local/bin/video + a menu entry (Linux)
./install.sh --no-build   # reuse target/release/video
./install.sh --uninstall
```

On Linux the file dialogs go through xdg-desktop-portal (a portal backend such as `xdg-desktop-portal-gtk` or `-gnome`
comes with every desktop). Building needs the ALSA development files for sound playback
(`sudo apt install libasound2-dev pkg-config` on Debian/Ubuntu).

## What it does

1. **Convert** to MP4, MOV, MKV (all H.264 + AAC), WebM (VP9 + Opus), an animated GIF, or sound only as MP3. Pick it in
   the *Convert to* menu; any video ffmpeg can read can be the source.
2. **Play** the video in the window to find the points: Play/Pause, back/forward 5 s, previous/next picture, a position
   slider, volume and mute. *Set start* / *Set end* take the current position.
   Keys: `Space` play/pause, `←` `→` one picture (with `Shift` 5 s), `I` start, `O` end.
3. **Cut**: the two handles on the timeline (or typed times like `1:30.5`) are a selection.
   - Used as they are, the selection is the part that is kept (plain trimming).
   - *Keep only selection* throws away everything outside it, *Remove selection* cuts the selection out and joins what is
     either side, *Reset* starts over. Repeat as often as you like; nothing is converted until you press Convert.
   - After a cut, playing skips the removed parts, so you hear the result. The timeline shows the kept parts (blue)
     and the selection (white line below the bar).
4. **Join**: *+ Add video before…* / *+ Add video after…* put another video at the start or end (dropping a video on the
   window adds it to the end). Click a video in the *Videos* row to edit it; each has its own cuts and crop. ◀ ▶ reorder,
   *Remove* takes one out. The result is as big as the first video's picture (after its crop); the others are fitted into
   it with black bars, and a video without sound gets silence so the rest stays in step.
5. **Crop** (per video): tick *Crop this video*, then drag on the picture to draw the area, drag inside it to move it,
   drag its edges to resize. The 16:9, 9:16, 1:1, 4:3 and 4:5 buttons make the biggest centred box of that shape.
6. **Size**: *Keep the file under* takes any size you type: `20`, `12.5`, `500kb`, `1.5gb` (a plain number is megabytes;
   1 MB is 1,000,000 bytes, so a 20 MB file is also under 20 MiB). The buttons are shortcuts. Untick it for plain high
   quality.

The output is saved next to the first video (`clip.mp4`, or `clip_edited.mp4` if that is also the original's name).
Nothing is overwritten without asking, and a half-finished conversion never leaves a file behind.

### How the size limit is kept

- The bit rate is worked out from the size, the length of everything that is kept and the sound, then the video is
  encoded in two passes so the file lands close to the target (MP3 uses the biggest constant bit rate that fits).
- The picture size is chosen by testing, not guessing: short samples spread over what is kept are encoded quickly at a
  fixed quality, and the biggest size whose estimated file fits the budget wins (1440, 1080, 900, 720, 540, 480, 360,
  270 or 240 p; never scaled up). A screen recording or other still-ish video keeps its full size even for a small
  limit; a busy one is shrunk only as far as it must be. Downscaling uses the sharp Lanczos filter.
- With a size limit the frame rate is capped at 30 fps (60 fps costs bits better spent on sharpness), and the slower
  `slow` x264 preset is used. The window shows what came out (size and frame rate) when it is done.
- The file is measured afterwards. If it is over, the bit rate is lowered by the overshoot and the encode is repeated (up
  to 4 tries). The limit is a promise: it either holds or you get an error, never a bigger file.
- It never spends more bits than the original had, so a small clip may come out well below the limit.
- If what is kept is too long for the size (about 40 kbit/s of video is the floor), it says so; keep less or allow more.

Playback is a preview at reduced size (up to 960 px, 30 fps) decoded by ffmpeg and played through the sound card, so
the picture and sound can be a few hundredths of a second out of step. The conversion itself is not affected.

## ffmpeg

On start the app looks for `ffmpeg` and `ffprobe` on `PATH` and in the usual install folders (Homebrew, `/usr/local/bin`,
WinGet, Chocolatey, Scoop). If they are missing it installs them and shows the progress:

| OS | How |
| --- | --- |
| macOS | `brew install ffmpeg` if Homebrew is there, otherwise a static build from ffmpeg.martin-riedl.de |
| Windows | static build from gyan.dev (BtbN's GitHub build as fallback) |
| Linux | static build from johnvansickle.com (needs `tar` with xz support, which every distro has) |

Downloaded copies go to the app's data folder (`~/Library/Application Support/video-app/ffmpeg`,
`%LOCALAPPDATA%\video-app\ffmpeg` or `~/.local/share/video-app/ffmpeg`) and need no admin rights.

## Layout

| File | What it holds |
| --- | --- |
| `src/ffmpeg.rs` | find / install ffmpeg, `ffprobe` info, preview frames |
| `src/encode.rs` | formats, joining and cutting (the ffmpeg filter graph), the size budget, the retry loop, progress and cancel |
| `src/player.rs` | playback in the window: decoded pictures on time, sound through `cpal` |
| `src/timeline.rs` | the trim bar and time formatting/parsing |
| `src/app.rs` | the window, crop box and conversion state |

## Testing

```sh
cargo test                                                    # logic, plus a real trim + crop + size-limit run if ffmpeg is installed
VIDEO_TEST_FILE=clip.mov [VIDEO_TEST_OUT=out.mp4] cargo test real_file -- --ignored --nocapture   # a real file to under 20 MB
VIDEO_SCREENSHOT=ui.png VIDEO_DEMO=1 cargo run -- clip.mov    # saves the window to a PNG and quits (VIDEO_DEMO=play | edit for other states)
VIDEO_MANAGED_FFMPEG_ONLY=1 cargo run                         # ignore the system ffmpeg: forces the download path
```
