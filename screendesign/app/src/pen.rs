//! A small drawing vocabulary shared by the editor (egui) and the exporters (SVG/PNG).
//! In sketch mode every line wobbles a little, like a pen on paper.

pub type Rgba = [u8; 4];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> R {
        R { x, y, w: w.max(0.0), h: h.max(0.0) }
    }
    pub fn r(&self) -> f32 {
        self.x + self.w
    }
    pub fn b(&self) -> f32 {
        self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    pub fn inset(&self, d: f32) -> R {
        R::new(self.x + d, self.y + d, self.w - 2.0 * d, self.h - 2.0 * d)
    }
    pub fn pad(&self, l: f32, t: f32, r: f32, b: f32) -> R {
        R::new(self.x + l, self.y + t, self.w - l - r, self.h - t - b)
    }
    pub fn square(&self) -> R {
        let s = self.w.min(self.h);
        R::new(self.cx() - s / 2.0, self.cy() - s / 2.0, s, s)
    }
}

#[derive(Clone, Debug)]
pub enum Prim {
    Path { pts: Vec<[f32; 2]>, closed: bool, fill: Option<Rgba>, stroke: Option<(f32, Rgba)>, convex: bool },
    Text { x: f32, y: f32, s: String, size: f32, color: Rgba, bold: bool, italic: bool, underline: bool, mono: bool, hx: f32 },
    /// A clickable area for exported prototypes.
    Link { r: R, target: String },
}

pub const INK: Rgba = [43, 43, 43, 255];
pub const PAPER: Rgba = [255, 255, 255, 255];
pub const SOFT: Rgba = [234, 234, 234, 255];
pub const MID: Rgba = [176, 176, 176, 255];
pub const ACCENT: Rgba = [64, 118, 204, 255];
pub const STICKY: Rgba = [255, 241, 140, 255];
pub const RED: Rgba = [212, 74, 58, 255];
pub const GREEN: Rgba = [64, 160, 96, 255];
pub const ORANGE: Rgba = [230, 150, 40, 255];

pub fn with_alpha(c: Rgba, a: f32) -> Rgba {
    [c[0], c[1], c[2], (c[3] as f32 * a.clamp(0.0, 1.0)) as u8]
}

pub fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    [l(a[0], b[0]), l(a[1], b[1]), l(a[2], b[2]), l(a[3], b[3])]
}

pub type Measure<'a> = &'a dyn Fn(&str, f32, bool) -> f32;

pub fn approx_measure(s: &str, size: f32, bold: bool) -> f32 {
    let mut w = 0.0;
    for ch in s.chars() {
        w += match ch {
            'i' | 'l' | 'j' | '\'' | '.' | ',' | ':' | ';' | '|' | '!' => 0.28,
            'f' | 't' | 'r' | ' ' | 'I' | '(' | ')' => 0.38,
            'm' | 'w' | 'M' | 'W' => 0.82,
            'A'..='Z' => 0.64,
            _ => 0.53,
        };
    }
    w * size * if bold { 1.06 } else { 1.0 }
}

pub struct Pen<'a> {
    pub out: Vec<Prim>,
    pub sketch: bool,
    seed: u32,
    pub ink: Rgba,
    pub paper: Rgba,
    pub soft: Rgba,
    pub mid: Rgba,
    pub accent: Rgba,
    pub alpha: f32,
    /// Overrides from the control being drawn.
    pub fill_over: Option<Rgba>,
    pub ink_over: Option<Rgba>,
    pub font: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub measure: Measure<'a>,
}

