//! The commit graph: columns, rows, lanes, the WIP row and multi-selection panel.

use super::*;

const ROW_H: f32 = 24.0;
const LANE_W: f32 = 16.0;
const MAX_GRAPH_COLS: u16 = 14;
const AUTHOR_W: f32 = 140.0;
const DATE_W: f32 = 100.0;
const HEADER_H: f32 = 22.0;

const PALETTE: [Color32; PALETTE_SIZE as usize] = [
    Color32::from_rgb(0x4e, 0xa8, 0xde),
    Color32::from_rgb(0xe0, 0x6c, 0x75),
    Color32::from_rgb(0x98, 0xc3, 0x79),
    Color32::from_rgb(0xe5, 0xc0, 0x7b),
    Color32::from_rgb(0xc6, 0x78, 0xdd),
    Color32::from_rgb(0x56, 0xb6, 0xc2),
    Color32::from_rgb(0xd1, 0x9a, 0x66),
    Color32::from_rgb(0x9a, 0xa5, 0xce),
];

pub(super) fn history_view(
    ui: &mut egui::Ui,
    l: &mut Loaded,
    tab_id: u64,
    widths: &mut PanelWidths,
    act: &mut Option<CommitAction>,
) {
    let now = timefmt::now();
    // Column widths: both are user-resizable by dragging the header dividers;
    // the graph column defaults to fitting the lanes seen so far.
    let auto_w = l.max_cols.clamp(3, MAX_GRAPH_COLS) as f32 * LANE_W + 12.0;
    let graph_w = widths.graph_col.unwrap_or(auto_w).clamp(40.0, 1200.0);
    let ref_w = widths.ref_col.clamp(60.0, 600.0);
    let wip_lane = l
        .head_row
        .or(Some(0))
        .and_then(|h| l.rows.get(h))
        .map_or((0, 0), |r| (r.graph.node_col, r.graph.node_color));
    // Highest lane index that still fits; wider lanes are squeezed into it.
    let max_col = (((graph_w - 12.0) / LANE_W).floor() as u16).max(1) - 1;
    let wip = l.has_wip() as usize;
    let wip_count = l.changes.as_ref().and_then(Changes::count).unwrap_or(0);
    let total = l.rows.len() + wip;
    let visuals = ui.visuals().clone();
    let text_color = visuals.text_color();
    let weak = visuals.weak_text_color();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let small = egui::TextStyle::Small.resolve(ui.style());

    let ctx = ui.ctx().clone();
    let search_id = egui::Id::new(("search", tab_id));
    let mut pending: Option<(usize, bool)> = None;
    // (row, Cmd/Ctrl held, Shift held) for a click that changes the multi-selection.
    let mut mclick: Option<(usize, bool, bool)> = None;

    // Keyboard: Cmd/Ctrl+F focuses search; arrows move the selection.
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
        ctx.memory_mut(|m| m.request_focus(search_id));
    }
    if !ctx.wants_keyboard_input() && !l.rows.is_empty() {
        let (up, down) = ctx.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown)));
        let last = l.rows.len() - 1;
        let next = match (l.selected, up, down) {
            (None, true, _) | (None, _, true) => Some(0),
            (Some(i), true, _) => Some(i.saturating_sub(1)),
            (Some(i), _, true) => Some((i + 1).min(last)),
            _ => None,
        };
        if let Some(n) = next.filter(|n| Some(*n) != l.selected) {
            pending = Some((n, true));
        }
    }

    // Search bar.
    let text_changed = l.search.update(&l.rows);
    let mut nav = 0i32;
    let mut goto_head = false;
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut l.search.text)
                .id(search_id)
                .hint_text("Search message, author or SHA")
                .desired_width(280.0),
        );
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            nav = if ui.input(|i| i.modifiers.shift) { -1 } else { 1 };
            r.request_focus();
        }
        goto_head = ui
            .button("HEAD")
            .on_hover_text("Scroll to the current commit")
            .clicked();
        let n = l.search.matches.len();
        if !l.search.text.trim().is_empty() {
            if n == 0 {
                ui.weak("No results");
            } else {
                ui.weak(format!("{}/{}", l.search.cursor.min(n - 1) + 1, n));
                if ui.small_button("Prev").clicked() {
                    nav = -1;
                }
                if ui.small_button("Next").clicked() {
                    nav = 1;
                }
            }
            if ui.small_button("Clear").clicked() {
                l.search.text.clear();
            }
        }
    });
    if goto_head {
        let head = l.info.as_ref().and_then(|i| i.head_oid);
        if let Some(idx) = head.and_then(|h| l.rows.iter().position(|r| r.id == h)) {
            l.reveal(idx, true);
            pending = Some((idx, false));
        }
    }
    let n = l.search.matches.len();
    if n > 0 {
        if text_changed {
            l.reveal(l.search.matches[0] as usize, true);
        }
        if nav != 0 {
            let cur = l.search.matches[l.search.cursor.min(n - 1)] as usize;
            if nav > 0 {
                if l.selected == Some(cur) {
                    l.search.cursor = (l.search.cursor + 1) % n;
                }
            } else {
                l.search.cursor = (l.search.cursor + n - 1) % n;
            }
            pending = Some((l.search.matches[l.search.cursor] as usize, true));
        }
    }

    // Column headers.
    let (hrect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), HEADER_H), Sense::hover());
    let hp = ui.painter_at(hrect);
    let hfont = egui::TextStyle::Small.resolve(ui.style());
    for (x, label) in [
        (hrect.left() + 6.0, "BRANCH / TAG"),
        (hrect.left() + ref_w + 6.0, "GRAPH"),
        (hrect.left() + ref_w + graph_w + 6.0, "COMMIT MESSAGE"),
    ] {
        hp.text(Pos2::new(x, hrect.center().y), egui::Align2::LEFT_CENTER, label, hfont.clone(), weak);
    }
    // Draggable dividers on the right edge of the BRANCH/TAG and GRAPH columns.
    for (i, x) in [hrect.left() + ref_w, hrect.left() + ref_w + graph_w].into_iter().enumerate() {
        let strip = Rect::from_center_size(Pos2::new(x, hrect.center().y), Vec2::new(8.0, HEADER_H));
        let r = ui.interact(strip, egui::Id::new(("colgrip", tab_id, i)), Sense::drag());
        let active = r.hovered() || r.dragged();
        if active {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        let stroke = if active {
            ui.visuals().widgets.active.fg_stroke
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        };
        hp.vline(x, hrect.y_range(), stroke);
        if r.dragged() {
            let dx = r.drag_delta().x;
            if i == 0 {
                widths.ref_col = (ref_w + dx).clamp(60.0, 600.0);
            } else {
                widths.graph_col = Some((graph_w + dx).clamp(40.0, 1200.0));
            }
        }
        if i == 1 && r.double_clicked() {
            widths.graph_col = None; // back to automatic width
        }
    }
    ui.separator();

    let mut area = egui::ScrollArea::vertical().id_salt(tab_id).auto_shrink(false);
    if let Some((disp, center)) = l.scroll_to.take() {
        let view_h = (l.visible.len().max(4) - 1) as f32 * ROW_H;
        let y = disp as f32 * ROW_H;
        let offset = if center {
            y - view_h / 2.0
        } else if disp < l.visible.start {
            y
        } else {
            y + ROW_H - view_h
        };
        area = area.vertical_scroll_offset(offset.max(0.0));
    }
    area.show_rows(ui, ROW_H, total, |ui, range| {
            l.visible = range.clone();
            for i in range {
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                let p = ui.painter_at(rect);
                let cy = rect.center().y;
                let graph_x = rect.left() + ref_w;

                if wip == 1 && i == 0 {
                    // Working-tree row.
                    if resp.clicked() {
                        l.show_changes = true;
                        l.detail = None;
                        l.selected = None;
                    }
                    if l.show_changes {
                        p.rect_filled(rect, 0.0, visuals.selection.bg_fill);
                    } else if resp.hovered() {
                        p.rect_filled(rect, 0.0, visuals.widgets.hovered.weak_bg_fill);
                    }
                    // The WIP node sits in the HEAD commit's lane, joined to it by a dashed line.
                    let (col, color) = wip_lane;
                    let x = graph_x + 8.0 + (col.min(max_col) as f32 + 0.5) * LANE_W;
                    let c = PALETTE[color as usize % PALETTE.len()];
                    dashed_vline(&p, x, cy + 5.5, rect.bottom(), c);
                    let ring: Vec<Pos2> = (0..=24)
                        .map(|k| {
                            let t = k as f32 / 24.0 * std::f32::consts::TAU;
                            Pos2::new(x + 5.0 * t.cos(), cy + 5.0 * t.sin())
                        })
                        .collect();
                    p.extend(egui::Shape::dashed_line(&ring, Stroke::new(1.8_f32, c), 3.0, 2.5));
                    p.text(
                        Pos2::new(graph_x + graph_w + 2.0, cy),
                        egui::Align2::LEFT_CENTER,
                        match l.changes.as_ref().and_then(|c| c.operation()) {
                            Some((op, 0)) => format!("// WIP  ({op} in progress: resolved, ready to continue)"),
                            Some((op, n)) => format!("// WIP  ({op} in progress: {n} file(s) with conflicts, click to resolve)"),
                            None => format!("// WIP  ({wip_count} changed)"),
                        },
                        font.clone(),
                        if l.changes.as_ref().and_then(|c| c.operation()).is_some_and(|(_, n)| n > 0) { Color32::from_rgb(0xe5, 0x53, 0x4b) } else { weak },
                    );
                    continue;
                }

                let idx = i - wip;
                let row = &l.rows[idx];
                if resp.clicked() {
                    let m = ui.input(|i| i.modifiers);
                    if m.command || m.shift {
                        mclick = Some((idx, m.command, m.shift));
                    } else {
                        pending = Some((idx, false));
                    }
                }
                let id = row.id;
                let multi_n = if l.multi.contains(&idx) { l.multi.len() } else { 0 };
                resp.context_menu(|ui| {
                    let mut pick = |a: CommitAction, ui: &mut egui::Ui| {
                        *act = Some(a);
                        ui.close_menu();
                    };
                    if ui.button("Create branch here…").clicked() {
                        pick(CommitAction::CreateBranch(id), ui);
                    }
                    if ui.button("Create tag here…").clicked() {
                        pick(CommitAction::CreateTag(id), ui);
                    }
                    if ui.button("Check out this commit (detached)").clicked() {
                        pick(CommitAction::Checkout(id), ui);
                    }
                    ui.separator();
                    if ui.button("Cherry-pick").clicked() {
                        pick(CommitAction::CherryPick(id), ui);
                    }
                    if ui.button("Revert").clicked() {
                        pick(CommitAction::Revert(id), ui);
                    }
                    if multi_n > 1 && ui.button(format!("Squash {multi_n} selected commits…")).clicked() {
                        pick(CommitAction::Squash, ui);
                    }
                    if ui.button("Interactive rebase from here…").clicked() {
                        pick(CommitAction::InteractiveRebase(id), ui);
                    }
                    ui.menu_button("Reset current branch to here", |ui| {
                        if ui.button("Soft (keep index and files)").clicked() {
                            pick(CommitAction::Reset(id, ResetMode::Soft), ui);
                        }
                        if ui.button("Mixed (keep files)").clicked() {
                            pick(CommitAction::Reset(id, ResetMode::Mixed), ui);
                        }
                        if ui.button("Hard (discard changes)…").clicked() {
                            pick(CommitAction::Reset(id, ResetMode::Hard), ui);
                        }
                    });
                    ui.separator();
                    if ui.button("Copy commit SHA").clicked() {
                        pick(CommitAction::CopySha(id), ui);
                    }
                });
                if l.search.matches.binary_search(&(idx as u32)).is_ok() {
                    p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(0xe5, 0xc0, 0x7b, 40));
                }
                if l.selected == Some(idx) || l.multi.contains(&idx) {
                    p.rect_filled(rect, 0.0, visuals.selection.bg_fill);
                } else if resp.hovered() {
                    p.rect_filled(rect, 0.0, visuals.widgets.hovered.weak_bg_fill);
                }

                let wip_line = (wip == 1)
                    .then_some(l.head_row)
                    .flatten()
                    .filter(|&h| idx <= h)
                    .map(|h| (wip_lane.0, wip_lane.1, idx == h));
                draw_graph(&p, rect, GraphCols { x: graph_x, width: graph_w, max_col }, row, l.info.as_ref().map(|i| &i.refs), wip_line);

                // Right-aligned fixed columns, flexible message in between.
                let date_x = rect.right() - DATE_W;
                let author_x = date_x - AUTHOR_W;
                p.text(
                    Pos2::new(date_x, cy),
                    egui::Align2::LEFT_CENTER,
                    timefmt::relative(row.time, now),
                    font.clone(),
                    weak,
                );
                let author_clip = Rect::from_min_max(
                    Pos2::new(author_x, rect.top()),
                    Pos2::new(date_x - 6.0, rect.bottom()),
                );
                ui.painter_at(author_clip).text(
                    Pos2::new(author_x, cy),
                    egui::Align2::LEFT_CENTER,
                    &*row.author,
                    font.clone(),
                    weak,
                );

                // Ref chips live in their own column left of the graph.
                let ref_clip = Rect::from_min_max(
                    Pos2::new(rect.left(), rect.top()),
                    Pos2::new(graph_x - 4.0, rect.bottom()),
                );
                let rp = ui.painter_at(ref_clip);
                let mut x = rect.left() + 4.0;
                if let Some(labels) = l.info.as_ref().and_then(|i| i.refs.get(&row.id)) {
                    for label in labels {
                        // Hidden branches and tags can still label a commit that other refs reach.
                        if !l.hidden.is_empty() && l.hidden.contains(&label.full_name()) {
                            continue;
                        }
                        let (bg, fg) = match label.kind {
                            RefKind::Head => (Color32::from_rgb(0x2e, 0x7d, 0x32), Color32::WHITE),
                            RefKind::Local => (Color32::from_rgb(0x1f, 0x6f, 0xa8), Color32::WHITE),
                            RefKind::Remote => (Color32::from_rgb(0x6a, 0x4c, 0x93), Color32::WHITE),
                            RefKind::Tag => (Color32::from_rgb(0xb2, 0x8a, 0x1e), Color32::BLACK),
                        };
                        let galley = rp.layout_no_wrap(label.name.clone(), small.clone(), fg);
                        let chip = Rect::from_min_size(
                            Pos2::new(x, cy - 8.0),
                            galley.size() + Vec2::new(10.0, 4.0),
                        );
                        rp.rect_filled(chip, 4.0, bg);
                        rp.galley(chip.min + Vec2::new(5.0, 2.0), galley, fg);
                        x = chip.right() + 4.0;
                    }
                }

                let msg_x = graph_x + graph_w + 2.0;
                let msg_clip = Rect::from_min_max(
                    Pos2::new(msg_x, rect.top()),
                    Pos2::new(author_x - 6.0, rect.bottom()),
                );
                ui.painter_at(msg_clip).text(
                    Pos2::new(msg_x, cy),
                    egui::Align2::LEFT_CENTER,
                    &*row.summary,
                    font.clone(),
                    text_color,
                );
            }
        });

    if let Some((idx, cmd, shift)) = mclick {
        if shift {
            let from = l.anchor.or(l.selected).unwrap_or(idx);
            l.multi = (from.min(idx)..=from.max(idx)).collect();
        } else if cmd {
            if l.multi.is_empty() {
                l.multi.extend(l.selected);
            }
            if !l.multi.remove(&idx) {
                l.multi.insert(idx);
            }
            l.anchor = Some(idx);
        }
        l.selected = Some(idx);
        match l.multi.len() {
            0 => {}
            1 => {
                // Back to a single commit: show it normally.
                let only = *l.multi.iter().next().unwrap();
                l.select(&ctx, only);
            }
            _ => {
                l.detail = None;
                l.show_changes = false;
            }
        }
    }
    if let Some((idx, reveal)) = pending {
        l.select(&ctx, idx);
        if reveal {
            l.reveal(idx, false);
        }
    }
}

