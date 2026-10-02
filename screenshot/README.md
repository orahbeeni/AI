# Screenshot Annotate

Extends Ubuntu's built-in screenshot tool (the Print Screen overlay in GNOME Shell) with:

- **Annotate** toggle: after a capture, opens an editor (pen, line, arrow, rectangle, ellipse, text, eraser, undo/redo, save, copy).
- **OCR** button in the editor: extracts the text from the image into a copyable text box, keeping line breaks, indentation and wide gaps.
- **Audio for screen recordings**: speaker (system audio) and microphone toggles, usable separately or together.

It does not replace or modify any system files. Everything is installed under your home folder, and the stock tool is unchanged when the toggles are off.

Tested on Ubuntu 26.04 with GNOME Shell 50 on Wayland.

## Requirements

| Needed for | Package | Installed by default? |
|---|---|---|
| Editor | `python3-gi`, `python3-gi-cairo`, `gir1.2-gtk-4.0`, `gir1.2-adw-1` | yes |
| Audio in recordings | `pipewire` (`pw-record`), `ffmpeg` | yes |
| OCR | `tesseract-ocr` | **no** |

```
sudo apt install tesseract-ocr
```

Extra OCR languages are optional: `sudo apt install tesseract-ocr-<lang>` (for example `deu`, `fra`, `ara`).

## Install

```
cd /path/to/screenshot
./install.sh
```

This copies:

- the editor to `~/.local/bin/screenshot-annotator` and its launcher to `~/.local/share/applications/`
- the extension to `~/.local/share/gnome-shell/extensions/screenshot-annotate@omar.local/`

Then:

1. **Log out and back in.** On Wayland, GNOME Shell only discovers new extensions at login.
2. Enable the extension if it isn't already on. The installer tries this, but it cannot succeed before the first login:
   ```
   gnome-extensions enable screenshot-annotate@omar.local
   ```
3. Check it:
   ```
   gnome-extensions info screenshot-annotate@omar.local | grep -E "Enabled|State"
   ```
   You want `Enabled: Yes` and `State: ACTIVE`.

## Use

Press **Print Screen**. The buttons sit at the bottom right of the overlay:

- Screenshot mode: the **pencil** toggle. When on, each capture opens in the editor.
- Record mode (video-camera toggle): the **speaker** and **microphone** toggles. When on, audio is recorded alongside the video and added to the file when you stop (you get a notification).
- Your toggle choices are remembered in `~/.config/screenshot-annotator/state.json`.

In the editor: **Save** writes `<name>-annotated.png` next to the original and copies the result to the clipboard. The original is never overwritten. **OCR** (or Ctrl+Shift+O) opens the extracted text.

Run the editor by itself without installing anything:

```
./annotator/screenshot-annotator path/to/screenshot.png
```

## Updating after you edit the code

Re-run `./install.sh`, then:

- Changes to `stylesheet.css` only: `gnome-extensions disable screenshot-annotate@omar.local && gnome-extensions enable screenshot-annotate@omar.local` is enough.
- Changes to `extension.js`: log out and back in (GNOME Shell caches extension code for the whole session).
- Changes to the editor: take effect the next time it is launched. Close any open editor window first, because a running instance receives new files instead of starting fresh.

## Uninstall

```
./uninstall.sh
```

This disables the extension and removes the files listed above plus `~/.config/screenshot-annotator`. Your screenshots, recordings and `-annotated.png` files are not touched. The overlay returns to stock after the next login (or immediately if you only ran `gnome-extensions disable`).

## Troubleshooting

- **No Annotate button:** check `gnome-extensions info ...` as above. If `Enabled: No`, enable it. Then look at `journalctl -b | grep -i screenshot-annotate`.
- **After a GNOME upgrade the extension stops loading:** it is pinned to Shell 50 in `extension/metadata.json` (`shell-version`). Add the new version number there and re-run `./install.sh`. The extension relies on private shell names (`_showPointerButtonContainer`, `_castButton`, the `screenshot-taken` signal), so a major GNOME change may need code updates.
- **Recording has no audio:** make sure `pw-record` and `ffmpeg` exist (`which pw-record ffmpeg`). Audio follows PipeWire's default output and input devices (Settings → Sound). If adding audio fails, the silent video is kept and you get a notification.
- **OCR says the engine is missing:** `sudo apt install tesseract-ocr`.
- **Toggle buttons overlap the capture circle:** button size is set in `extension/stylesheet.css` (`.screenshot-annotate-button` width/height/margin).

## Layout

```
annotator/screenshot-annotator              editor (GTK4 + libadwaita, Python)
annotator/com.omar.ScreenshotAnnotator.desktop
extension/extension.js                      GNOME Shell extension
extension/stylesheet.css
extension/metadata.json
install.sh / uninstall.sh
```
