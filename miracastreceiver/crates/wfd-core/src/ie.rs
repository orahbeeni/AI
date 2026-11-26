//! Wi-Fi Display Information Element and its subelements.
//!
//! A WFD IE is an 802.11 vendor-specific element (`DD len 50 6F 9A 0A ...`)
//! carrying subelements `[id:1][len:2 BE][body]`. wpa_supplicant's `WFDIEs`
//! D-Bus property and a peer's `IEs` property both use the bare subelement
//! list (no `DD` header); Windows' `WiFiDirectInformationElement` takes the
//! OUI (50 6F 9A), the OUI type (0x0A) and the subelement list separately.

use thiserror::Error;

pub const WFA_OUI: [u8; 3] = [0x50, 0x6F, 0x9A];
pub const WFD_OUI_TYPE: u8 = 0x0A;
pub const DEFAULT_RTSP_PORT: u16 = 7236;

pub mod subelement_id {
    pub const DEVICE_INFO: u8 = 0;
    pub const ASSOCIATED_BSSID: u8 = 1;
    pub const AUDIO_FORMATS: u8 = 2;
    pub const VIDEO_FORMATS: u8 = 3;
    pub const VIDEO_3D_FORMATS: u8 = 4;
    pub const CONTENT_PROTECTION: u8 = 5;
    pub const COUPLED_SINK: u8 = 6;
    pub const EXTENDED_CAPABILITY: u8 = 7;
    pub const LOCAL_IP: u8 = 8;
    pub const SESSION_INFO: u8 = 9;
    pub const ALT_MAC: u8 = 10;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IeError {
    #[error("WFD subelement truncated at offset {0}")]
    Truncated(usize),
    #[error("WFD device information subelement must be 6 bytes, got {0}")]
    BadDeviceInfo(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    Source = 0,
    PrimarySink = 1,
    SecondarySink = 2,
    SourceOrPrimarySink = 3,
}

impl DeviceType {
    pub fn is_source(self) -> bool {
        matches!(self, DeviceType::Source | DeviceType::SourceOrPrimarySink)
    }
    pub fn is_sink(self) -> bool {
        !matches!(self, DeviceType::Source)
    }
}

/// WFD Device Information subelement (id 0, 6 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceInfo {
    pub device_type: DeviceType,
    pub coupled_sink_at_source: bool,
    pub coupled_sink_at_sink: bool,
    pub session_available: bool,
    pub service_discovery: bool,
    pub prefer_tdls: bool,
    pub content_protection: bool,
    pub time_sync: bool,
    pub audio_unsupported_at_primary_sink: bool,
    pub audio_only_source: bool,
    pub control_port: u16,
    pub max_throughput_mbps: u16,
}

impl DeviceInfo {
    pub fn primary_sink(control_port: u16) -> Self {
        Self {
            device_type: DeviceType::PrimarySink,
            coupled_sink_at_source: false,
            coupled_sink_at_sink: false,
            session_available: true,
            service_discovery: false,
            prefer_tdls: false,
            content_protection: false,
            time_sync: false,
            audio_unsupported_at_primary_sink: false,
            audio_only_source: false,
            control_port,
            max_throughput_mbps: 300,
        }
    }

    pub fn bitmap(&self) -> u16 {
        let mut b = self.device_type as u16;
        let set = |b: &mut u16, on: bool, bit: u16| {
            if on {
                *b |= 1 << bit
            }
        };
        set(&mut b, self.coupled_sink_at_source, 2);
        set(&mut b, self.coupled_sink_at_sink, 3);
        if self.session_available {
            b |= 0b01 << 4;
        }
        set(&mut b, self.service_discovery, 6);
        set(&mut b, self.prefer_tdls, 7);
        set(&mut b, self.content_protection, 8);
        set(&mut b, self.time_sync, 9);
        set(&mut b, self.audio_unsupported_at_primary_sink, 10);
        set(&mut b, self.audio_only_source, 11);
        b
    }

