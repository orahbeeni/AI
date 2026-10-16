//! The split tree of a tab: leaves are terminals, inner nodes split their area in two.

use eframe::egui::{Pos2, Rect, Vec2, pos2};
use serde::{Deserialize, Serialize};

/// How a split arranges its two children.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Dir {
    /// Side by side, with a vertical divider (Terminator's "split vertically").
    Row,
    /// Stacked, with a horizontal divider (Terminator's "split horizontally").
    Col,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum Node<L> {
    Leaf(L),
    Split { dir: Dir, ratio: f32, a: Box<Node<L>>, b: Box<Node<L>> },
}

/// A divider between two children, for hit testing and dragging.
pub struct Divider {
    /// Path from the root to the split: false = a, true = b.
    pub path: Vec<bool>,
    pub dir: Dir,
    /// The divider's own rectangle.
    pub rect: Rect,
    /// The area of the whole split, used to turn a pointer position into a ratio.
    pub area: Rect,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Left,
    Right,
}

impl<L: Clone + PartialEq> Node<L> {
    pub fn leaves(&self) -> Vec<L> {
        let mut v = Vec::new();
        self.collect(&mut v);
        v
    }

    fn collect(&self, v: &mut Vec<L>) {
        match self {
            Node::Leaf(l) => v.push(l.clone()),
            Node::Split { a, b, .. } => {
                a.collect(v);
                b.collect(v);
            }
        }
    }

    pub fn contains(&self, id: &L) -> bool {
        match self {
            Node::Leaf(l) => l == id,
            Node::Split { a, b, .. } => a.contains(id) || b.contains(id),
        }
    }