impl<'a> Pen<'a> {
    pub fn new(sketch: bool, dark: bool, measure: Measure<'a>) -> Pen<'a> {
        let mut p = Pen {
            out: Vec::new(),
            sketch,
            seed: 1,
            ink: INK,
            paper: PAPER,
            soft: SOFT,
            mid: MID,
            accent: ACCENT,
            alpha: 1.0,
            fill_over: None,
            ink_over: None,
            font: 15.0,
            bold: false,
            italic: false,
            underline: false,
            measure,
        };
        p.set_dark(dark);
        p
    }

    pub fn set_dark(&mut self, dark: bool) {
        if dark {
            self.ink = [232, 232, 236, 255];
            self.paper = [30, 31, 36, 255];
            self.soft = [52, 54, 62, 255];
            self.mid = [96, 98, 110, 255];
            self.accent = [110, 158, 240, 255];
        } else {
            self.ink = INK;
            self.paper = PAPER;
            self.soft = SOFT;
            self.mid = MID;
            self.accent = ACCENT;
        }
    }

    pub fn seed(&mut self, s: u64) {
        self.seed = (s as u32).wrapping_mul(2654435761).wrapping_add(12345) | 1;
    }

    /// -1..1
    fn rnd(&mut self) -> f32 {
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x % 20001) as f32 / 10000.0 - 1.0
    }

    // ---- colours with the control's overrides ----
    pub fn fg(&self) -> Rgba {
        self.ink_over.unwrap_or(self.ink)
    }
    pub fn bg(&self) -> Rgba {
        self.fill_over.unwrap_or(self.paper)
    }
    pub fn bg_or(&self, c: Rgba) -> Rgba {
        self.fill_over.unwrap_or(c)
    }
    pub fn acc(&self) -> Rgba {
        self.fill_over.unwrap_or(self.accent)
    }
    pub fn lw(&self) -> f32 {
        if self.sketch {
            1.7
        } else {
            1.3
        }
    }

    fn a(&self, c: Rgba) -> Rgba {
        with_alpha(c, self.alpha)
    }

    // ---- primitives ----
    fn rough_seg(&mut self, a: [f32; 2], b: [f32; 2], out: &mut Vec<[f32; 2]>, first: bool) {
        if !self.sketch {
            if first {
                out.push(a);
            }
            out.push(b);
            return;
        }
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let len = (dx * dx + dy * dy).sqrt();
        if len < 0.5 {
            out.push(b);
            return;
        }
        let n = ((len / 18.0) as usize).clamp(1, 30);
        let (nx, ny) = (-dy / len, dx / len);
        let bow = (len * 0.012).min(1.6) * self.rnd();
        let j = 0.5;
        for i in 0..=n {
            if i == 0 && !first {
                continue;
            }
            let t = i as f32 / n as f32;
            let off = bow * (std::f32::consts::PI * t).sin() + if i > 0 && i < n { self.rnd() * 0.25 } else { 0.0 };
            let (ex, ey) = if i == 0 || i == n { (self.rnd() * j, self.rnd() * j) } else { (0.0, 0.0) };
            out.push([a[0] + dx * t + nx * off + ex, a[1] + dy * t + ny * off + ey]);
        }
    }

    pub fn stroke_pts(&mut self, pts: &[[f32; 2]], closed: bool, w: f32, color: Rgba) {
        if pts.len() < 2 {
            return;
        }
        let mut out = Vec::with_capacity(pts.len() * 4);
        for i in 0..pts.len() - 1 {
            self.rough_seg(pts[i], pts[i + 1], &mut out, i == 0);
        }
        if closed {
            let last = *pts.last().unwrap();
            self.rough_seg(last, pts[0], &mut out, false);
            if self.sketch {
                // overshoot a touch past the start, like a quick pen stroke
                if pts.len() > 1 {
                    let (a, b) = (pts[0], pts[1]);
                    out.push([a[0] + (b[0] - a[0]) * 0.04, a[1] + (b[1] - a[1]) * 0.04]);
                }
            }
        }
        let c = self.a(color);
        self.out.push(Prim::Path { pts: out, closed: closed && !self.sketch, fill: None, stroke: Some((w, c)), convex: true });
    }

