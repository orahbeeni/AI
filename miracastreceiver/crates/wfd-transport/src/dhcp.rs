//! Minimal DHCPv4 (RFC 2131) for the Wi-Fi Direct group link.
//!
//! When we are the P2P Group Owner the phone (P2P client) asks us for an
//! address; [`DhcpServer`] hands one out and the lease tells us the
//! source's IP. When the phone ends up GO instead, [`DhcpClient`] gets us an
//! address from it. Both are sans-IO; the socket runners at the bottom bind
//! to the group interface (Linux).

use std::collections::HashMap;
use std::net::Ipv4Addr;

pub const SERVER_PORT: u16 = 67;
pub const CLIENT_PORT: u16 = 68;
const MAGIC: [u8; 4] = [99, 130, 83, 99];

pub mod msg {
    pub const DISCOVER: u8 = 1;
    pub const OFFER: u8 = 2;
    pub const REQUEST: u8 = 3;
    pub const DECLINE: u8 = 4;
    pub const ACK: u8 = 5;
    pub const NAK: u8 = 6;
    pub const RELEASE: u8 = 7;
    pub const INFORM: u8 = 8;
}

pub mod opt {
    pub const PAD: u8 = 0;
    pub const SUBNET_MASK: u8 = 1;
    pub const ROUTER: u8 = 3;
    pub const DNS: u8 = 6;
    pub const HOSTNAME: u8 = 12;
    pub const REQUESTED_IP: u8 = 50;
    pub const LEASE_TIME: u8 = 51;
    pub const MSG_TYPE: u8 = 53;
    pub const SERVER_ID: u8 = 54;
    pub const PARAM_LIST: u8 = 55;
    pub const RENEWAL_TIME: u8 = 58;
    pub const REBINDING_TIME: u8 = 59;
    pub const CLIENT_ID: u8 = 61;
    pub const END: u8 = 255;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub op: u8,
    pub xid: u32,
    pub secs: u16,
    pub flags: u16,
    pub ciaddr: Ipv4Addr,
    pub yiaddr: Ipv4Addr,
    pub siaddr: Ipv4Addr,
    pub giaddr: Ipv4Addr,
    pub chaddr: [u8; 16],
    pub options: Vec<(u8, Vec<u8>)>,
}

fn ip_at(b: &[u8], i: usize) -> Ipv4Addr {
    Ipv4Addr::new(b[i], b[i + 1], b[i + 2], b[i + 3])
}

