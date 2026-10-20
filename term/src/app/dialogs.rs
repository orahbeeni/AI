//! Preferences, layouts, saved commands, find, rename and other windows.

use std::collections::BTreeMap;

use alacritty_terminal::term::search::Match;
use eframe::egui::{self, Align2, Color32, Key, Rect, RichText, Sense, pos2, vec2};

use super::{Act, App, format_duration};
use crate::config::{ACTIONS, Combo, CursorKind, QuitConfirm, Rgb, Settings, TabTitle, Theme, known_fonts};
use crate::integration::Shell;
use crate::links::EDITOR_PRESETS;
use crate::notify;
use crate::pane::{SavedCommand, SavedSearch};
use crate::search::{self, escape_regex, fuzzy};
use crate::session;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum PrefsPage {
    #[default]
    General,
    Appearance,
    Colours,
    Workflow,
    Keys,
}

pub struct Prefs {
    draft: Settings,
    page: PrefsPage,
    keys: BTreeMap<String, String>,
    /// The theme in effect before previewing, restored on Cancel.
    applied_theme: Theme,
    /// Outcome of the last shell-integration install.
    message: Option<String>,
    /// Outcome of the last notification or sound test, filled in from the thread that ran it.
    test_result: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    test_message: Option<String>,
}

/// A type-to-filter list opened over the window (go to…, command history).
#[derive(Default)]
pub struct Picker {
    query: String,
    selected: usize,
    focus: bool,
    /// Names offered besides the terminals (saved layouts), read once when the list opens.
    names: Vec<String>,
}

pub struct LayoutsDlg {
    filter: String,
    items: Vec<session::Summary>,
    /// Name of the highlighted layout.
    selected: Option<String>,
    replace: bool,
    focus: bool,
    at_startup: bool,
    rename: Option<(String, String)>,
    error: Option<String>,
}

pub struct ImportDlg {
    path: std::path::PathBuf,
    bundle: crate::transfer::Bundle,
    use_settings: bool,
    use_layouts: bool,
    /// Replace saved layouts that have the same name as one in the file.
    replace: bool,
    /// Names in the file that already exist here.
    clashes: Vec<String>,
}

pub struct SaveLayout {
    pub name: String,
    pub only_tab: bool,
    focus: bool,
    /// Saved layouts, offered for replacing.
    existing: Vec<String>,
}

pub struct Find {
    pane: u64,
    query: String,
    regex: bool,
    last: Option<Match>,
    status: String,
    focus: bool,
}

#[derive(Default)]
pub struct Dialogs {
    pub prefs: Option<Prefs>,
    pub layouts: Option<LayoutsDlg>,
    pub save_layout: Option<SaveLayout>,
    pub commands: Option<u64>,
    pub searches: Option<u64>,
    searches_focus: bool,
    new_search: String,
    new_search_regex: bool,
    new_search_alert: bool,
    pub goto: Option<Picker>,
    pub history: Option<Picker>,
    /// Focus the empty "new command" row on the next frame.
    commands_focus: bool,
    /// The always-present empty row at the end of the saved-commands list: text and auto-run.
    new_command: String,
    new_command_auto: bool,
    pub rename_pane: Option<(u64, String)>,
    pub rename_tab: Option<(usize, String)>,
    pub find: Option<Find>,
    pub import: Option<ImportDlg>,
    pub about: bool,
    pub confirm_quit: bool,
    confirm_delete: Option<String>,
}

impl Dialogs {
    pub fn open_prefs(&mut self, s: &Settings) {
        let keys = s.keybindings.iter().map(|(k, v)| (k.clone(), v.join(", "))).collect();
        self.prefs = Some(Prefs { draft: s.clone(), page: PrefsPage::General, keys,
            applied_theme: s.theme.clone(),
            message: None,
            test_result: Default::default(),
            test_message: None,
        });
    }

    pub fn open_prefs_page(&mut self, s: &Settings, page: PrefsPage) {
        self.open_prefs(s);
        if let Some(p) = &mut self.prefs {
            p.page = page;
        }
    }

    pub fn open_goto(&mut self) {
        self.history = None;
        self.goto = Some(Picker { focus: true, names: session::list_layouts(), ..Default::default() });
    }

    pub fn open_history(&mut self) {
        self.goto = None;
        self.history = Some(Picker { focus: true, ..Default::default() });
    }

    pub fn open_searches(&mut self, pane: u64) {
        if self.searches != Some(pane) {
            self.new_search.clear();
            self.new_search_regex = false;
            self.new_search_alert = false;
        }
        self.searches = Some(pane);
        self.searches_focus = true;
    }

    pub fn open_layouts(&mut self, at_startup: bool) {
        let items = session::summaries();
        let selected = items.first().map(|i| i.name.clone());
        self.layouts = Some(LayoutsDlg {
            filter: String::new(),
            items,
            selected,
            replace: at_startup,
            focus: true,
            at_startup,
            rename: None,
            error: None,
        });
    }

    fn refresh_layouts(&mut self) {
        if let Some(l) = &mut self.layouts {
            l.items = session::summaries();
            if l.selected.as_ref().is_none_or(|s| !l.items.iter().any(|i| &i.name == s)) {
                l.selected = l.items.first().map(|i| i.name.clone());
            }
        }
    }

    pub fn open_import(&mut self, path: std::path::PathBuf, bundle: crate::transfer::Bundle) {
        let existing = session::list_layouts();
        let clashes = bundle.layouts.iter().map(|l| l.name.clone()).filter(|n| existing.contains(n)).collect();
        let use_settings = bundle.settings.is_some();
        let use_layouts = !bundle.layouts.is_empty();
        self.import = Some(ImportDlg { path, bundle, use_settings, use_layouts, replace: true, clashes });
    }

    pub fn open_commands(&mut self, pane: u64) {
        if self.commands != Some(pane) {
            self.new_command.clear();
            self.new_command_auto = false;
        }
        self.commands = Some(pane);
        self.commands_focus = true;
    }

    pub fn open_save_layout(&mut self, name: String) {
        self.save_layout = Some(SaveLayout { name, only_tab: true, focus: true, existing: session::list_layouts() });
    }

    pub fn open_find(&mut self, pane: u64) {
        match &mut self.find {
            Some(f) if f.pane == pane => f.focus = true,
            _ => self.find = Some(Find { pane, query: String::new(), regex: false, last: None, status: String::new(), focus: true }),
        }
    }

