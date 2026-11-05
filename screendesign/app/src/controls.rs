//! Basic, text, input and navigation controls. The rest are in controls2.rs.

use crate::draw::*;
use crate::icons::icon;
use crate::model::{parse_hex, Control, State};
use crate::pen::*;

pub fn draw_kind(pen: &mut Pen, env: &Env, c: &Control, r: R) {
    let fs = pen.font;
    let fg = pen.fg();
    let mid = pen.mid;
    let soft = pen.soft;
    let paper = pen.paper;
    let acc = pen.accent;
    let lw = pen.lw();
    let sel = c.state == State::Selected;
    let focus = c.state == State::Focused;
    let text = c.text.as_str();
    let kind = c.kind.as_str();
    if let Some(style) = kind.strip_prefix("frame_") {
        let (l, t, rr, b) = frame_extent(style);
        let inner = r.pad(l, t, rr, b);
        let title = if text.is_empty() { "Untitled" } else { text };
        frame(pen, style, inner, title);
        let bgc = pen.bg();
        pen.rect(inner, if style == "watch_round" { inner.w.min(inner.h) / 2.0 } else { screen_radius(style) }, Some(bgc), None);
        frame_overlay(pen, style, inner);
        return;
    }
    match kind {
        "rect" => {
            pen.boxed(r, 0.0);
            centered_lines(pen, r, text, fs, fg);
        }
        "circle" => {
            let b = pen.bg();
            pen.ellipse(r, Some(b), Some(fg));
            centered_lines(pen, r, text, fs, fg);
        }
        "hline" => pen.ink_line(r.x, r.cy(), r.r(), r.cy()),
        "vline" => pen.ink_line(r.cx(), r.y, r.cx(), r.b()),
        "arrow" => {
            let y = r.cy();
            pen.ink_line(r.x, y, r.r(), y);
            let hs = 9.0f32.min(r.w / 3.0);
            pen.stroke_pts(&[[r.r() - hs, y - hs * 0.7], [r.r(), y], [r.r() - hs, y + hs * 0.7]], false, lw, fg);
            if c.value >= 1.0 {
                pen.stroke_pts(&[[r.x + hs, y - hs * 0.7], [r.x, y], [r.x + hs, y + hs * 0.7]], false, lw, fg);
            }
            if !text.is_empty() {
                pen.text(r.cx(), y - fs * 0.85, text, fs * 0.9, fg, 0.5);
            }
        }
        "bracket" => {
            let (x0, x1, xm) = (r.r() - 2.0, r.x + 2.0, r.cx());
            pen.stroke_pts(
                &[[x0, r.y], [xm, r.y + 6.0], [xm, r.cy() - 6.0], [x1, r.cy()], [xm, r.cy() + 6.0], [xm, r.b() - 6.0], [x0, r.b()]],
                false,
                lw,
                fg,
            );
        }
        "image" => {
            let b = pen.bg_or(soft);
            pen.rect(r, 0.0, Some(b), Some(fg));
            pen.line(r.x, r.y, r.r(), r.b(), 1.0, mid);
            pen.line(r.r(), r.y, r.x, r.b(), 1.0, mid);
            let s = (r.w.min(r.h) * 0.3).min(48.0);
            pen.rect(R::new(r.cx() - s * 0.6, r.cy() - s * 0.6, s * 1.2, s * 1.2), 4.0, Some(b), None);
            icon(pen, "image", R::new(r.cx() - s / 2.0, r.cy() - s / 2.0, s, s), fg);
            if !text.is_empty() {
                pen.text_in(R::new(r.x, r.b() - fs * 1.6, r.w, fs * 1.4), text, fs * 0.85, fg, 0.5);
            }
        }
        "picture" => match c.image.as_deref().and_then(|id| env.project.images.get(id)) {
            Some(img) => {
                if let Some(f) = pen.fill_over {
                    pen.rect(r, 0.0, Some(f), None);
                }
                pen.image(r, img.clone(), c.fit);
            }
            None => {
                dashed_round(pen, r, 4.0, 1.0, mid);
                let s = (r.w.min(r.h) * 0.3).min(40.0);
                icon(pen, "image", R::new(r.cx() - s / 2.0, r.cy() - s / 2.0 - 6.0, s, s), mid);
                if r.h > 60.0 && r.w > 90.0 {
                    pen.text_in(R::new(r.x, r.cy() + s / 2.0 - 2.0, r.w, fs * 1.6), "Double-click to choose", fs * 0.8, mid, 0.5);
                }
            }
        },
        "icon" => icon(pen, text.trim(), r, fg),
        "container" => {
            pen.boxed(r, 4.0);
            let first = text.lines().next().unwrap_or("");
            if !first.is_empty() {
                let bar = R::new(r.x, r.y, r.w, (fs * 2.0).min(r.h));
                pen.rect(bar.inset(1.0), 3.0, Some(soft), None);
                pen.ink_line(r.x, bar.b(), r.r(), bar.b());
                pen.bold = true;
                pen.text_in(bar.pad(10.0, 0.0, 10.0, 0.0), first, fs, fg, hx_of(c, 0.0));
                pen.bold = c.bold;
                let rest: Vec<&str> = text.lines().skip(1).collect();
                pen.para(r.pad(10.0, bar.h + 8.0, 10.0, 6.0), &rest.join("\n"), fs, fg, hx_of(c, 0.0));
            }
        }
        "card" => {
            pen.boxed(r, 10.0);
            let ih = (r.h * 0.45).min(r.h - 50.0).max(0.0);
            if ih > 20.0 {
                let img = R::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, ih);
                pen.rect(img, 9.0, Some(soft), None);
                pen.line(img.x, img.b(), img.r(), img.b(), 1.0, mid);
                let s = (ih * 0.35).min(44.0);
                icon(pen, "image", R::new(img.cx() - s / 2.0, img.cy() - s / 2.0, s, s), mid);
            }
            let mut ls = text.lines();
            let title = ls.next().unwrap_or("");
            let body: Vec<&str> = ls.collect();
            let tr = r.pad(14.0, ih + 12.0, 14.0, 10.0);
            pen.bold = true;
            pen.text_in(R::new(tr.x, tr.y, tr.w, fs * 1.5), title, fs * 1.1, fg, 0.0);
            pen.bold = c.bold;
            let mc = mid_or(pen, fg);
            pen.para(tr.pad(0.0, fs * 1.8, 0.0, 0.0), &body.join("\n"), fs * 0.9, mc, 0.0);
        }
        "sticky" => {
            let col = pen.fill_over.unwrap_or(STICKY);
            let fold = 18.0f32.min(r.w / 3.0);
            shadow(pen, r, 0.0);
            let pts = vec![[r.x, r.y], [r.r(), r.y], [r.r(), r.b() - fold], [r.r() - fold, r.b()], [r.x, r.b()]];
            pen.fill_pts(pts.clone(), col, true);
            pen.stroke_pts(&pts, true, 1.0, mix(col, INK, 0.35));
            pen.fill_pts(vec![[r.r(), r.b() - fold], [r.r() - fold, r.b() - fold], [r.r() - fold, r.b()]], mix(col, INK, 0.15), true);
            let ink = pen.ink_over.unwrap_or(INK);
            pen.para(r.pad(10.0, 10.0, 10.0, fold), text, fs, ink, hx_of(c, 0.0));
        }
        "callout" => {
            let col = pen.fill_over.unwrap_or(acc);
            pen.ellipse(r, Some(col), None);
            pen.bold = true;
            let tc = pen.ink_over.unwrap_or(PAPER);
            pen.text_in(r, text, fs.min(r.h * 0.6), tc, 0.5);
        }
        "scribble" => {
            let n = ((r.w / 6.0) as usize).max(1);
            let pts: Vec<[f32; 2]> = (0..=n)
                .map(|i| {
                    let t = i as f32 / n as f32;
                    [r.x + r.w * t, r.cy() + (t * 40.0).sin() * r.h * 0.35 * (1.0 - (t - 0.5).abs())]
                })
                .collect();
            pen.stroke_pts(&pts, false, lw, fg);
        }
        "heading" | "subheading" | "label" => {
            if let Some(f) = pen.fill_over {
                pen.rect(r, 0.0, Some(f), None);
            }
            let hx = hx_of(c, 0.0);
            let n = text.lines().count().max(1);
            if n == 1 {
                let it = parse_item(text);
                icon_label(pen, r, &it, fs, fg, hx);
            } else {
                let lh = fs * 1.3;
                let y0 = r.cy() - lh * n as f32 / 2.0;
                for (i, l) in text.lines().enumerate() {
                    pen.text_in(R::new(r.x, y0 + i as f32 * lh, r.w, lh), l, fs, fg, hx);
                }
            }
        }
        "paragraph" => {
            if let Some(f) = pen.fill_over {
                pen.rect(r, 0.0, Some(f), None);
            }
            pen.para(r, text, fs, fg, hx_of(c, 0.0));
        }
        "link" => {
            let col = pen.ink_over.unwrap_or(acc);
            pen.underline = true;
            let it = parse_item(text);
            icon_label(pen, r, &it, fs, col, hx_of(c, 0.0));
        }
        "link_bar" => {
            let its = items(text);
            let mut x = r.x;
            for (i, it) in its.iter().enumerate() {
                if i > 0 {
                    pen.text(x + 8.0, r.cy(), "|", fs, mid, 0.5);
                    x += 16.0;
                }
                let w = pen.tw(&it.label, fs, it.active);
                pen.underline = !it.active;
                pen.bold = it.active;
                let col = if it.active { fg } else if it.disabled { mid } else { acc };
                pen.text(x, r.cy(), &it.label, fs, col, 0.0);
                x += w;
            }
            pen.underline = false;
            pen.bold = c.bold;
        }
        "code" => {
            let b = pen.bg_or(soft);
            pen.rect(r, 6.0, Some(b), Some(mid));
            let lh = fs * 1.4;
            for (i, l) in text.lines().enumerate() {
                let y = r.y + 10.0 + lh * (i as f32 + 0.5);
                if y > r.b() - 4.0 {
                    break;
                }
                pen.text_ex(r.x + 12.0, y, l, fs, fg, 0.0, false, true);
            }
        }
        "quote" => {
            pen.rect(R::new(r.x, r.y, 4.0, r.h), 2.0, Some(acc), None);
            let mut body: Vec<&str> = text.lines().collect();
            let by = if body.last().map(|l| l.starts_with('-')).unwrap_or(false) { body.pop() } else { None };
            pen.italic = true;
            let used = pen.para(r.pad(16.0, 2.0, 0.0, 0.0), &body.join("\n"), fs * 1.1, fg, 0.0);
            pen.italic = false;
            if let Some(b) = by {
                pen.text(r.x + 16.0, r.y + used + fs, b, fs * 0.85, mid, 0.0);
            }
        }
        "bullets" => {
            let lh = fs * 1.5;
            for (i, l) in text.lines().enumerate() {
                let y = r.y + lh * (i as f32 + 0.5);
                if y > r.b() {
                    break;
                }
                let l = l.trim();
                let numbered = l.split_once(". ").map(|(n, _)| n.parse::<u32>().is_ok()).unwrap_or(false);
                if numbered {
                    pen.text(r.x, y, l, fs, fg, 0.0);
                } else {
                    pen.circle(r.x + 5.0, y, 2.6, Some(fg), None);
                    pen.text(r.x + 16.0, y, l, fs, fg, 0.0);
                }
            }
        }
        "button" | "button_primary" => {
            let primary = kind == "button_primary";
            let rad = 8.0f32.min(r.h / 2.0);
            if primary {
                let col = if c.state == State::Disabled { mid } else { pen.acc() };
                let edge = if pen.sketch { mix(col, INK, 0.3) } else { col };
                pen.rect(r, rad, Some(col), Some(edge));
            } else {
                let b = if sel { pen.bg_or(soft) } else { pen.bg() };
                pen.rect(r, rad, Some(b), Some(fg));
            }
            if focus {
                pen.rect(r.inset(-3.0), rad + 3.0, None, Some(acc));
            }
            let tc = if primary { pen.ink_over.unwrap_or(PAPER) } else { fg };
            let it = parse_item(text);
            icon_label(pen, r.pad(8.0, 0.0, 8.0, 0.0), &it, fs, tc, hx_of(c, 0.5));
        }
        "icon_button" => {
            let b = if sel { pen.bg_or(soft) } else { pen.bg() };
            pen.rect(r, 8.0f32.min(r.h / 2.0), Some(b), Some(fg));
            let s = r.w.min(r.h) * 0.55;
            icon(pen, text.trim(), R::new(r.cx() - s / 2.0, r.cy() - s / 2.0, s, s), fg);
        }
        "text_input" | "password" | "date_field" => {
            pen.boxed(r, 4.0);
            if focus {
                pen.rect(r, 4.0, None, Some(acc));
            }
            let inner = r.pad(10.0, 0.0, 10.0, 0.0);
            let shown = if kind == "password" && !text.eq_ignore_ascii_case("password") && !text.is_empty() {
                "•".repeat(text.chars().count().min(14))
            } else {
                text.to_string()
            };
            let col = if focus || kind == "date_field" { fg } else { mid_or(pen, fg) };
            let right = if kind == "text_input" { 0.0 } else { 24.0 };
            pen.text_in(inner.pad(0.0, 0.0, right, 0.0), &shown, fs, col, hx_of(c, 0.0));
            if focus {
                let x = inner.x + pen.tw(&shown, fs, false) + 2.0;
                pen.line(x, r.cy() - fs * 0.6, x, r.cy() + fs * 0.6, 1.4, acc);
            }
            if kind != "text_input" {
                let s = (r.h * 0.45).min(20.0);
                let ic = if kind == "password" { "eye" } else { "calendar" };
                icon(pen, ic, R::new(r.r() - s - 10.0, r.cy() - s / 2.0, s, s), fg);
            }
        }
        "text_area" => {
            pen.boxed(r, 4.0);
            pen.para(r.pad(10.0, 8.0, 18.0, 8.0), text, fs, fg, 0.0);
            pen.rect(R::new(r.r() - 10.0, r.y + 6.0, 4.0, (r.h * 0.35).max(10.0)), 2.0, Some(mid), None);
        }
        "search" | "search_bar" => {
            let rad = r.h / 2.0;
            if kind == "search_bar" {
                let b = pen.bg_or(soft);
                pen.rect(r, 10.0f32.min(rad), Some(b), None);
            } else {
                let b = pen.bg();
                pen.rect(r, rad, Some(b), Some(fg));
            }
            let s = (r.h * 0.45).min(20.0);
            let mc = mid_or(pen, fg);
            icon(pen, "search", R::new(r.x + 12.0, r.cy() - s / 2.0, s, s), mc);
            pen.text_in(r.pad(20.0 + s, 0.0, 12.0 + s, 0.0), text, fs, mc, 0.0);
            if kind == "search_bar" {
                icon(pen, "mic", R::new(r.r() - s - 12.0, r.cy() - s / 2.0, s, s), mc);
            }
        }
        "dropdown" => {
            pen.boxed(r, 4.0);
            let first = text.lines().next().unwrap_or("");
            pen.text_in(r.pad(10.0, 0.0, r.h + 4.0, 0.0), first, fs, fg, 0.0);
            pen.ink_line(r.r() - r.h, r.y + 4.0, r.r() - r.h, r.b() - 4.0);
            let s = r.h * 0.45;
            icon(pen, if focus { "chevron_up" } else { "chevron_down" }, R::new(r.r() - r.h / 2.0 - s / 2.0, r.cy() - s / 2.0, s, s), fg);
            if focus {
                let opts: Vec<Item> = text.lines().skip(1).map(parse_item).collect();
                let lh = fs * 2.0;
                let list = R::new(r.x, r.b() + 2.0, r.w, lh * opts.len().max(1) as f32);
                shadow(pen, list, 4.0);
                pen.boxed(list, 4.0);
                for (i, it) in opts.iter().enumerate() {
                    let row = R::new(list.x, list.y + i as f32 * lh, list.w, lh);
                    if it.active {
                        pen.rect(row.inset(2.0), 3.0, Some(acc), None);
                    }
                    let col = if it.active { PAPER } else if it.disabled { mid } else { fg };
                    icon_label(pen, row.pad(10.0, 0.0, 10.0, 0.0), it, fs, col, 0.0);
                }
            }
        }
        "checkbox" => check_row(pen, r, text, sel, false, fs, fg),
        "radio" => check_row(pen, r, text, sel, true, fs, fg),
        "checkbox_group" | "radio_group" => {
            let radio = kind == "radio_group";
            let ls: Vec<&str> = text.lines().collect();
            let lh = (r.h / ls.len().max(1) as f32).min(fs * 2.0);
            for (i, l) in ls.iter().enumerate() {
                let l = l.trim();
                let (on, dis, label) = if let Some(rest) = l.strip_prefix("[x]").or_else(|| l.strip_prefix("(o)")) {
                    (true, false, rest)
                } else if let Some(rest) = l.strip_prefix("[-]").or_else(|| l.strip_prefix("(-)")) {
                    (false, true, rest)
                } else if let Some(rest) = l.strip_prefix("[ ]").or_else(|| l.strip_prefix("( )")) {
                    (false, false, rest)
                } else {
                    (false, false, l)
                };
                let row = R::new(r.x, r.y + i as f32 * lh, r.w, lh);
                check_row(pen, row, label.trim(), on, radio, fs, if dis { mid } else { fg });
            }
        }
        "toggle" => {
            let sw = R::new(r.r() - 46.0, r.cy() - 13.0, 46.0, 26.0);
            pen.text_in(r.pad(0.0, 0.0, 54.0, 0.0), text, fs, fg, 0.0);
            if sel {
                let col = pen.acc();
                pen.rect(sw, 13.0, Some(col), None);
                pen.circle(sw.r() - 13.0, sw.cy(), 10.0, Some(PAPER), None);
            } else {
                pen.rect(sw, 13.0, Some(soft), Some(fg));
                pen.circle(sw.x + 13.0, sw.cy(), 10.0, Some(paper), Some(fg));
            }
        }
        "slider" => {
            let v = (c.value / 100.0).clamp(0.0, 1.0);
            let x = r.x + 10.0 + (r.w - 20.0) * v;
            pen.rect(R::new(r.x + 10.0, r.cy() - 2.5, r.w - 20.0, 5.0), 2.5, Some(soft), Some(fg));
            let col = pen.acc();
            pen.rect(R::new(r.x + 10.0, r.cy() - 2.5, x - r.x - 10.0, 5.0), 2.5, Some(col), None);
            pen.circle(x, r.cy(), 9.0, Some(paper), Some(fg));
        }
        "stepper" => {
            pen.boxed(r, 6.0);
            let bw = r.h.min(r.w / 3.0);
            pen.ink_line(r.x + bw, r.y, r.x + bw, r.b());
            pen.ink_line(r.r() - bw, r.y, r.r() - bw, r.b());
            let s = bw * 0.45;
            icon(pen, "minus", R::new(r.x + bw / 2.0 - s / 2.0, r.cy() - s / 2.0, s, s), fg);
            icon(pen, "plus", R::new(r.r() - bw / 2.0 - s / 2.0, r.cy() - s / 2.0, s, s), fg);
            pen.text_in(r.pad(bw, 0.0, bw, 0.0), text, fs, fg, 0.5);
        }
        "date_picker" => calendar(pen, r, text, c.value, fs, fg),
        "time_picker" => {
            pen.boxed(r, 10.0);
            let band = R::new(r.x + 8.0, r.cy() - fs, r.w - 16.0, fs * 2.0);
            pen.rect(band, 8.0, Some(soft), None);
            let (h, m) = text.split_once(':').unwrap_or((text, "00"));
            let hv: i32 = h.trim().parse().unwrap_or(9);
            let mv: i32 = m.trim().parse().unwrap_or(41);
            for (col, v, modn) in [(0.3f32, hv, 24), (0.7, mv, 60)] {
                for d in -2i32..=2 {
                    let y = r.cy() + d as f32 * fs * 1.6;
                    if y < r.y + 8.0 || y > r.b() - 8.0 {
                        continue;
                    }
                    let n = (v + d).rem_euclid(modn);
                    let colr = if d == 0 { fg } else { mid };
                    pen.text_ex(r.x + r.w * col, y, &format!("{n:02}"), if d == 0 { fs * 1.3 } else { fs }, colr, 0.5, d == 0, false);
                }
            }
            pen.text(r.cx(), r.cy(), ":", fs * 1.3, fg, 0.5);
        }
        "otp" => {
            let its: Vec<&str> = text.split(',').collect();
            let n = its.len().max(1) as f32;
            let gap = 8.0;
            let bw = ((r.w - gap * (n - 1.0)) / n).max(4.0);
            for (i, v) in its.iter().enumerate() {
                let b = R::new(r.x + i as f32 * (bw + gap), r.y, bw, r.h);
                let filled = v.trim() != "_" && !v.trim().is_empty();
                let next = !filled && (i == 0 || its[i - 1].trim() != "_");
                let bg = pen.bg();
                pen.rect(b, 6.0, Some(bg), Some(if next { acc } else { fg }));
                if filled {
                    pen.bold = true;
                    pen.text_in(b, v.trim(), fs * 1.3, fg, 0.5);
                    pen.bold = c.bold;
                }
            }
        }
        "chips" => {
            let mut x = r.x;
            for it in items(text) {
                let w = pen.tw(&it.label, fs * 0.9, false) + 28.0 + if it.active { 16.0 } else { 0.0 };
                if x + w > r.r() + 1.0 {
                    break;
                }
                let chip = R::new(x, r.y, w, r.h);
                if it.active {
                    let col = mix(pen.acc(), paper, 0.75);
                    pen.rect(chip, r.h / 2.0, Some(col), Some(fg));
                    icon(pen, "check", R::new(chip.x + 10.0, chip.cy() - 7.0, 14.0, 14.0), fg);
                    pen.text(chip.x + 28.0, chip.cy(), &it.label, fs * 0.9, fg, 0.0);
                } else {
                    let bg = pen.bg();
                    let col = if it.disabled { mid } else { fg };
                    pen.rect(chip, r.h / 2.0, Some(bg), Some(col));
                    pen.text(chip.cx(), chip.cy(), &it.label, fs * 0.9, col, 0.5);
                }
                x += w + 8.0;
            }
        }
        "segmented" => {
            let its = items(text);
            pen.boxed(r, 8.0f32.min(r.h / 2.0));
            let n = its.len().max(1) as f32;
            let sw = r.w / n;
            for (i, it) in its.iter().enumerate() {
                let seg = R::new(r.x + sw * i as f32, r.y, sw, r.h);
                if it.active {
                    let a = pen.acc();
                    pen.rect(seg.inset(3.0), 6.0f32.min(r.h / 2.0), Some(a), None);
                } else if i > 0 && !its[i - 1].active {
                    pen.line(seg.x, seg.y + 7.0, seg.x, seg.b() - 7.0, 1.0, mid);
                }
                let col = if it.active { PAPER } else if it.disabled { mid } else { fg };
                icon_label(pen, seg.pad(6.0, 0.0, 6.0, 0.0), it, fs * 0.92, col, 0.5);
            }
        }
        "rating" => {
            let s = r.h.min(r.w / 5.0);
            for i in 0..5 {
                let b = R::new(r.x + i as f32 * s, r.cy() - s / 2.0, s, s);
                if (i as f32) < c.value.round() {
                    let col = pen.fill_over.unwrap_or(ORANGE);
                    icon(pen, "star_fill", b, col);
                }
                icon(pen, "star", b, fg);
            }
        }
        "color_picker" => {
            let cols: Vec<Rgba> = text.split(',').filter_map(parse_hex).collect();
            let s = r.h.min(r.w / cols.len().max(1) as f32 - 6.0).max(6.0);
            for (i, col) in cols.iter().enumerate() {
                let cx = r.x + s / 2.0 + i as f32 * (s + 6.0);
                pen.circle(cx, r.cy(), s / 2.0, Some(*col), Some(fg));
                if i == 0 && sel {
                    pen.circle(cx, r.cy(), s / 2.0 + 3.0, None, Some(fg));
                }
            }
        }
        "file_drop" => {
            let b = pen.bg();
            pen.rect(r, 10.0, Some(b), None);
            dashed_round(pen, r, 10.0, 1.4, fg);
            let s = (r.h * 0.28).min(40.0);
            icon(pen, "upload", R::new(r.cx() - s / 2.0, r.cy() - s * 0.9, s, s), fg);
            pen.text_in(R::new(r.x, r.cy() + s * 0.3, r.w, fs * 1.6), text, fs, fg, 0.5);
        }
        "captcha" => {
            let b = pen.bg_or(soft);
            pen.rect(r, 4.0, Some(b), Some(mid));
            let bx = R::new(r.x + 16.0, r.cy() - 13.0, 26.0, 26.0);
            pen.rect(bx, 3.0, Some(paper), Some(fg));
            if sel {
                icon(pen, "check", bx.inset(3.0), GREEN);
            }
            pen.text_in(r.pad(54.0, 0.0, 70.0, 0.0), text, fs, fg, 0.0);
            icon(pen, "refresh", R::new(r.r() - 52.0, r.cy() - 18.0, 30.0, 30.0), acc);
        }
        "tabs" => {
            let its = items(text);
            let th = (fs * 2.3).min(r.h);
            let body = R::new(r.x, r.y + th, r.w, r.h - th);
            pen.boxed(body, 0.0);
            let mut x = r.x;
            for it in its.iter() {
                let w = pen.tw(&it.label, fs, false) + 28.0 + if it.icon.is_some() { fs * 1.6 } else { 0.0 };
                let tab = R::new(x, r.y + if it.active { 0.0 } else { 3.0 }, w, th - if it.active { 0.0 } else { 3.0 });
                let b = if it.active { pen.bg() } else { soft };
                pen.rect(tab, 0.0, Some(b), None);
                pen.stroke_pts(&[[tab.x, tab.b()], [tab.x, tab.y], [tab.r(), tab.y], [tab.r(), tab.b()]], false, lw, fg);
                if it.active {
                    pen.line(tab.x + 1.5, tab.b(), tab.r() - 1.5, tab.b(), 3.0, b);
                }
                icon_label(pen, tab, it, fs, if it.disabled { mid } else { fg }, 0.5);
                x += w - 1.0;
            }
        }
        "breadcrumbs" => {
            let parts: Vec<&str> = text.split('>').map(|s| s.trim()).collect();
            let mut x = r.x;
            for (i, p) in parts.iter().enumerate() {
                let last = i + 1 == parts.len();
                pen.underline = !last;
                pen.bold = last;
                let col = if last { fg } else { acc };
                pen.text(x, r.cy(), p, fs, col, 0.0);
                x += pen.tw(p, fs, last);
                pen.underline = false;
                pen.bold = false;
                if !last {
                    icon(pen, "chevron_right", R::new(x + 4.0, r.cy() - fs * 0.4, fs * 0.8, fs * 0.8), mid);
                    x += fs * 0.8 + 8.0;
                }
            }
        }
        "pagination" => {
            let its = items(text);
            let s = r.h;
            for (i, it) in its.iter().enumerate() {
                let b = R::new(r.x + i as f32 * (s + 6.0), r.y, s, s);
                if b.r() > r.r() + 1.0 {
                    break;
                }
                if it.active {
                    let a = pen.acc();
                    pen.rect(b, 6.0, Some(a), None);
                } else if it.label != "…" {
                    let bg = pen.bg();
                    pen.rect(b, 6.0, Some(bg), Some(fg));
                }
                let sz = s * 0.42;
                let ic = match it.label.as_str() {
                    "<" => Some("chevron_left"),
                    ">" => Some("chevron_right"),
                    _ => None,
                };
                if let Some(ic) = ic {
                    icon(pen, ic, R::new(b.cx() - sz / 2.0, b.cy() - sz / 2.0, sz, sz), fg);
                } else {
                    pen.text_in(b, &it.label, fs, if it.active { PAPER } else { fg }, 0.5);
                }
            }
        }
        "menu" => {
            shadow(pen, r, 6.0);
            pen.boxed(r, 6.0);
            let lh = fs * 2.0;
            let mut y = r.y + 4.0;
            for it in lines(text) {
                if it.label.starts_with("---") {
                    pen.line(r.x + 8.0, y + 5.0, r.r() - 8.0, y + 5.0, 1.0, mid);
                    y += 10.0;
                    continue;
                }
                if y + lh > r.b() + 1.0 {
                    break;
                }
                let row = R::new(r.x + 4.0, y, r.w - 8.0, lh);
                if it.active {
                    let a = pen.acc();
                    pen.rect(row, 4.0, Some(a), None);
                }
                let col = if it.active { PAPER } else if it.disabled { mid } else { fg };
                icon_label(pen, row.pad(10.0, 0.0, 10.0, 0.0), &it, fs, col, 0.0);
                if !it.extra.is_empty() {
                    pen.text(row.r() - 10.0, row.cy(), &it.extra, fs * 0.85, if it.active { PAPER } else { mid }, 1.0);
                }
                y += lh;
            }
        }
        "menubar" => {
            let b = pen.bg_or(soft);
            pen.rect(r, 0.0, Some(b), None);
            pen.ink_line(r.x, r.b(), r.r(), r.b());
            let mut x = r.x + 6.0;
            for it in items(text) {
                let w = pen.tw(&it.label, fs, false) + 20.0;
                let cell = R::new(x, r.y + 2.0, w, r.h - 4.0);
                if it.active {
                    let a = pen.acc();
                    pen.rect(cell, 4.0, Some(a), None);
                }
                pen.text_in(cell, &it.label, fs, if it.active { PAPER } else if it.disabled { mid } else { fg }, 0.5);
                x += w;
            }
        }
        "navbar" => {
            let b = pen.bg();
            pen.rect(r, 0.0, Some(b), None);
            pen.ink_line(r.x, r.b(), r.r(), r.b());
            let mut ls = text.lines();
            let brand = ls.next().unwrap_or("");
            let links = ls.next().unwrap_or("");
            let cta = ls.next().unwrap_or("");
            let lg = (r.h * 0.5).min(32.0);
            pen.rect(R::new(r.x + 24.0, r.cy() - lg / 2.0, lg, lg), 8.0, Some(fg), None);
            pen.bold = true;
            pen.text(r.x + 34.0 + lg, r.cy(), brand, fs * 1.2, fg, 0.0);
            pen.bold = c.bold;
            let mut x = r.x + 70.0 + lg + pen.tw(brand, fs * 1.2, true);
            for it in items(links) {
                let w = pen.tw(&it.label, fs, it.active);
                pen.bold = it.active;
                pen.text(x, r.cy(), &it.label, fs, if it.disabled { mid } else { fg }, 0.0);
                if it.active {
                    pen.line(x, r.b() - 4.0, x + w, r.b() - 4.0, 3.0, acc);
                }
                pen.bold = false;
                x += w + 28.0;
            }
            if !cta.is_empty() {
                let w = pen.tw(cta, fs, false) + 32.0;
                let btn = R::new(r.r() - 24.0 - w, r.cy() - 18.0, w, 36.0);
                let a = pen.acc();
                pen.rect(btn, 8.0, Some(a), None);
                pen.text_in(btn, cta, fs, PAPER, 0.5);
            }
        }
        "sidebar" | "list" | "timeline" => list_like(pen, r, kind, text, fs, fg),
        "vertical_tabs" => {
            let ls = lines(text);
            let cw = (r.w * 0.36).max(60.0);
            pen.boxed(r, 4.0);
            pen.ink_line(r.x + cw, r.y, r.x + cw, r.b());
            let lh = fs * 2.3;
            for (i, it) in ls.iter().enumerate() {
                let row = R::new(r.x, r.y + i as f32 * lh, cw, lh);
                if it.active {
                    pen.rect(row.inset(3.0), 4.0, Some(soft), None);
                    pen.rect(R::new(row.x + 3.0, row.y + 6.0, 3.0, lh - 12.0), 1.5, Some(acc), None);
                }
                icon_label(pen, row.pad(12.0, 0.0, 4.0, 0.0), it, fs, if it.disabled { mid } else { fg }, 0.0);
            }
            pen.lorem_bars(r.pad(cw + 14.0, 14.0, 14.0, 14.0), 5);
        }
        "steps" => {
            let its = items(text);
            let n = its.len().max(1);
            let active = its.iter().position(|i| i.active).unwrap_or(0);
            let seg = r.w / n as f32;
            let cy = r.y + 14.0;
            for i in 0..n.saturating_sub(1) {
                let cx = r.x + seg * (i as f32 + 0.5);
                pen.line(cx + 14.0, cy, cx + seg - 14.0, cy, 2.0, if i < active { acc } else { mid });
            }
            for (i, it) in its.iter().enumerate() {
                let cx = r.x + seg * (i as f32 + 0.5);
                if i < active {
                    pen.circle(cx, cy, 12.0, Some(acc), None);
                    icon(pen, "check", R::new(cx - 7.0, cy - 7.0, 14.0, 14.0), PAPER);
                } else if i == active {
                    pen.circle(cx, cy, 12.0, Some(paper), Some(acc));
                    pen.text(cx, cy, &(i + 1).to_string(), fs * 0.85, acc, 0.5);
                } else {
                    pen.circle(cx, cy, 12.0, Some(paper), Some(mid));
                    pen.text(cx, cy, &(i + 1).to_string(), fs * 0.85, mid, 0.5);
                }
                pen.bold = i == active;
                let lbl = pen.fit(&it.label, seg - 4.0, fs * 0.9, i == active);
                pen.text(cx, cy + 26.0, &lbl, fs * 0.9, if i > active { mid } else { fg }, 0.5);
                pen.bold = false;
            }
        }
        "accordion" => {
            let ls: Vec<&str> = text.lines().collect();
            let lh = fs * 2.4;
            let mut y = r.y;
            pen.boxed(r, 6.0);
            let mc = mid_or(pen, fg);
            let mut i = 0;
            while i < ls.len() {
                let it = parse_item(ls[i]);
                let row = R::new(r.x, y, r.w, lh);
                if row.b() > r.b() + 1.0 {
                    break;
                }
                pen.bold = true;
                pen.text_in(row.pad(14.0, 0.0, 36.0, 0.0), &it.label, fs, fg, 0.0);
                pen.bold = false;
                let s = fs * 0.8;
                icon(pen, if it.active { "chevron_up" } else { "chevron_down" }, R::new(row.r() - 14.0 - s, row.cy() - s / 2.0, s, s), fg);
                y += lh;
                if it.active && i + 1 < ls.len() {
                    let used = pen.wrap(ls[i + 1], r.w - 28.0, fs * 0.92, false).len() as f32 * fs * 0.92 * 1.35;
                    pen.para(R::new(r.x + 14.0, y - 4.0, r.w - 28.0, used), ls[i + 1], fs * 0.92, mc, 0.0);
                    y += used + 8.0;
                    i += 1;
                }
                if y < r.b() - 2.0 {
                    pen.line(r.x, y, r.r(), y, 1.0, mid);
                }
                i += 1;
            }
        }
        "tree" => {
            let lh = fs * 1.7;
            let hl = mix(pen.accent, paper, 0.75);
            for (i, raw) in text.lines().enumerate() {
                let indent = raw.len() - raw.trim_start().len();
                let mut t = raw.trim();
                let row = R::new(r.x, r.y + i as f32 * lh, r.w, lh);
                if row.b() > r.b() + 1.0 {
                    break;
                }
                if let Some(rest) = t.strip_prefix('*') {
                    t = rest.trim_start();
                    pen.rect(row, 3.0, Some(hl), None);
                }
                let x = r.x + 6.0 + indent as f32 * 7.0;
                let s = fs * 0.75;
                let (arrow, name, folder) = if let Some(n) = t.strip_prefix("v ") {
                    (Some("chevron_down"), n, true)
                } else if let Some(n) = t.strip_prefix("> ") {
                    (Some("chevron_right"), n, true)
                } else {
                    (None, t, false)
                };
                if let Some(a) = arrow {
                    icon(pen, a, R::new(x, row.cy() - s / 2.0, s, s), fg);
                }
                icon(pen, if folder { "folder" } else { "file" }, R::new(x + s + 4.0, row.cy() - s * 0.6, s * 1.2, s * 1.2), fg);
                pen.text(x + s * 2.4 + 8.0, row.cy(), name, fs, fg, 0.0);
            }
        }
        "command_palette" => {
            shadow(pen, r, 12.0);
            pen.boxed(r, 12.0);
            let mut ls = text.lines();
            let q = ls.next().unwrap_or("");
            let head = R::new(r.x, r.y, r.w, fs * 3.0);
            icon(pen, "search", R::new(head.x + 16.0, head.cy() - 9.0, 18.0, 18.0), mid);
            let mc = mid_or(pen, fg);
            pen.text_in(head.pad(44.0, 0.0, 16.0, 0.0), q, fs * 1.1, mc, 0.0);
            pen.line(r.x, head.b(), r.r(), head.b(), 1.0, mid);
            let lh = fs * 2.3;
            for (i, l) in ls.enumerate() {
                let it = parse_item(l);
                let row = R::new(r.x + 6.0, head.b() + 6.0 + i as f32 * lh, r.w - 12.0, lh);
                if row.b() > r.b() {
                    break;
                }
                if it.active {
                    pen.rect(row, 6.0, Some(soft), None);
                }
                icon_label(pen, row.pad(10.0, 0.0, 60.0, 0.0), &it, fs, fg, 0.0);
                if !it.extra.is_empty() {
                    pen.text(row.r() - 10.0, row.cy(), &it.extra, fs * 0.85, mid, 1.0);
                }
            }
        }
        _ => crate::controls2::draw_more(pen, env, c, r),
    }
}

