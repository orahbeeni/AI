//! Small cross-platform network helpers.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// Host name without domain (MS-MICE forbids '.' in the advertised name).
pub fn short_host_name() -> String {
    let raw = std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .or_else(|| std::process::Command::new("hostname").output().ok().and_then(|o| String::from_utf8(o.stdout).ok()))
        .unwrap_or_else(|| "miracast-receiver".into());
    let name = raw.trim().split('.').next().unwrap_or("").trim().to_string();
    if name.is_empty() { "miracast-receiver".into() } else { name }
}

/// The IPv4 address of the interface that carries the default route.
/// (Connecting a UDP socket sends no packets; it only selects a route.)
pub fn primary_ipv4() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_unspecified() => Some(v4),
        _ => None,
    }
}

/// Friendly name default: "<host> (Miracast)".
pub fn default_friendly_name() -> String {
    format!("{} Display", short_host_name())
}
