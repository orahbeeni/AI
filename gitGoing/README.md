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

## Build and run

```
cargo run --release -- /path/to/repo          # or run it without a path and use the + menu
cargo test --workspace
```

Fetch, pull, push, clone and rebase use the `git` command on your PATH, so your credential
helpers and SSH setup work as usual. Passwords and SSH passphrases are asked for in a small
prompt window (the app registers itself as git's askpass helper).

Command-line extras: `--resolve <file>` opens the conflict editor on a conflicted file;
`--console` opens the command console at start; `--no-splash` skips the start-up logo.

The logo and application icon live in `crates/app/assets/` (full logo, the snail-only app icon, and a
transparent snail mark for tiny sizes) and are embedded in the executable.

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
