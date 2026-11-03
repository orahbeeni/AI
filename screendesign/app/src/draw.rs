//! Device frames, screens, thumbnails and shared drawing helpers.
//! The individual controls live in controls.rs.

use crate::catalog;
use crate::controls::draw_kind;
use crate::icons::icon;
use crate::model::{parse_hex, Align, Control, Project, State};
use crate::pen::*;

// ---------------------------------------------------------------- text items

pub struct Item {
    pub label: String,
    pub active: bool,
    pub disabled: bool,
    pub icon: Option<String>,
    pub extra: String,
}

pub fn parse_item(s: &str) -> Item {
    let mut t = s.trim();
    let mut active = false;
    let mut disabled = false;
    if let Some(r) = t.strip_prefix('*') {
        active = true;
        t = r.trim_start();
    } else if t.starts_with('-') && !t.starts_with("---") {
        disabled = true;
        t = t[1..].trim_start();
    }
    let mut icon = None;
    if let Some(r) = t.strip_prefix(':') {
        let (tok, rest) = r.split_once(' ').unwrap_or((r, ""));
        if catalog::is_icon(tok) {
            icon = Some(tok.to_string());
            t = rest.trim_start();
        }
    }
    let (label, extra) = t.split_once('\t').unwrap_or((t, ""));
    Item { label: label.trim_end().to_string(), active, disabled, icon, extra: extra.trim().to_string() }
}

pub fn items(text: &str) -> Vec<Item> {
    text.split(',').filter(|s| !s.trim().is_empty()).map(parse_item).collect()
}

pub fn lines(text: &str) -> Vec<Item> {
    text.lines().map(parse_item).collect()
}

pub fn numbers(text: &str, fallback: &[f32]) -> Vec<f32> {
    let v: Vec<f32> = text.split([',', ' ', '\n']).filter_map(|s| s.trim().parse().ok()).collect();
    if v.is_empty() {
        fallback.to_vec()
    } else {
        v
    }
}

/// Icon (if any) then the label, laid out around hx in the box.
pub fn icon_label(pen: &mut Pen, r: R, it: &Item, size: f32, color: Rgba, hx: f32) {
    let isz = size * 1.25;
    let gap = if it.label.is_empty() { 0.0 } else { size * 0.45 };
    let iw = if it.icon.is_some() { isz + gap } else { 0.0 };
    let label = pen.fit(&it.label, (r.w - iw).max(0.0), size, pen.bold);
    let tw = pen.tw(&label, size, pen.bold);
    let total = iw + tw;
    let x0 = (r.x + (r.w - total) * hx).max(r.x);
    if let Some(name) = &it.icon {
        icon(pen, name, R::new(x0, r.cy() - isz / 2.0, isz, isz), color);
    }
    pen.text(x0 + iw, r.cy(), &label, size, color, 0.0);
}

pub fn shadow(pen: &mut Pen, r: R, rad: f32) {
    let c = [0, 0, 0, 30];
    let pts = Pen::round_pts(R::new(r.x + 3.0, r.y + 5.0, r.w, r.h), rad);
    pen.fill_pts(pts, c, true);
}

pub fn hx_of(c: &Control, default: f32) -> f32 {
    match c.align {
        Align::Auto => default,
        Align::Left => 0.0,
        Align::Center => 0.5,
        Align::Right => 1.0,
    }
}

pub fn dashed_round(pen: &mut Pen, r: R, rad: f32, w: f32, col: Rgba) {
    let pts = Pen::round_pts(r, rad);
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        pen.dashed(a[0], a[1], b[0], b[1], w, col);
    }
}

pub fn mid_or(pen: &Pen, fg: Rgba) -> Rgba {
    if pen.ink_over.is_some() {
        fg
    } else {
        mix(pen.ink, pen.paper, 0.4)
    }
}

pub fn soft_line(pen: &Pen) -> Rgba {
    mix(pen.soft, pen.mid, 0.4)
}

