//! Content, mobile, chart, media and feedback controls.

use crate::catalog;
use crate::draw::*;
use crate::icons::icon;
use crate::model::{Control, State};
use crate::pen::*;

pub fn draw_more(pen: &mut Pen, env: &Env, c: &Control, r: R) {
    let fs = pen.font;
    let fg = pen.fg();
    let mid = pen.mid;
    let soft = pen.soft;
    let paper = pen.paper;
    let acc = pen.accent;
    let lw = pen.lw();
    let sel = c.state == State::Selected;
    let text = c.text.as_str();
    let kind = c.kind.as_str();
    match kind {
        "table" => table(pen, r, text, fs, fg),
        "avatar" => {
            let b = pen.bg_or(soft);
            pen.ellipse(r, Some(b), Some(fg));
            if text.trim().is_empty() {
                icon(pen, "user", r.inset(r.w * 0.2), fg);
            } else {
                pen.bold = true;
                pen.text_in(r, text.trim(), (r.h * 0.38).max(8.0), fg, 0.5);
                pen.bold = false;
            }
        }
        "avatar_group" => {
            let s = r.h;
            for (i, t) in text.split(',').enumerate() {
                let b = R::new(r.x + i as f32 * s * 0.68, r.y, s, s);
                if b.r() > r.r() + 1.0 {
                    break;
                }
                let more = t.trim().starts_with('+');
                pen.circle(b.cx(), b.cy(), s / 2.0 + 2.0, Some(paper), None);
                pen.ellipse(b, Some(if more { mid } else { soft }), Some(fg));
                pen.text_in(b, t.trim(), s * 0.36, fg, 0.5);
            }
        }
        "badge" => {
            let col = pen.fill_over.unwrap_or(RED);
            pen.rect(r, r.h / 2.0, Some(col), None);
            pen.bold = true;
            let tc = pen.ink_over.unwrap_or(PAPER);
            pen.text_in(r, text, fs, tc, 0.5);
        }
        "tag" => {
            let b = pen.bg_or(soft);
            pen.rect(r, 6.0, Some(b), if sel { Some(fg) } else { None });
            pen.text_in(r.pad(6.0, 0.0, 6.0, 0.0), text, fs * 0.9, fg, 0.5);
        }
        "stat" => {
            pen.boxed(r, 10.0);
            let ls: Vec<&str> = text.lines().collect();
            let p = r.pad(16.0, 12.0, 16.0, 12.0);
            let mc = mid_or(pen, fg);
            if let Some(l) = ls.first() {
                pen.text(p.x, p.y + fs * 0.6, l, fs * 0.9, mc, 0.0);
            }
            if let Some(v) = ls.get(1) {
                pen.bold = true;
                pen.text(p.x, p.cy() + 2.0, v, fs * 2.0, fg, 0.0);
                pen.bold = false;
            }
            if let Some(d) = ls.get(2) {
                let up = !d.trim_start().starts_with('-');
                let col = if up { GREEN } else { RED };
                icon(pen, if up { "arrow_up" } else { "arrow_down" }, R::new(p.x, p.b() - fs * 1.1, fs, fs), col);
                pen.text(p.x + fs * 1.3, p.b() - fs * 0.6, d, fs * 0.9, col, 0.0);
            }
        }
        "chat" => {
            let mine = sel;
            let col = if mine { pen.acc() } else { pen.bg_or(soft) };
            let tc = if mine { pen.ink_over.unwrap_or(PAPER) } else { fg };
            let body = r.pad(if mine { 0.0 } else { 6.0 }, 0.0, if mine { 6.0 } else { 0.0 }, 0.0);
            pen.rect(body, 16.0f32.min(body.h / 2.0), Some(col), None);
            let tail = if mine {
                vec![[body.r() - 14.0, body.b() - 10.0], [r.r(), r.b()], [body.r() - 22.0, body.b() - 2.0]]
            } else {
                vec![[body.x + 14.0, body.b() - 10.0], [r.x, r.b()], [body.x + 22.0, body.b() - 2.0]]
            };
            pen.fill_pts(tail, col, true);
            pen.para(body.pad(14.0, 8.0, 14.0, 6.0), text, fs, tc, 0.0);
        }
        "comment_thread" => {
            pen.boxed(r, 10.0);
            let mut ls = text.lines();
            let who = ls.next().unwrap_or("");
            let initials: String = who.chars().take(1).collect();
            pen.circle(r.x + 26.0, r.y + 26.0, 14.0, Some(soft), Some(fg));
            pen.text(r.x + 26.0, r.y + 26.0, &initials, fs * 0.9, fg, 0.5);
            pen.bold = true;
            pen.text(r.x + 50.0, r.y + 26.0, who, fs * 0.9, fg, 0.0);
            pen.bold = false;
            let body: Vec<&str> = ls.collect();
            pen.para(r.pad(50.0, 42.0, 12.0, 6.0), &body.join("\n"), fs, fg, 0.0);
        }
        "pricing" => {
            if sel {
                let bg = pen.bg();
                pen.rect(r, 14.0, Some(bg), Some(acc));
                pen.rect(r.inset(1.5), 13.0, None, Some(acc));
            } else {
                pen.boxed(r, 14.0);
            }
            let ls: Vec<&str> = text.lines().collect();
            let p = r.pad(20.0, 18.0, 20.0, 18.0);
            pen.bold = true;
            if let Some(n) = ls.first() {
                pen.text(p.x, p.y + fs * 0.6, n, fs * 1.1, fg, 0.0);
            }
            if let Some(pr) = ls.get(1) {
                pen.text(p.x, p.y + fs * 2.6, pr, fs * 2.0, fg, 0.0);
            }
            pen.bold = false;
            let feats = if ls.len() > 3 { &ls[2..ls.len() - 1] } else { &[][..] };
            for (i, f) in feats.iter().enumerate() {
                let y = p.y + fs * 5.0 + i as f32 * fs * 1.8;
                icon(pen, "check", R::new(p.x, y - fs * 0.45, fs * 0.9, fs * 0.9), GREEN);
                pen.text(p.x + fs * 1.4, y, f, fs * 0.95, fg, 0.0);
            }
            if ls.len() > 2 {
                let btn = R::new(p.x, p.b() - 40.0, p.w, 40.0);
                let a = pen.acc();
                pen.rect(btn, 8.0, Some(a), None);
                pen.text_in(btn, ls[ls.len() - 1], fs, PAPER, 0.5);
            }
        }
        "kanban" => {
            let b = pen.bg_or(soft);
            pen.rect(r, 10.0, Some(b), None);
            let mut ls = text.lines();
            let title = ls.next().unwrap_or("");
            let cards: Vec<&str> = ls.collect();
            pen.bold = true;
            pen.text(r.x + 14.0, r.y + 20.0, title, fs, fg, 0.0);
            pen.bold = false;
            pen.text(r.r() - 14.0, r.y + 20.0, &cards.len().to_string(), fs * 0.85, mid, 1.0);
            let mut y = r.y + 40.0;
            for card in cards {
                let cr = R::new(r.x + 8.0, y, r.w - 16.0, fs * 3.4);
                if cr.b() > r.b() - 4.0 {
                    break;
                }
                pen.rect(cr, 6.0, Some(paper), Some(fg));
                pen.text_in(cr.pad(10.0, 0.0, 10.0, fs * 0.9), card, fs * 0.95, fg, 0.0);
                pen.rect(R::new(cr.x + 10.0, cr.b() - fs * 1.0, 34.0, 6.0), 3.0, Some(mid), None);
                y += cr.h + 8.0;
            }
        }
        "empty_state" => {
            let ls: Vec<&str> = text.lines().collect();
            let s = (r.h * 0.22).min(48.0);
            icon(pen, "folder", R::new(r.cx() - s / 2.0, r.y + 8.0, s, s), mid);
            if let Some(t) = ls.first() {
                pen.bold = true;
                pen.text(r.cx(), r.y + s + 26.0, t, fs * 1.15, fg, 0.5);
                pen.bold = false;
            }
            if let Some(t) = ls.get(1) {
                let mc = mid_or(pen, fg);
                pen.para(R::new(r.x + 10.0, r.y + s + 42.0, r.w - 20.0, fs * 2.8), t, fs * 0.9, mc, 0.5);
            }
            if let Some(t) = ls.get(2) {
                let w = pen.tw(t, fs, false) + 32.0;
                let btn = R::new(r.cx() - w / 2.0, r.b() - 42.0, w, 38.0);
                let a = pen.acc();
                pen.rect(btn, 8.0, Some(a), None);
                pen.text_in(btn, t, fs, PAPER, 0.5);
            }
        }
        "skeleton" => {
            let s = r.h.min(56.0);
            pen.circle(r.x + s / 2.0, r.y + s / 2.0, s / 2.0, Some(soft), None);
            let x = r.x + s + 14.0;
            let w = r.r() - x;
            for (i, f) in [0.6f32, 0.95, 0.8].iter().enumerate() {
                let y = r.y + 6.0 + i as f32 * 18.0;
                if y + 10.0 > r.b() {
                    break;
                }
                pen.rect(R::new(x, y, w * f, 10.0), 5.0, Some(soft), None);
            }
        }
        "carousel" => {
            let b = pen.bg_or(soft);
            pen.rect(r, 10.0, Some(b), Some(fg));
            let s = (r.h * 0.25).min(48.0);
            icon(pen, "image", R::new(r.cx() - s / 2.0, r.cy() - s / 2.0 - 8.0, s, s), mid);
            for (x, ic) in [(r.x + 22.0, "chevron_left"), (r.r() - 22.0, "chevron_right")] {
                pen.circle(x, r.cy(), 14.0, Some(paper), Some(fg));
                icon(pen, ic, R::new(x - 7.0, r.cy() - 7.0, 14.0, 14.0), fg);
            }
            dots(pen, r.cx(), r.b() - 14.0, 5, c.value as usize, fg, mid);
        }
        "page_dots" => {
            let n = ((r.w / 14.0) as usize).clamp(2, 8);
            dots(pen, r.cx(), r.cy(), n, c.value as usize, fg, mid);
        }
        "qr" => {
            let b = pen.bg();
            pen.rect(r, 0.0, Some(b), None);
            let n = 21;
            let cell = r.w.min(r.h) / n as f32;
            let mut seed: u32 = (c.id as u32).wrapping_mul(7919).wrapping_add(17) | 1;
            for y in 0..n {
                for x in 0..n {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let finder = (x < 8 && y < 8) || (x >= n - 8 && y < 8) || (x < 8 && y >= n - 8);
                    if !finder && seed % 2 == 0 {
                        pen.fill_pts(Pen::round_pts(R::new(r.x + x as f32 * cell, r.y + y as f32 * cell, cell, cell), 0.0), fg, true);
                    }
                }
            }
            for (fx, fy) in [(0, 0), (n - 7, 0), (0, n - 7)] {
                let o = R::new(r.x + fx as f32 * cell, r.y + fy as f32 * cell, cell * 7.0, cell * 7.0);
                pen.rect(o, 0.0, Some(fg), None);
                pen.rect(o.inset(cell), 0.0, Some(b), None);
                pen.rect(o.inset(cell * 2.0), 0.0, Some(fg), None);
            }
        }
        "cookie_banner" => {
            shadow(pen, r, 10.0);
            pen.boxed(r, 10.0);
            let mut ls = text.lines();
            let msg = ls.next().unwrap_or("");
            let btns = items(ls.next().unwrap_or("Accept"));
            let mut x = r.r() - 16.0;
            for (i, it) in btns.iter().enumerate() {
                let w = pen.tw(&it.label, fs, false) + 28.0;
                x -= w;
                let b = R::new(x, r.cy() - 18.0, w, 36.0);
                if i == 0 {
                    let a = pen.acc();
                    pen.rect(b, 8.0, Some(a), None);
                    pen.text_in(b, &it.label, fs, PAPER, 0.5);
                } else {
                    pen.rect(b, 8.0, Some(paper), Some(fg));
                    pen.text_in(b, &it.label, fs, fg, 0.5);
                }
                x -= 10.0;
            }
            pen.text_in(R::new(r.x + 18.0, r.y, x - r.x - 24.0, r.h), msg, fs, fg, 0.0);
        }
        "status_bar" => {
            pen.bold = true;
            let t = if text.trim().is_empty() { "9:41" } else { text.trim() };
            pen.text(r.x + r.w * 0.16, r.cy() + 2.0, t, fs * 1.15, fg, 0.5);
            pen.bold = false;
            let s = fs * 1.1;
            let x = r.r() - r.w * 0.1;
            icon(pen, "battery", R::new(x - s, r.cy() - s * 0.65 + 2.0, s * 1.3, s * 1.3), fg);
            icon(pen, "wifi", R::new(x - s * 2.3, r.cy() - s / 2.0 + 1.0, s, s), fg);
            for i in 0..4 {
                let h = 3.0 + i as f32 * 2.5;
                let bx = x - s * 3.6 + i as f32 * 4.5;
                pen.fill_pts(Pen::round_pts(R::new(bx, r.cy() + 7.0 - h, 3.0, h), 1.0), fg, true);
            }
        }
        "app_bar" => {
            let b = pen.bg();
            pen.rect(r, 0.0, Some(b), None);
            pen.line(r.x, r.b(), r.r(), r.b(), 1.0, mid);
            let toks: Vec<&str> = text.split_whitespace().collect();
            let lead: Vec<&str> = toks.iter().take_while(|t| t.starts_with(':')).map(|t| &t[1..]).collect();
            let rest = &toks[lead.len()..];
            let trail_n = rest.iter().rev().take_while(|t| t.starts_with(':')).count();
            let title = rest[..rest.len() - trail_n].join(" ");
            let trail: Vec<&str> = rest[rest.len() - trail_n..].iter().map(|t| &t[1..]).collect();
            let s = (r.h * 0.42).min(26.0);
            let mut x = r.x + 16.0;
            for ic in &lead {
                icon(pen, ic, R::new(x, r.cy() - s / 2.0, s, s), fg);
                x += s + 16.0;
            }
            let mut xr = r.r() - 16.0;
            for ic in trail.iter().rev() {
                icon(pen, ic, R::new(xr - s, r.cy() - s / 2.0, s, s), fg);
                xr -= s + 16.0;
            }
            pen.bold = true;
            if hx_of(c, 0.0) == 0.5 {
                pen.text_in(r.pad(60.0, 0.0, 60.0, 0.0), &title, fs * 1.2, fg, 0.5);
            } else {
                pen.text_in(R::new(x + 4.0, r.y, (xr - x - 4.0).max(0.0), r.h), &title, fs * 1.2, fg, 0.0);
            }
            pen.bold = false;
        }
        "bottom_nav" => {
            let b = pen.bg();
            pen.rect(r, 0.0, Some(b), None);
            pen.line(r.x, r.y, r.r(), r.y, 1.0, mid);
            let its = items(text);
            let n = its.len().max(1) as f32;
            let cw = r.w / n;
            let s = 22.0f32.min(r.h * 0.34);
            let hl = mix(pen.accent, paper, 0.72);
            let mc = mid_or(pen, fg);
            for (i, it) in its.iter().enumerate() {
                let cx = r.x + cw * (i as f32 + 0.5);
                let iy = r.y + r.h * 0.16;
                let col = if it.active { fg } else { mc };
                if it.active {
                    pen.rect(R::new(cx - 28.0, iy - 4.0, 56.0, s + 8.0), (s + 8.0) / 2.0, Some(hl), None);
                }
                icon(pen, it.icon.as_deref().unwrap_or("star"), R::new(cx - s / 2.0, iy, s, s), col);
                pen.bold = it.active;
                pen.text(cx, iy + s + fs * 0.95, &it.label, fs * 0.92, col, 0.5);
                pen.bold = false;
            }
        }
        "fab" => {
            let col = pen.fill_over.unwrap_or(mix(pen.accent, paper, 0.7));
            shadow(pen, r, 16.0);
            let edge = if pen.sketch { Some(fg) } else { None };
            pen.rect(r, 16.0f32.min(r.h / 3.5), Some(col), edge);
            let s = r.h * 0.42;
            let name = text.trim();
            if catalog::is_icon(name) {
                icon(pen, name, R::new(r.cx() - s / 2.0, r.cy() - s / 2.0, s, s), fg);
            } else {
                pen.text_in(r, text, fs, fg, 0.5);
            }
        }
        "bottom_sheet" => {
            shadow(pen, r, 22.0);
            let b = pen.bg();
            let mut pts = Pen::round_pts(R::new(r.x, r.y, r.w, r.h + 40.0), 22.0);
            for p in pts.iter_mut() {
                p[1] = p[1].min(r.b());
            }
            pen.fill_pts(pts.clone(), b, true);
            pen.stroke_pts(&pts, true, lw, fg);
            pen.rect(R::new(r.cx() - 20.0, r.y + 8.0, 40.0, 5.0), 2.5, Some(mid), None);
            let lh = fs * 2.8;
            for (i, it) in lines(text).iter().enumerate() {
                let row = R::new(r.x + 20.0, r.y + 24.0 + i as f32 * lh, r.w - 40.0, lh);
                if row.b() > r.b() {
                    break;
                }
                pen.bold = i == 0;
                icon_label(pen, row, it, if i == 0 { fs * 1.15 } else { fs }, fg, 0.0);
                pen.bold = false;
            }
        }
        "action_sheet" => {
            let ls = lines(text);
            let lh = fs * 3.4;
            let cancel = R::new(r.x, r.b() - lh, r.w, lh);
            let group = R::new(r.x, r.y, r.w, r.h - lh - 8.0);
            let b = pen.bg();
            pen.rect(group, 14.0, Some(b), Some(fg));
            pen.rect(cancel, 14.0, Some(b), Some(fg));
            let n = ls.len().saturating_sub(1);
            let rh = group.h / n.max(1) as f32;
            let mc = mid_or(pen, fg);
            for (i, it) in ls.iter().take(n).enumerate() {
                let row = R::new(group.x, group.y + i as f32 * rh, group.w, rh);
                if i == 0 {
                    pen.text_in(row, &it.label, fs * 0.85, mc, 0.5);
                } else {
                    pen.text_in(row, &it.label, fs * 1.1, if it.disabled { RED } else { acc }, 0.5);
                }
                if i + 1 < n {
                    pen.line(row.x, row.b(), row.r(), row.b(), 1.0, mid);
                }
            }
            if let Some(last) = ls.last() {
                pen.bold = true;
                pen.text_in(cancel, &last.label, fs * 1.1, acc, 0.5);
                pen.bold = false;
            }
        }
        "notification" => {
            let b = pen.bg_or(mix(soft, paper, 0.4));
            pen.rect(r, 18.0, Some(b), Some(fg));
            let s = (r.h - 28.0).min(38.0);
            pen.rect(R::new(r.x + 14.0, r.cy() - s / 2.0, s, s), 9.0, Some(acc), None);
            icon(pen, "chat", R::new(r.x + 14.0 + s * 0.2, r.cy() - s * 0.3, s * 0.6, s * 0.6), PAPER);
            let mut ls = text.lines();
            let x = r.x + 26.0 + s;
            let hh = (r.h - 16.0) / 2.0;
            pen.bold = true;
            pen.text_in(R::new(x, r.y + 8.0, r.r() - x - 12.0, hh), ls.next().unwrap_or(""), fs * 0.92, fg, 0.0);
            pen.bold = false;
            pen.text_in(R::new(x, r.cy(), r.r() - x - 12.0, hh), ls.next().unwrap_or(""), fs, fg, 0.0);
        }
        "keyboard" => {
            let b = pen.bg_or(soft);
            pen.rect(r, 0.0, Some(b), None);
            let rows = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
            let kw = (r.w - 12.0) / 10.0;
            let kh = (r.h - 50.0) / 4.0;
            for (ri, row) in rows.iter().enumerate() {
                let n = row.len() as f32;
                let x0 = r.x + (r.w - n * kw) / 2.0;
                let y = r.y + 10.0 + ri as f32 * (kh + 10.0);
                for (ci, ch) in row.chars().enumerate() {
                    let k = R::new(x0 + ci as f32 * kw + 2.5, y, kw - 5.0, kh);
                    pen.rect(k, 5.0, Some(paper), Some(mid));
                    pen.text_in(k, &ch.to_string(), fs * 1.1, fg, 0.5);
                }
                if ri == 2 {
                    for (x, ic) in [(r.x + 4.0, "arrow_up"), (r.r() - 4.0 - kw * 1.3, "arrow_left")] {
                        let k = R::new(x, y, kw * 1.3, kh);
                        pen.rect(k, 5.0, Some(mid), None);
                        icon(pen, ic, k.square().inset(kh * 0.22), fg);
                    }
                }
            }
            let y = r.y + 10.0 + 3.0 * (kh + 10.0);
            let k1 = R::new(r.x + 4.0, y, kw * 2.4, kh);
            let sp = R::new(k1.r() + 6.0, y, r.w - kw * 4.8 - 20.0, kh);
            let k3 = R::new(sp.r() + 6.0, y, kw * 2.4, kh);
            pen.rect(k1, 5.0, Some(mid), None);
            pen.text_in(k1, "123", fs, fg, 0.5);
            pen.rect(sp, 5.0, Some(paper), Some(mid));
            pen.text_in(sp, "space", fs, fg, 0.5);
            pen.rect(k3, 5.0, Some(acc), None);
            pen.text_in(k3, "return", fs, PAPER, 0.5);
        }
        "swipe_row" => {
            let mut ls = text.lines();
            let label = ls.next().unwrap_or("");
            let acts = items(ls.next().unwrap_or("Delete"));
            let aw = 80.0;
            let total = aw * acts.len() as f32;
            for (i, a) in acts.iter().enumerate() {
                let ar = R::new(r.r() - total + i as f32 * aw, r.y, aw, r.h);
                let col = if i + 1 == acts.len() { RED } else { mid };
                pen.rect(ar, 0.0, Some(col), None);
                pen.text_in(ar, &a.label, fs * 0.9, PAPER, 0.5);
            }
            let row = R::new(r.x, r.y, r.w - total, r.h);
            let b = pen.bg();
            pen.rect(row, 0.0, Some(b), None);
            pen.line(r.x, r.b(), r.r(), r.b(), 1.0, mid);
            pen.circle(r.x + 30.0, r.cy(), 16.0, Some(soft), Some(fg));
            pen.text_in(row.pad(58.0, 0.0, 8.0, 0.0), label, fs, fg, 0.0);
        }
        "home_widget" => {
            let b = pen.bg_or(soft);
            pen.rect(r, 22.0, Some(b), Some(fg));
            let ls: Vec<&str> = text.lines().collect();
            let p = r.pad(16.0, 14.0, 16.0, 14.0);
            if let Some(t) = ls.first() {
                pen.bold = true;
                pen.text(p.x, p.y + fs * 0.6, t, fs * 0.9, fg, 0.0);
                pen.bold = false;
            }
            if let Some(t) = ls.get(1) {
                pen.text(p.x, p.cy(), t, fs * 2.6, fg, 0.0);
            }
            if let Some(t) = ls.get(2) {
                icon(pen, "sun", R::new(p.x, p.b() - fs * 1.2, fs * 1.2, fs * 1.2), ORANGE);
                pen.text(p.x + fs * 1.6, p.b() - fs * 0.6, t, fs * 0.9, fg, 0.0);
            }
        }
        "dynamic_island" => {
            pen.rect(r, r.h / 2.0, Some([20, 20, 22, 255]), None);
            let s = r.h * 0.5;
            icon(pen, "clock", R::new(r.x + r.h * 0.35, r.cy() - s / 2.0, s, s), ORANGE);
            pen.text_in(r.pad(r.h, 0.0, r.h * 0.4, 0.0), text, fs * 0.92, PAPER, 1.0);
        }
        "chart_bar" | "chart_line" | "sparkline" => {
            let v = numbers(text, &[40.0, 70.0, 55.0, 90.0, 65.0]);
            let maxv = v.iter().cloned().fold(1.0f32, f32::max);
            let spark = kind == "sparkline";
            let a = if spark { r.inset(2.0) } else { r.pad(28.0, 10.0, 10.0, 22.0) };
            if !spark {
                pen.ink_line(a.x, a.y, a.x, a.b());
                pen.ink_line(a.x, a.b(), a.r(), a.b());
                let sl = soft_line(pen);
                for i in 1..4 {
                    let y = a.b() - a.h * i as f32 / 4.0;
                    pen.line(a.x + 2.0, y, a.r(), y, 1.0, sl);
                }
            }
            let n = v.len().max(1);
            let col = pen.acc();
            if kind == "chart_bar" {
                let bw = a.w / n as f32;
                for (i, x) in v.iter().enumerate() {
                    let h = a.h * (x / maxv) * 0.95;
                    let b = R::new(a.x + bw * i as f32 + bw * 0.18, a.b() - h, bw * 0.64, h);
                    let cc = if i % 2 == 0 { col } else { mix(col, paper, 0.45) };
                    pen.rect(b, 2.0, Some(cc), Some(fg));
                }
            } else {
                let pts: Vec<[f32; 2]> = v
                    .iter()
                    .enumerate()
                    .map(|(i, x)| [a.x + a.w * i as f32 / (n - 1).max(1) as f32, a.b() - a.h * (x / maxv) * 0.92])
                    .collect();
                pen.stroke_pts(&pts, false, if spark { 1.8 } else { 2.4 }, col);
                if !spark {
                    for p in &pts {
                        pen.circle(p[0], p[1], 3.5, Some(paper), Some(col));
                    }
                }
            }
        }
        "chart_pie" => {
            let v = numbers(text, &[50.0, 30.0, 20.0]);
            let total: f32 = v.iter().sum::<f32>().max(0.001);
            let rad = (r.w.min(r.h) / 2.0 - 4.0).max(4.0);
            let cx = r.x + rad + 4.0;
            let cy = r.cy();
            let mut a0 = -std::f32::consts::FRAC_PI_2;
            let a = pen.acc();
            let shades = [a, mix(a, paper, 0.45), mid, soft, mix(a, INK, 0.3)];
            for (i, x) in v.iter().enumerate() {
                let a1 = a0 + x / total * std::f32::consts::TAU;
                let mut pts = vec![[cx, cy]];
                pts.extend(Pen::arc_pts(cx, cy, rad, a0, a1));
                let col = shades[i % shades.len()];
                pen.fill_pts(pts.clone(), col, false);
                pen.stroke_pts(&pts, true, 1.2, fg);
                let lx = r.x + rad * 2.0 + 20.0;
                let ly = r.y + 16.0 + i as f32 * 22.0;
                if lx + 40.0 < r.r() && ly < r.b() {
                    pen.rect(R::new(lx, ly - 6.0, 12.0, 12.0), 2.0, Some(col), Some(fg));
                    pen.text(lx + 18.0, ly, &format!("{:.0}%", x / total * 100.0), fs * 0.85, fg, 0.0);
                }
                a0 = a1;
            }
        }
        "chart_donut" | "spinner" => {
            let sq = r.square();
            let th = (sq.w * 0.12).max(3.0);
            let rad = sq.w / 2.0 - th / 2.0;
            let frac = if kind == "spinner" { 0.3 } else { (c.value / 100.0).clamp(0.0, 1.0) };
            let ring = Pen::arc_pts(sq.cx(), sq.cy(), rad, 0.0, std::f32::consts::TAU);
            pen.stroke_pts(&ring, true, th, soft);
            let a0 = -std::f32::consts::FRAC_PI_2;
            let arc = Pen::arc_pts(sq.cx(), sq.cy(), rad, a0, a0 + frac * std::f32::consts::TAU);
            let col = pen.acc();
            pen.stroke_pts(&arc, false, th, col);
            if kind != "spinner" {
                pen.bold = true;
                pen.text(sq.cx(), sq.cy(), &format!("{:.0}%", c.value), (sq.w * 0.22).max(8.0), fg, 0.5);
                pen.bold = false;
            }
        }
        "video" => {
            let b = pen.bg_or(soft);
            pen.rect(r, 0.0, Some(b), Some(fg));
            let s = (r.h * 0.18).min(28.0);
            pen.circle(r.cx(), r.cy() - 8.0, s, Some(paper), Some(fg));
            icon(pen, "play", R::new(r.cx() - s * 0.5, r.cy() - 8.0 - s * 0.55, s * 1.1, s * 1.1), fg);
            let y = r.b() - 16.0;
            let v = (c.value / 100.0).clamp(0.0, 1.0);
            pen.line(r.x + 12.0, y, r.r() - 12.0, y, 3.0, mid);
            pen.line(r.x + 12.0, y, r.x + 12.0 + (r.w - 24.0) * v, y, 3.0, acc);
            pen.circle(r.x + 12.0 + (r.w - 24.0) * v, y, 5.0, Some(acc), None);
        }
        "webcam" => {
            let b = pen.bg_or(mix(soft, INK, 0.5));
            pen.rect(r, 10.0, Some(b), Some(fg));
            let s = r.h * 0.5;
            icon(pen, "user", R::new(r.cx() - s / 2.0, r.cy() - s / 2.0, s, s), soft);
            let tag = R::new(r.x + 8.0, r.b() - 30.0, pen.tw(text, fs * 0.9, false) + 34.0, 22.0);
            pen.rect(tag, 6.0, Some([0, 0, 0, 120]), None);
            icon(pen, "mic", R::new(tag.x + 6.0, tag.y + 4.0, 14.0, 14.0), PAPER);
            pen.text(tag.x + 26.0, tag.cy(), text, fs * 0.9, PAPER, 0.0);
        }
        "audio" => {
            pen.boxed(r, r.h / 2.0);
            let s = r.h - 16.0;
            let col = pen.acc();
            pen.circle(r.x + 8.0 + s / 2.0, r.cy(), s / 2.0, Some(col), None);
            icon(pen, "play", R::new(r.x + 8.0 + s * 0.27, r.cy() - s * 0.25, s * 0.5, s * 0.5), PAPER);
            let x0 = r.x + s + 20.0;
            pen.text_in(R::new(x0, r.y + 4.0, r.w - s - 80.0, r.h / 2.0), text, fs * 0.9, fg, 0.0);
            let y = r.y + r.h * 0.68;
            let v = (c.value / 100.0).clamp(0.0, 1.0);
            let x1 = r.r() - 64.0;
            pen.line(x0, y, x1, y, 3.0, mid);
            pen.line(x0, y, x0 + (x1 - x0) * v, y, 3.0, acc);
            let mc = mid_or(pen, fg);
            pen.text(r.r() - 18.0, r.cy(), "3:12", fs * 0.85, mc, 1.0);
        }
        "map" => {
            let b = pen.bg_or(mix(soft, GREEN, 0.12));
            pen.rect(r, 6.0, Some(b), Some(fg));
            pen.line(r.x + 2.0, r.y + r.h * 0.35, r.r() - 2.0, r.y + r.h * 0.6, 8.0, paper);
            pen.line(r.x + r.w * 0.3, r.y + 2.0, r.x + r.w * 0.55, r.b() - 2.0, 8.0, paper);
            pen.line(r.x + r.w * 0.75, r.y + 2.0, r.x + r.w * 0.68, r.b() - 2.0, 5.0, paper);
            pen.rect(R::new(r.x + r.w * 0.06, r.y + r.h * 0.62, r.w * 0.18, r.h * 0.26), 4.0, Some(mix(soft, acc, 0.25)), None);
            let s = (r.h * 0.2).min(36.0);
            icon(pen, "pin", R::new(r.cx() - s / 2.0, r.cy() - s, s, s), RED);
            for (i, ic) in ["plus", "minus"].iter().enumerate() {
                let b = R::new(r.r() - 34.0, r.y + 12.0 + i as f32 * 28.0, 24.0, 24.0);
                pen.rect(b, 4.0, Some(paper), Some(fg));
                icon(pen, ic, b.inset(4.0), fg);
            }
        }
        "alert" => {
            let (col, ic) = match c.state {
                State::Selected => (GREEN, "check"),
                State::Focused => (ORANGE, "warning"),
                State::Disabled => (RED, "warning"),
                State::Normal => (pen.accent, "info"),
            };
            let col = pen.fill_over.unwrap_or(col);
            pen.rect(r, 8.0, Some(mix(col, paper, 0.85)), Some(col));
            pen.rect(R::new(r.x, r.y, 5.0, r.h), 2.5, Some(col), None);
            let s = (r.h * 0.42).min(22.0);
            icon(pen, ic, R::new(r.x + 16.0, r.cy() - s / 2.0, s, s), col);
            let ink = pen.ink_over.unwrap_or(pen.ink);
            pen.para(r.pad(26.0 + s, (r.h - fs * 1.35).max(0.0) / 2.0, 34.0, 4.0), text, fs, ink, 0.0);
            icon(pen, "close", R::new(r.r() - 26.0, r.cy() - 7.0, 14.0, 14.0), ink);
        }
        "toast" => {
            let b = pen.fill_over.unwrap_or(mix(INK, PAPER, 0.08));
            shadow(pen, r, 8.0);
            pen.rect(r, 8.0, Some(b), None);
            let it = parse_item(text);
            let tc = pen.ink_over.unwrap_or(PAPER);
            pen.text_in(r.pad(16.0, 0.0, 90.0, 0.0), &it.label, fs, tc, 0.0);
            if !it.extra.is_empty() {
                pen.bold = true;
                pen.text(r.r() - 16.0, r.cy(), &it.extra, fs, [140, 180, 255, 255], 1.0);
                pen.bold = false;
            }
        }
        "tooltip" => {
            let b = pen.fill_over.unwrap_or(mix(INK, PAPER, 0.08));
            let body = R::new(r.x, r.y, r.w, r.h - 8.0);
            pen.rect(body, 6.0, Some(b), None);
            pen.fill_pts(vec![[r.cx() - 8.0, body.b()], [r.cx() + 8.0, body.b()], [r.cx(), r.b()]], b, true);
            let tc = pen.ink_over.unwrap_or(PAPER);
            pen.text_in(body.pad(10.0, 0.0, 10.0, 0.0), text, fs, tc, 0.5);
        }
        "dialog" => {
            shadow(pen, r, 14.0);
            pen.boxed(r, 14.0);
            let ls: Vec<&str> = text.lines().collect();
            let title = ls.first().copied().unwrap_or("");
            let btns = if ls.len() > 2 { ls[ls.len() - 1] } else { "OK" };
            let body = if ls.len() > 2 { ls[1..ls.len() - 1].join("\n") } else { ls.get(1).copied().unwrap_or("").to_string() };
            pen.bold = true;
            pen.text(r.x + 22.0, r.y + 30.0, title, fs * 1.2, fg, 0.0);
            pen.bold = false;
            let mc = mid_or(pen, fg);
            pen.para(r.pad(22.0, 50.0, 22.0, 60.0), &body, fs, mc, 0.0);
            let mut x = r.r() - 18.0;
            for it in items(btns).iter().rev() {
                let w = pen.tw(&it.label, fs, false) + 30.0;
                x -= w;
                let b = R::new(x, r.b() - 54.0, w, 38.0);
                if it.active {
                    let a = pen.acc();
                    pen.rect(b, 8.0, Some(a), None);
                    pen.text_in(b, &it.label, fs, PAPER, 0.5);
                } else {
                    pen.rect(b, 8.0, Some(paper), Some(fg));
                    pen.text_in(b, &it.label, fs, fg, 0.5);
                }
                x -= 10.0;
            }
        }
        "progress" => {
            pen.rect(r, r.h / 2.0, Some(soft), Some(fg));
            let v = (c.value / 100.0).clamp(0.0, 1.0);
            if v > 0.0 {
                let col = pen.acc();
                pen.rect(R::new(r.x, r.y, (r.w * v).max(r.h), r.h), r.h / 2.0, Some(col), None);
            }
        }
        "ai_prompt" => {
            let bg = pen.bg();
            pen.rect(r, 20.0f32.min(r.h / 2.0), Some(bg), Some(fg));
            let mc = mid_or(pen, fg);
            pen.text_in(R::new(r.x + 20.0, r.y + 10.0, r.w - 40.0, fs * 1.6), text, fs, mc, 0.0);
            let y = r.b() - 26.0;
            for (i, ic) in ["plus", "sparkles", "mic"].iter().enumerate() {
                icon(pen, ic, R::new(r.x + 18.0 + i as f32 * 30.0, y - 9.0, 18.0, 18.0), mc);
            }
            let col = pen.acc();
            pen.circle(r.r() - 30.0, y, 15.0, Some(col), None);
            icon(pen, "arrow_up", R::new(r.r() - 38.0, y - 8.0, 16.0, 16.0), PAPER);
        }
        "ai_answer" => {
            let b = pen.bg_or(mix(soft, paper, 0.5));
            pen.rect(r, 14.0, Some(b), None);
            icon(pen, "sparkles", R::new(r.x + 14.0, r.y + 14.0, 20.0, 20.0), acc);
            pen.para(r.pad(44.0, 14.0, 16.0, 34.0), text, fs, fg, 0.0);
            let mc = mid_or(pen, fg);
            for (i, ic) in ["file", "thumbs_up", "refresh"].iter().enumerate() {
                icon(pen, ic, R::new(r.x + 44.0 + i as f32 * 28.0, r.b() - 26.0, 16.0, 16.0), mc);
            }
        }
        "component" => {
            let target = c.component.as_deref().and_then(|k| env.project.screen_idx(k));
            match target {
                Some(i) if env.depth < 4 => {
                    let s = &env.project.screens[i];
                    let sx = c.w / s.w.max(1.0);
                    let sy = c.h / s.h.max(1.0);
                    let start = pen.out.len();
                    let sub = Env { project: env.project, depth: env.depth + 1 };
                    controls_of(pen, &sub, i);
                    transform(&mut pen.out[start..], c.x, c.y, sx, sy);
                    // the instance's own link wins over the component's links
                    if c.link.is_some() {
                        let tail = pen.out.split_off(start);
                        pen.out.extend(tail.into_iter().filter(|p| !matches!(p, Prim::Link { .. })));
                    }
                }
                _ => {
                    pen.rect(r, 6.0, Some(soft), None);
                    dashed_round(pen, r, 6.0, 1.2, mid);
                    pen.text_in(r, "Pick a component", fs * 0.9, mid, 0.5);
                }
            }
        }
        _ => {
            pen.boxed(r, 4.0);
            pen.text_in(r, &format!("? {kind}"), fs, fg, 0.5);
        }
    }
}

