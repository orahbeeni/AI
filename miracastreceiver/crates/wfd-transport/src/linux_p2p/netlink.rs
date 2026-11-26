//! Just enough rtnetlink to put an IPv4 address on the P2P group interface
//! (needs CAP_NET_ADMIN), plus a capability check for clear error messages.

use std::ffi::CString;
use std::io;
use std::mem::size_of;
use std::net::Ipv4Addr;

const RTM_NEWADDR: u16 = 20;
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_ACK: u16 = 0x4;
const NLM_F_REPLACE: u16 = 0x100;
const NLM_F_CREATE: u16 = 0x400;
const NLMSG_ERROR: u16 = 2;
const IFA_ADDRESS: u16 = 1;
const IFA_LOCAL: u16 = 2;
const IFA_BROADCAST: u16 = 4;
const GENL_ID_CTRL: u16 = 16;
const CTRL_CMD_GETFAMILY: u8 = 3;
const CTRL_ATTR_FAMILY_ID: u16 = 1;
const CTRL_ATTR_FAMILY_NAME: u16 = 2;
const NL80211_CMD_SET_POWER_SAVE: u8 = 61;
const NL80211_ATTR_IFINDEX: u16 = 3;
const NL80211_ATTR_PS_STATE: u16 = 93;

pub fn if_index(ifname: &str) -> io::Result<u32> {
    let c = CString::new(ifname).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad interface name"))?;
    let idx = unsafe { libc::if_nametoindex(c.as_ptr()) };
    if idx == 0 { Err(io::Error::last_os_error()) } else { Ok(idx) }
}

fn push_attr(buf: &mut Vec<u8>, kind: u16, data: &[u8]) {
    let len = 4 + data.len();
    buf.extend_from_slice(&(len as u16).to_ne_bytes());
    buf.extend_from_slice(&kind.to_ne_bytes());
    buf.extend_from_slice(data);
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

/// `ip addr replace <addr>/<prefix> brd + dev <ifname>`
pub fn add_ipv4_address(ifname: &str, addr: Ipv4Addr, prefix: u8) -> io::Result<()> {
    let index = if_index(ifname)?;
    let mut msg = vec![0u8; 16]; // nlmsghdr, filled below
    // struct ifaddrmsg { u8 family, u8 prefixlen, u8 flags, u8 scope, u32 index }
    msg.push(libc::AF_INET as u8);
    msg.push(prefix);
    msg.push(0);
    msg.push(0); // RT_SCOPE_UNIVERSE
    msg.extend_from_slice(&index.to_ne_bytes());
    push_attr(&mut msg, IFA_LOCAL, &addr.octets());
    push_attr(&mut msg, IFA_ADDRESS, &addr.octets());
    let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix as u32) };
    let brd = Ipv4Addr::from(u32::from(addr) | !mask);
    push_attr(&mut msg, IFA_BROADCAST, &brd.octets());
    let len = msg.len() as u32;
    msg[0..4].copy_from_slice(&len.to_ne_bytes());
    msg[4..6].copy_from_slice(&RTM_NEWADDR.to_ne_bytes());
    msg[6..8].copy_from_slice(&(NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_REPLACE).to_ne_bytes());
    msg[8..12].copy_from_slice(&1u32.to_ne_bytes()); // seq

    unsafe {
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_CLOEXEC, libc::NETLINK_ROUTE);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut sa: libc::sockaddr_nl = std::mem::zeroed();
        sa.nl_family = libc::AF_NETLINK as u16;
        let r = libc::sendto(
            fd,
            msg.as_ptr().cast(),
            msg.len(),
            0,
            (&sa as *const libc::sockaddr_nl).cast(),
            size_of::<libc::sockaddr_nl>() as u32,
        );
        if r < 0 {
            let e = io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        let mut reply = [0u8; 4096];
        let n = libc::recv(fd, reply.as_mut_ptr().cast(), reply.len(), 0);
        let e = io::Error::last_os_error();
        libc::close(fd);
        if n < 0 {
            return Err(e);
        }
        if n as usize >= 20 && u16::from_ne_bytes([reply[4], reply[5]]) == NLMSG_ERROR {
            let code = i32::from_ne_bytes([reply[16], reply[17], reply[18], reply[19]]);
            if code != 0 {
                return Err(io::Error::from_raw_os_error(-code));
            }
        }
    }
    Ok(())
}

