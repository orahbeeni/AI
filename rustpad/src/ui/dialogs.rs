//! Dialog windows: find / replace / find-in-files / mark, go to, column editor,
//! preferences, run, about, macro dialogs and the modal prompts.

use eframe::egui::{self, Align2, Key, RichText};

use crate::app::{App, Pending};
use crate::cmd::{self, Cmd, SearchOp};
use crate::doc::Eol;
use crate::ops::ColumnNumbers;
use crate::search::{Mode, Query};
use crate::settings::{RunEntry, SavedMacro, Settings};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FindTab {
    Find,
    Replace,
    Files,
    Mark,
}

pub struct Dialogs {
    pub find_open: bool,
    pub find_tab: FindTab,
    pub find_focus: bool,
    pub find_select: bool,
    pub find_refocus: bool,
    pub find_status: String,
    pub find_sel_range: Option<(usize, usize)>,
    pub mark_style: u8,
    pub mark_bookmark: bool,
    pub dir_history: Vec<String>,

    pub goto_open: bool,
    pub goto_text: String,
    pub goto_offset: bool,
    pub goto_focus: bool,

    pub column_open: bool,
    pub column_numbers: bool,
    pub column_text: String,
    pub column_start: String,
    pub column_step: String,
    pub column_repeat: String,
    pub column_zero: bool,
    pub column_radix: u32,

    pub macro_run_open: bool,
    pub macro_run_times: usize,
    pub macro_run_eof: bool,
    pub macro_run_which: usize,
    pub macro_save_open: bool,
    pub macro_name: String,

    pub prefs_open: bool,
    pub prefs_page: usize,
    pub about_open: bool,
    pub shortcuts_open: bool,
    pub map_filter: String,
    pub map_capture: Option<Cmd>,
    pub map_captured: Option<(Key, egui::Modifiers)>,
    pub map_status: String,
    pub run_open: bool,
    pub run_text: String,
    pub run_name: String,
}

impl Default for Dialogs {
    fn default() -> Self {
        Dialogs {
            find_open: false,
            find_tab: FindTab::Find,
            find_focus: false,
            find_select: false,
            find_refocus: false,
            find_status: String::new(),
            find_sel_range: None,
            mark_style: 0,
            mark_bookmark: false,
            dir_history: Vec::new(),
            goto_open: false,
            goto_text: String::new(),
            goto_offset: false,
            goto_focus: false,
            column_open: false,
            column_numbers: false,
            column_text: String::new(),
            column_start: "1".into(),
            column_step: "1".into(),
            column_repeat: "1".into(),
            column_zero: false,
            column_radix: 10,
            macro_run_open: false,
            macro_run_times: 1,
            macro_run_eof: false,
            macro_run_which: 0,
            macro_save_open: false,
            macro_name: String::new(),
            prefs_open: false,
            prefs_page: 0,
            about_open: false,
            shortcuts_open: false,
            map_filter: String::new(),
            map_capture: None,
            map_captured: None,
            map_status: String::new(),
            run_open: false,
            run_text: "$(FULL_CURRENT_PATH)".into(),
            run_name: String::new(),
        }
    }
}

fn history_field(ui: &mut egui::Ui, text: &mut String, history: &[String], hint: &str, width: f32, id: &str, select_all: &mut bool) -> egui::Response {
    let mut resp = None;
    ui.horizontal(|ui| {
        let eid = egui::Id::new(id);
        let r = ui.add(egui::TextEdit::singleline(text).id(eid).desired_width(width).hint_text(hint));
        if *select_all && r.has_focus() {
            // applied once focus is really ours (egui collapses selections on unfocused widgets)
            let mut st = egui::TextEdit::load_state(ui.ctx(), eid).unwrap_or_default();
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()))));
            st.store(ui.ctx(), eid);
            *select_all = false;
            ui.ctx().request_repaint();
        }
        resp = Some(r);
        ui.menu_button("▾", |ui| {
            if history.is_empty() {
                ui.label("(no history)");
            }
            for h in history.iter().take(20) {
                let short: String = h.chars().take(60).collect();
                if ui.button(short).clicked() {
                    *text = h.clone();
                    ui.close();
                }
            }
        });
    });
    resp.unwrap()
}

