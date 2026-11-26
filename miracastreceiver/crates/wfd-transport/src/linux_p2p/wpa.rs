//! Thin wrapper over wpa_supplicant's D-Bus API (`fi.w1.wpa_supplicant1`).

use std::collections::HashMap;

use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, Proxy};

pub const SERVICE: &str = "fi.w1.wpa_supplicant1";
pub const ROOT_PATH: &str = "/fi/w1/wpa_supplicant1";
pub const IFACE_ROOT: &str = "fi.w1.wpa_supplicant1";
pub const IFACE_INTERFACE: &str = "fi.w1.wpa_supplicant1.Interface";
pub const IFACE_P2P: &str = "fi.w1.wpa_supplicant1.Interface.P2PDevice";
pub const IFACE_WPS: &str = "fi.w1.wpa_supplicant1.Interface.WPS";
pub const IFACE_PEER: &str = "fi.w1.wpa_supplicant1.Peer";

#[derive(Debug, Clone)]
pub struct WpaError {
    pub access_denied: bool,
    pub message: String,
}

impl std::fmt::Display for WpaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<zbus::Error> for WpaError {
    fn from(e: zbus::Error) -> Self {
        let message = e.to_string();
        let access_denied = message.contains("AccessDenied") || message.contains("not allowed") || message.contains("Rejected send");
        Self { access_denied, message }
    }
}

impl From<zbus::fdo::Error> for WpaError {
    fn from(e: zbus::fdo::Error) -> Self {
        zbus::Error::from(e).into()
    }
}

pub type Result<T> = std::result::Result<T, WpaError>;

#[derive(Debug, Clone, Default)]
pub struct PeerInfo {
    pub name: String,
    pub device_address: [u8; 6],
    pub wfd_ies: Vec<u8>,
}

#[derive(Clone)]
pub struct Wpa {
    pub conn: Connection,
}

impl Wpa {
    /// System bus normally; `WFD_WPA_BUS=session` points at a mock service.
    pub async fn connect() -> Result<Self> {
        let conn = if std::env::var("WFD_WPA_BUS").as_deref() == Ok("session") {
            Connection::session().await?
        } else {
            Connection::system().await?
        };
        Ok(Self { conn })
    }

