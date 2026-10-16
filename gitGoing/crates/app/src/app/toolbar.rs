//! The toolbar and status bar of a repository tab.

use super::*;

impl RepoTab {
    pub(super) fn toolbar(&mut self, ctx: &egui::Context, pull_mode: PullMode) -> ToolbarOut {
        let mut out = ToolbarOut::default();
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if icons::tool_button(ui, "open", Icon::Folder, true).on_hover_text("Open a repository").clicked() {
                    out.open = rfd::FileDialog::new().pick_folder();
                }
                ui.separator();

                // Repository and branch, each with a small caption like the buttons.
                if let Some(info) = self.repo.as_ref().and_then(|l| l.info.as_ref()) {
                    let name = info.path.file_name().map_or_else(
                        || info.path.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    ui.vertical(|ui| {
                        ui.small("repository");
                        ui.label(egui::RichText::new(name).size(18.0).strong());
                    });
                    ui.add_space(10.0);
                    // The branch, with what it tracks right underneath (so the toolbar
                    // is only as wide as the branch name).
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.small("branch");
                        let b = info.head_branch.clone().unwrap_or_else(|| "(detached HEAD)".to_owned());
                        let name = ui.label(egui::RichText::new(b).size(18.0).strong());
                        if let Some(u) = &info.upstream {
                            let sync = match (u.ahead, u.behind) {
                                (0, 0) => String::new(),
                                (a, 0) => format!("  ·  {a} ahead"),
                                (0, b) => format!("  ·  {b} behind"),
                                (a, b) => format!("  ·  {a} ahead, {b} behind"),
                            };
                            let text = format!("tracking {}{sync}", u.name);
                            ui.scope(|ui| {
                                // Never wider than the branch name; the full text is a tooltip.
                                ui.set_max_width(name.rect.width());
                                ui.add(egui::Label::new(egui::RichText::new(&text).size(11.5).weak()).truncate())
                                    .on_hover_text(&text);
                            });
                        }
                    });
                    ui.separator();
                }