    pub fn from_bytes(body: &[u8]) -> Result<Self, IeError> {
        if body.len() < 6 {
            return Err(IeError::BadDeviceInfo(body.len()));
        }
        let b = u16::from_be_bytes([body[0], body[1]]);
        let bit = |n: u16| b & (1 << n) != 0;
        Ok(Self {
            device_type: match b & 0b11 {
                0 => DeviceType::Source,
                1 => DeviceType::PrimarySink,
                2 => DeviceType::SecondarySink,
                _ => DeviceType::SourceOrPrimarySink,
            },
            coupled_sink_at_source: bit(2),
            coupled_sink_at_sink: bit(3),
            session_available: (b >> 4) & 0b11 == 0b01,
            service_discovery: bit(6),
            prefer_tdls: bit(7),
            content_protection: bit(8),
            time_sync: bit(9),
            audio_unsupported_at_primary_sink: bit(10),
            audio_only_source: bit(11),
            control_port: u16::from_be_bytes([body[2], body[3]]),
            max_throughput_mbps: u16::from_be_bytes([body[4], body[5]]),
        })
    }

    pub fn to_bytes(&self) -> [u8; 6] {
        let b = self.bitmap().to_be_bytes();
        let p = self.control_port.to_be_bytes();
        let t = self.max_throughput_mbps.to_be_bytes();
        [b[0], b[1], p[0], p[1], t[0], t[1]]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subelement {
    pub id: u8,
    pub body: Vec<u8>,
}

pub fn encode_subelements(subs: &[Subelement]) -> Vec<u8> {
    let mut out = Vec::new();
    for s in subs {
        out.push(s.id);
        out.extend_from_slice(&(s.body.len() as u16).to_be_bytes());
        out.extend_from_slice(&s.body);
    }
    out
}

pub fn parse_subelements(data: &[u8]) -> Result<Vec<Subelement>, IeError> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if i + 3 > data.len() {
            return Err(IeError::Truncated(i));
        }
        let len = u16::from_be_bytes([data[i + 1], data[i + 2]]) as usize;
        if i + 3 + len > data.len() {
            return Err(IeError::Truncated(i));
        }
        out.push(Subelement { id: data[i], body: data[i + 3..i + 3 + len].to_vec() });
        i += 3 + len;
    }
    Ok(out)
}

/// Extracts WFD subelement bytes from either a bare subelement list or a
/// buffer of 802.11 IEs containing one or more WFD vendor IEs.
pub fn wfd_subelements_from_any(data: &[u8]) -> Vec<u8> {
    // Treat the buffer as an 802.11 IE list only if it tiles exactly into
    // [id][len][body] elements and contains at least one WFD vendor IE.
    let mut out = Vec::new();
    let mut i = 0;
    let mut found = false;
    while i + 2 <= data.len() {
        let (id, len) = (data[i], data[i + 1] as usize);
        if i + 2 + len > data.len() {
            break;
        }
        let body = &data[i + 2..i + 2 + len];
        if id == 0xDD && body.len() >= 4 && body[..3] == WFA_OUI && body[3] == WFD_OUI_TYPE {
            out.extend_from_slice(&body[4..]);
            found = true;
        }
        i += 2 + len;
    }
    if found && i == data.len() { out } else { data.to_vec() }
}

/// Wraps a subelement list into one or more vendor IEs (max 251 payload
/// bytes each after the 4-byte OUI header).
pub fn wrap_vendor_ie(subelements: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in subelements.chunks(251).chain(if subelements.is_empty() { Some(&[][..]) } else { None }) {
        out.push(0xDD);
        out.push((4 + chunk.len()) as u8);
        out.extend_from_slice(&WFA_OUI);
        out.push(WFD_OUI_TYPE);
        out.extend_from_slice(chunk);
    }
    out
}

/// The subelements a primary sink advertises: device info (+ associated
/// BSSID and coupled-sink placeholders, as shipping sinks do) and the
/// extended-capability subelement with the UIBC bit.
pub fn sink_subelements(control_port: u16, uibc: bool) -> Vec<u8> {
    let mut subs = vec![
        Subelement { id: subelement_id::DEVICE_INFO, body: DeviceInfo::primary_sink(control_port).to_bytes().to_vec() },
        Subelement { id: subelement_id::ASSOCIATED_BSSID, body: vec![0; 6] },
        Subelement { id: subelement_id::COUPLED_SINK, body: vec![0; 7] },
    ];
    if uibc {
        subs.push(Subelement { id: subelement_id::EXTENDED_CAPABILITY, body: vec![0x00, 0x01] });
    }
    encode_subelements(&subs)
}

/// What a sink needs to know about a peer from its WFD IE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerWfdInfo {
    pub device: DeviceInfo,
    pub uibc: bool,
}

