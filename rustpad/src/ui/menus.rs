//! Menu bar, tool bar, tab bar and status bar.

use eframe::egui::{self, Align, Color32, Layout, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::app::App;
use crate::cmd::{shortcut_text, Cmd};
use crate::doc::Eol;
use crate::encoding::{encoding_menu, Enc};
use crate::hash::HashKind;
use crate::lang;
use crate::ops::{CaseMode, SortMode, TrimMode};
use crate::util;

pub(crate) fn item(ui: &mut egui::Ui, cmds: &mut Vec<Cmd>, label: &str, cmd: Cmd) {
    let sc = shortcut_text(&cmd);
    let mut b = egui::Button::new(label);
    if !sc.is_empty() {
        b = b.shortcut_text(sc);
    }
    if ui.add(b).clicked() {
        cmds.push(cmd);
        ui.close();
    }
}

fn check(ui: &mut egui::Ui, cmds: &mut Vec<Cmd>, label: &str, on: bool, cmd: Cmd) {
    let sc = shortcut_text(&cmd);
    let text = format!("{} {}", if on { "✔" } else { "   " }, label);
    let mut b = egui::Button::new(text);
    if !sc.is_empty() {
        b = b.shortcut_text(sc);
    }
    if ui.add(b).clicked() {
        cmds.push(cmd);
        ui.close();
    }
}

fn radio(ui: &mut egui::Ui, cmds: &mut Vec<Cmd>, label: &str, on: bool, cmd: Cmd) {
    let text = format!("{} {}", if on { "●" } else { "   " }, label);
    if ui.add(egui::Button::new(text)).clicked() {
        cmds.push(cmd);
        ui.close();
    }
}

impl App {
    pub fn menu_bar(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
        let (read_only, enc, eol, cur_lang) = {
            let d = self.doc_ref();
            (d.read_only, d.enc, d.eol, d.lang)
        };
        let s = self.settings.clone();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                item(ui, cmds, "New", Cmd::New);
                item(ui, cmds, "Open...", Cmd::Open);
                item(ui, cmds, "Open Containing Folder", Cmd::OpenContainingFolder);
                item(ui, cmds, "Open Folder as Workspace...", Cmd::OpenFolderAsWorkspace);
                item(ui, cmds, "Reload from Disk", Cmd::Reload);
                ui.separator();
                item(ui, cmds, "Save", Cmd::Save);
                item(ui, cmds, "Save As...", Cmd::SaveAs);
                item(ui, cmds, "Save All", Cmd::SaveAll);
                item(ui, cmds, "Rename...", Cmd::Rename);
                ui.separator();
                item(ui, cmds, "Close", Cmd::Close);
                item(ui, cmds, "Close All", Cmd::CloseAll);
                item(ui, cmds, "Close All BUT Active Document", Cmd::CloseOthers);
                item(ui, cmds, "Close All to the Left", Cmd::CloseLeft);
                item(ui, cmds, "Close All to the Right", Cmd::CloseRight);
                item(ui, cmds, "Delete File from Disk...", Cmd::MoveToTrash);
                ui.separator();
                ui.menu_button("Copy to Clipboard", |ui| {
                    item(ui, cmds, "Current Full File path to Clipboard", Cmd::CopyPath);
                    item(ui, cmds, "Current Filename to Clipboard", Cmd::CopyFilename);
                    item(ui, cmds, "Current Dir. Path to Clipboard", Cmd::CopyDirectory);
                });
                ui.menu_button("Recent Files", |ui| {
                    if s.recent_files.is_empty() {
                        ui.label("(empty)");
                    }
                    for (i, f) in s.recent_files.iter().enumerate() {
                        if ui.button(format!("{}: {}", i + 1, f)).clicked() {
                            cmds.push(Cmd::OpenRecent(i));
                            ui.close();
                        }
                    }
                    if !s.recent_files.is_empty() {
                        ui.separator();
                        item(ui, cmds, "Clear Recent Files List", Cmd::ClearRecent);
                    }
                });
                item(ui, cmds, "Restore Recent Closed File", Cmd::ReopenClosed);
                ui.separator();
                item(ui, cmds, "Exit", Cmd::Exit);
            });

            ui.menu_button("Edit", |ui| {
                item(ui, cmds, "Undo", Cmd::Undo);
                item(ui, cmds, "Redo", Cmd::Redo);
                ui.separator();
                item(ui, cmds, "Cut", Cmd::Cut);
                item(ui, cmds, "Copy", Cmd::Copy);
                item(ui, cmds, "Paste", Cmd::Paste);
                item(ui, cmds, "Delete", Cmd::DeleteSel);
                item(ui, cmds, "Select All", Cmd::SelectAll);
                ui.separator();
                ui.menu_button("Insert", |ui| {
                    item(ui, cmds, "Date Time (short)", Cmd::InsertDateTime(false));
                    item(ui, cmds, "Date Time (long)", Cmd::InsertDateTime(true));
                });
                ui.menu_button("Line Operations", |ui| {
                    item(ui, cmds, "Duplicate Current Line", Cmd::DuplicateLine);
                    item(ui, cmds, "Remove Duplicate Lines", Cmd::DedupLines(false));
                    item(ui, cmds, "Remove Consecutive Duplicate Lines", Cmd::DedupLines(true));
                    item(ui, cmds, "Split Lines", Cmd::SplitLines);
                    item(ui, cmds, "Join Lines", Cmd::JoinLines);
                    item(ui, cmds, "Move Up Current Line", Cmd::MoveLineUp);
                    item(ui, cmds, "Move Down Current Line", Cmd::MoveLineDown);
                    item(ui, cmds, "Transpose Current Line", Cmd::TransposeLine);
                    item(ui, cmds, "Insert Blank Line Above Current", Cmd::BlankAbove);
                    item(ui, cmds, "Insert Blank Line Below Current", Cmd::BlankBelow);
                    ui.separator();
                    item(ui, cmds, "Cut Current Line", Cmd::CutLine);
                    item(ui, cmds, "Copy Current Line", Cmd::CopyLine);
                    item(ui, cmds, "Delete Current Line", Cmd::DeleteLine);
                    ui.separator();
                    item(ui, cmds, "Reverse Line Order", Cmd::Sort(SortMode::Reverse, false));
                    item(ui, cmds, "Randomize Line Order", Cmd::Sort(SortMode::Random, false));
                    ui.menu_button("Sort Lines", |ui| {
                        item(ui, cmds, "Lexicographically Ascending", Cmd::Sort(SortMode::Lexicographic, false));
                        item(ui, cmds, "Lexicographically Descending", Cmd::Sort(SortMode::Lexicographic, true));
                        item(ui, cmds, "Lexicographically Ascending (ignore case)", Cmd::Sort(SortMode::LexicographicCi, false));
                        item(ui, cmds, "Lexicographically Descending (ignore case)", Cmd::Sort(SortMode::LexicographicCi, true));
                        item(ui, cmds, "As Integers Ascending", Cmd::Sort(SortMode::Integer, false));
                        item(ui, cmds, "As Integers Descending", Cmd::Sort(SortMode::Integer, true));
                        item(ui, cmds, "As Decimals Ascending", Cmd::Sort(SortMode::Decimal, false));
                        item(ui, cmds, "As Decimals Descending", Cmd::Sort(SortMode::Decimal, true));
                    });
                });
                ui.menu_button("Convert Case to", |ui| {
                    item(ui, cmds, "UPPERCASE", Cmd::Case(CaseMode::Upper));
                    item(ui, cmds, "lowercase", Cmd::Case(CaseMode::Lower));
                    item(ui, cmds, "Proper Case", Cmd::Case(CaseMode::Title));
                    item(ui, cmds, "Sentence case", Cmd::Case(CaseMode::Sentence));
                    item(ui, cmds, "iNVERT cASE", Cmd::Case(CaseMode::Invert));
                    item(ui, cmds, "ranDOm CasE", Cmd::Case(CaseMode::Random));
                });
                ui.menu_button("Blank Operations", |ui| {
                    item(ui, cmds, "Trim Trailing Space", Cmd::Trim(TrimMode::Trailing));
                    item(ui, cmds, "Trim Leading Space", Cmd::Trim(TrimMode::Leading));
                    item(ui, cmds, "Trim Leading and Trailing Space", Cmd::Trim(TrimMode::Both));
                    ui.separator();
                    item(ui, cmds, "Remove Empty Lines", Cmd::RemoveEmpty(false));
                    item(ui, cmds, "Remove Empty Lines (Containing Blank Characters)", Cmd::RemoveEmpty(true));
                    ui.separator();
                    item(ui, cmds, "TAB to Space", Cmd::TabsToSpaces);
                    item(ui, cmds, "Space to TAB (All)", Cmd::SpacesToTabs(true));
                    item(ui, cmds, "Space to TAB (Leading)", Cmd::SpacesToTabs(false));
                });
                ui.menu_button("Comment/Uncomment", |ui| {
                    item(ui, cmds, "Toggle Single Line Comment", Cmd::ToggleComment);
                    item(ui, cmds, "Toggle Block Comment", Cmd::BlockComment);
                });
                ui.menu_button("EOL Conversion", |ui| {
                    for e in [Eol::CrLf, Eol::Lf, Eol::Cr] {
                        radio(ui, cmds, e.label(), eol == e, Cmd::ConvertEol(e));
                    }
                });
                ui.menu_button("Multi-select", |ui| {
                    item(ui, cmds, "Select Next Occurrence", Cmd::MultiSelectNext);
                    item(ui, cmds, "Select All Occurrences", Cmd::MultiSelectAll);
                });
                item(ui, cmds, "Auto-Completion", Cmd::AutoComplete);
                item(ui, cmds, "Column Editor...", Cmd::ColumnEditor);
                check(ui, cmds, "Read-Only", read_only, Cmd::ToggleReadOnly);
            });

            ui.menu_button("Search", |ui| {
                item(ui, cmds, "Find...", Cmd::Find);
                item(ui, cmds, "Find in Files...", Cmd::FindInFiles);
                item(ui, cmds, "Find Next", Cmd::FindNext);
                item(ui, cmds, "Find Previous", Cmd::FindPrev);
                item(ui, cmds, "Select and Find Next", Cmd::FindSelNext);
                item(ui, cmds, "Select and Find Previous", Cmd::FindSelPrev);
                item(ui, cmds, "Replace...", Cmd::Replace);
                item(ui, cmds, "Incremental Search", Cmd::IncrementalSearch);
                ui.separator();
                item(ui, cmds, "Go to...", Cmd::GotoLine);
                item(ui, cmds, "Go to Matching Brace", Cmd::GotoBrace);
                item(ui, cmds, "Select All Between Matching Braces", Cmd::SelectBrace);
                ui.separator();
                item(ui, cmds, "Mark...", Cmd::Mark);
                item(ui, cmds, "Clear All Marks", Cmd::ClearMarks);
                ui.menu_button("Bookmark", |ui| {
                    item(ui, cmds, "Toggle Bookmark", Cmd::ToggleBookmark);
                    item(ui, cmds, "Next Bookmark", Cmd::NextBookmark);
                    item(ui, cmds, "Previous Bookmark", Cmd::PrevBookmark);
                    item(ui, cmds, "Clear All Bookmarks", Cmd::ClearBookmarks);
                    item(ui, cmds, "Cut Bookmarked Lines", Cmd::CutBookmarked);
                    item(ui, cmds, "Copy Bookmarked Lines", Cmd::CopyBookmarked);
                    item(ui, cmds, "Remove Bookmarked Lines", Cmd::RemoveBookmarked);
                    item(ui, cmds, "Inverse Bookmark", Cmd::InverseBookmarks);
                });
            });

            ui.menu_button("View", |ui| {
                check(ui, cmds, "Always on Top", self.always_on_top, Cmd::ToggleAlwaysOnTop);
                check(ui, cmds, "Toggle Full Screen Mode", self.fullscreen, Cmd::ToggleFullscreen);
                check(ui, cmds, "Post-It", self.post_it, Cmd::TogglePostIt);
                ui.separator();
                ui.menu_button("Show Symbol", |ui| {
                    check(ui, cmds, "Show White Space and TAB", s.show_whitespace, Cmd::ToggleWhitespace);
                    check(ui, cmds, "Show End of Line", s.show_eol, Cmd::ToggleEol);
                    check(ui, cmds, "Show All Characters", s.show_whitespace && s.show_eol, Cmd::ToggleAllChars);
                    check(ui, cmds, "Show Indent Guide", s.indent_guides, Cmd::ToggleIndentGuides);
                });
                ui.menu_button("Zoom", |ui| {
                    item(ui, cmds, "Zoom In", Cmd::ZoomIn);
                    item(ui, cmds, "Zoom Out", Cmd::ZoomOut);
                    item(ui, cmds, "Restore Default Zoom", Cmd::ZoomReset);
                });
                check(ui, cmds, "Word Wrap", s.word_wrap, Cmd::ToggleWrap);
                ui.menu_button("Fold", |ui| {
                    item(ui, cmds, "Fold All", Cmd::FoldAll);
                    item(ui, cmds, "Unfold All", Cmd::UnfoldAll);
                    item(ui, cmds, "Toggle Current Fold", Cmd::ToggleFold);
                    ui.separator();
                    for n in 1..=8 {
                        item(ui, cmds, &format!("Fold Level {n}"), Cmd::FoldLevel(n));
                    }
                    ui.separator();
                    for n in 1..=8 {
                        item(ui, cmds, &format!("Unfold Level {n}"), Cmd::UnfoldLevel(n));
                    }
                });
                ui.separator();
                check(ui, cmds, "Split View", self.split, Cmd::ToggleSplit);
                item(ui, cmds, if self.split_vertical { "Split Horizontally (stack)" } else { "Split Vertically (side by side)" }, Cmd::ToggleSplitOrientation);
                check(ui, cmds, "Document Map", s.doc_map, Cmd::ToggleDocMap);
                check(ui, cmds, "Function List", s.function_list, Cmd::ToggleFunctionList);
                check(ui, cmds, "Folder as Workspace", s.workspace, Cmd::ToggleWorkspace);
                check(ui, cmds, "Search Results Panel", self.show_results, Cmd::ToggleResults);
                ui.separator();
                check(ui, cmds, "Tool Bar", s.tool_bar, Cmd::ToggleToolBar);
                check(ui, cmds, "Tab Bar", s.tab_bar, Cmd::ToggleTabBar);
                check(ui, cmds, "Status Bar", s.status_bar, Cmd::ToggleStatusBar);
                check(ui, cmds, "Line Numbers", s.line_numbers, Cmd::ToggleLineNumbers);
                check(ui, cmds, "Bookmark Margin", s.bookmark_margin, Cmd::ToggleBookmarkMargin);
                check(ui, cmds, "Folding Margin", s.fold_margin, Cmd::ToggleFoldMargin);
            });

            ui.menu_button("Encoding", |ui| {
                ui.label(RichText::new(format!("Current: {}", enc.label())).weak());
                ui.separator();
                for e in [Enc::ansi(), Enc::Utf8, Enc::Utf8Bom, Enc::Utf16Be, Enc::Utf16Le] {
                    radio(ui, cmds, &format!("Convert to {}", e.label()), enc == e, Cmd::SetEncoding(e.label()));
                }
                ui.separator();
                ui.menu_button("Convert to Character Set", |ui| {
                    for (group, list) in encoding_menu() {
                        ui.menu_button(group, |ui| {
                            for e in list {
                                radio(ui, cmds, &e.label(), enc == e, Cmd::SetEncoding(e.label()));
                            }
                        });
                    }
                });
                ui.menu_button("Reload File as", |ui| {
                    for e in [Enc::Utf8, Enc::Utf8Bom, Enc::Utf16Be, Enc::Utf16Le] {
                        item(ui, cmds, &e.label(), Cmd::ReloadAsEncoding(e.label()));
                    }
                    ui.separator();
                    for (group, list) in encoding_menu() {
                        ui.menu_button(group, |ui| {
                            for e in list {
                                item(ui, cmds, &e.label(), Cmd::ReloadAsEncoding(e.label()));
                            }
                        });
                    }
                });
            });

            ui.menu_button("Language", |ui| {
                egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
                    let mut ids: Vec<usize> = (1..lang::LANGS.len()).collect();
                    ids.sort_by_key(|&i| lang::LANGS[i].name.to_lowercase());
                    radio(ui, cmds, "Normal Text", cur_lang == lang::PLAIN, Cmd::SetLang(lang::PLAIN));
                    ui.separator();
                    for i in ids {
                        radio(ui, cmds, lang::LANGS[i].name, cur_lang == i, Cmd::SetLang(i));
                    }
                });
            });

            ui.menu_button("Settings", |ui| {
                item(ui, cmds, "Preferences...", Cmd::Preferences);
                ui.menu_button("Theme", |ui| {
                    for t in &self.themes {
                        radio(ui, cmds, &t.name, s.theme == t.name, Cmd::SetTheme(t.name.clone()));
                    }
                });
                item(ui, cmds, "Shortcut Mapper...", Cmd::Shortcuts);
                if ui.button("Open Config Folder").clicked() {
                    crate::app::open_with_default(&crate::settings::config_dir().to_string_lossy());
                    ui.close();
                }
            });

            ui.menu_button("Tools", |ui| {
                for k in [HashKind::Md5, HashKind::Sha1, HashKind::Sha256, HashKind::Sha512] {
                    ui.menu_button(k.name(), |ui| {
                        item(ui, cmds, "Generate from selection / document", Cmd::Hash(k));
                        item(ui, cmds, "Generate from files...", Cmd::HashFiles(k));
                    });
                }
            });

            ui.menu_button("Macro", |ui| {
                if self.recording {
                    item(ui, cmds, "Stop Recording", Cmd::MacroStop);
                } else {
                    item(ui, cmds, "Start Recording", Cmd::MacroRecord);
                }
                item(ui, cmds, "Playback", Cmd::MacroPlay);
                item(ui, cmds, "Run a Macro Multiple Times...", Cmd::MacroRunMultiple);
                item(ui, cmds, "Save Current Recorded Macro...", Cmd::MacroSave);
                if !s.macros.is_empty() {
                    ui.separator();
                    for (i, m) in s.macros.iter().enumerate() {
                        if ui.button(&m.name).clicked() {
                            cmds.push(Cmd::MacroPlayNamed(i));
                            ui.close();
                        }
                    }
                }
            });

            ui.menu_button("Run", |ui| {
                item(ui, cmds, "Run...", Cmd::Run);
                item(ui, cmds, "Open in Default Viewer", Cmd::LaunchInBrowser);
                if !s.run_commands.is_empty() {
                    ui.separator();
                    for (i, r) in s.run_commands.iter().enumerate() {
                        if ui.button(&r.name).clicked() {
                            cmds.push(Cmd::RunNamed(i));
                            ui.close();
                        }
                    }
                }
            });

            ui.menu_button("Window", |ui| {
                for (i, d) in self.tabs.iter().enumerate() {
                    let mark = if i == self.active { "●" } else { "   " };
                    if ui.button(format!("{mark} {}. {}", i + 1, d.display_name())).clicked() {
                        cmds.push(Cmd::Nop);
                        self.active = i;
                        ui.close();
                    }
                }
            });

            ui.menu_button("?", |ui| {
                item(ui, cmds, "About RustPad", Cmd::About);
                item(ui, cmds, "Keyboard Shortcuts", Cmd::Shortcuts);
            });
        });
    }

    pub fn tool_bar(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
        let (can_undo, can_redo) = {
            let d = self.doc_ref();
            (d.can_undo(), d.can_redo())
        };
        let recording = self.recording;
        let wrap = self.settings.word_wrap;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let mut b = |ui: &mut egui::Ui, label: &str, tip: &str, cmd: Cmd, enabled: bool, selected: bool| {
                let r = ui.add_enabled(enabled, egui::Button::new(label).selected(selected).min_size(Vec2::new(26.0, 22.0)));
                if r.on_hover_text(format!("{tip}  {}", shortcut_text(&cmd))).clicked() {
                    cmds.push(cmd);
                }
            };
            b(ui, "New", "New", Cmd::New, true, false);
            b(ui, "Open", "Open", Cmd::Open, true, false);
            b(ui, "Save", "Save", Cmd::Save, true, false);
            b(ui, "Save All", "Save All", Cmd::SaveAll, true, false);
            b(ui, "Close", "Close", Cmd::Close, true, false);
            ui.separator();
            b(ui, "Cut", "Cut", Cmd::Cut, true, false);
            b(ui, "Copy", "Copy", Cmd::Copy, true, false);
            b(ui, "Paste", "Paste", Cmd::Paste, true, false);
            ui.separator();
            b(ui, "Undo", "Undo", Cmd::Undo, can_undo, false);
            b(ui, "Redo", "Redo", Cmd::Redo, can_redo, false);
            ui.separator();
            b(ui, "Find", "Find", Cmd::Find, true, false);
            b(ui, "Replace", "Replace", Cmd::Replace, true, false);
            ui.separator();
            b(ui, "Zoom +", "Zoom In", Cmd::ZoomIn, true, false);
            b(ui, "Zoom −", "Zoom Out", Cmd::ZoomOut, true, false);
            b(ui, "Wrap", "Word Wrap", Cmd::ToggleWrap, true, wrap);
            b(ui, "¶", "Show All Characters", Cmd::ToggleAllChars, true, self.settings.show_whitespace && self.settings.show_eol);
            ui.separator();
            if recording {
                b(ui, "■ Stop", "Stop Recording", Cmd::MacroStop, true, true);
            } else {
                b(ui, "● Rec", "Start Recording", Cmd::MacroRecord, true, false);
            }
            b(ui, "▶ Play", "Playback", Cmd::MacroPlay, !self.last_macro.is_empty(), false);
        });
    }

    pub fn tab_bar(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
        let theme = self.theme().clone();
        let n = self.tabs.len();
        let mut swap: Option<(usize, usize)> = None;
        let mut select: Option<usize> = None;
        let mut close: Option<usize> = None;
        let active = self.active;
        let view2 = if self.split { Some(self.view2) } else { None };
        let font = egui::FontId::proportional(13.0);
        let strip_bg = if theme.dark { Color32::from_rgb(0x23, 0x25, 0x29) } else { Color32::from_rgb(0xdd, 0xdd, 0xdd) };
        let h = 26.0;
        let (strip, strip_resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
        if strip_resp.double_clicked() {
            cmds.push(Cmd::New);
        }
        ui.painter().rect_filled(strip, 0.0, strip_bg);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(strip).layout(Layout::left_to_right(Align::Min)));
        egui::ScrollArea::horizontal().id_salt("tabscroll").auto_shrink([false, false]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(&mut child, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                for i in 0..n {
                    let (name, modified, read_only, id) = {
                        let d = &self.tabs[i];
                        (d.display_name(), d.is_modified(), d.read_only, d.id)
                    };
                    let is_active = i == active;
                    let galley = ui.painter().layout_no_wrap(name.clone(), font.clone(), Color32::WHITE);
                    let w = galley.size().x + 56.0;
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click_and_drag());
                    let bg = if is_active {
                        theme.bg.c()
                    } else if resp.hovered() {
                        if theme.dark { Color32::from_rgb(0x34, 0x37, 0x3d) } else { Color32::from_rgb(0xe8, 0xe8, 0xe8) }
                    } else if theme.dark {
                        Color32::from_rgb(0x2b, 0x2d, 0x31)
                    } else {
                        Color32::from_rgb(0xcc, 0xcc, 0xcc)
                    };
                    let p = ui.painter();
                    p.rect_filled(rect, 0.0, bg);
                    if is_active {
                        p.rect_filled(Rect::from_min_size(rect.min, Vec2::new(w, 3.0)), 0.0, Color32::from_rgb(0xff, 0x8c, 0x00));
                    } else if Some(i) == view2 {
                        p.rect_filled(Rect::from_min_size(rect.min, Vec2::new(w, 2.0)), 0.0, theme.bookmark.c());
                    }
                    // save-state icon
                    let icon_c = Pos2::new(rect.left() + 12.0, rect.center().y + 1.0);
                    let icon_col = if modified { Color32::from_rgb(0xe0, 0x30, 0x30) } else if read_only { Color32::from_rgb(0x90, 0x90, 0x90) } else { Color32::from_rgb(0x30, 0x70, 0xe0) };
                    p.rect_filled(Rect::from_center_size(icon_c, Vec2::new(9.0, 9.0)), 2.0, icon_col);
                    p.rect_filled(Rect::from_center_size(Pos2::new(icon_c.x, icon_c.y - 2.0), Vec2::new(5.0, 3.0)), 0.0, Color32::from_white_alpha(200));
                    let fg = if theme.dark { Color32::from_gray(if is_active { 235 } else { 170 }) } else { Color32::from_gray(if is_active { 20 } else { 70 }) };
                    p.text(Pos2::new(rect.left() + 24.0, rect.center().y), egui::Align2::LEFT_CENTER, &name, font.clone(), fg);
                    // close button
                    let close_rect = Rect::from_center_size(Pos2::new(rect.right() - 13.0, rect.center().y), Vec2::splat(16.0));
                    let close_resp = ui.interact(close_rect, ui.id().with(("tabclose", id)), Sense::click());
                    if resp.hovered() || is_active || close_resp.hovered() {
                        let cc = if close_resp.hovered() { Color32::from_rgb(0xe0, 0x40, 0x40) } else { fg };
                        let c = close_rect.center();
                        let pp = ui.painter();
                        pp.line_segment([c + Vec2::new(-3.5, -3.5), c + Vec2::new(3.5, 3.5)], Stroke::new(1.5, cc));
                        pp.line_segment([c + Vec2::new(-3.5, 3.5), c + Vec2::new(3.5, -3.5)], Stroke::new(1.5, cc));
                    }
                    if close_resp.clicked() {
                        close = Some(i);
                    } else if resp.clicked() {
                        select = Some(i);
                    }
                    if resp.middle_clicked() {
                        close = Some(i);
                    }
                    if resp.double_clicked() {
                        // double-click toggles nothing; reserved
                    }
                    // drag to reorder
                    if resp.drag_started() {
                        self.tab_drag = Some(i);
                    }
                    if resp.dragged() && self.tab_drag == Some(i) {
                        if let Some(px) = ui.input(|inp| inp.pointer.interact_pos()).map(|p| p.x) {
                            if px > rect.right() && i + 1 < n {
                                swap = Some((i, i + 1));
                            } else if px < rect.left() && i > 0 {
                                swap = Some((i, i - 1));
                            }
                        }
                    }
                    if is_active && self.tab_scroll_to_active {
                        resp.scroll_to_me(None);
                    }
                    if resp.hovered() {
                        if let Some(pth) = &self.tabs[i].path {
                            resp.clone().on_hover_text(pth.to_string_lossy());
                        }
                    }
                    resp.context_menu(|ui| {
                        self.active = i;
                        item(ui, cmds, "Close", Cmd::Close);
                        item(ui, cmds, "Close All BUT This", Cmd::CloseOthers);
                        item(ui, cmds, "Close All to the Left", Cmd::CloseLeft);
                        item(ui, cmds, "Close All to the Right", Cmd::CloseRight);
                        ui.separator();
                        item(ui, cmds, "Save", Cmd::Save);
                        item(ui, cmds, "Save As...", Cmd::SaveAs);
                        item(ui, cmds, "Rename...", Cmd::Rename);
                        item(ui, cmds, "Reload from Disk", Cmd::Reload);
                        ui.separator();
                        item(ui, cmds, "Open Containing Folder", Cmd::OpenContainingFolder);
                        item(ui, cmds, "Copy Full Path", Cmd::CopyPath);
                        item(ui, cmds, "Copy Filename", Cmd::CopyFilename);
                        item(ui, cmds, "Read-Only", Cmd::ToggleReadOnly);
                    });
                }
                if ui.add(egui::Button::new("+").min_size(Vec2::new(24.0, 22.0))).on_hover_text("New").clicked() {
                    cmds.push(Cmd::New);
                }
            });
        });
        self.tab_scroll_to_active = false;
        if !ui.input(|i| i.pointer.primary_down()) {
            self.tab_drag = None;
        }
        if let Some((a, b)) = swap {
            self.tabs.swap(a, b);
            if self.active == a {
                self.active = b;
            } else if self.active == b {
                self.active = a;
            }
            self.tab_drag = Some(b);
        }
        if let Some(i) = select {
            self.select_tab(i);
        }
        if let Some(i) = close {
            self.active = i;
            cmds.push(Cmd::Close);
        }
    }

    pub fn status_bar(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
        let (lang_name, len, lines, ln, col, pos, sel_chars, sel_lines, eol, enc, overwrite, read_only) = {
            let d = self.doc_ref();
            let head = d.caret();
            let (l, c) = d.linecol(head);
            let text = d.line_text(l);
            let dcol = util::display_col(&text, c, d.opts.tab_width) + 1;
            let sel_chars: usize = d.sels.iter().map(|s| s.end() - s.start()).sum();
            let sel_lines = if sel_chars > 0 {
                d.sels.iter().filter(|s| !s.is_empty()).map(|s| d.line_of(s.end()) - d.line_of(s.start()) + 1).sum::<usize>()
            } else {
                0
            };
            (lang::name(d.lang), d.len_chars(), d.len_lines(), l + 1, dcol, head + 1, sel_chars, sel_lines, d.eol, d.enc, d.overwrite, d.read_only)
        };
        let recording = self.recording;
        let msg = self.status_msg.as_ref().map(|(m, _)| m.clone());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.label(lang_name);
            ui.separator();
            ui.label(format!("length : {len}   lines : {lines}"));
            ui.separator();
            let mut s = format!("Ln : {ln}   Col : {col}   Pos : {pos}");
            if sel_chars > 0 {
                s.push_str(&format!("   Sel : {sel_chars} | {sel_lines}"));
            }
            if self.doc_ref().sels.len() > 1 {
                s.push_str(&format!("   ({} carets)", self.doc_ref().sels.len()));
            }
            ui.label(s);
            if recording {
                ui.separator();
                ui.label(RichText::new("● REC").color(Color32::from_rgb(0xe0, 0x30, 0x30)).strong());
            }
            if let Some(m) = msg {
                ui.separator();
                ui.label(RichText::new(m).italics());
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(if overwrite { "OVR" } else { "INS" });
                if read_only {
                    ui.label("R/O");
                }
                ui.separator();
                ui.menu_button(enc.label(), |ui| {
                    for e in [Enc::ansi(), Enc::Utf8, Enc::Utf8Bom, Enc::Utf16Be, Enc::Utf16Le] {
                        radio(ui, cmds, &e.label(), enc == e, Cmd::SetEncoding(e.label()));
                    }
                });
                ui.separator();
                ui.menu_button(eol.label(), |ui| {
                    for e in [Eol::CrLf, Eol::Lf, Eol::Cr] {
                        radio(ui, cmds, e.label(), eol == e, Cmd::ConvertEol(e));
                    }
                });
            });
        });
    }
}
