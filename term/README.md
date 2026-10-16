# term

A fast split-pane terminal for **macOS, Linux and Windows**, modelled on Terminator: many
resizable terminals in one window, tabs, saved layouts with per-terminal folders and commands,
and selection that stays put while a log is streaming.

Built in Rust with egui/eframe for the UI, `alacritty_terminal` for terminal emulation and
`portable-pty` for the shell (Unix PTYs and Windows ConPTY).

## Running

```sh
cargo run --release                 # one terminal, or the last session
cargo run --release -- ~/projects   # start in a folder
cargo run --release -- -l hybris    # open the saved layout "hybris"
cargo run --release -- -l hybris -r # …and run its auto-run commands
./mac_bundle.sh --install           # build /Applications/Term.app
```

## Copying from a live log

Text selection is anchored to the text, not to the screen. While you select (or whenever you
have scrolled back) the view stays still, even though output keeps arriving underneath. A badge
in the corner counts the new lines; click it, scroll to the bottom, or start typing to follow
the output again. **Scroll lock** (🔒 in the terminal's title bar, or `Ctrl+Shift+Space`)
freezes a terminal on purpose, for example while reading a server start-up.

Also: double-click selects a word, triple-click a line, `Alt`-drag a block, `Shift`-click extends.
**Find** (`Ctrl+Shift+F` / `⌘F`) searches the whole scrollback. The right-click menu can copy
or save the whole output to a file.

## Layouts and saved commands

Each terminal keeps a list of **saved commands** (▶ in its title bar). You can run any of them
with a click. Commands marked *auto-run* are typed in automatically when a layout opens, after
the shell has drawn its prompt.

**Save layout** (`Ctrl+Shift+S`) asks for a name and stores the tabs, the splits and their sizes,
and each terminal's current folder, name and saved commands.

When term starts and you have saved layouts, it shows **Open a layout**: type to filter, `↑`/`↓` to
choose, `Enter` to open (or "Not now" to keep a plain terminal). The same list is under
**Layouts** (`Ctrl+Shift+L`), where you can also rename (✏), delete (🗑) or mark a layout to always
open at startup (★, which skips the list). Opening can replace the current tabs or add to them,
with or without running the auto-run commands. Turn the list off with "Show this list when term
starts". Tabs opened from (or saved as) a layout show its name, e.g. "hybris · core-customize";
Preferences → Appearance → "Tab titles show" switches between the terminal title, the layout name,
or both. The last session is restored underneath; its auto-run commands only run if you enable
that in Preferences.

Example: a "hybris" layout with one terminal in `hybris/bin/platform` and the auto-run commands
`. ./setantenv.sh` and `./hybrisserver.sh debug`, next to a terminal for builds.

## Keys (Linux / Windows; macOS also has ⌘ equivalents)

| Action | Keys |
|---|---|
| Split horizontally / vertically | `Ctrl+Shift+O` / `Ctrl+Shift+E` (`⌘⇧D` / `⌘D`) |
| Close terminal | `Ctrl+Shift+W` |
| New tab, next / previous tab | `Ctrl+Shift+T`, `Ctrl+PageDown` / `Ctrl+PageUp` |
| Move focus | `Alt+Arrows`, `Ctrl+Shift+N` / `Ctrl+Shift+P` |
| Move a divider | `Ctrl+Shift+Arrows` (or drag it) |
| Zoom a terminal, equalise sizes | `Ctrl+Shift+X`, `Ctrl+Shift+=` |
| Copy / paste | `Ctrl+Shift+C` / `Ctrl+Shift+V` (`⌘C` / `⌘V`) |
| Scroll lock, find, clear | `Ctrl+Shift+Space`, `Ctrl+Shift+F`, `Ctrl+Shift+G` |
| Broadcast typing to the tab / all | `Ctrl+Shift+B` (cycles) |
| Saved commands, save layout, layouts | `Ctrl+Shift+R`, `Ctrl+Shift+S`, `Ctrl+Shift+L` |
| Font size | `Ctrl+=` / `Ctrl+-` / `Ctrl+0` |

Every binding can be changed in **Preferences → Keybindings**.

## Customising

Preferences (☰ menu) cover the shell, font and size, line height, padding, cursor, colours
(11 presets plus every colour and the 16-colour palette), dividers, title bars, dimming of
unfocused terminals, clipboard behaviour and keybindings. Everything is stored as JSON:

- macOS: `~/Library/Application Support/term/`
- Linux: `~/.config/term/`
- Windows: `%APPDATA%\term\`

containing `settings.json`, `session.json` and `layouts/*.json`.

## Development

```sh
cargo test                                                    # unit + real-shell tests
TERM_SHOTS=/tmp/shots cargo test visual -- --ignored          # render the app offscreen to PNGs
```
