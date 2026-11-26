//! Wi-Fi Direct connections made through NetworkManager.
//!
//! NetworkManager deletes any P2P group it didn't ask for: on `GroupStarted`
//! it wraps the new interface, ignores it unless its own `wifi-p2p` device is
//! activating, and dropping that wrapper calls `RemoveInterface`. So when it
//! manages the P2P device we have it make the connection, with a temporary
//! `wifi-p2p` profile. It then also configures IP for either role (DHCP client,
//! the address handed out during pairing, or "shared" as group owner), which
//! needs no extra privileges from us.

use std::collections::HashMap;
use std::net::{Ipv4Addr, UdpSocket};
use std::time::Duration;

use tracing::debug;
use zbus::proxy::CacheProperties;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, Proxy};

const SERVICE: &str = "org.freedesktop.NetworkManager";
const PATH: &str = "/org/freedesktop/NetworkManager";
const IFACE: &str = "org.freedesktop.NetworkManager";
const IFACE_DEVICE: &str = "org.freedesktop.NetworkManager.Device";
const IFACE_ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const IFACE_IP4: &str = "org.freedesktop.NetworkManager.IP4Config";
const IFACE_DHCP4: &str = "org.freedesktop.NetworkManager.DHCP4Config";
const DEVICE_TYPE_WIFI_P2P: u32 = 30;
const ACTIVE_ACTIVATED: u32 = 2;
const ACTIVE_DEACTIVATED: u32 = 4;

#[derive(Clone)]
pub struct Nm {
    conn: Connection,
    device: OwnedObjectPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveState {
    Activating,
    Activated,
    Ended,
}

/// The configured group link.
#[derive(Debug, Default)]
struct Link {
    ifname: String,
    address: Option<Ipv4Addr>,
    /// The group owner's address, when we are its client.
    owner: Option<Ipv4Addr>,
}

async fn proxy(conn: &Connection, path: &str, iface: &'static str) -> zbus::Result<Proxy<'static>> {
    zbus::proxy::Builder::new(conn)
        .destination(SERVICE)?
        .path(path.to_owned())?
        .interface(iface)?
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

impl Nm {
    /// NetworkManager's P2P device for `wifi_ifname`, if it manages one.
    pub async fn detect(conn: &Connection, wifi_ifname: &str) -> Option<Self> {
        let name = if wifi_ifname.starts_with("p2p-dev-") { wifi_ifname.to_string() } else { format!("p2p-dev-{wifi_ifname}") };
        let nm = proxy(conn, PATH, IFACE).await.ok()?;
        let device: OwnedObjectPath = nm.call("GetDeviceByIpIface", &(name.as_str(),)).await.ok()?;
        let d = proxy(conn, device.as_str(), IFACE_DEVICE).await.ok()?;
        let kind: u32 = d.get_property("DeviceType").await.ok()?;
        let managed: bool = d.get_property("Managed").await.ok()?;
        (kind == DEVICE_TYPE_WIFI_P2P && managed).then(|| Self { conn: conn.clone(), device })
    }

    /// Connects to the peer with push-button pairing. The profile is never
    /// saved, and NetworkManager drops the link if this program goes away.
    pub async fn activate(&self, peer_mac: &str, wfd_ies: &[u8]) -> zbus::Result<OwnedObjectPath> {
        let mut settings: HashMap<&str, HashMap<&str, Value<'_>>> = HashMap::new();
        settings.insert(
            "connection",
            HashMap::from([
                ("id", Value::from("Miracast receiver")),
                ("type", Value::from("wifi-p2p")),
                ("autoconnect", Value::from(false)),
            ]),
        );
        settings.insert(
            "wifi-p2p",
            HashMap::from([("peer", Value::from(peer_mac.to_uppercase())), ("wfd-ies", Value::from(wfd_ies.to_vec()))]),
        );
        // the phone must never become the route to the internet or the DNS server
        settings.insert(
            "ipv4",
            HashMap::from([("method", Value::from("auto")), ("never-default", Value::from(true)), ("ignore-auto-dns", Value::from(true))]),
        );
        settings.insert("ipv6", HashMap::from([("method", Value::from("disabled"))]));
        let options: HashMap<&str, Value<'_>> =
            HashMap::from([("persist", Value::from("volatile")), ("bind-activation", Value::from("dbus-client"))]);
        let any = ObjectPath::from_static_str_unchecked("/");
        let nm = proxy(&self.conn, PATH, IFACE).await?;
        let (_profile, active, _result): (OwnedObjectPath, OwnedObjectPath, HashMap<String, OwnedValue>) =
            nm.call("AddAndActivateConnection2", &(settings, &self.device, any, options)).await?;
        Ok(active)
    }

    pub async fn deactivate(&self, active: &OwnedObjectPath) {
        let r: zbus::Result<()> = async { proxy(&self.conn, PATH, IFACE).await?.call("DeactivateConnection", &(active,)).await }.await;
        if let Err(e) = r {
            debug!("NetworkManager DeactivateConnection: {e}");
        }
    }

    pub async fn state(&self, active: &OwnedObjectPath) -> ActiveState {
        let state: zbus::Result<u32> = async { proxy(&self.conn, active.as_str(), IFACE_ACTIVE).await?.get_property("State").await }.await;
        match state {
            Ok(ACTIVE_ACTIVATED) => ActiveState::Activated,
            Ok(ACTIVE_DEACTIVATED) | Err(_) => ActiveState::Ended,
            Ok(_) => ActiveState::Activating,
        }
    }

    async fn link(&self) -> Option<Link> {
        let d = proxy(&self.conn, self.device.as_str(), IFACE_DEVICE).await.ok()?;
        let mut link = Link { ifname: d.get_property("IpInterface").await.ok()?, ..Default::default() };
        if let Ok(path) = d.get_property::<OwnedObjectPath>("Ip4Config").await
            && path.as_str() != "/"
            && let Ok(ip4) = proxy(&self.conn, path.as_str(), IFACE_IP4).await
        {
            if let Ok(list) = ip4.get_property::<Vec<HashMap<String, OwnedValue>>>("AddressData").await {
                link.address = list.first().and_then(|a| ip_value(a.get("address")?));
            }
            link.owner = ip4.get_property::<String>("Gateway").await.ok().and_then(|s| s.parse().ok());
        }
        if let Ok(path) = d.get_property::<OwnedObjectPath>("Dhcp4Config").await
            && path.as_str() != "/"
            && let Ok(dhcp) = proxy(&self.conn, path.as_str(), IFACE_DHCP4).await
            && let Ok(options) = dhcp.get_property::<HashMap<String, OwnedValue>>("Options").await
        {
            let server = ["dhcp_server_identifier", "routers"].iter().find_map(|k| ip_value(options.get(*k)?));
            link.owner = server.or(link.owner);
        }
        Some(link)
    }

    /// Waits until the link is configured and returns the source's address.
    /// `go` is our role; `owner_hint` the owner address from pairing, if any.
    pub async fn peer_address(&self, active: &OwnedObjectPath, go: bool, owner_hint: Option<Ipv4Addr>) -> Result<Ipv4Addr, String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(500)).await;
            match self.state(active).await {
                ActiveState::Ended => return Err("NetworkManager dropped the Wi-Fi Direct connection".into()),
                ActiveState::Activating => continue,
                ActiveState::Activated => {}
            }
            let Some(link) = self.link().await else { continue };
            if !go {
                if let Some(ip) = owner_hint.or(link.owner) {
                    return Ok(ip);
                }
            } else if let Some(own) = link.address {
                // We handed out the source's address; make the kernel resolve
                // every host on the link and see which one answers.
                if let Some(ip) = std::fs::read_to_string("/proc/net/arp").ok().and_then(|t| neighbour(&t, &link.ifname)) {
                    return Ok(ip);
                }
                probe_subnet(own);
            }
        }
        Err("could not find the source's address on the Wi-Fi Direct link".into())
    }
}

