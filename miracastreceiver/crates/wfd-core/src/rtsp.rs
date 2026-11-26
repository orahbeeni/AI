//! Minimal RTSP/1.0 message model, serializer and incremental parser.
//!
//! Wi-Fi Display uses RTSP in both directions over one TCP connection: the
//! source sends requests (M1, M3, M4, M5, M16) and the sink sends requests
//! (M2, M6, M7, M8, M13). So the parser accepts requests and responses alike.

use std::fmt;

use thiserror::Error;

/// Upper bound for a header block. Real WFD messages are well under 4 KiB.
const MAX_HEADER_BYTES: usize = 64 * 1024;
/// Upper bound for a body (M3/M4 parameter lists, EDID blobs).
const MAX_BODY_BYTES: usize = 1024 * 1024;

pub const RTSP_VERSION: &str = "RTSP/1.0";
pub const WFD_REQUIRE: &str = "org.wfa.wfd1.0";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RtspError {
    #[error("malformed RTSP start line: {0:?}")]
    BadStartLine(String),
    #[error("malformed RTSP header line: {0:?}")]
    BadHeader(String),
    #[error("bad Content-Length: {0:?}")]
    BadContentLength(String),
    #[error("RTSP header block exceeds {MAX_HEADER_BYTES} bytes")]
    HeaderTooLarge,
    #[error("RTSP body exceeds {MAX_BODY_BYTES} bytes")]
    BodyTooLarge,
}

/// Ordered, case-insensitive header list. Order is kept so serialized
/// messages look like the ones real sources send (some are picky about CSeq
/// coming first).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers(Vec<(String, String)>);

impl Headers {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Replace an existing header (keeping its position) or append it.
    pub fn set(&mut self, name: &str, value: impl Into<String>) {
        let value = value.into();
        if let Some(slot) = self.0.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
            slot.1 = value;
        } else {
            self.0.push((name.to_string(), value));
        }
    }

    pub fn remove(&mut self, name: &str) {
        self.0.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn cseq(&self) -> Option<u32> {
        self.get("CSeq").and_then(|v| v.trim().parse().ok())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub uri: String,
    pub headers: Headers,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub headers: Headers,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Request(Request),
    Response(Response),
}

impl Request {
    pub fn new(method: &str, uri: &str, cseq: u32) -> Self {
        let mut headers = Headers::new();
        headers.set("CSeq", cseq.to_string());
        Self { method: method.to_string(), uri: uri.to_string(), headers, body: Vec::new() }
    }

    pub fn with_header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.set(name, value);
        self
    }

    pub fn with_body(mut self, content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self.headers.set("Content-Type", content_type);
        self
    }

    pub fn cseq(&self) -> Option<u32> {
        self.headers.cseq()
    }

    pub fn body_str(&self) -> &str {
        std::str::from_utf8(&self.body).unwrap_or("")
    }
}

impl Response {
    pub fn new(status: u16, cseq: Option<u32>) -> Self {
        let mut headers = Headers::new();
        if let Some(c) = cseq {
            headers.set("CSeq", c.to_string());
        }
        Self { status, reason: reason_phrase(status).to_string(), headers, body: Vec::new() }
    }

    /// A 200 OK echoing the request's CSeq.
    pub fn ok_for(req: &Request) -> Self {
        Self::new(200, req.cseq())
    }

    pub fn with_header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.set(name, value);
        self
    }

    pub fn with_body(mut self, content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self.headers.set("Content-Type", content_type);
        self
    }

    pub fn cseq(&self) -> Option<u32> {
        self.headers.cseq()
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn body_str(&self) -> &str {
        std::str::from_utf8(&self.body).unwrap_or("")
    }
}

pub fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        303 => "See Other",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        451 => "Parameter Not Understood",
        454 => "Session Not Found",
        455 => "Method Not Valid in This State",
        457 => "Invalid Range",
        459 => "Aggregate Operation Not Allowed",
        461 => "Unsupported Transport",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        505 => "RTSP Version Not Supported",
        551 => "Option Not Supported",
        _ => "Unknown",
    }
}

