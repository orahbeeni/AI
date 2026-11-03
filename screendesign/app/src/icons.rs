//! Simple line icons drawn in a unit square.

use crate::pen::{Pen, Rgba, R};
use std::f32::consts::PI;

pub fn icon(pen: &mut Pen, name: &str, r: R, color: Rgba) {
    let r = r.square();
    let s = r.w;
    let (ox, oy) = (r.x, r.y);
    let p = |x: f32, y: f32| [ox + x * s, oy + y * s];
    let lw = (s / 13.0).clamp(1.1, 3.5);
    let st = |pen: &mut Pen, pts: &[[f32; 2]], closed: bool| pen.stroke_pts(pts, closed, lw, color);
    let circ = |pen: &mut Pen, x: f32, y: f32, rad: f32, fill: bool| {
        let (cx, cy) = (ox + x * s, oy + y * s);
        let rr = R::new(cx - rad * s, cy - rad * s, rad * 2.0 * s, rad * 2.0 * s);
        let pts = Pen::ellipse_pts(rr, 28);
        if fill {
            pen.fill_pts(pts, color, true);
        } else {
            pen.stroke_pts(&pts, true, lw, color);
        }
    };
    let arc = |x: f32, y: f32, rad: f32, a0: f32, a1: f32| -> Vec<[f32; 2]> {
        Pen::arc_pts(ox + x * s, oy + y * s, rad * s, a0.to_radians(), a1.to_radians())
    };
    let fill = |pen: &mut Pen, pts: Vec<[f32; 2]>| pen.fill_pts(pts, color, true);
    match name {
        "home" => {
            st(pen, &[p(0.12, 0.5), p(0.5, 0.14), p(0.88, 0.5)], false);
            st(pen, &[p(0.24, 0.42), p(0.24, 0.86), p(0.76, 0.86), p(0.76, 0.42)], false);
            st(pen, &[p(0.42, 0.86), p(0.42, 0.62), p(0.58, 0.62), p(0.58, 0.86)], false);
        }
        "search" => {
            circ(pen, 0.42, 0.42, 0.26, false);
            st(pen, &[p(0.62, 0.62), p(0.86, 0.86)], false);
        }
        "user" => {
            circ(pen, 0.5, 0.33, 0.17, false);
            st(pen, &arc(0.5, 0.92, 0.34, 180.0, 360.0), false);
        }
        "users" => {
            circ(pen, 0.38, 0.36, 0.14, false);
            st(pen, &arc(0.38, 0.9, 0.28, 180.0, 360.0), false);
            circ(pen, 0.68, 0.3, 0.12, false);
            st(pen, &arc(0.7, 0.78, 0.22, 230.0, 360.0), false);
        }
        "settings" => {
            circ(pen, 0.5, 0.5, 0.13, false);
            let pts: Vec<[f32; 2]> = (0..32)
                .map(|i| {
                    let a = i as f32 / 32.0 * 2.0 * PI;
                    let rad = if (i / 2) % 2 == 0 { 0.4 } else { 0.31 };
                    p(0.5 + rad * a.cos(), 0.5 + rad * a.sin())
                })
                .collect();
            st(pen, &pts, true);
        }
        "heart" => {
            let pts: Vec<[f32; 2]> = (0..40)
                .map(|i| {
                    let t = i as f32 / 40.0 * 2.0 * PI;
                    let x = 16.0 * t.sin().powi(3);
                    let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
                    p(0.5 + x / 40.0, 0.47 - y / 40.0)
                })
                .collect();
            st(pen, &pts, true);
        }
        "star" => {
            let pts = star_pts(p(0.5, 0.53), 0.44 * s, 0.19 * s, 5);
            st(pen, &pts, true);
        }
        "star_fill" => {
            let c = p(0.5, 0.53);
            let mut pts = vec![c];
            pts.extend(star_pts(c, 0.44 * s, 0.19 * s, 5));
            pts.push(pts[1]);
            pen.fill_pts(pts, color, false);
        }
        "bell" => {
            let mut pts = vec![p(0.18, 0.72), p(0.28, 0.6)];
            pts.extend(arc(0.5, 0.44, 0.22, 180.0, 360.0));
            pts.extend([p(0.72, 0.6), p(0.82, 0.72)]);
            st(pen, &pts, true);
            circ(pen, 0.5, 0.82, 0.06, false);
        }
        "menu" => {
            for y in [0.28, 0.5, 0.72] {
                st(pen, &[p(0.16, y), p(0.84, y)], false);
            }
        }
        "close" => {
            st(pen, &[p(0.24, 0.24), p(0.76, 0.76)], false);
            st(pen, &[p(0.76, 0.24), p(0.24, 0.76)], false);
        }
        "plus" => {
            st(pen, &[p(0.5, 0.18), p(0.5, 0.82)], false);
            st(pen, &[p(0.18, 0.5), p(0.82, 0.5)], false);
        }
        "minus" => st(pen, &[p(0.18, 0.5), p(0.82, 0.5)], false),
        "check" => st(pen, &[p(0.18, 0.52), p(0.4, 0.74), p(0.84, 0.28)], false),
        "chevron_left" => st(pen, &[p(0.62, 0.2), p(0.34, 0.5), p(0.62, 0.8)], false),
        "chevron_right" => st(pen, &[p(0.38, 0.2), p(0.66, 0.5), p(0.38, 0.8)], false),
        "chevron_down" => st(pen, &[p(0.2, 0.38), p(0.5, 0.66), p(0.8, 0.38)], false),
        "chevron_up" => st(pen, &[p(0.2, 0.62), p(0.5, 0.34), p(0.8, 0.62)], false),
        "arrow_left" => {
            st(pen, &[p(0.84, 0.5), p(0.16, 0.5)], false);
            st(pen, &[p(0.42, 0.24), p(0.16, 0.5), p(0.42, 0.76)], false);
        }
        "arrow_right" => {
            st(pen, &[p(0.16, 0.5), p(0.84, 0.5)], false);
            st(pen, &[p(0.58, 0.24), p(0.84, 0.5), p(0.58, 0.76)], false);
        }
        "arrow_up" => {
            st(pen, &[p(0.5, 0.84), p(0.5, 0.16)], false);
            st(pen, &[p(0.24, 0.42), p(0.5, 0.16), p(0.76, 0.42)], false);
        }
        "arrow_down" => {
            st(pen, &[p(0.5, 0.16), p(0.5, 0.84)], false);
            st(pen, &[p(0.24, 0.58), p(0.5, 0.84), p(0.76, 0.58)], false);
        }
        "cart" => {
            st(pen, &[p(0.08, 0.2), p(0.22, 0.2), p(0.33, 0.64), p(0.8, 0.64), p(0.9, 0.32), p(0.26, 0.32)], false);
            circ(pen, 0.38, 0.8, 0.06, false);
            circ(pen, 0.74, 0.8, 0.06, false);
        }
        "mail" => {
            st(pen, &[p(0.1, 0.24), p(0.9, 0.24), p(0.9, 0.76), p(0.1, 0.76)], true);
            st(pen, &[p(0.1, 0.26), p(0.5, 0.54), p(0.9, 0.26)], false);
        }
        "camera" => {
            st(pen, &[p(0.08, 0.3), p(0.32, 0.3), p(0.4, 0.18), p(0.6, 0.18), p(0.68, 0.3), p(0.92, 0.3), p(0.92, 0.82), p(0.08, 0.82)], true);
            circ(pen, 0.5, 0.56, 0.15, false);
        }
        "share" => {
            circ(pen, 0.72, 0.22, 0.1, false);
            circ(pen, 0.28, 0.5, 0.1, false);
            circ(pen, 0.72, 0.78, 0.1, false);
            st(pen, &[p(0.37, 0.45), p(0.63, 0.28)], false);
            st(pen, &[p(0.37, 0.55), p(0.63, 0.72)], false);
        }
        "trash" => {
            st(pen, &[p(0.16, 0.26), p(0.84, 0.26)], false);
            st(pen, &[p(0.4, 0.26), p(0.4, 0.15), p(0.6, 0.15), p(0.6, 0.26)], false);
            st(pen, &[p(0.25, 0.26), p(0.3, 0.88), p(0.7, 0.88), p(0.75, 0.26)], false);
        }
        "edit" => {
            st(pen, &[p(0.2, 0.8), p(0.24, 0.62), p(0.66, 0.2), p(0.82, 0.36), p(0.4, 0.78)], true);
            st(pen, &[p(0.58, 0.28), p(0.74, 0.44)], false);
        }
        "lock" => {
            st(pen, &[p(0.22, 0.46), p(0.78, 0.46), p(0.78, 0.88), p(0.22, 0.88)], true);
            let mut sh = vec![p(0.32, 0.46)];
            sh.extend(arc(0.5, 0.32, 0.18, 180.0, 360.0));
            sh.push(p(0.68, 0.46));
            st(pen, &sh, false);
        }
        "calendar" => {
            st(pen, &[p(0.12, 0.22), p(0.88, 0.22), p(0.88, 0.86), p(0.12, 0.86)], true);
            st(pen, &[p(0.12, 0.4), p(0.88, 0.4)], false);
            st(pen, &[p(0.32, 0.12), p(0.32, 0.3)], false);
            st(pen, &[p(0.68, 0.12), p(0.68, 0.3)], false);
        }
        "chat" => {
            st(pen, &[p(0.12, 0.18), p(0.88, 0.18), p(0.88, 0.68), p(0.46, 0.68), p(0.26, 0.86), p(0.3, 0.68), p(0.12, 0.68)], true);
        }
        "play" => fill(pen, vec![p(0.3, 0.18), p(0.82, 0.5), p(0.3, 0.82)]),
        "pause" => {
            fill(pen, vec![p(0.26, 0.2), p(0.42, 0.2), p(0.42, 0.8), p(0.26, 0.8)]);
            fill(pen, vec![p(0.58, 0.2), p(0.74, 0.2), p(0.74, 0.8), p(0.58, 0.8)]);
        }
        "image" => {
            st(pen, &[p(0.1, 0.18), p(0.9, 0.18), p(0.9, 0.82), p(0.1, 0.82)], true);
            st(pen, &[p(0.1, 0.72), p(0.36, 0.46), p(0.56, 0.66), p(0.68, 0.54), p(0.9, 0.76)], false);
            circ(pen, 0.68, 0.36, 0.07, false);
        }
        "pin" => {
            let mut pts = arc(0.5, 0.4, 0.27, 145.0, 395.0);
            pts.push(p(0.5, 0.9));
            st(pen, &pts, true);
            circ(pen, 0.5, 0.4, 0.09, false);
        }
        "phone" => {
            let rr = R::new(ox + 0.28 * s, oy + 0.1 * s, 0.44 * s, 0.8 * s);
            let pts = Pen::round_pts(rr, 0.08 * s);
            pen.stroke_pts(&pts, true, lw, color);
            st(pen, &[p(0.44, 0.8), p(0.56, 0.8)], false);
        }
        "download" | "upload" => {
            if name == "download" {
                st(pen, &[p(0.5, 0.12), p(0.5, 0.64)], false);
                st(pen, &[p(0.3, 0.44), p(0.5, 0.64), p(0.7, 0.44)], false);
            } else {
                st(pen, &[p(0.5, 0.64), p(0.5, 0.12)], false);
                st(pen, &[p(0.3, 0.32), p(0.5, 0.12), p(0.7, 0.32)], false);
            }
            st(pen, &[p(0.14, 0.64), p(0.14, 0.86), p(0.86, 0.86), p(0.86, 0.64)], false);
        }
        "filter" => st(pen, &[p(0.12, 0.2), p(0.88, 0.2), p(0.58, 0.52), p(0.58, 0.84), p(0.42, 0.74), p(0.42, 0.52)], true),
        "more" | "more_v" => {
            for t in [0.24, 0.5, 0.76] {
                if name == "more" {
                    circ(pen, t, 0.5, 0.07, true);
                } else {
                    circ(pen, 0.5, t, 0.07, true);
                }
            }
        }
        "info" | "help" => {
            circ(pen, 0.5, 0.5, 0.4, false);
            if name == "info" {
                circ(pen, 0.5, 0.3, 0.045, true);
                st(pen, &[p(0.5, 0.44), p(0.5, 0.72)], false);
            } else {
                let mut q = arc(0.5, 0.38, 0.13, 200.0, 400.0);
                q.push(p(0.5, 0.6));
                st(pen, &q, false);
                circ(pen, 0.5, 0.73, 0.04, true);
            }
        }
        "warning" => {
            st(pen, &[p(0.5, 0.12), p(0.9, 0.84), p(0.1, 0.84)], true);
            st(pen, &[p(0.5, 0.38), p(0.5, 0.6)], false);
            circ(pen, 0.5, 0.72, 0.04, true);
        }
        "globe" => {
            circ(pen, 0.5, 0.5, 0.38, false);
            let e = Pen::ellipse_pts(R::new(ox + 0.33 * s, oy + 0.12 * s, 0.34 * s, 0.76 * s), 24);
            st(pen, &e, true);
            st(pen, &[p(0.12, 0.5), p(0.88, 0.5)], false);
            st(pen, &[p(0.2, 0.3), p(0.8, 0.3)], false);
            st(pen, &[p(0.2, 0.7), p(0.8, 0.7)], false);
        }
        "bookmark" => st(pen, &[p(0.28, 0.12), p(0.72, 0.12), p(0.72, 0.88), p(0.5, 0.68), p(0.28, 0.88)], true),
        "clock" => {
            circ(pen, 0.5, 0.5, 0.38, false);
            st(pen, &[p(0.5, 0.26), p(0.5, 0.5), p(0.68, 0.6)], false);
        }
        "wifi" => {
            for rad in [0.18, 0.36, 0.54] {
                st(pen, &arc(0.5, 0.82, rad, 225.0, 315.0), false);
            }
            circ(pen, 0.5, 0.8, 0.05, true);
        }
        "battery" => {
            st(pen, &[p(0.08, 0.32), p(0.8, 0.32), p(0.8, 0.68), p(0.08, 0.68)], true);
            fill(pen, vec![p(0.84, 0.42), p(0.92, 0.42), p(0.92, 0.58), p(0.84, 0.58)]);
            fill(pen, vec![p(0.14, 0.38), p(0.6, 0.38), p(0.6, 0.62), p(0.14, 0.62)]);
        }
        "mic" => {
            let rr = R::new(ox + 0.37 * s, oy + 0.1 * s, 0.26 * s, 0.46 * s);
            let pts = Pen::round_pts(rr, 0.13 * s);
            pen.stroke_pts(&pts, true, lw, color);
            st(pen, &arc(0.5, 0.46, 0.24, 0.0, 180.0), false);
            st(pen, &[p(0.5, 0.7), p(0.5, 0.86)], false);
            st(pen, &[p(0.34, 0.86), p(0.66, 0.86)], false);
        }
        "grid" => {
            for (x, y) in [(0.14, 0.14), (0.54, 0.14), (0.14, 0.54), (0.54, 0.54)] {
                st(pen, &[p(x, y), p(x + 0.32, y), p(x + 0.32, y + 0.32), p(x, y + 0.32)], true);
            }
        }
        "list" => {
            for y in [0.26, 0.5, 0.74] {
                circ(pen, 0.18, y, 0.05, true);
                st(pen, &[p(0.34, y), p(0.86, y)], false);
            }
        }
        "sun" => {
            circ(pen, 0.5, 0.5, 0.17, false);
            for i in 0..8 {
                let a = i as f32 / 8.0 * 2.0 * PI;
                st(pen, &[p(0.5 + 0.28 * a.cos(), 0.5 + 0.28 * a.sin()), p(0.5 + 0.4 * a.cos(), 0.5 + 0.4 * a.sin())], false);
            }
        }
        "moon" => {
            let mut m = arc(0.5, 0.5, 0.36, 50.0, 310.0);
            m.extend(arc(0.7, 0.42, 0.3, 230.0, 130.0));
            st(pen, &m, true);
        }
        "cloud" => {
            let mut pts = arc(0.26, 0.6, 0.13, 90.0, 270.0);
            pts.extend(arc(0.5, 0.46, 0.2, 200.0, 340.0));
            pts.extend(arc(0.74, 0.6, 0.13, 270.0, 450.0));
            st(pen, &pts, true);
        }
        "folder" => st(pen, &[p(0.1, 0.24), p(0.4, 0.24), p(0.48, 0.33), p(0.9, 0.33), p(0.9, 0.8), p(0.1, 0.8)], true),
        "file" => {
            st(pen, &[p(0.22, 0.1), p(0.6, 0.1), p(0.8, 0.3), p(0.8, 0.9), p(0.22, 0.9)], true);
            st(pen, &[p(0.6, 0.1), p(0.6, 0.3), p(0.8, 0.3)], false);
        }
        "link" => {
            for x in [0.08, 0.46] {
                let rr = R::new(ox + x * s, oy + 0.36 * s, 0.46 * s, 0.28 * s);
                let pts = Pen::round_pts(rr, 0.14 * s);
                pen.stroke_pts(&pts, true, lw, color);
            }
        }
        "eye" => {
            let mut pts: Vec<[f32; 2]> = (0..=16).map(|i| {
                let t = i as f32 / 16.0;
                p(0.08 + 0.84 * t, 0.5 - 0.27 * (PI * t).sin())
            }).collect();
            pts.extend((1..16).rev().map(|i| {
                let t = i as f32 / 16.0;
                p(0.08 + 0.84 * t, 0.5 + 0.27 * (PI * t).sin())
            }));
            st(pen, &pts, true);
            circ(pen, 0.5, 0.5, 0.11, false);
        }
        "refresh" => {
            st(pen, &arc(0.5, 0.5, 0.32, -50.0, 250.0), false);
            st(pen, &[p(0.6, 0.18), p(0.71, 0.26), p(0.62, 0.38)], false);
        }
        "send" => {
            st(pen, &[p(0.1, 0.46), p(0.9, 0.14), p(0.62, 0.88), p(0.48, 0.56)], true);
            st(pen, &[p(0.48, 0.56), p(0.9, 0.14)], false);
        }
        "sparkles" => {
            let c = p(0.42, 0.56);
            let mut a = vec![c];
            a.extend(star_pts(c, 0.36 * s, 0.09 * s, 4));
            a.push(a[1]);
            pen.fill_pts(a, color, false);
            let c2 = p(0.78, 0.22);
            let mut b = vec![c2];
            b.extend(star_pts(c2, 0.16 * s, 0.045 * s, 4));
            b.push(b[1]);
            pen.fill_pts(b, color, false);
        }
        "logout" => {
            st(pen, &[p(0.5, 0.16), p(0.16, 0.16), p(0.16, 0.84), p(0.5, 0.84)], false);
            st(pen, &[p(0.38, 0.5), p(0.88, 0.5)], false);
            st(pen, &[p(0.68, 0.3), p(0.88, 0.5), p(0.68, 0.7)], false);
        }
        "bolt" => {
            let pts = vec![p(0.56, 0.08), p(0.22, 0.56), p(0.48, 0.56), p(0.42, 0.92), p(0.78, 0.42), p(0.52, 0.42)];
            st(pen, &pts, true);
        }
        "music" => {
            st(pen, &[p(0.36, 0.76), p(0.36, 0.28), p(0.78, 0.18), p(0.78, 0.66)], false);
            circ(pen, 0.27, 0.76, 0.1, true);
            circ(pen, 0.69, 0.66, 0.1, true);
        }
        "video" => {
            st(pen, &[p(0.08, 0.28), p(0.64, 0.28), p(0.64, 0.72), p(0.08, 0.72)], true);
            st(pen, &[p(0.64, 0.44), p(0.92, 0.3), p(0.92, 0.7), p(0.64, 0.56)], true);
        }
        "tag" => {
            st(pen, &[p(0.1, 0.12), p(0.5, 0.12), p(0.9, 0.52), p(0.52, 0.9), p(0.1, 0.5)], true);
            circ(pen, 0.3, 0.32, 0.06, false);
        }
        "gift" => {
            st(pen, &[p(0.16, 0.42), p(0.84, 0.42), p(0.84, 0.88), p(0.16, 0.88)], true);
            st(pen, &[p(0.1, 0.28), p(0.9, 0.28), p(0.9, 0.42), p(0.1, 0.42)], true);
            st(pen, &[p(0.5, 0.28), p(0.5, 0.88)], false);
            st(pen, &[p(0.5, 0.28), p(0.32, 0.12), p(0.28, 0.24), p(0.5, 0.28), p(0.72, 0.12), p(0.76, 0.24), p(0.5, 0.28)], false);
        }
        "card" => {
            st(pen, &[p(0.08, 0.22), p(0.92, 0.22), p(0.92, 0.78), p(0.08, 0.78)], true);
            fill(pen, vec![p(0.08, 0.34), p(0.92, 0.34), p(0.92, 0.44), p(0.08, 0.44)]);
            st(pen, &[p(0.18, 0.64), p(0.42, 0.64)], false);
        }
        "thumbs_up" => {
            st(pen, &[p(0.1, 0.46), p(0.26, 0.46), p(0.26, 0.86), p(0.1, 0.86)], true);
            st(pen, &[p(0.32, 0.46), p(0.5, 0.14), p(0.6, 0.18), p(0.56, 0.4), p(0.84, 0.4), p(0.8, 0.86), p(0.32, 0.86)], true);
        }
        _ => {
            circ(pen, 0.5, 0.5, 0.38, false);
            st(pen, &[p(0.3, 0.3), p(0.7, 0.7)], false);
        }
    }
}

pub fn star_pts(c: [f32; 2], outer: f32, inner: f32, n: usize) -> Vec<[f32; 2]> {
    (0..n * 2)
        .map(|i| {
            let a = -PI / 2.0 + i as f32 * PI / n as f32;
            let rad = if i % 2 == 0 { outer } else { inner };
            [c[0] + rad * a.cos(), c[1] + rad * a.sin()]
        })
        .collect()
}
