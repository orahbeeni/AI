//! Left sidebar: branches, remotes and tags with a filter box.

use crate::filetree::{self, Node};
use crate::icons::{self, Icon};
use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, Vec2};
use gitgoing_core::history::{RefKind, RepoInfo};
use gitgoing_core::changesets::ChangeSet;
use gitgoing_core::stash::StashEntry;
use gitgoing_core::Oid;
use std::collections::{BTreeMap, BTreeSet};

pub enum SidebarAction {
    Checkout(String),
    CheckoutRemote(String),
    Delete(String),
    Merge(String),
    RebaseOnto(String),
    /// Compare the checked-out branch with this one (full reference name, short name).
    Compare(String, String),
    DeleteTag(String),
    AddRemote,
    EditRemote(String),
    RenameRemote(String),
    RemoveRemote(String),
    FetchRemote(String),
    /// Fetch one branch: (name, is_local).
    FetchBranch(String, bool),
    /// Pull this local branch alone (short name), checked out or not.
    PullBranch(String),
    DeleteRemoteBranch(String),
    SetUpstream(String),
    PushTag(String),
    DeleteRemoteTag(String),
    /// Pin (or unpin) a local branch, by full reference name.
    SetPinned(String, bool),
    /// Move a pinned branch up (-1) or down (+1) in the pinned list.
    MovePinned(String, i32),
    /// Drag a pinned branch onto another: move the first (full reference name) to the second's place.
    ReorderPinned(String, String),
    /// Add or edit the local comment of a local branch (short name).
    EditComment(String),
    /// Hide (or show) these branches/tags (full reference names) in the graph.
    SetHidden(Vec<String>, bool),
    /// Show only this branch or tag (full reference name).
    Solo(String),
    /// Show every branch and tag again.
    ShowAll,
    /// Scroll the graph to this commit.
    Goto(Oid),
    ShowStash(Oid),
    ApplyStash(usize),
    PopStash(usize),
    DropStash(usize),
    /// Save the current changes as a named set (they stay in the working folder).
    SaveChanges,
    ShowChangeSet(Oid),
    /// Apply a saved set (reference name).
    ApplyChangeSet(String),
    /// Replace a saved set's contents with the current changes (display name).
    UpdateChangeSet(String),
    RenameChangeSet(String, String),
    DeleteChangeSet(String, String),
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
    /// The local branch being checked out right now (its row shows a spinner).
    pub busy_branch: Option<String>,
    stashes: Vec<StashEntry>,
    change_sets: Vec<ChangeSet>,
    /// Local branch -> number of files it would get from a pull.
    unpulled: BTreeMap<String, (usize, usize)>,
    /// Local, remote, tags, stashes, pinned, saved changes.
    sections: [SectionState; 6],
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
        let mut s = Sidebar {
            stashes: info.stashes.clone(),
            change_sets: info.change_sets.clone(),
            unpulled: info.unpulled.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            ..Default::default()
        };
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

    /// Keep the filter text and section layout of `old` when the lists are rebuilt
    /// after a reload, so toggling a branch doesn't reset the sidebar.
    pub fn adopt_ui(&mut self, old: Sidebar) {
        self.filter = old.filter;
        self.sections = old.sections;
    }

