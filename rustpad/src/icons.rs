//! Toolbar icons: the colour Fugue set (CC BY 3.0, see `assets/icons/FUGUE-LICENSE.txt`), embedded as PNGs.

use std::collections::HashMap;

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

macro_rules! icon_files {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../assets/icons/", $name, ".png")))),*]
    };
}

const FILES: &[(&str, &[u8])] = icon_files![
    "new", "open", "save", "save_all", "close", "cut", "copy", "paste", "undo", "redo", "find", "replace", "zoom_in",
    "zoom_out", "wrap", "all_chars", "rec", "stop", "play",
];

pub struct Icons {
    map: HashMap<&'static str, TextureHandle>,
}

impl Icons {
    pub fn load(ctx: &egui::Context) -> Icons {
        let mut map = HashMap::new();
        for (name, bytes) in FILES {
            if let Some(img) = decode(bytes) {
                map.insert(*name, ctx.load_texture(format!("icon-{name}"), img, TextureOptions::NEAREST));
            }
        }
        Icons { map }
    }

    pub fn get(&self, name: &str) -> Option<&TextureHandle> {
        self.map.get(name)
    }
}

fn decode(bytes: &[u8]) -> Option<ColorImage> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let (w, h) = (info.width as usize, info.height as usize);
    let px = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some(ColorImage::from_rgba_unmultiplied([w, h], &rgba))
}
