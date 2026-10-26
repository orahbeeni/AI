//! User settings: fonts, colours, behaviour and keybindings, stored as JSON in the config dir.

use std::collections::BTreeMap;
use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An RGB colour, stored in JSON as "#rrggbb".
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn hex(s: &str) -> Rgb {
        Rgb::parse(s).unwrap_or_default()
    }

    pub fn parse(s: &str) -> Option<Rgb> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        Some(Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    pub fn color(self) -> egui::Color32 {
        egui::Color32::from_rgb(self.0, self.1, self.2)
    }

    pub fn luminance(self) -> f32 {
        (0.299 * self.0 as f32 + 0.587 * self.1 as f32 + 0.114 * self.2 as f32) / 255.0
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgb::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("bad colour {s:?}")))
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Theme {
    pub name: String,
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub cursor_text: Rgb,
    pub selection_bg: Rgb,
    pub selection_fg: Rgb,
    /// Colour of the pane dividers and the focused pane's title bar.
    pub accent: Rgb,
    /// The 16 ANSI colours: black, red, green, yellow, blue, magenta, cyan, white, then the bright ones.
    pub palette: [Rgb; 16],
}

impl Default for Theme {
    fn default() -> Self {
        Theme::presets().remove(0)
    }
}

fn pal(s: [&str; 16]) -> [Rgb; 16] {
    s.map(Rgb::hex)
}

impl Theme {
    #[allow(clippy::too_many_arguments)]
    fn new(name: &str, fg: &str, bg: &str, cursor: &str, sel_bg: &str, sel_fg: &str, accent: &str, palette: [&str; 16]) -> Theme {
        Theme {
            name: name.into(),
            foreground: Rgb::hex(fg),
            background: Rgb::hex(bg),
            cursor: Rgb::hex(cursor),
            cursor_text: Rgb::hex(bg),
            selection_bg: Rgb::hex(sel_bg),
            selection_fg: Rgb::hex(sel_fg),
            accent: Rgb::hex(accent),
            palette: pal(palette),
        }
    }

