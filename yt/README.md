# YT Grab

A friendly desktop app for [yt-dlp](https://github.com/yt-dlp/yt-dlp). Paste a link, pick what you want,
and let it run — including **scheduled live streams**, which it waits for and records from the start.

Runs as a normal desktop app (Windows, macOS, Linux). No server, no browser tab. The interface is a web UI
(Svelte) shown in a native window by [Tauri](https://tauri.app); a small Rust backend runs yt-dlp.

Your original command is the built-in **"Scheduled / Live → MP4"** preset:

```
yt-dlp --wait-for-video 30-60 --live-from-start -f "bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]" URL
```

## Features

- **Presets** — Best quality, Best MP4, 1080/720/480p, MP3/M4A/FLAC audio, Scheduled/Live → MP4, plus your own.
- **Scheduled & live streams** — waits with a countdown, records from the start, keeps going from the tray.
- **Everything else in plain language** — quality picker with a full format table, subtitles, SponsorBlock,
  clip cutting (start/end), chapters, playlists with checkboxes, file-name builder, cookies/proxy/speed limit.
- **Show command** — every download shows the exact yt-dlp command (copyable), so you learn as you go.
- **Subscriptions** — watch a channel/playlist and auto-download only new videos.
- **Queue** — concurrency limit, stop/retry, persisted across restarts, "when finished" action (notify/quit/shut down).
- **Plain-English errors** with a suggested fix (bot check → use browser cookies, missing ffmpeg, geo-block…).
- **Clipboard offer** & drag-and-drop links, disk-space guard, desktop notifications.
- **Self-contained** — downloads its own yt-dlp, ffmpeg and deno (checksum-verified yt-dlp) on first run and updates them.

## Portable mode (USB stick / another computer)

Create an empty file called `portable` next to the app (the release zips already include it). All settings,
the queue, subscriptions and the downloaded tools are then kept in a `ytgrab-data` folder beside the app, and
downloads go to `downloads/` beside it. Copy the folder anywhere and it keeps working — saved paths are stored
relative to the app folder.

## Develop

Requirements: Node 20+, Rust (stable). Linux also needs the Tauri system libraries
(`libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev`).

```bash
cd app
npm install
npm run tauri dev        # run the real app
npm run dev              # UI only in a browser, with a simulated backend (src/lib/mock.ts)
npm run check            # type-check
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri build      # installers for the current OS
```

Network tests (real downloads) are opt-in:

```bash
cargo test --manifest-path src-tauri/Cargo.toml e2e_install -- --ignored --nocapture
YTGRAB_TEST_BIN=/dir/with/yt-dlp,ffmpeg cargo test --manifest-path src-tauri/Cargo.toml e2e_real -- --ignored --nocapture
```

## How it's organised

| Where | What |
|---|---|
| `app/src/lib/pages`, `components` | The UI. |
| `app/src/lib/defaults.ts` | Built-in presets, filename tokens, SponsorBlock categories. |
| `app/src-tauri/src/args.rs` | UI options → yt-dlp arguments (unit-tested). |
| `app/src-tauri/src/parser.rs` | Reads yt-dlp's progress/wait/live output. |
| `app/src-tauri/src/jobs.rs` | Queue, process control, persistence, subscriptions. |
| `app/src-tauri/src/binaries.rs` | Finds/downloads/updates yt-dlp, ffmpeg, deno. |
| `app/src-tauri/src/platform.rs` | **All** OS-specific code (Windows/macOS/Linux) lives here. |
| `app/src-tauri/src/paths.rs` | Portable-mode data locations. |
| `.github/workflows/build.yml` | Tests on all 3 OSes; builds installers + portable zips. |

## Known limits

- Stopping a live recording is graceful on Linux/macOS (Ctrl+C-style). On Windows the process is ended
  directly, so a partly recorded stream may need remuxing with ffmpeg.
- macOS builds are unsigned: on first launch use right-click → Open.
- Waiting/scheduled jobs and subscriptions only run while the app is running (window closed to tray counts).