impl Message {
    pub fn cseq(&self) -> Option<u32> {
        match self {
            Message::Request(r) => r.cseq(),
            Message::Response(r) => r.cseq(),
        }
    }

    /// Serialize to wire format. Content-Length is set from the body (and
    /// removed when the body is empty).
    pub fn to_bytes(&self) -> Vec<u8> {
        let (start, headers, body) = match self {
            Message::Request(r) => (format!("{} {} {}", r.method, r.uri, RTSP_VERSION), &r.headers, &r.body),
            Message::Response(r) => (format!("{} {} {}", RTSP_VERSION, r.status, r.reason), &r.headers, &r.body),
        };
        let mut out = String::with_capacity(256 + body.len());
        out.push_str(&start);
        out.push_str("\r\n");
        for (k, v) in headers.iter() {
            if k.eq_ignore_ascii_case("Content-Length") {
                continue;
            }
            out.push_str(k);
            out.push_str(": ");
            out.push_str(v);
            out.push_str("\r\n");
        }
        if !body.is_empty() {
            out.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        out.push_str("\r\n");
        let mut bytes = out.into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }

    /// One-line summary for logs.
    pub fn summary(&self) -> String {
        match self {
            Message::Request(r) => format!("{} {} (CSeq {})", r.method, r.uri, r.cseq().map_or("?".into(), |c| c.to_string())),
            Message::Response(r) => {
                format!("{} {} (CSeq {})", r.status, r.reason, r.cseq().map_or("?".into(), |c| c.to_string()))
            }
        }
    }
}

impl From<Request> for Message {
    fn from(r: Request) -> Self {
        Message::Request(r)
    }
}

impl From<Response> for Message {
    fn from(r: Response) -> Self {
        Message::Response(r)
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.to_bytes()))
    }
}

/// Incremental parser: feed raw TCP bytes, pull complete messages.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// Returns the next complete message, `Ok(None)` if more bytes are needed.
    pub fn next_message(&mut self) -> Result<Option<Message>, RtspError> {
        // Some stacks send stray CRLFs between messages (keep-alive padding).
        let lead = self.buf.iter().take_while(|b| **b == b'\r' || **b == b'\n').count();
        if lead > 0 {
            self.buf.drain(..lead);
        }
        if self.buf.is_empty() {
            return Ok(None);
        }
        let Some((head_len, sep_len)) = find_header_end(&self.buf) else {
            if self.buf.len() > MAX_HEADER_BYTES {
                return Err(RtspError::HeaderTooLarge);
            }
            return Ok(None);
        };
        let head = String::from_utf8_lossy(&self.buf[..head_len]).into_owned();
        let mut lines = head.split('\n').map(|l| l.trim_end_matches('\r'));
        let start = lines.next().unwrap_or_default().trim().to_string();
        let mut headers = Headers::new();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let Some((k, v)) = line.split_once(':') else {
                return Err(RtspError::BadHeader(line.to_string()));
            };
            headers.0.push((k.trim().to_string(), v.trim().to_string()));
        }
        let body_len = match headers.get("Content-Length") {
            Some(v) => v.trim().parse::<usize>().map_err(|_| RtspError::BadContentLength(v.to_string()))?,
            None => 0,
        };
        if body_len > MAX_BODY_BYTES {
            return Err(RtspError::BodyTooLarge);
        }
        // The body length is the source of truth; serialization re-adds it.
        headers.remove("Content-Length");
        let total = head_len + sep_len + body_len;
        if self.buf.len() < total {
            return Ok(None);
        }
        let body = self.buf[head_len + sep_len..total].to_vec();
        self.buf.drain(..total);

        let msg = if let Some(rest) = start.strip_prefix("RTSP/") {
            // "RTSP/1.0 200 OK"
            let mut parts = rest.splitn(3, ' ');
            let _version = parts.next();
            let status = parts.next().and_then(|s| s.parse::<u16>().ok()).ok_or_else(|| RtspError::BadStartLine(start.clone()))?;
            let reason = parts.next().unwrap_or("").to_string();
            Message::Response(Response { status, reason, headers, body })
        } else {
            let mut parts = start.split_whitespace();
            let (Some(method), Some(uri), Some(version)) = (parts.next(), parts.next(), parts.next()) else {
                return Err(RtspError::BadStartLine(start.clone()));
            };
            if !version.starts_with("RTSP/") {
                return Err(RtspError::BadStartLine(start.clone()));
            }
            Message::Request(Request { method: method.to_string(), uri: uri.to_string(), headers, body })
        };
        Ok(Some(msg))
    }
}

