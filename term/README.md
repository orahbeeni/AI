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
./deb_build.sh                      # build target/deb/term_<version>_<arch>.deb (Debian/Ubuntu)
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

## Shell integration: prompts, output and status

Most of the features below need the shell to tell term where each prompt and each command's output
starts and ends (the OSC 133 marks that iTerm2, kitty and VS Code also use). Turn it on in
**Preferences → Workflow → Shell integration** (or ☰ → *Shell integration…*): **Install** adds a
guarded line to your `~/.bashrc`, `~/.zshrc`, fish's `conf.d` or your PowerShell profile (it only
does anything inside term, and **Copy line** shows what it adds). Open a new terminal afterwards.

With it you get, per terminal:

- a green / red bar beside each command and its output, and a badge in the title bar with the last
  command's exit status and duration (`✗ 1 · 3.2s`, `⏵ 12s` while it runs);
- **jump to the previous / next prompt** (`Ctrl+Shift+PageUp` / `PageDown`, `⌘⇧↑` / `⌘⇧↓`);
- **select** or **copy the last command's output** (`Ctrl+Shift+J` / `Ctrl+Shift+Y`);
- **command history across all terminals** (`Ctrl+Shift+H`, `⌘Y`): type to filter, `Enter` types
  the command into the current terminal, `Shift+Enter` runs it. It covers this run of term only;
- alerts for finished commands that work on every OS (see below).

The marks stay on the right lines while output scrolls, also when the scrollback is full. Changing
the terminal's width re-wraps the text, which forgets the marks of older commands.

## Alerts

term tells you when something happens in a terminal you are not looking at: a toast in the window
(click *Go* to jump there), a dot on the tab, a 🔔 in the terminal's title bar and, when the window
is in the background, a desktop notification (`notify-send` on Linux, a notification on macOS, a
toast on Windows) plus a taskbar attention request. Triggers:

- **A long command ends** (default: after 10 seconds). With shell integration this reports the exit
  status, on any OS. Without it, term notices when the foreground job ends (Linux and macOS only,
  and not for full-screen programs such as vim).
- **Watched text appears.** Each terminal has **saved searches** (🔍 in its title bar, or ☆ in the
  Find bar to keep what you just searched for). Tick *Alert* on one to be told whenever it shows
  up in new output, e.g. `ERROR`, `Started Application` or the regex `Exception in thread`.
- **A program asks.** `printf '\e]9;Build done\a'` (or `\e]777;notify;Title;Body\a`) raises an alert,
  also from a remote host over ssh.

Tick **Play a ping sound** to hear one with each alert (the desktop's own message sound; on macOS
`Ping`; or pick a sound file). **Send a test notification** waits 5 seconds so you can switch to
another window, because desktops such as GNOME hide notifications from the window you are using; it
then says whether the notification was handed to the system.

Every trigger is in **Preferences → Workflow → Alerts**. Alerts are not raised for the terminal
you are looking at, unless you ask for that.

## Saved searches

Like saved commands, each terminal keeps a list of searches (🔍 in its title bar, or *Saved
searches* in the right-click menu). Click one to select its latest match in that terminal, click
again for earlier ones (`Shift`-click goes the other way). Plain text ignores case; tick *Regex*
for a regular expression. They are saved with layouts and the session.

## Links

Hold `Ctrl` (`⌘` on macOS) and the URL or file name under the pointer lights up; click to open it.
`src/main.rs:45:10`, `Foo.java:123` (from a stack trace), `File "x.py", line 12` and `x.ts(12,5)` open
at that line in your editor (**Preferences → Workflow → Links**: VS Code, IntelliJ, Sublime, Kate,
gedit or any command with `{file}`, `{line}`, `{col}`). Relative names are looked up from the
terminal's folder and below it, so a bare `Foo.java` finds its file; names that lead nowhere are not
links. Programs' own hyperlinks (OSC 8, e.g. `ls --hyperlink`) work too.

## Over ssh, in containers

