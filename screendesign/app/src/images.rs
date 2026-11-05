//! Pictures the user adds to a design: PNG, JPEG, GIF, WebP, BMP, ICO, TIFF and SVG.
//! They are stored inside the project file, drawn through textures and exported as data URIs.

use crate::pen::R;
use base64::Engine;
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;
use std::sync::Arc;

/// Larger pictures are shrunk when they are added, to keep project files small.
pub const MAX_SIDE: u32 = 2560;

/// How a picture sits in its box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    /// Fill the box, changing the proportions.
    #[default]
    Stretch,
    /// The whole picture, keeping its proportions.
    Contain,
    /// Fill the box, keeping proportions and cropping the overflow.
    Cover,
}

impl Fit {
    pub const ALL: [Fit; 3] = [Fit::Stretch, Fit::Contain, Fit::Cover];
    pub fn label(self) -> &'static str {
        match self {
            Fit::Stretch => "Stretch to the box",
            Fit::Contain => "Fit inside (keep proportions)",
            Fit::Cover => "Fill and crop (keep proportions)",
        }
    }
    pub fn parse(s: &str) -> Option<Fit> {
        match s.to_ascii_lowercase().as_str() {
            "stretch" => Some(Fit::Stretch),
            "contain" | "fit" => Some(Fit::Contain),
            "cover" | "fill" | "crop" => Some(Fit::Cover),
            _ => None,
        }
    }
    /// The SVG `preserveAspectRatio` value that draws the same thing.
    pub fn svg_aspect(self) -> &'static str {
        match self {
            Fit::Stretch => "none",
            Fit::Contain => "xMidYMid meet",
            Fit::Cover => "xMidYMid slice",
        }
    }
}

pub struct Img {
    pub name: String,
    /// The file as stored: PNG, JPEG, GIF, WebP or SVG.
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    /// Natural size in pixels (SVG: its own size).
    pub w: f32,
    pub h: f32,
    pub key: u64,
}

pub type ImgRef = Arc<Img>;

impl std::fmt::Debug for Img {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Img({}, {}x{}, {} bytes)", self.name, self.w, self.h, self.bytes.len())
    }
}

impl PartialEq for Img {
    fn eq(&self, o: &Img) -> bool {
        self.key == o.key && self.name == o.name
    }
}
impl Eq for Img {}

pub fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

pub fn sniff(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if b.starts_with(b"GIF8") {
        Some("image/gif")
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else if b.starts_with(b"BM") {
        Some("image/bmp")
    } else if b.starts_with(&[0, 0, 1, 0]) {
        Some("image/x-icon")
    } else if b.starts_with(b"II*\0") || b.starts_with(b"MM\0*") {
        Some("image/tiff")
    } else {
        let head = String::from_utf8_lossy(&b[..b.len().min(4096)]);
        if head.contains("<svg") {
            Some("image/svg+xml")
        } else {
            None
        }
    }
}

pub const FILE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tif", "tiff", "svg"];

pub fn is_image_path(p: &std::path::Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| FILE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())).unwrap_or(false)
}