    #[cfg(any(test, debug_assertions))]
    pub fn pick_theme(&mut self, name: &str) {
        if let (Some(p), Some(t)) = (&mut self.prefs, Theme::presets().into_iter().find(|t| t.name == name)) {
            p.page = PrefsPage::Colours;
            p.draft.theme = t;
        }
    }
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgb(f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

fn colour(ui: &mut egui::Ui, label: &str, c: &mut Rgb) {
    ui.label(label);
    let mut a = [c.0, c.1, c.2];
    if ui.color_edit_button_srgb(&mut a).changed() {
        *c = Rgb(a[0], a[1], a[2]);
    }
}

impl App {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        self.prefs_window(ctx);
        self.layouts_window(ctx);
        self.save_layout_window(ctx);
        self.commands_window(ctx);
        self.searches_window(ctx);
        self.goto_window(ctx);
        self.history_window(ctx);
        self.rename_windows(ctx);
        self.find_bar(ctx);
        self.import_window(ctx);
        self.misc_windows(ctx);
    }

    fn import_window(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.dlg.import else { return };
        let accent = self.s.theme.accent;
        let on_accent = if accent.luminance() > 0.55 { Color32::BLACK } else { Color32::WHITE };
        let mut open = true;
        let mut go = false;
        let mut cancel = false;
        egui::Window::new("Import settings & layouts")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(460.0)
            .max_width(460.0)
            .anchor(Align2::CENTER_CENTER, [0.0, -40.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.label(RichText::new(d.path.display().to_string()).weak().small());
                ui.add_space(8.0);
                ui.add_enabled(
                    d.bundle.settings.is_some(),
                    egui::Checkbox::new(&mut d.use_settings, "Settings: colours, fonts, keybindings and behaviour"),
                )
                .on_hover_text("Replaces your current settings");
                let n = d.bundle.layouts.len();
                let label = if n == 1 { "1 layout".to_string() } else { format!("{n} layouts") };
                ui.add_enabled(n > 0, egui::Checkbox::new(&mut d.use_layouts, format!("Layouts: {label}")));
                if n > 0 {
                    let names: Vec<&str> = d.bundle.layouts.iter().map(|l| l.name.as_str()).collect();
                    ui.indent("layout_names", |ui| ui.label(RichText::new(names.join(", ")).weak()));
                }
                if d.use_layouts && !d.clashes.is_empty() {
                    ui.add_space(6.0);
                    ui.label(format!("Already saved here: {}", d.clashes.join(", ")));
                    ui.radio_value(&mut d.replace, true, "Replace them with the imported ones");
                    ui.radio_value(&mut d.replace, false, "Keep mine, skip those");
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let btn = egui::Button::new(RichText::new("Import").color(on_accent)).fill(accent.color()).min_size(vec2(72.0, 26.0));
                    if ui.add_enabled(d.use_settings || d.use_layouts, btn).clicked() {
                        go = true;
                    }
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if go {
            let d = self.dlg.import.take().unwrap();
            let mut done = Vec::new();
            if d.use_settings
                && let Some(s) = d.bundle.settings.clone()
            {
                // Keep anything Preferences was editing from overwriting the import later.
                self.dlg.prefs = None;
                self.apply_settings(s);
                self.s.save();
                done.push("settings".to_string());
            }
            if d.use_layouts {
                match crate::transfer::import_layouts(&d.bundle, d.replace) {
                    Ok((saved, skipped)) => {
                        let mut t = if saved == 1 { "1 layout".to_string() } else { format!("{saved} layouts") };
                        if skipped > 0 {
                            t.push_str(&format!(" ({skipped} skipped)"));
                        }
                        done.push(t);
                    }
                    Err(e) => self.error = Some(format!("Could not import the layouts: {e}")),
                }
                self.dlg.refresh_layouts();
            }
            if !done.is_empty() {
                self.notice = Some(format!("Imported {}.", done.join(" and ")));
            }
        } else if cancel || !open {
            self.dlg.import = None;
        }
    }

    fn prefs_window(&mut self, ctx: &egui::Context) {
        // Whether the focused terminal's shell is marking its prompts, for the Workflow page.
        let integrated = self.focused().and_then(|id| self.panes.get(&id)).is_some_and(|p| p.shared.marks.lock().unwrap().seen);
        let Some(prefs) = &mut self.dlg.prefs else { return };
        let mut open = true;
        let mut apply = false;
        let mut save = false;
        let mut cancel = false;
        // A preset under the mouse in the dropdown, previewed for this frame only.
        let mut hovered: Option<Theme> = None;
        egui::Window::new("Preferences").open(&mut open).collapsible(false).default_size([620.0, 520.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (p, label) in [
                    (PrefsPage::General, "General"),
                    (PrefsPage::Appearance, "Appearance"),
                    (PrefsPage::Colours, "Colours"),
                    (PrefsPage::Workflow, "Workflow"),
                    (PrefsPage::Keys, "Keybindings"),
                ] {
                    ui.selectable_value(&mut prefs.page, p, label);
                }
            });
            ui.separator();
            let d = &mut prefs.draft;
            egui::ScrollArea::vertical().max_height(400.0).auto_shrink([false, true]).show(ui, |ui| match prefs.page {
                PrefsPage::General => {
                    egui::Grid::new("general").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label("Shell");
                        ui.add(egui::TextEdit::singleline(&mut d.shell).hint_text("login shell").desired_width(300.0));
                        ui.end_row();
                        ui.label("Shell arguments");
                        let mut args = d.shell_args.join(" ");
                        if ui.add(egui::TextEdit::singleline(&mut args).desired_width(300.0)).changed() {
                            d.shell_args = args.split_whitespace().map(String::from).collect();
                        }
                        ui.end_row();
                        ui.label("Scrollback lines");
                        ui.add(egui::DragValue::new(&mut d.scrollback_lines).range(100..=1_000_000).speed(100));
                        ui.end_row();
                        ui.label("Word separators");
                        ui.add(egui::TextEdit::singleline(&mut d.word_separators).desired_width(300.0))
                            .on_hover_text("Characters that end a double-click selection");
                        ui.end_row();
                        ui.label("At startup");
                        ui.vertical(|ui| {
                            ui.checkbox(&mut d.restore_session, "Restore the last session's tabs, splits and folders");
                            ui.add_enabled(
                                d.restore_session,
                                egui::Checkbox::new(&mut d.run_commands_on_restore, "…and run their auto-run commands"),
                            );
                            ui.horizontal(|ui| {
                                ui.label("Or open layout:");
                                let names = session::list_layouts();
                                let shown = if d.startup_layout.is_empty() { "(none)".to_string() } else { d.startup_layout.clone() };
                                egui::ComboBox::from_id_salt("startup_layout").selected_text(shown).show_ui(ui, |ui| {
                                    ui.selectable_value(&mut d.startup_layout, String::new(), "(none)");
                                    for n in names {
                                        ui.selectable_value(&mut d.startup_layout, n.clone(), n);
                                    }
                                });
                            });
                        });
                        ui.end_row();
                    });
                    ui.add_space(8.0);
                    ui.heading("Selection & clipboard");
                    ui.checkbox(&mut d.lock_on_select, "Freeze the view while selecting (output keeps arriving below)")
                        .on_hover_text("Makes it possible to select and copy from a fast-moving log, e.g. a server's console.");
                    ui.checkbox(&mut d.copy_on_select, "Copy selected text automatically when the mouse button is released");
                    ui.checkbox(&mut d.middle_click_paste, "Middle-click pastes");
                    if !cfg!(target_os = "macos") {
                        ui.checkbox(&mut d.ctrl_without_shift, "Shortcuts work without Shift (Ctrl+C, Ctrl+V, Ctrl+T, Ctrl+W …)").on_hover_text(
                            "Every Ctrl+Shift+<key> shortcut also works as Ctrl+<key>, unless that key is bound to something else. \
                             Ctrl+C copies only while text is selected, otherwise it interrupts the program as usual. \
                             The program no longer receives these Ctrl keys (e.g. readline's Ctrl+W, Ctrl+R, Ctrl+L).",
                        );
                    }
                    ui.checkbox(&mut d.right_click_menu, "Right-click opens a menu (otherwise it pastes)");
                    ui.add_space(8.0);
                    ui.heading("Behaviour");
                    ui.checkbox(&mut d.scroll_on_keystroke, "Scroll to the bottom when typing");
                    if cfg!(target_os = "macos") {
                        ui.checkbox(&mut d.option_as_meta, "Option key acts as Meta (Alt)");
                    }
                    ui.horizontal(|ui| {
                        ui.label("Ask before quitting");
                        let label = |q: QuitConfirm| match q {
                            QuitConfirm::Always => "Always",
                            QuitConfirm::WhenRunning => "Only while a program is running",
                            QuitConfirm::Never => "Never",
                        };
                        egui::ComboBox::from_id_salt("quit_confirm").selected_text(label(d.quit_confirm)).show_ui(ui, |ui| {
                            for q in [QuitConfirm::Always, QuitConfirm::WhenRunning, QuitConfirm::Never] {
                                ui.selectable_value(&mut d.quit_confirm, q, label(q));
                            }
                        });
                    });
                }
                PrefsPage::Appearance => {
                    egui::Grid::new("appearance").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label("Font");
                        ui.vertical(|ui| {
                            let shown = known_fonts()
                                .into_iter()
                                .find(|(_, p)| *p == d.font_path)
                                .map(|(n, _)| n.to_string())
                                .unwrap_or_else(|| if d.font_path.is_empty() { "Hack (built in)".into() } else { "Custom".into() });
                            egui::ComboBox::from_id_salt("font").selected_text(shown).show_ui(ui, |ui| {
                                ui.selectable_value(&mut d.font_path, String::new(), "Hack (built in)");
                                for (n, p) in known_fonts() {
                                    ui.selectable_value(&mut d.font_path, p.to_string(), n);
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut d.font_path).hint_text("path to .ttf / .otf").desired_width(240.0));
                                if ui.button("Browse…").clicked()
                                    && let Some(p) = rfd::FileDialog::new().add_filter("Fonts", &["ttf", "otf", "ttc"]).pick_file()
                                {
                                    d.font_path = p.display().to_string();
                                }
                            });
                        });
                        ui.end_row();
                        ui.label("Font size");
                        ui.add(egui::DragValue::new(&mut d.font_size).range(6.0..=72.0).speed(0.25).suffix(" pt"));
                        ui.end_row();
                        ui.label("Line height");
                        ui.add(egui::Slider::new(&mut d.line_height, 0.8..=2.0));
                        ui.end_row();
                        ui.label("Padding");
                        ui.add(egui::Slider::new(&mut d.padding, 0.0..=20.0));
                        ui.end_row();
                        ui.label("Divider width");
                        ui.add(egui::Slider::new(&mut d.divider_width, 1.0..=12.0));
                        ui.end_row();
                        ui.label("Dim unfocused terminals");
                        ui.add(egui::Slider::new(&mut d.inactive_dim, 0.0..=0.6));
                        ui.end_row();
                        ui.label("Cursor");
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut d.cursor_shape, CursorKind::Block, "Block");
                            ui.selectable_value(&mut d.cursor_shape, CursorKind::Beam, "Beam");
                            ui.selectable_value(&mut d.cursor_shape, CursorKind::Underline, "Underline");
                            ui.checkbox(&mut d.cursor_blink, "Blink");
                        });
                        ui.end_row();
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Tab titles show");
                        let label = |t: TabTitle| match t {
                            TabTitle::Terminal => "Terminal title",
                            TabTitle::Layout => "Layout name",
                            TabTitle::LayoutAndTerminal => "Layout name · terminal title",
                        };
                        egui::ComboBox::from_id_salt("tab_title").selected_text(label(d.tab_title)).show_ui(ui, |ui| {
                            for t in [TabTitle::Terminal, TabTitle::Layout, TabTitle::LayoutAndTerminal] {
                                ui.selectable_value(&mut d.tab_title, t, label(t));
                            }
                        });
                    })
                    .response
                    .on_hover_text("For tabs opened from (or saved as) a layout. A tab you renamed keeps its name.");
                    ui.checkbox(&mut d.show_titlebars, "Show a title bar on each terminal");
                    ui.checkbox(&mut d.always_show_tabs, "Always show the tab bar");
                    ui.checkbox(&mut d.bold_is_bright, "Show bold text in bright colours");
                    ui.checkbox(&mut d.faux_bold, "Draw bold text heavier");
                }
                PrefsPage::Colours => {
                    ui.horizontal(|ui| {
                        ui.label("Preset");
                        egui::ComboBox::from_id_salt("theme").selected_text(d.theme.name.clone()).show_ui(ui, |ui| {
                            for t in Theme::presets() {
                                let name = t.name.clone();
                                let r = ui.selectable_label(d.theme.name == name, &name);
                                if r.hovered() {
                                    hovered = Some(t.clone());
                                }
                                if r.clicked() {
                                    d.theme = t;
                                }
                            }
                        });
                    });
                    ui.add_space(6.0);
                    let t = &mut d.theme;
                    egui::Grid::new("colours").num_columns(4).spacing([12.0, 8.0]).show(ui, |ui| {
                        colour(ui, "Text", &mut t.foreground);
                        colour(ui, "Background", &mut t.background);
                        ui.end_row();
                        colour(ui, "Cursor", &mut t.cursor);
                        colour(ui, "Text under cursor", &mut t.cursor_text);
                        ui.end_row();
                        colour(ui, "Selection", &mut t.selection_bg);
                        colour(ui, "Selected text", &mut t.selection_fg);
                        ui.end_row();
                        colour(ui, "Accent", &mut t.accent);
                        ui.end_row();
                    });
                    ui.add_space(8.0);
                    ui.label("Palette");
                    let names = ["Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White"];
                    egui::Grid::new("palette").num_columns(4).spacing([12.0, 6.0]).show(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            colour(ui, n, &mut t.palette[i]);
                            colour(ui, &format!("Bright {}", n.to_lowercase()), &mut t.palette[i + 8]);
                            ui.end_row();
                        }
                    });
                }
                PrefsPage::Workflow => {
                    let weak = |ui: &mut egui::Ui, t: &str| {
                        ui.label(RichText::new(t).weak().size(12.0));
                    };
                    ui.heading("Alerts");
                    ui.checkbox(&mut d.desktop_notifications, "Desktop notifications (and taskbar attention) when the window is in the background");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut d.notify_long_commands, "Alert when a command that ran for at least");
                        ui.add(egui::DragValue::new(&mut d.long_command_secs).range(1..=3600).suffix(" s"));
                        ui.label("ends");
                    });
                    ui.checkbox(&mut d.notify_when_focused, "Alert even while I am looking at that terminal");
                    let ctx = ui.ctx().clone();
                    ui.horizontal(|ui| {
                        if ui.button("Send a test notification").on_hover_text("Sent after 5 seconds, so you can switch to another window first").clicked() {
                            let slot = prefs.test_result.clone();
                            let repaint = ctx.clone();
                            prefs.test_message = Some("Sending in 5 seconds: switch to another window now (desktops hide notifications from the window you are using)…".into());
                            notify::test(std::time::Duration::from_secs(5), move |text| {
                                *slot.lock().unwrap() = Some(text);
                                repaint.request_repaint();
                            });
                        }
                    });
                    ui.checkbox(&mut d.alert_sound, "Play a ping sound when an alert is raised");
                    ui.add_enabled_ui(d.alert_sound, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Sound");
                            ui.add(egui::TextEdit::singleline(&mut d.alert_sound_file).hint_text("the system's ping").desired_width(240.0));
                            if ui.button("Browse…").clicked()
                                && let Some(p) = rfd::FileDialog::new().add_filter("Sounds", &["wav", "ogg", "oga", "mp3", "flac", "aiff"]).pick_file()
                            {
                                d.alert_sound_file = p.display().to_string();
                            }
                            if ui.button("▶ Play").clicked() {
                                let slot = prefs.test_result.clone();
                                let repaint = ctx.clone();
                                crate::sound::test(&d.alert_sound_file, move |text| {
                                    *slot.lock().unwrap() = Some(text);
                                    repaint.request_repaint();
                                });
                            }
                        });
                    });
                    if let Some(text) = prefs.test_result.lock().unwrap().take() {
                        prefs.test_message = Some(text);
                    }
                    if let Some(m) = &prefs.test_message {
                        ui.label(RichText::new(m).size(12.0));
                    }
                    weak(ui, "Also: watch text with a terminal's 🔍 saved searches, or have a script ask with  printf '\\e]9;Build done\\a'  (works over ssh too).");

                    ui.add_space(10.0);
                    ui.heading("Links");
                    ui.checkbox(&mut d.clickable_links, "Ctrl+click opens URLs and file names in the output (⌘+click on macOS)");
                    ui.horizontal(|ui| {
                        ui.label("Open files with");
                        let current = EDITOR_PRESETS.iter().find(|(_, c)| *c == d.editor_command).map(|(n, _)| *n).unwrap_or("Custom");
                        egui::ComboBox::from_id_salt("editor").selected_text(current).show_ui(ui, |ui| {
                            for (n, c) in EDITOR_PRESETS {
                                ui.selectable_value(&mut d.editor_command, c.to_string(), *n);
                            }
                        });
                    });
                    ui.add(egui::TextEdit::singleline(&mut d.editor_command).hint_text("system default").desired_width(420.0).font(egui::TextStyle::Monospace));
                    weak(ui, "{file}, {line} and {col} are filled in, e.g.  code -g {file}:{line}:{col}");

                    ui.add_space(10.0);
                    ui.heading("Shell integration");
                    weak(
                        ui,
                        "Makes the shell mark each prompt and command. That gives you jump-to-prompt, select or copy a command's output, exit status and timing, the command history, and finish alerts that work on every OS.",
                    );
                    ui.horizontal(|ui| {
                        if integrated {
                            ui.colored_label(Color32::from_rgb(80, 190, 110), "● Active in the current terminal");
                        } else {
                            ui.label(RichText::new("○ Not active in the current terminal").weak());
                        }
                    });
                    let detected = Shell::detect();
                    for shell in Shell::ALL {
                        ui.horizontal(|ui| {
                            let mark = if shell.installed() { "✓ installed" } else { "" };
                            let label = if detected == Some(shell) { format!("{} (your shell)", shell.name()) } else { shell.name().to_string() };
                            ui.add_sized([150.0, 20.0], egui::Label::new(label));
                            if ui.button(if shell.installed() { "Reinstall" } else { "Install" }).clicked() {
                                prefs.message = Some(match shell.install() {
                                    Ok(rc) => format!("Added to {}. Open a new terminal to use it.", rc.display()),
                                    Err(e) => e,
                                });
                            }
                            if ui.button("Copy line").on_hover_text("Copy what Install adds to the shell's startup file").clicked() {
                                ui.ctx().copy_text(shell.snippet());
                            }
                            ui.label(RichText::new(mark).weak());
                        });
                    }
                    if let Some(m) = &prefs.message {
                        ui.label(m);
                    }
                    ui.checkbox(&mut d.command_marks, "Show a green / red bar beside each command's output");

                    ui.add_space(10.0);
                    ui.heading("Recording and session");
                    ui.checkbox(&mut d.record_output, "Write every terminal's output to a text file as it arrives");
                    ui.horizontal(|ui| {
                        ui.label("Into");
                        ui.add(egui::TextEdit::singleline(&mut d.record_dir).hint_text("logs folder in the settings folder").desired_width(300.0));
                        if ui.button("Browse…").clicked()
                            && let Some(p) = rfd::FileDialog::new().pick_folder()
                        {
                            d.record_dir = p.display().to_string();
                        }
                        if ui.button("Open").clicked() {
                            open_path(&crate::record::record_dir(d));
                        }
                    });
                    weak(ui, "One terminal only: right-click → Record to a file.");
                    ui.checkbox(&mut d.persist_scrollback, "Keep each terminal's recent output across restarts and crashes");
                    ui.add_enabled_ui(d.persist_scrollback, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Keep the last");
                            ui.add(egui::DragValue::new(&mut d.persist_lines).range(100..=100_000).speed(50));
                            ui.label("lines of each");
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label("Save the session every");
                        ui.add(egui::DragValue::new(&mut d.autosave_secs).range(0..=3600).suffix(" s"));
                        weak(ui, "(0 = only when quitting)");
                    });

                    ui.add_space(10.0);
                    ui.heading("Remote sessions");
                    ui.checkbox(&mut d.osc52_read, "Let programs read my clipboard (OSC 52)")
                        .on_hover_text("Programs can always put text on your clipboard, which makes copying from remote editors work. Reading it back is off by default, because a remote host could then see what you copied.");
                }
                PrefsPage::Keys => {
                    ui.label("Separate several shortcuts with commas, e.g. \"Ctrl+Shift+O, Cmd+D\". Modifiers: Ctrl, Shift, Alt, Cmd.");
                    ui.add_space(6.0);
                    egui::Grid::new("keys").num_columns(2).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                        for (id, label) in ACTIONS {
                            ui.label(*label);
                            let text = prefs.keys.entry(id.to_string()).or_default();
                            let bad = text.split(',').map(str::trim).any(|k| !k.is_empty() && Combo::parse(k).is_none());
                            let mut edit = egui::TextEdit::singleline(text).desired_width(260.0);
                            if bad {
                                edit = edit.text_color(Color32::from_rgb(230, 80, 60));
                            }
                            ui.add(edit);
                            ui.end_row();
                        }
                    });
                    if ui.button("Reset all to defaults").clicked() {
                        let mut s = Settings::default();
                        s.reset_keybindings();
                        prefs.keys = s.keybindings.iter().map(|(k, v)| (k.clone(), v.join(", "))).collect();
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    save = true;
                }
                if ui.button("Apply").clicked() {
                    apply = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Import…").on_hover_text("Load settings and layouts from an exported file").clicked() {
                        self.acts.push(Act::Action("import"));
                    }
                    if ui.button("Export…").on_hover_text("Save all your settings and layouts to one file").clicked() {
                        self.acts.push(Act::Action("export"));
                    }
                });
            });
            ui.label(RichText::new(format!("Stored in {}", Settings::path().display())).weak().small());
        });
        // Live preview: the whole app shows the hovered preset, or the colours being edited.
        let preview = hovered.unwrap_or_else(|| prefs.draft.theme.clone());
        let applied_theme = prefs.applied_theme.clone();
        if preview != self.s.theme {
            self.set_theme(preview);
        }
        if apply || save {
            let Some(prefs) = &mut self.dlg.prefs else { return };
            prefs.applied_theme = prefs.draft.theme.clone();
            let mut new = prefs.draft.clone();
            new.keybindings = prefs
                .keys
                .iter()
                .map(|(k, v)| (k.clone(), v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()))
                .collect();
            prefs.draft = new.clone();
            self.apply_settings(new);
            if save {
                self.s.save();
                self.dlg.prefs = None;
            }
        }
        if cancel || !open {
            if self.s.theme != applied_theme {
                self.set_theme(applied_theme);
            }
            self.dlg.prefs = None;
        }
    }

    fn layouts_window(&mut self, ctx: &egui::Context) {
        if self.dlg.layouts.is_some() {
            self.layouts_list(ctx);
        }
        self.layout_delete_confirm(ctx);
    }

    fn layouts_list(&mut self, ctx: &egui::Context) {
        let theme = self.s.theme.clone();
        let (bg, fg, accent) = (theme.background, theme.foreground, theme.accent);
        let card = mix(bg, fg, 0.06);
        let card_hover = mix(bg, fg, 0.10);
        let card_sel = mix(bg, accent, 0.28);
        let on_accent = if accent.luminance() > 0.55 { Color32::BLACK } else { Color32::WHITE };
        let startup_name = self.s.startup_layout.clone();
        let layout_count;
        let mut open = true;
        let mut open_it: Option<String> = None;
        let mut close = false;
        let mut delete: Option<String> = None;
        let mut toggle_startup: Option<String> = None;
        let mut commit_rename: Option<(String, String)> = None;
        let mut save_current = false;
        let mut run = self.s.launcher_run;
        let mut picker = self.s.startup_picker;
        {
            let Some(d) = &mut self.dlg.layouts else { return };
            layout_count = d.items.len();
            let title = if d.at_startup { "Open a layout" } else { "Layouts" };
            let filter = d.filter.to_lowercase();
            let shown: Vec<usize> = (0..d.items.len())
                .filter(|&i| {
                    let it = &d.items[i];
                    filter.is_empty()
                        || it.name.to_lowercase().contains(&filter)
                        || it.folders.iter().any(|f| f.to_lowercase().contains(&filter))
                })
                .collect();
            if d.selected.as_ref().is_none_or(|s| !shown.iter().any(|&i| &d.items[i].name == s)) {
                d.selected = shown.first().map(|&i| d.items[i].name.clone());
            }

            egui::Window::new(title)
                .id(egui::Id::new("layouts_window"))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .min_width(560.0)
                .max_width(560.0)
                .anchor(Align2::CENTER_CENTER, [0.0, -40.0])
                .show(ctx, |ui| {
                    if d.items.is_empty() {
                        ui.add_space(6.0);
                        ui.label("No saved layouts yet.");
                        ui.label(
                            RichText::new("Set up your tabs and splits (each terminal in its folder, with its saved commands), then save them under a name.")
                                .weak(),
                        );
                        ui.add_space(10.0);
                        if ui.add(egui::Button::new(RichText::new("Save current layout…").color(on_accent)).fill(accent.color())).clicked() {
                            save_current = true;
                        }
                        return;
                    }

                    // Filter box; arrows move the highlight, Enter opens, Esc closes.
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut d.filter)
                            .hint_text("Filter by name or folder…   ↑ ↓ to choose, Enter to open")
                            .desired_width(f32::INFINITY)
                            .margin(vec2(8.0, 6.0)),
                    );
                    if d.focus {
                        r.request_focus();
                        d.focus = false;
                    }
                    if d.rename.is_none() {
                        let (up, down, enter, esc) = ui.input(|i| {
                            (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.key_pressed(Key::Enter), i.key_pressed(Key::Escape))
                        });
                        let pos = shown.iter().position(|&i| Some(&d.items[i].name) == d.selected.as_ref());
                        if (up || down) && !shown.is_empty() {
                            let p = pos.unwrap_or(0) as i32 + if down { 1 } else { -1 };
                            d.selected = Some(d.items[shown[p.clamp(0, shown.len() as i32 - 1) as usize]].name.clone());
                            r.request_focus();
                        }
                        if enter && d.selected.is_some() {
                            open_it = d.selected.clone();
                        }
                        if esc {
                            close = true;
                        }
                    }
                    ui.add_space(8.0);

                    egui::ScrollArea::vertical().max_height(330.0).auto_shrink([false, true]).show(ui, |ui| {
                        if shown.is_empty() {
                            ui.label(RichText::new("No layout matches the filter.").weak());
                        }
                        for &i in &shown {
                            let it = &d.items[i];
                            let selected = d.selected.as_deref() == Some(it.name.as_str());
                            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 62.0), Sense::click());
                            // By pointer position: the action icons sit on top of the row, and
                            // resp.hovered() turns false over them, which would hide them again.
                            let hovered = ui.rect_contains_pointer(rect);
                            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &it.name));
                            let fill = if selected {
                                card_sel
                            } else if hovered {
                                card_hover
                            } else {
                                card
                            };
                            ui.painter().rect_filled(rect, 8.0, fill.color());
                            if selected {
                                let bar = Rect::from_min_size(rect.min + vec2(0.0, 10.0), vec2(3.0, rect.height() - 20.0));
                                ui.painter().rect_filled(bar, 2.0, accent.color());
                            }
                            let text = fg.color();
                            let weak = mix(fill, fg, 0.62).color();
                            let x = rect.min.x + 14.0;

                            // Name (or the rename field) and a startup badge.
                            let renaming = d.rename.as_ref().is_some_and(|(old, _)| *old == it.name);
                            if renaming {
                                let (_, new) = d.rename.as_mut().unwrap();
                                let field = Rect::from_min_size(pos2(x - 4.0, rect.min.y + 6.0), vec2(260.0, 24.0));
                                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field));
                                let r = child.add(egui::TextEdit::singleline(new).desired_width(field.width()).margin(vec2(4.0, 2.0)));
                                r.request_focus();
                                if ui.input(|i| i.key_pressed(Key::Enter)) {
                                    commit_rename = d.rename.clone();
                                } else if ui.input(|i| i.key_pressed(Key::Escape)) {
                                    commit_rename = Some((String::new(), String::new()));
                                }
                            } else {
                                let g = ui.painter().layout_no_wrap(it.name.clone(), egui::FontId::proportional(14.5), text);
                                let w = g.size().x;
                                ui.painter().galley(pos2(x, rect.min.y + 9.0), g, text);
                                if it.name == startup_name {
                                    let bg = ui.painter().layout_no_wrap("opens at startup".into(), egui::FontId::proportional(10.5), on_accent);
                                    let pill = Rect::from_min_size(pos2(x + w + 10.0, rect.min.y + 11.0), bg.size() + vec2(12.0, 4.0));
                                    ui.painter().rect_filled(pill, 8.0, accent.color());
                                    ui.painter().galley(pill.min + vec2(6.0, 2.0), bg, on_accent);
                                }
                            }
                            let plural = |n: usize, w: &str| if n == 1 { format!("1 {w}") } else { format!("{n} {w}s") };
                            let mut info = format!("{} · {}", plural(it.tabs, "tab"), plural(it.terminals, "terminal"));
                            if it.auto_run > 0 {
                                info.push_str(&format!(" · {} to auto-run", plural(it.auto_run, "command")));
                            }
                            ui.painter().text(pos2(x, rect.min.y + 30.0), Align2::LEFT_TOP, info, egui::FontId::proportional(11.5), weak);
                            let mut job = egui::text::LayoutJob::simple_singleline(it.folders.join("   "), egui::FontId::monospace(11.0), weak);
                            job.wrap = egui::text::TextWrapping {
                                max_width: rect.width() - 140.0,
                                max_rows: 1,
                                break_anywhere: true,
                                overflow_character: Some('…'),
                            };
                            let g = ui.painter().layout_job(job);
                            ui.painter().galley(pos2(x, rect.min.y + 45.0), g, weak);

                            // Row actions, shown on the highlighted or hovered row.
                            if (selected || hovered) && !renaming {
                                let mut bx = rect.max.x - 34.0;
                                let mut icon = |s: &str, tip: &str| {
                                    let r = Rect::from_min_size(pos2(bx, rect.center().y - 13.0), vec2(26.0, 26.0));
                                    bx -= 30.0;
                                    // A child Ui places the button without moving the list's own layout.
                                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
                                    child.add(egui::Button::new(RichText::new(s).size(13.0)).frame(false)).on_hover_text(tip).clicked()
                                };
                                if icon("🗑", "Delete") {
                                    delete = Some(it.name.clone());
                                }
                                let star = it.name == startup_name;
                                if icon(if star { "★" } else { "☆" }, if star { "Stop opening at startup" } else { "Always open at startup" }) {
                                    toggle_startup = Some(it.name.clone());
                                }
                                if icon("✏", "Rename") {
                                    d.rename = Some((it.name.clone(), it.name.clone()));
                                }
                            }
                            if resp.clicked() {
                                d.selected = Some(it.name.clone());
                            }
                            if resp.double_clicked() {
                                open_it = Some(it.name.clone());
                            }
                            ui.add_space(6.0);
                        }
                    });
                    if let Some(e) = &d.error {
                        ui.colored_label(Color32::from_rgb(230, 90, 70), e);
                    }

                    ui.add_space(4.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut run, "Run auto-run commands");
                        ui.checkbox(&mut d.replace, "Replace current tabs")
                            .on_hover_text("Off: the layout's tabs are added next to the ones already open");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let open_btn = egui::Button::new(RichText::new("Open").color(on_accent)).fill(accent.color()).min_size(vec2(72.0, 26.0));
                            if ui.add_enabled(d.selected.is_some(), open_btn).clicked() {
                                open_it = d.selected.clone();
                            }
                            if ui.button(if d.at_startup { "Not now" } else { "Close" }).clicked() {
                                close = true;
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Save current layout…").clicked() {
                            save_current = true;
                        }
                        if ui.button("Open folder").on_hover_text("Layouts are JSON files you can copy or edit").clicked() {
                            let dir = crate::config::layouts_dir();
                            let _ = std::fs::create_dir_all(&dir);
                            open_path(&dir);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.checkbox(&mut picker, "Show this list when term starts");
                        });
                    });
                });
        }

        if run != self.s.launcher_run || picker != self.s.startup_picker {
            self.s.launcher_run = run;
            self.s.startup_picker = picker;
            self.s.save();
        }
        if let Some(name) = toggle_startup {
            self.s.startup_layout = if self.s.startup_layout == name { String::new() } else { name };
            self.s.save();
        }
        if let Some((old, new)) = commit_rename {
            let new = new.trim().to_string();
            let mut error = None;
            if !old.is_empty() && !new.is_empty() && new != old {
                match session::rename_layout(&old, &new) {
                    Ok(()) => {
                        for t in self.tabs.iter_mut().filter(|t| t.layout.as_deref() == Some(old.as_str())) {
                            t.layout = Some(new.clone());
                        }
                        if self.s.startup_layout == old {
                            self.s.startup_layout = new.clone();
                            self.s.save();
                        }
                        if self.layout_name == old {
                            self.layout_name = new.clone();
                        }
                        self.dlg.refresh_layouts();
                        if let Some(d) = &mut self.dlg.layouts {
                            d.selected = Some(new);
                        }
                    }
                    Err(e) => error = Some(e),
                }
            }
            if let Some(d) = &mut self.dlg.layouts {
                d.rename = None;
                d.error = error;
                d.focus = true;
            }
        }
        if let Some(name) = delete {
            self.dlg.confirm_delete = Some(name);
        }
        if save_current {
            self.open_save_layout(self.active);
        }
        if let Some(name) = open_it {
            let replace = self.dlg.layouts.as_ref().is_some_and(|d| d.replace);
            self.acts.push(Act::OpenLayout(name, replace, self.s.launcher_run));
            close = true;
        }
        if close || !open || (layout_count == 0 && save_current) {
            self.dlg.layouts = None;
        }
    }

    fn layout_delete_confirm(&mut self, ctx: &egui::Context) {
        if let Some(name) = self.dlg.confirm_delete.clone() {
            let mut keep = true;
            egui::Window::new("Delete layout?")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.label(format!("Delete the layout \"{name}\"? This cannot be undone."));
                    ui.horizontal(|ui| {
                        if ui.button("Delete").clicked() {
                            if let Err(e) = session::delete_layout(&name) {
                                self.error = Some(format!("Could not delete the layout: {e}"));
                            }
                            if self.s.startup_layout == name {
                                self.s.startup_layout.clear();
                                self.s.save();
                            }
                            self.dlg.refresh_layouts();
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                });
            if !keep {
                self.dlg.confirm_delete = None;
            }
        }
    }

    fn save_layout_window(&mut self, ctx: &egui::Context) {
        let Some(st) = &mut self.dlg.save_layout else { return };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        let theme = self.s.theme.clone();
        let accent = theme.accent.color();
        let on_accent = if theme.accent.luminance() > 0.55 { Color32::BLACK } else { Color32::WHITE };
        let exists = st.existing.iter().any(|n| n == st.name.trim());
        egui::Window::new("Save layout")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(440.0)
            .max_width(440.0)
            .anchor(Align2::CENTER_CENTER, [0.0, -60.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut st.only_tab, true, "Only the current tab");
                    ui.radio_value(&mut st.only_tab, false, "All tabs");
                });
                ui.add_space(6.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut st.name)
                        .desired_width(f32::INFINITY)
                        .hint_text("Name of a new layout, e.g. hybris")
                        .margin(vec2(8.0, 6.0)),
                );
                if st.focus {
                    r.request_focus();
                    st.focus = false;
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    save = true;
                }
                if !st.existing.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new("Or replace an existing layout:").weak());
                    ui.horizontal_wrapped(|ui| {
                        for n in &st.existing {
                            if ui.selectable_label(st.name.trim() == n, n).clicked() {
                                st.name = n.clone();
                            }
                        }
                    });
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Saves the splits, each terminal's current folder, its name and its saved commands.").weak());
                if exists {
                    ui.colored_label(Color32::from_rgb(220, 160, 40), format!("\u{201c}{}\u{201d} will be replaced.", st.name.trim()));
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let label = if exists { "Replace" } else { "Save" };
                    let btn = egui::Button::new(RichText::new(label).color(on_accent)).fill(accent).min_size(vec2(72.0, 26.0));
                    if ui.add_enabled(!st.name.trim().is_empty(), btn).clicked() {
                        save = true;
                    }
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if save && !st.name.trim().is_empty() {
            let name = st.name.trim().to_string();
            let only = st.only_tab.then_some(self.active);
            // The saved tabs now belong to this layout (shown in their titles).
            for (i, t) in self.tabs.iter_mut().enumerate() {
                if only.is_none_or(|o| o == i) {
                    t.layout = Some(name.clone());
                }
            }
            let snap = self.snapshot(&name, only, false);
            match session::save_layout(&snap) {
                Ok(()) => {
                    self.layout_name = name;
                    self.dlg.refresh_layouts();
                }
                Err(e) => self.error = Some(format!("Could not save the layout: {e}")),
            }
            self.dlg.save_layout = None;
        } else if !open || cancel {
            self.dlg.save_layout = None;
        }
    }

    fn commands_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.dlg.commands else { return };
        let theme = self.s.theme.clone();
        let Some(p) = self.panes.get_mut(&id) else {
            self.dlg.commands = None;
            return;
        };
        let mut open = true;
        let title = p.title();
        let mut run: Option<String> = None;
        let card = mix(theme.background, theme.foreground, 0.07).color();
        let accent = theme.accent.color();
        let focus_new = std::mem::take(&mut self.dlg.commands_focus);
        egui::Window::new(format!("Saved commands — {title}"))
            .id(egui::Id::new("commands_window"))
            .open(&mut open)
            .collapsible(false)
            .default_width(680.0)
            .min_width(520.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Commands kept with this terminal, saved with layouts and the session.").weak());
                ui.label(RichText::new("Tick Auto-run to have a command typed in automatically when the layout opens.").weak());
                ui.add_space(10.0);

                let mut remove = None;
                let mut swap = None;
                let mut focus_draft = false;
                // Where the rows' text boxes sit, so the empty row's box lines up with them.
                let mut edit_span: Option<(f32, f32)> = None;
                let n = p.commands.len();
                for (i, c) in p.commands.iter_mut().enumerate() {
                    egui::Frame::new().fill(card).corner_radius(8).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let play = egui::Button::new(RichText::new("▶").color(accent).size(13.0)).min_size(vec2(30.0, 28.0));
                            if ui.add(play).on_hover_text("Run now").clicked() {
                                run = Some(c.command.clone());
                            }
                            let ins = egui::Button::new(RichText::new("Insert").size(11.5)).min_size(vec2(0.0, 28.0));
                            if ui.add(ins).on_hover_text("Type it into the terminal without pressing Enter").clicked() {
                                self.acts.push(Act::Insert(id, c.command.clone()));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let icon = |ui: &mut egui::Ui, s: &str, enabled: bool, tip: &str| {
                                    ui.add_enabled(
                                        enabled,
                                        egui::Button::new(RichText::new(s).size(13.0)).frame(false).min_size(vec2(22.0, 26.0)),
                                    )
                                    .on_hover_text(tip)
                                    .clicked()
                                };
                                if icon(ui, "🗑", true, "Remove") {
                                    remove = Some(i);
                                }
                                if icon(ui, "↓", i + 1 < n, "Move down") {
                                    swap = Some((i, i + 1));
                                }
                                if icon(ui, "↑", i > 0, "Move up") {
                                    swap = Some((i, i - 1));
                                }
                                ui.add_space(4.0);
                                ui.checkbox(&mut c.reconnect, "Keep alive").on_hover_text(
                                    "If this command ends with an error (an ssh connection that dropped, say), run it again automatically, waiting a little longer each time.",
                                );
                                ui.checkbox(&mut c.auto_run, "Auto-run");
                                ui.add_space(4.0);
                                let edit =
                                    egui::TextEdit::singleline(&mut c.command).font(egui::TextStyle::Monospace).margin(vec2(8.0, 5.0));
                                let r = ui.add_sized([ui.available_width(), 28.0], edit);
                                edit_span.get_or_insert((r.rect.min.x, r.rect.max.x));
                                if r.lost_focus() && ui.input(|inp| inp.key_pressed(Key::Enter)) {
                                    if ui.input(|inp| inp.modifiers.command) {
                                        run = Some(c.command.clone());
                                    } else {
                                        // Enter moves on to the empty row, ready for the next command.
                                        focus_draft = true;
                                    }
                                }
                            });
                        });
                    });
                    ui.add_space(4.0);
                }

                // The empty row: type a command and press Enter (or click away) to keep it.
                let draft_fill = mix(theme.background, theme.foreground, 0.04).color();
                let mut commit = false;
                egui::Frame::new()
                    .fill(draft_fill)
                    .stroke(egui::Stroke::new(1.0, mix(theme.background, theme.foreground, 0.12).color()))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.add_sized([30.0, 28.0], egui::Label::new(RichText::new("+").size(15.0).color(accent)));
                            let left = ui.cursor().min.x;
                            let (x0, x1) = edit_span.unwrap_or((left, left + (ui.available_width() - 110.0).max(120.0)));
                            ui.add_space((x0 - left).max(0.0));
                            let edit = egui::TextEdit::singleline(&mut self.dlg.new_command)
                                .font(egui::TextStyle::Monospace)
                                .hint_text("New command: type it and press Enter")
                                .margin(vec2(8.0, 5.0));
                            let r = ui.add_sized([x1 - x0, 28.0], edit);
                            if focus_new || focus_draft {
                                r.request_focus();
                            }
                            if r.lost_focus() && !self.dlg.new_command.trim().is_empty() {
                                commit = true;
                                // After Enter, stay in the (new) empty row for the next one.
                                if ui.input(|inp| inp.key_pressed(Key::Enter)) {
                                    self.dlg.commands_focus = true;
                                }
                            }
                            ui.add_space(4.0);
                            ui.checkbox(&mut self.dlg.new_command_auto, "Auto-run");
                        });
                    });
                if commit {
                    p.commands.push(SavedCommand {
                        command: self.dlg.new_command.trim().to_string(),
                        auto_run: self.dlg.new_command_auto,
                        ..Default::default()
                    });
                    self.dlg.new_command.clear();
                    self.dlg.new_command_auto = false;
                }
                if let Some((a, b)) = swap {
                    p.commands.swap(a, b);
                }
                if let Some(i) = remove {
                    p.commands.remove(i);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let sel = p.term.lock().selection_to_string().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                    if ui
                        .add_enabled(sel.is_some(), egui::Button::new("Add selected text"))
                        .on_disabled_hover_text("Select some text in the terminal first")
                        .clicked()
                        && let Some(s) = sel
                    {
                        p.commands.push(SavedCommand { command: s, ..Default::default() });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let autos: Vec<String> =
                            p.commands.iter().filter(|c| c.auto_run && !c.command.trim().is_empty()).map(|c| c.command.clone()).collect();
                        let label = if autos.is_empty() {
                            "Run auto-run commands".to_string()
                        } else {
                            format!("Run auto-run commands ({})", autos.len())
                        };
                        if ui.add_enabled(!autos.is_empty(), egui::Button::new(label)).clicked() {
                            run = Some(autos.join("\n"));
                        }
                    });
                });
            });
        if let Some(c) = run {
            for line in c.lines().filter(|l| !l.trim().is_empty()) {
                self.acts.push(Act::Run(id, line.to_string()));
            }
        }
        if !open {
            if let Some(p) = self.panes.get_mut(&id) {
                // Keep a command typed into the empty row but never confirmed.
                let pending = self.dlg.new_command.trim();
                if !pending.is_empty() {
                    p.commands.push(SavedCommand { command: pending.to_string(), auto_run: self.dlg.new_command_auto, ..Default::default() });
                }
                p.commands.retain(|c| !c.command.trim().is_empty());
            }
            self.dlg.new_command.clear();
            self.dlg.new_command_auto = false;
            self.dlg.commands = None;
        }
    }


    /// The query box and keys of a picker. Returns (selection moved, Enter pressed, Escape pressed).
    fn picker_header(ui: &mut egui::Ui, pk: &mut Picker, hint: &str, count: usize) -> (bool, bool, bool) {
        let r = ui.add(egui::TextEdit::singleline(&mut pk.query).hint_text(hint).desired_width(f32::INFINITY).margin(vec2(8.0, 6.0)));
        if pk.focus {
            r.request_focus();
            pk.focus = false;
        }
        if r.changed() {
            pk.selected = 0;
        }
        let (down, up, enter, esc) =
            ui.input(|i| (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        let before = pk.selected;
        if down {
            pk.selected += 1;
        }
        if up {
            pk.selected = pk.selected.saturating_sub(1);
        }
        pk.selected = pk.selected.min(count.saturating_sub(1));
        (pk.selected != before, enter, esc)
    }

    /// "Go to…": jump to any terminal, open a saved layout, or run one of this terminal's saved commands.
    fn goto_window(&mut self, ctx: &egui::Context) {
        let Some(mut pk) = self.dlg.goto.take() else { return };
        enum Item {
            Pane(u64),
            Layout(String),
            Command(u64, String),
        }
        let home = dirs::home_dir().map(|h| h.display().to_string()).unwrap_or_default();
        let tilde = |p: &std::path::Path| {
            let d = p.display().to_string();
            if !home.is_empty() && d.starts_with(&home) { format!("~{}", &d[home.len()..]) } else { d }
        };
        let mut rows: Vec<(String, String, Item)> = Vec::new();
        for (ti, tab) in self.tabs.iter().enumerate() {
            for id in tab.root.leaves() {
                let Some(p) = self.panes.get(&id) else { continue };
                let mut label = p.title();
                if p.alert {
                    label = format!("🔔 {label}");
                }
                let layout = tab.layout.as_ref().map(|l| format!("{l} · ")).unwrap_or_default();
                rows.push((label, format!("tab {} · {layout}{}", ti + 1, tilde(&p.cwd)), Item::Pane(id)));
            }
        }
        if let Some(id) = self.focused() {
            for c in self.panes.get(&id).map(|p| p.commands.clone()).unwrap_or_default() {
                rows.push((format!("▶ {}", c.command), "run in this terminal".into(), Item::Command(id, c.command)));
            }
        }
        for n in &pk.names {
            rows.push((format!("Open layout: {n}"), "saved layout".into(), Item::Layout(n.clone())));
        }
        let mut scored: Vec<(i32, (String, String, Item))> =
            rows.into_iter().filter_map(|r| fuzzy(&pk.query, &format!("{} {}", r.0, r.1)).map(|s| (s, r))).collect();
        scored.sort_by_key(|(s, _)| -*s);

        let mut chosen: Option<usize> = None;
        let mut close = false;
        egui::Window::new("goto_picker")
            .title_bar(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, [0.0, 70.0])
            .show(ctx, |ui| {
                ui.set_width(580.0);
                let (moved, enter, esc) = Self::picker_header(ui, &mut pk, "Go to a terminal, layout or saved command…", scored.len());
                close = esc;
                if enter && !scored.is_empty() {
                    chosen = Some(pk.selected);
                }
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                    if scored.is_empty() {
                        ui.weak("Nothing matches");
                    }
                    for (i, (_, (label, detail, _))) in scored.iter().enumerate() {
                        let b = egui::Button::new(())
                            .left_text(RichText::new(label))
                            .right_text(RichText::new(detail).weak().size(11.5))
                            .selected(i == pk.selected)
                            .min_size(vec2(ui.available_width(), 26.0))
                            .truncate();
                        let r = ui.add(b);
                        if i == pk.selected && moved {
                            r.scroll_to_me(None);
                        }
                        if r.clicked() {
                            chosen = Some(i);
                        }
                    }
                });
            });
        if let Some(i) = chosen {
            match scored.swap_remove(i).1.2 {
                Item::Pane(id) => self.acts.push(Act::Focus(id)),
                Item::Layout(n) => self.acts.push(Act::OpenLayout(n, false, true)),
                Item::Command(id, c) => self.acts.push(Act::Run(id, c)),
            }
            close = true;
        }
        if !close {
            self.dlg.goto = Some(pk);
        }
    }

    /// Every command typed in any terminal since term started, newest first.
    fn history_window(&mut self, ctx: &egui::Context) {
        let Some(mut pk) = self.dlg.history.take() else { return };
        struct Entry {
            command: String,
            pane: u64,
            exit: Option<i32>,
            ago: Option<std::time::Duration>,
            at: Option<std::time::Instant>,
        }
        let mut entries: Vec<Entry> = Vec::new();
        let mut integrated = false;
        for (id, p) in &self.panes {
            let marks = p.shared.marks.lock().unwrap();
            integrated |= marks.seen;
            for b in marks.blocks.iter().rev().take(500) {
                if !b.command.is_empty() {
                    entries.push(Entry {
                        command: b.command.clone(),
                        pane: *id,
                        exit: b.exit,
                        ago: b.started.map(|s| s.elapsed()),
                        at: b.started,
                    });
                }
            }
        }
        entries.sort_by(|a, b| b.at.cmp(&a.at));
        let mut seen = std::collections::HashSet::new();
        entries.retain(|e| seen.insert(e.command.clone()));
        let entries: Vec<Entry> = entries.into_iter().filter(|e| fuzzy(&pk.query, &e.command).is_some()).collect();

        let target = self.focused();
        let mut chosen: Option<(usize, bool)> = None;
        let mut close = false;
        let mut open_prefs = false;
        egui::Window::new("history_picker")
            .title_bar(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, [0.0, 70.0])
            .show(ctx, |ui| {
                ui.set_width(620.0);
                let (moved, enter, esc) = Self::picker_header(ui, &mut pk, "Search commands from all terminals…", entries.len());
                close = esc;
                if enter && !entries.is_empty() {
                    chosen = Some((pk.selected, ui.input(|i| i.modifiers.shift)));
                }
                ui.add_space(4.0);
                if !integrated {
                    ui.label("Command history needs shell integration, so the shell can tell term what was typed.");
                    if ui.button("Set up shell integration…").clicked() {
                        open_prefs = true;
                    }
                } else if entries.is_empty() {
                    ui.weak("No commands yet");
                }
                egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                    for (i, e) in entries.iter().enumerate() {
                        let name = self.panes.get(&e.pane).map(|p| p.title()).unwrap_or_default();
                        let mut detail = name;
                        if let Some(c) = e.exit.filter(|c| *c != 0) {
                            detail = format!("✗ {c} · {detail}");
                        }
                        if let Some(a) = e.ago {
                            detail = format!("{detail} · {} ago", format_duration(std::time::Duration::from_secs(a.as_secs())));
                        }
                        let b = egui::Button::new(())
                            .left_text(RichText::new(&e.command).monospace())
                            .right_text(RichText::new(detail).weak().size(11.5))
                            .selected(i == pk.selected)
                            .min_size(vec2(ui.available_width(), 26.0))
                            .truncate();
                        let r = ui.add(b);
                        if i == pk.selected && moved {
                            r.scroll_to_me(None);
                        }
                        if r.clicked() {
                            chosen = Some((i, ui.input(|i| i.modifiers.shift)));
                        }
                    }
                });
                ui.weak("Enter types it into the terminal · Shift+Enter runs it");
            });
        if let (Some((i, run)), Some(t)) = (chosen, target) {
            let cmd = entries[i].command.clone();
            self.acts.push(if run { Act::Run(t, cmd) } else { Act::Insert(t, cmd) });
            close = true;
        }
        if open_prefs {
            self.dlg.open_prefs_page(&self.s, PrefsPage::Workflow);
            close = true;
        }
        if !close {
            self.dlg.history = Some(pk);
        }
    }

    /// The saved searches of one terminal: text to find again, and optionally to be alerted about.
    fn searches_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.dlg.searches else { return };
        let theme = self.s.theme.clone();
        let Some(p) = self.panes.get_mut(&id) else {
            self.dlg.searches = None;
            return;
        };
        let mut open = true;
        let title = p.title();
        let card = mix(theme.background, theme.foreground, 0.07).color();
        let accent = theme.accent.color();
        let focus_new = std::mem::take(&mut self.dlg.searches_focus);
        let mut find: Option<usize> = None;
        egui::Window::new(format!("Saved searches — {title}"))
            .id(egui::Id::new("searches_window"))
            .open(&mut open)
            .collapsible(false)
            .default_width(640.0)
            .min_width(520.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Text to find again in this terminal's output. Click Find to select the latest match, again for earlier ones.").weak());
                ui.label(RichText::new("Tick Alert to be told when it shows up in new output (a notification when you are elsewhere).").weak());
                ui.add_space(10.0);
                let mut remove = None;
                let mut swap = None;
                let n = p.searches.len();
                for (i, sv) in p.searches.iter_mut().enumerate() {
                    egui::Frame::new().fill(card).corner_radius(8).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui.add(egui::Button::new(RichText::new("Find").color(accent)).min_size(vec2(44.0, 28.0))).clicked() {
                                find = Some(i);
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let icon = |ui: &mut egui::Ui, s: &str, enabled: bool, tip: &str| {
                                    ui.add_enabled(enabled, egui::Button::new(RichText::new(s).size(13.0)).frame(false).min_size(vec2(22.0, 26.0)))
                                        .on_hover_text(tip)
                                        .clicked()
                                };
                                if icon(ui, "🗑", true, "Remove") {
                                    remove = Some(i);
                                }
                                if icon(ui, "↓", i + 1 < n, "Move down") {
                                    swap = Some((i, i + 1));
                                }
                                if icon(ui, "↑", i > 0, "Move up") {
                                    swap = Some((i, i - 1));
                                }
                                ui.checkbox(&mut sv.alert, "🔔 Alert");
                                ui.checkbox(&mut sv.regex, "Regex");
                                let edit = egui::TextEdit::singleline(&mut sv.pattern).font(egui::TextStyle::Monospace).margin(vec2(8.0, 5.0));
                                ui.add_sized([ui.available_width(), 28.0], edit);
                            });
                        });
                    });
                    ui.add_space(4.0);
                }
                // The empty row: type and press Enter.
                let mut commit = false;
                egui::Frame::new()
                    .fill(mix(theme.background, theme.foreground, 0.04).color())
                    .stroke(egui::Stroke::new(1.0, mix(theme.background, theme.foreground, 0.12).color()))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.add_sized([44.0, 28.0], egui::Label::new(RichText::new("+").size(15.0).color(accent)));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.checkbox(&mut self.dlg.new_search_alert, "🔔 Alert");
                                ui.checkbox(&mut self.dlg.new_search_regex, "Regex");
                                let edit = egui::TextEdit::singleline(&mut self.dlg.new_search)
                                    .font(egui::TextStyle::Monospace)
                                    .hint_text("New search: type it and press Enter")
                                    .margin(vec2(8.0, 5.0));
                                let r = ui.add_sized([ui.available_width(), 28.0], edit);
                                if focus_new {
                                    r.request_focus();
                                }
                                if r.lost_focus() && !self.dlg.new_search.trim().is_empty() {
                                    commit = true;
                                }
                            });
                        });
                    });
                if commit {
                    p.searches.push(SavedSearch {
                        pattern: self.dlg.new_search.trim().to_string(),
                        regex: self.dlg.new_search_regex,
                        alert: self.dlg.new_search_alert,
                    });
                    self.dlg.new_search.clear();
                    self.dlg.new_search_alert = false;
                    if ui.input(|i| i.key_pressed(Key::Enter)) {
                        self.dlg.searches_focus = true;
                    }
                }
                if let Some((a, b)) = swap {
                    p.searches.swap(a, b);
                }
                if let Some(i) = remove {
                    p.searches.remove(i);
                }
                ui.add_space(8.0);
                let sel = p.term.lock().selection_to_string().map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && !s.contains('\n'));
                if ui
                    .add_enabled(sel.is_some(), egui::Button::new("Add selected text"))
                    .on_disabled_hover_text("Select some text (one line) in the terminal first")
                    .clicked()
                    && let Some(s) = sel
                {
                    p.searches.push(SavedSearch { pattern: s, ..Default::default() });
                }
            });
        if let Some(i) = find {
            self.acts.push(Act::FindSaved(id, i, false));
        }
        if !open && let Some(p) = self.panes.get_mut(&id) {
            let pending = self.dlg.new_search.trim();
            if !pending.is_empty() {
                p.searches.push(SavedSearch {
                    pattern: pending.to_string(),
                    regex: self.dlg.new_search_regex,
                    alert: self.dlg.new_search_alert,
                });
            }
            p.searches.retain(|s| !s.pattern.trim().is_empty());
            self.dlg.new_search.clear();
            self.dlg.new_search_alert = false;
            self.dlg.searches = None;
        }
    }

    fn rename_windows(&mut self, ctx: &egui::Context) {
        if let Some((id, name)) = &mut self.dlg.rename_pane {
            let mut done = false;
            let mut cancel = false;
            let mut clear = false;
            egui::Window::new("Rename terminal").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, -60.0]).show(
                ctx,
                |ui| {
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(260.0));
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        done = true;
                    }
                    ui.horizontal(|ui| {
                        done |= ui.button("OK").clicked();
                        clear = ui.button("Use automatic title").clicked();
                        cancel = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape));
                    });
                },
            );
            let (id, name) = (*id, name.trim().to_string());
            if (done || clear)
                && let Some(p) = self.panes.get_mut(&id)
            {
                p.name = if clear || name.is_empty() { None } else { Some(name) };
            }
            if done || clear || cancel {
                self.dlg.rename_pane = None;
            }
        }
        if let Some((i, name)) = &mut self.dlg.rename_tab {
            let mut done = false;
            let mut cancel = false;
            let mut clear = false;
            egui::Window::new("Rename tab").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, -60.0]).show(
                ctx,
                |ui| {
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(260.0));
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        done = true;
                    }
                    ui.horizontal(|ui| {
                        done |= ui.button("OK").clicked();
                        clear = ui.button("Use automatic title").clicked();
                        cancel = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape));
                    });
                },
            );
            let (i, name) = (*i, name.trim().to_string());
            if (done || clear)
                && let Some(t) = self.tabs.get_mut(i)
            {
                t.title = if clear || name.is_empty() { None } else { Some(name) };
            }
            if done || clear || cancel {
                self.dlg.rename_tab = None;
            }
        }
    }

    fn find_bar(&mut self, ctx: &egui::Context) {
        let Some(f) = &mut self.dlg.find else { return };
        let Some(rect) = self.rects.iter().find(|(id, _)| *id == f.pane).map(|(_, r)| *r) else {
            if !self.panes.contains_key(&f.pane) {
                self.dlg.find = None;
            }
            return;
        };
        let mut step: Option<bool> = None;
        let mut close = false;
        let mut save = false;
        let before = f.query.clone();
        egui::Area::new(egui::Id::new("find_bar"))
            .fixed_pos(egui::pos2(rect.max.x - 430.0, rect.min.y + 24.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut f.query).desired_width(180.0).hint_text("Find in scrollback"));
                        if f.focus {
                            r.request_focus();
                            f.focus = false;
                        }
                        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            step = Some(!ui.input(|i| i.modifiers.shift));
                            f.focus = true;
                        }
                        if ui.input(|i| i.key_pressed(Key::Escape)) {
                            close = true;
                        }
                        if ui.button("↑").on_hover_text("Previous (Enter)").clicked() {
                            step = Some(true);
                        }
                        if ui.button("↓").on_hover_text("Next (Shift+Enter)").clicked() {
                            step = Some(false);
                        }
                        ui.checkbox(&mut f.regex, ".*").on_hover_text("Regular expression");
                        if ui
                            .add_enabled(!f.query.is_empty(), egui::Button::new("☆").frame(false))
                            .on_hover_text("Save this search with the terminal (see 🔍 in its title bar)")
                            .clicked()
                        {
                            save = true;
                        }
                        if !f.status.is_empty() {
                            ui.weak(&f.status);
                        }
                        if ui.button("✕").clicked() {
                            close = true;
                        }
                    });
                });
            });
        if f.query != before {
            f.last = None;
            f.status.clear();
        }
        if save {
            let (id, query, regex) = (f.pane, f.query.clone(), f.regex);
            if let Some(p) = self.panes.get_mut(&id) {
                if !p.searches.iter().any(|s| s.pattern == query && s.regex == regex) {
                    p.searches.push(SavedSearch { pattern: query.clone(), regex, alert: false });
                }
                self.toast(id, "Search saved", format!("“{query}” is under 🔍 in this terminal's title bar."));
            }
        }
        if close {
            self.dlg.find = None;
            return;
        }
        if let Some(backwards) = step {
            self.find_step(backwards);
        }
    }

    fn find_step(&mut self, backwards: bool) {
        let Some(f) = &mut self.dlg.find else { return };
        let Some(p) = self.panes.get(&f.pane) else { return };
        if f.query.is_empty() {
            return;
        }
        let pattern = if f.regex { f.query.clone() } else { escape_regex(&f.query) };
        let mut t = p.term.lock();
        match search::find(&t, &pattern, f.last.as_ref(), backwards) {
            Err(()) => f.status = "Invalid pattern".into(),
            Ok(Some(m)) => {
                search::select(&mut t, &m);
                f.last = Some(m);
                f.status.clear();
                drop(t);
                p.set_hold(true);
            }
            Ok(None) => f.status = "Not found".into(),
        }
    }

    fn misc_windows(&mut self, ctx: &egui::Context) {
        if self.dlg.about {
            let mut open = true;
            egui::Window::new("About term")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.heading(format!("term {}", env!("CARGO_PKG_VERSION")));
                    ui.label("A fast split-pane terminal for macOS, Linux and Windows.");
                    ui.label(format!("Settings and layouts: {}", crate::config::config_dir().display()));
                });
            self.dlg.about = open;
        }
        if let Some(n) = self.notice.clone() {
            let mut open = true;
            let mut ok = false;
            egui::Window::new("Done")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.label(n);
                    ui.add_space(6.0);
                    ok = ui.button("OK").clicked();
                });
            if !open || ok {
                self.notice = None;
            }
        }
        if let Some(e) = self.error.clone() {
            let mut open = true;
            egui::Window::new("Problem")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(e);
                });
            if !open {
                self.error = None;
            }
        }
        if self.dlg.confirm_quit {
            let running = self.running_jobs();
            let terminals: usize = self.tabs.iter().map(|t| t.root.leaves().len()).sum();
            let accent = self.s.theme.accent;
            let on_accent = if accent.luminance() > 0.55 { Color32::BLACK } else { Color32::WHITE };
            let quit_key = self.s.shortcut_label("quit");
            let mut never = self.s.quit_confirm == QuitConfirm::Never;
            egui::Window::new("Quit term?")
                .collapsible(false)
                .resizable(false)
                .min_width(360.0)
                .anchor(Align2::CENTER_CENTER, [0.0, -40.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let what = if terminals == 1 { "the terminal".to_string() } else { format!("all {terminals} terminals") };
                    ui.label(format!("This closes {what} in this window."));
                    if running > 0 {
                        let r = if running == 1 { "1 terminal is".to_string() } else { format!("{running} terminals are") };
                        ui.colored_label(Color32::from_rgb(230, 150, 50), format!("{r} still running a program (e.g. a server)."));
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new("Your tabs, splits and folders are restored next time.").weak());
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let quit =
                            egui::Button::new(RichText::new("Quit").color(on_accent)).fill(accent.color()).min_size(vec2(72.0, 26.0));
                        if ui.add(quit).clicked() {
                            self.close_window = true;
                        }
                        if ui.button("Cancel").clicked() {
                            self.dlg.confirm_quit = false;
                        }
                        let hint = if quit_key.is_empty() {
                            "Enter quits, Esc cancels".to_string()
                        } else {
                            format!("Enter or {quit_key} quits, Esc cancels")
                        };
                        ui.label(RichText::new(hint).weak().small());
                    });
                    ui.add_space(4.0);
                    ui.checkbox(&mut never, "Don't ask again").on_hover_text("Change this later in Preferences → General");
                });
            if never != (self.s.quit_confirm == QuitConfirm::Never) {
                self.s.quit_confirm = if never { QuitConfirm::Never } else { QuitConfirm::Always };
                self.s.save();
            }
        }
    }
}

fn open_path(p: &std::path::Path) {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(cmd).arg(p).spawn();
}
