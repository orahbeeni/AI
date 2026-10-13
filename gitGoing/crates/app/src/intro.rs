//! The start-up intro animation.
//!
//! The video is stored as a bundle of JPEG frames (see `scripts/make-intro.sh`)
//! and decoded one frame at a time as it plays, so memory use stays at a single
//! frame no matter how long the animation is.
//!
//! File layout (little endian): `GGIV`, then `u16` version, `u16` fps,
//! `u16` width, `u16` height, `u32` frame count, one `u32` length per frame, and
//! finally the frames back to back.

use eframe::egui::ColorImage;

const BUNDLE: &[u8] = include_bytes!("../assets/intro.bin");

pub struct Intro {
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    frames: Vec<&'static [u8]>,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

impl Intro {
    /// The embedded intro, or `None` if the bundle is damaged.
    pub fn embedded() -> Option<Intro> {
        Intro::parse(BUNDLE)
    }

    pub fn parse(data: &'static [u8]) -> Option<Intro> {
        if data.get(..4)? != b"GGIV" || u16_at(data, 4)? != 1 {
            return None;
        }
        let fps = u16_at(data, 6)?;
        let (width, height) = (u16_at(data, 8)? as u32, u16_at(data, 10)? as u32);
        let count = u32_at(data, 12)? as usize;
        if fps == 0 || width == 0 || height == 0 || count == 0 {
            return None;
        }
        let mut offset = 16 + count * 4;
        let mut frames = Vec::with_capacity(count);
        for k in 0..count {
            let len = u32_at(data, 16 + k * 4)? as usize;
            frames.push(data.get(offset..offset.checked_add(len)?)?);
            offset += len;
        }
        Some(Intro { fps: fps as f64, width, height, frames })
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// How long the animation runs, in seconds.
    pub fn duration(&self) -> f64 {
        self.frame_count() as f64 / self.fps
    }

    /// The frame to show `t` seconds in (the last one once it has finished).
    pub fn frame_at(&self, t: f64) -> usize {
        ((t.max(0.0) * self.fps) as usize).min(self.frame_count() - 1)
    }

    pub fn decode(&self, index: usize) -> Option<ColorImage> {
        let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(*self.frames.get(index)?));
        let pixels = decoder.decode().ok()?;
        let info = decoder.info()?;
        let size = [info.width as usize, info.height as usize];
        match info.pixel_format {
            jpeg_decoder::PixelFormat::RGB24 => Some(ColorImage::from_rgb(size, &pixels)),
            jpeg_decoder::PixelFormat::L8 => Some(ColorImage::from_gray(size, &pixels)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_intro_parses_and_its_frames_decode() {
        let intro = Intro::embedded().expect("embedded intro is valid");
        assert_eq!(intro.frame_count(), 240);
        assert_eq!((intro.width, intro.height), (1152, 648));
        assert!((intro.duration() - 10.0).abs() < 0.5, "about ten seconds: {}", intro.duration());
        for i in [0, intro.frame_count() / 2, intro.frame_count() - 1] {
            let img = intro.decode(i).unwrap_or_else(|| panic!("frame {i} decodes"));
            assert_eq!(img.size, [1152, 648]);
        }
    }

    /// Playback decodes a frame per tick on the UI thread, so each must be quick:
    /// `cargo test -p gitgoing --release intro -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn decoding_a_frame_is_fast() {
        let intro = Intro::embedded().unwrap();
        let start = std::time::Instant::now();
        for i in 0..intro.frame_count() {
            intro.decode(i).unwrap();
        }
        let per_frame = start.elapsed() / intro.frame_count() as u32;
        eprintln!("decoded {} frames, {per_frame:?} per frame", intro.frame_count());
        assert!(per_frame.as_millis() < 20, "a frame must decode well within one 41 ms tick");
    }

    #[test]
    fn frame_at_follows_time_and_clamps() {
        let intro = Intro::embedded().unwrap();
        assert_eq!(intro.frame_at(-1.0), 0);
        assert_eq!(intro.frame_at(0.0), 0);
        assert_eq!(intro.frame_at(1.0), intro.fps as usize);
        assert_eq!(intro.frame_at(1e6), intro.frame_count() - 1);
    }

    #[test]
    fn damaged_bundles_are_rejected() {
        assert!(Intro::parse(b"").is_none());
        assert!(Intro::parse(b"NOPE0000000000000000").is_none());
        // Claims 2 frames but has no data for them.
        let mut bad = b"GGIV".to_vec();
        bad.extend([1, 0, 20, 0, 4, 0, 4, 0, 2, 0, 0, 0]);
        bad.extend([9, 0, 0, 0, 9, 0, 0, 0]);
        let leaked: &'static [u8] = Box::leak(bad.into_boxed_slice());
        assert!(Intro::parse(leaked).is_none());
    }
}