fn list_like(pen: &mut Pen, r: R, kind: &str, text: &str, fs: f32, fg: Rgba) {
    let mid = pen.mid;
    let soft = pen.soft;
    let paper = pen.paper;
    let acc = pen.accent;
    if kind == "sidebar" {
        let b = pen.bg_or(soft);
        pen.rect(r, 0.0, Some(b), None);
        pen.ink_line(r.r(), r.y, r.r(), r.b());
    } else if kind == "list" {
        pen.boxed(r, 6.0);
    }
    let ls = lines(text);
    let lh = if kind == "sidebar" { fs * 2.6 } else { (r.h / ls.len().max(1) as f32).clamp(fs * 1.8, fs * 3.4) };
    let active_at = ls.iter().position(|x| x.active);
    let hl = mix(acc, paper, 0.75);
    let sl = soft_line(pen);
    for (i, it) in ls.iter().enumerate() {
        let row = R::new(r.x, r.y + i as f32 * lh + if kind == "sidebar" { 12.0 } else { 0.0 }, r.w, lh);
        if row.b() > r.b() + 1.0 {
            break;
        }
        let col = if it.disabled { mid } else { fg };
        match kind {
            "timeline" => {
                let done = active_at.map(|a| i <= a).unwrap_or(false);
                if i + 1 < ls.len() {
                    let lc = if active_at.map(|a| i < a).unwrap_or(false) { acc } else { mid };
                    pen.line(r.x + 10.0, row.cy() + 7.0, r.x + 10.0, row.cy() + lh - 7.0, 2.0, lc);
                }
                pen.circle(r.x + 10.0, row.cy(), 7.0, Some(if done { acc } else { paper }), Some(if done { acc } else { fg }));
                pen.bold = it.active;
                pen.text_in(row.pad(30.0, 0.0, 0.0, 0.0), &it.label, fs, col, 0.0);
                pen.bold = false;
            }
            "sidebar" => {
                let inner = row.pad(10.0, 3.0, 10.0, 3.0);
                if it.active {
                    pen.rect(inner, 8.0, Some(hl), None);
                }
                pen.bold = it.active;
                icon_label(pen, inner.pad(12.0, 0.0, 8.0, 0.0), it, fs, col, 0.0);
                pen.bold = false;
            }
            _ => {
                if it.active {
                    pen.rect(row.inset(2.0), 4.0, Some(hl), None);
                }
                icon_label(pen, row.pad(14.0, 0.0, 30.0, 0.0), it, fs, col, 0.0);
                if !it.extra.is_empty() {
                    pen.text(row.r() - 14.0, row.cy(), &it.extra, fs * 0.85, mid, 1.0);
                } else {
                    let s = fs * 0.8;
                    icon(pen, "chevron_right", R::new(row.r() - 14.0 - s, row.cy() - s / 2.0, s, s), mid);
                }
                if i + 1 < ls.len() {
                    pen.line(row.x + 14.0, row.b(), row.r() - 14.0, row.b(), 1.0, sl);
                }
            }
        }
    }
}