/// One generic netlink request; returns the first reply message.
fn generic_request(fd: i32, family: u16, command: u8, flags: u16, attrs: &[(u16, &[u8])]) -> io::Result<Vec<u8>> {
    let mut msg = vec![0u8; 16]; // nlmsghdr, filled below
    msg.extend_from_slice(&[command, 1, 0, 0]); // genlmsghdr: cmd, version
    for (kind, data) in attrs {
        push_attr(&mut msg, *kind, data);
    }
    let len = msg.len() as u32;
    msg[0..4].copy_from_slice(&len.to_ne_bytes());
    msg[4..6].copy_from_slice(&family.to_ne_bytes());
    msg[6..8].copy_from_slice(&(NLM_F_REQUEST | flags).to_ne_bytes());
    msg[8..12].copy_from_slice(&1u32.to_ne_bytes()); // seq
    let mut reply = vec![0u8; 8192];
    let n = unsafe {
        // an unconnected netlink socket sends to the kernel
        if libc::send(fd, msg.as_ptr().cast(), msg.len(), 0) < 0 {
            return Err(io::Error::last_os_error());
        }
        libc::recv(fd, reply.as_mut_ptr().cast(), reply.len(), 0)
    };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    reply.truncate(n as usize);
    if reply.len() >= 20 && u16::from_ne_bytes([reply[4], reply[5]]) == NLMSG_ERROR {
        let code = i32::from_ne_bytes([reply[16], reply[17], reply[18], reply[19]]);
        if code != 0 {
            return Err(io::Error::from_raw_os_error(-code));
        }
    }
    Ok(reply)
}

fn find_attr(mut attrs: &[u8], wanted: u16) -> Option<&[u8]> {
    while attrs.len() >= 4 {
        let len = u16::from_ne_bytes([attrs[0], attrs[1]]) as usize;
        if len < 4 || len > attrs.len() {
            return None;
        }
        if u16::from_ne_bytes([attrs[2], attrs[3]]) == wanted {
            return Some(&attrs[4..len]);
        }
        attrs = &attrs[len.next_multiple_of(4).min(attrs.len())..];
    }
    None
}

/// `iw dev <ifname> set power_save off`. A sleeping Wi-Fi client only collects
/// its data every beacon or so, which adds tens of milliseconds of lag.
pub fn disable_wifi_power_save(ifname: &str) -> io::Result<()> {
    let index = if_index(ifname)?;
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_CLOEXEC, libc::NETLINK_GENERIC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a new descriptor that nothing else owns
    let socket = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    let fd = std::os::fd::AsRawFd::as_raw_fd(&socket);
    let reply = generic_request(fd, GENL_ID_CTRL, CTRL_CMD_GETFAMILY, 0, &[(CTRL_ATTR_FAMILY_NAME, b"nl80211\0")])?;
    let family = reply
        .get(20..)
        .and_then(|a| find_attr(a, CTRL_ATTR_FAMILY_ID))
        .and_then(|v| v.get(..2))
        .map(|v| u16::from_ne_bytes([v[0], v[1]]))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no nl80211 in this kernel"))?;
    generic_request(
        fd,
        family,
        NL80211_CMD_SET_POWER_SAVE,
        NLM_F_ACK,
        &[(NL80211_ATTR_IFINDEX, &index.to_ne_bytes()), (NL80211_ATTR_PS_STATE, &0u32.to_ne_bytes())],
    )?;
    Ok(())
}

/// MAC address of an interface (from sysfs).
pub fn mac_address(ifname: &str) -> Option<[u8; 6]> {
    let s = std::fs::read_to_string(format!("/sys/class/net/{ifname}/address")).ok()?;
    let parts: Vec<u8> = s.trim().split(':').filter_map(|p| u8::from_str_radix(p, 16).ok()).collect();
    parts.try_into().ok()
}

/// IPv4 addresses currently on any interface (to avoid subnet clashes).
pub fn local_ipv4_addresses() -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) != 0 {
            return out;
        }
        let mut cur = ifap;
        while !cur.is_null() {
            let a = (*cur).ifa_addr;
            if !a.is_null() && (*a).sa_family as i32 == libc::AF_INET {
                let sin = &*(a as *const libc::sockaddr_in);
                out.push(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)));
            }
            cur = (*cur).ifa_next;
        }
        libc::freeifaddrs(ifap);
    }
    out
}

/// Effective Linux capabilities of this process.
pub fn effective_caps() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("CapEff:").map(|v| u64::from_str_radix(v.trim(), 16).unwrap_or(0))))
        .unwrap_or(0)
}

pub const CAP_NET_BIND_SERVICE: u32 = 10;
pub const CAP_NET_ADMIN: u32 = 12;
pub const CAP_NET_RAW: u32 = 13;

pub fn missing_caps() -> Vec<&'static str> {
    let caps = effective_caps();
    let mut missing = Vec::new();
    for (bit, name) in [(CAP_NET_ADMIN, "cap_net_admin"), (CAP_NET_BIND_SERVICE, "cap_net_bind_service"), (CAP_NET_RAW, "cap_net_raw")] {
        if caps & (1 << bit) == 0 {
            missing.push(name);
        }
    }
    missing
}
