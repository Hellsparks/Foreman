//! Own reachable addresses (for `/api/info`) and the `_foreman._tcp` mDNS announcement.

use std::{
    net::{IpAddr, Ipv4Addr},
    time::Duration,
};

use mdns_sd::{ServiceDaemon, ServiceInfo as MdnsService};
use serde::Serialize;
use tracing::{info, warn};

pub const MDNS_SERVICE: &str = "_foreman._tcp.local.";

/// One way to reach this server, e.g. `{ "address": "10.0.0.20:9090", "kind": "lan_ip" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Address {
    pub address: String,
    pub kind: &'static str,
}

/// Tailscale's CGNAT range 100.64.0.0/10.
pub fn is_tailscale(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (o[1] & 0xC0) == 64
}

/// Container / VM bridges that other devices can't reach.
fn is_virtual_iface(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "docker", "br-", "veth", "virbr", "vboxnet", "vmnet", "cni", "flannel", "podman", "lxc",
        "lxd", "incus",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Usable IPv4 interface addresses as (interface name, ip).
pub fn interface_ips() -> Vec<(String, Ipv4Addr)> {
    let ifaces = match if_addrs::get_if_addrs() {
        Ok(i) => i,
        Err(e) => {
            warn!("could not list network interfaces: {e}");
            return Vec::new();
        }
    };
    let mut out: Vec<(String, Ipv4Addr)> = ifaces
        .into_iter()
        .filter(|i| !i.is_loopback() && !i.is_link_local() && !is_virtual_iface(&i.name))
        .filter_map(|i| match i.ip() {
            IpAddr::V4(v4) => Some((i.name, v4)),
            IpAddr::V6(_) => None,
        })
        .collect();
    out.sort_by_key(|(_, ip)| *ip);
    out.dedup_by_key(|(_, ip)| *ip);
    out
}

/// The addresses a client can use, given where the listener is bound.
///
/// Loopback-only → just `127.0.0.1`. A specific IP → that IP. Wildcard → LAN IPs,
/// `hostname.local`, then Tailscale IP + MagicDNS name when a 100.64.0.0/10 interface exists.
pub fn own_addresses(
    bound_host: &str,
    port: u16,
    ifaces: &[(String, Ipv4Addr)],
    hostname: &str,
    magic_dns: Option<&str>,
) -> Vec<Address> {
    let addr = |host: &str, kind| Address {
        address: format!("{host}:{port}"),
        kind,
    };
    match bound_host.parse::<IpAddr>() {
        Ok(ip) if ip.is_loopback() => return vec![addr("127.0.0.1", "loopback")],
        Ok(ip) if !ip.is_unspecified() => {
            let kind = match ip {
                IpAddr::V4(v4) if is_tailscale(v4) => "tailscale_ip",
                _ => "lan_ip",
            };
            return vec![addr(&ip.to_string(), kind)];
        }
        Err(_) if bound_host == "localhost" => return vec![addr("127.0.0.1", "loopback")],
        _ => {}
    }
    let mut out: Vec<Address> = ifaces
        .iter()
        .filter(|(_, ip)| !is_tailscale(*ip))
        .map(|(_, ip)| addr(&ip.to_string(), "lan_ip"))
        .collect();
    let host = hostname.trim();
    if !host.is_empty() && !host.contains('.') {
        out.push(addr(&format!("{host}.local"), "lan_name"));
    }
    let tailscale: Vec<&Ipv4Addr> = ifaces
        .iter()
        .map(|(_, ip)| ip)
        .filter(|ip| is_tailscale(**ip))
        .collect();
    for ip in &tailscale {
        out.push(addr(&ip.to_string(), "tailscale_ip"));
    }
    if let (false, Some(name)) = (tailscale.is_empty(), magic_dns) {
        out.push(addr(name, "tailscale_name"));
    }
    out
}

/// The machine's hostname (no domain).
pub fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().split('.').next().unwrap_or_default().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
}