impl Img {
    /// Reads a picture file's bytes. Pictures that browsers can't show (BMP, ICO, TIFF), that are
    /// rotated by EXIF, or that are very large are converted to PNG/JPEG first.
    pub fn from_bytes(name: &str, raw: Vec<u8>) -> Result<Img, String> {
        let mime = sniff(&raw).ok_or("that is not a picture Sketchframe can read (use PNG, JPEG, GIF, WebP, BMP, ICO, TIFF or SVG)")?;
        let name = name.to_string();
        if mime == "image/svg+xml" {
            let tree = resvg::usvg::Tree::from_data(&raw, &resvg::usvg::Options::default()).map_err(|e| format!("can't read this SVG: {e}"))?;
            let s = tree.size();
            return Ok(Img { name, mime, w: s.width().max(1.0), h: s.height().max(1.0), key: fnv(&raw), bytes: raw });
        }
        let mut decoder = image::ImageReader::new(Cursor::new(&raw))
            .with_guessed_format()
            .map_err(|e| e.to_string())?
            .into_decoder()
            .map_err(|e| format!("can't read this picture: {e}"))?;
        let orientation = image::ImageDecoder::orientation(&mut decoder).unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut pic = image::DynamicImage::from_decoder(decoder).map_err(|e| format!("can't read this picture: {e}"))?;
        pic.apply_orientation(orientation);
        let (w, h) = (pic.width(), pic.height());
        let native = matches!(mime, "image/png" | "image/jpeg" | "image/gif" | "image/webp");
        if native && orientation == image::metadata::Orientation::NoTransforms && w.max(h) <= MAX_SIDE {
            return Ok(Img { name, mime, w: w as f32, h: h as f32, key: fnv(&raw), bytes: raw });
        }
        if w.max(h) > MAX_SIDE {
            pic = pic.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Lanczos3);
        }
        let (bytes, mime) = encode(&pic, mime == "image/jpeg")?;
        Ok(Img { name, mime, w: pic.width() as f32, h: pic.height() as f32, key: fnv(&bytes), bytes })
    }

    pub fn from_rgba(name: &str, w: u32, h: u32, rgba: Vec<u8>) -> Result<Img, String> {
        let buf = image::RgbaImage::from_raw(w, h, rgba).ok_or("bad picture data")?;
        let mut pic = image::DynamicImage::ImageRgba8(buf);
        if w.max(h) > MAX_SIDE {
            pic = pic.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Lanczos3);
        }
        let (bytes, mime) = encode(&pic, false)?;
        Ok(Img { name: name.to_string(), mime, w: pic.width() as f32, h: pic.height() as f32, key: fnv(&bytes), bytes })
    }

    pub fn id(&self) -> String {
        format!("img_{:016x}", self.key)
    }

    pub fn is_svg(&self) -> bool {
        self.mime == "image/svg+xml"
    }

    pub fn data_uri(&self) -> String {
        format!("data:{};base64,{}", self.mime, base64::engine::general_purpose::STANDARD.encode(&self.bytes))
    }

    /// The picture as pixels. SVGs are drawn `px` pixels wide.
    pub fn color_image(&self, px: u32) -> Option<egui::ColorImage> {
        if self.is_svg() {
            use resvg::{tiny_skia, usvg};
            let mut opt = usvg::Options::default();
            opt.fontdb = crate::render::fontdb();
            let tree = usvg::Tree::from_data(&self.bytes, &opt).ok()?;
            let size = tree.size();
            let scale = px.clamp(16, 4096) as f32 / size.width().max(1.0);
            let w = ((size.width() * scale).ceil() as u32).clamp(1, 4096);
            let h = ((size.height() * scale).ceil() as u32).clamp(1, 4096);
            let mut pm = tiny_skia::Pixmap::new(w, h)?;
            resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pm.as_mut());
            Some(egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pm.data()))
        } else {
            let pic = image::load_from_memory(&self.bytes).ok()?.to_rgba8();
            Some(egui::ColorImage::from_rgba_unmultiplied([pic.width() as usize, pic.height() as usize], pic.as_raw()))
        }
    }

    /// Unmultiplied RGBA of a raster picture.
    pub fn rgba(&self) -> Option<(u32, u32, Vec<u8>)> {
        if self.is_svg() {
            return None;
        }
        let pic = image::load_from_memory(&self.bytes).ok()?.to_rgba8();
        Some((pic.width(), pic.height(), pic.into_raw()))
    }

    /// Suggested file extension when saving it out.
    pub fn extension(&self) -> &'static str {
        match self.mime {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "image/svg+xml" => "svg",
            _ => "png",
        }
    }
}

