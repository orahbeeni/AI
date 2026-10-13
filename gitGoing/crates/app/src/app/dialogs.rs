//! The pop-up windows of a repository tab: create branch/tag, force push,
//! squash, interactive rebase, remotes, upstream, and confirmations.

use super::*;

impl RepoTab {
    /// Draw whichever dialogs are open.
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        self.dialog_new_branch(ctx);
        self.dialog_new_tag(ctx);
        self.dialog_force_push(ctx);
        self.dialog_squash(ctx);
        self.dialog_rebase(ctx);
        self.dialog_remote(ctx);
        self.dialog_upstream(ctx);
        self.dialog_confirm(ctx);
    }

    fn dialog_new_branch(&mut self, ctx: &egui::Context) {
        let mut create = None;
        let mut close = false;
        let at = self.branch_at;
        if let Some(name) = &mut self.new_branch {
            egui::Window::new("New branch")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(match at {
                        Some(id) => format!("Create a branch at {} and check it out.", &id.to_string()[..8]),
                        None => "Create a branch at the current commit and check it out.".to_owned(),
                    });
                    let r = ui.add(egui::TextEdit::singleline(name).hint_text("branch-name"));
                    r.request_focus();
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        let ok = !name.trim().is_empty();
                        if ui.add_enabled(ok, egui::Button::new("Create")).clicked() || (enter && ok) {
                            create = Some(name.trim().to_owned());
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
        }
        if close || create.is_some() {
            self.new_branch = None;
            self.branch_at = None;
        }
        if let Some(n) = create {
            match at {
                Some(id) => self.run_op(ctx, "Create branch", move |p| branches::create_at(p, &n, id, true)),
                None => self.run_op(ctx, "Create branch", move |p| branches::create_and_checkout(p, &n)),
            }
        }
    }

    fn dialog_new_tag(&mut self, ctx: &egui::Context) {
        let mut tag_create = None;
        let mut tag_close = false;
        if let Some((id, name, msg)) = &mut self.new_tag {
            egui::Window::new("New tag")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(format!("Tag commit {}.", &id.to_string()[..8]));
                    let r = ui.add(egui::TextEdit::singleline(name).hint_text("tag-name"));
                    r.request_focus();
                    ui.add(egui::TextEdit::multiline(msg).hint_text("Message (optional; makes an annotated tag)").desired_rows(2));
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!name.trim().is_empty(), egui::Button::new("Create")).clicked() {
                            tag_create = Some((*id, name.trim().to_owned(), msg.clone()));
                        }
                        if ui.button("Cancel").clicked() {
                            tag_close = true;
                        }
                    });
                });
        }
        if tag_close || tag_create.is_some() {
            self.new_tag = None;
        }
        if let Some((id, n, m)) = tag_create {
            self.run_op(ctx, "Create tag", move |p| commit_ops::create_tag(p, &n, id, &m));
        }
    }

    fn dialog_force_push(&mut self, ctx: &egui::Context) {
        // Force push confirmation.
        let mut force_go = false;
        let mut force_close = false;
        if let Some(d) = &self.force_dlg {
            egui::Window::new("Force push")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_width(460.0);
                    ui.label(format!("Force push '{}' to '{}'?", d.branch, d.upstream));
                    ui.add_space(4.0);
                    ui.label(if d.behind > 0 {
                        format!(
                            "This replaces {} commit(s) on the remote with your {} commit(s). Anyone who has already \
                             pulled the old commits will need to reset their branch.",
                            d.behind, d.ahead
                        )
                    } else {
                        "The remote branch will be replaced by your local branch.".to_owned()
                    });
                    ui.add_space(4.0);
                    ui.weak(
                        "Uses --force-with-lease: it refuses if the remote branch has changed since your last fetch, \
                         so a teammate's newer commits are never silently overwritten. If it refuses, fetch and look \
                         at what changed first. Some servers also protect branches such as main from force pushes.",
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(egui::RichText::new("Force push").color(Color32::from_rgb(0xe5, 0x53, 0x4b)))
                            .clicked()
                        {
                            force_go = true;
                        }
                        if ui.button("Cancel").clicked() {
                            force_close = true;
                        }
                    });
                });
        }
        if force_close || force_go {
            self.force_dlg = None;
        }
        if force_go {
            self.run_op(ctx, "Force push", remote::force_push);
        }
    }

    fn dialog_squash(&mut self, ctx: &egui::Context) {
        // Squash window.
        let mut squash_go = false;
        let mut squash_close = false;
        if let Some(d) = &mut self.squash_dlg {
            egui::Window::new("Squash commits")
                .collapsible(false)
                .resizable(true)
                .default_width(520.0)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(format!("Combine {} commits into one.", d.ids.len()));
                    if d.plan.pushed > 0 {
                        ui.colored_label(
                            Color32::from_rgb(0xe5, 0xc0, 0x7b),
                            format!(
                                "{} of the commits this rewrites are already pushed. Afterwards you will need to \
                                 force push (Push drop-down, or the button that appears if a push is rejected).",
                                d.plan.pushed
                            ),
                        );
                    }
                    if !d.plan.consecutive {
                        ui.add_space(4.0);
                        ui.colored_label(
                            Color32::from_rgb(0xe5, 0xc0, 0x7b),
                            "These commits are not next to each other. The commits between them will be moved \
                             after the combined commit, which rewrites them too. New order, oldest first:",
                        );
                        egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                            for c in &d.plan.order {
                                let text = format!("{}  {}", &c.id.to_string()[..7], c.summary);
                                if c.squashed {
                                    ui.label(egui::RichText::new(format!("> {text}")).strong());
                                } else {
                                    ui.label(format!("  {text}"));
                                }
                            }
                        });
                        ui.weak("Bold commits are combined. Only text conflicts are checked; code that depends on the order can still break.");
                    }
                    match (&d.plan.conflict, &d.plan.check_error) {
                        (Some(c), _) => {
                            ui.add_space(6.0);
                            egui::Frame::new()
                                .fill(Color32::from_rgba_unmultiplied(0xe5, 0x53, 0x4b, 40))
                                .corner_radius(4.0)
                                .inner_margin(8.0)
                                .show(ui, |ui| {
                                    ui.colored_label(Color32::from_rgb(0xe5, 0x53, 0x4b), "A conflict is predicted");
                                    ui.label(format!(
                                        "Re-applying \"{}\" after the combined commit will not apply cleanly in: {}.",
                                        c.summary,
                                        c.files.join(", ")
                                    ));
                                    ui.label(
                                        "The squash will stop there and you will need to resolve it in the conflict \
                                         editor, then continue. You can abort at any time to restore your history. \
                                         There may be further conflicts after this one.",
                                    );
                                    ui.checkbox(&mut d.confirmed, "I understand and will resolve the conflict");
                                });
                        }
                        (None, Some(why)) => {
                            ui.add_space(4.0);
                            ui.weak(format!("Could not check for conflicts ({why}). If there are any, you will resolve them afterwards."));
                        }
                        (None, None) if !d.plan.consecutive => {
                            ui.add_space(4.0);
                            ui.colored_label(Color32::from_rgb(0x4c, 0xaf, 0x50), "No conflicts predicted.");
                        }
                        _ => {}
                    }
                    ui.add_space(6.0);
                    ui.label("Message for the new commit:");
                    ui.add(
                        egui::TextEdit::multiline(&mut d.message)
                            .desired_rows(7)
                            .desired_width(f32::INFINITY)
                            .code_editor(),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let conflict = d.plan.conflict.is_some();
                        let ready = !d.message.trim().is_empty() && (!conflict || d.confirmed);
                        let label = if conflict { "Squash and resolve conflicts" } else { "Squash" };
                        if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                            squash_go = true;
                        }
                        if ui.button("Cancel").clicked() {
                            squash_close = true;
                        }
                    });
                });
        }
        if squash_close {
            self.squash_dlg = None;
        }
        if squash_go {
            if let Some(d) = self.squash_dlg.take() {
                let (ids, msg) = (d.ids, d.message);
                self.run_op(ctx, "Squash", move |p| rebase_outcome(rebase::squash(p, &ids, &msg)));
            }
        }
    }

    fn dialog_rebase(&mut self, ctx: &egui::Context) {
        // Interactive rebase planner.
        let mut rebase_go = false;
        let mut rebase_close = false;
        if let Some(d) = &mut self.rebase_dlg {
            egui::Window::new("Interactive rebase")
                .collapsible(false)
                .resizable(true)
                .default_width(560.0)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Newest first. Commits are applied from the bottom up; choose what happens to each.");
                    if d.pushed > 0 {
                        ui.colored_label(
                            Color32::from_rgb(0xe5, 0xc0, 0x7b),
                            format!("{} of these commits are already pushed; you will need to force push afterwards.", d.pushed),
                        );
                    }
                    ui.add_space(4.0);
                    let n = d.items.len();
                    let mut swap = None;
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        for i in 0..n {
                            ui.horizontal(|ui| {
                                if ui.add_enabled(i > 0, egui::Button::new("Up").small()).on_hover_text("Apply later").clicked() {
                                    swap = Some((i, i - 1));
                                }
                                if ui.add_enabled(i + 1 < n, egui::Button::new("Down").small()).on_hover_text("Apply earlier").clicked() {
                                    swap = Some((i, i + 1));
                                }
                                let item = &mut d.items[i];
                                egui::ComboBox::from_id_salt(("rebase_action", i))
                                    .width(80.0)
                                    .selected_text(item.action.label())
                                    .show_ui(ui, |ui| {
                                        for a in RebaseAction::ALL {
                                            ui.selectable_value(&mut item.action, a, a.label());
                                        }
                                    });
                                ui.monospace(&item.id.to_string()[..7]);
                                let dim = item.action == RebaseAction::Drop;
                                let text = egui::RichText::new(&item.summary);
                                ui.add(egui::Label::new(if dim { text.strikethrough().weak() } else { text }).truncate());
                            });
                            if d.items[i].action == RebaseAction::Reword {
                                ui.add(
                                    egui::TextEdit::multiline(&mut d.items[i].message)
                                        .desired_rows(2)
                                        .desired_width(f32::INFINITY),
                                );
                            }
                        }
                    });
                    if let Some((a, b)) = swap {
                        d.items.swap(a, b);
                    }
                    let last_ok = !matches!(d.items.last().map(|i| i.action), Some(RebaseAction::Squash | RebaseAction::Fixup));
                    if !last_ok {
                        ui.colored_label(Color32::LIGHT_RED, "The oldest commit (bottom) cannot be squashed or fixed up.");
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(last_ok, egui::Button::new("Start rebase")).clicked() {
                            rebase_go = true;
                        }
                        if ui.button("Cancel").clicked() {
                            rebase_close = true;
                        }
                    });
                });
        }
        if rebase_close {
            self.rebase_dlg = None;
        }
        if rebase_go {
            if let Some(d) = self.rebase_dlg.take() {
                let mut items = d.items;
                items.reverse(); // git wants oldest first
                let from = d.from;
                self.run_op(ctx, "Rebase", move |p| rebase_outcome(rebase::start_interactive(p, from, &items)));
            }
        }
    }

    fn dialog_remote(&mut self, ctx: &egui::Context) {
        // Add / edit / rename remote.
        let mut remote_go = None;
        let mut remote_close = false;
        if let Some(d) = &mut self.remote_dlg {
            let title = match d.kind {
                RemoteDlgKind::Add => "Add remote",
                RemoteDlgKind::EditUrl => "Edit remote URL",
                RemoteDlgKind::Rename => "Rename remote",
            };
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_width(420.0);
                    if d.kind == RemoteDlgKind::Add {
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(f32::INFINITY));
                    }
                    ui.label(if d.kind == RemoteDlgKind::Rename { "New name" } else { "URL" });
                    let r = ui.add(egui::TextEdit::singleline(&mut d.text).desired_width(f32::INFINITY));
                    if d.kind != RemoteDlgKind::Add {
                        r.request_focus();
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!d.text.trim().is_empty(), egui::Button::new("OK")).clicked() {
                            remote_go = Some((d.kind, d.name.clone(), d.text.clone()));
                        }
                        if ui.button("Cancel").clicked() {
                            remote_close = true;
                        }
                    });
                });
        }
        if remote_close || remote_go.is_some() {
            self.remote_dlg = None;
        }
        if let Some((kind, name, text)) = remote_go {
            match kind {
                RemoteDlgKind::Add => self.run_op(ctx, "Add remote", move |p| remotes::add(p, &name, &text)),
                RemoteDlgKind::EditUrl => self.run_op(ctx, "Edit remote", move |p| remotes::set_url(p, &name, &text)),
                RemoteDlgKind::Rename => self.run_op(ctx, "Rename remote", move |p| remotes::rename(p, &name, &text)),
            }
        }
    }

    fn dialog_upstream(&mut self, ctx: &egui::Context) {
        // Set upstream.
        let mut up_go = None;
        let mut up_close = false;
        if let Some(d) = &mut self.upstream_dlg {
            egui::Window::new("Set upstream")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_width(380.0);
                    ui.label(format!("Branch '{}' should track:", d.local));
                    egui::ComboBox::from_id_salt("upstream_choice")
                        .width(340.0)
                        .selected_text(d.choice.as_deref().unwrap_or("(no upstream)"))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut d.choice, None, "(no upstream)");
                            for o in &d.options {
                                ui.selectable_value(&mut d.choice, Some(o.clone()), o);
                            }
                        });
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            up_go = Some((d.local.clone(), d.choice.clone()));
                        }
                        if ui.button("Cancel").clicked() {
                            up_close = true;
                        }
                    });
                });
        }
        if up_close || up_go.is_some() {
            self.upstream_dlg = None;
        }
        if let Some((local, up)) = up_go {
            self.run_op(ctx, "Set upstream", move |p| remotes::set_upstream(p, &local, up.as_deref()));
        }
    }

    fn dialog_confirm(&mut self, ctx: &egui::Context) {
        let mut confirmed = false;
        let mut cancel = false;
        if let Some(c) = &self.confirm {
            let (title, text) = match c {
                Confirm::DeleteBranch(n) => (
                    "Delete branch",
                    format!("Delete local branch '{n}'? Commits only reachable from it will be lost."),
                ),
                Confirm::DropStash(i) => (
                    "Delete stash",
                    format!("Permanently delete stash {i}? Its changes will be lost."),
                ),
                Confirm::DeleteTag(n) => ("Delete tag", format!("Delete local tag '{n}'?")),
                Confirm::DeleteRemoteBranch(n) => (
                    "Delete remote branch",
                    format!("Delete '{n}' on the remote? Others who use it will lose it."),
                ),
                Confirm::RemoveRemote(n) => (
                    "Remove remote",
                    format!("Remove the remote '{n}' and its remote-tracking branches from this repository?"),
                ),
                Confirm::DeleteRemoteTag(n) => ("Delete remote tag", format!("Delete tag '{n}' on the remote?")),
                Confirm::HardReset(id) => (
                    "Hard reset",
                    format!(
                        "Reset the current branch to {} and DISCARD all uncommitted changes? This cannot be undone for uncommitted work.",
                        &id.to_string()[..8]
                    ),
                ),
            };
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(text);
                    ui.horizontal(|ui| {
                        confirmed = ui.button("Delete").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
        }
        if confirmed || cancel {
            let c = self.confirm.take();
            if confirmed {
                match c {
                    Some(Confirm::DeleteBranch(n)) => {
                        self.run_op(ctx, "Delete branch", move |p| branches::delete(p, &n))
                    }
                    Some(Confirm::DeleteRemoteBranch(full)) => {
                        let (remote, branch) = full.split_once('/').map(|(r, b)| (r.to_owned(), b.to_owned())).unwrap_or_default();
                        self.run_op(ctx, "Delete remote branch", move |p| remotes::delete_remote_branch(p, &remote, &branch))
                    }
                    Some(Confirm::RemoveRemote(n)) => self.run_op(ctx, "Remove remote", move |p| remotes::remove(p, &n)),
                    Some(Confirm::DeleteRemoteTag(n)) => {
                        self.run_op(ctx, "Delete remote tag", move |p| remotes::delete_remote_tag(p, &n))
                    }
                    Some(Confirm::DeleteTag(n)) => {
                        self.run_op(ctx, "Delete tag", move |p| commit_ops::delete_tag(p, &n))
                    }
                    Some(Confirm::HardReset(id)) => {
                        self.run_op(ctx, "Hard reset", move |p| commit_ops::reset(p, id, ResetMode::Hard))
                    }
                    Some(Confirm::DropStash(i)) => {
                        self.run_op(ctx, "Drop stash", move |p| stash::drop(p, i))
                    }
                    None => {}
                }
            }
        }
    }
}
