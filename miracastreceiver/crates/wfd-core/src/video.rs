//! `wfd_video_formats` (WFD Technical Specification 1.x, section 6.1.3).
//!
//! ```text
//! wfd_video_formats: <native> <preferred-display-mode> <H.264-codec>[, <H.264-codec>]*
//! H.264-codec = profile level CEA VESA HH latency min-slice-size slice-enc-params
//!               frame-rate-control max-hres max-vres
//! ```

use std::fmt;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
#[error("malformed wfd_video_formats: {0}")]
pub struct VideoFormatError(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolutionTable {
    Cea = 0,
    Vesa = 1,
    Hh = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoMode {
    pub table: ResolutionTable,
    pub index: u8,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub interlaced: bool,
}

impl fmt::Display for VideoMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}{}{}", self.width, self.height, if self.interlaced { "i" } else { "p" }, self.fps)
    }
}

/// (width, height, fps, interlaced) for each bit of the CEA bitmap.
const CEA: &[(u32, u32, u32, bool)] = &[
    (640, 480, 60, false),
    (720, 480, 60, false),
    (720, 480, 60, true),
    (720, 576, 50, false),
    (720, 576, 50, true),
    (1280, 720, 30, false),
    (1280, 720, 60, false),
    (1920, 1080, 30, false),
    (1920, 1080, 60, false),
    (1920, 1080, 60, true),
    (1280, 720, 25, false),
    (1280, 720, 50, false),
    (1920, 1080, 25, false),
    (1920, 1080, 50, false),
    (1920, 1080, 50, true),
    (1280, 720, 24, false),
    (1920, 1080, 24, false),
];

const VESA: &[(u32, u32, u32, bool)] = &[
    (800, 600, 30, false),
    (800, 600, 60, false),
    (1024, 768, 30, false),
    (1024, 768, 60, false),
    (1152, 864, 30, false),
    (1152, 864, 60, false),
    (1280, 768, 30, false),
    (1280, 768, 60, false),
    (1280, 800, 30, false),
    (1280, 800, 60, false),
    (1360, 768, 30, false),
    (1360, 768, 60, false),
    (1366, 768, 30, false),
    (1366, 768, 60, false),
    (1280, 1024, 30, false),
    (1280, 1024, 60, false),
    (1400, 1050, 30, false),
    (1400, 1050, 60, false),
    (1440, 900, 30, false),
    (1440, 900, 60, false),
    (1600, 900, 30, false),
    (1600, 900, 60, false),
    (1600, 1200, 30, false),
    (1600, 1200, 60, false),
    (1680, 1024, 30, false),
    (1680, 1024, 60, false),
    (1680, 1050, 30, false),
    (1680, 1050, 60, false),
    (1920, 1200, 30, false),
    (1920, 1200, 60, false),
];

const HH: &[(u32, u32, u32, bool)] = &[
    (800, 480, 30, false),
    (800, 480, 60, false),
    (854, 480, 30, false),
    (854, 480, 60, false),
    (864, 480, 30, false),
    (864, 480, 60, false),
    (640, 360, 30, false),
    (640, 360, 60, false),
    (960, 540, 30, false),
    (960, 540, 60, false),
    (848, 480, 30, false),
    (848, 480, 60, false),
];

impl ResolutionTable {
    fn entries(self) -> &'static [(u32, u32, u32, bool)] {
        match self {
            ResolutionTable::Cea => CEA,
            ResolutionTable::Vesa => VESA,
            ResolutionTable::Hh => HH,
        }
    }

    pub fn mode(self, index: u8) -> Option<VideoMode> {
        let &(width, height, fps, interlaced) = self.entries().get(index as usize)?;
        Some(VideoMode { table: self, index, width, height, fps, interlaced })
    }

    pub fn modes(self) -> impl Iterator<Item = VideoMode> {
        (0..self.entries().len() as u8).filter_map(move |i| self.mode(i))
    }

    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Cea),
            1 => Some(Self::Vesa),
            2 => Some(Self::Hh),
            _ => None,
        }
    }
}

pub mod profile {
    pub const CBP: u8 = 0x01;
    pub const CHP: u8 = 0x02;
}

pub mod level {
    pub const L3_1: u8 = 0x01;
    pub const L3_2: u8 = 0x02;
    pub const L4: u8 = 0x04;
    pub const L4_1: u8 = 0x08;
    pub const L4_2: u8 = 0x10;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H264Codec {
    pub profile: u8,
    pub level: u8,
    pub cea: u32,
    pub vesa: u32,
    pub hh: u32,
    pub latency: u8,
    pub min_slice_size: u16,
    pub slice_enc_params: u16,
    pub frame_rate_control: u8,
    pub max_hres: Option<u16>,
    pub max_vres: Option<u16>,
}

impl H264Codec {
    pub fn bitmap(&self, table: ResolutionTable) -> u32 {
        match table {
            ResolutionTable::Cea => self.cea,
            ResolutionTable::Vesa => self.vesa,
            ResolutionTable::Hh => self.hh,
        }
    }