pub fn parse_peer_info(ies: &[u8]) -> Option<PeerWfdInfo> {
    let subs = parse_subelements(&wfd_subelements_from_any(ies)).ok()?;
    let device = subs.iter().find(|s| s.id == subelement_id::DEVICE_INFO).and_then(|s| DeviceInfo::from_bytes(&s.body).ok())?;
    let uibc = subs.iter().find(|s| s.id == subelement_id::EXTENDED_CAPABILITY).is_some_and(|s| s.body.len() >= 2 && s.body[1] & 0x01 != 0);
    Some(PeerWfdInfo { device, uibc })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_info_matches_shipping_sinks() {
        // lazycast advertises `wfd_subelem_set 0 000600111c44012c`: primary
        // sink, session available, port 7236, 300 Mbps.
        let d = DeviceInfo::primary_sink(7236);
        assert_eq!(d.bitmap(), 0x0011);
        assert_eq!(d.to_bytes(), [0x00, 0x11, 0x1c, 0x44, 0x01, 0x2c]);
        assert_eq!(DeviceInfo::from_bytes(&d.to_bytes()).unwrap(), d);
    }

    #[test]
    fn sink_subelements_layout() {
        let s = sink_subelements(7236, true);
        assert_eq!(
            s,
            vec![
                0x00, 0x00, 0x06, 0x00, 0x11, 0x1c, 0x44, 0x01, 0x2c, // device info
                0x01, 0x00, 0x06, 0, 0, 0, 0, 0, 0, // associated BSSID
                0x06, 0x00, 0x07, 0, 0, 0, 0, 0, 0, 0, // coupled sink
                0x07, 0x00, 0x02, 0x00, 0x01, // extended capability: UIBC
            ]
        );
        let info = parse_peer_info(&s).unwrap();
        assert!(info.uibc);
        assert_eq!(info.device.control_port, 7236);
    }

    #[test]
    fn parses_source_inside_vendor_ie_list() {
        // A source (type 0) on port 7236 with session available, wrapped in a
        // vendor IE and preceded by an unrelated IE.
        let sub = encode_subelements(&[Subelement { id: 0, body: vec![0x00, 0x10, 0x1c, 0x44, 0x00, 0x32] }]);
        let mut ies = vec![0x01, 0x02, 0xAA, 0xBB];
        ies.extend(wrap_vendor_ie(&sub));
        for buf in [wrap_vendor_ie(&sub), ies, sub.clone()] {
            let info = parse_peer_info(&buf).unwrap();
            assert!(info.device.device_type.is_source());
            assert!(info.device.session_available);
            assert_eq!(info.device.control_port, 7236);
            assert!(!info.uibc);
        }
    }

    #[test]
    fn truncated_subelements_error() {
        assert_eq!(parse_subelements(&[0x00, 0x00, 0x06, 0x00]), Err(IeError::Truncated(0)));
        assert_eq!(parse_subelements(&[0x00, 0x00]), Err(IeError::Truncated(0)));
        assert_eq!(parse_subelements(&[]), Ok(vec![]));
    }
}
