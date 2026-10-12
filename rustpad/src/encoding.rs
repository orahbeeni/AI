//! Text encodings: detection on load, conversion on save.

use encoding_rs::Encoding;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Enc {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// Any single / multi byte legacy charset supported by `encoding_rs`.
    Legacy(&'static Encoding),
}

impl Enc {
    pub fn ansi() -> Enc {
        Enc::Legacy(encoding_rs::WINDOWS_1252)
    }

    pub fn label(&self) -> String {
        match self {
            Enc::Utf8 => "UTF-8".into(),
            Enc::Utf8Bom => "UTF-8-BOM".into(),
            Enc::Utf16Le => "UTF-16 LE BOM".into(),
            Enc::Utf16Be => "UTF-16 BE BOM".into(),
            Enc::Legacy(e) if *e == encoding_rs::WINDOWS_1252 => "ANSI".into(),
            Enc::Legacy(e) => e.name().to_string(),
        }
    }

    pub fn from_label(label: &str) -> Option<Enc> {
        match label {
            "UTF-8" => Some(Enc::Utf8),
            "UTF-8-BOM" => Some(Enc::Utf8Bom),
            "UTF-16 LE BOM" => Some(Enc::Utf16Le),
            "UTF-16 BE BOM" => Some(Enc::Utf16Be),
            "ANSI" => Some(Enc::ansi()),
            other => Encoding::for_label(other.as_bytes()).map(Enc::Legacy),
        }
    }
}

/// Menu layout: (group name, entries).
pub fn encoding_menu() -> Vec<(&'static str, Vec<Enc>)> {
    use encoding_rs as e;
    vec![
        (
            "Western European",
            vec![Enc::Legacy(e::WINDOWS_1252), Enc::Legacy(e::ISO_8859_15)],
        ),
        ("Central European", vec![Enc::Legacy(e::WINDOWS_1250), Enc::Legacy(e::ISO_8859_2)]),
        (
            "Cyrillic",
            vec![Enc::Legacy(e::WINDOWS_1251), Enc::Legacy(e::KOI8_R), Enc::Legacy(e::ISO_8859_5), Enc::Legacy(e::IBM866)],
        ),
        ("Greek", vec![Enc::Legacy(e::WINDOWS_1253), Enc::Legacy(e::ISO_8859_7)]),
        ("Turkish", vec![Enc::Legacy(e::WINDOWS_1254)]),
        ("Hebrew", vec![Enc::Legacy(e::WINDOWS_1255), Enc::Legacy(e::ISO_8859_8)]),
        ("Arabic", vec![Enc::Legacy(e::WINDOWS_1256), Enc::Legacy(e::ISO_8859_6)]),
        ("Baltic", vec![Enc::Legacy(e::WINDOWS_1257), Enc::Legacy(e::ISO_8859_4)]),
        ("Vietnamese", vec![Enc::Legacy(e::WINDOWS_1258)]),
        ("Thai", vec![Enc::Legacy(e::WINDOWS_874)]),
        (
            "Japanese",
            vec![Enc::Legacy(e::SHIFT_JIS), Enc::Legacy(e::EUC_JP), Enc::Legacy(e::ISO_2022_JP)],
        ),
        ("Korean", vec![Enc::Legacy(e::EUC_KR)]),
        ("Chinese", vec![Enc::Legacy(e::GBK), Enc::Legacy(e::GB18030), Enc::Legacy(e::BIG5)]),
    ]
}

pub struct Decoded {
    pub text: String,
    pub enc: Enc,
}

/// Decode raw file bytes, guessing the encoding.
pub fn decode_auto(bytes: &[u8]) -> Decoded {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Decoded { text: String::from_utf8_lossy(rest).into_owned(), enc: Enc::Utf8Bom };
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Decoded { text: decode_utf16(rest, false), enc: Enc::Utf16Le };
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return Decoded { text: decode_utf16(rest, true), enc: Enc::Utf16Be };
    }
    // BOM-less UTF-16 heuristic: lots of NULs on one parity (valid UTF-8, so check first).
    if let Some(be) = sniff_utf16(bytes) {
        let enc = if be { Enc::Utf16Be } else { Enc::Utf16Le };
        return Decoded { text: decode_utf16(bytes, be), enc };
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Decoded { text: s.to_owned(), enc: Enc::Utf8 };
    }
    let mut det = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Allow);
    det.feed(bytes, true);
    let guess = det.guess(None, chardetng::Utf8Detection::Deny);
    let (cow, _) = guess.decode_without_bom_handling(bytes);
    Decoded { text: cow.into_owned(), enc: Enc::Legacy(guess) }
}