    /// All modes advertised by this codec entry.
    pub fn modes(&self) -> Vec<VideoMode> {
        [ResolutionTable::Cea, ResolutionTable::Vesa, ResolutionTable::Hh]
            .into_iter()
            .flat_map(|t| t.modes().filter(move |m| self.bitmap(t) & (1 << m.index) != 0))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFormats {
    /// bits 2:0 table, bits 7:3 index into that table
    pub native: u8,
    pub preferred_display_mode: u8,
    pub codecs: Vec<H264Codec>,
}

/// Limits for the formats this sink advertises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SinkVideoLimits {
    pub max_width: u32,
    pub max_height: u32,
    pub max_fps: u32,
}

impl Default for SinkVideoLimits {
    fn default() -> Self {
        Self { max_width: 1920, max_height: 1080, max_fps: 60 }
    }
}

impl VideoFormats {
    /// The capability set a desktop sink advertises in M3: H.264 CBP+CHP,
    /// level 4.2, every progressive mode within `limits`, native = the largest
    /// CEA mode allowed (1080p60 by default).
    pub fn sink_capabilities(limits: SinkVideoLimits) -> Self {
        let fits = |m: &VideoMode| !m.interlaced && m.width <= limits.max_width && m.height <= limits.max_height && m.fps <= limits.max_fps;
        let bitmap = |t: ResolutionTable| t.modes().filter(fits).fold(0u32, |acc, m| acc | (1 << m.index));
        let native_mode = ResolutionTable::Cea
            .modes()
            .filter(fits)
            .max_by_key(|m| (m.width * m.height, m.fps))
            .unwrap_or(ResolutionTable::Cea.mode(0).unwrap());
        Self {
            native: (native_mode.index << 3) | ResolutionTable::Cea as u8,
            preferred_display_mode: 0,
            codecs: vec![H264Codec {
                profile: profile::CBP | profile::CHP,
                level: level::L4_2,
                cea: bitmap(ResolutionTable::Cea),
                vesa: bitmap(ResolutionTable::Vesa),
                hh: bitmap(ResolutionTable::Hh),
                latency: 0,
                min_slice_size: 0,
                slice_enc_params: 0,
                frame_rate_control: 0,
                max_hres: None,
                max_vres: None,
            }],
        }
    }

    /// A single-mode selection as a source sends it in M4.
    pub fn selection(mode: VideoMode, profile: u8, level: u8) -> Self {
        let bit = 1u32 << mode.index;
        Self {
            native: (mode.index << 3) | mode.table as u8,
            preferred_display_mode: 0,
            codecs: vec![H264Codec {
                profile,
                level,
                cea: if mode.table == ResolutionTable::Cea { bit } else { 0 },
                vesa: if mode.table == ResolutionTable::Vesa { bit } else { 0 },
                hh: if mode.table == ResolutionTable::Hh { bit } else { 0 },
                latency: 0,
                min_slice_size: 0,
                slice_enc_params: 0,
                frame_rate_control: 0,
                max_hres: None,
                max_vres: None,
            }],
        }
    }

    pub fn native_mode(&self) -> Option<VideoMode> {
        ResolutionTable::from_u8(self.native & 0x07)?.mode(self.native >> 3)
    }

    /// The mode a source selected in M4: the single bit set in the first
    /// codec entry (CEA, then VESA, then HH); falls back to the native field.
    pub fn selected_mode(&self) -> Option<VideoMode> {
        if let Some(c) = self.codecs.first() {
            for t in [ResolutionTable::Cea, ResolutionTable::Vesa, ResolutionTable::Hh] {
                let bm = c.bitmap(t);
                if bm != 0 {
                    return t.mode(bm.trailing_zeros() as u8);
                }
            }
        }
        self.native_mode()
    }

