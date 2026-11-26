//! `<name>._display._tcp.local` registration. On Linux this goes through the
//! system's Avahi daemon (so we don't run a second mDNS responder next to
//! it); elsewhere, or if Avahi is missing, through the `mdns-sd` responder.

use tracing::{info, warn};

use crate::net;

pub struct MdnsRegistration {
    pub backend: &'static str,
    #[cfg(target_os = "linux")]
    _avahi: Option<zbus::Connection>,
    mdns: Option<(mdns_sd::ServiceDaemon, String)>,
}

impl Drop for MdnsRegistration {
    fn drop(&mut self) {
        // Avahi frees the entry group when our D-Bus connection closes.
        if let Some((daemon, fullname)) = self.mdns.take() {
            let _ = daemon.unregister(&fullname);
            let _ = daemon.shutdown();
        }
    }
}

/// Registers the MS-MICE display service.
pub async fn register_display_service(instance: &str, port: u16, container_id: &str) -> Result<MdnsRegistration, String> {
    let txt = format!("container_id={container_id}");
    #[cfg(target_os = "linux")]
    {
        match register_avahi(instance, port, &txt).await {
            Ok(conn) => {
                info!(%instance, port, "registered _display._tcp with Avahi");
                return Ok(MdnsRegistration { backend: "Avahi", _avahi: Some(conn), mdns: None });
            }
            Err(e) => tracing::debug!("Avahi unavailable ({e}); using built-in mDNS responder"),
        }
    }
    let ip = net::primary_ipv4().ok_or("no IPv4 network address")?;
    let daemon = mdns_sd::ServiceDaemon::new().map_err(|e| e.to_string())?;
    let host = format!("{}.local.", net::short_host_name());
    let info = mdns_sd::ServiceInfo::new(
        &format!("{}.local.", super::SERVICE_TYPE),
        instance,
        &host,
        ip.to_string().as_str(),
        port,
        &[("container_id", container_id)][..],
    )
    .map_err(|e| e.to_string())?
    .enable_addr_auto();
    let fullname = info.get_fullname().to_string();
    daemon.register(info).map_err(|e| {
        warn!("mDNS register failed: {e}");
        e.to_string()
    })?;
    let _ = txt;
    info!(%instance, port, "registered _display._tcp with built-in mDNS");
    Ok(MdnsRegistration {
        backend: "mdns-sd",
        #[cfg(target_os = "linux")]
        _avahi: None,
        mdns: Some((daemon, fullname)),
    })
}

#[cfg(target_os = "linux")]
async fn register_avahi(instance: &str, port: u16, txt: &str) -> Result<zbus::Connection, String> {
    use zbus::zvariant::OwnedObjectPath;
    let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
    let server = zbus::Proxy::new(&conn, "org.freedesktop.Avahi", "/", "org.freedesktop.Avahi.Server").await.map_err(|e| e.to_string())?;
    let group: OwnedObjectPath = server.call("EntryGroupNew", &()).await.map_err(|e| e.to_string())?;
    let eg = zbus::Proxy::new(&conn, "org.freedesktop.Avahi", group.as_str().to_owned(), "org.freedesktop.Avahi.EntryGroup")
        .await
        .map_err(|e| e.to_string())?;
    let txt_records: Vec<Vec<u8>> = vec![txt.as_bytes().to_vec()];
    // AddService(iface=-1 any, proto=-1 any, flags=0, name, type, domain, host, port, txt)
    eg.call::<_, _, ()>("AddService", &(-1i32, -1i32, 0u32, instance, super::SERVICE_TYPE, "", "", port, txt_records))
        .await
        .map_err(|e| e.to_string())?;
    eg.call::<_, _, ()>("Commit", &()).await.map_err(|e| e.to_string())?;
    Ok(conn)
}