impl App {
    pub fn show_dialogs(&mut self, ctx: &egui::Context) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        self.show_pending(ctx);
        self.show_find(ctx, &mut cmds);
        self.show_goto(ctx);
        self.show_column(ctx);
        self.show_macro_dialogs(ctx);
        self.show_prefs(ctx);
        self.show_run(ctx);
        self.show_about(ctx);
        self.show_shortcuts(ctx);
        cmds
    }

    // ----------------------------------------------------------- find

    fn show_find(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
        if !self.dialogs.find_open {
            return;
        }
        let mut open = true;
        let mut q: Query = self.settings.find.clone();
        let mut tab = self.dialogs.find_tab;
        let mut action: Option<FindAction> = None;
        let focus = std::mem::take(&mut self.dialogs.find_focus);
        let mut select_pending = self.dialogs.find_select || focus;
        let refocus = std::mem::take(&mut self.dialogs.find_refocus);
        let find_hist = self.settings.find_history.clone();
        let repl_hist = self.settings.replace_history.clone();
        let mut filters = self.settings.find_filters.clone();
        let mut dir = self.settings.find_dir.clone();
        let mut recursive = self.settings.find_recursive;
        let mut hidden = self.settings.find_hidden;
        let has_sel_range = self.dialogs.find_sel_range.is_some();
        let status = self.dialogs.find_status.clone();
        let running = self.fif_running;
        let mut mark_style = self.dialogs.mark_style;
        let mut mark_bm = self.dialogs.mark_bookmark;
        let dir_hist = self.dialogs.dir_history.clone();
        let mut pattern_resp_enter = (false, false);

        egui::Window::new("Find")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::RIGHT_TOP, [-24.0, 96.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (t, label) in [(FindTab::Find, "Find"), (FindTab::Replace, "Replace"), (FindTab::Files, "Find in Files"), (FindTab::Mark, "Mark")] {
                        if ui.selectable_label(tab == t, label).clicked() {
                            tab = t;
                        }
                    }
                });
                ui.separator();
                egui::Grid::new("find_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Find what:");
                    let r = history_field(ui, &mut q.pattern, &find_hist, "", 260.0, "find_what", &mut select_pending);
                    if focus || refocus {
                        r.request_focus();
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        pattern_resp_enter = (
                            true,
                            ui.input(|i| i.modifiers.shift || i.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if modifiers.shift))),
                        );
                    }
                    ui.end_row();
                    if tab == FindTab::Replace || tab == FindTab::Files {
                        ui.label("Replace with:");
                        let r = history_field(ui, &mut q.replace, &repl_hist, "", 260.0, "replace_with", &mut false);
                        let _ = r;
                        ui.end_row();
                    }
                    if tab == FindTab::Files {
                        ui.label("Filters:");
                        ui.add(egui::TextEdit::singleline(&mut filters).desired_width(284.0));
                        ui.end_row();
                        ui.label("Directory:");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut dir).desired_width(230.0));
                            ui.menu_button("▾", |ui| {
                                for h in dir_hist.iter().take(15) {
                                    if ui.button(h).clicked() {
                                        dir = h.clone();
                                        ui.close();
                                    }
                                }
                            });
                            if ui.button("…").clicked() {
                                if let Some(p) = rfd::FileDialog::new().set_directory(&dir).pick_folder() {
                                    dir = p.to_string_lossy().into_owned();
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
                ui.add_space(4.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        if tab != FindTab::Files && tab != FindTab::Mark {
                            ui.checkbox(&mut q.backward, "Backward direction");
                        }
                        ui.checkbox(&mut q.whole_word, "Match whole word only");
                        ui.checkbox(&mut q.match_case, "Match case");
                        if tab != FindTab::Files && tab != FindTab::Mark {
                            ui.checkbox(&mut q.wrap, "Wrap around");
                        }
                        if tab == FindTab::Files {
                            ui.checkbox(&mut recursive, "In all sub-folders");
                            ui.checkbox(&mut hidden, "In hidden folders");
                        }
                        if tab == FindTab::Mark {
                            ui.checkbox(&mut mark_bm, "Bookmark line");
                        }
                        if tab != FindTab::Files {
                            ui.add_enabled(has_sel_range, egui::Checkbox::new(&mut q.in_selection, "In selection"));
                        }
                    });
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Search Mode").strong());
                        ui.radio_value(&mut q.mode, Mode::Normal, "Normal");
                        ui.radio_value(&mut q.mode, Mode::Extended, "Extended (\\n, \\r, \\t, \\0, \\x...)");
                        ui.radio_value(&mut q.mode, Mode::Regex, "Regular expression");
                        ui.add_enabled(q.mode == Mode::Regex, egui::Checkbox::new(&mut q.dot_newline, ". matches newline"));
                        if tab == FindTab::Mark {
                            ui.horizontal(|ui| {
                                ui.label("Style:");
                                for i in 0..5u8 {
                                    ui.radio_value(&mut mark_style, i, (i + 1).to_string());
                                }
                            });
                        }
                    });
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    match tab {
                        FindTab::Find => {
                            if ui.button("Find Next").clicked() {
                                action = Some(FindAction::Op(SearchOp::FindNext));
                            }
                            if ui.button("Count").clicked() {
                                action = Some(FindAction::Op(SearchOp::Count));
                            }
                            if ui.button("Find All in Current Document").clicked() {
                                action = Some(FindAction::Op(SearchOp::FindAll));
                            }
                            if ui.button("Find All in All Opened Documents").clicked() {
                                action = Some(FindAction::FindAllDocs);
                            }
                        }
                        FindTab::Replace => {
                            if ui.button("Find Next").clicked() {
                                action = Some(FindAction::Op(SearchOp::FindNext));
                            }
                            if ui.button("Replace").clicked() {
                                action = Some(FindAction::Op(SearchOp::Replace));
                            }
                            if ui.button("Replace All").clicked() {
                                action = Some(FindAction::Op(SearchOp::ReplaceAll));
                            }
                            if ui.button("Replace All in All Opened Documents").clicked() {
                                action = Some(FindAction::ReplaceAllDocs);
                            }
                        }
                        FindTab::Files => {
                            if running {
                                if ui.button("Stop").clicked() {
                                    action = Some(FindAction::Stop);
                                }
                            } else {
                                if ui.button("Find All").clicked() {
                                    action = Some(FindAction::FindFiles);
                                }
                                if ui.button("Replace in Files").clicked() {
                                    action = Some(FindAction::ReplaceFiles);
                                }
                            }
                        }
                        FindTab::Mark => {
                            if ui.button("Mark All").clicked() {
                                action = Some(FindAction::Op(SearchOp::MarkAll));
                            }
                            if ui.button("Clear all marks").clicked() {
                                action = Some(FindAction::ClearMarks);
                            }
                        }
                    }
                });
                ui.add_space(4.0);
                if running {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Searching...");
                    });
                }
                if !status.is_empty() {
                    ui.label(RichText::new(&status).italics());
                }
            });

        if ctx.input(|i| i.key_pressed(Key::Escape)) && open && self.ed.has_focus == false {
            open = false;
        }

        self.dialogs.find_select = select_pending;
        self.settings.find = q.clone();
        self.settings.find_filters = filters;
        self.settings.find_dir = dir;
        self.settings.find_recursive = recursive;
        self.settings.find_hidden = hidden;
        self.dialogs.find_tab = tab;
        self.dialogs.mark_style = mark_style;
        self.dialogs.mark_bookmark = mark_bm;
        if !open {
            self.dialogs.find_open = false;
            self.want_editor_focus = true;
        }
        if pattern_resp_enter.0 {
            self.dialogs.find_refocus = true;
        }
        if pattern_resp_enter.0 && action.is_none() {
            let op = match (tab, pattern_resp_enter.1) {
                (FindTab::Files, _) => FindAction::FindFiles,
                (FindTab::Mark, _) => FindAction::Op(SearchOp::MarkAll),
                (_, true) => FindAction::Op(SearchOp::FindPrev),
                _ => FindAction::Op(SearchOp::FindNext),
            };
            action = Some(op);
        }
        if let Some(a) = action {
            match a {
                FindAction::Op(op) => cmds.push(Cmd::Search(op, q)),
                FindAction::FindAllDocs => self.find_all_open_docs(q),
                FindAction::ReplaceAllDocs => self.replace_all_open_docs(q),
                FindAction::FindFiles => self.start_find_in_files(q),
                FindAction::ReplaceFiles => self.start_replace_in_files(q),
                FindAction::Stop => self.fif_cancel.store(true, std::sync::atomic::Ordering::Relaxed),
                FindAction::ClearMarks => cmds.push(Cmd::ClearMarks),
            }
            self.dialogs.find_focus = false;
        }
    }

    // ----------------------------------------------------------- goto

    fn show_goto(&mut self, ctx: &egui::Context) {
        if !self.dialogs.goto_open {
            return;
        }
        let mut open = true;
        let mut go = false;
        let mut cancel = false;
        let (cur_line, max_line, cur_pos, max_pos) = {
            let d = self.doc_ref();
            (d.line_of(d.caret()) + 1, d.len_lines(), d.caret(), d.len_chars())
        };
        let mut text = std::mem::take(&mut self.dialogs.goto_text);
        let mut offset = self.dialogs.goto_offset;
        let focus = std::mem::take(&mut self.dialogs.goto_focus);
        let first = !focus && text.is_empty();
        egui::Window::new("Go To...").open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, -80.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.radio_value(&mut offset, false, "Line");
                ui.radio_value(&mut offset, true, "Offset");
            });
            if offset {
                ui.label(format!("You are here: {cur_pos}"));
                ui.label(format!("You want to go: (max {max_pos})"));
            } else {
                ui.label(format!("You are here: {cur_line}"));
                ui.label(format!("You want to go: (max {max_line})"));
            }
            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(180.0));
            if first {
                r.request_focus();
            }
            ui.horizontal(|ui| {
                if ui.button("Go").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            open = false;
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            open = false;
        }
        self.dialogs.goto_offset = offset;
        if go {
            if let Ok(n) = text.trim().parse::<usize>() {
                if offset {
                    let d = self.doc();
                    d.set_caret(n.min(max_pos));
                    self.reveal();
                } else {
                    self.goto_line(n.max(1));
                }
                open = false;
            }
        }
        self.dialogs.goto_text = if open { text } else { String::new() };
        if !open {
            self.dialogs.goto_open = false;
            self.want_editor_focus = true;
        } else if first {
            self.dialogs.goto_focus = false;
        }
    }

    // --------------------------------------------------------- column

    fn show_column(&mut self, ctx: &egui::Context) {
        if !self.dialogs.column_open {
            return;
        }
        let mut open = true;
        let mut apply = false;
        let mut numbers = self.dialogs.column_numbers;
        let mut text = self.dialogs.column_text.clone();
        let mut start = self.dialogs.column_start.clone();
        let mut step = self.dialogs.column_step.clone();
        let mut repeat = self.dialogs.column_repeat.clone();
        let mut zero = self.dialogs.column_zero;
        let mut radix = self.dialogs.column_radix;
        egui::Window::new("Column / Multi-selection Editor").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label("Applies to every caret / selection (Alt+drag selects a column).");
            ui.radio_value(&mut numbers, false, "Text to insert");
            ui.add_enabled(!numbers, egui::TextEdit::singleline(&mut text).desired_width(260.0));
            ui.radio_value(&mut numbers, true, "Number to insert");
            ui.add_enabled_ui(numbers, |ui| {
                egui::Grid::new("col_nums").num_columns(2).show(ui, |ui| {
                    ui.label("Initial number:");
                    ui.add(egui::TextEdit::singleline(&mut start).desired_width(80.0));
                    ui.end_row();
                    ui.label("Increase by:");
                    ui.add(egui::TextEdit::singleline(&mut step).desired_width(80.0));
                    ui.end_row();
                    ui.label("Repeat:");
                    ui.add(egui::TextEdit::singleline(&mut repeat).desired_width(80.0));
                    ui.end_row();
                });
                ui.checkbox(&mut zero, "Leading zeros");
                ui.horizontal(|ui| {
                    for (r, l) in [(10, "Dec"), (16, "Hex"), (8, "Oct"), (2, "Bin")] {
                        ui.radio_value(&mut radix, r, l);
                    }
                });
            });
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    apply = true;
                }
                if ui.button("Close").clicked() {
                    // handled below through `open`
                }
            });
        });
        self.dialogs.column_numbers = numbers;
        self.dialogs.column_text = text.clone();
        self.dialogs.column_start = start.clone();
        self.dialogs.column_step = step.clone();
        self.dialogs.column_repeat = repeat.clone();
        self.dialogs.column_zero = zero;
        self.dialogs.column_radix = radix;
        if apply {
            if numbers {
                let s = start.trim().parse::<i64>().unwrap_or(0);
                let n = ColumnNumbers {
                    start: s,
                    step: step.trim().parse().unwrap_or(1),
                    repeat: repeat.trim().parse().unwrap_or(1),
                    width: start.trim().trim_start_matches('-').len().max(self.doc_ref().sels.len().to_string().len()),
                    radix,
                    zero_pad: zero,
                };
                self.doc().column_insert("", Some(&n));
            } else {
                self.doc().column_insert(&text, None);
            }
            open = false;
        }
        if !open {
            self.dialogs.column_open = false;
            self.want_editor_focus = true;
        }
    }

    // ---------------------------------------------------------- macros

    fn show_macro_dialogs(&mut self, ctx: &egui::Context) {
        if self.dialogs.macro_save_open {
            let mut open = true;
            let mut name = self.dialogs.macro_name.clone();
            let mut save = false;
            egui::Window::new("Save Current Recorded Macro").open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                let r = ui.text_edit_singleline(&mut name);
                r.request_focus();
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
                        save = true;
                    }
                });
            });
            self.dialogs.macro_name = name.clone();
            if save && !name.trim().is_empty() {
                let steps = self.last_macro.clone();
                self.settings.macros.push(SavedMacro::new(name.trim().to_string(), steps));
                open = false;
            }
            if !open {
                self.dialogs.macro_save_open = false;
            }
        }
        if self.dialogs.macro_run_open {
            let mut open = true;
            let mut run = false;
            let mut times = self.dialogs.macro_run_times;
            let mut eof = self.dialogs.macro_run_eof;
            let mut which = self.dialogs.macro_run_which;
            let names: Vec<String> = std::iter::once("Current recorded macro".to_string()).chain(self.settings.macros.iter().map(|m| m.name.clone())).collect();
            egui::Window::new("Run a Macro Multiple Times").open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                egui::ComboBox::from_label("Macro").selected_text(names[which.min(names.len() - 1)].clone()).show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        ui.selectable_value(&mut which, i, n);
                    }
                });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut eof, false, "Run");
                    ui.add_enabled(!eof, egui::DragValue::new(&mut times).range(1..=100000));
                    ui.label("time(s)");
                });
                ui.radio_value(&mut eof, true, "Run until the end of file");
                if ui.button("Run").clicked() {
                    run = true;
                }
            });
            self.dialogs.macro_run_times = times;
            self.dialogs.macro_run_eof = eof;
            self.dialogs.macro_run_which = which;
            if run {
                let steps = if which == 0 { self.last_macro.clone() } else { self.settings.macros.get(which - 1).map(|m| m.steps.clone()).unwrap_or_default() };
                if eof {
                    let mut guard = 0;
                    loop {
                        let before = self.doc_ref().caret();
                        self.play_macro(ctx, &steps, 1);
                        let d = self.doc_ref();
                        guard += 1;
                        if d.caret() >= d.len_chars() || d.caret() == before || guard > 200_000 {
                            break;
                        }
                    }
                } else {
                    self.play_macro(ctx, &steps, times);
                }
                open = false;
            }
            if !open {
                self.dialogs.macro_run_open = false;
            }
        }
    }

    // ----------------------------------------------------------- run

    fn show_run(&mut self, ctx: &egui::Context) {
        if !self.dialogs.run_open {
            return;
        }
        let mut open = true;
        let mut text = self.dialogs.run_text.clone();
        let mut name = self.dialogs.run_name.clone();
        let mut run = false;
        let mut save = false;
        egui::Window::new("Run...").open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label("Command (variables: $(FULL_CURRENT_PATH) $(CURRENT_DIRECTORY) $(FILE_NAME) $(NAME_PART) $(CURRENT_WORD))");
            ui.add(egui::TextEdit::singleline(&mut text).desired_width(420.0));
            ui.horizontal(|ui| {
                if ui.button("Run").clicked() {
                    run = true;
                }
                ui.label("Save as:");
                ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0));
                if ui.button("Save").clicked() {
                    save = true;
                }
            });
        });
        self.dialogs.run_text = text.clone();
        self.dialogs.run_name = name.clone();
        if save && !name.trim().is_empty() {
            self.settings.run_commands.push(RunEntry { name: name.trim().to_string(), command: text.clone() });
        }
        if run {
            self.run_command(&text);
            open = false;
        }
        if !open {
            self.dialogs.run_open = false;
        }
    }

    // ----------------------------------------------------------- about

    fn show_about(&mut self, ctx: &egui::Context) {
        if !self.dialogs.about_open {
            return;
        }
        let mut open = true;
        egui::Window::new("About RustPad").open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.heading("RustPad");
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            let build = format!("Build {}", env!("RUSTPAD_BUILD_NUMBER"));
            let commit = env!("RUSTPAD_BUILD_COMMIT");
            ui.horizontal(|ui| {
                ui.strong(&build);
                if !commit.is_empty() {
                    ui.weak(format!("({commit})"));
                }
                if ui.small_button("Copy").on_hover_text("Copy the build number").clicked() {
                    let text = if commit.is_empty() { build.clone() } else { format!("{build} ({commit})") };
                    ui.ctx().copy_text(text);
                }
            });
            ui.weak(format!("Built {}", env!("RUSTPAD_BUILD_TIME")));
            ui.label("A cross-platform source code editor written in Rust.");
            ui.label("Built on egui / eframe, ropey, regex and encoding_rs.");
            ui.separator();
            ui.label("Independent clean-room implementation: no code from any other editor was used.");
        });
        if !open {
            self.dialogs.about_open = false;
        }
    }

    fn show_shortcuts(&mut self, ctx: &egui::Context) {
        if !self.dialogs.shortcuts_open {
            self.dialogs.map_capture = None;
            return;
        }
        let mut open = true;
        let mut filter = self.dialogs.map_filter.clone();
        let mut capture = self.dialogs.map_capture.clone();
        let captured = self.dialogs.map_captured.take();
        let mut status = self.dialogs.map_status.clone();
        let mut reset = false;
        if let (Some(cmd), Some((key, mods))) = (capture.clone(), captured) {
            let plain_typing = !(mods.command || mods.ctrl || mods.alt) && !matches!(key, Key::F1 | Key::F2 | Key::F3 | Key::F4 | Key::F5 | Key::F6 | Key::F7 | Key::F8 | Key::F9 | Key::F10 | Key::F11 | Key::F12);
            if key == Key::Escape {
                capture = None;
            } else if plain_typing {
                status = "Use Ctrl/Alt with a key, or a function key.".into();
            } else {
                let clash = cmd::with_shortcuts(|l| l.iter().find(|s| s.mods.matches_exact(mods) && s.key == key && s.cmd != cmd).map(|s| s.cmd.label()));
                cmd::set_binding(cmd.clone(), Some((mods, key)));
                status = match clash {
                    Some(other) => format!("Bound {}; removed from \"{other}\"", cmd::format_shortcut(mods, key)),
                    None => format!("Bound {}", cmd::format_shortcut(mods, key)),
                };
                capture = None;
            }
        }
        egui::Window::new("Shortcut Mapper").open(&mut open).default_size([560.0, 500.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Filter:");
                ui.add(egui::TextEdit::singleline(&mut filter).desired_width(220.0));
                if ui.button("Reset all to defaults").clicked() {
                    reset = true;
                }
            });
            if !status.is_empty() {
                ui.label(RichText::new(&status).italics());
            }
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("shortcut_grid").striped(true).num_columns(3).spacing([14.0, 4.0]).show(ui, |ui| {
                    let f = filter.to_lowercase();
                    for c in Cmd::bindable() {
                        let label = c.label();
                        if !f.is_empty() && !label.to_lowercase().contains(&f) {
                            continue;
                        }
                        ui.label(&label);
                        let capturing = capture.as_ref() == Some(&c);
                        if capturing {
                            ui.label(RichText::new("press a key combination… (Esc cancels)").italics());
                        } else {
                            let t = cmd::shortcut_text(&c);
                            ui.label(RichText::new(if t.is_empty() { "—".to_string() } else { t }).monospace());
                        }
                        ui.horizontal(|ui| {
                            if ui.small_button(if capturing { "Cancel" } else { "Change" }).clicked() {
                                capture = if capturing { None } else { Some(c.clone()) };
                                status.clear();
                            }
                            if ui.small_button("Clear").clicked() {
                                cmd::set_binding(c.clone(), None);
                                status = format!("Cleared shortcut for \"{label}\"");
                            }
                        });
                        ui.end_row();
                    }
                });
            });
        });
        if reset {
            cmd::reset_bindings();
            status = "All shortcuts restored to defaults".into();
        }
        self.dialogs.map_filter = filter;
        self.dialogs.map_capture = capture;
        self.dialogs.map_status = status;
        if !open {
            self.dialogs.shortcuts_open = false;
            self.dialogs.map_capture = None;
        }
    }

    // ----------------------------------------------------- preferences

    fn show_prefs(&mut self, ctx: &egui::Context) {
        if !self.dialogs.prefs_open {
            return;
        }
        let mut open = true;
        let mut page = self.dialogs.prefs_page;
        let mut s: Settings = self.settings.clone();
        let theme_names: Vec<String> = self.themes.iter().map(|t| t.name.clone()).collect();
        egui::Window::new("Preferences").open(&mut open).collapsible(false).default_size([520.0, 420.0]).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_min_width(110.0);
                    for (i, n) in ["General", "Editing", "Display", "Files", "Search"].iter().enumerate() {
                        if ui.selectable_label(page == i, *n).clicked() {
                            page = i;
                        }
                    }
                });
                ui.separator();
                ui.vertical(|ui| match page {
                    0 => {
                        egui::ComboBox::from_label("Theme").selected_text(s.theme.clone()).show_ui(ui, |ui| {
                            for n in &theme_names {
                                ui.selectable_value(&mut s.theme, n.clone(), n);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Font size");
                            ui.add(egui::DragValue::new(&mut s.font_size).range(6.0..=48.0).speed(0.2));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Custom monospace font file");
                            ui.add(egui::TextEdit::singleline(&mut s.font_name).desired_width(220.0).hint_text("(restart to apply)"));
                            if ui.button("…").clicked() {
                                if let Some(p) = rfd::FileDialog::new().add_filter("Fonts", &["ttf", "otf", "ttc"]).pick_file() {
                                    s.font_name = p.to_string_lossy().into_owned();
                                }
                            }
                        });
                        ui.checkbox(&mut s.restore_session, "Remember current session for next launch (hot exit)");
                        ui.horizontal(|ui| {
                            ui.label("Snapshot interval (s, 0 = off)");
                            ui.add(egui::DragValue::new(&mut s.autosave_secs).range(0..=600));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Maximum recent files");
                            ui.add(egui::DragValue::new(&mut s.max_recent).range(0..=50));
                        });
                    }
                    1 => {
                        ui.horizontal(|ui| {
                            ui.label("Tab size");
                            ui.add(egui::DragValue::new(&mut s.tab_width).range(1..=16));
                        });
                        ui.checkbox(&mut s.use_spaces, "Replace tab by spaces");
                        ui.checkbox(&mut s.auto_indent, "Auto-indent");
                        ui.checkbox(&mut s.auto_close, "Auto-close brackets and quotes");
                        ui.checkbox(&mut s.auto_close_tags, "Auto-close HTML/XML tags");
                        ui.checkbox(&mut s.word_wrap, "Word wrap");
                        ui.checkbox(&mut s.smart_highlight, "Highlight other occurrences of the selected word");
                        ui.checkbox(&mut s.match_brackets, "Highlight matching brackets");
                        ui.checkbox(&mut s.highlight_current_line, "Highlight current line");
                        ui.checkbox(&mut s.caret_blink, "Caret blinking");
                        ui.checkbox(&mut s.scroll_beyond_last_line, "Allow scrolling past the last line");
                    }
                    2 => {
                        ui.checkbox(&mut s.line_numbers, "Line numbers");
                        ui.checkbox(&mut s.bookmark_margin, "Bookmark margin");
                        ui.checkbox(&mut s.fold_margin, "Folding margin");
                        ui.checkbox(&mut s.indent_guides, "Indent guides");
                        ui.checkbox(&mut s.show_whitespace, "Show spaces and tabs");
                        ui.checkbox(&mut s.show_eol, "Show end-of-line markers");
                        ui.checkbox(&mut s.tool_bar, "Tool bar");
                        ui.checkbox(&mut s.tab_bar, "Tab bar");
                        ui.checkbox(&mut s.status_bar, "Status bar");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut s.show_edge, "Vertical edge at column");
                            ui.add(egui::DragValue::new(&mut s.edge_column).range(10..=400));
                        });
                    }
                    3 => {
                        ui.checkbox(&mut s.watch_files, "Detect files modified outside the editor");
                        ui.checkbox(&mut s.trim_on_save, "Trim trailing spaces when saving");
                        ui.checkbox(&mut s.final_newline_on_save, "Ensure a final newline when saving");
                        egui::ComboBox::from_label("Default line ending for new files")
                            .selected_text(match s.default_eol.as_str() {
                                "crlf" => Eol::CrLf.label(),
                                "cr" => Eol::Cr.label(),
                                _ => Eol::Lf.label(),
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut s.default_eol, "lf".into(), Eol::Lf.label());
                                ui.selectable_value(&mut s.default_eol, "crlf".into(), Eol::CrLf.label());
                                ui.selectable_value(&mut s.default_eol, "cr".into(), Eol::Cr.label());
                            });
                        egui::ComboBox::from_label("Default encoding for new files")
                            .selected_text(s.default_new_encoding.clone())
                            .show_ui(ui, |ui| {
                                for e in ["UTF-8", "UTF-8-BOM", "UTF-16 LE BOM", "UTF-16 BE BOM", "ANSI"] {
                                    ui.selectable_value(&mut s.default_new_encoding, e.to_string(), e);
                                }
                            });
                    }
                    _ => {
                        ui.checkbox(&mut s.find.wrap, "Wrap around by default");
                        ui.label("Search history is kept between sessions.");
                        if ui.button("Clear find history").clicked() {
                            s.find_history.clear();
                            s.replace_history.clear();
                        }
                    }
                });
            });
        });
        let theme_changed = s.theme != self.settings.theme;
        self.settings = s;
        self.dialogs.prefs_page = page;
        if theme_changed {
            self.apply_theme(ctx);
        }
        if !open {
            self.dialogs.prefs_open = false;
            self.settings.save();
            self.want_editor_focus = true;
        }
    }

    // ------------------------------------------------------- modal prompts

    fn show_pending(&mut self, ctx: &egui::Context) {
        let Some(p) = self.pending.take() else { return };
        let mut keep: Option<Pending> = None;
        match p {
            Pending::Message(msg) => {
                let mut ok = false;
                egui::Window::new("RustPad").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(&msg);
                    if ui.button("OK").clicked() {
                        ok = true;
                    }
                });
                if !ok {
                    keep = Some(Pending::Message(msg));
                }
            }
            Pending::ConfirmClose { queue, exit_after } => {
                let Some(&id) = queue.first() else {
                    self.pending = Some(Pending::ConfirmClose { queue, exit_after });
                    self.advance_close();
                    return;
                };
                let Some(idx) = self.tabs.iter().position(|d| d.id == id) else {
                    self.pending = Some(Pending::ConfirmClose { queue, exit_after });
                    self.advance_close();
                    return;
                };
                if !self.tabs[idx].is_modified() {
                    self.pending = Some(Pending::ConfirmClose { queue, exit_after });
                    self.advance_close();
                    return;
                }
                let name = self.tabs[idx].display_name();
                let mut choice = 0; // 1 save, 2 discard, 3 cancel
                egui::Window::new("Save").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("Save file \"{name}\"?"));
                    ui.horizontal(|ui| {
                        if ui.button("Yes").clicked() {
                            choice = 1;
                        }
                        if ui.button("No").clicked() {
                            choice = 2;
                        }
                        if ui.button("Cancel").clicked() {
                            choice = 3;
                        }
                    });
                });
                if ctx.input(|i| i.key_pressed(Key::Escape)) {
                    choice = 3;
                }
                match choice {
                    1 => {
                        if self.save_doc(idx, false) {
                            self.pending = Some(Pending::ConfirmClose { queue, exit_after });
                            self.advance_close();
                        } else {
                            // save dialog cancelled → keep waiting
                            if self.pending.is_none() {
                                keep = Some(Pending::ConfirmClose { queue, exit_after });
                            }
                        }
                    }
                    2 => {
                        // discard changes: close without saving
                        self.tabs[idx].mark_saved();
                        self.pending = Some(Pending::ConfirmClose { queue, exit_after });
                        self.advance_close();
                    }
                    3 => {}
                    _ => keep = Some(Pending::ConfirmClose { queue, exit_after }),
                }
            }
            Pending::ExternalChange(id) => {
                let Some(idx) = self.tabs.iter().position(|d| d.id == id) else { return };
                let name = self.tabs[idx].display_name();
                let modified = self.tabs[idx].is_modified();
                let mut choice = 0;
                egui::Window::new("File changed").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("\"{name}\" was modified by another program."));
                    if modified {
                        ui.label("Reloading will discard your unsaved changes.");
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Reload").clicked() {
                            choice = 1;
                        }
                        if ui.button("Keep my version").clicked() {
                            choice = 2;
                        }
                    });
                });
                match choice {
                    1 => {
                        let cur = self.active;
                        self.active = idx;
                        self.reload_active();
                        self.active = cur;
                    }
                    2 => self.tabs[idx].mark_modified(),
                    _ => keep = Some(Pending::ExternalChange(id)),
                }
            }
            Pending::ExternalDeleted(id) => {
                let Some(idx) = self.tabs.iter().position(|d| d.id == id) else { return };
                let name = self.tabs[idx].display_name();
                let mut choice = 0;
                egui::Window::new("File removed").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("\"{name}\" no longer exists on disk."));
                    ui.horizontal(|ui| {
                        if ui.button("Keep in editor").clicked() {
                            choice = 1;
                        }
                        if ui.button("Close tab").clicked() {
                            choice = 2;
                        }
                    });
                });
                match choice {
                    1 => self.tabs[idx].mark_modified(),
                    2 => self.force_close(idx),
                    _ => keep = Some(Pending::ExternalDeleted(id)),
                }
            }
            Pending::ConfirmReload(id) => {
                let mut choice = 0;
                egui::Window::new("Reload").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label("Discard unsaved changes and reload from disk?");
                    ui.horizontal(|ui| {
                        if ui.button("Reload").clicked() {
                            choice = 1;
                        }
                        if ui.button("Cancel").clicked() {
                            choice = 2;
                        }
                    });
                });
                match choice {
                    1 => {
                        if let Some(i) = self.tabs.iter().position(|d| d.id == id) {
                            let cur = self.active;
                            self.active = i;
                            self.reload_active();
                            self.active = cur;
                        }
                    }
                    2 => {}
                    _ => keep = Some(Pending::ConfirmReload(id)),
                }
            }
            Pending::Rename { doc, mut name } => {
                let mut choice = 0;
                egui::Window::new("Rename").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label("New name:");
                    let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(280.0));
                    r.request_focus();
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
                            choice = 1;
                        }
                        if ui.button("Cancel").clicked() {
                            choice = 2;
                        }
                    });
                });
                if ctx.input(|i| i.key_pressed(Key::Escape)) {
                    choice = 2;
                }
                match choice {
                    1 => {
                        if let Some(i) = self.tabs.iter().position(|d| d.id == doc) {
                            match self.tabs[i].path.clone() {
                                Some(old) => {
                                    let new = old.with_file_name(&name);
                                    if new.exists() {
                                        self.pending = Some(Pending::Message(format!("{} already exists.", new.display())));
                                    } else if let Err(e) = std::fs::rename(&old, &new) {
                                        self.pending = Some(Pending::Message(format!("Rename failed: {e}")));
                                    } else {
                                        let first = self.tabs[i].line_text(0);
                                        let l = crate::lang::detect(Some(&new), &first);
                                        let d = &mut self.tabs[i];
                                        d.path = Some(new.clone());
                                        d.disk_mtime = std::fs::metadata(&new).ok().and_then(|m| m.modified().ok());
                                        d.set_lang(l);
                                    }
                                }
                                None => {
                                    // unsaved buffer: just ask for a location via Save As
                                    let cur = self.active;
                                    self.save_doc(i, true);
                                    self.active = cur;
                                }
                            }
                        }
                    }
                    2 => {}
                    _ => keep = Some(Pending::Rename { doc, name }),
                }
            }
            Pending::Hash { title, text } => {
                let mut choice = 0;
                egui::Window::new(&title).collapsible(false).resizable(true).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(&text).monospace()).selectable(true));
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Copy").clicked() {
                            ui.ctx().copy_text(text.trim_end().to_string());
                        }
                        if ui.button("Insert at caret").clicked() {
                            choice = 2;
                        }
                        if ui.button("Close").clicked() {
                            choice = 1;
                        }
                    });
                });
                match choice {
                    2 => {
                        let t = text.trim_end().to_string();
                        self.doc().type_text(&t);
                    }
                    1 => {}
                    _ => keep = Some(Pending::Hash { title, text }),
                }
            }
            Pending::ConfirmTrash(id) => {
                let mut choice = 0;
                egui::Window::new("Delete file").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label("Permanently delete this file from disk and close the tab?");
                    ui.horizontal(|ui| {
                        if ui.button("Delete").clicked() {
                            choice = 1;
                        }
                        if ui.button("Cancel").clicked() {
                            choice = 2;
                        }
                    });
                });
                match choice {
                    1 => {
                        if let Some(i) = self.tabs.iter().position(|d| d.id == id) {
                            if let Some(p) = self.tabs[i].path.clone() {
                                match std::fs::remove_file(&p) {
                                    Ok(()) => {
                                        self.tabs[i].mark_saved();
                                        self.force_close(i);
                                    }
                                    Err(e) => self.pending = Some(Pending::Message(format!("Delete failed: {e}"))),
                                }
                            }
                        }
                    }
                    2 => {}
                    _ => keep = Some(Pending::ConfirmTrash(id)),
                }
            }
        }
        if self.pending.is_none() {
            self.pending = keep;
        }
    }
}

enum FindAction {
    Op(SearchOp),
    FindAllDocs,
    ReplaceAllDocs,
    FindFiles,
    ReplaceFiles,
    Stop,
    ClearMarks,
}
