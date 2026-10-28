//! The app icon, drawn in code (two screens under a Wi-Fi signal) so no image files ship.

/// RGBA pixels of a `size` x `size` icon.
pub fn rgba(size: u32) -> Vec<u8> {
    let n = size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for py in 0..size {
        for px in 0..size {
            let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
            // Rounded-square background with a blue-to-teal gradient.
            let bg = cover(rounded_rect(x, y, n * 0.5, n * 0.5, n * 0.46, n * 0.46, n * 0.2));
            if bg <= 0.0 {
                continue;
            }
            let t = y / n;
            let base = [lerp(0.16, 0.08, t), lerp(0.42, 0.62, t), lerp(0.90, 0.62, t)];

            // Two monitor outlines with stands, and Wi-Fi arcs above them.
            let mut white: f32 = 0.0;
            for cx in [0.30, 0.70] {
                let screen = rounded_rect(x, y, n * cx, n * 0.56, n * 0.155, n * 0.12, n * 0.035).abs() - n * 0.025;
                let stand = rounded_rect(x, y, n * cx, n * 0.71, n * 0.02, n * 0.035, 0.0);
                let foot = rounded_rect(x, y, n * cx, n * 0.755, n * 0.075, n * 0.014, n * 0.01);
                white = white.max(cover(screen)).max(cover(stand)).max(cover(foot));
            }
            let (ax, ay) = (n * 0.5, n * 0.40);
            let (dx, dy) = (x - ax, y - ay);
            let r = (dx * dx + dy * dy).sqrt();
            // Arcs open upwards: only within 45 degrees of straight up.
            if dy < 0.0 && dx.abs() <= -dy {
                for radius in [0.10, 0.18] {
                    white = white.max(cover((r - n * radius).abs() - n * 0.022));
                }
            }
            white = white.max(cover(r - n * 0.032));

            let i = ((py * size + px) * 4) as usize;
            for c in 0..3 {
                out[i + c] = (255.0 * lerp(base[c], 1.0, white)) as u8;
            }
            out[i + 3] = (255.0 * bg) as u8;
        }
    }
    out
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// Anti-aliased coverage of a shape from its signed distance in pixels.
fn cover(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

/// Signed distance to a rounded rectangle centred at (cx, cy) with half extents (hx, hy).
fn rounded_rect(x: f32, y: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (x - cx).abs() - (hx - r);
    let qy = (y - cy).abs() - (hy - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// The icon as PNG bytes.
#[allow(dead_code)]
pub fn png(size: u32) -> Vec<u8> {
    crate::clipboard::encode_png(size, size, &rgba(size)).unwrap_or_default()
}