/// Tailscale MagicDNS name of this machine (`tailscale status --json` → `Self.DNSName`).
pub async fn magic_dns_name() -> Option<String> {
    let run = tokio::process::Command::new("tailscale")
        .args(["status", "--json"])
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let name = v
        .get("Self")?
        .get("DNSName")?
        .as_str()?
        .trim_end_matches('.');
    (!name.is_empty()).then(|| name.to_owned())
}

/// Live `_foreman._tcp` registration; unregistered on drop.
pub struct Announcer {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Drop for Announcer {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// Announce this server on the LAN (TXT: id, name, version, needs_setup).
pub fn announce(
    server_id: &str,
    name: &str,
    version: &str,
    needs_setup: bool,
    hostname: &str,
    port: u16,
    ips: &[Ipv4Addr],
) -> Option<Announcer> {
    if ips.is_empty() {
        warn!("mDNS: no LAN address to announce");
        return None;
    }
    let host = if hostname.is_empty() {
        "foreman".to_owned()
    } else {
        hostname.to_owned()
    };
    let ip_list = ips
        .iter()
        .map(|ip| ip.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let props = [
        ("id", server_id),
        ("name", name),
        ("version", version),
        ("needs_setup", if needs_setup { "1" } else { "0" }),
    ];
    let result = ServiceDaemon::new().and_then(|daemon| {
        let svc = MdnsService::new(
            MDNS_SERVICE,
            name,
            &format!("{host}.local."),
            ip_list.as_str(),
            port,
            &props[..],
        )?;
        let fullname = svc.get_fullname().to_owned();
        daemon.register(svc)?;
        Ok(Announcer { daemon, fullname })
    });
    match result {
        Ok(a) => {
            info!(service = MDNS_SERVICE, %name, needs_setup, "announced on the LAN via mDNS");
            Some(a)
        }
        Err(e) => {
            warn!("mDNS announce failed: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ifaces() -> Vec<(String, Ipv4Addr)> {
        vec![
            ("eno1".into(), Ipv4Addr::new(10, 0, 0, 20)),
            ("tailscale0".into(), Ipv4Addr::new(100, 101, 5, 7)),
        ]
    }

    #[test]
    fn tailscale_range() {
        assert!(is_tailscale(Ipv4Addr::new(100, 64, 0, 1)));
        assert!(is_tailscale(Ipv4Addr::new(100, 127, 255, 254)));
        assert!(!is_tailscale(Ipv4Addr::new(100, 128, 0, 1)));
        assert!(!is_tailscale(Ipv4Addr::new(10, 0, 0, 1)));
    }

    #[test]
    fn loopback_bind_reports_only_loopback() {
        let a = own_addresses("127.0.0.1", 9090, &ifaces(), "pi", None);
        assert_eq!(
            a,
            vec![Address {
                address: "127.0.0.1:9090".into(),
                kind: "loopback"
            }]
        );
    }

    #[test]
    fn wildcard_bind_lists_lan_name_and_tailscale() {
        let a = own_addresses("0.0.0.0", 9090, &ifaces(), "pi", Some("pi.tail1234.ts.net"));
        let list: Vec<(&str, &str)> = a.iter().map(|x| (x.address.as_str(), x.kind)).collect();
        assert_eq!(
            list,
            vec![
                ("10.0.0.20:9090", "lan_ip"),
                ("pi.local:9090", "lan_name"),
                ("100.101.5.7:9090", "tailscale_ip"),
                ("pi.tail1234.ts.net:9090", "tailscale_name"),
            ]
        );
    }

    #[test]
    fn no_tailscale_interface_means_no_magic_dns() {
        let lan = vec![("eno1".to_owned(), Ipv4Addr::new(192, 168, 1, 5))];
        let a = own_addresses("0.0.0.0", 9090, &lan, "pi", Some("pi.tail1234.ts.net"));
        assert!(a.iter().all(|x| !x.kind.starts_with("tailscale")));
    }

    #[test]
    fn virtual_bridges_skipped() {
        assert!(is_virtual_iface("docker0"));
        assert!(is_virtual_iface("br-1a2b"));
        assert!(!is_virtual_iface("eno1"));
        assert!(!is_virtual_iface("tailscale0"));
    }
}