pub fn transform(prims: &mut [Prim], ox: f32, oy: f32, sx: f32, sy: f32) {
    let ts = (sx * sy).sqrt();
    for p in prims.iter_mut() {
        match p {
            Prim::Path { pts, stroke, .. } => {
                for q in pts.iter_mut() {
                    q[0] = ox + q[0] * sx;
                    q[1] = oy + q[1] * sy;
                }
                if let Some((w, _)) = stroke {
                    *w = (*w * ts.sqrt()).max(0.7);
                }
            }
            Prim::Text { x, y, size, .. } => {
                *x = ox + *x * sx;
                *y = oy + *y * sy;
                *size *= ts;
            }
            Prim::Link { r, .. } => {
                *r = R::new(ox + r.x * sx, oy + r.y * sy, r.w * sx, r.h * sy);
            }
        }
    }
}

// ---------------------------------------------------------------- device frames

/// Space a frame takes around its screen: left, top, right, bottom.
pub fn frame_extent(style: &str) -> (f32, f32, f32, f32) {
    match style {
        "phone" => (14.0, 14.0, 14.0, 14.0),
        "android" => (11.0, 11.0, 11.0, 11.0),
        "classic" => (16.0, 70.0, 16.0, 70.0),
        "foldable" | "flip" => (13.0, 13.0, 13.0, 13.0),
        "flip_cover" => (16.0, 16.0, 16.0, 150.0),
        "tablet" => (22.0, 22.0, 22.0, 22.0),
        "laptop" => (76.0, 24.0, 76.0, 48.0),
        "desktop" => (18.0, 18.0, 18.0, 124.0),
        "browser" => (1.0, 80.0, 1.0, 1.0),
        "window" => (1.0, 34.0, 1.0, 1.0),
        "watch" => (18.0, 64.0, 28.0, 64.0),
        "watch_round" => (20.0, 60.0, 30.0, 60.0),
        "tv" => (12.0, 12.0, 12.0, 44.0),
        "car" => (20.0, 20.0, 20.0, 20.0),
        "spatial" => (6.0, 6.0, 6.0, 36.0),
        _ => (0.0, 0.0, 0.0, 0.0),
    }
}

pub fn screen_radius(style: &str) -> f32 {
    match style {
        "phone" => 42.0,
        "android" => 28.0,
        "foldable" | "flip" | "car" => 16.0,
        "flip_cover" => 26.0,
        "tablet" => 14.0,
        "spatial" => 28.0,
        "watch" => 40.0,
        _ => 0.0,
    }
}

const TRAFFIC: [Rgba; 3] = [[236, 95, 87, 255], [244, 190, 80, 255], [98, 196, 98, 255]];
pub const DARK: Rgba = [24, 24, 26, 255];

