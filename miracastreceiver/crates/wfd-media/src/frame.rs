//! Decoded video frames handed from the GStreamer streaming thread to the GUI.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use gstreamer as gst;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMatrix {
    Bt601,
    Bt709,
}

/// One decoded I420 frame. Holds the GStreamer buffer (no copy); the GUI maps
/// it with [`DecodedFrame::with_planes`] when uploading to the GPU.
pub struct DecodedFrame {
    pub index: u64,
    pub width: u32,
    pub height: u32,
    pub matrix: ColorMatrix,
    pub full_range: bool,
    /// When the frame left the decoder.
    pub decoded_at: Instant,
    buffer: gst::Buffer,
    info: gst_video::VideoInfo,
}

/// A borrowed plane: bytes, stride, width, height.
pub struct Plane<'a> {
    pub data: &'a [u8],
    pub stride: usize,
    pub width: u32,
    pub height: u32,
}

impl DecodedFrame {
    pub(crate) fn new(index: u64, buffer: gst::Buffer, info: gst_video::VideoInfo) -> Self {
        let colorimetry = info.colorimetry();
        let matrix = match colorimetry.matrix() {
            gst_video::VideoColorMatrix::Bt601 => ColorMatrix::Bt601,
            gst_video::VideoColorMatrix::Bt709 => ColorMatrix::Bt709,
            // HD streams without colour info are BT.709 in practice
            _ if info.height() >= 720 => ColorMatrix::Bt709,
            _ => ColorMatrix::Bt601,
        };
        let full_range = colorimetry.range() == gst_video::VideoColorRange::Range0_255;
        Self { index, width: info.width(), height: info.height(), matrix, full_range, decoded_at: Instant::now(), buffer, info }
    }

    /// Calls `f` with the Y, U and V planes. Returns `None` if mapping fails.
    pub fn with_planes<R>(&self, f: impl FnOnce([Plane<'_>; 3]) -> R) -> Option<R> {
        let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(self.buffer.as_ref(), &self.info).ok()?;
        let strides = frame.plane_stride();
        let (w, h) = (self.width, self.height);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let y = frame.plane_data(0).ok()?;
        let u = frame.plane_data(1).ok()?;
        let v = frame.plane_data(2).ok()?;
        Some(f([
            Plane { data: y, stride: strides[0] as usize, width: w, height: h },
            Plane { data: u, stride: strides[1] as usize, width: cw, height: ch },
            Plane { data: v, stride: strides[2] as usize, width: cw, height: ch },
        ]))
    }

    /// Converts to packed RGBA (CPU fallback renderer, screenshots, tests).
    /// Fixed-point BT.601/BT.709 with limited or full range.
    pub fn to_rgba(&self) -> Option<Vec<u8>> {
        // (y scale, v->r, u->g, v->g, u->b) in 1/256 units
        let (ys, vr, ug, vg, ub, yoff) = match (self.matrix, self.full_range) {
            (ColorMatrix::Bt709, false) => (298, 459, 55, 136, 541, 16),
            (ColorMatrix::Bt601, false) => (298, 409, 100, 208, 516, 16),
            (ColorMatrix::Bt709, true) => (256, 403, 48, 120, 475, 0),
            (ColorMatrix::Bt601, true) => (256, 359, 88, 183, 454, 0),
        };
        let (w, h) = (self.width as usize, self.height as usize);
        self.with_planes(|[y, u, v]| {
            let mut out = vec![255u8; w * h * 4];
            for row in 0..h {
                let yr = &y.data[row * y.stride..row * y.stride + w];
                let ur = &u.data[(row / 2) * u.stride..];
                let vr_ = &v.data[(row / 2) * v.stride..];
                let o = &mut out[row * w * 4..(row + 1) * w * 4];
                for col in 0..w {
                    let c = (yr[col] as i32 - yoff) * ys + 128;
                    let d = ur[col / 2] as i32 - 128;
                    let e = vr_[col / 2] as i32 - 128;
                    let px = &mut o[col * 4..col * 4 + 3];
                    px[0] = ((c + vr * e) >> 8).clamp(0, 255) as u8;
                    px[1] = ((c - ug * d - vg * e) >> 8).clamp(0, 255) as u8;
                    px[2] = ((c + ub * d) >> 8).clamp(0, 255) as u8;
                }
            }
            out
        })
    }
}

/// Latest-frame mailbox: the decoder overwrites, the renderer takes. Older
/// frames are dropped rather than queued, which keeps display latency at one
/// frame regardless of render rate.
#[derive(Default, Clone)]
pub struct FrameSlot {
    inner: Arc<Mutex<Option<Arc<DecodedFrame>>>>,
}

impl FrameSlot {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn put(&self, f: DecodedFrame) {
        *self.inner.lock().unwrap() = Some(Arc::new(f));
    }

    /// Takes the newest frame if one arrived since the last call.
    pub fn take(&self) -> Option<Arc<DecodedFrame>> {
        self.inner.lock().unwrap().take()
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().take();
    }
}
