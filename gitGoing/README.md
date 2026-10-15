# gitgoing

A fast, memory-frugal, cross-platform desktop git client written in Rust, with a
graph-first interface in the spirit of other visual git clients. Built clean-room:
the code is original and uses no assets or code from any other client.

Status: working prototype. Developed and tested on macOS; Linux and Windows are
expected to work (all dependencies are cross-platform) but have not been run yet.

## Features

- Streaming commit graph with lanes, branch/tag chips, search, keyboard navigation
- Stage and unstage files, hunks, or individual lines; discard changes; commit; amend
- Diff view: unified or side by side, whole-file view, word-level highlights, syntax colours
- Branches, remotes, tags, stashes in a filterable sidebar; checkout, create, delete, merge, rebase
- Fetch, pull (merge / fast-forward only / rebase), push, force push with lease
- Cherry-pick, revert, reset, undo and redo of ref-changing operations
- Interactive rebase, and squashing several commits (with a conflict check before it runs)
- Conflict editor in its own window: ours and theirs side by side, pick whole blocks or single
  lines in any order, editable result
- Tabs for several repositories, clone and init, settings, a console showing the git commands used

## Build and install

You need a recent stable [Rust toolchain](https://rustup.rs) (install it with `rustup`) and `git` on
your PATH. libgit2 is vendored and compiled from source, so a C compiler is needed as well. The
result is a single self-contained executable, `target/release/gitgoing`.

Get the code and build it the same way on every OS:

```
git clone <this repository> gitgoing
cd gitgoing
cargo build --release -p gitgoing
```

Then follow the steps for your system. Only macOS has been tested so far; the Linux and
Windows steps are what the dependencies require but have not been run yet.

### macOS

```
xcode-select --install     # C compiler and git, if you don't have them yet
./mac_bundle.sh --install  # builds target/macos/gitgoing.app and copies it into /Applications
```

After that gitgoing is in Launchpad and Spotlight, with its icon. Without `--install` the script only
builds `target/macos/gitgoing.app`; drag it into Applications yourself. `--no-build` reuses an existing
release build. To remove the app, delete it from Applications.

The bundle is signed ad hoc, which is enough to run it on the Mac that built it. If you copy it to another
Mac, Gatekeeper will block it at first: right-click the app and choose Open once.

If you only want the command-line program, build as above and
`sudo install -m 755 target/release/gitgoing /usr/local/bin/gitgoing`, then run `gitgoing /path/to/repo`.

### Linux

Install a C toolchain, `pkg-config` and git, plus the usual windowing libraries.

```
# Debian / Ubuntu
sudo apt install build-essential pkg-config git libgl1 libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgtk-3-dev
# Fedora
sudo dnf install gcc make pkgconf-pkg-config git mesa-libGL-devel libxkbcommon-devel wayland-devel libX11-devel libXcursor-devel libXrandr-devel libXi-devel gtk3-devel
# Arch
sudo pacman -S --needed base-devel git mesa libxkbcommon wayland libx11 libxcursor libxrandr libxi gtk3
```

(`gtk3` is used by the file dialogs.) Then build, and install by either of:

```
sudo install -m 755 target/release/gitgoing /usr/local/bin/gitgoing   # any distro
./deb_build.sh && sudo dpkg -i target/deb/gitgoing_*.deb              # Debian / Ubuntu package
```

`deb_build.sh` also installs the icon and a desktop entry, so the app shows up in your launcher.
`./deb_build.sh --no-build` reuses an existing release build. To remove it: `sudo apt remove gitgoing`.

### Windows

1. Install [Git for Windows](https://git-scm.com/download/win) and make sure `git` works from a terminal.
2. Install [Rustup](https://rustup.rs) and, when asked, the **Visual Studio C++ Build Tools**
   (the "Desktop development with C++" workload). Use the default `x86_64-pc-windows-msvc` toolchain.
3. In PowerShell or the "x64 Native Tools" prompt, from the project folder:

```
cargo build --release -p gitgoing
```

The program is `target\release\gitgoing.exe`. Copy it anywhere you like (for example
`C:\Tools\`) and add that folder to your PATH, or create a shortcut to it. Passwords and passphrases
are requested through the app's own prompt window; if that doesn't work on your setup, use a
credential manager such as Git Credential Manager (included with Git for Windows).

### Run and test

```
cargo run --release -- /path/to/repo          # or run it without a path and use the + menu
cargo test --workspace
```

Fetch, pull, push, clone and rebase use the `git` command on your PATH, so your credential
helpers and SSH setup work as usual. Passwords and SSH passphrases are asked for in a small
prompt window (the app registers itself as git's askpass helper).

Command-line extras: `--resolve <file>` opens the conflict editor on a conflicted file;
`--console` opens the command console at start; `--no-splash` skips the start-up intro animation
(it can also be turned off in Settings). The intro plays in a borderless window that then becomes the
normal window; the window's last size and place are remembered in a small file next to the app's settings.

The logo and application icon live in `crates/app/assets/` (full logo, the snail-only app icon, and a
transparent snail mark for tiny sizes) and are embedded in the executable. The intro animation is
stored as JPEG frames in `crates/app/assets/intro.bin`; regenerate it from a video with
`scripts/make-intro.sh video.mp4` (needs ffmpeg; audio is not included; the default 1152x648 at 24 fps is about 6 MB).

## Trying things out

The `scripts/` folder builds throwaway repositories for the features that need a specific setup:

| Script | Creates |
|---|---|
| `make-conflict-repo.sh` | a merge stopped on several kinds of conflict |
| `make-history-repo.sh` | a linear history for squashing and rebasing |
| `make-squash-conflict-repo.sh` | commits where squashing non-adjacent ones conflicts |
| `make-force-push-repo.sh` | a pushed branch with a local "remote", for force pushing |

## Layout

- `crates/core` (`gitgoing-core`): all git logic, no UI. Uses libgit2 (vendored) for reads and
  local writes, and the system `git` for network operations and rebase.
- `crates/app` (`gitgoing`): the egui/eframe application.
  `app.rs` holds the tabs and per-repository state; `app/` has the graph, toolbar and dialogs;
  the other modules are one window or panel each (`changes`, `detail`, `diffview`, `sidebar`,
  `conflictwin`, `consolewin`, `settings`, ...).

## Known limits

No file history or blame yet, no pull-request integrations, no submodule or LFS support,
and interactive rebase refuses ranges that contain merge commits.
