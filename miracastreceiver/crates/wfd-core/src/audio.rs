//! `wfd_audio_codecs: LPCM 00000003 00, AAC 00000001 00`

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioFormat {
    Lpcm,
    Aac,
    Ac3,
}

impl AudioFormat {
    pub fn name(self) -> &'static str {
        match self {
            AudioFormat::Lpcm => "LPCM",
            AudioFormat::Aac => "AAC",
            AudioFormat::Ac3 => "AC3",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "LPCM" => Some(Self::Lpcm),
            "AAC" => Some(Self::Aac),
            "AC3" => Some(Self::Ac3),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioCodec {
    pub format: AudioFormat,
    /// Bitmap of supported modes (see [`AudioCodec::modes`]).
    pub modes: u32,
    pub latency: u8,
}

/// One concrete audio mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioMode {
    pub format: AudioFormat,
    pub sample_rate: u32,
    pub bits: u32,
    pub channels: u32,
}

impl fmt::Display for AudioMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} Hz {}-bit {}ch", self.format.name(), self.sample_rate, self.bits, self.channels)
    }
}

impl AudioCodec {
    /// Modes for each bit of the bitmap.
    pub fn mode_for_bit(format: AudioFormat, bit: u32) -> Option<AudioMode> {
        let (sample_rate, channels) = match (format, bit) {
            (AudioFormat::Lpcm, 0) => (44_100, 2),
            (AudioFormat::Lpcm, 1) => (48_000, 2),
            (AudioFormat::Aac, 0) | (AudioFormat::Ac3, 0) => (48_000, 2),
            (AudioFormat::Aac, 1) | (AudioFormat::Ac3, 1) => (48_000, 4),
            (AudioFormat::Aac, 2) | (AudioFormat::Ac3, 2) => (48_000, 6),
            (AudioFormat::Aac, 3) => (48_000, 8),
            _ => return None,
        };
        Some(AudioMode { format, sample_rate, bits: 16, channels })
    }

    /// The lowest mode bit set (sources set exactly one in M4).
    pub fn selected_mode(&self) -> Option<AudioMode> {
        if self.modes == 0 {
            return None;
        }
        Self::mode_for_bit(self.format, self.modes.trailing_zeros())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AudioCodecs(pub Vec<AudioCodec>);

impl AudioCodecs {
    /// LPCM 44.1/48 kHz stereo (mandatory) + AAC 48 kHz stereo.
    pub fn sink_capabilities() -> Self {
        Self(vec![
            AudioCodec { format: AudioFormat::Lpcm, modes: 0b11, latency: 0 },
            AudioCodec { format: AudioFormat::Aac, modes: 0b1, latency: 0 },
        ])
    }

    pub fn none() -> Self {
        Self(Vec::new())
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("none") {
            return Some(Self::none());
        }
        let mut out = Vec::new();
        for entry in s.split(',') {
            let f: Vec<&str> = entry.split_whitespace().collect();
            if f.is_empty() {
                continue;
            }
            if f.len() < 3 {
                return None;
            }
            out.push(AudioCodec {
                format: AudioFormat::parse(f[0])?,
                modes: u32::from_str_radix(f[1], 16).ok()?,
                latency: u8::from_str_radix(f[2], 16).ok()?,
            });
        }
        Some(Self(out))
    }

    pub fn selected_mode(&self) -> Option<AudioMode> {
        self.0.first().and_then(AudioCodec::selected_mode)
    }

    pub fn supports(&self, format: AudioFormat) -> Option<&AudioCodec> {
        self.0.iter().find(|c| c.format == format && c.modes != 0)
    }
}

impl fmt::Display for AudioCodecs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str("none");
        }
        for (i, c) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{} {:08x} {:02x}", c.format.name(), c.modes, c.latency)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_select() {
        let caps = AudioCodecs::sink_capabilities();
        assert_eq!(caps.to_string(), "LPCM 00000003 00, AAC 00000001 00");
        assert_eq!(AudioCodecs::parse(&caps.to_string()), Some(caps));

        let m4 = AudioCodecs::parse("AAC 00000001 00").unwrap();
        assert_eq!(m4.selected_mode().unwrap().to_string(), "AAC 48000 Hz 16-bit 2ch");
        let m4 = AudioCodecs::parse("LPCM 00000002 00").unwrap();
        assert_eq!(m4.selected_mode().unwrap().sample_rate, 48_000);
        assert_eq!(AudioCodecs::parse("none"), Some(AudioCodecs::none()));
        assert_eq!(AudioCodecs::none().to_string(), "none");
        assert!(AudioCodecs::parse("MP3 1 0").is_none());
    }
}