fn encode(pic: &image::DynamicImage, prefer_jpeg: bool) -> Result<(Vec<u8>, &'static str), String> {
    let mut out = Cursor::new(Vec::new());
    if prefer_jpeg && !pic.color().has_alpha() {
        let rgb = image::DynamicImage::ImageRgb8(pic.to_rgb8());
        rgb.write_to(&mut out, image::ImageFormat::Jpeg).map_err(|e| e.to_string())?;
        Ok((out.into_inner(), "image/jpeg"))
    } else {
        pic.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
        Ok((out.into_inner(), "image/png"))
    }
}

// ---------------------------------------------------------------- where it lands in the box

/// The part of the box the picture covers and the part of the picture shown (uv).
pub fn fit_rect(r: R, iw: f32, ih: f32, fit: Fit) -> (R, egui::Rect) {
    let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    if iw <= 0.0 || ih <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
        return (r, full);
    }
    match fit {
        Fit::Stretch => (r, full),
        Fit::Contain => {
            let s = (r.w / iw).min(r.h / ih);
            let (w, h) = (iw * s, ih * s);
            (R::new(r.x + (r.w - w) / 2.0, r.y + (r.h - h) / 2.0, w, h), full)
        }
        Fit::Cover => {
            let s = (r.w / iw).max(r.h / ih);
            let (u, v) = (r.w / (iw * s), r.h / (ih * s));
            (r, egui::Rect::from_min_max(egui::pos2((1.0 - u) / 2.0, (1.0 - v) / 2.0), egui::pos2((1.0 + u) / 2.0, (1.0 + v) / 2.0)))
        }
    }
}

// ---------------------------------------------------------------- textures

thread_local! {
    /// picture key -> (SVG raster width, texture). `None` when the picture can't be decoded.
    static TEXTURES: RefCell<HashMap<u64, (u32, Option<egui::TextureHandle>)>> = RefCell::new(HashMap::new());
}

/// The texture for a picture; SVGs are re-drawn when shown much larger than before.
pub fn texture(ctx: &egui::Context, img: &Img, want_px: f32) -> Option<egui::TextureId> {
    TEXTURES.with(|t| {
        let mut t = t.borrow_mut();
        let bucket = if img.is_svg() { (want_px.clamp(64.0, 4096.0) as u32).next_power_of_two().min(4096) } else { 0 };
        let stale = match t.get(&img.key) {
            None => true,
            Some((have, _)) => img.is_svg() && (*have < bucket || *have > bucket * 4),
        };
        if stale {
            let handle = img.color_image(bucket).map(|ci| ctx.load_texture(format!("picture-{:x}", img.key), ci, egui::TextureOptions::LINEAR));
            t.insert(img.key, (bucket, handle));
        }
        t.get(&img.key).and_then(|(_, h)| h.as_ref().map(|h| h.id()))
    })
}

// ---------------------------------------------------------------- project storage

