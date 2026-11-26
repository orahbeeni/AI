//! `text/parameters` bodies used by GET_PARAMETER / SET_PARAMETER.

/// Parameter names in a GET_PARAMETER request body (one per line).
pub fn parse_names(body: &str) -> Vec<String> {
    body.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| l.trim_end_matches(':').to_string()).collect()
}

/// `name: value` pairs in a SET_PARAMETER request or GET_PARAMETER response.
/// Lines without a colon are kept as names with an empty value.
pub fn parse_params(body: &str) -> Vec<(String, String)> {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| match l.split_once(':') {
            Some((k, v)) => (k.trim().to_string(), v.trim().to_string()),
            None => (l.to_string(), String::new()),
        })
        .collect()
}

pub fn get<'a>(params: &'a [(String, String)], name: &str) -> Option<&'a str> {
    params.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

pub fn format_params<K: AsRef<str>, V: AsRef<str>>(params: &[(K, V)]) -> String {
    let mut s = String::new();
    for (k, v) in params {
        s.push_str(k.as_ref());
        s.push_str(": ");
        s.push_str(v.as_ref());
        s.push_str("\r\n");
    }
    s
}

pub fn format_names<S: AsRef<str>>(names: &[S]) -> String {
    let mut s = String::new();
    for n in names {
        s.push_str(n.as_ref());
        s.push_str("\r\n");
    }
    s
}

/// `wfd_client_rtp_ports: RTP/AVP/UDP;unicast 19000 0 mode=play`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientRtpPorts {
    pub profile: String,
    pub rtp_port0: u16,
    pub rtp_port1: u16,
    pub mode: String,
}

impl ClientRtpPorts {
    pub fn udp(port: u16) -> Self {
        Self { profile: "RTP/AVP/UDP;unicast".into(), rtp_port0: port, rtp_port1: 0, mode: "mode=play".into() }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let mut it = s.split_whitespace();
        Some(Self {
            profile: it.next()?.to_string(),
            rtp_port0: it.next()?.parse().ok()?,
            rtp_port1: it.next()?.parse().ok()?,
            mode: it.next().unwrap_or("mode=play").to_string(),
        })
    }
}

impl std::fmt::Display for ClientRtpPorts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} {} {}", self.profile, self.rtp_port0, self.rtp_port1, self.mode)
    }
}

/// Values of the RTSP `Transport:` header we care about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transport {
    pub client_port: Option<u16>,
    pub server_port: Option<u16>,
}

impl Transport {
    pub fn parse(s: &str) -> Self {
        let mut t = Self::default();
        for part in s.split(';') {
            let part = part.trim();
            let first_port = |v: &str| v.split('-').next().and_then(|p| p.trim().parse().ok());
            if let Some(v) = part.strip_prefix("client_port=") {
                t.client_port = first_port(v);
            } else if let Some(v) = part.strip_prefix("server_port=") {
                t.server_port = first_port(v);
            }
        }
        t
    }
}

/// `Session: 6B8B4567;timeout=30` -> ("6B8B4567", Some(30))
pub fn parse_session_header(s: &str) -> (String, Option<u64>) {
    let mut parts = s.split(';');
    let id = parts.next().unwrap_or("").trim().to_string();
    let timeout = parts.find_map(|p| p.trim().strip_prefix("timeout=").and_then(|v| v.trim().parse().ok()));
    (id, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_params() {
        let names = parse_names("wfd_video_formats\r\nwfd_audio_codecs\r\n\r\nintel_friendly_name\n");
        assert_eq!(names, vec!["wfd_video_formats", "wfd_audio_codecs", "intel_friendly_name"]);
        let p = parse_params("wfd_trigger_method: SETUP\r\nwfd_presentation_URL: rtsp://192.168.173.80/wfd1.0/streamid=0 none\r\n");
        assert_eq!(get(&p, "WFD_TRIGGER_METHOD"), Some("SETUP"));
        assert_eq!(get(&p, "wfd_presentation_URL"), Some("rtsp://192.168.173.80/wfd1.0/streamid=0 none"));
        assert_eq!(format_params(&[("a", "1"), ("b", "two")]), "a: 1\r\nb: two\r\n");
    }

    #[test]
    fn rtp_ports_and_transport() {
        let p = ClientRtpPorts::parse("RTP/AVP/UDP;unicast 1028 0 mode=play").unwrap();
        assert_eq!(p.rtp_port0, 1028);
        assert_eq!(p.to_string(), "RTP/AVP/UDP;unicast 1028 0 mode=play");
        let t = Transport::parse("RTP/AVP/UDP;unicast;client_port=19000-19001;server_port=5000-5001");
        assert_eq!(t, Transport { client_port: Some(19000), server_port: Some(5000) });
        assert_eq!(parse_session_header("6B8B4567;timeout=30"), ("6B8B4567".to_string(), Some(30)));
        assert_eq!(parse_session_header("abc"), ("abc".to_string(), None));
    }
}
