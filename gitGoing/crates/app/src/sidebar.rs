//! Left sidebar: branches, remotes and tags with a filter box.

use crate::filetree::{self, Node};
use crate::icons::{self, Icon};
use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, Vec2};
use gitgoing_core::history::{RefKind, RepoInfo};
use gitgoing_core::stash::StashEntry;
use gitgoing_core::Oid;
use std::collections::BTreeMap;

pub enum SidebarAction {
    Checkout(String),
    CheckoutRemote(String),
    Delete(String),
    Merge(String),
    RebaseOnto(String),
    DeleteTag(String),
    AddRemote,
    EditRemote(String),
    RenameRemote(String),
    RemoveRemote(String),
    FetchRemote(String),
    DeleteRemoteBranch(String),
    SetUpstream(String),
    PushTag(String),
    DeleteRemoteTag(String),
    /// Scroll the graph to this commit.
    Goto(Oid),
    ShowStash(Oid),
    ApplyStash(usize),
    PopStash(usize),
    DropStash(usize),
}

#[derive(Default)]
pub struct Sidebar {
    filter: String,
    /// (name, is_current, tip)
    local: Vec<(String, bool, Oid)>,
    /// remote name -> (branch name, tip)
    remotes: BTreeMap<String, Vec<(String, Oid)>>,
    tags: Vec<(String, Oid)>,
    remote_urls: BTreeMap<String, String>,
    /// Scroll the local list to the current branch on the next frame.
    locate: bool,
    stashes: Vec<StashEntry>,
    /// Local, remote, tags, stashes.
    sections: [SectionState; 4],
}

/// Whether a section is expanded and how tall its scrolling list is.
struct SectionState {
    open: bool,
    height: f32,
}

impl Default for SectionState {
    fn default() -> Self {
        SectionState { open: true, height: 200.0 }
    }
}

