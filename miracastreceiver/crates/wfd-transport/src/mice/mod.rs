//! [MS-MICE] Miracast over Infrastructure, sink side.
//!
//! Discovery: the sink puts a WSC Vendor Extension (OUI 00:01:37) with a
//! Capability, Host Name and IP Address attributes into its Wi-Fi Direct
//! probe responses (done by the Wi-Fi Direct transport, see
//! [`vendor_extension`]) and registers `<name>._display._tcp.local` on port
//! 7250 with TXT `container_id={GUID}`.
//!
//! Projection: the source connects to TCP 7250 and sends SOURCE_READY with
//! its RTSP port; the sink connects back to `source_ip:rtsp_port` and runs
//! normal Wi-Fi Display RTSP. STOP_PROJECTION (either way) ends it.
//!
//! Message: `[size:2 BE, whole message][version:1 = 1][command:1][TLVs]`,
//! TLV: `[type:1][length:2 BE][value]`. Strings are UTF-16LE.
//!
//! Stream encryption (DTLS) and PIN entry are not implemented, so the
//! capability attribute advertises neither and a source that tries them
//! is disconnected, as the spec requires for unexpected messages.

use std::net::IpAddr;

use sha2::{Digest, Sha256};
use thiserror::Error;

#[cfg(feature = "ms-mice")]
mod mdns;
#[cfg(feature = "ms-mice")]
mod server;

#[cfg(feature = "ms-mice")]
pub use mdns::{MdnsRegistration, register_display_service};
#[cfg(feature = "ms-mice")]
pub use server::MiceTransport;

pub const MICE_PORT: u16 = 7250;
pub const VERSION: u8 = 0x01;
pub const SERVICE_TYPE: &str = "_display._tcp";

pub mod command {
    pub const SOURCE_READY: u8 = 0x01;
    pub const STOP_PROJECTION: u8 = 0x02;
    pub const SECURITY_HANDSHAKE: u8 = 0x03;
    pub const SESSION_REQUEST: u8 = 0x04;
    pub const PIN_CHALLENGE: u8 = 0x05;
    pub const PIN_RESPONSE: u8 = 0x06;
}

