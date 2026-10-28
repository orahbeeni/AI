//! LAN discovery, so a client finds its server again after the server changes networks or IP.
//!
//! The server broadcasts a small beacon (key, TCP port, name) every second, to the limited
//! broadcast address, every interface's directed broadcast and a multicast group, and
//! immediately whenever its network changes. Clients also send probes while disconnected, which
//! the server answers directly; that covers networks that drop broadcast in one direction.
//! Beacons are unauthenticated hints only: the Noise handshake still checks the pinned key.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use anyhow::Result;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};

use crate::netwatch::NetState;

const BEACON: &[u8; 4] = b"WSB1";
const PROBE: &[u8; 4] = b"WSP1";
const MULTICAST: Ipv4Addr = Ipv4Addr::new(239, 255, 24, 90);

#[derive(Clone, Debug, PartialEq)]
pub struct Beacon {
    pub key: Vec<u8>,
    pub port: u16,
    pub name: String,
}

impl Beacon {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = BEACON.to_vec();
        out.extend_from_slice(&self.port.to_be_bytes());
        out.push(self.key.len() as u8);
        out.extend_from_slice(&self.key);
        out.extend_from_slice(self.name.as_bytes());
        out
    }

    pub fn decode(buf: &[u8]) -> Option<Beacon> {
        let rest = buf.strip_prefix(BEACON)?;
        let port = u16::from_be_bytes([*rest.first()?, *rest.get(1)?]);
        let klen = *rest.get(2)? as usize;
        let key = rest.get(3..3 + klen)?.to_vec();
        let name = String::from_utf8_lossy(rest.get(3 + klen..)?).into_owned();
        Some(Beacon { key, port, name })
    }
}

fn bind(port: u16) -> Result<UdpSocket> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    #[cfg(unix)]
    sock.set_reuse_port(true)?;
    sock.set_broadcast(true)?;
    sock.set_multicast_loop_v4(true)?;
    sock.set_nonblocking(true)?;
    sock.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port).into())?;
    Ok(UdpSocket::from_std(sock.into())?)
}

fn join_multicast(sock: &UdpSocket, net: &NetState) {
    let _ = sock.join_multicast_v4(MULTICAST, Ipv4Addr::UNSPECIFIED);
    for addr in &net.addrs {
        let _ = sock.join_multicast_v4(MULTICAST, *addr);
    }
}

/// Every address a discovery datagram should go to.
fn targets(port: u16) -> Vec<SocketAddr> {
    let mut out = vec![SocketAddr::from((Ipv4Addr::BROADCAST, port)), SocketAddr::from((MULTICAST, port))];
    for iface in if_addrs::get_if_addrs().unwrap_or_default() {
        if let if_addrs::IfAddr::V4(v4) = iface.addr
            && !v4.ip.is_loopback()
            && let Some(b) = v4.broadcast
        {
            let t = SocketAddr::from((b, port));
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

async fn send_all(sock: &UdpSocket, data: &[u8], port: u16) {
    for t in targets(port) {
        let _ = sock.send_to(data, t).await;
    }
}

/// Server side: announce this machine and answer probes, forever.
pub async fn announce(beacon: Beacon, udp_port: u16, mut net: watch::Receiver<NetState>) {
    let data = beacon.encode();
    loop {
        let sock = match bind(udp_port) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("discovery: cannot bind UDP {udp_port}: {e}; retrying");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut buf = [0u8; 512];
        loop {
            tokio::select! {
                _ = tick.tick() => send_all(&sock, &data, udp_port).await,
                r = net.changed() => {
                    if r.is_err() { return; }
                    // A new network: announce right away rather than at the next tick.
                    send_all(&sock, &data, udp_port).await;
                }
                r = sock.recv_from(&mut buf) => match r {
                    Ok((n, from)) if buf[..n].starts_with(PROBE) => { let _ = sock.send_to(&data, from).await; }
                    Ok(_) => {}
                    Err(_) => break,
                },
            }
        }
    }
}

/// Client side: report every beacon heard as (beacon, server TCP address). While `probing` is
/// true, also ask for beacons once a second.
pub async fn listen(udp_port: u16, found: mpsc::UnboundedSender<(Beacon, SocketAddr)>, mut probing: watch::Receiver<bool>, mut net: watch::Receiver<NetState>) {
    loop {
        let (beacons, probes) = match (bind(udp_port), bind(0)) {
            (Ok(b), Ok(p)) => (b, p),
            (Err(e), _) | (_, Err(e)) => {
                log::warn!("discovery: cannot bind UDP {udp_port}: {e}; relying on probes and the configured address");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        join_multicast(&beacons, &net.borrow());
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let (mut b1, mut b2) = ([0u8; 512], [0u8; 512]);
        loop {
            let got = tokio::select! {
                _ = tick.tick() => {
                    if *probing.borrow() { send_all(&probes, PROBE, udp_port).await; }
                    None
                }
                r = probing.changed() => {
                    if r.is_err() { return; }
                    if *probing.borrow() { send_all(&probes, PROBE, udp_port).await; }
                    None
                }
                r = net.changed() => {
                    if r.is_err() { return; }
                    join_multicast(&beacons, &net.borrow());
                    None
                }
                r = beacons.recv_from(&mut b1) => match r { Ok((n, from)) => Beacon::decode(&b1[..n]).map(|b| (b, from)), Err(_) => break },
                r = probes.recv_from(&mut b2) => match r { Ok((n, from)) => Beacon::decode(&b2[..n]).map(|b| (b, from)), Err(_) => break },
            };
            if let Some((b, from)) = got {
                let addr = SocketAddr::new(from.ip(), b.port);
                if found.send((b, addr)).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacon_round_trips_and_rejects_noise() {
        let b = Beacon { key: vec![7; 32], port: 24900, name: "MacBook".into() };
        assert_eq!(Beacon::decode(&b.encode()), Some(b));
        assert_eq!(Beacon::decode(b"WSB1\x00"), None);
        assert_eq!(Beacon::decode(b"hello"), None);
    }
}
