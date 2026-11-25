# Organise (v0.1, Linux)

Lists with nested checkboxes and bullets, a board, and links to files and folders that follow them when they
move. The plan and designs are in [PLAN.md](PLAN.md) and [design/](design/).

## Run it

```bash
./run.sh                                  # builds if needed (first build ~5 min), then starts
ORGANISE_DIR=/tmp/organise-try ./run.sh   # try it on a separate data folder
```

Your data is plain text in `~/Organise`: one Markdown file per list in `lists/`, plus `workspace.yaml`,
`board.yaml`, `links.yaml` and `.machines/<computer>.yaml` (where each linked file is on this computer).

## What to try

- **Lists:** Enter adds an item, Tab / Shift+Tab nests, Ctrl+Enter ticks, Ctrl+. switches checkbox/bullet,
  Alt+↑/↓ moves, Backspace at the start of an item joins it to the item above (its links, notes and
  sub-items come too; the first item of a list stays put). Ctrl+Z / Ctrl+Shift+Z undo and redo.
  Paste several lines to get several items; paste a URL to add a web link.
- **Drag items** by the ⋮ handle (it shows on hover): onto another item to nest, between items to reorder,
  or onto a list in the sidebar to move it there.
- **Reorder lists in the sidebar:** drag a list up or down (a handle shows on hover; dropping it under another
  category heading changes its category), or right-click → Move up / Move down, or select it and press Alt+↑/↓.
  Dragging near the top or bottom edge of anything that scrolls scrolls it.
- **Put a list inside another list:** drag it from the sidebar onto the *middle* of another list (the top or bottom
  edge reorders instead), or onto an item of the open list; or right-click → Move into another list. Its items
  come along, and if it's on the board its card stays in the same column and place. To undo, right-click the
  sub-list → More… → Turn into a list (or drag any item to the sidebar between two lists).
- **Drop files or folders** from the file manager onto an item (links them), below the list (one new item per
  file), or onto a list in the sidebar. Click a link to open it. Right-click it for Show in folder, Copy path,
  and (under More…) Link to another file.
- **Move or rename a linked file** in the file manager. The link follows, and a note says so.
  The *Links* page (reached from the amber pill in the sidebar when something is missing) shows what moved,
  and offers likely matches when a file can't be found.
- **Board** (Ctrl+2): drag cards between columns, add a list with *Add list*, column options on hover (⋯).
  A column that holds cards can't be deleted. Clicking a card opens it in a pop-up over the board: edit it
  there, switch its column or category from the chips at the top, open an item's details beside it, and press
  **Open full page** to make it the main view (Esc or a click outside closes it). Settings › Board › *Clicking a
  card* switches to opening the list straight away; right-click a card for the other way.
- **Move the pop-up** by dragging its header (double-click the header to centre it again; where you leave it is
  remembered). **Drag it past the edge of the window** and let go, or press its pop-out button, and the card opens
  in a window of its own that you can put anywhere (another screen too). That window edits the same data: changes
  show in the main window as soon as you switch to it. Its buttons put it back into the main window as a pop-up,
  or open it as the main window's full page. Closing the main window closes card windows too. On Wayland the
  desktop decides where a new window appears; on X11, Windows and macOS it opens where you dropped it.
  Undo works per window; after the other window changes something, older steps can't be undone.
- **Categories:** each heading has a + for a new list in it; the sidebar's bottom button adds a category.
  Drag a heading to reorder categories (its lists move with it), or use its ⋯ menu → Move up / Move down.
  Click a heading (or its ▸/▾) to fold it away; folded categories show how many lists they hold, are
  remembered, and unfold by themselves when you open or add one of their lists. ⋯ → Fold → Collapse / Expand all.
- **Archive a category:** right-click its heading in the sidebar (or use its ⋯, or Settings › Categories) →
  Archive. It and its lists disappear from the sidebar and their cards from the board; nothing is deleted and
  board places are kept. They're listed under *Archived* at the bottom of the sidebar (you can still open them
  there) and come back with *Unarchive*.
- **Details** (the › at the end of a row, or Ctrl+I): links, notes, and *More details* (type, board column,
  delete). Drag the panel's left edge to make it wider or narrower (double-click the edge to reset); the
  width is remembered. Actions whose result isn't obvious on screen (board, category, delete, remove link…)
  show a short message with Undo. **Settings:** backup/restore, categories, columns; *Advanced* has watched folders.

## How file tracking works on Linux

Linux allows a limited number of folder watches (65,536 here; this home folder has ~409,000 folders), so
Organise watches the folders that hold linked files plus any folders you add under Settings › Advanced.
- A rename or move between watched folders is seen instantly.
- A move anywhere else is noticed when the file leaves its folder. Organise then searches for it, starting
  where it was and working outward, matching the file's identity (inode). A copy to another disk is matched by
  name, size and content.
- Moves made while Organise was closed are found the same way at the next start.
- A worst-case search of this home folder (file deleted) takes about 6–12 s, or ~70 s on a cold disk cache.
  It runs in the background.

## Tests

```bash
cargo test -p organise-core          # model, Markdown, storage, backup, live file moves, search
./tools/selftest.sh                  # drives the real app in a virtual display (needs xvfb-run)
./tools/closetest.sh                 # clicks close like a window manager does and checks the app quits
./tools/screenshot.sh <dir> board out.png
```

## Not in v0.1 yet

Outlook emails (Windows build), Windows/macOS file tracking, sync between devices, phone app, `[[item]]`
links between items, daily snapshots. Edits made to the Markdown files while the app is running are not
picked up until the next start (and may be overwritten by the app's next save).
