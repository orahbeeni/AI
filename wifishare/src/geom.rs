//! Screen geometry shared by the server's switching logic and the platform backends.
//!
//! Coordinates are in each machine's own desktop space (macOS points, Windows virtual-desktop
//! pixels, X11/Wayland logical pixels). Each machine sends its monitor rectangles to the peer.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64 && x < self.right() as f64 && y >= self.y as f64 && y < self.bottom() as f64
    }

    /// The point inside this rectangle closest to (x, y).
    pub fn clamp(&self, x: f64, y: f64) -> (f64, f64) {
        (x.clamp(self.x as f64, (self.right() - 1) as f64), y.clamp(self.y as f64, (self.bottom() - 1) as f64))
    }

    fn distance2(&self, x: f64, y: f64) -> f64 {
        let (cx, cy) = self.clamp(x, y);
        (cx - x).powi(2) + (cy - y).powi(2)
    }
}

/// The edge of the server's desktop a client is attached to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl std::str::FromStr for Side {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "left" => Ok(Side::Left),
            "right" => Ok(Side::Right),
            "top" | "up" | "above" => Ok(Side::Top),
            "bottom" | "down" | "below" => Ok(Side::Bottom),
            _ => Err(format!("unknown side '{s}' (use left, right, top or bottom)")),
        }
    }
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Side::Left => "left",
            Side::Right => "right",
            Side::Top => "top",
            Side::Bottom => "bottom",
        })
    }
}

/// All monitors of one machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    pub monitors: Vec<Rect>,
}

impl Screen {
    pub fn single(w: i32, h: i32) -> Self {
        Screen { monitors: vec![Rect::new(0, 0, w, h)] }
    }

    pub fn bbox(&self) -> Rect {
        let Some(first) = self.monitors.first() else { return Rect::new(0, 0, 1920, 1080) };
        let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.right(), first.bottom());
        for m in &self.monitors[1..] {
            x0 = x0.min(m.x);
            y0 = y0.min(m.y);
            x1 = x1.max(m.right());
            y1 = y1.max(m.bottom());
        }
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    pub fn monitor_at(&self, x: f64, y: f64) -> Option<&Rect> {
        self.monitors.iter().find(|m| m.contains(x, y))
    }

    /// The point on any monitor closest to (x, y).
    pub fn nearest(&self, x: f64, y: f64) -> (f64, f64) {
        self.monitors
            .iter()
            .min_by(|a, b| a.distance2(x, y).total_cmp(&b.distance2(x, y)))
            .map(|m| m.clamp(x, y))
            .unwrap_or((x, y))
    }

    pub fn center(&self) -> (f64, f64) {
        let m = self.monitors.first().copied().unwrap_or(Rect::new(0, 0, 1920, 1080));
        ((m.x + m.w / 2) as f64, (m.y + m.h / 2) as f64)
    }

    /// Whether (x, y) sits on the outer edge of the desktop facing `side`.
    pub fn at_edge(&self, side: Side, x: f64, y: f64) -> bool {
        let b = self.bbox();
        match side {
            Side::Left => x <= b.x as f64 + 0.5,
            Side::Right => x >= b.right() as f64 - 1.5,
            Side::Top => y <= b.y as f64 + 0.5,
            Side::Bottom => y >= b.bottom() as f64 - 1.5,
        }
    }

    /// Where along `side` a point at fraction `t` (0..1) lands, nudged `inset` pixels inwards.
    pub fn edge_point(&self, side: Side, t: f64, inset: f64) -> (f64, f64) {
        let b = self.bbox();
        let t = t.clamp(0.0, 1.0);
        let along_x = b.x as f64 + t * (b.w - 1) as f64;
        let along_y = b.y as f64 + t * (b.h - 1) as f64;
        let p = match side {
            Side::Left => (b.x as f64 + inset, along_y),
            Side::Right => ((b.right() - 1) as f64 - inset, along_y),
            Side::Top => (along_x, b.y as f64 + inset),
            Side::Bottom => (along_x, (b.bottom() - 1) as f64 - inset),
        };
        self.nearest(p.0, p.1)
    }

    /// Fraction (0..1) of the way along the axis perpendicular to `side`.
    pub fn fraction(&self, side: Side, x: f64, y: f64) -> f64 {
        let b = self.bbox();
        match side {
            Side::Left | Side::Right => (y - b.y as f64) / (b.h.max(2) - 1) as f64,
            Side::Top | Side::Bottom => (x - b.x as f64) / (b.w.max(2) - 1) as f64,
        }
        .clamp(0.0, 1.0)
    }
}

impl Side {
    pub fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
        }
    }

    /// Whether a movement of (dx, dy) does not point away from this side.
    pub fn pushing(self, dx: f64, dy: f64) -> bool {
        match self {
            Side::Left => dx <= 0.0,
            Side::Right => dx >= 0.0,
            Side::Top => dy <= 0.0,
            Side::Bottom => dy >= 0.0,
        }
    }

    /// Whether (x, y) lies beyond the desktop's outer edge on this side.
    pub fn beyond(self, b: &Rect, x: f64, y: f64) -> bool {
        match self {
            Side::Left => x < b.x as f64,
            Side::Right => x >= b.right() as f64,
            Side::Top => y < b.y as f64,
            Side::Bottom => y >= b.bottom() as f64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triple() -> Screen {
        Screen { monitors: vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1920, 1080), Rect::new(3840, 0, 1920, 1080)] }
    }

    #[test]
    fn bbox_spans_all_monitors() {
        assert_eq!(triple().bbox(), Rect::new(0, 0, 5760, 1080));
    }

    #[test]
    fn edges_and_fractions() {
        let s = triple();
        assert!(s.at_edge(Side::Right, 5759.0, 10.0));
        assert!(!s.at_edge(Side::Right, 3000.0, 10.0));
        assert!(s.at_edge(Side::Left, 0.0, 500.0));
        assert_eq!(s.fraction(Side::Left, 0.0, 1079.0), 1.0);
        assert_eq!(s.edge_point(Side::Left, 0.5, 0.0), (0.0, 539.5));
    }

    #[test]
    fn nearest_clamps_into_gaps() {
        // Second monitor is shorter and sits lower: (2000, 250) is in a dead zone above it.
        let s = Screen { monitors: vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 300, 1280, 720)] };
        assert_eq!(s.nearest(2000.0, 250.0), (2000.0, 300.0));
        assert_eq!(s.nearest(2000.0, 0.0), (1919.0, 0.0));
        assert!(s.monitor_at(2000.0, 0.0).is_none());
    }
}
