//! Browse the LAN for `_foreman._tcp` servers.

use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr},
    time::{Duration, Instant},
};

use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde::Serialize;

pub const SERVICE: &str = "_foreman._tcp.local.";

#[derive(Debug, Clone, Serialize)]
pub struct Found {
    pub id: String,
    pub name: String,
    pub version: String,
    pub host: String,
    /// From TXT `needs_setup=1`: no admin yet (shown as "Not set up").
    pub needs_setup: bool,
    /// `ip:port`, LAN addresses first.
    pub addresses: Vec<String>,
}

fn is_tailscale(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (o[1] & 0xC0) == 64
}

/// LAN (private) first, then Tailscale, then anything else.
fn rank(ip: &Ipv4Addr) -> u8 {
    if ip.is_private() {
        0
    } else if is_tailscale(*ip) {
        1
    } else {
        2
    }
}

/// Blocking browse for `timeout`; one entry per server id.
pub fn discover(timeout: Duration) -> Result<Vec<Found>, String> {
    let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let rx = daemon.browse(SERVICE).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + timeout;
    let mut found: BTreeMap<String, Found> = BTreeMap::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match rx.recv_timeout(left) {
            Ok(ServiceEvent::ServiceResolved(s)) => {
                let Some(id) = s.get_property_val_str("id").map(str::to_owned) else {
                    continue;
                };
                let mut ips: Vec<Ipv4Addr> = s
                    .get_addresses()
                    .iter()
                    .filter_map(|a| match a.to_ip_addr() {
                        IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_link_local() => Some(v4),
                        _ => None,
                    })
                    .collect();
                ips.sort_by_key(|ip| (rank(ip), *ip));
                ips.dedup();
                let port = s.get_port();
                found.insert(
                    id.clone(),
                    Found {
                        id,
                        name: s
                            .get_property_val_str("name")
                            .unwrap_or("Foreman")
                            .to_owned(),
                        version: s.get_property_val_str("version").unwrap_or("").to_owned(),
                        host: s.get_hostname().trim_end_matches('.').to_owned(),
                        needs_setup: s.get_property_val_str("needs_setup") == Some("1"),
                        addresses: ips.iter().map(|ip| format!("{ip}:{port}")).collect(),
                    },
                );
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.stop_browse(SERVICE);
    let _ = daemon.shutdown();
    Ok(found.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_before_tailscale() {
        let mut ips = [Ipv4Addr::new(100, 101, 5, 7), Ipv4Addr::new(10, 0, 0, 20)];
        ips.sort_by_key(|ip| (rank(ip), *ip));
        assert_eq!(ips[0], Ipv4Addr::new(10, 0, 0, 20));
    }

    /// Manual: `cargo test -p foreman-desktop -- --ignored --nocapture` with a server on the LAN.
    #[test]
    #[ignore = "needs a foreman-server announcing on the LAN"]
    fn finds_a_running_server() {
        let found = discover(Duration::from_secs(3)).expect("browse");
        println!("{found:#?}");
        assert!(!found.is_empty());
    }
}
