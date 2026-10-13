//! The logo and icons, embedded in the executable.
//!
//! * `logo.png`: the full logo (snail and name), used on the splash screen and
//!   the welcome page.
//! * `icon.png`: the snail alone on a rounded square, used as the application
//!   icon in the dock, task bar and window switcher (no name, so it stays legible).
//! * `mark.png`: the snail alone on a transparent background, for tiny sizes.

use eframe::egui::{self, ColorImage, IconData, TextureHandle, TextureOptions};

const LOGO: &[u8] = include_bytes!("../assets/logo.png");
const ICON: &[u8] = include_bytes!("../assets/icon.png");
const MARK: &[u8] = include_bytes!("../assets/mark.png");

/// Decode a PNG into straight (non-premultiplied) RGBA8.
fn decode(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // Expand palettes and low bit depths, and reduce 16-bit channels to 8.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let px = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.as_chunks::<3>().0.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.as_chunks::<2>().0.iter().flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed PNG was not expanded".into()),
    };
    Ok((rgba, info.width, info.height))
}

fn color_image(bytes: &[u8]) -> ColorImage {
    match decode(bytes) {
        Ok((rgba, w, h)) => ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba),
        // The assets are compiled in, so this cannot happen; fall back to a blank pixel.
        Err(_) => ColorImage::new([1, 1], egui::Color32::TRANSPARENT),
    }
}

/// The icon for the window, dock and task bar.
pub fn window_icon() -> IconData {
    match decode(ICON) {
        Ok((rgba, width, height)) => IconData { rgba, width, height },
        Err(_) => IconData::default(),
    }
}

pub struct Assets {
    pub logo: TextureHandle,
    pub mark: TextureHandle,
}

impl Assets {
    pub fn load(ctx: &egui::Context) -> Self {
        Assets {
            logo: ctx.load_texture("gitgoing-logo", color_image(LOGO), TextureOptions::LINEAR),
            mark: ctx.load_texture("gitgoing-mark", color_image(MARK), TextureOptions::LINEAR),
        }
    }
}

/// Where the tiny mark is kept so any panel can draw it.
fn mark_id() -> egui::Id {
    egui::Id::new("gitgoing-mark-texture")
}

pub fn share_mark(ctx: &egui::Context, mark: &TextureHandle) {
    ctx.data_mut(|d| d.insert_temp(mark_id(), mark.clone()));
}

/// The tiny mark, if the assets have been loaded.
pub fn mark(ctx: &egui::Context) -> Option<TextureHandle> {
    ctx.data(|d| d.get_temp(mark_id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_images_decode_with_the_expected_sizes() {
        for (name, bytes, size) in [("logo", LOGO, 512), ("icon", ICON, 512), ("mark", MARK, 96)] {
            let (rgba, w, h) = decode(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!((w, h), (size, size), "{name}");
            assert_eq!(rgba.len(), (w * h * 4) as usize, "{name}");
        }
    }

    #[test]
    fn the_icon_has_transparent_corners_and_opaque_centre() {
        let (rgba, w, _) = decode(ICON).unwrap();
        let alpha = |x: u32, y: u32| rgba[((y * w + x) * 4 + 3) as usize];
        assert_eq!(alpha(2, 2), 0, "corner is transparent");
        assert_eq!(alpha(256, 256), 255, "centre is opaque");
        // The window icon wraps the same data.
        let icon = window_icon();
        assert_eq!((icon.width, icon.height), (512, 512));
    }

    #[test]
    fn the_mark_is_a_transparent_cutout() {
        let (rgba, w, _) = decode(MARK).unwrap();
        let alpha = |x: u32, y: u32| rgba[((y * w + x) * 4 + 3) as usize];
        assert_eq!(alpha(1, 1), 0, "background is transparent");
        assert!((0..96).any(|y| (0..96).any(|x| alpha(x, y) > 200)), "the snail itself is opaque");
    }
}