    /// Replace leaf `target` with a split holding it and `new` (new goes second).
    pub fn split(&mut self, target: &L, dir: Dir, new: L) -> bool {
        match self {
            Node::Leaf(l) if l == target => {
                let old = Node::Leaf(l.clone());
                *self = Node::Split { dir, ratio: 0.5, a: Box::new(old), b: Box::new(Node::Leaf(new)) };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(target, dir, new.clone()) || b.split(target, dir, new),
        }
    }

    /// Remove a leaf, letting its sibling take the parent's place. Returns false when `id` is
    /// the only leaf (the caller closes the tab instead).
    pub fn remove(&mut self, id: &L) -> bool {
        let Node::Split { a, b, .. } = self else { return false };
        if matches!(a.as_ref(), Node::Leaf(l) if l == id) {
            *self = (**b).clone();
            return true;
        }
        if matches!(b.as_ref(), Node::Leaf(l) if l == id) {
            *self = (**a).clone();
            return true;
        }
        a.remove(id) || b.remove(id)
    }

    pub fn map<M>(&self, f: &mut impl FnMut(&L) -> M) -> Node<M> {
        match self {
            Node::Leaf(l) => Node::Leaf(f(l)),
            Node::Split { dir, ratio, a, b } => Node::Split { dir: *dir, ratio: *ratio, a: Box::new(a.map(f)), b: Box::new(b.map(f)) },
        }
    }

    /// Lay the tree out in `area`, giving each leaf its rectangle and each split its divider.
    pub fn layout(&self, area: Rect, gap: f32, out: &mut Vec<(L, Rect)>, dividers: &mut Vec<Divider>) {
        self.layout_at(area, gap, &mut Vec::new(), out, dividers);
    }

    fn layout_at(&self, area: Rect, gap: f32, path: &mut Vec<bool>, out: &mut Vec<(L, Rect)>, dividers: &mut Vec<Divider>) {
        match self {
            Node::Leaf(l) => out.push((l.clone(), area)),
            Node::Split { dir, ratio, a, b } => {
                let (ra, rd, rb) = split_rect(area, *dir, *ratio, gap);
                dividers.push(Divider { path: path.clone(), dir: *dir, rect: rd, area });
                path.push(false);
                a.layout_at(ra, gap, path, out, dividers);
                path.pop();
                path.push(true);
                b.layout_at(rb, gap, path, out, dividers);
                path.pop();
            }
        }
    }

    fn at_path(&mut self, path: &[bool]) -> Option<&mut Node<L>> {
        match path.split_first() {
            None => Some(self),
            Some((first, rest)) => match self {
                Node::Split { a, b, .. } => if *first { b } else { a }.at_path(rest),
                Node::Leaf(_) => None,
            },
        }
    }

    pub fn set_ratio(&mut self, path: &[bool], value: f32) {
        if let Some(Node::Split { ratio, .. }) = self.at_path(path) {
            *ratio = value.clamp(0.05, 0.95);
        }
    }

    /// Move the nearest enclosing divider of `id` that runs across `nav`, by `step` of its split.
    pub fn nudge(&mut self, id: &L, nav: Nav, step: f32) -> bool {
        let want = if matches!(nav, Nav::Left | Nav::Right) { Dir::Row } else { Dir::Col };
        let delta = if matches!(nav, Nav::Left | Nav::Up) { -step } else { step };
        self.nudge_inner(id, want, delta)
    }

    fn nudge_inner(&mut self, id: &L, want: Dir, delta: f32) -> bool {
        let Node::Split { dir, ratio, a, b } = self else { return false };
        let inner = if a.contains(id) { a.nudge_inner(id, want, delta) } else { b.nudge_inner(id, want, delta) };
        if inner {
            return true;
        }
        if *dir == want && (a.contains(id) || b.contains(id)) {
            *ratio = (*ratio + delta).clamp(0.05, 0.95);
            return true;
        }
        false
    }

    pub fn equalize(&mut self) {
        if let Node::Split { ratio, a, b, dir } = self {
            // Weight by leaf count along the same direction so N panes in a row end up equal.
            let wa = a.count_along(*dir) as f32;
            let wb = b.count_along(*dir) as f32;
            *ratio = wa / (wa + wb);
            a.equalize();
            b.equalize();
        }
    }

    fn count_along(&self, d: Dir) -> usize {
        match self {
            Node::Split { dir, a, b, .. } if *dir == d => a.count_along(d) + b.count_along(d),
            _ => 1,
        }
    }
}

pub fn split_rect(area: Rect, dir: Dir, ratio: f32, gap: f32) -> (Rect, Rect, Rect) {
    match dir {
        Dir::Row => {
            let x = (area.min.x + (area.width() - gap) * ratio).round();
            (
                Rect::from_min_max(area.min, pos2(x, area.max.y)),
                Rect::from_min_max(pos2(x, area.min.y), pos2(x + gap, area.max.y)),
                Rect::from_min_max(pos2(x + gap, area.min.y), area.max),
            )
        }
        Dir::Col => {
            let y = (area.min.y + (area.height() - gap) * ratio).round();
            (
                Rect::from_min_max(area.min, pos2(area.max.x, y)),
                Rect::from_min_max(pos2(area.min.x, y), pos2(area.max.x, y + gap)),
                Rect::from_min_max(pos2(area.min.x, y + gap), area.max),
            )
        }
    }
}

/// The ratio a divider should take for a pointer at `p`.
pub fn ratio_for(d: &Divider, p: Pos2, gap: f32) -> f32 {
    match d.dir {
        Dir::Row => (p.x - d.area.min.x - gap / 2.0) / (d.area.width() - gap).max(1.0),
        Dir::Col => (p.y - d.area.min.y - gap / 2.0) / (d.area.height() - gap).max(1.0),
    }
}

/// The best neighbour of `from` in direction `nav`, among laid-out rectangles.
pub fn neighbour<L: Clone + PartialEq>(rects: &[(L, Rect)], from: &L, nav: Nav) -> Option<L> {
    let (_, r) = rects.iter().find(|(l, _)| l == from)?;
    let c = r.center();
    let mut best: Option<(f32, L)> = None;
    for (l, o) in rects {
        if l == from {
            continue;
        }
        let ok = match nav {
            Nav::Left => o.max.x <= r.min.x + 1.0,
            Nav::Right => o.min.x >= r.max.x - 1.0,
            Nav::Up => o.max.y <= r.min.y + 1.0,
            Nav::Down => o.min.y >= r.max.y - 1.0,
        };
        if !ok {
            continue;
        }
        // Prefer panes that overlap on the perpendicular axis, then the closest.
        let overlap = match nav {
            Nav::Left | Nav::Right => o.min.y < r.max.y && o.max.y > r.min.y,
            Nav::Up | Nav::Down => o.min.x < r.max.x && o.max.x > r.min.x,
        };
        let d: Vec2 = o.center() - c;
        let score = d.length() + if overlap { 0.0 } else { 10_000.0 };
        if best.as_ref().is_none_or(|(s, _)| score < *s) {
            best = Some((score, l.clone()));
        }
    }
    best.map(|(_, l)| l)
}