/// Draws the device around the screen area `c` (the screen itself is not filled).
pub fn frame(pen: &mut Pen, style: &str, c: R, title: &str) {
    let ink = pen.ink;
    let body: Rgba = if pen.paper[0] < 128 { [18, 18, 20, 255] } else { [246, 246, 246, 255] };
    let (l, t, r, b) = frame_extent(style);
    let lw = pen.lw();
    let around = |pen: &mut Pen, rad: f32, b: f32| {
        pen.rect(R::new(c.x - l, c.y - t, c.w + l + r, c.h + t + b), rad, Some(body), Some(ink));
    };
    match style {
        "phone" => {
            around(pen, 54.0, b);
            for (yy, hh) in [(0.15, 0.04), (0.22, 0.07), (0.31, 0.07)] {
                pen.rect(R::new(c.x - l - 3.0, c.y + c.h * yy, 3.0, c.h * hh), 1.0, Some(body), Some(ink));
            }
            pen.rect(R::new(c.r() + r, c.y + c.h * 0.25, 3.0, c.h * 0.1), 1.0, Some(body), Some(ink));
            pen.rect(c, 42.0, None, Some(ink));
        }
        "android" => {
            around(pen, 38.0, b);
            pen.rect(R::new(c.r() + r, c.y + c.h * 0.2, 3.0, c.h * 0.08), 1.0, Some(body), Some(ink));
            pen.rect(R::new(c.r() + r, c.y + c.h * 0.32, 3.0, c.h * 0.14), 1.0, Some(body), Some(ink));
            pen.rect(c, 28.0, None, Some(ink));
        }
        "classic" => {
            around(pen, 40.0, b);
            pen.rect(c, 0.0, None, Some(ink));
            pen.rect(R::new(c.cx() - 30.0, c.y - 38.0, 60.0, 7.0), 3.5, Some(body), Some(ink));
            pen.circle(c.cx() - 50.0, c.y - 35.0, 4.0, None, Some(ink));
            pen.circle(c.cx(), c.b() + 35.0, 22.0, None, Some(ink));
        }
        "foldable" | "flip" => {
            around(pen, 26.0, b);
            pen.rect(c, 16.0, None, Some(ink));
        }
        "flip_cover" => {
            around(pen, 34.0, b);
            pen.rect(c, 26.0, None, Some(ink));
            let hinge = c.b() + b * 0.55;
            pen.line(c.x - l, hinge, c.r() + r, hinge, 1.5, ink);
            pen.circle(c.x + 30.0, c.b() + 34.0, 9.0, Some(DARK), None);
            pen.circle(c.x + 58.0, c.b() + 34.0, 9.0, Some(DARK), None);
        }
        "tablet" => {
            around(pen, 36.0, b);
            pen.rect(c, 14.0, None, Some(ink));
            pen.circle(c.cx(), c.y - t / 2.0, 3.5, Some(DARK), None);
        }
        "laptop" => {
            let lid = R::new(c.x - 16.0, c.y - t, c.w + 32.0, c.h + t + 26.0);
            pen.rect(lid, 16.0, Some(body), Some(ink));
            pen.rect(c, 0.0, None, Some(ink));
            pen.circle(c.cx(), c.y - t / 2.0, 3.0, Some(DARK), None);
            let by = lid.b();
            let base = vec![[c.x - l, by], [c.r() + r, by], [c.r() + r - 14.0, by + 20.0], [c.x - l + 14.0, by + 20.0]];
            pen.fill_pts(base.clone(), body, true);
            pen.stroke_pts(&base, true, lw, ink);
            let m = pen.mid;
            pen.line(c.cx() - 50.0, by + 2.0, c.cx() + 50.0, by + 2.0, 3.0, m);
        }
        "desktop" => {
            let mon = R::new(c.x - l, c.y - t, c.w + l + r, c.h + t + 46.0);
            pen.rect(mon, 14.0, Some(body), Some(ink));
            pen.rect(c, 0.0, None, Some(ink));
            let neck = vec![[c.cx() - 40.0, mon.b()], [c.cx() + 40.0, mon.b()], [c.cx() + 52.0, mon.b() + 64.0], [c.cx() - 52.0, mon.b() + 64.0]];
            pen.fill_pts(neck.clone(), body, true);
            pen.stroke_pts(&neck, true, lw, ink);
            pen.rect(R::new(c.cx() - 130.0, mon.b() + 64.0, 260.0, 14.0), 7.0, Some(body), Some(ink));
        }
        "browser" => {
            let outer = R::new(c.x - l, c.y - t, c.w + l + r, c.h + t + b);
            pen.rect(outer, 10.0, Some(body), Some(ink));
            let soft = pen.soft;
            let paper = pen.paper;
            for (i, col) in TRAFFIC.iter().enumerate() {
                pen.circle(outer.x + 18.0 + i as f32 * 18.0, outer.y + 20.0, 6.0, Some(*col), None);
            }
            let tab = R::new(outer.x + 84.0, outer.y + 6.0, 220.0f32.min(c.w * 0.4), 30.0);
            pen.rect(tab, 8.0, Some(paper), Some(ink));
            let small = 12.5;
            pen.text_in(tab.pad(12.0, 0.0, 26.0, 0.0), title, small, ink, 0.0);
            icon(pen, "close", R::new(tab.r() - 22.0, tab.cy() - 6.0, 12.0, 12.0), ink);
            let bar_y = outer.y + 40.0;
            let m = pen.mid;
            pen.line(outer.x, bar_y, outer.r(), bar_y, 1.0, m);
            for (i, name) in ["arrow_left", "arrow_right", "refresh"].iter().enumerate() {
                icon(pen, name, R::new(outer.x + 14.0 + i as f32 * 28.0, bar_y + 9.0, 18.0, 18.0), ink);
            }
            let addr = R::new(outer.x + 104.0, bar_y + 6.0, (c.w - 160.0).max(40.0), 26.0);
            pen.rect(addr, 13.0, Some(soft), None);
            icon(pen, "lock", R::new(addr.x + 10.0, addr.cy() - 6.0, 12.0, 12.0), ink);
            let slug: String = title.to_lowercase().replace(' ', "-");
            let url = format!("example.com/{slug}");
            pen.text_in(addr.pad(30.0, 0.0, 10.0, 0.0), &url, small, ink, 0.0);
            icon(pen, "more_v", R::new(outer.r() - 34.0, bar_y + 9.0, 18.0, 18.0), ink);
            pen.line(outer.x, c.y, outer.r(), c.y, 1.0, ink);
        }
        "window" => {
            let outer = R::new(c.x - l, c.y - t, c.w + l + r, c.h + t + b);
            shadow(pen, outer, 10.0);
            pen.rect(outer, 10.0, Some(body), Some(ink));
            for (i, col) in TRAFFIC.iter().enumerate() {
                pen.circle(outer.x + 18.0 + i as f32 * 18.0, outer.y + 17.0, 6.0, Some(*col), None);
            }
            let b0 = pen.bold;
            pen.bold = true;
            pen.text_in(R::new(outer.x + 70.0, outer.y, outer.w - 140.0, 34.0), title, 13.0, ink, 0.5);
            pen.bold = b0;
            pen.line(outer.x, c.y, outer.r(), c.y, 1.0, ink);
        }
        "watch" => {
            let band = R::new(c.x + c.w * 0.12, c.y - t, c.w * 0.76, c.h + t + b);
            pen.rect(band, 12.0, Some(body), Some(ink));
            for i in 0..3 {
                pen.circle(c.cx(), c.b() + 30.0 + i as f32 * 9.0, 2.0, Some(ink), None);
            }
            pen.rect(R::new(c.x - 18.0, c.y - 18.0, c.w + 36.0, c.h + 36.0), 54.0, Some(body), Some(ink));
            pen.rect(R::new(c.r() + 18.0, c.y + c.h * 0.22, 9.0, 34.0), 4.0, Some(body), Some(ink));
            pen.rect(c, 40.0, None, Some(ink));
        }
        "watch_round" => {
            let band = R::new(c.x + c.w * 0.2, c.y - t, c.w * 0.6, c.h + t + b);
            pen.rect(band, 10.0, Some(body), Some(ink));
            pen.circle(c.cx(), c.cy(), c.w / 2.0 + 20.0, Some(body), Some(ink));
            pen.rect(R::new(c.r() + 18.0, c.cy() - 16.0, 11.0, 32.0), 4.0, Some(body), Some(ink));
        }
        "tv" => {
            around(pen, 6.0, 12.0);
            pen.rect(c, 0.0, None, Some(ink));
            let fy = c.b() + 12.0;
            for x in [c.x + c.w * 0.18, c.r() - c.w * 0.18] {
                pen.stroke_pts(&[[x - 18.0, fy + 30.0], [x, fy], [x + 18.0, fy + 30.0]], false, lw, ink);
            }
        }
        "car" => {
            around(pen, 30.0, b);
            pen.rect(c, 16.0, None, Some(ink));
        }
        "spatial" => {
            let glass = R::new(c.x - l, c.y - t, c.w + l + r, c.h + t + 6.0);
            let m = pen.mid;
            pen.rect(glass, 34.0, Some(with_alpha(body, 0.6)), Some(m));
            pen.rect(c, 28.0, None, Some(ink));
            pen.rect(R::new(c.cx() - 60.0, c.b() + 18.0, 120.0, 9.0), 4.5, Some(m), None);
            pen.circle(c.cx() + 78.0, c.b() + 22.5, 5.0, Some(m), None);
        }
        _ => {
            shadow(pen, c, 2.0);
        }
    }
}