    pub fn presets() -> Vec<Theme> {
        let tango = [
            "#000000", "#cc0000", "#4e9a06", "#c4a000", "#3465a4", "#75507b", "#06989a", "#d3d7cf", "#555753", "#ef2929", "#8ae234",
            "#fce94f", "#729fcf", "#ad7fa8", "#34e2e2", "#eeeeec",
        ];
        vec![
            Theme::new("Terminator", "#ffffff", "#000000", "#aaaaaa", "#3465a4", "#ffffff", "#cc0000", tango),
            Theme::new(
                "Graphite",
                "#a9a9a9",
                "#212121",
                "#a9a9a9",
                "#404040",
                "#d0d0d0",
                "#5c5c5c",
                [
                    "#2b2b2b", "#a0524f", "#697545", "#a08a4f", "#5a7194", "#84607f", "#5f8a87", "#a9a9a9", "#606060", "#c26a66",
                    "#85935a", "#c4a865", "#7590b8", "#a37c9e", "#7aa8a4", "#d0d0d0",
                ],
            ),
            Theme::new("Tango Dark", "#d3d7cf", "#2e3436", "#d3d7cf", "#4a5559", "#ffffff", "#729fcf", tango),
            Theme::new(
                "Solarized Dark",
                "#839496",
                "#002b36",
                "#93a1a1",
                "#073642",
                "#93a1a1",
                "#268bd2",
                [
                    "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5", "#002b36", "#cb4b16",
                    "#586e75", "#657b83", "#839496", "#6c71c4", "#93a1a1", "#fdf6e3",
                ],
            ),
            Theme::new(
                "Solarized Light",
                "#657b83",
                "#fdf6e3",
                "#586e75",
                "#eee8d5",
                "#586e75",
                "#268bd2",
                [
                    "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5", "#002b36", "#cb4b16",
                    "#586e75", "#657b83", "#839496", "#6c71c4", "#93a1a1", "#fdf6e3",
                ],
            ),
            Theme::new(
                "Dracula",
                "#f8f8f2",
                "#282a36",
                "#f8f8f2",
                "#44475a",
                "#f8f8f2",
                "#bd93f9",
                [
                    "#21222c", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2", "#6272a4", "#ff6e6e",
                    "#69ff94", "#ffffa5", "#d6acff", "#ff92df", "#a4ffff", "#ffffff",
                ],
            ),
            Theme::new(
                "One Dark",
                "#abb2bf",
                "#282c34",
                "#528bff",
                "#3e4451",
                "#ffffff",
                "#61afef",
                [
                    "#282c34", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf", "#5c6370", "#e06c75",
                    "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#ffffff",
                ],
            ),
            Theme::new(
                "Gruvbox Dark",
                "#ebdbb2",
                "#282828",
                "#ebdbb2",
                "#504945",
                "#ebdbb2",
                "#d79921",
                [
                    "#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#a89984", "#928374", "#fb4934",
                    "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c", "#ebdbb2",
                ],
            ),
            Theme::new(
                "Monokai",
                "#f8f8f2",
                "#272822",
                "#f8f8f0",
                "#49483e",
                "#f8f8f2",
                "#a6e22e",
                [
                    "#272822", "#f92672", "#a6e22e", "#f4bf75", "#66d9ef", "#ae81ff", "#a1efe4", "#f8f8f2", "#75715e", "#f92672",
                    "#a6e22e", "#f4bf75", "#66d9ef", "#ae81ff", "#a1efe4", "#f9f8f5",
                ],
            ),
            Theme::new(
                "Nord",
                "#d8dee9",
                "#2e3440",
                "#d8dee9",
                "#434c5e",
                "#eceff4",
                "#88c0d0",
                [
                    "#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0", "#4c566a", "#bf616a",
                    "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4",
                ],
            ),
            Theme::new(
                "Light",
                "#1e1e1e",
                "#ffffff",
                "#1e1e1e",
                "#b5d5ff",
                "#000000",
                "#3465a4",
                [
                    "#000000", "#c91b00", "#00a600", "#a68b00", "#0225c7", "#ca30c7", "#00a6b2", "#bfbfbf", "#686868", "#ff6e67",
                    "#3ac142", "#c7c400", "#6871ff", "#ff77ff", "#3ac9d6", "#ffffff",
                ],
            ),
        ]
    }

    /// Colour for an index into the 256-colour xterm palette.
    pub fn indexed(&self, i: usize) -> Rgb {
        match i {
            0..=15 => self.palette[i],
            16..=231 => {
                let i = i - 16;
                let step = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
                Rgb(step(i / 36), step((i / 6) % 6), step(i % 6))
            }
            232..=255 => {
                let v = (8 + (i - 232) * 10) as u8;
                Rgb(v, v, v)
            }
            _ => self.foreground,
        }
    }
}

/// When quitting asks for confirmation.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum QuitConfirm {
    Always,
    /// Only while a terminal is running something other than its shell.
    WhenRunning,
    Never,
}

/// What a tab shows (a name given with "Rename tab" always wins).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum TabTitle {
    /// The focused terminal's title.
    Terminal,
    /// The saved layout the tab came from, else the terminal's title.
    Layout,
    /// "layout · terminal" for tabs from a layout, else the terminal's title.
    LayoutAndTerminal,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum CursorKind {
    Block,
    Beam,
    Underline,
}