fn ip_value(v: &OwnedValue) -> Option<Ipv4Addr> {
    let s = String::try_from(v.try_clone().ok()?).ok()?;
    s.split_whitespace().next()?.parse().ok()
}

/// First resolved neighbour on `ifname` in `/proc/net/arp`.
fn neighbour(table: &str, ifname: &str) -> Option<Ipv4Addr> {
    table.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        let complete = u32::from_str_radix(f.get(2)?.trim_start_matches("0x"), 16).ok()? & 0x2 != 0;
        (complete && f.get(5) == Some(&ifname)).then(|| f[0].parse().ok())?
    })
}

fn probe_subnet(own: Ipv4Addr) {
    let Ok(socket) = UdpSocket::bind((own, 0)) else { return };
    let [a, b, c, mine] = own.octets();
    for host in (1..=254).filter(|h| *h != mine) {
        let _ = socket.send_to(&[0], (Ipv4Addr::new(a, b, c, host), 9));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_neighbour_on_the_group_link() {
        let table = "IP address       HW type     Flags       HW address            Mask     Device\n\
            192.168.1.1      0x1         0x2         aa:bb:cc:dd:ee:01     *        wlp4s0\n\
            10.42.0.9        0x1         0x0         00:00:00:00:00:00     *        p2p-wlp4s0-0\n\
            10.42.0.57       0x1         0x2         aa:bb:cc:dd:ee:02     *        p2p-wlp4s0-0\n";
        assert_eq!(neighbour(table, "p2p-wlp4s0-0"), Some(Ipv4Addr::new(10, 42, 0, 57)));
        assert_eq!(neighbour(table, "p2p-wlp4s0-1"), None);
    }
}