impl Packet {
    fn blank(op: u8, xid: u32, mac: [u8; 6]) -> Self {
        let mut chaddr = [0u8; 16];
        chaddr[..6].copy_from_slice(&mac);
        Self {
            op,
            xid,
            secs: 0,
            flags: 0,
            ciaddr: Ipv4Addr::UNSPECIFIED,
            yiaddr: Ipv4Addr::UNSPECIFIED,
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            chaddr,
            options: Vec::new(),
        }
    }

    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 240 || b[1] != 1 || b[2] != 6 || b[236..240] != MAGIC {
            return None;
        }
        let mut chaddr = [0u8; 16];
        chaddr.copy_from_slice(&b[28..44]);
        let mut options = Vec::new();
        let mut i = 240;
        while i < b.len() {
            let code = b[i];
            if code == opt::END {
                break;
            }
            if code == opt::PAD {
                i += 1;
                continue;
            }
            let len = *b.get(i + 1)? as usize;
            let val = b.get(i + 2..i + 2 + len)?;
            options.push((code, val.to_vec()));
            i += 2 + len;
        }
        Some(Self {
            op: b[0],
            xid: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            secs: u16::from_be_bytes([b[8], b[9]]),
            flags: u16::from_be_bytes([b[10], b[11]]),
            ciaddr: ip_at(b, 12),
            yiaddr: ip_at(b, 16),
            siaddr: ip_at(b, 20),
            giaddr: ip_at(b, 24),
            chaddr,
            options,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![0u8; 240];
        b[0] = self.op;
        b[1] = 1; // ethernet
        b[2] = 6;
        b[4..8].copy_from_slice(&self.xid.to_be_bytes());
        b[8..10].copy_from_slice(&self.secs.to_be_bytes());
        b[10..12].copy_from_slice(&self.flags.to_be_bytes());
        b[12..16].copy_from_slice(&self.ciaddr.octets());
        b[16..20].copy_from_slice(&self.yiaddr.octets());
        b[20..24].copy_from_slice(&self.siaddr.octets());
        b[24..28].copy_from_slice(&self.giaddr.octets());
        b[28..44].copy_from_slice(&self.chaddr);
        b[236..240].copy_from_slice(&MAGIC);
        for (code, val) in &self.options {
            b.push(*code);
            b.push(val.len() as u8);
            b.extend_from_slice(val);
        }
        b.push(opt::END);
        if b.len() < 300 {
            b.resize(300, 0); // BOOTP minimum; some clients insist
        }
        b
    }

    pub fn option(&self, code: u8) -> Option<&[u8]> {
        self.options.iter().find(|(c, _)| *c == code).map(|(_, v)| v.as_slice())
    }

    fn option_ip(&self, code: u8) -> Option<Ipv4Addr> {
        self.option(code).filter(|v| v.len() >= 4).map(|v| ip_at(v, 0))
    }

    fn set(&mut self, code: u8, val: impl Into<Vec<u8>>) {
        self.options.retain(|(c, _)| *c != code);
        self.options.push((code, val.into()));
    }

    pub fn msg_type(&self) -> Option<u8> {
        self.option(opt::MSG_TYPE).and_then(|v| v.first().copied())
    }

    pub fn mac(&self) -> [u8; 6] {
        let mut m = [0u8; 6];
        m.copy_from_slice(&self.chaddr[..6]);
        m
    }

    pub fn hostname(&self) -> Option<String> {
        self.option(opt::HOSTNAME).map(|v| String::from_utf8_lossy(v).into_owned())
    }
}

pub fn format_mac(m: &[u8; 6]) -> String {
    m.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

pub fn prefix_to_mask(prefix: u8) -> Ipv4Addr {
    Ipv4Addr::from(if prefix == 0 { 0 } else { u32::MAX << (32 - prefix as u32) })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub mac: [u8; 6],
    pub ip: Ipv4Addr,
    pub hostname: Option<String>,
}

pub struct DhcpServer {
    pub server_ip: Ipv4Addr,
    pub prefix: u8,
    pool_start: u32,
    pool_size: u32,
    pub lease_secs: u32,
    leases: HashMap<[u8; 6], Ipv4Addr>,
}

impl DhcpServer {
    pub fn new(server_ip: Ipv4Addr, prefix: u8, pool_start: Ipv4Addr, pool_size: u32) -> Self {
        Self { server_ip, prefix, pool_start: u32::from(pool_start), pool_size, lease_secs: 3600, leases: HashMap::new() }
    }

    /// The usual Wi-Fi Direct setup: GO at .1, clients from .10.
    pub fn for_group(server_ip: Ipv4Addr) -> Self {
        let base = u32::from(server_ip) & 0xFFFF_FF00;
        Self::new(server_ip, 24, Ipv4Addr::from(base + 10), 40)
    }

    fn in_pool(&self, ip: Ipv4Addr) -> bool {
        let v = u32::from(ip);
        v >= self.pool_start && v < self.pool_start + self.pool_size
    }

    fn address_for(&mut self, mac: [u8; 6], wanted: Option<Ipv4Addr>) -> Option<Ipv4Addr> {
        if let Some(ip) = self.leases.get(&mac) {
            return Some(*ip);
        }
        let taken = |ip: &Ipv4Addr, leases: &HashMap<[u8; 6], Ipv4Addr>| leases.values().any(|l| l == ip);
        if let Some(w) = wanted
            && self.in_pool(w)
            && !taken(&w, &self.leases)
        {
            self.leases.insert(mac, w);
            return Some(w);
        }
        let ip = (0..self.pool_size).map(|i| Ipv4Addr::from(self.pool_start + i)).find(|ip| !taken(ip, &self.leases))?;
        self.leases.insert(mac, ip);
        Some(ip)
    }

    fn reply(&self, req: &Packet, kind: u8, yiaddr: Ipv4Addr) -> Packet {
        let mut p = Packet::blank(2, req.xid, req.mac());
        p.flags = req.flags;
        p.giaddr = req.giaddr;
        p.chaddr = req.chaddr;
        p.yiaddr = yiaddr;
        p.siaddr = self.server_ip;
        p.set(opt::MSG_TYPE, [kind]);
        p.set(opt::SERVER_ID, self.server_ip.octets());
        if kind != msg::NAK {
            p.set(opt::LEASE_TIME, self.lease_secs.to_be_bytes());
            p.set(opt::RENEWAL_TIME, (self.lease_secs / 2).to_be_bytes());
            p.set(opt::REBINDING_TIME, (self.lease_secs / 8 * 7).to_be_bytes());
            p.set(opt::SUBNET_MASK, prefix_to_mask(self.prefix).octets());
        }
        p
    }

    /// Handles one client packet. Returns the reply and, on ACK, the lease.
    pub fn handle(&mut self, req: &Packet) -> (Option<Packet>, Option<Lease>) {
        if req.op != 1 {
            return (None, None);
        }
        let mac = req.mac();
        match req.msg_type() {
            Some(msg::DISCOVER) => {
                let wanted = req.option_ip(opt::REQUESTED_IP);
                match self.address_for(mac, wanted) {
                    Some(ip) => (Some(self.reply(req, msg::OFFER, ip)), None),
                    None => (None, None),
                }
            }
            Some(msg::REQUEST) => {
                if let Some(sid) = req.option_ip(opt::SERVER_ID)
                    && sid != self.server_ip
                {
                    // client chose another server
                    self.leases.remove(&mac);
                    return (None, None);
                }
                let wanted = req.option_ip(opt::REQUESTED_IP).or((!req.ciaddr.is_unspecified()).then_some(req.ciaddr));
                let ok = match (wanted, self.leases.get(&mac)) {
                    (Some(w), Some(l)) => w == *l,
                    (Some(w), None) => self.address_for(mac, Some(w)) == Some(w),
                    (None, Some(_)) => true,
                    (None, None) => false,
                };
                if ok {
                    let ip = self.leases[&mac];
                    let lease = Lease { mac, ip, hostname: req.hostname() };
                    (Some(self.reply(req, msg::ACK, ip)), Some(lease))
                } else {
                    (Some(self.reply(req, msg::NAK, Ipv4Addr::UNSPECIFIED)), None)
                }
            }
            Some(msg::INFORM) => (Some(self.reply(req, msg::ACK, Ipv4Addr::UNSPECIFIED)), None),
            Some(msg::RELEASE) | Some(msg::DECLINE) => {
                self.leases.remove(&mac);
                (None, None)
            }
            _ => (None, None),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientLease {
    pub ip: Ipv4Addr,
    pub prefix: u8,
    pub server: Ipv4Addr,
    pub router: Option<Ipv4Addr>,
    pub lease_secs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientAction {
    Send(Packet),
    Bound(ClientLease),
    Restart,
    Ignore,
}

pub struct DhcpClient {
    mac: [u8; 6],
    xid: u32,
    hostname: String,
    offered: Option<(Ipv4Addr, Ipv4Addr)>,
}

impl DhcpClient {
    pub fn new(mac: [u8; 6], xid: u32, hostname: &str) -> Self {
        Self { mac, xid, hostname: hostname.to_string(), offered: None }
    }

    fn base(&self, kind: u8) -> Packet {
        let mut p = Packet::blank(1, self.xid, self.mac);
        p.flags = 0x8000; // ask for broadcast replies: we have no address yet
        p.set(opt::MSG_TYPE, [kind]);
        p.set(opt::HOSTNAME, self.hostname.as_bytes());
        p.set(opt::PARAM_LIST, [opt::SUBNET_MASK, opt::ROUTER, opt::DNS, opt::LEASE_TIME]);
        p
    }

    pub fn discover(&mut self) -> Packet {
        self.offered = None;
        self.base(msg::DISCOVER)
    }

    pub fn handle(&mut self, p: &Packet) -> ClientAction {
        if p.op != 2 || p.xid != self.xid || p.mac() != self.mac {
            return ClientAction::Ignore;
        }
        match (p.msg_type(), self.offered) {
            (Some(msg::OFFER), None) => {
                let server = p.option_ip(opt::SERVER_ID).unwrap_or(p.siaddr);
                self.offered = Some((p.yiaddr, server));
                let mut r = self.base(msg::REQUEST);
                r.set(opt::REQUESTED_IP, p.yiaddr.octets());
                r.set(opt::SERVER_ID, server.octets());
                ClientAction::Send(r)
            }
            (Some(msg::ACK), Some((ip, server))) if p.yiaddr == ip => {
                let mask = p.option_ip(opt::SUBNET_MASK).map_or(24, |m| u32::from(m).count_ones() as u8);
                let lease_secs =
                    p.option(opt::LEASE_TIME).filter(|v| v.len() == 4).map_or(3600, |v| u32::from_be_bytes([v[0], v[1], v[2], v[3]]));
                ClientAction::Bound(ClientLease { ip, prefix: mask, server, router: p.option_ip(opt::ROUTER), lease_secs })
            }
            (Some(msg::NAK), _) => {
                self.offered = None;
                ClientAction::Restart
            }
            _ => ClientAction::Ignore,
        }
    }
}

/// Socket runners bound to one interface (Linux: SO_BINDTODEVICE).
#[cfg(target_os = "linux")]
pub mod runner {
    use std::io;
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    use std::time::Duration;

    use socket2::{Domain, Protocol, Socket, Type};
    use tokio::net::UdpSocket;
    use tokio::sync::mpsc;
    use tracing::{debug, info};

    use super::*;

    pub fn bound_udp(ifname: &str, port: u16) -> io::Result<UdpSocket> {
        let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        s.set_reuse_address(true)?;
        s.set_broadcast(true)?;
        s.bind_device(Some(ifname.as_bytes()))?;
        s.set_nonblocking(true)?;
        s.bind(&SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)).into())?;
        UdpSocket::from_std(s.into())
    }

    /// Serves DHCP on `ifname` until the lease channel closes; every ACKed
    /// lease is sent to `leases`.
    pub async fn serve(ifname: String, mut server: DhcpServer, leases: mpsc::UnboundedSender<Lease>) -> io::Result<()> {
        let sock = bound_udp(&ifname, SERVER_PORT)?;
        info!(%ifname, server = %server.server_ip, "DHCP server running");
        let mut buf = vec![0u8; 1500];
        loop {
            tokio::select! {
                r = sock.recv_from(&mut buf) => {
                    let (n, from) = r?;
                    let Some(req) = Packet::parse(&buf[..n]) else { continue };
                    debug!(%from, kind = ?req.msg_type(), mac = %format_mac(&req.mac()), "DHCP request");
                    let (reply, lease) = server.handle(&req);
                    if let Some(rep) = reply {
                        let dest: SocketAddr = if !req.ciaddr.is_unspecified() && req.flags & 0x8000 == 0 {
                            SocketAddrV4::new(req.ciaddr, CLIENT_PORT).into()
                        } else {
                            SocketAddrV4::new(Ipv4Addr::BROADCAST, CLIENT_PORT).into()
                        };
                        sock.send_to(&rep.encode(), dest).await?;
                    }
                    if let Some(l) = lease {
                        info!(ip = %l.ip, mac = %format_mac(&l.mac), host = ?l.hostname, "DHCP lease");
                        if leases.send(l).is_err() {
                            return Ok(());
                        }
                    }
                }
                _ = leases.closed() => return Ok(()),
            }
        }
    }

    /// Obtains a lease on `ifname` (we are the P2P client).
    pub async fn obtain(ifname: &str, mac: [u8; 6], hostname: &str, timeout: Duration) -> io::Result<ClientLease> {
        let sock = bound_udp(ifname, CLIENT_PORT)?;
        let xid = u32::from_be_bytes([mac[2], mac[3], mac[4], mac[5]]) ^ std::process::id();
        let mut client = DhcpClient::new(mac, xid, hostname);
        let dest = SocketAddrV4::new(Ipv4Addr::BROADCAST, SERVER_PORT);
        let deadline = tokio::time::Instant::now() + timeout;
        let mut buf = vec![0u8; 1500];
        let mut outgoing = client.discover();
        loop {
            sock.send_to(&outgoing.encode(), dest).await?;
            let resend = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                let r = tokio::time::timeout_at(resend.min(deadline), sock.recv_from(&mut buf)).await;
                let Ok(r) = r else { break };
                let (n, _) = r?;
                let Some(p) = Packet::parse(&buf[..n]) else { continue };
                match client.handle(&p) {
                    ClientAction::Send(req) => {
                        outgoing = req;
                        break;
                    }
                    ClientAction::Bound(lease) => return Ok(lease),
                    ClientAction::Restart => {
                        outgoing = client.discover();
                        break;
                    }
                    ClientAction::Ignore => {}
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "no DHCP answer from the group owner"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: [u8; 6] = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];

    #[test]
    fn full_dora_between_client_and_server() {
        let mut server = DhcpServer::for_group(Ipv4Addr::new(192, 168, 173, 1));
        let mut client = DhcpClient::new(PHONE, 0xdead_beef, "Galaxy-S24");

        let discover = Packet::parse(&client.discover().encode()).unwrap();
        assert_eq!(discover.msg_type(), Some(msg::DISCOVER));
        let (offer, lease) = server.handle(&discover);
        assert!(lease.is_none());
        let offer = Packet::parse(&offer.unwrap().encode()).unwrap();
        assert_eq!(offer.msg_type(), Some(msg::OFFER));
        assert_eq!(offer.yiaddr, Ipv4Addr::new(192, 168, 173, 10));

        let ClientAction::Send(request) = client.handle(&offer) else { panic!() };
        let request = Packet::parse(&request.encode()).unwrap();
        assert_eq!(request.option(opt::REQUESTED_IP), Some(&[192, 168, 173, 10][..]));
        let (ack, lease) = server.handle(&request);
        let lease = lease.unwrap();
        assert_eq!(lease, Lease { mac: PHONE, ip: Ipv4Addr::new(192, 168, 173, 10), hostname: Some("Galaxy-S24".into()) });

        let ClientAction::Bound(cl) = client.handle(&Packet::parse(&ack.unwrap().encode()).unwrap()) else { panic!() };
        assert_eq!(
            cl,
            ClientLease {
                ip: Ipv4Addr::new(192, 168, 173, 10),
                prefix: 24,
                server: Ipv4Addr::new(192, 168, 173, 1),
                router: None,
                lease_secs: 3600
            }
        );

        // a second device gets the next address; the first keeps its lease
        let mut other = DhcpClient::new([2, 0, 0, 0, 0, 9], 7, "tab");
        let (offer2, _) = server.handle(&other.discover());
        assert_eq!(offer2.unwrap().yiaddr, Ipv4Addr::new(192, 168, 173, 11));
        let (again, _) = server.handle(&client.discover());
        assert_eq!(again.unwrap().yiaddr, Ipv4Addr::new(192, 168, 173, 10));
    }

    #[test]
    fn request_for_foreign_address_gets_nak() {
        let mut server = DhcpServer::for_group(Ipv4Addr::new(192, 168, 173, 1));
        let mut req = Packet::blank(1, 1, PHONE);
        req.set(opt::MSG_TYPE, [msg::REQUEST]);
        req.set(opt::REQUESTED_IP, [10, 0, 0, 5]); // remembered from another network
        let (reply, lease) = server.handle(&req);
        assert_eq!(reply.unwrap().msg_type(), Some(msg::NAK));
        assert!(lease.is_none());
    }

    #[test]
    fn ignores_replies_and_other_servers() {
        let mut server = DhcpServer::for_group(Ipv4Addr::new(192, 168, 173, 1));
        let mut req = Packet::blank(1, 1, PHONE);
        req.set(opt::MSG_TYPE, [msg::REQUEST]);
        req.set(opt::SERVER_ID, [192, 168, 49, 1]);
        assert_eq!(server.handle(&req), (None, None));
        assert!(Packet::parse(&[0u8; 100]).is_none());
        assert_eq!(prefix_to_mask(24), Ipv4Addr::new(255, 255, 255, 0));
        assert_eq!(format_mac(&PHONE), "02:11:22:33:44:55");
    }
}