/// The right panel while several commits are selected.
pub(super) fn multi_panel(ui: &mut egui::Ui, l: &Loaded) -> Option<MultiAction> {
    let mut action = None;
    ui.add_space(6.0);
    ui.heading(format!("{} commits selected", l.multi.len()));
    ui.weak("Cmd/Ctrl-click adds or removes a commit; Shift-click selects a range.");
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button(egui::RichText::new("Squash into one commit…").strong()).clicked() {
            action = Some(MultiAction::Squash);
        }
        if ui.button("Clear selection").clicked() {
            action = Some(MultiAction::Clear);
        }
    });
    ui.add_space(6.0);
    ui.separator();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        // Newest first, like the graph.
        for &i in &l.multi {
            if let Some(r) = l.rows.get(i) {
                ui.horizontal(|ui| {
                    detail::hash_link(ui, &r.id.to_string()[..8], &r.id.to_string());
                    ui.add(egui::Label::new(&*r.summary).truncate());
                });
            }
        }
    });
    action
}

/// A vertical dashed line (dash and gap divide the row height evenly, so it
/// stays continuous across rows).
fn dashed_vline(p: &egui::Painter, x: f32, y0: f32, y1: f32, color: Color32) {
    let stroke = Stroke::new(2.0_f32, color);
    let mut y = y0;
    while y < y1 {
        p.line_segment([Pos2::new(x, y), Pos2::new(x, (y + 3.0).min(y1))], stroke);
        y += 6.0;
    }
}