fn dots(pen: &mut Pen, cx: f32, cy: f32, n: usize, active: usize, fg: Rgba, mid: Rgba) {
    let gap = 14.0;
    let x0 = cx - gap * (n as f32 - 1.0) / 2.0;
    for i in 0..n {
        pen.circle(x0 + i as f32 * gap, cy, 4.0, Some(if i == active { fg } else { mid }), None);
    }
}

fn table(pen: &mut Pen, r: R, text: &str, fs: f32, fg: Rgba) {
    let rows: Vec<&str> = text.lines().collect();
    pen.boxed(r, 4.0);
    if rows.is_empty() {
        return;
    }
    let cols = rows.iter().map(|l| l.split(',').count()).max().unwrap_or(1).max(1);
    let rh = (r.h / rows.len() as f32).min(fs * 2.6);
    let cw = r.w / cols as f32;
    let soft = pen.soft;
    let mid = pen.mid;
    let hl = mix(pen.accent, pen.paper, 0.75);
    let lw = pen.lw();
    for (ri, row) in rows.iter().enumerate() {
        let y = r.y + ri as f32 * rh;
        if y + rh > r.b() + 1.0 {
            break;
        }
        let mut row = *row;
        if let Some(rest) = row.strip_prefix('*') {
            row = rest;
            pen.rect(R::new(r.x + 1.0, y, r.w - 2.0, rh), 0.0, Some(hl), None);
        }
        if ri == 0 {
            pen.rect(R::new(r.x + 1.0, y + 1.0, r.w - 2.0, rh - 1.0), 3.0, Some(soft), None);
        }
        for (ci, cell) in row.split(',').enumerate() {
            pen.bold = ri == 0;
            pen.text_in(R::new(r.x + ci as f32 * cw + 10.0, y, cw - 14.0, rh), cell.trim(), fs * 0.95, fg, 0.0);
            pen.bold = false;
        }
        let yb = y + rh;
        if yb < r.b() - 2.0 {
            pen.line(r.x, yb, r.r(), yb, if ri == 0 { lw } else { 1.0 }, if ri == 0 { fg } else { mid });
        }
    }
}
