//! Notices network changes (Wi-Fi joined or lost, new DHCP address) and waking from sleep, so
//! links can be re-dialled at once instead of waiting for a timeout or a backoff.

use std::net::Ipv4Addr;
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::watch;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetState {
    /// This machine's non-loopback IPv4 addresses, sorted.
    pub addrs: Vec<Ipv4Addr>,
    /// Bumped on every change, including a wake from sleep with unchanged addresses.
    pub epoch: u64,
}

pub fn current_addrs() -> Vec<Ipv4Addr> {
    let mut addrs: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) if !v4.ip.is_loopback() => Some(v4.ip),
            _ => None,
        })
        .collect();
    addrs.sort();
    addrs.dedup();
    addrs
}

pub fn spawn() -> watch::Receiver<NetState> {
    let (tx, rx) = watch::channel(NetState { addrs: current_addrs(), epoch: 0 });
    tokio::spawn(async move {
        let period = Duration::from_secs(2);
        let (mut mono, mut wall) = (Instant::now(), SystemTime::now());
        loop {
            tokio::time::sleep(period).await;
            // The monotonic clock stops while suspended; the wall clock does not.
            let slept = wall.elapsed().unwrap_or_default().saturating_sub(mono.elapsed()) > Duration::from_secs(3);
            (mono, wall) = (Instant::now(), SystemTime::now());
            let addrs = current_addrs();
            let changed = addrs != tx.borrow().addrs;
            if changed || slept {
                if slept {
                    log::info!("woke from sleep");
                }
                if changed {
                    log::info!("network changed: {}", if addrs.is_empty() { "offline".to_string() } else { fmt_addrs(&addrs) });
                }
                tx.send_modify(|s| {
                    s.addrs = addrs;
                    s.epoch += 1;
                });
            }
            if tx.is_closed() {
                return;
            }
        }
    });
    rx
}

pub fn fmt_addrs(addrs: &[Ipv4Addr]) -> String {
    addrs.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ")
}