pub fn centered_lines(pen: &mut Pen, r: R, text: &str, fs: f32, fg: Rgba) {
    if text.is_empty() {
        return;
    }
    let ls = pen.wrap(text, r.w - 12.0, fs, pen.bold);
    let lh = fs * 1.3;
    let y0 = r.cy() - lh * ls.len() as f32 / 2.0 + lh / 2.0;
    for (i, l) in ls.iter().enumerate() {
        pen.text(r.cx(), y0 + i as f32 * lh, l, fs, fg, 0.5);
    }
}

fn check_row(pen: &mut Pen, r: R, label: &str, on: bool, radio: bool, fs: f32, fg: Rgba) {
    let s = 16.0f32.min(r.h);
    let b = R::new(r.x + 1.0, r.cy() - s / 2.0, s, s);
    let paper = pen.paper;
    if radio {
        pen.ellipse(b, Some(paper), Some(fg));
        if on {
            let a = pen.acc();
            pen.ellipse(b.inset(4.0), Some(a), None);
        }
    } else {
        let f = if on { pen.acc() } else { paper };
        pen.rect(b, 3.0, Some(f), Some(fg));
        if on {
            icon(pen, "check", b.inset(1.5), PAPER);
        }
    }
    pen.text_in(R::new(b.r() + 8.0, r.y, (r.w - s - 10.0).max(0.0), r.h), label, fs, fg, 0.0);
}