/// Hardware drawn over the screen content (camera cut-outs, creases, round mask).
pub fn frame_overlay(pen: &mut Pen, style: &str, c: R) {
    let body: Rgba = if pen.paper[0] < 128 { [18, 18, 20, 255] } else { [246, 246, 246, 255] };
    let ink = pen.ink;
    let s = (c.w / 393.0).clamp(0.45, 1.3);
    match style {
        "phone" => {
            pen.rect(R::new(c.cx() - 62.0 * s, c.y + 11.0 * s, 124.0 * s, 36.0 * s), 18.0 * s, Some(DARK), None);
            pen.rect(R::new(c.cx() - 70.0 * s, c.b() - 12.0 * s, 140.0 * s, 5.0 * s), 3.0, Some(ink), None);
        }
        "android" => {
            pen.circle(c.cx(), c.y + 18.0 * s, 7.0 * s, Some(DARK), None);
            pen.rect(R::new(c.cx() - 54.0 * s, c.b() - 10.0 * s, 108.0 * s, 4.0 * s), 2.0, Some(ink), None);
        }
        "foldable" => {
            let m = pen.mid;
            pen.dashed(c.cx(), c.y + 8.0, c.cx(), c.b() - 8.0, 1.2, m);
            pen.circle(c.r() - 26.0, c.y + 16.0, 5.0, Some(DARK), None);
        }
        "flip" => {
            let m = pen.mid;
            pen.dashed(c.x + 8.0, c.cy(), c.r() - 8.0, c.cy(), 1.2, m);
            pen.circle(c.cx(), c.y + 16.0, 5.0, Some(DARK), None);
        }
        "watch_round" => {
            // cover whatever pokes out of the round screen, up to the bezel's edge
            let rad = c.w.min(c.h) / 2.0;
            let outer = rad + 20.0;
            let n = 96;
            let ray = |a: f32, d: f32| [c.cx() + d * a.cos(), c.cy() + d * a.sin()];
            for i in 0..n {
                let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
                let a1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
                let edge = |a: f32| (rad / a.cos().abs().max(a.sin().abs())).min(outer);
                let (e0, e1) = (edge(a0), edge(a1));
                if e0 <= rad + 0.5 && e1 <= rad + 0.5 {
                    continue;
                }
                pen.fill_pts(vec![ray(a0, rad - 0.5), ray(a0, e0), ray(a1, e1), ray(a1, rad - 0.5)], body, true);
            }
            pen.circle(c.cx(), c.cy(), rad, None, Some(ink));
        }
        _ => {}
    }
}