/// Pictures keyed by id, stored in the project file as {name, data (base64)}.
pub mod store {
    use super::*;
    use serde::{Deserializer, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Stored {
        name: String,
        data: String,
    }

    pub fn serialize<S: Serializer>(m: &BTreeMap<String, ImgRef>, s: S) -> Result<S::Ok, S::Error> {
        let stored: BTreeMap<&String, Stored> = m
            .iter()
            .map(|(k, v)| (k, Stored { name: v.name.clone(), data: base64::engine::general_purpose::STANDARD.encode(&v.bytes) }))
            .collect();
        stored.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, ImgRef>, D::Error> {
        let stored = BTreeMap::<String, Stored>::deserialize(d)?;
        let mut out = BTreeMap::new();
        for (id, s) in stored {
            // a picture that can't be read is dropped; its parts show the "choose an image" placeholder
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(s.data.as_bytes()) else { continue };
            let Ok(img) = Img::from_bytes(&s.name, bytes) else { continue };
            out.insert(id, Arc::new(img));
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- make the background transparent

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BgParams {
    pub color: [u8; 3],
    /// 0..100: how far from `color` a pixel may be and still go.
    pub tolerance: f32,
    /// Only remove areas that touch the picture's edge, so the same colour inside the subject stays.
    pub from_edges: bool,
}

fn distance(p: &[u8], c: [u8; 3]) -> f32 {
    let d = |a: u8, b: u8| (a as f32 - b as f32).powi(2);
    (d(p[0], c[0]) + d(p[1], c[1]) + d(p[2], c[2])).sqrt() / 441.67 * 100.0
}

/// Makes pixels close to `p.color` transparent, softening the edge of what remains.
pub fn remove_background(w: usize, h: usize, rgba: &mut [u8], p: &BgParams) {
    let n = w * h;
    if n == 0 || rgba.len() < n * 4 {
        return;
    }
    let matches = |i: usize| rgba[i * 4 + 3] == 0 || distance(&rgba[i * 4..i * 4 + 3], p.color) <= p.tolerance;
    let mut gone = vec![false; n];
    if p.from_edges {
        let mut stack = vec![];
        let seed = |i: usize, gone: &mut Vec<bool>, stack: &mut Vec<usize>| {
            if !gone[i] && matches(i) {
                gone[i] = true;
                stack.push(i);
            }
        };
        for x in 0..w {
            seed(x, &mut gone, &mut stack);
            seed((h - 1) * w + x, &mut gone, &mut stack);
        }
        for y in 0..h {
            seed(y * w, &mut gone, &mut stack);
            seed(y * w + w - 1, &mut gone, &mut stack);
        }
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let mut next = [None; 4];
            if x > 0 {
                next[0] = Some(i - 1);
            }
            if x + 1 < w {
                next[1] = Some(i + 1);
            }
            if y > 0 {
                next[2] = Some(i - w);
            }
            if y + 1 < h {
                next[3] = Some(i + w);
            }
            for j in next.into_iter().flatten() {
                seed(j, &mut gone, &mut stack);
            }
        }
    } else {
        for (i, g) in gone.iter_mut().enumerate() {
            *g = matches(i);
        }
    }
    // soft edge: pixels next to a removed one fade out the closer they are to the background colour
    let soft = (p.tolerance * 0.5).max(6.0);
    let mut alpha: Vec<u8> = (0..n).map(|i| rgba[i * 4 + 3]).collect();
    for i in 0..n {
        if gone[i] {
            alpha[i] = 0;
            continue;
        }
        let (x, y) = (i % w, i / w);
        let touches = (x > 0 && gone[i - 1]) || (x + 1 < w && gone[i + 1]) || (y > 0 && gone[i - w]) || (y + 1 < h && gone[i + w]);
        if touches {
            let d = distance(&rgba[i * 4..i * 4 + 3], p.color);
            if d < p.tolerance + soft {
                let f = ((d - p.tolerance) / soft).clamp(0.0, 1.0);
                alpha[i] = (alpha[i] as f32 * f) as u8;
            }
        }
    }
    for i in 0..n {
        rgba[i * 4 + 3] = alpha[i];
    }
}

/// A guess at the background: the colour most of the picture's corners share.
pub fn guess_background(w: usize, h: usize, rgba: &[u8]) -> [u8; 3] {
    if w == 0 || h == 0 {
        return [255, 255, 255];
    }
    let px = |x: usize, y: usize| {
        let i = (y * w + x) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2]]
    };
    let corners = [px(0, 0), px(w - 1, 0), px(0, h - 1), px(w - 1, h - 1)];
    corners
        .iter()
        .max_by_key(|c| corners.iter().filter(|o| distance(&[o[0], o[1], o[2]], **c) < 8.0).count())
        .copied()
        .unwrap_or([255, 255, 255])
}

/// A shrunken copy for the live preview.
pub fn thumbnail(w: u32, h: u32, rgba: &[u8], max: u32) -> (usize, usize, Vec<u8>) {
    let Some(buf) = image::RgbaImage::from_raw(w, h, rgba.to_vec()) else { return (0, 0, vec![]) };
    let small = image::DynamicImage::ImageRgba8(buf).resize(max, max, image::imageops::FilterType::Triangle).to_rgba8();
    (small.width() as usize, small.height() as usize, small.into_raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white_photo() -> (usize, usize, Vec<u8>) {
        // white background, green square, and a white dot inside the square
        let (w, h) = (60usize, 40usize);
        let mut px = vec![255u8; w * h * 4];
        for y in 10..30 {
            for x in 15..45 {
                let i = (y * w + x) * 4;
                px[i..i + 4].copy_from_slice(&[30, 160, 80, 255]);
            }
        }
        for y in 18..22 {
            for x in 28..32 {
                let i = (y * w + x) * 4;
                px[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        (w, h, px)
    }

    #[test]
    fn background_goes_but_the_inside_stays() {
        let (w, h, mut px) = white_photo();
        let color = guess_background(w, h, &px);
        assert_eq!(color, [255, 255, 255]);
        remove_background(w, h, &mut px, &BgParams { color, tolerance: 10.0, from_edges: true });
        let a = |x: usize, y: usize| px[(y * w + x) * 4 + 3];
        assert_eq!(a(0, 0), 0, "corner is transparent");
        assert_eq!(a(w - 1, h - 1), 0);
        assert_eq!(a(20, 15), 255, "the subject stays");
        assert_eq!(a(29, 19), 255, "white inside the subject stays when only edges are removed");
        let (_, _, mut all) = white_photo();
        remove_background(w, h, &mut all, &BgParams { color, tolerance: 10.0, from_edges: false });
        assert_eq!(all[(19 * w + 29) * 4 + 3], 0, "everywhere mode removes the inner white too");
    }

    #[test]
    fn picture_round_trip_and_unused_pictures_are_dropped() {
        let (w, h, px) = white_photo();
        let png = Img::from_rgba("dot.png", w as u32, h as u32, px).unwrap();
        assert_eq!(png.mime, "image/png");
        let svg = Img::from_bytes("a.svg", br#"<svg xmlns="http://www.w3.org/2000/svg" width="30" height="20"><rect width="30" height="20"/></svg>"#.to_vec()).unwrap();
        assert_eq!((svg.w, svg.h), (30.0, 20.0));
        let mut p = crate::model::Project::empty();
        p.add_picture(0, png, None, None);
        let unused = p.add_image(svg);
        let path = std::env::temp_dir().join("sketchframe-pic-test.json");
        p.save(&path).unwrap();
        let back = crate::model::Project::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.images.len(), 1, "the unused SVG is left out of the file");
        assert!(!back.images.contains_key(&unused));
        let c = back.screens[0].controls.iter().find(|c| c.kind == "picture").unwrap();
        let img = &back.images[c.image.as_deref().unwrap()];
        assert_eq!((img.w, img.h), (60.0, 40.0));
        assert_eq!(c.name.as_deref(), Some("dot.png"));
    }

    #[test]
    fn other_formats_are_converted_and_junk_is_refused() {
        let (w, h, px) = white_photo();
        let buf = image::RgbaImage::from_raw(w as u32, h as u32, px).unwrap();
        let mut bmp = Cursor::new(vec![]);
        image::DynamicImage::ImageRgba8(buf).write_to(&mut bmp, image::ImageFormat::Bmp).unwrap();
        let img = Img::from_bytes("x.bmp", bmp.into_inner()).unwrap();
        assert_eq!(img.mime, "image/png", "BMP is stored as PNG so browsers can show it");
        assert!(Img::from_bytes("notes.txt", b"hello".to_vec()).is_err());
    }

    #[test]
    fn fit_modes() {
        let r = R::new(0.0, 0.0, 200.0, 100.0);
        assert_eq!(fit_rect(r, 100.0, 100.0, Fit::Contain).0, R::new(50.0, 0.0, 100.0, 100.0));
        let (dst, uv) = fit_rect(r, 100.0, 100.0, Fit::Cover);
        assert_eq!(dst, r);
        assert!((uv.height() - 0.5).abs() < 1e-5 && (uv.width() - 1.0).abs() < 1e-5);
    }
}
