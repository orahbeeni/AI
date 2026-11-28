//! Line icons drawn with the painter, so they stay sharp at any scale and
//! need no icon font. Coordinates are on a 24x24 grid.

use std::f32::consts::{PI, TAU};

use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Cast,
    Sliders,
    Expand,
    Stats,
    List,
    Volume,
    VolumeOff,
    Refresh,
    Folder,
    File,
    Close,
    Copy,
    Phone,
    Monitor,
    Stop,
    Alert,
    Info,
    Upload,
    Download,
    Play,
}

struct Pen<'a> {
    painter: &'a Painter,
    origin: Pos2,
    unit: f32,
    color: Color32,
    stroke: Stroke,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.unit
    }

    /// A line through the points, with round ends.
    fn line(&self, points: &[(f32, f32)]) {
        let pts: Vec<Pos2> = points.iter().map(|&(x, y)| self.at(x, y)).collect();
        self.cap(pts[0]);
        self.cap(pts[pts.len() - 1]);
        self.painter.add(Shape::line(pts, self.stroke));
    }

    fn closed(&self, points: &[(f32, f32)]) {
        let pts: Vec<Pos2> = points.iter().map(|&(x, y)| self.at(x, y)).collect();
        self.painter.add(Shape::closed_line(pts, self.stroke));
    }

    fn cap(&self, p: Pos2) {
        self.painter.circle_filled(p, self.stroke.width * 0.5, self.color);
    }

    /// Angles are in radians, clockwise from the positive x axis.
    fn arc(&self, cx: f32, cy: f32, r: f32, from: f32, to: f32) {
        let steps = 24;
        let points: Vec<(f32, f32)> = (0..=steps)
            .map(|i| {
                let a = from + (to - from) * i as f32 / steps as f32;
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        self.line(&points);
    }

    fn rect(&self, x: f32, y: f32, w: f32, h: f32, r: f32) {
        let rect = Rect::from_min_size(self.at(x, y), vec2(w, h) * self.unit);
        self.painter.rect_stroke(rect, CornerRadius::same((r * self.unit).round() as u8), self.stroke, StrokeKind::Middle);
    }

    fn circle(&self, x: f32, y: f32, r: f32) {
        self.painter.circle_stroke(self.at(x, y), r * self.unit, self.stroke);
    }

    fn dot(&self, x: f32, y: f32, r: f32) {
        self.painter.circle_filled(self.at(x, y), r * self.unit, self.color);
    }
}

/// Draws `icon` centred in `rect`, as large as the shorter side allows.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let side = rect.width().min(rect.height());
    let unit = side / 24.0;
    let origin = rect.center() - vec2(side, side) * 0.5;
    let width = (1.75 * unit).max(1.25);
    let p = Pen { painter, origin, unit, color, stroke: Stroke::new(width, color) };
    match icon {
        Icon::Cast => {
            // screen with its lower-left corner open for the signal arcs
            p.line(&[(3.0, 8.5), (3.0, 6.0), (4.0, 5.0), (20.0, 5.0), (21.0, 6.0), (21.0, 18.0), (20.0, 19.0), (14.0, 19.0)]);
            p.dot(3.6, 18.6, 1.4);
            p.arc(3.0, 19.0, 4.2, -PI / 2.0, 0.0);
            p.arc(3.0, 19.0, 7.8, -PI / 2.0, 0.0);
        }
        Icon::Sliders => {
            for (y, knob) in [(6.0, 15.0), (12.0, 8.0), (18.0, 16.5)] {
                p.line(&[(3.5, y), (knob - 2.6, y)]);
                p.circle(knob, y, 2.4);
                p.line(&[(knob + 2.6, y), (20.5, y)]);
            }
        }
        Icon::Expand => {
            p.line(&[(4.0, 9.0), (4.0, 4.0), (9.0, 4.0)]);
            p.line(&[(15.0, 4.0), (20.0, 4.0), (20.0, 9.0)]);
            p.line(&[(20.0, 15.0), (20.0, 20.0), (15.0, 20.0)]);
            p.line(&[(9.0, 20.0), (4.0, 20.0), (4.0, 15.0)]);
        }
        Icon::Stats => {
            p.line(&[(3.0, 12.0), (7.0, 12.0), (10.0, 4.5), (14.0, 19.5), (17.0, 12.0), (21.0, 12.0)]);
        }
        Icon::List => {
            for y in [6.0, 12.0, 18.0] {
                p.dot(4.5, y, 1.3);
                p.line(&[(9.0, y), (20.0, y)]);
            }
        }
        Icon::Volume | Icon::VolumeOff => {
            p.closed(&[(3.0, 9.0), (7.0, 9.0), (12.0, 4.5), (12.0, 19.5), (7.0, 15.0), (3.0, 15.0)]);
            if icon == Icon::Volume {
                p.arc(12.0, 12.0, 4.0, -0.75, 0.75);
                p.arc(12.0, 12.0, 8.0, -0.85, 0.85);
            } else {
                p.line(&[(15.5, 9.0), (21.5, 15.0)]);
                p.line(&[(21.5, 9.0), (15.5, 15.0)]);
            }
        }
        Icon::Refresh => {
            p.arc(12.0, 12.0, 8.0, 0.35, TAU - 0.55);
            p.line(&[(20.0, 3.5), (19.2, 8.2), (14.5, 7.6)]);
        }
        Icon::Folder => {
            p.closed(&[
                (3.0, 6.0),
                (4.0, 5.0),
                (9.0, 5.0),
                (11.5, 7.5),
                (20.0, 7.5),
                (21.0, 8.5),
                (21.0, 18.0),
                (20.0, 19.0),
                (4.0, 19.0),
                (3.0, 18.0),
            ]);
        }
        Icon::File => {
            p.closed(&[(6.0, 3.0), (14.0, 3.0), (19.0, 8.0), (19.0, 21.0), (6.0, 21.0), (5.0, 20.0), (5.0, 4.0)]);
            p.line(&[(14.0, 3.5), (14.0, 8.0), (18.5, 8.0)]);
        }
        Icon::Close => {
            p.line(&[(6.0, 6.0), (18.0, 18.0)]);
            p.line(&[(18.0, 6.0), (6.0, 18.0)]);
        }
        Icon::Copy => {
            p.rect(8.5, 8.5, 12.0, 12.0, 2.0);
            p.line(&[(5.0, 15.5), (4.0, 15.0), (3.5, 14.0), (3.5, 5.0), (5.0, 3.5), (14.0, 3.5), (15.0, 4.0), (15.5, 5.0)]);
        }
        Icon::Phone => {
            p.rect(6.5, 2.5, 11.0, 19.0, 2.5);
            p.line(&[(10.5, 18.5), (13.5, 18.5)]);
        }
        Icon::Monitor => {
            p.rect(2.5, 4.0, 19.0, 13.0, 2.0);
            p.line(&[(12.0, 17.0), (12.0, 20.5)]);
            p.line(&[(8.0, 20.5), (16.0, 20.5)]);
        }
        Icon::Stop => {
            p.circle(12.0, 12.0, 9.0);
            p.painter.rect_filled(Rect::from_min_max(p.at(9.0, 9.0), p.at(15.0, 15.0)), 1.5 * unit, color);
        }
        Icon::Alert => {
            p.closed(&[(12.0, 3.5), (21.5, 20.0), (2.5, 20.0)]);
            p.line(&[(12.0, 9.5), (12.0, 14.0)]);
            p.dot(12.0, 17.0, 1.2);
        }
        Icon::Info => {
            p.circle(12.0, 12.0, 9.0);
            p.line(&[(12.0, 11.0), (12.0, 16.5)]);
            p.dot(12.0, 7.7, 1.2);
        }
        Icon::Upload | Icon::Download => {
            let (tip, wing) = if icon == Icon::Upload { (3.5, 8.5) } else { (15.5, 10.5) };
            let tail = if icon == Icon::Upload { 15.5 } else { 3.5 };
            p.line(&[(12.0, tail), (12.0, tip)]);
            p.line(&[(7.0, wing), (12.0, tip), (17.0, wing)]);
            p.line(&[(4.0, 15.0), (4.0, 19.0), (5.0, 20.0), (19.0, 20.0), (20.0, 19.0), (20.0, 15.0)]);
        }
        Icon::Play => {
            p.closed(&[(7.5, 4.5), (19.5, 12.0), (7.5, 19.5)]);
        }
    }
}

/// Reserves a square of `size` in the layout and draws the icon there.
pub fn show(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    paint(ui.painter(), rect, icon, color);
    resp
}

/// A rounded tile with an icon, used as a visual anchor on cards and rows.
pub fn tile(painter: &Painter, center: Pos2, size: f32, icon: Icon, color: Color32) {
    let rect = Rect::from_center_size(center, vec2(size, size));
    painter.rect_filled(rect, size * 0.28, super::theme::tint(color, 0.14));
    paint(painter, Rect::from_center_size(center, vec2(size, size) * 0.56), icon, color);
}

/// Convenience for [`tile`] inside a layout.
pub fn show_tile(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    tile(ui.painter(), rect.center(), size, icon, color);
}