                let can = self.repo.as_ref().is_some_and(|l| l.info.is_some()) && self.op_rx.is_none();
                let tip = |v: &[UndoEntry], verb: &str| {
                    v.last().map_or_else(|| format!("Nothing to {}", verb.to_lowercase()), |e| format!("{verb} {}", e.label))
                };
                if icons::tool_button(ui, "Undo", Icon::Undo, can && !self.undo.is_empty())
                    .on_hover_text(tip(&self.undo, "Undo"))
                    .on_disabled_hover_text(tip(&self.undo, "Undo"))
                    .clicked()
                {
                    out.undo = true;
                }
                if icons::tool_button(ui, "Redo", Icon::Redo, can && !self.redo.is_empty())
                    .on_hover_text(tip(&self.redo, "Redo"))
                    .on_disabled_hover_text(tip(&self.redo, "Redo"))
                    .clicked()
                {
                    out.redo = true;
                }
                ui.separator();
                if icons::tool_button(ui, "Fetch", Icon::Fetch, can).on_hover_text("Fetch all remotes and prune").clicked() {
                    out.remote = Some(("Fetch", Box::new(remote::fetch)));
                }
                if icons::tool_button(ui, "Pull", Icon::Pull, can).on_hover_text(pull_mode.label()).clicked() {
                    out.pull = Some(pull_mode);
                }
                // Drop-down next to Pull to choose how to integrate for this one pull.
                ui.spacing_mut().item_spacing.x = 0.0;
                let arrow = egui::menu::menu_custom_button(
                    ui,
                    egui::Button::new("").frame(false).min_size(Vec2::new(14.0, 50.0)),
                    |ui| {
                        for mode in PullMode::ALL {
                            if ui.add_enabled(can, egui::Button::new(mode.label())).clicked() {
                                out.pull = Some(mode);
                                ui.close_menu();
                            }
                        }
                    },
                );
                let r = arrow.response.rect;
                let tri = ui.visuals().weak_text_color();
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(r.center().x - 3.5, r.center().y + 4.0),
                        Pos2::new(r.center().x + 3.5, r.center().y + 4.0),
                        Pos2::new(r.center().x, r.center().y + 8.0),
                    ],
                    tri,
                    Stroke::NONE,
                ));
                ui.spacing_mut().item_spacing.x = 4.0;
                if icons::tool_button(ui, "Push", Icon::Push, can).on_hover_text("Push the current branch").clicked() {
                    out.remote = Some(("Push", Box::new(remote::push)));
                }
                // Drop-down next to Push: the force push needed after rewriting pushed history.
                ui.spacing_mut().item_spacing.x = 0.0;
                let push_arrow = egui::menu::menu_custom_button(
                    ui,
                    egui::Button::new("").frame(false).min_size(Vec2::new(14.0, 50.0)),
                    |ui| {
                        if ui.add_enabled(can, egui::Button::new("Push")).clicked() {
                            out.remote = Some(("Push", Box::new(remote::push)));
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(can, egui::Button::new("Force push (with lease)…"))
                            .on_hover_text("Replace the remote branch after amend, squash or rebase")
                            .clicked()
                        {
                            out.force_push = true;
                            ui.close_menu();
                        }
                    },
                );
                let r = push_arrow.response.rect;
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(r.center().x - 3.5, r.center().y + 4.0),
                        Pos2::new(r.center().x + 3.5, r.center().y + 4.0),
                        Pos2::new(r.center().x, r.center().y + 8.0),
                    ],
                    ui.visuals().weak_text_color(),
                    Stroke::NONE,
                ));
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.separator();
                if icons::tool_button(ui, "Branch", Icon::Branch, can).on_hover_text("Create a branch").clicked() {
                    self.new_branch = Some(String::new());
                }
                let has_stash = self
                    .repo
                    .as_ref()
                    .and_then(|l| l.info.as_ref())
                    .is_some_and(|i| !i.stashes.is_empty());
                if icons::tool_button(ui, "Stash", Icon::Stash, can).on_hover_text("Stash changes…").clicked() {
                    out.stash = Some(StashOp::Save);
                }
                if icons::tool_button(ui, "Pop", Icon::Pop, can && has_stash).on_hover_text("Apply and remove the latest stash").clicked() {
                    out.stash = Some(StashOp::Pop(0));
                }
                ui.separator();
                if icons::tool_button(ui, "Console", Icon::Terminal, true)
                    .on_hover_text("Show the git commands the app runs, as they happen")
                    .clicked()
                {
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("toggle_console"), true));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::tool_button(ui, "search", Icon::Search, true)
                        .on_hover_text("Search commits (Cmd/Ctrl+F)")
                        .clicked()
                    {
                        ui.ctx().memory_mut(|m| m.request_focus(egui::Id::new(("search", self.id))));
                    }
                });

                if self.op_rx.is_some() {
                    ui.add_space(8.0);
                    ui.spinner();
                    ui.weak(format!("{}…", self.op_label));
                } else if self.plan_rx.is_some() {
                    ui.add_space(8.0);
                    ui.spinner();
                    ui.weak("Checking the commits…");
                }
            });
            ui.add_space(3.0);
        });
        out
    }

    pub(super) fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // The logo, tiny, at the left of the status bar.
                if let Some(mark) = crate::assets::mark(ctx) {
                    ui.add(egui::Image::new(&mark).fit_to_exact_size(Vec2::splat(16.0))).on_hover_text("gitgoing");
                }
                if let Some((n, is_err)) = &self.notice {
                    if *is_err {
                        ui.colored_label(Color32::LIGHT_RED, n.lines().last().unwrap_or(n)).on_hover_text(n);
                    } else {
                        ui.label(n);
                    }
                } else if let Some(info) = self.repo.as_ref().and_then(|l| l.info.as_ref()) {
                    ui.weak(info.path.display().to_string());
                }
                if let Some(l) = &self.repo {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.notice.is_some() && ui.small_button("Dismiss").clicked() {
                            self.notice = None;
                            self.push_rejected = false;
                        }
                        if self.push_rejected
                            && ui
                                .small_button("Force push…")
                                .on_hover_text("Replace the remote branch with yours (asks first)")
                                .clicked()
                        {
                            self.force_requested = true;
                        }
                        ui.weak(format!(
                            "{} commits{}",
                            l.rows.len(),
                            if l.loading { " (loading…)" } else { "" }
                        ));
                    });
                }
            });
        });
    }
}