/// Finds the blank line ending the header block. Returns (header length,
/// separator length). Accepts CRLFCRLF and bare LFLF.
fn find_header_end(buf: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < buf.len() {
        if buf[i] == b'\n' {
            if buf.get(i + 1) == Some(&b'\n') {
                return Some((i, 2));
            }
            if buf.get(i + 1) == Some(&b'\r') && buf.get(i + 2) == Some(&b'\n') {
                // "\r\n\r\n": header ends before the first \r
                let head = if i > 0 && buf[i - 1] == b'\r' { i - 1 } else { i };
                return Some((head, i + 3 - head));
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_request_with_body() {
        let req = Request::new("SET_PARAMETER", "rtsp://localhost/wfd1.0", 4).with_body("text/parameters", "wfd_trigger_method: SETUP\r\n");
        let bytes = Message::from(req.clone()).to_bytes();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.starts_with("SET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\nCSeq: 4\r\n"));
        assert!(text.contains("Content-Length: 27\r\n"));
        let mut d = Decoder::new();
        d.feed(&bytes);
        assert_eq!(d.next_message().unwrap(), Some(Message::Request(req)));
        assert_eq!(d.next_message().unwrap(), None);
    }

    #[test]
    fn parses_split_and_pipelined_messages() {
        let a = Message::from(Response::new(200, Some(1)).with_header("Public", "org.wfa.wfd1.0, SETUP")).to_bytes();
        let b = Message::from(Request::new("OPTIONS", "*", 1).with_header("Require", WFD_REQUIRE)).to_bytes();
        let mut all = a.clone();
        all.extend_from_slice(&b);
        let mut d = Decoder::new();
        // byte-at-a-time feeding must still yield exactly two messages
        let mut got = Vec::new();
        for byte in &all {
            d.feed(std::slice::from_ref(byte));
            while let Some(m) = d.next_message().unwrap() {
                got.push(m);
            }
        }
        assert_eq!(got.len(), 2);
        assert!(matches!(&got[0], Message::Response(r) if r.status == 200 && r.headers.get("public").is_some()));
        assert!(matches!(&got[1], Message::Request(r) if r.method == "OPTIONS" && r.uri == "*"));
    }

    #[test]
    fn accepts_bare_lf_and_leading_crlf() {
        let mut d = Decoder::new();
        d.feed(b"\r\nGET_PARAMETER rtsp://x/wfd1.0 RTSP/1.0\nCSeq: 9\nSession: 1234\n\n");
        let m = d.next_message().unwrap().unwrap();
        assert_eq!(m.cseq(), Some(9));
        let Message::Request(r) = m else { panic!() };
        assert_eq!(r.headers.get("session"), Some("1234"));
        assert!(r.body.is_empty());
    }

    #[test]
    fn waits_for_full_body() {
        let mut d = Decoder::new();
        d.feed(b"RTSP/1.0 200 OK\r\nCSeq: 2\r\nContent-Length: 10\r\n\r\n12345");
        assert_eq!(d.next_message().unwrap(), None);
        d.feed(b"67890");
        let Message::Response(r) = d.next_message().unwrap().unwrap() else { panic!() };
        assert_eq!(r.body, b"1234567890");
    }

    #[test]
    fn rejects_garbage() {
        let mut d = Decoder::new();
        d.feed(b"HELLO\r\n\r\n");
        assert!(matches!(d.next_message(), Err(RtspError::BadStartLine(_))));
    }
}