/// Decode with a user-chosen encoding ("Reload as ...").
pub fn decode_with(bytes: &[u8], enc: Enc) -> String {
    match enc {
        Enc::Utf8 => String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)).into_owned(),
        Enc::Utf8Bom => String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)).into_owned(),
        Enc::Utf16Le => decode_utf16(bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes), false),
        Enc::Utf16Be => decode_utf16(bytes.strip_prefix(&[0xFE, 0xFF]).unwrap_or(bytes), true),
        Enc::Legacy(e) => e.decode_without_bom_handling(bytes).0.into_owned(),
    }
}

fn sniff_utf16(bytes: &[u8]) -> Option<bool> {
    let n = bytes.len().min(4096) & !1;
    if n < 4 {
        return None;
    }
    let (mut even0, mut odd0) = (0usize, 0usize);
    for (i, b) in bytes[..n].iter().enumerate() {
        if *b == 0 {
            if i % 2 == 0 {
                even0 += 1;
            } else {
                odd0 += 1;
            }
        }
    }
    let half = n / 2;
    if odd0 * 10 > half * 3 && even0 * 20 < half {
        Some(false) // ASCII-ish text in LE: high byte (odd) is zero
    } else if even0 * 10 > half * 3 && odd0 * 20 < half {
        Some(true)
    } else {
        None
    }
}

fn decode_utf16(bytes: &[u8], big_endian: bool) -> String {
    let units = bytes.chunks_exact(2).map(|c| {
        if big_endian { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }
    });
    char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect()
}

/// Encode text for writing to disk (BOM included where applicable).
pub fn encode(text: &str, enc: Enc) -> Vec<u8> {
    match enc {
        Enc::Utf8 => text.as_bytes().to_vec(),
        Enc::Utf8Bom => {
            let mut v = vec![0xEF, 0xBB, 0xBF];
            v.extend_from_slice(text.as_bytes());
            v
        }
        Enc::Utf16Le => {
            let mut v = vec![0xFF, 0xFE];
            for u in text.encode_utf16() {
                v.extend_from_slice(&u.to_le_bytes());
            }
            v
        }
        Enc::Utf16Be => {
            let mut v = vec![0xFE, 0xFF];
            for u in text.encode_utf16() {
                v.extend_from_slice(&u.to_be_bytes());
            }
            v
        }
        Enc::Legacy(e) => e.encode(text).0.into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_utf16() {
        for enc in [Enc::Utf16Le, Enc::Utf16Be, Enc::Utf8Bom, Enc::Utf8] {
            let bytes = encode("héllo\nwörld ✓", enc);
            let d = decode_auto(&bytes);
            assert_eq!(d.text, "héllo\nwörld ✓");
            assert_eq!(d.enc, enc);
        }
    }

    #[test]
    fn legacy_detect() {
        let bytes = encode("caf\u{e9} cr\u{e8}me", Enc::ansi());
        let d = decode_auto(&bytes);
        assert_eq!(d.text, "café crème");
        assert!(matches!(d.enc, Enc::Legacy(_)));
    }

    #[test]
    fn bomless_utf16() {
        let mut v = Vec::new();
        for u in "hello world, this is text".encode_utf16() {
            v.extend_from_slice(&u.to_le_bytes());
        }
        let d = decode_auto(&v);
        assert_eq!(d.text, "hello world, this is text");
        assert_eq!(d.enc, Enc::Utf16Le);
    }
}
