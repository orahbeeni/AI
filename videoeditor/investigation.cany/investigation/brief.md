# Brief

**Question:** Build a simple, cross-platform desktop video editor driven by ffmpeg: drag and drop clips, alphabetical order that can be rearranged, any output format (mp4 default), choosable final picture size, split into N equal parts of the same quality, each part with its own max file size (default 20 MB), and a View Console option showing the ffmpeg commands as they run.

**Purpose:** New app built from the person's request; no target to study.

**Constraints:**
- Rust with eframe/egui
- Files changed only inside app/
- Linux, macOS and Windows; ffmpeg does the editing

**Done when:** It builds, the --screenshot option saves a real picture, and a real export of mixed clips produces correct parts under their size limits.