    pub fn parse(s: &str) -> Result<Self, VideoFormatError> {
        let err = |m: &str| VideoFormatError(format!("{m}: {s:?}"));
        let s = s.trim();
        if s.eq_ignore_ascii_case("none") {
            return Err(err("value is none"));
        }
        let mut head = s.split_whitespace();
        let native = hex_u8(head.next().ok_or_else(|| err("missing native"))?).ok_or_else(|| err("bad native"))?;
        let preferred_display_mode =
            hex_u8(head.next().ok_or_else(|| err("missing preferred mode"))?).ok_or_else(|| err("bad preferred mode"))?;
        // Remainder: codec entries separated by commas.
        let rest: Vec<&str> = s.split_whitespace().skip(2).collect();
        let rest = rest.join(" ");
        let mut codecs = Vec::new();
        for entry in rest.split(',') {
            let f: Vec<&str> = entry.split_whitespace().collect();
            if f.is_empty() {
                continue;
            }
            if f.len() < 11 {
                return Err(err("codec entry needs 11 fields"));
            }
            let opt16 = |v: &str| if v.eq_ignore_ascii_case("none") { Ok(None) } else { hex_u16(v).map(Some).ok_or(()) };
            codecs.push(H264Codec {
                profile: hex_u8(f[0]).ok_or_else(|| err("bad profile"))?,
                level: hex_u8(f[1]).ok_or_else(|| err("bad level"))?,
                cea: hex_u32(f[2]).ok_or_else(|| err("bad CEA bitmap"))?,
                vesa: hex_u32(f[3]).ok_or_else(|| err("bad VESA bitmap"))?,
                hh: hex_u32(f[4]).ok_or_else(|| err("bad HH bitmap"))?,
                latency: hex_u8(f[5]).ok_or_else(|| err("bad latency"))?,
                min_slice_size: hex_u16(f[6]).ok_or_else(|| err("bad min slice size"))?,
                slice_enc_params: hex_u16(f[7]).ok_or_else(|| err("bad slice enc params"))?,
                frame_rate_control: hex_u8(f[8]).ok_or_else(|| err("bad frame rate control"))?,
                max_hres: opt16(f[9]).map_err(|_| err("bad max-hres"))?,
                max_vres: opt16(f[10]).map_err(|_| err("bad max-vres"))?,
            });
        }
        if codecs.is_empty() {
            return Err(err("no codec entries"));
        }
        Ok(Self { native, preferred_display_mode, codecs })
    }
}

impl fmt::Display for VideoFormats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02x} {:02x} ", self.native, self.preferred_display_mode)?;
        for (i, c) in self.codecs.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            let opt = |v: Option<u16>| v.map_or("none".to_string(), |v| format!("{v:04x}"));
            write!(
                f,
                "{:02x} {:02x} {:08x} {:08x} {:08x} {:02x} {:04x} {:04x} {:02x} {} {}",
                c.profile,
                c.level,
                c.cea,
                c.vesa,
                c.hh,
                c.latency,
                c.min_slice_size,
                c.slice_enc_params,
                c.frame_rate_control,
                opt(c.max_hres),
                opt(c.max_vres)
            )?;
        }
        Ok(())
    }
}

fn hex_u8(s: &str) -> Option<u8> {
    u8::from_str_radix(s, 16).ok()
}
fn hex_u16(s: &str) -> Option<u16> {
    u16::from_str_radix(s, 16).ok()
}
fn hex_u32(s: &str) -> Option<u32> {
    u32::from_str_radix(s, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sink_caps_default() {
        let v = VideoFormats::sink_capabilities(SinkVideoLimits::default());
        // native = CEA index 8 (1080p60)
        assert_eq!(v.native, 0x40);
        assert_eq!(v.native_mode().unwrap().to_string(), "1920x1080p60");
        let c = &v.codecs[0];
        assert_eq!(c.cea, 0x0001_BDEB, "all progressive CEA modes up to 1080p60");
        assert_eq!(c.vesa & (1 << 29), 0, "1920x1200p60 exceeds 1080p60 limits");
        assert_eq!(c.vesa & (1 << 28), 0, "1920x1200p30 is taller than 1080");
        assert_eq!(c.hh, 0x0FFF);
        assert_eq!(v.to_string(), format!("40 00 03 10 0001bdeb {:08x} 00000fff 00 0000 0000 00 none none", c.vesa));
        assert_eq!(VideoFormats::parse(&v.to_string()).unwrap(), v);
    }

    #[test]
    fn limits_to_720p30() {
        let v = VideoFormats::sink_capabilities(SinkVideoLimits { max_width: 1280, max_height: 720, max_fps: 30 });
        assert_eq!(v.native_mode().unwrap().to_string(), "1280x720p30");
        assert!(v.codecs[0].modes().iter().all(|m| m.width <= 1280 && m.height <= 720 && m.fps <= 30));
    }

    #[test]
    fn parses_android_m4_selection() {
        // Typical M4 from an Android source choosing 1920x1080p30 (CEA bit 7)
        let v = VideoFormats::parse("00 00 02 02 00000080 00000000 00000000 00 0000 0000 00 none none").unwrap();
        assert_eq!(v.selected_mode().unwrap().to_string(), "1920x1080p30");
        let v = VideoFormats::parse("00 00 01 01 00000000 00000008 00000000 00 0000 0000 00 none none").unwrap();
        assert_eq!(v.selected_mode().unwrap().to_string(), "1024x768p60");
    }

    #[test]
    fn parses_multi_entry_and_uppercase() {
        let s =
            "40 00 02 10 0001FFFF 1FFFFFFF 00000FFF 00 0000 0000 11 none none, 01 10 0001FFFF 1FFFFFFF 00000FFF 00 0000 0000 11 0780 0438";
        let v = VideoFormats::parse(s).unwrap();
        assert_eq!(v.codecs.len(), 2);
        assert_eq!(v.codecs[1].max_hres, Some(1920));
        assert_eq!(v.codecs[1].max_vres, Some(1080));
        assert!(VideoFormats::parse("none").is_err());
        assert!(VideoFormats::parse("00 00 02 02").is_err());
    }

    #[test]
    fn selection_roundtrip() {
        let m = ResolutionTable::Cea.mode(5).unwrap();
        let v = VideoFormats::selection(m, profile::CBP, level::L3_1);
        assert_eq!(v.to_string(), "28 00 01 01 00000020 00000000 00000000 00 0000 0000 00 none none");
        assert_eq!(v.selected_mode(), Some(m));
    }
}