impl Sidebar {
    pub fn from_info(info: &RepoInfo) -> Self {
        let mut s = Sidebar { stashes: info.stashes.clone(), ..Default::default() };
        for r in &info.remotes {
            // Show remotes even before they have any branches.
            s.remotes.entry(r.name.clone()).or_default();
            s.remote_urls.insert(r.name.clone(), r.url.clone());
        }
        for (oid, labels) in &info.refs {
            for l in labels {
                match l.kind {
                    RefKind::Head => s.local.push((l.name.clone(), true, *oid)),
                    RefKind::Local => s.local.push((l.name.clone(), false, *oid)),
                    RefKind::Remote => {
                        let (remote, branch) = l.name.split_once('/').unwrap_or(("", &l.name));
                        s.remotes.entry(remote.to_owned()).or_default().push((branch.to_owned(), *oid));
                    }
                    RefKind::Tag => s.tags.push((l.name.clone(), *oid)),
                }
            }
        }
        s.local.sort_by(|a, b| a.0.cmp(&b.0));
        s.local.dedup_by(|a, b| a.0 == b.0);
        s.tags.sort_by(|a, b| a.0.cmp(&b.0));
        s.tags.dedup_by(|a, b| a.0 == b.0);
        for v in s.remotes.values_mut() {
            v.sort_by(|a, b| a.0.cmp(&b.0));
            v.dedup_by(|a, b| a.0 == b.0);
        }
        s
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<SidebarAction> {
        let mut act = None;
        // Never wrap names; cut off the right-hand side instead.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        // Visible scrollbars (the default ones only appear on hover).
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        ui.add_space(4.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("Filter")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        let locate = std::mem::take(&mut self.locate);
        let mut locate_clicked = false;
        let q = self.filter.to_lowercase();
        let hit = |n: &str| q.is_empty() || n.to_lowercase().contains(&q);

        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let force_open = !q.is_empty();
            let local: Vec<_> = self.local.iter().filter(|(n, _, _)| hit(n)).collect();
            locate_clicked = section(ui, "LOCAL", local.len(), self.local.len(), HeaderButton::Locate, &mut self.sections[0], |ui| {
                let items: Vec<(&str, usize)> = local.iter().enumerate().map(|(i, e)| (e.0.as_str(), i)).collect();
                let nodes = filetree::build(&items, false, false);
                tree(ui, "local", &nodes, 0, "", force_open, &mut |ui, i, name, depth| {
                    let (full, current, oid) = local[i];
                    let r = row(ui, depth, Icon::Branch, name, *current)
                        .on_hover_text("Click to show in the graph; double-click to check out");
                    if *current && locate {
                        r.scroll_to_me(Some(egui::Align::Center));
                    }
                    if r.clicked() {
                        act = Some(SidebarAction::Goto(*oid));
                    }
                    if r.double_clicked() && !*current {
                        act = Some(SidebarAction::Checkout(full.clone()));
                    }
                    r.context_menu(|ui| {
                        if ui.add_enabled(!*current, egui::Button::new("Check out")).clicked() {
                            act = Some(SidebarAction::Checkout(full.clone()));
                            ui.close_menu();
                        }
                        if ui.add_enabled(!*current, egui::Button::new("Merge into current branch")).clicked() {
                            act = Some(SidebarAction::Merge(full.clone()));
                            ui.close_menu();
                        }
                        if ui.add_enabled(!*current, egui::Button::new("Rebase current branch onto this")).clicked() {
                            act = Some(SidebarAction::RebaseOnto(full.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Set upstream…").clicked() {
                            act = Some(SidebarAction::SetUpstream(full.clone()));
                            ui.close_menu();
                        }
                        if ui.add_enabled(!*current, egui::Button::new("Delete…")).clicked() {
                            act = Some(SidebarAction::Delete(full.clone()));
                            ui.close_menu();
                        }
                    });
                });
            });

            let total: usize = self.remotes.values().map(Vec::len).sum();
            let shown: usize = self.remotes.values().map(|v| v.iter().filter(|(n, _)| hit(n)).count()).sum();
            if section(ui, "REMOTE", shown, total, HeaderButton::Add, &mut self.sections[1], |ui| {
                for (remote, branches) in &self.remotes {
                    let bs: Vec<_> = branches.iter().filter(|(n, _)| hit(n)).collect();
                    if bs.is_empty() && !q.is_empty() {
                        continue;
                    }
                    let items: Vec<(&str, usize)> = bs.iter().enumerate().map(|(i, e)| (e.0.as_str(), i)).collect();
                    let nodes = filetree::build(&items, false, false);
                    let salt = format!("remote:{remote}");
                    // The remote itself is the top-level folder.
                    let id = egui::Id::new((&salt, "/"));
                    let mut open = force_open || ui.data_mut(|d| *d.get_persisted_mut_or(id, true));
                    let rr = row(ui, 0, Icon::Folder, remote, false);
                    let rr = match self.remote_urls.get(remote) {
                        Some(url) => rr.on_hover_text(url),
                        None => rr,
                    };
                    if rr.clicked() {
                        open = !open;
                        ui.data_mut(|d| d.insert_persisted(id, open));
                    }
                    rr.context_menu(|ui| {
                        if ui.button("Fetch").clicked() {
                            act = Some(SidebarAction::FetchRemote(remote.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Edit URL…").clicked() {
                            act = Some(SidebarAction::EditRemote(remote.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Rename…").clicked() {
                            act = Some(SidebarAction::RenameRemote(remote.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Remove…").clicked() {
                            act = Some(SidebarAction::RemoveRemote(remote.clone()));
                            ui.close_menu();
                        }
                    });
                    if !open {
                        continue;
                    }
                    tree(ui, &salt, &nodes, 1, "", force_open, &mut |ui, i, name, depth| {
                        let (b, oid) = bs[i];
                        let full = format!("{remote}/{b}");
                        let r = row(ui, depth, Icon::Branch, name, false)
                            .on_hover_text("Click to show in the graph; double-click to check out as a local branch");
                        if r.clicked() {
                            act = Some(SidebarAction::Goto(*oid));
                        }
                        if r.double_clicked() {
                            act = Some(SidebarAction::CheckoutRemote(full.clone()));
                        }
                        r.context_menu(|ui| {
                            if ui.button("Check out as local branch").clicked() {
                                act = Some(SidebarAction::CheckoutRemote(full.clone()));
                                ui.close_menu();
                            }
                            if ui.button("Merge into current branch").clicked() {
                                act = Some(SidebarAction::Merge(full.clone()));
                                ui.close_menu();
                            }
                            if ui.button("Rebase current branch onto this").clicked() {
                                act = Some(SidebarAction::RebaseOnto(full.clone()));
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Delete branch on remote…").clicked() {
                                act = Some(SidebarAction::DeleteRemoteBranch(full.clone()));
                                ui.close_menu();
                            }
                        });
                    });
                }
            }) {
                act = Some(SidebarAction::AddRemote);
            }

            let tags: Vec<_> = self.tags.iter().filter(|(n, _)| hit(n)).collect();
            section(ui, "TAGS", tags.len(), self.tags.len(), HeaderButton::None, &mut self.sections[2], |ui| {
                let items: Vec<(&str, usize)> = tags.iter().enumerate().map(|(i, e)| (e.0.as_str(), i)).collect();
                let nodes = filetree::build(&items, false, false);
                tree(ui, "tags", &nodes, 0, "", force_open, &mut |ui, i, name, depth| {
                    let (full, oid) = tags[i];
                    let r = row(ui, depth, Icon::Tag, name, false);
                    if r.clicked() {
                        act = Some(SidebarAction::Goto(*oid));
                    }
                    r.context_menu(|ui| {
                        if ui.button("Push tag to remote").clicked() {
                            act = Some(SidebarAction::PushTag(full.clone()));
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Delete tag…").clicked() {
                            act = Some(SidebarAction::DeleteTag(full.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Delete tag on remote…").clicked() {
                            act = Some(SidebarAction::DeleteRemoteTag(full.clone()));
                            ui.close_menu();
                        }
                    });
                });
            });

            let stashes: Vec<_> = self.stashes.iter().filter(|s| hit(&s.message)).collect();
            section(ui, "STASHES", stashes.len(), self.stashes.len(), HeaderButton::None, &mut self.sections[3], |ui| {
                for s in stashes {
                    let r = row(ui, 0, Icon::None, &format!("{}: {}", s.index, s.message), false)
                        .on_hover_text(&s.message);
                    if r.clicked() {
                        act = Some(SidebarAction::ShowStash(s.id));
                    }
                    r.context_menu(|ui| {
                        if ui.button("Apply").clicked() {
                            act = Some(SidebarAction::ApplyStash(s.index));
                            ui.close_menu();
                        }
                        if ui.button("Pop").clicked() {
                            act = Some(SidebarAction::PopStash(s.index));
                            ui.close_menu();
                        }
                        if ui.button("Delete…").clicked() {
                            act = Some(SidebarAction::DropStash(s.index));
                            ui.close_menu();
                        }
                    });
                }
            });

            ui.add_space(6.0);
            ui.add_enabled(false, egui::Label::new("SUBMODULES"));
        });
        if locate_clicked {
            // Make sure the current branch isn't filtered out, then scroll to
            // it on the next frame (once the section is open and laid out).
            self.filter.clear();
            self.sections[0].open = true;
            self.locate = true;
            ui.ctx().request_repaint();
        }
        act
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HeaderButton {
    None,
    Locate,
    Add,
}

/// A collapsible section whose list scrolls vertically and can be resized by
/// dragging the separator below it. With `locate`, the header also gets a
/// "scroll to current" button; returns true when it was clicked.
fn section(
    ui: &mut egui::Ui,
    title: &str,
    shown: usize,
    total: usize,
    button: HeaderButton,
    state: &mut SectionState,
    body: impl FnOnce(&mut egui::Ui),
) -> bool {
    let count = if shown == total { total.to_string() } else { format!("{shown}/{total}") };
    let mut located = false;

    // Header: small toggle arrow, title, locate button, right-aligned count.
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    let strong = ui.visuals().strong_text_color();
    let weak = ui.visuals().weak_text_color();
    let c = Pos2::new(rect.left() + 9.0, rect.center().y);
    let arrow = if state.open {
        vec![c + Vec2::new(-4.0, -2.0), c + Vec2::new(4.0, -2.0), c + Vec2::new(0.0, 3.0)]
    } else {
        vec![c + Vec2::new(-2.0, -4.0), c + Vec2::new(-2.0, 4.0), c + Vec2::new(3.0, 0.0)]
    };
    ui.painter().add(egui::Shape::convex_polygon(arrow, weak, egui::Stroke::NONE));
    let p = ui.painter().clone();
    p.text(Pos2::new(rect.left() + 20.0, rect.center().y), egui::Align2::LEFT_CENTER, title, font.clone(), strong);
    p.text(Pos2::new(rect.right() - 6.0, rect.center().y), egui::Align2::RIGHT_CENTER, count, font, weak);
    if resp.clicked() {
        state.open = !state.open;
    }
    if button != HeaderButton::None {
        let btn = Rect::from_center_size(Pos2::new(rect.right() - 62.0, rect.center().y), Vec2::splat(18.0));
        let r = ui
            .interact(btn, ui.id().with(("header_button", title)), Sense::click())
            .on_hover_text(if button == HeaderButton::Locate { "Scroll to the current branch" } else { "Add a remote" });
        let col = if r.hovered() { strong } else { weak };
        if button == HeaderButton::Locate {
            p.circle_stroke(btn.center(), 5.5, egui::Stroke::new(1.3_f32, col));
            p.circle_filled(btn.center(), 2.0, col);
        } else {
            let c = btn.center();
            let st = egui::Stroke::new(1.6_f32, col);
            p.line_segment([c + Vec2::new(-5.0, 0.0), c + Vec2::new(5.0, 0.0)], st);
            p.line_segment([c + Vec2::new(0.0, -5.0), c + Vec2::new(0.0, 5.0)], st);
        }
        located = r.clicked();
    }
    if !state.open {
        return located;
    }

    let list = egui::ScrollArea::vertical()
        .id_salt(("section", title))
        .max_height(state.height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            body(ui);
        });

    // Separator handle: drag to change this section's height.
    let (grip, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 7.0), Sense::drag());
    let active = resp.hovered() || resp.dragged();
    if active {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    let stroke = if active {
        ui.visuals().widgets.active.fg_stroke
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke
    };
    ui.painter().hline(grip.x_range(), grip.center().y, stroke);
    if resp.dragged() {
        // Base on the height actually shown, which is less than `height` when
        // the list is short.
        state.height = (list.inner_rect.height() + resp.drag_delta().y).clamp(40.0, 2000.0);
    }
    located
}

const ROW_H: f32 = 24.0;
const INDENT: f32 = 16.0;

/// One full-width row of a sidebar tree. The current branch gets a green
/// highlight and a green check box.
fn row(ui: &mut egui::Ui, depth: usize, icon: Icon, text: &str, current: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    let p = ui.painter_at(rect);
    let visuals = ui.visuals();
    if current {
        p.rect_filled(rect, 0.0, Color32::from_rgb(0x2f, 0x4f, 0x3a));
    } else if resp.hovered() {
        p.rect_filled(rect, 0.0, visuals.widgets.hovered.weak_bg_fill);
    }
    let cy = rect.center().y;
    if current {
        let b = Rect::from_center_size(Pos2::new(rect.left() + 14.0, cy), Vec2::splat(15.0));
        p.rect_filled(b, 3.0, Color32::from_rgb(0x4c, 0xaf, 0x50));
        let w = Stroke::new(2.0_f32, Color32::WHITE);
        let m = b.center();
        p.line_segment([m + Vec2::new(-3.5, 0.0), m + Vec2::new(-1.0, 3.0)], w);
        p.line_segment([m + Vec2::new(-1.0, 3.0), m + Vec2::new(3.5, -3.0)], w);
    }
    let x = rect.left() + 34.0 + depth as f32 * INDENT;
    let color = if current { visuals.strong_text_color() } else { visuals.text_color() };
    let mut text_x = x;
    if !matches!(icon, Icon::None) {
        icons::draw(&p, Pos2::new(x + 7.0, cy), icon, 1.0, visuals.weak_text_color());
        text_x = x + 22.0;
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    // Names are cut off at the right edge, never wrapped.
    p.text(Pos2::new(text_x, cy), egui::Align2::LEFT_CENTER, text, font, color);
    resp
}

/// Render `nodes` as folders (collapsible rows) and leaves.
fn tree(
    ui: &mut egui::Ui,
    salt: &str,
    nodes: &[Node],
    depth: usize,
    prefix: &str,
    force_open: bool,
    leaf: &mut dyn FnMut(&mut egui::Ui, usize, &str, usize),
) {
    for n in nodes {
        match n {
            Node::File(name, id) => leaf(ui, *id, name, depth),
            Node::Dir(name, children) => {
                let full = format!("{prefix}{name}/");
                let id = egui::Id::new((salt, &full));
                let mut open = force_open || ui.data_mut(|d| *d.get_persisted_mut_or(id, true));
                if row(ui, depth, Icon::Folder, name, false).clicked() {
                    open = !open;
                    ui.data_mut(|d| d.insert_persisted(id, open));
                }
                if open {
                    tree(ui, salt, children, depth + 1, &full, force_open, leaf);
                }
            }
        }
    }
}
