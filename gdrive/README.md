# Google Drive for Linux

A Drive client for Ubuntu that behaves like Google's Windows/Mac app in **"Mirror files"** mode:
Pick any Drive folders to sync, and **choose where each one lives on your computer**. Each synced folder is a normal local folder kept in two-way sync with its Drive folder (not your whole Drive).

| Feature | |
|---|---|
| Two-way sync, changes picked up within ~30s (remote) / ~3s (local) | yes |
| Works offline, any app can use the files | yes |
| Tray icon: status, sync now, pause, open folder | yes (needs one apt package) |
| Rename / move / delete propagate (deletes go to Drive's trash) | yes |
| Conflicts: both versions kept (`name (conflicted copy ...)`) | yes |
| Google Docs/Sheets/Slides appear as `.gdoc/.gsheet/.gslides`; double-click opens in browser | yes |
| Desktop notifications, autostart at login, "Google Drive" bookmark in Files | yes |
| Safety stop if the folder suddenly looks emptied (e.g. unmounted) | yes |
| "Stream files" mode (on-demand virtual drive) | no, mirror only |
| Choose which Drive folders to sync, and the local parent folder for each (e.g. Photos -> `/mnt/data/Pictures`, Work -> `~/Documents`) | yes |
| Folders from **Computers** (backups made by Drive for desktop on another machine), e.g. a Mac's backup. Default is download-only so the backup is never altered | yes (see below) |
| Download-only mode for any folder: keep a copy, never change Drive | yes |
| Shared drives / "Shared with me" / backing up arbitrary local folders (e.g. ~/Documents) to Drive | not yet |

## Install

```bash
./install.sh
sudo apt install gir1.2-ayatana-appindicator3-0.1   # tray icon (optional)
```

## Step 1: one-time Google setup (about 5 minutes)

Google requires every app to identify itself, so you create your own free credentials:

1. Go to <https://console.cloud.google.com/>, create a project (any name).
2. **APIs & Services > Library** > enable **Google Drive API**.
3. **APIs & Services > OAuth consent screen**: User type *External*, fill in the app name and your email.
   Add yourself as a *Test user*, then click **Publish app** (to "In production").
   *Do this: apps left in "Testing" have their sign-in expire every 7 days.*
   You'll get an "unverified app" warning when signing in; that's expected for your own app. Click Advanced > Continue.
4. **Credentials > Create credentials > OAuth client ID > Desktop app**. Download the JSON.
5. Save it as `~/.config/gdrive-linux/client_secret.json`.

## Use

Open **Google Drive** from the app menu. The window walks you through credentials and sign-in, then shows a tick-box tree of your Drive folders, sync status, and Sync now / Pause. It keeps syncing from the tray when closed (if the tray package is installed). The same things are available from the command line:

```bash
gdrive login      # opens the browser once
gdrive folders browse               # list top-level Drive folders
gdrive folders browse Work          # ...or the subfolders of one
gdrive folders add "Work/Projects" --to ~/Documents   # syncs to ~/Documents/Projects
gdrive folders add Photos                              # (asks where, if --to is left out)
gdrive folders                      # list synced folders and where they live
gdrive folders remove Photos        # stop syncing; your local files are kept
gdrive run                          # sync with no window
gdrive sync                         # one pass of everything, then exit
gdrive status
```

In the window, tick a folder and a dialog asks which folder on your computer it should go into (the Drive folder is created inside the one you pick; you can make a new one there). Changes made with `gdrive folders` apply within ~30s to a running app. Removing a folder only stops syncing: nothing is deleted on either side. Synced folders can't overlap (not nested on Drive, nor inside each other locally).

The first sync merges with whatever is already in the folder: identical files are matched, differing ones are kept as conflict copies.

## Computers (backups from another machine)

In the window, expand **Computers** and tick the computer or one of its folders. If it isn't listed,
open the folder in drive.google.com and use **Add a folder by link…**. On the command line:

```bash
gdrive folders browse Computers
gdrive folders add "Computers/my mac shoprite/Documents" --to ~/Backups   # download-only by default
gdrive folders add --link "https://drive.google.com/drive/computers/<id>" --to ~/Backups
```

Computers folders default to **download only**: you get a copy and Drive changes come down, but nothing you do
here ever changes the backup. Use `--two-way` (or "Two-way sync" in the window) if you want changes to flow back.
Download-only also works on any normal folder (`--download-only`).

## Safety

Nothing here ever permanently deletes anything on Drive: removals use Drive's **Trash** (30 days), and
overwrites create a new revision (File > Version history). Specifically:

- A local delete trashes the Drive copy only if the item hasn't changed on Drive since the last sync;
  otherwise the Drive edit wins and the file comes back.
- Each synced folder is mapped directly to its own Drive folder, so nothing outside it is ever touched. If the local folder
  is moved, renamed or unmounted, or the Drive folder is trashed, that pair stops with a warning and changes nothing.
- A Drive folder is only trashed if everything in it is known and was synced here.
- Deleting a `.gdoc`/`.gsheet` stub never deletes the document.
- Files Drive deletes are moved to your desktop Trash, not erased; locally edited files are never removed.
- Before overwriting a Drive file, it checks the file hasn't changed there meanwhile; if both changed you get a conflicted copy.
- It refuses to run if the folder looks emptied/unmounted, and only one sync can run at a time.
- Renaming a folder locally renames it on Drive (same folder, same sharing/links) instead of re-uploading.

`tests/test_fuzz.py` throws random edits/deletes/renames at both sides and asserts both converge with no content lost.

## Notes

- Uses the full `drive` scope (required to sync arbitrary files). Token is stored in
  `~/.config/gdrive-linux/token.json` (mode 0600). Nothing is sent anywhere except Google.
- Sync state lives in `~/.local/share/gdrive-linux/pairs/`, outside your synced folders.
- Files edited locally after being deleted remotely are kept and re-uploaded, never silently lost.
- Tests: `.venv/bin/python -m pytest` (runs against an in-memory fake Drive).