    /// `hidden` holds the full names of branches and tags hidden from the graph;
    /// `head` is the short name of the checked-out branch, which can't be hidden.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        hidden: &BTreeSet<String>,
        pinned: &[String],
        comments: &BTreeMap<String, String>,
        head: Option<&str>,
    ) -> Option<SidebarAction> {
        let mut act = None;
        // Never wrap names; cut off the right-hand side instead.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        // Visible scrollbars (the default ones only appear on hover).
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        ui.add_space(4.0);
        // "Viewing X/Y": how many branches and tags are shown in the graph.
        let all_refs = self.local.iter().map(|(n, _, _)| format!("refs/heads/{n}"))
            .chain(self.remotes.iter().flat_map(|(r, bs)| bs.iter().map(move |(b, _)| format!("refs/remotes/{r}/{b}"))))
            .chain(self.tags.iter().map(|(n, _)| format!("refs/tags/{n}")));
        let (total, hidden_n) = all_refs.fold((0usize, 0usize), |(t, h), f| (t + 1, h + hidden.contains(&f) as usize));
        ui.horizontal(|ui| {
            ui.label(format!("Viewing {}/{}", total - hidden_n, total));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(hidden_n > 0, egui::Button::new("Show all").small()).clicked() {
                    act = Some(SidebarAction::ShowAll);
                }
            });
        });
        ui.add_space(2.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("Filter")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        let head_full = head.map(|h| format!("refs/heads/{h}"));
        let _ = &head_full;
        let locate = std::mem::take(&mut self.locate);
        let mut locate_clicked = false;
        let q = self.filter.to_lowercase();
        let hit = |n: &str| q.is_empty() || n.to_lowercase().contains(&q);

        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let force_open = !q.is_empty();
            let bc = BranchCtx { hidden, pinned, comments, head_full: head_full.as_deref(), unpulled: &self.unpulled, busy: self.busy_branch.as_deref() };

            // PINNED: the branches the user keeps an eye on, in the order they were pinned.
            let pinned_rows: Vec<_> = pinned
                .iter()
                .filter_map(|full| {
                    let name = full.strip_prefix("refs/heads/")?;
                    self.local.iter().find(|(n, _, _)| n == name)
                })
                .filter(|(n, _, _)| hit(n))
                .collect();
            // The checked-out branch gets an honorary pin, even if it isn't pinned.
            let honorary = self
                .local
                .iter()
                .find(|(n, current, _)| *current && !pinned.contains(&format!("refs/heads/{n}")) && hit(n));
            section(ui, "PINNED", pinned_rows.len() + honorary.is_some() as usize, pinned.len(), HeaderButton::None, &mut self.sections[4], |ui| {
                if let Some((name, current, oid)) = honorary {
                    local_branch_row(ui, &bc, &mut act, 0, name, name, *current, *oid, false, None, true);
                }
                if pinned.is_empty() && honorary.is_none() {
                    ui.add_space(2.0);
                    ui.weak("  Hover a branch in LOCAL and click its pin to keep it here.");
                }
                let n = pinned_rows.len();
                for (i, (name, current, oid)) in pinned_rows.iter().enumerate() {
                    local_branch_row(ui, &bc, &mut act, 0, name, name, *current, *oid, false, Some((i, n)), false);
                }
            });

            let local: Vec<_> = self.local.iter().filter(|(n, _, _)| hit(n)).collect();
            // The current branch may sit inside collapsed folders (`feature/x`): open them,
            // or there is no row to scroll to.
            if locate {
                if let Some((name, _, _)) = self.local.iter().find(|(_, current, _)| *current) {
                    expand_ancestors(ui, "local", name);
                }
            }
            locate_clicked = section(ui, "LOCAL", local.len(), self.local.len(), HeaderButton::Locate, &mut self.sections[0], |ui| {
                let items: Vec<(&str, usize)> = local.iter().enumerate().map(|(i, e)| (e.0.as_str(), i)).collect();
                let nodes = filetree::build(&items, false, false);
                let mut toggles = Vec::new();
                // The checked-out branch always counts as shown, so a folder holding it can still be toggled.
                let is_hidden = |i: usize| !local[i].1 && hidden.contains(&format!("refs/heads/{}", local[i].0));
                tree(ui, "local", &nodes, 0, "", force_open, &is_hidden, &mut toggles, &mut |ui, i, name, depth| {
                    let (full, current, oid) = local[i];
                    local_branch_row(ui, &bc, &mut act, depth, name, full, *current, *oid, locate, None, false);
                });
                for (ids, hide) in toggles {
                    act = Some(SidebarAction::SetHidden(ids.iter().map(|&i| format!("refs/heads/{}", local[i].0)).collect(), hide));
                }
            });

            let total: usize = self.remotes.values().map(Vec::len).sum();
            let shown: usize = self.remotes.values().map(|v| v.iter().filter(|(n, _)| hit(n)).count()).sum();
            if section(ui, "REMOTE", shown, total, HeaderButton::Add("Add a remote"), &mut self.sections[1], |ui| {
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
                    let all_refs: Vec<String> = bs.iter().map(|(b, _)| format!("refs/remotes/{remote}/{b}")).collect();
                    let all_hidden = !all_refs.is_empty() && all_refs.iter().all(|f| hidden.contains(f));
                    let (rr, eye) = row_eye(ui, 0, Icon::Folder, remote, false, (!all_refs.is_empty()).then_some(all_hidden));
                    if eye {
                        act = Some(SidebarAction::SetHidden(all_refs, !all_hidden));
                        continue;
                    }
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
                    let mut toggles = Vec::new();
                    let is_hidden = |i: usize| hidden.contains(&format!("refs/remotes/{remote}/{}", bs[i].0));
                    tree(ui, &salt, &nodes, 1, "", force_open, &is_hidden, &mut toggles, &mut |ui, i, name, depth| {
                        let (b, oid) = bs[i];
                        let full = format!("{remote}/{b}");
                        let full_ref = format!("refs/remotes/{full}");
                        let hid = hidden.contains(&full_ref);
                        let (r, eye) = row_eye(ui, depth, Icon::Branch, name, false, Some(hid));
                        if eye {
                            act = Some(SidebarAction::SetHidden(vec![full_ref], !hid));
                            return;
                        }
                        let r = r.on_hover_text("Click to show in the graph; double-click to check out as a local branch");
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
                            if ui.button("Fetch latest").clicked() {
                                act = Some(SidebarAction::FetchBranch(full.clone(), false));
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
                            if ui.button("Compare with current branch…").clicked() {
                                act = Some(SidebarAction::Compare(full_ref.clone(), full.clone()));
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Delete branch on remote…").clicked() {
                                act = Some(SidebarAction::DeleteRemoteBranch(full.clone()));
                                ui.close_menu();
                            }
                            ui.separator();
                            visibility_menu(ui, &mut act, &format!("refs/remotes/{full}"), hid, false);
                        });
                    });
                    for (ids, hide) in toggles {
                        act = Some(SidebarAction::SetHidden(
                            ids.iter().map(|&i| format!("refs/remotes/{remote}/{}", bs[i].0)).collect(),
                            hide,
                        ));
                    }
                }
            }) {
                act = Some(SidebarAction::AddRemote);
            }

            let tags: Vec<_> = self.tags.iter().filter(|(n, _)| hit(n)).collect();
            section(ui, "TAGS", tags.len(), self.tags.len(), HeaderButton::None, &mut self.sections[2], |ui| {
                let items: Vec<(&str, usize)> = tags.iter().enumerate().map(|(i, e)| (e.0.as_str(), i)).collect();
                let nodes = filetree::build(&items, false, false);
                let mut toggles = Vec::new();
                let is_hidden = |i: usize| hidden.contains(&format!("refs/tags/{}", tags[i].0));
                tree(ui, "tags", &nodes, 0, "", force_open, &is_hidden, &mut toggles, &mut |ui, i, name, depth| {
                    let (full, oid) = tags[i];
                    let full_ref = format!("refs/tags/{full}");
                    let hid = hidden.contains(&full_ref);
                    let (r, eye) = row_eye(ui, depth, Icon::Tag, name, false, Some(hid));
                    if eye {
                        act = Some(SidebarAction::SetHidden(vec![full_ref], !hid));
                        return;
                    }
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
                        ui.separator();
                        visibility_menu(ui, &mut act, &format!("refs/tags/{full}"), hid, false);
                    });
                });
                for (ids, hide) in toggles {
                    act = Some(SidebarAction::SetHidden(ids.iter().map(|&i| format!("refs/tags/{}", tags[i].0)).collect(), hide));
                }
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

            // SAVED CHANGES: named snapshots of uncommitted changes, re-applied at will.
            let sets: Vec<_> = self.change_sets.iter().filter(|s| hit(&s.name)).collect();
            let add_tip = "Save the current changes as a named set (they stay in your working folder)";
            if section(ui, "SAVED CHANGES", sets.len(), self.change_sets.len(), HeaderButton::Add(add_tip), &mut self.sections[5], |ui| {
                if self.change_sets.is_empty() {
                    ui.add_space(2.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(
                                "  Click + to save your current changes as a named set you can re-apply whenever you like. \
                                 Your changes stay where they are.",
                            )
                            .weak(),
                        )
                        .wrap(),
                    );
                }
                let now = crate::timefmt::now();
                for s in sets {
                    let files = if s.files == 1 { "1 file".to_owned() } else { format!("{} files", s.files) };
                    let on = if s.branch.is_empty() { String::new() } else { format!(" on {}", s.branch) };
                    let tip = format!(
                        "{files}, saved{on} {}\nClick to see the changes, double-click to apply them",
                        crate::timefmt::relative(s.time, now)
                    );
                    let r = row(ui, 0, Icon::Stash, &s.name, false).on_hover_text(tip);
                    if r.clicked() {
                        act = Some(SidebarAction::ShowChangeSet(s.id));
                    }
                    if r.double_clicked() {
                        act = Some(SidebarAction::ApplyChangeSet(s.refname.clone()));
                    }
                    r.context_menu(|ui| {
                        if ui.button("Apply to working changes").clicked() {
                            act = Some(SidebarAction::ApplyChangeSet(s.refname.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Update with current changes…").clicked() {
                            act = Some(SidebarAction::UpdateChangeSet(s.name.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Rename…").clicked() {
                            act = Some(SidebarAction::RenameChangeSet(s.refname.clone(), s.name.clone()));
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Delete…").clicked() {
                            act = Some(SidebarAction::DeleteChangeSet(s.refname.clone(), s.name.clone()));
                            ui.close_menu();
                        }
                    });
                }
            }) {
                act = Some(SidebarAction::SaveChanges);
            }

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
    /// A + button with this tooltip.
    Add(&'static str),
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
            .on_hover_text(match button {
                HeaderButton::Add(tip) => tip,
                _ => "Scroll to the current branch",
            });
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
    row_eye(ui, depth, icon, text, current, None).0
}

/// Like [`row`], with an eye button at the right edge when `eye` is `Some`
/// (`Some(true)` means hidden from the graph). Returns the row's response and
/// whether the eye was clicked.
fn row_eye(
    ui: &mut egui::Ui,
    depth: usize,
    icon: Icon,
    text: &str,
    current: bool,
    eye: Option<bool>,
) -> (Response, bool) {
    let (resp, clicks) = row_full(ui, depth, icon, text, &[], current, eye, None, false);
    (resp, clicks.eye)
}

/// Which of a row's small buttons were clicked.
#[derive(Default, Clone, Copy)]
struct RowClicks {
    eye: bool,
    pin: bool,
}

/// A tree row with up to two small buttons at its right edge: an eye (hide the
/// branch from the graph; `Some(true)` = hidden) and a pin (`Some(true)` = pinned).
/// The eye shows on hover and always when hidden; the pin shows on hover and
/// always when pinned.
#[allow(clippy::too_many_arguments)]
fn row_full(
    ui: &mut egui::Ui,
    depth: usize,
    icon: Icon,
    text: &str,
    badge: &[(usize, bool)],
    current: bool,
    eye: Option<bool>,
    pin: Option<bool>,
    drag: bool,
) -> (Response, RowClicks) {
    let sense = if drag { Sense::click_and_drag() } else { Sense::click() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), sense);
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
    let hidden = eye == Some(true);
    let x = rect.left() + 34.0 + depth as f32 * INDENT;
    let mut color = if current { visuals.strong_text_color() } else { visuals.text_color() };
    let mut icon_color = visuals.weak_text_color();
    if hidden {
        color = color.gamma_multiply(0.45);
        icon_color = icon_color.gamma_multiply(0.5);
    }
    let mut text_x = x;
    if !matches!(icon, Icon::None) {
        icons::draw(&p, Pos2::new(x + 7.0, cy), icon, 1.0, icon_color);
        text_x = x + 22.0;
    }
    let font = egui::TextStyle::Body.resolve(ui.style());

    let mut clicks = RowClicks::default();
    let mut text_clip = rect;
    let mut right = rect.right() - 16.0;
    let pointer = resp.hover_pos();
    let click = resp.clicked().then(|| resp.interact_pointer_pos()).flatten();
    for (kind, state) in [(0, eye), (1, pin)] {
        let Some(on) = state else { continue };
        let button = Rect::from_center_size(Pos2::new(right, cy), Vec2::splat(20.0));
        text_clip.max.x = button.left() - 2.0;
        right -= 22.0;
        // Shown while hovering, and always when on, so the state is easy to spot.
        if !(resp.hovered() || on) {
            continue;
        }
        let over = pointer.is_some_and(|p| button.contains(p));
        let col = if over {
            visuals.strong_text_color()
        } else if kind == 1 && on {
            Color32::from_rgb(0xe5, 0xc0, 0x7b)
        } else {
            visuals.weak_text_color()
        };
        let icon = match (kind, on) {
            (0, true) => Icon::EyeOff,
            (0, false) => Icon::Eye,
            (_, true) => Icon::PinFilled,
            (_, false) => Icon::Pin,
        };
        icons::draw(&p, button.center(), icon, 1.0, col);
        if click.is_some_and(|p| button.contains(p)) {
            if kind == 0 {
                clicks.eye = true;
            } else {
                clicks.pin = true;
            }
        }
    }
    // Push/pull counts sit at the right edge, left of the buttons. The arrows are
    // drawn rather than typed: not every font has them (they show as squares).
    if !badge.is_empty() {
        let mut x = right + 10.0;
        for &(n, up) in badge.iter().rev() {
            // Full-strength text: the dimmed colour is unreadable on the green current-branch row.
            let col = if current { visuals.strong_text_color() } else { visuals.text_color() };
            // A solid block arrow as one mesh: head and stem share vertices and the same
            // (non-feathered) edges, so they cannot drift apart. Snapped to whole pixels.
            let ppp = ui.ctx().pixels_per_point();
            let snap = |v: f32| (v * ppp).round() / ppp;
            let (ax, ay, d) = (snap(x - 4.0), snap(cy), if up { -1.0_f32 } else { 1.0 });
            let w = snap(3.5).max(1.0 / ppp);
            let sw = snap(1.0).max(1.0 / ppp);
            let (tip, neck, tail) = (snap(4.5), snap(0.5), snap(4.5));
            let mut m = egui::Mesh::default();
            let mut v = |dx: f32, dy: f32| {
                m.colored_vertex(Pos2::new(ax + dx, ay + d * dy), col);
                m.vertices.len() as u32 - 1
            };
            let (apex, hl, hr) = (v(0.0, tip), v(-w, 0.0), v(w, 0.0));
            let (sl0, sr0, sl1, sr1) = (v(-sw, neck), v(sw, neck), v(-sw, -tail), v(sw, -tail));
            m.add_triangle(apex, hl, hr);
            m.add_triangle(sl0, sr0, sr1);
            m.add_triangle(sl0, sr1, sl1);
            p.add(egui::Shape::mesh(m));
            let r = p.text(Pos2::new(x - 10.0, cy), egui::Align2::RIGHT_CENTER, n.to_string(), font.clone(), col);
            x = r.left() - 6.0;
        }
        text_clip.max.x = text_clip.max.x.min(x);
    }
    // Names are cut off at the right edge (before the buttons), never wrapped.
    ui.painter_at(text_clip).text(Pos2::new(text_x, cy), egui::Align2::LEFT_CENTER, text, font, color);
    (resp, clicks)
}

/// What the branch rows need to know about the repository's state.
struct BranchCtx<'a> {
    hidden: &'a BTreeSet<String>,
    pinned: &'a [String],
    /// Local comments by full reference name.
    comments: &'a BTreeMap<String, String>,
    head_full: Option<&'a str>,
    /// (commits to push, files to pull) per local branch, by short name.
    unpulled: &'a BTreeMap<String, (usize, usize)>,
    busy: Option<&'a str>,
}

/// One local branch row, used by both the PINNED and LOCAL sections.
/// `slot` is `Some((index, count))` in the PINNED list, which adds move up/down.
#[allow(clippy::too_many_arguments)]
fn local_branch_row(
    ui: &mut egui::Ui,
    bc: &BranchCtx,
    act: &mut Option<SidebarAction>,
    depth: usize,
    label: &str,
    full: &str,
    current: bool,
    oid: Oid,
    locate: bool,
    slot: Option<(usize, usize)>,
    honorary: bool,
) {
    let full_ref = format!("refs/heads/{full}");
    let hid = bc.hidden.contains(&full_ref);
    let is_pinned = bc.pinned.contains(&full_ref);
    let _ = bc.head_full;
    // (count, is_push): commits to push first, then files to pull.
    let mut badge: Vec<(usize, bool)> = Vec::new();
    if let Some(&(ahead, files)) = bc.unpulled.get(full) {
        if ahead > 0 {
            badge.push((ahead, true));
        }
        if files > 0 {
            badge.push((files, false));
        }
    }
    let busy = bc.busy == Some(full);
    let icon = if busy { Icon::None } else { Icon::Branch };
    let (r, clicks) = row_full(ui, depth, icon, label, &badge, current, (!current).then_some(hid), Some(is_pinned || honorary), slot.is_some());
    if busy {
        // Checking this branch out: a spinner where its icon was.
        let at = Pos2::new(r.rect.left() + 34.0 + depth as f32 * INDENT + 7.0, r.rect.center().y);
        egui::Spinner::new().size(14.0).paint_at(ui, Rect::from_center_size(at, Vec2::splat(14.0)));
    }
    if clicks.eye {
        *act = Some(SidebarAction::SetHidden(vec![full_ref], !hid));
        return;
    }
    if clicks.pin {
        *act = Some(SidebarAction::SetPinned(full_ref, !is_pinned));
        return;
    }
    if slot.is_some() {
        pin_drag(ui, &r, &full_ref, slot.map_or(0, |(i, _)| i), act);
    }
    let comment = bc.comments.get(&full_ref).filter(|c| !c.trim().is_empty());
    let r = r.on_hover_ui(|ui| {
        ui.strong(full);
        ui.label("Click to show in the graph; double-click to check out");
        if let Some(&(ahead, files)) = bc.unpulled.get(full) {
            if ahead > 0 {
                ui.label(format!("{ahead} commit(s) not pushed yet"));
            }
            if files > 0 {
                ui.label(format!("{files} file(s) changed upstream, not pulled yet"));
            }
        }
        if let Some(c) = comment {
            ui.separator();
            ui.set_max_width(320.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.label(c.as_str());
        }
    });
    if current && locate {
        r.scroll_to_me(Some(egui::Align::Center));
    }
    if r.clicked() {
        *act = Some(SidebarAction::Goto(oid));
    }
    if r.double_clicked() && !current {
        *act = Some(SidebarAction::Checkout(full.to_owned()));
    }
    r.context_menu(|ui| {
        if ui.add_enabled(!current, egui::Button::new("Check out")).clicked() {
            *act = Some(SidebarAction::Checkout(full.to_owned()));
            ui.close_menu();
        }
        if ui.button("Fetch latest").clicked() {
            *act = Some(SidebarAction::FetchBranch(full.to_owned(), true));
            ui.close_menu();
        }
        if ui.button("Pull this branch").clicked() {
            *act = Some(SidebarAction::PullBranch(full.to_owned()));
            ui.close_menu();
        }
        if ui.add_enabled(!current, egui::Button::new("Merge into current branch")).clicked() {
            *act = Some(SidebarAction::Merge(full.to_owned()));
            ui.close_menu();
        }
        if ui.add_enabled(!current, egui::Button::new("Rebase current branch onto this")).clicked() {
            *act = Some(SidebarAction::RebaseOnto(full.to_owned()));
            ui.close_menu();
        }
        if ui.add_enabled(!current, egui::Button::new("Compare with current branch…")).clicked() {
            *act = Some(SidebarAction::Compare(format!("refs/heads/{full}"), full.to_owned()));
            ui.close_menu();
        }
        if ui.button("Set upstream…").clicked() {
            *act = Some(SidebarAction::SetUpstream(full.to_owned()));
            ui.close_menu();
        }
        if ui.add_enabled(!current, egui::Button::new("Delete…")).clicked() {
            *act = Some(SidebarAction::Delete(full.to_owned()));
            ui.close_menu();
        }
        ui.separator();
        if ui.button(if comment.is_some() { "Edit comment…" } else { "Add comment…" }).clicked() {
            *act = Some(SidebarAction::EditComment(full.to_owned()));
            ui.close_menu();
        }
        if ui.button(if is_pinned { "Unpin" } else { "Pin to the top" }).clicked() {
            *act = Some(SidebarAction::SetPinned(format!("refs/heads/{full}"), !is_pinned));
            ui.close_menu();
        }
        if let Some((i, n)) = slot {
            if ui.add_enabled(i > 0, egui::Button::new("Move up")).clicked() {
                *act = Some(SidebarAction::MovePinned(format!("refs/heads/{full}"), -1));
                ui.close_menu();
            }
            if ui.add_enabled(i + 1 < n, egui::Button::new("Move down")).clicked() {
                *act = Some(SidebarAction::MovePinned(format!("refs/heads/{full}"), 1));
                ui.close_menu();
            }
        }
        ui.separator();
        visibility_menu(ui, act, &format!("refs/heads/{full}"), hid, current);
    });
}

/// Drag-and-drop reordering of a PINNED row: the row being dragged remembers the
/// row under the pointer (kept in egui memory across frames), shows an insertion
/// line there, and reports the move when released.
fn pin_drag(ui: &mut egui::Ui, r: &Response, full_ref: &str, index: usize, act: &mut Option<SidebarAction>) {
    let ctx = ui.ctx().clone();
    let target_id = egui::Id::new("pinned_drag_target");
    let src_id = egui::Id::new("pinned_drag_source");
    let src: Option<(String, usize)> = ctx.data(|d| d.get_temp(src_id));
    if r.drag_started() {
        ctx.data_mut(|d| {
            d.insert_temp(src_id, (full_ref.to_owned(), index));
            d.remove::<String>(target_id);
        });
    }
    if r.dragged() {
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    if let Some((_, src_index)) = src.filter(|(s, _)| s != full_ref) {
        // Another row is being dragged: is the pointer over this one?
        if ctx.pointer_latest_pos().is_some_and(|p| r.rect.contains(p)) {
            ctx.data_mut(|d| d.insert_temp(target_id, full_ref.to_owned()));
            // Dropping below the source puts it after this row, above puts it before.
            let y = if src_index < index { r.rect.bottom() } else { r.rect.top() };
            ui.painter().hline(r.rect.x_range(), y, egui::Stroke::new(2.0_f32, Color32::from_rgb(0xe5, 0xc0, 0x7b)));
        }
    }
    if r.drag_stopped() {
        let target: Option<String> = ctx.data(|d| d.get_temp(target_id));
        ctx.data_mut(|d| {
            d.remove::<(String, usize)>(src_id);
            d.remove::<String>(target_id);
        });
        if let Some(t) = target.filter(|t| t != full_ref) {
            *act = Some(SidebarAction::ReorderPinned(full_ref.to_owned(), t));
        }
    }
}

/// The visibility entries of a branch/tag context menu.
fn visibility_menu(ui: &mut egui::Ui, act: &mut Option<SidebarAction>, full_ref: &str, hidden: bool, current: bool) {
    if ui
        .add_enabled(!current, egui::Button::new(if hidden { "Show in graph" } else { "Hide from graph" }))
        .clicked()
    {
        *act = Some(SidebarAction::SetHidden(vec![full_ref.to_owned()], !hidden));
        ui.close_menu();
    }
    if ui.button("Show only this in the graph").clicked() {
        *act = Some(SidebarAction::Solo(full_ref.to_owned()));
        ui.close_menu();
    }
    if ui.button("Show all").clicked() {
        *act = Some(SidebarAction::ShowAll);
        ui.close_menu();
    }
}

/// Render `nodes` as folders (collapsible rows) and leaves.
/// Expand every folder above `name` (a `/`-separated branch or tag name) in the
/// tree identified by `salt`, using the same ids `tree` stores its state under.
fn expand_ancestors(ui: &egui::Ui, salt: &str, name: &str) {
    let parts: Vec<&str> = name.split('/').collect();
    let mut prefix = String::new();
    for part in &parts[..parts.len().saturating_sub(1)] {
        prefix.push_str(part);
        prefix.push('/');
        let id = egui::Id::new((salt, &prefix));
        ui.data_mut(|d| d.insert_persisted(id, true));
    }
}

/// The ids of every leaf under `nodes`.
fn leaf_ids(nodes: &[Node]) -> Vec<usize> {
    let mut out = Vec::new();
    for n in nodes {
        match n {
            Node::File(_, id) => out.push(*id),
            Node::Dir(_, children) => out.extend(leaf_ids(children)),
        }
    }
    out
}

/// Render `nodes` as folders (collapsible rows) and leaves. Each folder has an
/// eye that hides or shows everything inside it; clicks are collected in `toggles`
/// as (leaf ids, hide?).
#[allow(clippy::too_many_arguments)]
fn tree(
    ui: &mut egui::Ui,
    salt: &str,
    nodes: &[Node],
    depth: usize,
    prefix: &str,
    force_open: bool,
    is_hidden: &dyn Fn(usize) -> bool,
    toggles: &mut Vec<(Vec<usize>, bool)>,
    leaf: &mut dyn FnMut(&mut egui::Ui, usize, &str, usize),
) {
    for n in nodes {
        match n {
            Node::File(name, id) => leaf(ui, *id, name, depth),
            Node::Dir(name, children) => {
                let full = format!("{prefix}{name}/");
                let id = egui::Id::new((salt, &full));
                let mut open = force_open || ui.data_mut(|d| *d.get_persisted_mut_or(id, true));
                let ids = leaf_ids(children);
                let all_hidden = !ids.is_empty() && ids.iter().all(|&i| is_hidden(i));
                let (r, eye) = row_eye(ui, depth, Icon::Folder, name, false, (!ids.is_empty()).then_some(all_hidden));
                if eye {
                    toggles.push((ids, !all_hidden));
                } else if r.clicked() {
                    open = !open;
                    ui.data_mut(|d| d.insert_persisted(id, open));
                }
                if open {
                    tree(ui, salt, children, depth + 1, &full, force_open, is_hidden, toggles, leaf);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collapsed_branch_row_appears_once_its_folders_are_expanded() {
        let ctx = egui::Context::default();
        let items = [("feature/login/fix-typo", 0usize), ("main", 1)];
        let nodes = filetree::build(&items, false, false);
        let rendered = |ui: &mut egui::Ui| {
            let mut leaves = Vec::new();
            let mut toggles = Vec::new();
            tree(ui, "local", &nodes, 0, "", false, &|_| false, &mut toggles, &mut |_, id, _, _| leaves.push(id));
            leaves
        };
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // The user collapsed the folders.
                for full in ["feature/", "feature/login/"] {
                    ui.data_mut(|d| d.insert_persisted(egui::Id::new(("local", full.to_owned())), false));
                }
                assert_eq!(rendered(ui), vec![1], "only `main` is listed while the folders are closed");
                expand_ancestors(ui, "local", "feature/login/fix-typo");
                assert_eq!(rendered(ui), vec![0, 1], "the nested branch shows once its folders are open");
            });
        });
    }

    #[test]
    fn locating_a_branch_opens_every_folder_above_it() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // Collapse the folders first, as the user would have.
                for full in ["feature/", "feature/login/"] {
                    ui.data_mut(|d| d.insert_persisted(egui::Id::new(("local", full.to_owned())), false));
                }
                expand_ancestors(ui, "local", "feature/login/fix-typo");
                for full in ["feature/", "feature/login/"] {
                    let open = ui.data_mut(|d| d.get_persisted::<bool>(egui::Id::new(("local", full.to_owned()))));
                    assert_eq!(open, Some(true), "{full} is open");
                }
                // A branch with no folder has nothing to open; unrelated folders are untouched.
                ui.data_mut(|d| d.insert_persisted(egui::Id::new(("local", "other/".to_owned())), false));
                expand_ancestors(ui, "local", "main");
                let other = ui.data_mut(|d| d.get_persisted::<bool>(egui::Id::new(("local", "other/".to_owned()))));
                assert_eq!(other, Some(false));
            });
        });
    }
}