/// Every action a key combination can be bound to: (id, label).
pub const ACTIONS: &[(&str, &str)] = &[
    ("copy", "Copy selection"),
    ("paste", "Paste"),
    ("select_all", "Select all"),
    ("split_horizontal", "Split horizontally (stacked)"),
    ("split_vertical", "Split vertically (side by side)"),
    ("close_pane", "Close terminal"),
    ("new_tab", "New tab"),
    ("close_tab", "Close tab"),
    ("next_tab", "Next tab"),
    ("prev_tab", "Previous tab"),
    ("next_pane", "Focus next terminal"),
    ("prev_pane", "Focus previous terminal"),
    ("focus_up", "Focus terminal above"),
    ("focus_down", "Focus terminal below"),
    ("focus_left", "Focus terminal to the left"),
    ("focus_right", "Focus terminal to the right"),
    ("resize_up", "Move divider up"),
    ("resize_down", "Move divider down"),
    ("resize_left", "Move divider left"),
    ("resize_right", "Move divider right"),
    ("zoom", "Zoom / unzoom terminal"),
    ("equalize", "Make all terminals equal size"),
    ("scroll_lock", "Toggle scroll lock (freeze view)"),
    ("scroll_page_up", "Scroll page up"),
    ("scroll_page_down", "Scroll page down"),
    ("scroll_top", "Scroll to top"),
    ("scroll_bottom", "Scroll to bottom"),
    ("find", "Find in scrollback"),
    ("prev_prompt", "Jump to previous prompt"),
    ("next_prompt", "Jump to next prompt"),
    ("select_output", "Select the last command's output"),
    ("copy_output", "Copy the last command's output"),
    ("goto", "Go to a terminal or tab…"),
    ("command_history", "Command history…"),
    ("record", "Record this terminal to a file"),
    ("clear", "Clear scrollback and reset"),
    ("broadcast", "Cycle broadcast input (off / tab / all)"),
    ("commands", "Run a saved command"),
    ("save_layout", "Save layout"),
    ("layouts", "Layouts"),
    ("font_bigger", "Increase font size"),
    ("font_smaller", "Decrease font size"),
    ("font_reset", "Reset font size"),
    ("fullscreen", "Toggle full screen"),
    ("preferences", "Preferences"),
    ("hide", "Hide term (macOS)"),
    ("minimize", "Minimise window"),
    ("quit", "Quit"),
];

fn default_keybindings() -> BTreeMap<String, Vec<String>> {
    let mac = cfg!(target_os = "macos");
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut b = |a: &str, keys: &[&str], mac_keys: &[&str]| {
        let mut v: Vec<String> = keys.iter().map(|s| s.to_string()).collect();
        if mac {
            v.extend(mac_keys.iter().map(|s| s.to_string()));
        }
        m.insert(a.to_string(), v);
    };
    b("copy", &["Ctrl+Shift+C"], &["Cmd+C"]);
    b("paste", &["Ctrl+Shift+V", "Shift+Insert"], &["Cmd+V"]);
    b("select_all", &["Ctrl+Shift+A"], &["Cmd+A"]);
    b("split_horizontal", &["Ctrl+Shift+O"], &["Cmd+Shift+D"]);
    b("split_vertical", &["Ctrl+Shift+E"], &["Cmd+D"]);
    b("close_pane", &["Ctrl+Shift+W"], &["Cmd+W"]);
    b("new_tab", &["Ctrl+Shift+T"], &["Cmd+T"]);
    b("close_tab", &[], &["Cmd+Shift+W"]);
    b("next_tab", &["Ctrl+PageDown"], &["Cmd+Shift+CloseBracket"]);
    b("prev_tab", &["Ctrl+PageUp"], &["Cmd+Shift+OpenBracket"]);
    b("next_pane", &["Ctrl+Shift+N", "Ctrl+Tab"], &["Cmd+CloseBracket"]);
    b("prev_pane", &["Ctrl+Shift+P", "Ctrl+Shift+Tab"], &["Cmd+OpenBracket"]);
    b("focus_up", &["Alt+Up"], &["Cmd+Alt+Up"]);
    b("focus_down", &["Alt+Down"], &["Cmd+Alt+Down"]);
    b("focus_left", &["Alt+Left"], &["Cmd+Alt+Left"]);
    b("focus_right", &["Alt+Right"], &["Cmd+Alt+Right"]);
    b("resize_up", &["Ctrl+Shift+Up"], &["Cmd+Ctrl+Up"]);
    b("resize_down", &["Ctrl+Shift+Down"], &["Cmd+Ctrl+Down"]);
    b("resize_left", &["Ctrl+Shift+Left"], &["Cmd+Ctrl+Left"]);
    b("resize_right", &["Ctrl+Shift+Right"], &["Cmd+Ctrl+Right"]);
    b("zoom", &["Ctrl+Shift+X"], &["Cmd+Shift+Enter"]);
    b("equalize", &["Ctrl+Shift+Equals"], &[]);
    b("scroll_lock", &["Ctrl+Shift+Space"], &["Cmd+Shift+Space"]);
    b("scroll_page_up", &["Shift+PageUp"], &[]);
    b("scroll_page_down", &["Shift+PageDown"], &[]);
    b("scroll_top", &["Shift+Home"], &["Cmd+Home"]);
    b("scroll_bottom", &["Shift+End"], &["Cmd+End"]);
    b("find", &["Ctrl+Shift+F"], &["Cmd+F"]);
    b("prev_prompt", &["Ctrl+Shift+PageUp"], &["Cmd+Shift+Up"]);
    b("next_prompt", &["Ctrl+Shift+PageDown"], &["Cmd+Shift+Down"]);
    b("select_output", &["Ctrl+Shift+J"], &["Cmd+Shift+J"]);
    b("copy_output", &["Ctrl+Shift+Y"], &["Cmd+Shift+Y"]);
    b("goto", &["Ctrl+Shift+K"], &["Cmd+Shift+K"]);
    b("command_history", &["Ctrl+Shift+H"], &["Cmd+Y"]);
    b("record", &[], &[]);
    b("clear", &["Ctrl+Shift+G"], &["Cmd+K"]);
    b("broadcast", &["Ctrl+Shift+B"], &["Cmd+Shift+B"]);
    b("commands", &["Ctrl+Shift+R"], &["Cmd+Shift+R"]);
    b("save_layout", &["Ctrl+Shift+S"], &["Cmd+Shift+S"]);
    b("layouts", &["Ctrl+Shift+L"], &["Cmd+Shift+L"]);
    b("font_bigger", &["Ctrl+Plus", "Ctrl+Equals"], &["Cmd+Plus", "Cmd+Equals"]);
    b("font_smaller", &["Ctrl+Minus"], &["Cmd+Minus"]);
    b("font_reset", &["Ctrl+0"], &["Cmd+0"]);
    b("fullscreen", &["F11"], &["Cmd+Ctrl+F"]);
    b("preferences", &["Ctrl+Shift+Comma"], &["Cmd+Comma"]);
    b("quit", &["Ctrl+Shift+Q"], &["Cmd+Q"]);
    b("hide", &[], &["Cmd+H"]);
    b("minimize", &[], &["Cmd+M"]);
    m
}