// ---------------------------------------------------------------- screens

pub struct Env<'p> {
    pub project: &'p Project,
    pub depth: u32,
}

pub fn frame_style(device: &str) -> &'static str {
    catalog::device(device).map(|d| d.frame).unwrap_or("none")
}

/// Draws a whole screen in its own coordinates (0,0 is the screen's top-left).
pub fn screen(pen: &mut Pen, env: &Env, idx: usize, with_frame: bool) {
    let s = &env.project.screens[idx];
    pen.set_dark(s.dark);
    let area = R::new(0.0, 0.0, s.w, s.h);
    let style = frame_style(&s.device);
    if with_frame {
        pen.seed(idx as u64 * 977 + 3);
        frame(pen, style, area, &s.name);
    }
    let paper = pen.paper;
    let rad = if style == "watch_round" && with_frame { s.w.min(s.h) / 2.0 } else { screen_radius(style) };
    pen.rect(area, rad, Some(paper), None);
    controls_of(pen, env, idx);
    if with_frame {
        frame_overlay(pen, style, area);
    }
}

/// The controls of a screen (and its background screen) without frame or paper.
pub fn controls_of(pen: &mut Pen, env: &Env, idx: usize) {
    let s = &env.project.screens[idx];
    if env.depth < 4 {
        if let Some(bg) = s.background.as_deref().and_then(|b| env.project.screen_idx(b)) {
            if bg != idx {
                let sub = Env { project: env.project, depth: env.depth + 1 };
                controls_of(pen, &sub, bg);
            }
        }
    }
    for c in &s.controls {
        if !c.hidden {
            control(pen, env, c);
        }
    }
}