- **Keep alive:** tick it on a saved command such as `ssh build-server`. When it ends with an error
  (a dropped connection) term runs it again after 2, 5, 10, 30, then 60 seconds, and shows
  *↻ reconnecting* in the title bar (click to give up). Without shell integration term cannot see
  the exit status, so it offers *↻ Reconnect* instead.
- **Clipboard:** programs, including remote ones, can put text on your clipboard (OSC 52), so
  copying in a remote editor works, also through tmux with `set -g set-clipboard on`. Reading your
  clipboard is off unless you allow it in Preferences → Workflow → Remote sessions.
- Alerts and the finished-command notification work over ssh when the *remote* shell has the
  integration installed, or the remote script sends `printf '\e]9;…\a'`.

## Recording and keeping your scrollback

- **Record to a file** (right-click) writes one terminal's output as plain text, without colours or
  escape codes, to `logs/` in the settings folder (change it in Preferences). *Record every
  terminal* does it for all.
- **Scrollback survives restarts and crashes.** The last 5,000 lines of every terminal are saved
  with the session (when quitting and every 30 seconds) and shown, dimmed, above the new prompt
  next time. Adjust or switch off under *Recording and session*.

## Moving around

- **Go to…** (`Ctrl+Shift+K`, `⌘⇧K`) is a fuzzy picker over every terminal, your saved layouts and the
  current terminal's saved commands.
- **Move a terminal to another tab** by dragging its title bar onto the tab (or onto +), or with
  right-click → *Move to tab*.
- A new split or tab opens in the folder of the terminal you split.

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
| Previous / next prompt | `Ctrl+Shift+PageUp` / `PageDown` (`⌘⇧↑` / `⌘⇧↓`) |
| Select / copy last output | `Ctrl+Shift+J` / `Ctrl+Shift+Y` |
| Go to…, command history | `Ctrl+Shift+K`, `Ctrl+Shift+H` (`⌘⇧K`, `⌘Y`) |
| Open a link | `Ctrl`+click (`⌘`+click) |
| Broadcast typing to the tab / all | `Ctrl+Shift+B` (cycles) |
| Saved commands, save layout, layouts | `Ctrl+Shift+R`, `Ctrl+Shift+S`, `Ctrl+Shift+L` |
| Font size | `Ctrl+=` / `Ctrl+-` / `Ctrl+0` |

Every binding can be changed in **Preferences → Keybindings**. On Linux / Windows, **Preferences →
Selection & clipboard → Shortcuts work without Shift** makes every `Ctrl+Shift+<key>` binding also
work as `Ctrl+<key>` (unless that key is bound to something else). `Ctrl+C` copies only while text
is selected (and clears the selection), otherwise it still interrupts. The shell no longer sees
those Ctrl keys, so readline's `Ctrl+W`, `Ctrl+R`, `Ctrl+L` etc. are taken by term.

## If the window stops redrawing (Linux)

Native Wayland can stop redrawing a window for good when the compositor loses a frame callback
(seen with NVIDIA's driver): the window frame still moves and resizes, but the contents are frozen
and nothing helps. By default term therefore runs through X11 (XWayland) when started in a Wayland
session; switch it in **Preferences → General** (takes effect after a restart). `term.log` in the
settings folder notes which window system was used and logs a `STALL:` line if output keeps arriving
but no frame is drawn.

## Customising

Preferences (☰ menu) cover the shell, font and size, line height, padding, cursor, colours
(11 presets plus every colour and the 16-colour palette), dividers, title bars, dimming of
unfocused terminals, clipboard behaviour and keybindings. Everything is stored as JSON:

- macOS: `~/Library/Application Support/term/`
- Linux: `~/.config/term/`
- Windows: `%APPDATA%\term\`

containing `settings.json`, `session.json`, `layouts/*.json`, `scrollback/` (each terminal's saved
output), `logs/` (recordings) and `shell-integration/`.

## Development

```sh
cargo test                                                    # unit + real-shell tests (incl. bash integration)
cargo test -- --ignored --test-threads=1                      # also the offscreen UI tests
TERM_SHOTS=/tmp/shots cargo test visual -- --ignored          # render the app offscreen to PNGs
```