/// A parsed key combination such as "Ctrl+Shift+O".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Combo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub cmd: bool,
    pub key: egui::Key,
}

impl Combo {
    pub fn parse(s: &str) -> Option<Combo> {
        let mut c = Combo { ctrl: false, shift: false, alt: false, cmd: false, key: egui::Key::Escape };
        let mut key = None;
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => c.ctrl = true,
                "shift" => c.shift = true,
                "alt" | "option" | "opt" => c.alt = true,
                "cmd" | "command" | "super" | "meta" => c.cmd = true,
                _ => key = Some(egui::Key::from_name(part)?),
            }
        }
        c.key = key?;
        Some(c)
    }

    pub fn matches(&self, key: egui::Key, m: egui::Modifiers) -> bool {
        // Cmd only exists on macOS; bindings that use it are ignored elsewhere.
        if self.cmd && !cfg!(target_os = "macos") {
            return false;
        }
        let same_key = self.key == key
            || (self.key == egui::Key::Equals && key == egui::Key::Plus)
            || (self.key == egui::Key::Plus && key == egui::Key::Equals);
        same_key && self.ctrl == m.ctrl && self.shift == m.shift && self.alt == m.alt && self.cmd == m.mac_cmd
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Settings {
    /// Shell to run; empty means the user's login shell (or PowerShell on Windows).
    pub shell: String,
    pub shell_args: Vec<String>,
    /// Path to a .ttf/.otf/.ttc monospace font; empty uses the built-in Hack font.
    pub font_path: String,
    pub font_size: f32,
    /// Multiplier on the font's natural line height.
    pub line_height: f32,
    /// Space between a terminal's edge and its text, in points.
    pub padding: f32,
    pub theme: Theme,
    pub cursor_shape: CursorKind,
    pub cursor_blink: bool,
    pub scrollback_lines: usize,
    pub copy_on_select: bool,
    /// Freeze the view while a selection exists so live output cannot move the text being copied.
    pub lock_on_select: bool,
    pub middle_click_paste: bool,
    /// Every Ctrl+Shift+<key> binding also works as Ctrl+<key> (Ctrl+C copies only while text is selected).
    /// Not used on macOS.
    #[serde(alias = "plain_copy_paste")]
    pub ctrl_without_shift: bool,
    pub right_click_menu: bool,
    /// Ctrl+click (⌘+click on macOS) opens URLs and file paths in the output.
    pub clickable_links: bool,
    /// Opens `file:line` links. `{file}`, `{line}` and `{col}` are replaced; empty uses the system opener.
    pub editor_command: String,
    /// Desktop notifications for finished commands, watched text and programs that ask for one.
    pub desktop_notifications: bool,
    /// Notify when a command that ran at least `long_command_secs` finishes out of sight.
    pub notify_long_commands: bool,
    pub long_command_secs: u32,
    /// Also notify while looking at the terminal.
    pub notify_when_focused: bool,
    /// Play a ping whenever an alert is raised.
    pub alert_sound: bool,
    /// A sound file to play instead of the system's ping (wav, ogg, mp3 … depending on the OS's player).
    pub alert_sound_file: String,
    /// Colour bars beside each command's output and a status badge in the title bar (needs shell integration).
    pub command_marks: bool,
    /// Linux: run through X11 (XWayland) even in a Wayland session. Native Wayland can stop redrawing the
    /// window for good (a lost frame callback with some GPU drivers), which looks like a frozen window.
    pub force_x11: bool,
    /// Let programs read the clipboard through OSC 52. Off: a remote host cannot read what you copied.
    pub osc52_read: bool,
    /// Write every terminal's output to a text file as it arrives.
    pub record_output: bool,
    /// Folder for recordings; empty uses `logs` in the settings folder.
    pub record_dir: String,
    /// Keep each terminal's recent scrollback across restarts and crashes.
    pub persist_scrollback: bool,
    pub persist_lines: usize,
    /// Seconds between automatic saves of the session (0 = only when quitting).
    pub autosave_secs: u32,
    pub bold_is_bright: bool,
    pub faux_bold: bool,
    pub show_titlebars: bool,
    pub always_show_tabs: bool,
    pub tab_title: TabTitle,
    pub scroll_on_keystroke: bool,
    pub option_as_meta: bool,
    /// Characters, besides whitespace, that end a double-click word selection.
    pub word_separators: String,
    pub divider_width: f32,
    /// How much to dim terminals that are not focused (0 = not at all, 0.5 = half).
    pub inactive_dim: f32,
    pub restore_session: bool,
    pub run_commands_on_restore: bool,
    /// Name of a saved layout to open at startup instead of the last session.
    pub startup_layout: String,
    /// Offer the list of saved layouts when term starts.
    pub startup_picker: bool,
    /// Run a layout's auto-run commands when opening it from the list.
    pub launcher_run: bool,
    pub quit_confirm: QuitConfirm,
    pub keybindings: BTreeMap<String, Vec<String>>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            shell: String::new(),
            shell_args: Vec::new(),
            font_path: String::new(),
            font_size: 13.0,
            line_height: 1.0,
            padding: 10.0,
            theme: Theme::default(),
            cursor_shape: CursorKind::Block,
            cursor_blink: false,
            scrollback_lines: 20_000,
            copy_on_select: true,
            lock_on_select: true,
            middle_click_paste: true,
            ctrl_without_shift: false,
            right_click_menu: true,
            clickable_links: true,
            editor_command: String::new(),
            desktop_notifications: true,
            notify_long_commands: true,
            long_command_secs: 10,
            notify_when_focused: false,
            alert_sound: false,
            alert_sound_file: String::new(),
            command_marks: true,
            force_x11: true,
            osc52_read: false,
            record_output: false,
            record_dir: String::new(),
            persist_scrollback: true,
            persist_lines: 5000,
            autosave_secs: 30,
            bold_is_bright: true,
            faux_bold: true,
            show_titlebars: true,
            always_show_tabs: true,
            tab_title: TabTitle::LayoutAndTerminal,
            scroll_on_keystroke: true,
            option_as_meta: false,
            word_separators: ",│`|:\"' ()[]{}<>\t".into(),
            divider_width: 5.0,
            inactive_dim: 0.0,
            restore_session: true,
            run_commands_on_restore: false,
            startup_layout: String::new(),
            startup_picker: true,
            launcher_run: true,
            quit_confirm: QuitConfirm::Always,
            keybindings: default_keybindings(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    // TERM_CONFIG_DIR keeps a separate set of settings and layouts (also used by the tests).
    if let Some(d) = std::env::var_os("TERM_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("term")
}

pub fn layouts_dir() -> PathBuf {
    config_dir().join("layouts")
}

impl Settings {
    pub fn path() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn load() -> Settings {
        let mut s: Settings = std::fs::read_to_string(Self::path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        s.normalize();
        s
    }

    /// Fill in what older (or imported) settings lack and keep values in range.
    pub fn normalize(&mut self) {
        // New actions added in later versions get their default keys.
        for (action, keys) in default_keybindings() {
            self.keybindings.entry(action).or_insert(keys);
        }
        self.font_size = self.font_size.clamp(6.0, 72.0);
        self.long_command_secs = self.long_command_secs.clamp(1, 3600);
        self.persist_lines = self.persist_lines.clamp(100, 100_000);
        self.autosave_secs = self.autosave_secs.min(3600);
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(config_dir());
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), t);
        }
    }

    pub fn reset_keybindings(&mut self) {
        self.keybindings = default_keybindings();
    }

    /// Parsed bindings, for matching key presses.
    pub fn combos(&self) -> Vec<(String, Combo)> {
        let mut v = Vec::new();
        for (action, keys) in &self.keybindings {
            for k in keys {
                if let Some(c) = Combo::parse(k) {
                    v.push((action.clone(), c));
                }
            }
        }
        v
    }

    pub fn shortcut_label(&self, action: &str) -> String {
        let keys = self.keybindings.get(action).map(|v| v.as_slice()).unwrap_or(&[]);
        let has_cmd = |k: &&String| k.to_ascii_lowercase().contains("cmd");
        // On macOS show the ⌘ shortcut when there is one; elsewhere ⌘ shortcuts do not apply.
        let pick = if cfg!(target_os = "macos") { keys.iter().find(has_cmd).or(keys.first()) } else { keys.iter().find(|k| !has_cmd(k)) };
        pick.cloned().unwrap_or_default()
    }
}

/// Monospace fonts commonly installed on each platform, offered in Preferences.
pub fn known_fonts() -> Vec<(&'static str, &'static str)> {
    let candidates: &[(&str, &str)] = &[
        ("Menlo", "/System/Library/Fonts/Menlo.ttc"),
        ("SF Mono", "/System/Library/Fonts/SFNSMono.ttf"),
        ("Monaco", "/System/Library/Fonts/Monaco.ttf"),
        ("Courier New", "/System/Library/Fonts/Supplemental/Courier New.ttf"),
        ("DejaVu Sans Mono", "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"),
        ("DejaVu Sans Mono", "/usr/share/fonts/TTF/DejaVuSansMono.ttf"),
        ("Ubuntu Mono", "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf"),
        ("Liberation Mono", "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf"),
        ("Noto Sans Mono", "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf"),
        ("JetBrains Mono", "/usr/share/fonts/truetype/jetbrains-mono/JetBrainsMono-Regular.ttf"),
        ("Fira Code", "/usr/share/fonts/truetype/firacode/FiraCode-Regular.ttf"),
        ("Consolas", "C:\\Windows\\Fonts\\consola.ttf"),
        ("Cascadia Mono", "C:\\Windows\\Fonts\\CascadiaMono.ttf"),
        ("Lucida Console", "C:\\Windows\\Fonts\\lucon.ttf"),
    ];
    let mut out: Vec<(&str, &str)> = candidates.iter().copied().filter(|(_, p)| std::path::Path::new(p).exists()).collect();
    out.dedup_by_key(|(n, _)| *n);
    out
}