/// Where the graph column sits and how many lanes fit in it.
#[derive(Clone, Copy)]
struct GraphCols {
    x: f32,
    width: f32,
    max_col: u16,
}

fn draw_graph(
    p: &egui::Painter,
    rect: Rect,
    cols: GraphCols,
    row: &CommitRow,
    refs: Option<&std::collections::HashMap<gitgoing_core::Oid, Vec<history::RefLabel>>>,
    wip_line: Option<(u16, u8, bool)>,
) {
    let GraphCols { x: graph_x, width: graph_w, max_col } = cols;
    let clip = Rect::from_min_size(Pos2::new(graph_x, rect.top()), Vec2::new(graph_w, ROW_H));
    let p = p.with_clip_rect(clip.intersect(p.clip_rect()));
    let cx = |col: u16| graph_x + 8.0 + (col.min(max_col) as f32 + 0.5) * LANE_W;
    let top = rect.top();
    let mid = rect.center().y;
    let bot = rect.bottom();
    let color = |c: u8| PALETTE[c as usize % PALETTE.len()];
    let node = cx(row.graph.node_col);

    // Dashed line from the WIP row down to the HEAD commit.
    if let Some((col, color, ends_here)) = wip_line {
        let c = PALETTE[color as usize % PALETTE.len()];
        dashed_vline(&p, cx(col), top, if ends_here { mid - 6.0 } else { bot }, c);
    }

    for s in &row.graph.segs {
        let stroke = Stroke::new(2.0_f32, color(s.color));
        let (a, b) = match s.kind {
            SegKind::Pass => (Pos2::new(cx(s.from), top), Pos2::new(cx(s.to), bot)),
            SegKind::In => (Pos2::new(cx(s.from), top), Pos2::new(node, mid)),
            SegKind::Out => (Pos2::new(node, mid), Pos2::new(cx(s.to), bot)),
        };
        p.line_segment([a, b], stroke);
    }

    let c = color(row.graph.node_color);
    let is_head = refs
        .and_then(|r| r.get(&row.id))
        .is_some_and(|l| l.iter().any(|l| l.kind == RefKind::Head));
    let center = Pos2::new(node, mid);
    if is_head {
        p.circle_filled(center, 6.0, c);
        p.circle_stroke(center, 6.0, Stroke::new(2.0_f32, Color32::WHITE));
    } else {
        p.circle_filled(center, 4.5, c);
    }
}

