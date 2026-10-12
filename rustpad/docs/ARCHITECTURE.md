# Architecture

RustPad is split into a UI-free **core** (everything that can be unit-tested without a window)
and a thin **egui** layer.

```
src/
  doc.rs        Document model: rope text, multiple selections, undo history, bookmarks, fold state,
                lexer caches. Every edit goes through Doc::exec / commit so undo is uniform.
  ops.rs        Text commands on a Doc: line ops, sort, case, trim, comments, column insert, brackets.
  search.rs     Find / replace engine (normal, extended, regex), in documents and on disk (threaded).
  lexer.rs      Line-oriented syntax lexer; state carried between lines is a tiny enum (LexState).
  lang.rs       Language table (extensions, comment syntax, keyword sets) — pure data.
  fold.rs       Fold-range computation (braces, indentation, tags, block comments).
  encoding.rs   BOM sniffing, UTF-16 detection, legacy code pages (encoding_rs / chardetng).
  rowmap.rs     Logical-line ⇄ visual-row mapping for word wrap and folded lines.
  util.rs       Display columns (tabs, wide chars), wrap breaking, small string helpers.
  hash.rs       Digests for the Tools menu.
  cmd.rs        The `Cmd` enum — every user action — plus the editable shortcut table.
  settings.rs   JSON settings.     theme.rs   Colour themes.     session.rs  Hot-exit snapshots.
  fonts.rs      Optional user font + CJK/symbol fallbacks.

  editor.rs     The editor widget: gutter, painting, caret/selection, mouse + keyboard input,
                scrollbars, auto-complete popup. Returns Cmds; mouse selection edits the Doc directly.
  app.rs        Application state: tabs, file I/O, command execution (App::exec), search glue, macros.
  ui/           Menus, tool/tab/status bars (menus.rs), dialogs (dialogs.rs), side panels (panels.rs),
                top-level frame composition and eframe::App impl (mod.rs).
  debug.rs      Input-replay test hook (RUSTPAD_SCRIPT).
```

## Data flow

1. egui delivers input. Global shortcuts are consumed first (`ui/mod.rs`, from `cmd::with_shortcuts`);
   the focused editor turns remaining key events into `Cmd`s.
2. Menus, toolbar, dialogs and the editor all return `Vec<Cmd>`.
3. `App::exec(cmd)` is the single entry point that mutates state. Macro recording simply
   appends recordable `Cmd`s; playback replays them through `exec`.
4. The editor paints visible rows only: rows come from `RowMap`, tokens from `Doc::line_spans`
   (cached lexer state per line, invalidated from the edit point onward).

## Design notes

- **Positions are char indices** into a `ropey::Rope` configured with `cr_lines` only, so
  `\n`, `\r\n` and `\r` are the only line breaks and mixed endings round-trip untouched.
  CR LF is treated as one unit by caret movement and deletion.
- **Selections** are a sorted, non-overlapping `Vec<Sel>`; all edit commands work per selection.
- **Undo** records (position, removed, inserted) triples per step; consecutive single-character
  typing/deleting coalesces. The saved state is a history index, so undoing back to the saved
  text clears the modified flag.
- **Large files**: folds and line-wrap are disabled above a size threshold; lexer state for a far
  jump is computed from a short look-back window instead of lexing everything before it;
  session snapshots clone the rope (O(1)) and write on a background thread.
- **Adding a language**: add a `LangDef` to `lang::LANGS` (generic kinds need only data; special
  kinds live in `lexer.rs`). **Adding a command**: add a `Cmd` variant, handle it in `App::exec`,
  add a menu entry and optionally a default shortcut in `cmd::default_bindings`.

## Testing

- Unit tests live next to the code. `src/fuzz_tests.rs` runs thousands of random edit sequences
  and checks invariants, undo-all round trips, lexer span validity and search robustness.
- `tests/ui/cases/*.txt` are headless end-to-end scripts (`tests/ui/run_ui_tests.py`).