    pub async fn proxy(&self, path: &str, iface: &'static str) -> Result<Proxy<'static>> {
        Ok(Proxy::new(&self.conn, SERVICE, path.to_owned(), iface).await?)
    }

    pub async fn capabilities(&self) -> Result<Vec<String>> {
        Ok(self.proxy(ROOT_PATH, IFACE_ROOT).await?.get_property("Capabilities").await?)
    }

    pub async fn interfaces(&self) -> Result<Vec<(OwnedObjectPath, String)>> {
        let paths: Vec<OwnedObjectPath> = self.proxy(ROOT_PATH, IFACE_ROOT).await?.get_property("Interfaces").await?;
        let mut out = Vec::new();
        for p in paths {
            let name = self.ifname(p.as_str()).await.unwrap_or_default();
            out.push((p, name));
        }
        Ok(out)
    }

    pub async fn ifname(&self, path: &str) -> Result<String> {
        Ok(self.proxy(path, IFACE_INTERFACE).await?.get_property("Ifname").await?)
    }

    pub async fn wfd_ies(&self) -> Result<Vec<u8>> {
        Ok(self.proxy(ROOT_PATH, IFACE_ROOT).await?.get_property("WFDIEs").await?)
    }

    pub async fn set_wfd_ies(&self, ies: &[u8]) -> Result<()> {
        Ok(self.proxy(ROOT_PATH, IFACE_ROOT).await?.set_property("WFDIEs", ies.to_vec()).await?)
    }

    pub async fn p2p_config(&self, dev: &str) -> Result<HashMap<String, OwnedValue>> {
        Ok(self.proxy(dev, IFACE_P2P).await?.get_property("P2PDeviceConfig").await?)
    }

    pub async fn set_p2p_config(&self, dev: &str, cfg: HashMap<&str, Value<'_>>) -> Result<()> {
        Ok(self.proxy(dev, IFACE_P2P).await?.set_property("P2PDeviceConfig", cfg).await?)
    }

    pub async fn config_methods(&self, dev: &str) -> Result<String> {
        Ok(self.proxy(dev, IFACE_WPS).await?.get_property("ConfigMethods").await?)
    }

    pub async fn set_config_methods(&self, dev: &str, methods: &str) -> Result<()> {
        Ok(self.proxy(dev, IFACE_WPS).await?.set_property("ConfigMethods", methods).await?)
    }

    pub async fn find(&self, dev: &str) -> Result<()> {
        let mut args: HashMap<&str, Value<'_>> = HashMap::new();
        args.insert("DiscoveryType", Value::from("social"));
        Ok(self.proxy(dev, IFACE_P2P).await?.call("Find", &(args,)).await?)
    }

    pub async fn stop_find(&self, dev: &str) -> Result<()> {
        Ok(self.proxy(dev, IFACE_P2P).await?.call("StopFind", &()).await?)
    }

    pub async fn extended_listen(&self, dev: &str, period_ms: i32, interval_ms: i32) -> Result<()> {
        let mut args: HashMap<&str, Value<'_>> = HashMap::new();
        args.insert("period", Value::from(period_ms));
        args.insert("interval", Value::from(interval_ms));
        Ok(self.proxy(dev, IFACE_P2P).await?.call("ExtendedListen", &(args,)).await?)
    }

    /// `Connect`: method is "pbc", "display" or "keypad". Returns the PIN
    /// wpa_supplicant generated, if any.
    pub async fn connect_peer(
        &self,
        dev: &str,
        peer: &str,
        method: &str,
        pin: Option<&str>,
        go_intent: i32,
        authorize_only: bool,
    ) -> Result<Option<String>> {
        let mut args: HashMap<&str, Value<'_>> = HashMap::new();
        args.insert(
            "peer",
            Value::from(ObjectPath::try_from(peer.to_string()).map_err(|e| WpaError { access_denied: false, message: e.to_string() })?),
        );
        args.insert("wps_method", Value::from(method));
        args.insert("go_intent", Value::from(go_intent));
        args.insert("persistent", Value::from(false));
        args.insert("join", Value::from(false));
        args.insert("authorize_only", Value::from(authorize_only));
        if let Some(p) = pin {
            args.insert("pin", Value::from(p));
        }
        let reply = self.proxy(dev, IFACE_P2P).await?.call_method("Connect", &(args,)).await?;
        Ok(reply.body().deserialize::<String>().ok().filter(|s| !s.is_empty()))
    }

    pub async fn reject_peer(&self, dev: &str, peer: &str) -> Result<()> {
        let p = ObjectPath::try_from(peer.to_string()).map_err(|e| WpaError { access_denied: false, message: e.to_string() })?;
        Ok(self.proxy(dev, IFACE_P2P).await?.call("RejectPeer", &(p,)).await?)
    }

    /// Removes the group this interface belongs to.
    pub async fn disconnect_group(&self, group_iface: &str) -> Result<()> {
        Ok(self.proxy(group_iface, IFACE_P2P).await?.call("Disconnect", &()).await?)
    }

    pub async fn cancel(&self, dev: &str) -> Result<()> {
        Ok(self.proxy(dev, IFACE_P2P).await?.call("Cancel", &()).await?)
    }

    pub async fn peer_info(&self, peer: &str) -> Result<PeerInfo> {
        let p = self.proxy(peer, IFACE_PEER).await?;
        let name: String = p.get_property("DeviceName").await.unwrap_or_default();
        let addr: Vec<u8> = p.get_property("DeviceAddress").await.unwrap_or_default();
        let ies: Vec<u8> = p.get_property("IEs").await.unwrap_or_default();
        let mut device_address = [0u8; 6];
        if addr.len() == 6 {
            device_address.copy_from_slice(&addr);
        }
        Ok(PeerInfo { name, device_address, wfd_ies: ies })
    }
}

/// Marks an interface as unmanaged in NetworkManager so it doesn't try to
/// configure the P2P group link we manage (best effort).
pub async fn nm_unmanage(conn: &Connection, ifname: &str) {
    let Ok(nm) =
        Proxy::new(conn, "org.freedesktop.NetworkManager", "/org/freedesktop/NetworkManager", "org.freedesktop.NetworkManager").await
    else {
        return;
    };
    let Ok(dev): std::result::Result<OwnedObjectPath, _> = nm.call("GetDeviceByIpIface", &(ifname,)).await else { return };
    if let Ok(d) = Proxy::new(conn, "org.freedesktop.NetworkManager", dev, "org.freedesktop.NetworkManager.Device").await {
        let _ = d.set_property("Managed", false).await;
    }
}
