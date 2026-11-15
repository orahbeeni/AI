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
  Set the options the way you like them (format, transcript, subtitles…), press *Save choices as preset*, give it a name
  and optionally tick *with folder* to remember the download folder too. Your presets can be updated with the choices on
  screen, renamed or deleted, and one can be made the starting point for **every new link** (otherwise a new link starts
  from whatever you chose last). They are kept between sessions.
- **Scheduled & live streams** — waits with a countdown, records from the start, keeps going from the tray.
- **Everything else in plain language** — quality picker with a full format table, subtitles, SponsorBlock,
  clip cutting (start/end), chapters, playlists with checkboxes, file-name builder, cookies/proxy/speed limit.
- **Dated file names** — downloads are named `2026-11-14 - Title [id].ext` (the upload date first, so a folder sorts by date); pick another pattern under Options → File name. A transcript starts with a line giving the upload date, channel and link of the video. Live and scheduled recordings keep yt-dlp's plain name.
- **Show command** — every download shows the exact yt-dlp command (copyable), so you learn as you go.
- **Subscriptions** — watch a channel/playlist and auto-download only new videos.
- **Queue** — concurrency limit, stop/retry, persisted across restarts, "when finished" action (notify/quit/shut down).
- **Gentle on YouTube** — when you download several videos (a playlist or a list of links) a *How careful should it be?*
  choice appears: **Quick** (no pauses), **Steady** (one at a time, random 5–20 s pauses) and **Careful** (one at a time,
  20–60 s pauses, for hundreds of videos or after a block). **Auto**, the default, uses Quick below 10 videos and
  Steady from 10 up. If YouTube still pushes back (HTTP 429 / "confirm you're not a bot") the whole queue pauses for
  15 minutes, the affected download is retried afterwards, and the Queue page shows a countdown with *Resume now*.
- **Plain-English errors** with a suggested fix (bot check → use browser cookies, missing ffmpeg, geo-block…).
- **Clipboard offer** & drag-and-drop links, disk-space guard, desktop notifications.
- **Transcripts** — tick *Write a transcript* (Options → Transcript) and, once the download finishes, Whisper turns it into
  text on your own computer. The text file has a `[HH:MM:SS]` mark at the start of every paragraph (every 15 s – 5 min, your
  choice) so you can check any line against the original audio; `.srt`/`.vtt` subtitles are optional. The engine and the
  models (Tiny … Large v3) are downloaded automatically the first time they are needed, or ahead of time in Settings.
  **Who is speaking** can label paragraphs *Speaker 1*, *Speaker 2*… using two small extra models (36 MB, public, no
  account). It is approximate: it works best on clean audio with clearly different voices, and it mixes people up when
  they talk over each other. Whisper itself cannot tell speakers apart, so this is a separate step that adds roughly
  15–20 minutes per audio hour on a 2016 laptop.
  **Files you already have:** on the Download page, *Already have the video or audio?* → *Choose files…* makes a transcript of
  any mp4, mkv, webm, mov, mp3, m4a, opus, ogg, flac, wav… on your computer (several at once), with the same choices. Nothing
  is downloaded; each transcript is saved next to its file and the job appears in the Queue like any other.
- **Continue instead of starting over** — a job that stopped part-way gets a *Continue* button: videos already saved in the folder are skipped (no re-download, no re-tagging), a failed
  playlist says which videos failed and why, and every job keeps its full yt-dlp log (Details → *Open log file*). If Whisper fails on one file (for example a very long recording that runs the computer out of memory) the other files still get their transcripts, and *Continue* tries only the failed ones again. Long recordings are transcribed in 25-minute windows so memory stays around 2 GB however long the file is (speaker labels are skipped above 4 hours, noted in the transcript's first line).
- **Recent lookups** — the link box has a drop-down of your last 10 lookups; picking one looks it up again and brings back
  every setting you had for it (format, options, transcript choices, which playlist videos were ticked).
- **Self-contained** — downloads its own yt-dlp, ffmpeg and deno (checksum-verified yt-dlp) on first run and updates them.

### Transcript speed

Measured on an Intel i7-6700HQ (2016 laptop, 4 cores / 8 threads, no GPU), English speech, the settings the app uses
(greedy decoding, voice-activity filter). Minutes to transcribe **one hour** of audio (only Small was timed on a full hour; the others are scaled from 2–5 minute clips); newer machines are faster.

| Model | Download | Minutes per audio hour |
|---|---|---|
| Tiny | 75 MB | ~6 |
| Base | 145 MB | ~5 |
| Small | 485 MB | ~12 (measured on a full hour: 11.8) |
| Medium | 1.5 GB | ~30 |
| Turbo (large-v3) | 1.6 GB | ~32 |
| Large v3 | 3.1 GB | ~57 |

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