pub mod tlv {
    pub const FRIENDLY_NAME: u8 = 0x00;
    pub const RTSP_PORT: u8 = 0x02;
    pub const SOURCE_ID: u8 = 0x03;
    pub const SECURITY_TOKEN: u8 = 0x04;
    pub const SECURITY_OPTIONS: u8 = 0x05;
    pub const PIN_CHALLENGE: u8 = 0x06;
    pub const PIN_RESPONSE_REASON: u8 = 0x07;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MiceError {
    #[error("message shorter than its header")]
    Short,
    #[error("message size field {0} is invalid")]
    BadSize(usize),
    #[error("unsupported MS-MICE version {0}")]
    BadVersion(u8),
    #[error("TLV overruns the message at offset {0}")]
    BadTlv(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tlv {
    pub kind: u8,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MiceMessage {
    pub command: u8,
    pub tlvs: Vec<Tlv>,
}

impl MiceMessage {
    pub fn new(command: u8) -> Self {
        Self { command, tlvs: Vec::new() }
    }

    pub fn with(mut self, kind: u8, value: impl Into<Vec<u8>>) -> Self {
        self.tlvs.push(Tlv { kind, value: value.into() });
        self
    }

    pub fn encode(&self) -> Vec<u8> {
        let size = 4 + self.tlvs.iter().map(|t| 3 + t.value.len()).sum::<usize>();
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&(size as u16).to_be_bytes());
        out.push(VERSION);
        out.push(self.command);
        for t in &self.tlvs {
            out.push(t.kind);
            out.extend_from_slice(&(t.value.len() as u16).to_be_bytes());
            out.extend_from_slice(&t.value);
        }
        out
    }

    /// Size of the next message in `buf`, if the header is there.
    pub fn frame_len(buf: &[u8]) -> Option<usize> {
        (buf.len() >= 2).then(|| u16::from_be_bytes([buf[0], buf[1]]) as usize)
    }

    pub fn decode(buf: &[u8]) -> Result<Self, MiceError> {
        if buf.len() < 4 {
            return Err(MiceError::Short);
        }
        let size = u16::from_be_bytes([buf[0], buf[1]]) as usize;
        if size < 4 || size > buf.len() {
            return Err(MiceError::BadSize(size));
        }
        if buf[2] != VERSION {
            return Err(MiceError::BadVersion(buf[2]));
        }
        let mut tlvs = Vec::new();
        let mut i = 4;
        while i < size {
            if i + 3 > size {
                return Err(MiceError::BadTlv(i));
            }
            let len = u16::from_be_bytes([buf[i + 1], buf[i + 2]]) as usize;
            if i + 3 + len > size {
                return Err(MiceError::BadTlv(i));
            }
            tlvs.push(Tlv { kind: buf[i], value: buf[i + 3..i + 3 + len].to_vec() });
            i += 3 + len;
        }
        Ok(Self { command: buf[3], tlvs })
    }

    pub fn get(&self, kind: u8) -> Option<&[u8]> {
        self.tlvs.iter().find(|t| t.kind == kind).map(|t| t.value.as_slice())
    }

    pub fn friendly_name(&self) -> Option<String> {
        self.get(tlv::FRIENDLY_NAME).map(utf16le_decode)
    }

    pub fn rtsp_port(&self) -> Option<u16> {
        self.get(tlv::RTSP_PORT).filter(|v| v.len() == 2).map(|v| u16::from_be_bytes([v[0], v[1]]))
    }

    pub fn source_id(&self) -> Option<&[u8]> {
        self.get(tlv::SOURCE_ID)
    }

    /// STOP_PROJECTION sent by the sink (graceful teardown).
    pub fn stop_projection(friendly_name: &str, source_id: &[u8]) -> Self {
        Self::new(command::STOP_PROJECTION).with(tlv::FRIENDLY_NAME, utf16le_encode(friendly_name)).with(tlv::SOURCE_ID, source_id.to_vec())
    }
}

pub fn utf16le_encode(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

pub fn utf16le_decode(b: &[u8]) -> String {
    let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units).trim_end_matches('\0').to_string()
}

/// Capability attribute bits (MS-MICE 2.2.8.1).
pub mod capability {
    pub const MICE_SUPPORTED: u8 = 0x01;
    pub const STREAM_ENCRYPTION: u8 = 0x02;
    pub const VERSION_1: u8 = 0x01 << 2;
    pub const PIN_SUPPORTED: u8 = 0x20;
}

/// Payload of the WSC Vendor Extension attribute (after the 0x1049 type and
/// length, which wpa_supplicant adds): OUI 00:01:37 + MS-MICE attributes.
pub fn vendor_extension(host_name: &str, ips: &[IpAddr]) -> Vec<u8> {
    let mut v = vec![0x00, 0x01, 0x37];
    let mut attr = |id: u16, data: &[u8]| {
        v.extend_from_slice(&id.to_be_bytes());
        v.extend_from_slice(&(data.len() as u16).to_be_bytes());
        v.extend_from_slice(data);
    };
    attr(0x2001, &[capability::MICE_SUPPORTED | capability::VERSION_1]);
    attr(0x2002, host_name.as_bytes());
    for ip in ips {
        attr(0x2005, ip.to_string().as_bytes());
    }
    v
}

/// The complete WSC attribute, with the 0x1049 header.
pub fn vendor_extension_attribute(host_name: &str, ips: &[IpAddr]) -> Vec<u8> {
    let body = vendor_extension(host_name, ips);
    let mut v = vec![0x10, 0x49];
    v.extend_from_slice(&(body.len() as u16).to_be_bytes());
    v.extend(body);
    v
}

/// SHA-256(ASCII PIN || sender IP bytes) (MS-MICE 3.1.5.6.1).
pub fn pin_hash(pin: &str, sender_ip: IpAddr) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(pin.as_bytes());
    match sender_ip {
        IpAddr::V4(v4) => h.update(v4.octets()),
        IpAddr::V6(v6) => h.update(v6.octets()),
    }
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    const SOURCE_ID: &str = "91 F4 AB E9 EF F5 46 4A AE E2 69 72 2A ED 11 B5";
    const NAME_UTF16: &str = "44 00 75 00 6D 00 6D 00 79 00 31 00 2D 00 4B 00 61 00 62 00 79 00 6C 00 61 00 6B 00 65 00";

    #[test]
    fn spec_example_source_ready() {
        // [MS-MICE] 4.2
        let bytes = hex(&format!("00 3D 01 01 00 00 1E {NAME_UTF16} 02 00 02 1C 44 03 00 10 {SOURCE_ID}"));
        let m = MiceMessage::decode(&bytes).unwrap();
        assert_eq!(m.command, command::SOURCE_READY);
        assert_eq!(m.friendly_name().as_deref(), Some("Dummy1-Kabylake"));
        assert_eq!(m.rtsp_port(), Some(7236));
        assert_eq!(m.source_id(), Some(hex(SOURCE_ID).as_slice()));
        assert_eq!(m.encode(), bytes);
        assert_eq!(MiceMessage::frame_len(&bytes), Some(61));
    }

    #[test]
    fn spec_example_stop_projection() {
        // [MS-MICE] 4.3
        let bytes = hex(&format!("00 38 01 02 00 00 1E {NAME_UTF16} 03 00 10 {SOURCE_ID}"));
        assert_eq!(MiceMessage::stop_projection("Dummy1-Kabylake", &hex(SOURCE_ID)).encode(), bytes);
        assert_eq!(MiceMessage::decode(&bytes).unwrap().command, command::STOP_PROJECTION);
    }

    #[test]
    fn spec_example_session_request_and_pin_challenge() {
        // [MS-MICE] 4.5 and 4.6. The 4.5 example prints Size 0x3A (58), but
        // its own bytes add up to 60 (4 + 4 + 33 + 19), so it is a typo in
        // the spec; the 4.6 example (58) is consistent.
        let sr_bytes = hex(&format!("00 3C 01 04 05 00 01 03 00 00 1E {NAME_UTF16} 03 00 10 {SOURCE_ID}"));
        assert_eq!(sr_bytes.len(), 60);
        let sr = MiceMessage::decode(&sr_bytes).unwrap();
        assert_eq!(sr.command, command::SESSION_REQUEST);
        assert_eq!(sr.get(tlv::SECURITY_OPTIONS), Some(&[0x03][..]));
        let pc = MiceMessage::decode(&hex(&format!(
            "00 3A 01 05 06 00 20 60 54 09 F8 32 30 8A D0 B8 93 A7 F9 1B E4 2B 26 4C 73 72 B3 6E 90 77 50 6E 1B 4C C1 83 DE 79 DA 03 00 10 {SOURCE_ID}"
        )))
        .unwrap();
        assert_eq!(pc.get(tlv::PIN_CHALLENGE).unwrap(), pin_hash("12345678", "192.0.2.100".parse().unwrap()));
    }

    #[test]
    fn spec_pin_hashes() {
        // [MS-MICE] 3.1.5.6.1
        assert_eq!(
            pin_hash("12345678", "192.0.2.100".parse().unwrap()).to_vec(),
            hex("60 54 09 f8 32 30 8a d0 b8 93 a7 f9 1b e4 2b 26 4c 73 72 b3 6e 90 77 50 6e 1b 4c c1 83 de 79 da")
        );
        assert_eq!(
            pin_hash("98765432", "2001:db8:1f::4242".parse().unwrap()).to_vec(),
            hex("b3 45 2b 2c 46 c8 3d 28 d8 d4 64 b6 69 7a 81 d1 af 3f 35 61 07 e1 d0 73 1e a9 bb 18 38 03 f9 c7")
        );
    }

    #[test]
    fn spec_example_vendor_extension() {
        // [MS-MICE] 4.1
        let expected = hex("10 49 00 1B 00 01 37 20 01 00 01 05 20 02 00 0F 44 75 6D 6D 79 31 2D 4B 61 62 79 6C 61 6B 65");
        assert_eq!(vendor_extension_attribute("Dummy1-Kabylake", &[]), expected);
        // with an IP address attribute (ASCII dotted decimal)
        let v = vendor_extension("pc", &["192.168.1.20".parse().unwrap()]);
        assert_eq!(&v[v.len() - 16..], &hex("20 05 00 0C 31 39 32 2E 31 36 38 2E 31 2E 32 30")[..]);
    }

    #[test]
    fn decode_errors() {
        assert_eq!(MiceMessage::decode(&[0, 3, 1]), Err(MiceError::Short));
        assert_eq!(MiceMessage::decode(&[0, 9, 1, 1]), Err(MiceError::BadSize(9)));
        assert_eq!(MiceMessage::decode(&[0, 4, 2, 1]), Err(MiceError::BadVersion(2)));
        assert_eq!(MiceMessage::decode(&[0, 7, 1, 1, 0, 0, 5]), Err(MiceError::BadTlv(4)));
    }
}
