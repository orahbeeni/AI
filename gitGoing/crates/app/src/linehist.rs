//! The "Line history" window: every commit that changed a range of lines, newest first, with the
//! part of each commit's diff that touched them. Opened from a removed line's origin note or a
//! hunk's "Line History" button, to see why lines exist before changing them.

use crate::detail::{spawn, Fetch};
use eframe::egui::{self, Color32, RichText, Vec2};
use gitgoing_core::origins::{self, LineCommit};
use gitgoing_core::Oid;
use std::path::PathBuf;

/// What to show the history of.
pub struct Request {
    pub repo: PathBuf,
    /// The commit whose line numbers `start..=end` refer to.
    pub rev: Oid,
    pub path: String,
    pub start: u32,
    pub end: u32,
}

pub enum Outcome {
    Open,
    Closed,
    /// Show this commit in the commit panel.
    ShowCommit(Oid),
}

pub struct LineHistWin {
    path: String,
    start: u32,
    end: u32,
    rx: Fetch<Vec<LineCommit>>,
    list: Option<Result<Vec<LineCommit>, String>>,
    selected: usize,
    /// Distinguishes windows opened one after another, so each starts fresh.
    serial: u64,
}

impl LineHistWin {
    pub fn new(ctx: &egui::Context, req: Request, serial: u64) -> Self {
        let Request { repo, rev, path, start, end } = req;
        let p = path.clone();
        LineHistWin {
            rx: Some(spawn(ctx, move || origins::line_history(&repo, rev, &p, start, end, 100))),
            path,
            start,
            end,
            list: None,
            selected: 0,
            serial,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        if let Some(rx) = &self.rx {
            if let Ok(r) = rx.try_recv() {
                self.rx = None;
                self.list = Some(r);
            }
        }
        let mut open = true;
        let mut outcome = Outcome::Open;
        let lines = if self.start == self.end { format!("line {}", self.start) } else { format!("lines {}–{}", self.start, self.end) };
        egui::Window::new("Line history")
            .id(egui::Id::new(("line_history", self.serial)))
            .open(&mut open)
            .default_size([960.0, 560.0])
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.path).strong());
                    ui.weak(format!("{lines}, as committed"));
                });
                ui.add_space(4.0);
                let list = match &self.list {
                    None => {
                        ui.spinner();
                        return;
                    }
                    Some(Err(e)) => {
                        ui.colored_label(Color32::LIGHT_RED, e);
                        return;
                    }
                    Some(Ok(l)) if l.is_empty() => {
                        ui.weak("No commits changed these lines.");
                        return;
                    }
                    Some(Ok(l)) => l,
                };
                let now = crate::timefmt::now();
                let selected = self.selected.min(list.len() - 1);
                let avail = ui.available_size();
                ui.horizontal_top(|ui| {
                    // The commits, newest first.
                    ui.allocate_ui_with_layout(Vec2::new(320.0, avail.y), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        egui::ScrollArea::vertical().id_salt("line_history_list").auto_shrink([false, false]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (i, c) in list.iter().enumerate() {
                                let (rect, resp) = ui.allocate_exact_size(Vec2::new(310.0, 42.0), egui::Sense::click());
                                let v = ui.visuals();
                                let fill = if i == selected {
                                    v.selection.bg_fill
                                } else if resp.hovered() {
                                    v.widgets.hovered.weak_bg_fill
                                } else {
                                    Color32::TRANSPARENT
                                };
                                ui.painter().rect_filled(rect, 4.0, fill);
                                let strong = if i == selected { v.selection.stroke.color } else { v.strong_text_color() };
                                let weak = v.weak_text_color();
                                let clip = ui.painter().with_clip_rect(rect.shrink(4.0));
                                let font = egui::FontId::proportional(13.0);
                                clip.text(rect.left_top() + Vec2::new(8.0, 5.0), egui::Align2::LEFT_TOP, &c.summary, font, strong);
                                let meta = format!("{} · {}", c.author, crate::timefmt::relative(c.time, now));
                                clip.text(rect.left_top() + Vec2::new(8.0, 23.0), egui::Align2::LEFT_TOP, meta, egui::FontId::proportional(11.5), weak);
                                if resp.clicked() {
                                    self.selected = i;
                                }
                            }
                            if list.len() >= 100 {
                                ui.weak("Showing the latest 100 changes.");
                            }
                        });
                    });
                    ui.separator();
                    // The chosen commit: message and its diff of these lines.
                    let c = &list[selected];
                    ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), avail.y), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if selected == 0 {
                                crate::icons::chip(ui, "Latest change");
                            }
                            if let Some(t) = &c.ticket {
                                crate::icons::chip(ui, t);
                            }
                            ui.label(RichText::new(&c.summary).strong().size(15.0));
                        });
                        ui.horizontal(|ui| {
                            ui.weak(format!("{} · {}", c.author, crate::timefmt::relative(c.time, now)));
                            crate::detail::hash_link(ui, &c.id.to_string()[..10], &c.id.to_string());
                            if ui.button("Show commit").on_hover_text("Open the whole commit in the commit panel").clicked() {
                                outcome = Outcome::ShowCommit(c.id);
                            }
                        });
                        let body = c.message.split_once('\n').map(|(_, b)| b.trim()).unwrap_or("");
                        if !body.is_empty() {
                            ui.add_space(2.0);
                            ui.label(RichText::new(body).weak());
                        }
                        ui.add_space(6.0);
                        ui.weak("What this commit changed in these lines:");
                        let font = egui::FontId::monospace(13.0);
                        egui::ScrollArea::both().id_salt(("line_history_diff", selected)).auto_shrink(false).show(ui, |ui| {
                            ui.spacing_mut().item_spacing = Vec2::ZERO;
                            for l in &c.diff {
                                let (bg, fg) = match l.chars().next() {
                                    Some('+') => (Color32::from_rgba_unmultiplied(0x4c, 0xaf, 0x50, 40), None),
                                    Some('-') => (Color32::from_rgba_unmultiplied(0xe5, 0x53, 0x4b, 40), None),
                                    Some('@') => (Color32::TRANSPARENT, Some(ui.visuals().weak_text_color())),
                                    _ => (Color32::TRANSPARENT, None),
                                };
                                let mut t = RichText::new(l.replace('\t', "    ")).font(font.clone()).background_color(bg);
                                if let Some(fg) = fg {
                                    t = t.color(fg);
                                }
                                ui.add(egui::Label::new(t).extend());
                            }
                        });
                    });
                });
            });
        if !open {
            return Outcome::Closed;
        }
        outcome
    }
}