fn calendar(pen: &mut Pen, r: R, title: &str, day: f32, fs: f32, fg: Rgba) {
    pen.boxed(r, 8.0);
    let mid = pen.mid;
    let head = R::new(r.x, r.y, r.w, fs * 2.6);
    pen.bold = true;
    pen.text_in(head, title, fs, fg, 0.5);
    pen.bold = false;
    icon(pen, "chevron_left", R::new(r.x + 12.0, head.cy() - 7.0, 14.0, 14.0), fg);
    icon(pen, "chevron_right", R::new(r.r() - 26.0, head.cy() - 7.0, 14.0, 14.0), fg);
    let cw = (r.w - 16.0) / 7.0;
    let rh = (r.h - head.h - 10.0) / 7.0;
    for (i, d) in ["M", "T", "W", "T", "F", "S", "S"].iter().enumerate() {
        pen.text(r.x + 8.0 + cw * (i as f32 + 0.5), head.b() + rh / 2.0, d, fs * 0.8, mid, 0.5);
    }
    let start = 6;
    for n in 1..=30 {
        let k = start + n - 1;
        let (col, row) = (k % 7, k / 7);
        let cx = r.x + 8.0 + cw * (col as f32 + 0.5);
        let cy = head.b() + rh * (row as f32 + 1.5);
        if n as f32 == day.round() {
            let a = pen.acc();
            pen.circle(cx, cy, rh.min(cw) * 0.45, Some(a), None);
            pen.text(cx, cy, &n.to_string(), fs * 0.85, PAPER, 0.5);
        } else {
            pen.text(cx, cy, &n.to_string(), fs * 0.85, fg, 0.5);
        }
    }
}