    pub fn fill_pts(&mut self, pts: Vec<[f32; 2]>, color: Rgba, convex: bool) {
        let c = self.a(color);
        self.out.push(Prim::Path { pts, closed: true, fill: Some(c), stroke: None, convex });
    }

    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, w: f32, color: Rgba) {
        self.stroke_pts(&[[x1, y1], [x2, y2]], false, w, color);
    }

    pub fn ink_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32) {
        let (w, c) = (self.lw(), self.fg());
        self.line(x1, y1, x2, y2, w, c);
    }

    pub fn dashed(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, w: f32, color: Rgba) {
        let len = ((x2 - x1).powi(2) + (y2 - y1).powi(2)).sqrt();
        let n = (len / 8.0) as usize;
        for i in (0..n).step_by(2) {
            let t0 = i as f32 / n as f32;
            let t1 = (i + 1) as f32 / n as f32;
            let p = |t: f32| (x1 + (x2 - x1) * t, y1 + (y2 - y1) * t);
            let (a, b) = (p(t0), p(t1));
            self.out.push(Prim::Path {
                pts: vec![[a.0, a.1], [b.0, b.1]],
                closed: false,
                fill: None,
                stroke: Some((w, with_alpha(color, self.alpha))),
                convex: true,
            });
        }
    }

    pub fn round_pts(r: R, rad: f32) -> Vec<[f32; 2]> {
        let rad = rad.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
        if rad < 0.5 {
            return vec![[r.x, r.y], [r.r(), r.y], [r.r(), r.b()], [r.x, r.b()]];
        }
        let mut v = Vec::with_capacity(40);
        let steps = if rad > 20.0 { 10 } else { 6 };
        let corners = [
            (r.r() - rad, r.y + rad, -90.0f32),
            (r.r() - rad, r.b() - rad, 0.0),
            (r.x + rad, r.b() - rad, 90.0),
            (r.x + rad, r.y + rad, 180.0),
        ];
        for (cx, cy, a0) in corners {
            for i in 0..=steps {
                let a = (a0 + 90.0 * i as f32 / steps as f32).to_radians();
                v.push([cx + rad * a.cos(), cy + rad * a.sin()]);
            }
        }
        v
    }

    pub fn rect(&mut self, r: R, rad: f32, fill: Option<Rgba>, stroke: Option<Rgba>) {
        let pts = Self::round_pts(r, rad);
        if let Some(f) = fill {
            self.fill_pts(pts.clone(), f, true);
        }
        if let Some(s) = stroke {
            let w = self.lw();
            self.stroke_pts(&pts, true, w, s);
        }
    }

    /// The usual control box: paper (or the fill override) with an ink outline.
    pub fn boxed(&mut self, r: R, rad: f32) {
        let (f, s) = (self.bg(), self.fg());
        self.rect(r, rad, Some(f), Some(s));
    }

    pub fn ellipse_pts(r: R, n: usize) -> Vec<[f32; 2]> {
        (0..n)
            .map(|i| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                [r.cx() + r.w / 2.0 * a.cos(), r.cy() + r.h / 2.0 * a.sin()]
            })
            .collect()
    }

    pub fn ellipse(&mut self, r: R, fill: Option<Rgba>, stroke: Option<Rgba>) {
        let n = ((r.w + r.h) / 4.0).clamp(16.0, 72.0) as usize;
        let pts = Self::ellipse_pts(r, n);
        if let Some(f) = fill {
            self.fill_pts(pts.clone(), f, true);
        }
        if let Some(s) = stroke {
            let w = self.lw();
            if self.sketch {
                // one loop that doesn't quite close, slightly wobbly
                let start = self.rnd() * 3.0;
                let k = self.rnd() * 0.02;
                let mut v = Vec::with_capacity(n + 4);
                for i in 0..=n + 2 {
                    let a = start + i as f32 / n as f32 * std::f32::consts::TAU;
                    let wob = 1.0 + k * (a * 2.0).sin() + self.rnd() * 0.004;
                    v.push([r.cx() + r.w / 2.0 * wob * a.cos(), r.cy() + r.h / 2.0 * wob * a.sin()]);
                }
                let c = self.a(s);
                self.out.push(Prim::Path { pts: v, closed: false, fill: None, stroke: Some((w, c)), convex: true });
            } else {
                self.stroke_pts(&pts, true, w, s);
            }
        }
    }

    pub fn circle(&mut self, cx: f32, cy: f32, rad: f32, fill: Option<Rgba>, stroke: Option<Rgba>) {
        self.ellipse(R::new(cx - rad, cy - rad, rad * 2.0, rad * 2.0), fill, stroke);
    }

    pub fn arc_pts(cx: f32, cy: f32, rad: f32, a0: f32, a1: f32) -> Vec<[f32; 2]> {
        let n = (((a1 - a0).abs() * rad / 6.0) as usize).clamp(6, 80);
        (0..=n)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / n as f32;
                [cx + rad * a.cos(), cy + rad * a.sin()]
            })
            .collect()
    }

    // ---- text ----
    pub fn tw(&self, s: &str, size: f32, bold: bool) -> f32 {
        (self.measure)(s, size, bold)
    }

    /// Text whose vertical centre is at y. hx: 0 left, 0.5 centre, 1 right of x.
    pub fn text(&mut self, x: f32, y: f32, s: &str, size: f32, color: Rgba, hx: f32) {
        self.text_ex(x, y, s, size, color, hx, self.bold, false);
    }

    pub fn text_ex(&mut self, x: f32, y: f32, s: &str, size: f32, color: Rgba, hx: f32, bold: bool, mono: bool) {
        if s.is_empty() {
            return;
        }
        let c = self.a(color);
        self.out.push(Prim::Text {
            x,
            y,
            s: s.to_string(),
            size,
            color: c,
            bold,
            italic: self.italic,
            underline: self.underline,
            mono,
            hx,
        });
    }

    /// Text centred in a box, shortened with … if it doesn't fit.
    pub fn text_in(&mut self, r: R, s: &str, size: f32, color: Rgba, hx: f32) {
        let fit = self.fit(s, r.w, size, self.bold);
        let x = r.x + r.w * hx;
        self.text(x, r.cy(), &fit, size, color, hx);
    }

    pub fn fit(&self, s: &str, w: f32, size: f32, bold: bool) -> String {
        if self.tw(s, size, bold) <= w + 1.0 {
            return s.to_string();
        }
        let chars: Vec<char> = s.chars().collect();
        let mut n = chars.len();
        while n > 0 {
            let t: String = chars[..n].iter().collect::<String>() + "…";
            if self.tw(&t, size, bold) <= w {
                return t;
            }
            n -= 1;
        }
        String::new()
    }

    pub fn wrap(&self, s: &str, w: f32, size: f32, bold: bool) -> Vec<String> {
        let mut lines = Vec::new();
        for para in s.split('\n') {
            let mut cur = String::new();
            for word in para.split(' ') {
                let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
                if self.tw(&cand, size, bold) > w && !cur.is_empty() {
                    lines.push(std::mem::take(&mut cur));
                    cur = word.to_string();
                } else {
                    cur = cand;
                }
            }
            lines.push(cur);
        }
        lines
    }

    /// Wrapped text from the top-left of a box; returns the height used.
    pub fn para(&mut self, r: R, s: &str, size: f32, color: Rgba, hx: f32) -> f32 {
        let lh = size * 1.35;
        let lines = self.wrap(s, r.w, size, self.bold);
        let mut y = r.y + lh / 2.0;
        for l in &lines {
            if y + lh / 2.0 > r.b() + 2.0 {
                break;
            }
            self.text(r.x + r.w * hx, y, l, size, color, hx);
            y += lh;
        }
        lines.len() as f32 * lh
    }

    /// Grey bars that stand in for text (like a squiggle in a sketch).
    pub fn lorem_bars(&mut self, r: R, lines: usize) {
        let lh = 12.0;
        let c = self.mid;
        for i in 0..lines {
            let y = r.y + i as f32 * lh + 4.0;
            if y + 4.0 > r.b() {
                break;
            }
            let w = if i + 1 == lines { r.w * 0.6 } else { r.w * (0.86 + 0.1 * ((i * 7 % 3) as f32 / 2.0)) };
            self.rect(R::new(r.x, y, w.min(r.w), 5.0), 2.5, Some(c), None);
        }
    }

    pub fn link(&mut self, r: R, target: &str) {
        self.out.push(Prim::Link { r, target: target.to_string() });
    }
}