pub fn default_font(kind: &str) -> f32 {
    match kind {
        "heading" => 28.0,
        "subheading" => 20.0,
        "code" => 13.0,
        "badge" | "tag" | "status_bar" | "breadcrumbs" | "pagination" | "notification" | "tooltip" | "bottom_nav" => 13.0,
        "callout" => 14.0,
        _ => 15.0,
    }
}

pub fn control(pen: &mut Pen, env: &Env, c: &Control) {
    pen.seed(c.id);
    pen.alpha = c.opacity.unwrap_or(1.0).clamp(0.05, 1.0);
    pen.fill_over = c.fill.as_deref().and_then(parse_hex);
    pen.ink_over = c.color.as_deref().and_then(parse_hex);
    if c.state == State::Disabled && pen.ink_over.is_none() && c.kind != "alert" {
        pen.ink_over = Some(pen.mid);
    }
    pen.font = c.font_size.unwrap_or_else(|| default_font(&c.kind));
    pen.bold = c.bold || matches!(c.kind.as_str(), "heading" | "subheading");
    pen.italic = c.italic;
    pen.underline = c.underline;
    let r = R::new(c.x, c.y, c.w, c.h);
    draw_kind(pen, env, c, r);
    if let Some(l) = &c.link {
        pen.link(r, l);
    }
    pen.alpha = 1.0;
    pen.fill_over = None;
    pen.ink_over = None;
    pen.bold = false;
    pen.italic = false;
    pen.underline = false;
}

/// Small preview of a control kind, fitted into `area` (used by the library strip).
pub fn preview(pen: &mut Pen, project: &Project, kind: &str, area: R) {
    let mut scratch = Project { version: 1, name: String::new(), screens: vec![], next_id: 900_000, versions: vec![] };
    let mut c = scratch.make_control(kind, 0.0, 0.0);
    c.id = kind.bytes().fold(7u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64));
    if kind == "component" {
        c.component = project.screens.iter().find(|s| s.is_component).map(|s| s.id.clone());
    }
    let env = Env { project, depth: 0 };
    let start = pen.out.len();
    control(pen, &env, &c);
    let s = (area.w / c.w).min(area.h / c.h).min(1.0);
    let ox = area.cx() - c.w * s / 2.0;
    let oy = area.cy() - c.h * s / 2.0;
    transform(&mut pen.out[start..], ox, oy, s, s);
    pen.out.retain(|p| !matches!(p, Prim::Link { .. }));
}

/// Draws one screen as a thumbnail fitted into `area` (screens list and flow map).
pub fn thumbnail(pen: &mut Pen, project: &Project, idx: usize, area: R) {
    let s = &project.screens[idx];
    let start = pen.out.len();
    let env = Env { project, depth: 0 };
    screen(pen, &env, idx, false);
    let k = (area.w / s.w).min(area.h / s.h);
    let ox = area.cx() - s.w * k / 2.0;
    let oy = area.cy() - s.h * k / 2.0;
    transform(&mut pen.out[start..], ox, oy, k, k);
    pen.out.retain(|p| !matches!(p, Prim::Link { .. }));
    pen.set_dark(false);
}
