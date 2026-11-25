# Organise: plan (working name)

A list app for work and personal life. Lists nest as checkboxes or bullets, and any item can hold
**links** to files, folders, emails and web pages. Links follow their target when it moves.
A board shows lists as cards in columns. All data is plain text. Dragging things in is a desktop and
laptop feature. Phones view and edit the same lists, and can fetch a linked file from the PC that has it.

Designs: [design/screens/](design/screens/) (PNG), [design/prototype.html](design/prototype.html)
(click-through), source [design/organise.sketch.json](design/organise.sketch.json) (open in Sketchframe;
the `sketchframe` MCP server is registered for this workspace). `design/tools/build2.py` redraws every
screen in place through the MCP server and keeps the version history.

---

## 1. Is it possible? Feature by feature

| Feature | Possible? | Notes |
|---|---|---|
| Nested lists, checkboxes/bullets, drag to re-parent | Yes | Standard tree UI. Keyboard: Tab/Shift+Tab to nest, Alt+↑/↓ to move. |
| Categories (Work, Personal, your own) | Yes | Set on a list; sub-items inherit it unless overridden. |
| Board with editable columns, no deleting a non-empty column | Yes | Cards are lists (or any sub-list). |
| Drop a file/folder from anywhere → link (no copy) | Yes, all desktops | Explorer/Finder/Nautilus all hand over the path. |
| Click opens in the default app | Yes | `ShellExecute` / `open` / `xdg-open` (or the XDG portal). |
| Right-click → show in folder with the file selected | Yes | Windows `explorer /select,` (or `SHOpenFolderAndSelectItems`), macOS `NSWorkspace activateFileViewerSelectingURLs` (`open -R`), Ubuntu D-Bus `org.freedesktop.FileManager1.ShowItems` (Nautilus, Dolphin, Nemo). |
| **Links update when a file is moved/renamed by any app** | **Yes on Windows and macOS. Mostly on Linux.** | See §2. Obsidian only does this for moves made inside Obsidian; this goes further. |
| Drag an Outlook email in (Windows, **no sign-in**) | **Yes** | Classic Outlook: a live link to the real email. New Outlook: a saved copy of the email. See §3. |
| Other apps (Gmail, Jira, Teams, browser pages…) | Yes, one by one | Each is a "link provider" plugin (§3). Web links work from day one. |
| Everything saved as text, backup and restore | Yes | A folder of Markdown + YAML files (§5). |
| Sync between machines, both ways | Yes, the largest piece | Design for it now (stable IDs everywhere), build it last (§6). |
| A PC file seen on the phone or another laptop | Yes | Shows what the file is and where it lives. A copy can be fetched peer-to-peer from that PC (§6). |
| Android | Yes | Lists, board, emails/web links, Share-in. No dragging needed on phones. |
| iOS | Maybe | Same as Android. Needs a Mac and an Apple developer account to build and ship. |

## 2. How links follow moved files

Each file link stores **several ways to find the file again**. Resolution tries them in order:

1. **The stored path still works**, and the size/fingerprint roughly match: done.
2. **The watcher saw the move** while the app was running: the path was already updated.
3. **The OS finds the file by its identity**, even when the move happened while the app was closed:
   - **Windows (NTFS):** the file ID (`FILE_ID_INFO`) and the NTFS *object ID*, which is how Windows
     shortcuts (.lnk) find moved targets (Distributed Link Tracking). `OpenFileById` then
     `GetFinalPathNameByHandle` gives the new path. Needs no admin rights. The object ID survives
     `ReplaceFile`-style "safe saves" (Office) where the plain file ID would not.
     Live updates: `ReadDirectoryChangesW` on watched folders.
   - **macOS:** an `NSURL` **bookmark** (what Finder aliases use) finds the file after moves and renames.
     Live updates: FSEvents with file events.
   - **Linux:** the inode + device survive renames and moves within the same filesystem, but the kernel
     has no unprivileged "open by inode", so the app needs to know where to look. Live updates:
     **inotify** on the watched folders (home by default). The `MOVED_FROM`/`MOVED_TO` pair gives the
     old and new path. Moves outside the watched folders are found at step 4.
     (`fanotify` can watch a whole disk but needs root, so it isn't the default.)
4. **Content fingerprint search:** size + hash of the first and last 64 KB + name, matched against an index
   of the watched folders. This catches moves to another drive (a copy + delete, so the identity changes).
5. **Ask you** on the *Moved & missing* page, with the likely matches (screen 4).

## 3. Outlook (Windows, no sign-in) and other apps

First check which Outlook you have. If the toggle at the top right of the window says
**"Try the new Outlook"** and is off, it's **classic** Outlook (`outlook.exe`). If it says **"New Outlook"**
and is on, it's the new one (`olk.exe`).

- **Classic Outlook: a live link.** A drag from Outlook carries only a *copy* of the message
  (`FileGroupDescriptorW` + `FileContents`, a .msg). The app sees that the drop came from Outlook and asks
  the Outlook running on the same PC (its local COM API, no sign-in, no IT approval) which items were
  dragged: `Application.ActiveExplorer().Selection`. It stores:
  - `EntryID` + `StoreID`, to open the real message: `Namespace.GetItemFromID(...).Display()`
  - the **Internet Message-ID**, a permanent ID. If the EntryID stops working (archived, moved to
    another mailbox or PST), the app searches Outlook's stores for it.
  - subject, sender and date, for display.
  Meetings, contacts and tasks work the same way. Microsoft supports classic Outlook until at least 2029.
  *Check early:* if IT has locked down Outlook's "programmatic access", reading the sender address can
  trigger an Outlook security prompt. The EntryID and subject don't. Spike S2 tests this on your work PC.
- **New Outlook: a saved copy.** New Outlook has no local API, so without a sign-in nothing can link to
  the live message. Dragging an email out gives an `.eml` copy. Sometimes the email must be clicked first.
  The app saves the copy in `Organise/emails/` and links to that. Clicking it opens the copy in Outlook.
  It's a snapshot, so later replies aren't in it. The Message-ID is kept, so if you ever move to classic
  Outlook, the copies can be upgraded to live links.
- **Gmail and other web apps.** Drop the browser tab or paste the URL. Gmail message URLs don't change when
  labels change. No sign-in.
- **Plugin shape.** Each source is a *link provider*:
  `recognise(drop data) → Link`, `resolve(Link) → location | moved | missing`, `open(Link)`, `reveal(Link)`.
  The first four are files, web links, classic Outlook and Outlook `.eml` copies. Jira, Teams, Slack and
  OneNote can be added later.

## 4. Recommended technology

**Rust core + Tauri 2 shell + a web UI (TypeScript, Svelte or React).**

- One UI codebase for Windows, macOS, Linux, Android and iOS (Tauri 2 targets all five).
- Web drag-and-drop libraries for trees and boards are mature.
- The hard parts (file identity, watchers, Outlook COM, text storage, sync, file transfer) are OS-level
  Rust code, which fits your other projects. The core is a plain crate that can be tested without any UI.
- **Native drop target:** on Windows the app replaces the WebView's drop handling with its own
  `IDropTarget` (`windows` crate). Only then can it see Outlook's formats and pair the drop with the COM
  lookup. The equivalent on macOS reads `NSPasteboard`, including file promises. Proved in spike S2.

Rejected options:
- **egui** (used by Sketchframe and Cutline): winit only passes file paths, so no Outlook or URL drops.
  Rich text editing is weak and mobile support is thin.
- **Flutter**: workable (`super_drag_and_drop` handles Outlook's virtual files), but it adds a second
  language next to the Rust core.

Crates: `notify` (watchers, with our own Windows/macOS ID code beside it), `windows` (COM, NTFS IDs),
`objc2` (bookmarks), `serde_yaml`, `pulldown-cmark`, `xxhash-rust` (fingerprints),
`opener` (open + reveal), `mail-parser` (.eml headers), and later `loro` or `automerge` (sync) and
`iroh` (device-to-device connections).

```
organise/
  core/          Rust: model, Markdown/YAML store, link registry, resolver, search index
  tracker/       Rust: per-OS file identity + watchers (win / mac / linux modules)
  providers/     Rust: file, web link, outlook_classic (win), outlook_eml
  sync/          Rust: CRDT docs, device pairing, mailbox, peer-to-peer file fetch
  app/           Tauri 2 shell + web UI (lists, board, moved/missing, settings)
  design/        Sketchframe designs
```

## 5. Data on disk: plain text you can back up

```
Organise/
  workspace.yaml          categories (name, colour), settings
  board.yaml              columns (id, name, colour, order) and which list sits in which column, in order
  links.yaml              one entry per link: kind, name, size, fingerprint, cloud-relative path, email IDs
  lists/work/q4-budget-review.md     one Markdown file per top-level list
  emails/                 .eml copies dropped from new Outlook
  .machines/OMAR-PC.yaml  this machine's paths and file IDs/bookmarks/inodes per link
  .history/               daily snapshots
```

A list file (Obsidian can still read it):

```markdown
---
id: L7f3a
title: Q4 budget review
category: work
due: 2026-12-12
---
- [x] Collect department forecasts ^i01
  [Forecasts_Q4.xlsx](link:f19c2) [HR headcount.csv](link:f19c4)
- [ ] Reconcile with actuals ^i02
  - [ ] Check travel spend ^i04 [RE: Travel policy changes](link:m0a71)
  - [ ] Ask Dana about licence costs ^i05 [[Hiring – backend dev]]
```

- Every list, item, link and column has a **stable short ID**: `^i04` (Obsidian block-ID syntax) and
  `link:f19c2`. Moving an item, renaming a list or moving a file never breaks a reference, and sync can merge item by item.
- Lists hold only the link ID, so when a file moves, **one line in `.machines/<pc>.yaml` changes**.
- Paths are per machine because `C:\Users\omar\…` means nothing on a Mac.
- **Backup:** copy the folder, or *Export backup (.zip)*. **Restore:** unzip, or point the app at the folder.
  The output is git-friendly: stable order, one item per line.

## 6. Sync, and files that live on another device

**Lists sync. Files don't, unless you ask for one.**

- **Accounts:** sign in with Google. The account identifies you and pairs your devices.
- **Merging:** each list becomes a CRDT document (Loro or Automerge), so edits made offline on two machines
  merge without "which copy wins" conflicts. The text files stay the readable, editable copy. Edits made in a
  text editor are diffed back into the CRDT.
- **Transport:** pure peer-to-peer only works when two devices are online at the same time, and a phone and
  a PC often aren't. So: **(a)** a mailbox in *your own* Google Drive (`appDataFolder`), end-to-end
  encrypted, with no server for us to run; **(b)** direct device-to-device (iroh: QUIC with NAT hole
  punching, relay fallback) for instant sync when both are on.

**A file link on another device** (screen 10). Every file link syncs its details, but not its content:
name, type, size, date changed, fingerprint, which devices have it and where, and when each was last seen.
The phone or laptop shows *"Budget_pack_v3.pptx · 2.4 MB · on OMAR-PC (online now)"*. You can then choose:

- **Get a copy now:** if OMAR-PC is online, the file comes straight from it, peer-to-peer and encrypted.
- **Send it when OMAR-PC is next online:** the request waits in the sync mailbox. The PC sends the file the
  next time both are online. Optionally, files up to a size limit (say 50 MB) go through the Drive mailbox,
  so they arrive even if the two are never online together.
- **Keep on this device:** a pinned file. New versions are sent automatically.

Safety rules: only your own paired devices can ask. A device only serves files that are linked in your
lists, never anything else on the disk. Each device can turn serving off. A fetched copy is marked as a
copy, and changes to it don't go back to the PC (possible later). To serve files, the PC app has to be
running: a tray icon that starts at login.

## 7. Phases

**Phase 0: spikes. Prove the risky parts before building the UI.**
- **S1 tracker CLI**: link 50 files, then move, rename, safe-save, cross-drive move and delete them, with the
  app both running and closed. Report what each OS resolves. Windows, Ubuntu, then macOS.
  Include OneDrive and Google Drive folders.
- **S2 drop target**: a bare Tauri window on Windows that accepts Explorer files, Outlook emails (classic:
  EntryID via COM, then reopen; new: the .eml copy) and browser URLs. **Run it on your work PC.**
  Same window on Ubuntu (files + URLs).
- **S3 open/reveal** on all three desktops.

**Phase 1: MVP on Windows + Ubuntu**: lists tree with nesting and drag, checkboxes/bullets, file and
folder links with tracking, open/reveal, categories, plain-text store, export/restore. Screens 1, 2 and 6.
*Status (25 Nov 2026): Linux v0.1 built, together with the Phase 2 board and Links page. See
[README.md](README.md). Linux tracking watches only linked files' folders plus chosen folders (the watch
limit is far below this home folder's ~409k folders), and finds anything else by an inode search
outward from the old place.*

**Phase 2**: board with column add/rename/reorder/delete-when-empty (screen 3), *Moved & missing*
page (screen 4), `[[item]]` links with backlinks, search.

**Phase 3**: Outlook providers (classic live links, new-Outlook copies), web links, the provider plugin
interface, settings (screen 5).

**Phase 4**: sync (Google sign-in, CRDT, Drive mailbox, device-to-device), then fetching a file from another
device (screen 10). macOS build (bookmarks, FSEvents, pasteboard).

**Phase 5**: Android (Tauri mobile, Share target, screens 7–10). iOS if wanted.

## 8. What can't be done, and what's hard

Can't be fully done:
- **Following every moved file:** a deleted file, a drive or share that isn't connected, and a move made
  on another computer can't be found automatically. They show as "missing" with likely matches.
- **A live link to an email in new Outlook without a sign-in:** you get a saved copy instead (§3).

Hard but doable:
- **Sync**, the biggest piece of work (§6).
- **Moves on Ubuntu while the app is closed and outside watched folders:** found by fingerprint only,
  which fails if the file was also heavily edited.
- **Catching Outlook drops in a Tauri window:** needs the custom Windows drop target (spike S2).
- **Network drives and Google Drive for desktop:** file IDs may not stay the same after a move, so these
  may need regular re-checking instead of live tracking (spike S1).
- **Reaching a PC from a phone on another network:** hole punching usually works. When it doesn't, the
  connection goes through a relay. iroh has public relays, and we can run our own later.
- **Distribution:**
  - **macOS:** an Apple developer account ($99/year) so macOS doesn't block the app.
  - **Windows:** a code-signing certificate, or Windows will warn people the app is unrecognised.
  - **Mac App Store:** avoid it, because its rules stop an app from watching your whole disk. A direct
    download from your own site is fine.
  - **Ubuntu:** Snap and Flatpak packages limit disk access, so ship it as a .deb or AppImage.

## 9. Open questions

1. Classic or new Outlook on your work PC (see the top of §3)?
2. Is a *list* only a top-level thing, or can any sub-list sit on the board as its own card? (The designs
   allow both, with a "Show sub-lists as cards" toggle.)
3. One board, or one per category (a Work board and a Personal board)?
4. The name.
