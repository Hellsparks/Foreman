//! Probe an address typed into the Connect screen: Foreman, Spoolman port, a Moonraker
//! printer, or nothing.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

pub const DEFAULT_PORT: u16 = 9090;
pub const SPOOLMAN_PORT: u16 = 7912;
const MOONRAKER_PORT: u16 = 7125;

/// A normalised address: `base` is the URL to call, `display` what we save and show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub base: String,
    pub display: String,
    pub host: String,
    pub port: u16,
    pub https: bool,
    pub port_given: bool,
}

/// `10.0.0.20` → `http://10.0.0.20:9090`; keeps an explicit port or `https://`.
pub fn normalize(raw: &str) -> Result<Target, String> {
    let raw = raw.trim().trim_end_matches('/');
    if raw.is_empty() {
        return Err("Enter an address".into());
    }
    let (https, rest) = if let Some(r) = raw.strip_prefix("https://") {
        (true, r)
    } else {
        (false, raw.strip_prefix("http://").unwrap_or(raw))
    };
    let scheme = if https { "https" } else { "http" };
    let url = reqwest::Url::parse(&format!("{scheme}://{rest}"))
        .map_err(|e| format!("Not a valid address: {e}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| "Not a valid address".to_owned())?
        .to_owned();
    if url.path() != "/" && !url.path().is_empty() {
        return Err("Leave out the path; just the host and port".into());
    }
    let port_given = url.port().is_some();
    let port = url.port().unwrap_or(DEFAULT_PORT);
    // `host_str` keeps the brackets on IPv6 literals.
    let host_port = format!("{host}:{port}");
    Ok(Target {
        base: format!("{scheme}://{host_port}"),
        display: if https {
            format!("https://{host_port}")
        } else {
            host_port
        },
        host,
        port,
        https,
        port_given,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Probe {
    Foreman {
        address: String,
        server_id: String,
        name: String,
        version: String,
        needs_setup: bool,
        /// Addresses the server reports for itself (`/api/info` → `addresses`).
        addresses: Value,
    },
    Spoolman {
        address: String,
        suggestion: String,
    },
    Moonraker {
        address: String,
        hostname: Option<String>,
    },
    NoAnswer {
        address: String,
        error: String,
    },
    Invalid {
        error: String,
    },
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap_or_default()
}

async fn get_json(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let res = client.get(url).send().await.map_err(describe)?;
    res.json::<Value>().await.map_err(|e| e.to_string())
}

fn describe(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "timed out".into()
    } else if e.is_connect() {
        "connection refused or host unreachable".into()
    } else {
        e.to_string()
    }
}

/// Parse an `/api/info` body; `None` if it isn't a Foreman server.
pub fn foreman_from_info(address: &str, v: &Value) -> Option<Probe> {
    let id = v.get("server_id")?.as_str()?;
    Some(Probe::Foreman {
        address: address.to_owned(),
        server_id: id.to_owned(),
        name: v
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Foreman")
            .to_owned(),
        version: v
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        needs_setup: v
            .get("needs_setup")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        addresses: v.get("addresses").cloned().unwrap_or(Value::Array(vec![])),
    })
}

fn is_moonraker(v: &Value) -> bool {
    let r = v.get("result").unwrap_or(&Value::Null);
    r.get("moonraker_version").is_some() || r.get("klippy_state").is_some()
}

/// `server_id` at this address, if a Foreman server answers there.
pub async fn server_id_at(client: &reqwest::Client, raw: &str) -> Option<String> {
    let t = normalize(raw).ok()?;
    let v = get_json(client, &format!("{}/api/info", t.base))
        .await
        .ok()?;
    v.get("server_id")?.as_str().map(str::to_owned)
}

pub async fn probe(client: &reqwest::Client, raw: &str) -> Probe {
    let t = match normalize(raw) {
        Ok(t) => t,
        Err(error) => return Probe::Invalid { error },
    };
    if t.port == SPOOLMAN_PORT {
        let suggestion = normalize(&format!("{}:{DEFAULT_PORT}", t.host))
            .map(|s| s.display)
            .unwrap_or_default();
        return Probe::Spoolman {
            address: t.display,
            suggestion,
        };
    }
    let foreman_err = match get_json(client, &format!("{}/api/info", t.base)).await {
        Ok(v) => match foreman_from_info(&t.display, &v) {
            Some(p) => return p,
            None => "answered, but not as Foreman".to_owned(),
        },
        Err(e) => e,
    };

    // Not Foreman: is it a printer? Try the address itself, then Moonraker's usual ports.
    let scheme = if t.https { "https" } else { "http" };
    let mut candidates = vec![t.base.clone()];
    if !t.port_given {
        candidates.push(format!("{scheme}://{}:{MOONRAKER_PORT}", t.host));
        candidates.push(format!("{scheme}://{}", t.host));
    }
    for base in candidates {
        if let Ok(v) = get_json(client, &format!("{base}/server/info")).await {
            if is_moonraker(&v) {
                let hostname = get_json(client, &format!("{base}/printer/info"))
                    .await
                    .ok()
                    .and_then(|p| {
                        p.get("result")?
                            .get("hostname")?
                            .as_str()
                            .map(str::to_owned)
                    });
                return Probe::Moonraker {
                    address: t.display,
                    hostname,
                };
            }
        }
    }
    Probe::NoAnswer {
        address: t.display,
        error: foreman_err,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn adds_default_port() {
        let t = normalize(" 10.0.0.20 ").expect("ok");
        assert_eq!(t.base, "http://10.0.0.20:9090");
        assert_eq!(t.display, "10.0.0.20:9090");
        assert!(!t.port_given);
    }

    #[test]
    fn keeps_port_and_scheme() {
        let t = normalize("http://pi.local:8080/").expect("ok");
        assert_eq!((t.display.as_str(), t.port_given), ("pi.local:8080", true));
        let t = normalize("https://pi.tail1234.ts.net").expect("ok");
        assert_eq!(t.base, "https://pi.tail1234.ts.net:9090");
        assert_eq!(t.display, "https://pi.tail1234.ts.net:9090");
    }

    #[test]
    fn rejects_empty_and_paths() {
        assert!(normalize("  ").is_err());
        assert!(normalize("10.0.0.20/api").is_err());
    }

    #[test]
    fn info_parsing() {
        let v =
            json!({ "server_id": "abc", "name": "Pi", "version": "0.1.0", "needs_setup": true });
        match foreman_from_info("10.0.0.20:9090", &v) {
            Some(Probe::Foreman {
                server_id,
                needs_setup,
                ..
            }) => {
                assert_eq!(server_id, "abc");
                assert!(needs_setup);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(foreman_from_info("x", &json!({ "result": {} })).is_none());
        assert!(is_moonraker(
            &json!({ "result": { "klippy_state": "ready" } })
        ));
    }

    #[tokio::test]
    async fn spoolman_port_short_circuits() {
        match probe(&client(), "10.0.0.20:7912").await {
            Probe::Spoolman { suggestion, .. } => assert_eq!(suggestion, "10.0.0.20:9090"),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// Manual: `FOREMAN_PROBE=addr1,addr2 cargo test -p foreman-desktop -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "probes real addresses from FOREMAN_PROBE"]
    async fn probe_real_addresses() {
        let list = std::env::var("FOREMAN_PROBE").unwrap_or_default();
        for a in list.split(',').filter(|a| !a.is_empty()) {
            println!("{a} => {:?}", probe(&client(), a).await);
        }
    }
}