/// Renders the window for a small real repository to `$GITGOING_SHOTS/line_history.png`.
/// Run with: GITGOING_SHOTS=/some/dir cargo test -p gitgoing line_history_render -- --ignored
#[cfg(test)]
mod render_tests {
    use super::*;
    use std::process::Command;

    #[test]
    #[ignore]
    fn line_history_render() {
        let Ok(out) = std::env::var("GITGOING_SHOTS") else { return };
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let git = |args: &[&str]| {
            let ok = Command::new("git").arg("-C").arg(p).args(["-c", "user.name=Jane Smith", "-c", "user.email=j@x"]).args(args).status().unwrap();
            assert!(ok.success());
        };
        git(&["init", "-q"]);
        let write = |body: &str| std::fs::write(p.join("users.impex"), body).unwrap();
        write("$uid=uid\n;originalUid;email;groups(uid);\n;approvers(uid)\n");
        git(&["add", "."]);
        git(&["commit", "-qm", "initial B2B import"]);
        write("$uid=uid\n;originalUid;email;groups(uid);loginDisabled;\n;approvers(uid)\n");
        git(&["commit", "-qam", "ECOMD-4821 disable B2B logins on import\n\nCustomers imported from SAP must not log in until approved."]);
        let head = String::from_utf8(Command::new("git").arg("-C").arg(p).args(["rev-parse", "HEAD"]).output().unwrap().stdout).unwrap();
        let rev = Oid::from_str(head.trim()).unwrap();

        let mut win: Option<LineHistWin> = None;
        let repo = p.to_path_buf();
        let mut h = egui_kittest::Harness::builder().with_size([1000.0, 420.0]).build(move |ctx| {
            let w = win.get_or_insert_with(|| {
                LineHistWin::new(ctx, Request { repo: repo.clone(), rev, path: "users.impex".into(), start: 2, end: 2 }, 1)
            });
            w.show(ctx);
        });
        for _ in 0..50 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.render().unwrap().save(format!("{out}/line_history.png")).unwrap();
    }
}
