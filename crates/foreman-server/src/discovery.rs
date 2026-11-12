//! Printer discovery and probing for first-run setup (task 6.4, W1).
//!
//! Read-only: HTTP GETs to Moonraker, a TLS handshake (certificate only) and an FTPS
//! sign-in to Bambu printers, mDNS browsing and SSDP listening. Never a command.
//!
//! Sources, merged by IP address:
//! - mDNS: `_moonraker._tcp` (Moonraker's `[zeroconf]`, off by default), plus hosts that
//!   advertise SSH / SFTP / workstation / HTTP, which are then probed for Moonraker.
//! - Sweep: TCP connect to 7125 (Moonraker) and 8883 (Bambu MQTT) on this machine's
//!   private /24 networks.
//! - Bambu SSDP: NOTIFY on UDP 2021 + an M-SEARCH to 239.255.255.250:1990. A host
//!   firewall often drops these; the 8883 certificate (subject CN = serial) covers that.
//!
//! With printer connections off (`FOREMAN__DRIVERS_ENABLED=false`) nothing touches the
//! network: [`mock_scan`] returns sample printers and probes answer from the same set.

use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use foreman_core::{
    domain::{FirmwareKind, Printer},
    drivers::bambu,
};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpStream, sync::Semaphore};
use tracing::{debug, info};
use ts_rs::TS;

/// What a found printer still needs before it can be added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum Readiness {
    /// Answers and needs nothing else.
    Ready,
    /// Moonraker refused us without an API key.
    ApiKey,
    /// Bambu: the 8-digit LAN access code from the printer's screen.
    AccessCode,
    /// Did not answer the probe.
    Unreachable,
}

/// One line of "what was probed" shown under a printer.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Check {
    pub label: String,
    pub ok: bool,
    pub detail: String,
}

impl Check {
    fn ok(label: &str, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ok: true,
            detail: detail.into(),
        }
    }
    fn bad(label: &str, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ok: false,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct FoundPrinter {
    /// Stable key for the UI (`<firmware>:<host>`).
    pub key: String,
    pub firmware: FirmwareKind,
    /// Suggested name (Klipper hostname, Bambu device name, else the address).
    pub name: String,
    pub host: String,
    /// What to store as the printer's address (`http://host[:port]` or the Bambu IP).
    pub url: String,
    pub serial: Option<String>,
    pub model: Option<String>,
    pub version: Option<String>,
    pub needs: Readiness,
    /// How it was found: `mdns`, `sweep`, `ssdp`, `tls`, `sample`.
    pub via: Vec<String>,
    pub checks: Vec<Check>,
    /// Already a printer on this server (its id).
    #[ts(type = "number | null")]
    pub added_as: Option<i64>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ScanResult {
    pub printers: Vec<FoundPrinter>,
    /// Things the user should know (blocked port, sample data, nothing found).
    pub notes: Vec<String>,
    pub sample: bool,
    #[ts(type = "number")]
    pub took_ms: u64,
    pub scanned_at: String,
}

/// What the user typed (manual add) or picked (credentials for a found printer).
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct ProbeRequest {
    pub firmware: FirmwareKind,
    /// IP, hostname or URL.
    pub host: String,
    #[serde(default)]
    #[ts(optional)]
    pub api_key: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub serial: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub access_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ProbeResult {
    /// Everything checked passed: the printer can be added as entered.
    pub ok: bool,
    pub printer: FoundPrinter,
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

/// `10.0.0.177`, `HellV0.local:7125`, `http://x/` → (host, port, scheme).
pub fn split_host(input: &str) -> Option<(String, Option<u16>, Option<String>)> {
    let t = input.trim().trim_end_matches('/');
    if t.is_empty() {
        return None;
    }
    let (scheme, rest) = match t.split_once("://") {
        Some((s, r)) => (Some(s.to_ascii_lowercase()), r),
        None => (None, t),
    };
    let rest = rest.split('/').next().unwrap_or(rest);
    if rest.is_empty() || rest.contains(char::is_whitespace) {
        return None;
    }
    if let Some((h, p)) = rest.rsplit_once(':') {
        if !h.contains(':') {
            let port = p.parse::<u16>().ok()?;
            return Some((h.to_owned(), Some(port), scheme));
        }
    }
    Some((rest.to_owned(), None, scheme))
}

/// Moonraker base URL: keep the scheme and port the user gave.
pub fn moonraker_url(host: &str, port: Option<u16>, scheme: Option<&str>) -> String {
    let scheme = scheme.unwrap_or("http");
    match port {
        Some(80) if scheme == "http" => format!("http://{host}"),
        Some(443) if scheme == "https" => format!("https://{host}"),
        Some(p) => format!("{scheme}://{host}:{p}"),
        None => format!("{scheme}://{host}"),
    }
}

/// Bambu serials are 15 upper-case letters/digits (e.g. `03919C461200718`).
pub fn valid_bambu_serial(s: &str) -> bool {
    s.len() == 15
        && s.chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
}

/// The LAN access code shown on the printer: 8 characters, letters or digits.
pub fn valid_access_code(s: &str) -> bool {
    s.len() == 8 && s.chars().all(|c| c.is_ascii_alphanumeric())
}

/// SSDP `DevModel.bambu.com` codes → model names.
pub fn bambu_model_code(code: &str) -> Option<&'static str> {
    Some(match code {
        "3DPrinter-X1-Carbon" | "BL-P001" => "X1 Carbon",
        "3DPrinter-X1" | "BL-P002" => "X1",
        "C13" => "X1E",
        "C11" => "P1P",
        "C12" => "P1S",
        "N1" => "A1 mini",
        "N2S" => "A1",
        "O1D" => "H2D",
        _ => return None,
    })
}

/// A Bambu SSDP announcement (NOTIFY or M-SEARCH reply).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsdpBambu {
    pub ip: String,
    pub serial: String,
    pub name: Option<String>,
    pub model: Option<String>,
    /// `lan` (LAN only mode) or `cloud`.
    pub connect: Option<String>,
}

pub fn parse_ssdp(msg: &str) -> Option<SsdpBambu> {
    let mut h: BTreeMap<String, String> = BTreeMap::new();
    for line in msg.lines() {
        if let Some((k, v)) = line.split_once(':') {
            h.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    let nt = h.get("nt").or_else(|| h.get("st"))?;
    if !nt.contains("bambulab") {
        return None;
    }
    let ip = h.get("location")?.clone();
    let serial = h.get("usn")?.clone();
    Some(SsdpBambu {
        ip,
        serial,
        name: h.get("devname.bambu.com").cloned(),
        model: h.get("devmodel.bambu.com").map(|c| {
            bambu_model_code(c)
                .map(str::to_owned)
                .unwrap_or_else(|| c.clone())
        }),
        connect: h.get("devconnect.bambu.com").cloned(),
    })
}

/// Host part of a stored printer address.
fn stored_host(p: &Printer) -> Option<String> {
    p.primary_url
        .as_deref()
        .and_then(split_host)
        .map(|(h, _, _)| h.to_ascii_lowercase())
}

/// Mark found printers that are already on this server.
pub fn mark_added(found: &mut [FoundPrinter], existing: &[Printer]) {
    for f in found.iter_mut() {
        f.added_as = existing
            .iter()
            .find(|p| {
                p.firmware == f.firmware
                    && (stored_host(p).as_deref() == Some(&f.host.to_ascii_lowercase())
                        || (f.serial.is_some() && p.serial_number == f.serial))
            })
            .map(|p| p.id);
    }
}

// ---------------------------------------------------------------------------
// Sample printers (printer connections off)
// ---------------------------------------------------------------------------

pub const SAMPLE_NOTE: &str = "Printer connections are off (FOREMAN__DRIVERS_ENABLED=false): \
     these are sample printers, not a real scan. Adding one stores it without connecting.";

fn sample_printers() -> Vec<FoundPrinter> {
    vec![
        FoundPrinter {
            key: "moonraker:192.0.2.10".into(),
            firmware: FirmwareKind::Moonraker,
            name: "voron24".into(),
            host: "192.0.2.10".into(),
            url: "http://192.0.2.10".into(),
            serial: None,
            model: Some("Klipper".into()),
            version: Some("v0.9.3 (sample)".into()),
            needs: Readiness::Ready,
            via: vec!["sample".into()],
            checks: vec![Check::ok("Moonraker", "answers /server/info (sample)")],
            added_as: None,
        },
        FoundPrinter {
            key: "moonraker:192.0.2.11".into(),
            firmware: FirmwareKind::Moonraker,
            name: "trident".into(),
            host: "192.0.2.11".into(),
            url: "http://192.0.2.11:7125".into(),
            serial: None,
            model: Some("Klipper".into()),
            version: None,
            needs: Readiness::ApiKey,
            via: vec!["sample".into()],
            checks: vec![Check::bad(
                "Moonraker",
                "401: this Moonraker wants an API key (sample)",
            )],
            added_as: None,
        },
        FoundPrinter {
            key: "bambu:192.0.2.20".into(),
            firmware: FirmwareKind::Bambu,
            name: "Bambu A1".into(),
            host: "192.0.2.20".into(),
            url: "192.0.2.20".into(),
            serial: Some("03919C000000001".into()),
            model: Some("A1".into()),
            version: None,
            needs: Readiness::AccessCode,
            via: vec!["sample".into()],
            checks: vec![Check::ok("Certificate", "serial 03919C000000001 (sample)")],
            added_as: None,
        },
    ]
}

pub fn mock_scan(existing: &[Printer]) -> ScanResult {
    let mut printers = sample_printers();
    mark_added(&mut printers, existing);
    ScanResult {
        printers,
        notes: vec![SAMPLE_NOTE.into()],
        sample: true,
        took_ms: 0,
        scanned_at: chrono::Utc::now().to_rfc3339(),
    }
}

/// Probe against the sample set: the API-key printer wants `sample-key`, the Bambu
/// any valid 8-character code. Any other host is "not probed".
pub fn mock_probe(req: &ProbeRequest) -> ProbeResult {
    let host = split_host(&req.host)
        .map(|(h, _, _)| h)
        .unwrap_or_else(|| req.host.trim().to_owned());
    let sample = sample_printers()
        .into_iter()
        .find(|p| p.host == host && p.firmware == req.firmware);
    let Some(mut p) = sample else {
        let mut p = manual_shell(req, &host);
        p.checks.push(Check::ok(
            "Not probed",
            "printer connections are off; stored without connecting",
        ));
        p.needs = Readiness::Ready;
        return ProbeResult {
            ok: true,
            printer: p,
        };
    };
    match p.needs {
        Readiness::ApiKey => {
            if req.api_key.as_deref() == Some("sample-key") {
                p.needs = Readiness::Ready;
                p.checks = vec![Check::ok("API key", "accepted (sample)")];
            } else if req.api_key.as_deref().is_some_and(|k| !k.trim().is_empty()) {
                p.checks = vec![Check::bad(
                    "API key",
                    "401: key refused (sample wants sample-key)",
                )];
            }
        }
        Readiness::AccessCode => {
            if let Some(code) = req.access_code.as_deref() {
                if valid_access_code(code) {
                    p.needs = Readiness::Ready;
                    p.checks
                        .push(Check::ok("Access code", "signed in to FTPS (sample)"));
                } else {
                    p.checks.push(Check::bad(
                        "Access code",
                        "8 letters or digits, from the printer's screen",
                    ));
                }
            }
        }
        _ => {}
    }
    ProbeResult {
        ok: p.needs == Readiness::Ready,
        printer: p,
    }
}

fn manual_shell(req: &ProbeRequest, host: &str) -> FoundPrinter {
    let fw = format!("{:?}", req.firmware).to_ascii_lowercase();
    FoundPrinter {
        key: format!("{fw}:{host}"),
        firmware: req.firmware.clone(),
        name: host.to_owned(),
        host: host.to_owned(),
        url: host.to_owned(),
        serial: req.serial.clone().filter(|s| !s.is_empty()),
        model: None,
        version: None,
        needs: Readiness::Unreachable,
        via: vec!["manual".into()],
        checks: Vec::new(),
        added_as: None,
    }
}

// ---------------------------------------------------------------------------
// Probes
// ---------------------------------------------------------------------------

const HTTP_TIMEOUT: Duration = Duration::from_millis(2500);
const TLS_TIMEOUT: Duration = Duration::from_millis(2500);
const FTPS_TIMEOUT: Duration = Duration::from_secs(8);

/// Probe a Moonraker at `base` (`http://host[:port]`).
async fn probe_moonraker_at(
    http: &reqwest::Client,
    base: &str,
    api_key: Option<&str>,
) -> Result<(Readiness, Vec<Check>, Option<String>, Option<String>), String> {
    let mut req = http
        .get(format!("{base}/server/info"))
        .timeout(HTTP_TIMEOUT);
    if let Some(k) = api_key.filter(|k| !k.trim().is_empty()) {
        req = req.header("X-Api-Key", k.trim());
    }
    let resp = req.send().await.map_err(|e| short_err(&e))?;
    let status = resp.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        // Moonraker refuses with a JSON-RPC style error; routers and NAS pages don't.
        let text = resp.text().await.unwrap_or_default();
        if !is_moonraker_refusal(&text) {
            return Err(format!(
                "/server/info answered {status} and is not Moonraker"
            ));
        }
        let detail = if api_key.is_some_and(|k| !k.trim().is_empty()) {
            format!("{status}: the API key was refused")
        } else {
            format!("{status}: this Moonraker wants an API key")
        };
        return Ok((
            Readiness::ApiKey,
            vec![Check::bad("Moonraker", detail)],
            None,
            None,
        ));
    }
    if !status.is_success() {
        return Err(format!("/server/info answered {status}"));
    }
    let body: serde_json::Value =
        serde_json::from_str(&resp.text().await.map_err(|e| short_err(&e))?)
            .map_err(|_| "not Moonraker (no JSON from /server/info)".to_string())?;
    let r = body
        .get("result")
        .ok_or_else(|| "not Moonraker (no result in /server/info)".to_string())?;
    let version = r
        .get("moonraker_version")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let klippy = r
        .get("klippy_state")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_owned();
    let mut checks = vec![Check::ok(
        "Moonraker",
        format!(
            "{} answers /server/info",
            version.as_deref().unwrap_or("Moonraker")
        ),
    )];
    checks.push(if klippy == "ready" {
        Check::ok("Klipper", "ready")
    } else {
        Check::bad("Klipper", format!("state {klippy} (it can still be added)"))
    });
    if api_key.is_some_and(|k| !k.trim().is_empty()) {
        checks.push(Check::ok("API key", "accepted"));
    }
    // Klipper's hostname makes a better default name than the IP.
    let mut name = None;
    let mut info = http
        .get(format!("{base}/printer/info"))
        .timeout(HTTP_TIMEOUT);
    if let Some(k) = api_key.filter(|k| !k.trim().is_empty()) {
        info = info.header("X-Api-Key", k.trim());
    }
    if let Ok(resp) = info.send().await {
        if let Ok(t) = resp.text().await {
            name = serde_json::from_str::<serde_json::Value>(&t)
                .ok()
                .and_then(|v| v["result"]["hostname"].as_str().map(str::to_owned))
                .filter(|s| !s.is_empty());
        }
    }
    Ok((Readiness::Ready, checks, version, name))
}

/// Moonraker's 401/403 body: `{"error": {"code": 401, "message": "Unauthorized"}}`.
pub fn is_moonraker_refusal(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("code"))
                .and_then(|c| c.as_i64())
        })
        .is_some_and(|c| c == 401 || c == 403)
}

fn short_err(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "timed out".into()
    } else if e.is_connect() {
        "connection refused or unreachable".into()
    } else {
        e.to_string()
    }
}

/// Moonraker on a host: the given port, else 80 (nginx, what Mainsail/Fluidd use) then 7125.
async fn probe_moonraker_host(
    http: &reqwest::Client,
    host: &str,
    port: Option<u16>,
    scheme: Option<&str>,
    api_key: Option<&str>,
) -> FoundPrinter {
    let ports: Vec<Option<u16>> = match port {
        Some(p) => vec![Some(p)],
        None => vec![None, Some(7125)],
    };
    let mut fails = Vec::new();
    for p in ports {
        let base = moonraker_url(host, p, scheme);
        match probe_moonraker_at(http, &base, api_key).await {
            Ok((needs, checks, version, name)) => {
                return FoundPrinter {
                    key: format!("moonraker:{host}"),
                    firmware: FirmwareKind::Moonraker,
                    name: name.unwrap_or_else(|| host.to_owned()),
                    host: host.to_owned(),
                    url: base,
                    serial: None,
                    model: Some("Klipper".into()),
                    version,
                    needs,
                    via: Vec::new(),
                    checks,
                    added_as: None,
                }
            }
            Err(e) => fails.push(Check::bad("Moonraker", format!("{base}: {e}"))),
        }
    }
    FoundPrinter {
        key: format!("moonraker:{host}"),
        firmware: FirmwareKind::Moonraker,
        name: host.to_owned(),
        host: host.to_owned(),
        url: moonraker_url(host, port, scheme),
        serial: None,
        model: None,
        version: None,
        needs: Readiness::Unreachable,
        via: Vec::new(),
        checks: fails,
        added_as: None,
    }
}

/// Bambu on a host: certificate (serial, model), then the access code if given.
async fn probe_bambu_host(
    host: &str,
    serial: Option<&str>,
    access_code: Option<&str>,
    ssdp: Option<&SsdpBambu>,
) -> FoundPrinter {
    let mut checks = Vec::new();
    let mut found_serial = ssdp.map(|s| s.serial.clone());
    let mut reachable = ssdp.is_some();
    match bambu::probe_tls_identity(host, TLS_TIMEOUT).await {
        Ok(id) => match id.bambu_serial() {
            Some(s) => {
                reachable = true;
                checks.push(Check::ok(
                    "Certificate",
                    format!(
                        "port 8883, issued by {}, serial {s}",
                        id.issuer_cn.as_deref().unwrap_or("?")
                    ),
                ));
                found_serial = Some(s.to_owned());
            }
            None => checks.push(Check::bad(
                "Certificate",
                format!(
                    "port 8883 is not a Bambu printer (issuer {})",
                    id.issuer_cn.as_deref().unwrap_or("unknown")
                ),
            )),
        },
        Err(e) => checks.push(Check::bad("Certificate", format!("port 8883: {e}"))),
    }
    if let (Some(given), Some(seen)) = (serial.filter(|s| !s.is_empty()), found_serial.as_deref()) {
        if given != seen {
            checks.push(Check::bad(
                "Serial",
                format!("you entered {given}, the printer says {seen}"),
            ));
        }
    }
    let model = ssdp.and_then(|s| s.model.clone()).or_else(|| {
        found_serial
            .as_deref()
            .and_then(bambu::model_from_serial)
            .map(str::to_owned)
    });
    if let Some(c) = ssdp.and_then(|s| s.connect.as_deref()) {
        checks.push(Check::ok(
            "Mode",
            if c == "lan" {
                "LAN only"
            } else {
                "cloud (LAN access still uses the access code)"
            },
        ));
    }
    let mut needs = if reachable {
        Readiness::AccessCode
    } else {
        Readiness::Unreachable
    };
    if let Some(code) = access_code.filter(|c| !c.is_empty()) {
        if !valid_access_code(code) {
            checks.push(Check::bad(
                "Access code",
                "8 letters or digits, from the printer's screen",
            ));
        } else if reachable {
            match bambu::check_access_code(host, code, FTPS_TIMEOUT).await {
                Ok(()) => {
                    needs = Readiness::Ready;
                    checks.push(Check::ok("Access code", "signed in to FTPS (port 990)"));
                }
                Err(e) => checks.push(Check::bad(
                    "Access code",
                    if e.to_string().contains("530") {
                        "the printer refused this access code (check it on the printer's screen)"
                            .to_string()
                    } else {
                        format!("FTPS sign-in failed: {e}")
                    },
                )),
            }
        }
    }
    let serial_mismatch = checks.iter().any(|c| c.label == "Serial" && !c.ok);
    if serial_mismatch && needs == Readiness::Ready {
        needs = Readiness::AccessCode;
    }
    FoundPrinter {
        key: format!("bambu:{host}"),
        firmware: FirmwareKind::Bambu,
        name: ssdp
            .and_then(|s| s.name.clone())
            .or_else(|| model.as_ref().map(|m| format!("Bambu {m}")))
            .unwrap_or_else(|| format!("Bambu {host}")),
        host: host.to_owned(),
        url: host.to_owned(),
        serial: found_serial.or_else(|| serial.map(str::to_owned)),
        model,
        version: None,
        needs,
        via: Vec::new(),
        checks,
        added_as: None,
    }
}

fn unsupported(fw: &FirmwareKind) -> String {
    format!("{fw:?} printers are not supported in setup yet; Klipper (Moonraker) and Bambu are.")
}

/// Probe one printer the user typed in or entered credentials for.
pub async fn probe(http: &reqwest::Client, req: &ProbeRequest) -> Result<ProbeResult, String> {
    let (host, port, scheme) = split_host(&req.host)
        .ok_or_else(|| "Enter the printer's IP address or hostname.".to_string())?;
    let printer = match &req.firmware {
        FirmwareKind::Moonraker => {
            probe_moonraker_host(http, &host, port, scheme.as_deref(), req.api_key.as_deref()).await
        }
        FirmwareKind::Bambu => {
            if let Some(s) = req.serial.as_deref().filter(|s| !s.is_empty()) {
                if !valid_bambu_serial(s) {
                    return Err(
                        "A Bambu serial is 15 capital letters and digits (Settings → Device on the printer)."
                            .into(),
                    );
                }
            }
            probe_bambu_host(
                &host,
                req.serial.as_deref(),
                req.access_code.as_deref(),
                None,
            )
            .await
        }
        other => return Err(unsupported(other)),
    };
    let mut printer = printer;
    printer.via = vec!["manual".into()];
    Ok(ProbeResult {
        ok: printer.needs == Readiness::Ready,
        printer,
    })
}

// ---------------------------------------------------------------------------
// Scan
// ---------------------------------------------------------------------------

/// mDNS service types worth asking about. Moonraker's own type first; the rest only
/// give us hosts to probe.
const MDNS_TYPES: &[&str] = &[
    "_moonraker._tcp.local.",
    "_ssh._tcp.local.",
    "_sftp-ssh._tcp.local.",
    "_workstation._tcp.local.",
    "_http._tcp.local.",
];

#[derive(Debug, Default)]
struct MdnsHits {
    /// ip → (moonraker port if advertised, a name)
    hosts: BTreeMap<Ipv4Addr, (Option<u16>, Option<String>)>,
}

fn mdns_browse(window: Duration) -> Result<MdnsHits, String> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};
    let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let mut rxs = Vec::new();
    for t in MDNS_TYPES {
        if let Ok(rx) = daemon.browse(t) {
            rxs.push((*t, rx));
        }
    }
    let deadline = Instant::now() + window;
    let mut hits = MdnsHits::default();
    while Instant::now() < deadline {
        let mut idle = true;
        for (t, rx) in &rxs {
            while let Ok(ev) = rx.try_recv() {
                idle = false;
                if let ServiceEvent::ServiceResolved(s) = ev {
                    let name = s
                        .get_hostname()
                        .trim_end_matches('.')
                        .trim_end_matches(".local")
                        .to_owned();
                    for a in s.get_addresses() {
                        if let IpAddr::V4(v4) = a.to_ip_addr() {
                            if !v4.is_private() || v4.is_loopback() {
                                continue;
                            }
                            let e = hits.hosts.entry(v4).or_insert((None, None));
                            if *t == "_moonraker._tcp.local." {
                                e.0 = Some(s.get_port());
                            }
                            if e.1.is_none() && !name.is_empty() {
                                e.1 = Some(name.clone());
                            }
                        }
                    }
                }
            }
        }
        if idle {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let _ = daemon.shutdown();
    Ok(hits)
}

/// This machine's private IPv4 /24 networks (LAN interfaces only).
fn local_nets() -> Vec<Ipv4Addr> {
    let mut nets: BTreeSet<Ipv4Addr> = BTreeSet::new();
    for (_, ip) in crate::net::interface_ips() {
        if ip.is_private() && !crate::net::is_tailscale(ip) {
            let o = ip.octets();
            nets.insert(Ipv4Addr::new(o[0], o[1], o[2], 0));
        }
    }
    nets.into_iter().collect()
}

/// TCP connect to `port` on every address of each /24.
async fn sweep(nets: &[Ipv4Addr], port: u16) -> BTreeSet<Ipv4Addr> {
    let sem = Arc::new(Semaphore::new(128));
    let mut tasks = Vec::new();
    for net in nets {
        let o = net.octets();
        for last in 1..=254u8 {
            let ip = Ipv4Addr::new(o[0], o[1], o[2], last);
            let sem = Arc::clone(&sem);
            tasks.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.ok()?;
                let addr = SocketAddr::new(IpAddr::V4(ip), port);
                match tokio::time::timeout(Duration::from_millis(600), TcpStream::connect(addr))
                    .await
                {
                    Ok(Ok(_)) => Some(ip),
                    _ => None,
                }
            }));
        }
    }
    let mut open = BTreeSet::new();
    for t in tasks {
        if let Ok(Some(ip)) = t.await {
            open.insert(ip);
        }
    }
    open
}

/// Listen for Bambu SSDP for `window`; also ask with an M-SEARCH.
async fn bambu_ssdp(window: Duration) -> Result<Vec<SsdpBambu>, String> {
    use socket2::{Domain, Protocol, Socket, Type};
    let sock =
        Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).map_err(|e| e.to_string())?;
    sock.set_reuse_address(true).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    sock.set_reuse_port(true).ok();
    sock.set_broadcast(true).map_err(|e| e.to_string())?;
    sock.bind(&SocketAddr::from(([0, 0, 0, 0], 2021)).into())
        .map_err(|e| format!("UDP 2021: {e}"))?;
    sock.join_multicast_v4(&Ipv4Addr::new(239, 255, 255, 250), &Ipv4Addr::UNSPECIFIED)
        .ok();
    sock.set_nonblocking(true).map_err(|e| e.to_string())?;
    let std_sock: std::net::UdpSocket = sock.into();
    let udp = tokio::net::UdpSocket::from_std(std_sock).map_err(|e| e.to_string())?;
    let search = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1990\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: urn:bambulab-com:device:3dprinter:1\r\n\r\n";
    let _ = udp.send_to(search.as_bytes(), "239.255.255.250:1990").await;
    let deadline = tokio::time::Instant::now() + window;
    let mut found: BTreeMap<String, SsdpBambu> = BTreeMap::new();
    let mut buf = [0u8; 2048];
    loop {
        match tokio::time::timeout_at(deadline, udp.recv_from(&mut buf)).await {
            Ok(Ok((n, _))) => {
                if let Some(b) = parse_ssdp(&String::from_utf8_lossy(&buf[..n])) {
                    found.insert(b.ip.clone(), b);
                }
            }
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => break,
        }
    }
    Ok(found.into_values().collect())
}

/// Scan the LAN for Moonraker and Bambu printers (about `window` + probe time).
pub async fn scan(http: &reqwest::Client, existing: &[Printer], window: Duration) -> ScanResult {
    let started = Instant::now();
    let nets = local_nets();
    let mut notes = Vec::new();
    let mdns = tokio::task::spawn_blocking(move || mdns_browse(window));
    let (mdns, swept_mr, swept_bb, ssdp) = tokio::join!(
        mdns,
        sweep(&nets, 7125),
        sweep(&nets, 8883),
        bambu_ssdp(window)
    );
    let mdns = match mdns {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => {
            notes.push(format!("mDNS browse failed: {e}"));
            MdnsHits::default()
        }
        Err(e) => {
            notes.push(format!("mDNS browse failed: {e}"));
            MdnsHits::default()
        }
    };
    let ssdp = match ssdp {
        Ok(v) => v,
        Err(e) => {
            notes.push(format!(
                "Bambu broadcast listener unavailable ({e}); Bambu printers are found by their certificate instead."
            ));
            Vec::new()
        }
    };
    debug!(
        mdns = mdns.hosts.len(),
        moonraker_port = swept_mr.len(),
        mqtt_port = swept_bb.len(),
        ssdp = ssdp.len(),
        "discovery candidates"
    );

    // Moonraker candidates: every mDNS host + hosts with 7125 open.
    let mut mr: BTreeMap<Ipv4Addr, (Option<u16>, Vec<String>)> = BTreeMap::new();
    for (ip, (port, _)) in &mdns.hosts {
        mr.entry(*ip)
            .or_insert((*port, Vec::new()))
            .1
            .push("mdns".into());
    }
    for ip in &swept_mr {
        mr.entry(*ip)
            .or_insert((None, Vec::new()))
            .1
            .push("sweep".into());
    }
    // Bambu candidates: SSDP + hosts with 8883 open.
    let mut bb: BTreeMap<String, (Option<SsdpBambu>, Vec<String>)> = BTreeMap::new();
    for s in ssdp {
        bb.entry(s.ip.clone())
            .or_insert((None, Vec::new()))
            .1
            .push("ssdp".into());
        if let Some(e) = bb.get_mut(&s.ip) {
            e.0 = Some(s);
        }
    }
    for ip in &swept_bb {
        bb.entry(ip.to_string())
            .or_insert((None, Vec::new()))
            .1
            .push("tls".into());
    }

    let sem = Arc::new(Semaphore::new(16));
    let mut tasks = Vec::new();
    for (ip, (port, via)) in mr {
        let http = http.clone();
        let sem = Arc::clone(&sem);
        let mdns_name = mdns.hosts.get(&ip).and_then(|(_, n)| n.clone());
        let host = ip.to_string();
        tasks.push(tokio::spawn(async move {
            let _p = sem.acquire().await.ok()?;
            let mut f = probe_moonraker_host(&http, &host, port, None, None).await;
            if f.needs == Readiness::Unreachable {
                return None; // an mDNS host that is not a printer
            }
            if f.name == host {
                if let Some(n) = mdns_name {
                    f.name = n;
                }
            }
            f.via = via;
            Some(f)
        }));
    }
    for (host, (ssdp, via)) in bb {
        let sem = Arc::clone(&sem);
        tasks.push(tokio::spawn(async move {
            let _p = sem.acquire().await.ok()?;
            let mut f = probe_bambu_host(&host, None, None, ssdp.as_ref()).await;
            // 8883 open but not a Bambu printer.
            f.serial.as_ref()?;
            f.via = via;
            Some(f)
        }));
    }
    let mut printers = Vec::new();
    for t in tasks {
        if let Ok(Some(f)) = t.await {
            printers.push(f);
        }
    }
    printers.sort_by_key(|a| a.name.to_ascii_lowercase());
    mark_added(&mut printers, existing);
    if printers.is_empty() {
        notes.push(
            "No printers answered. Add one by address below; a printer on another network or VLAN \
             will not show up in a scan."
                .into(),
        );
    }
    info!(
        found = printers.len(),
        took_ms = started.elapsed().as_millis() as u64,
        "printer discovery finished"
    );
    ScanResult {
        printers,
        notes,
        sample: false,
        took_ms: started.elapsed().as_millis() as u64,
        scanned_at: chrono::Utc::now().to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_host_forms() {
        assert_eq!(
            split_host("10.0.0.177"),
            Some(("10.0.0.177".into(), None, None))
        );
        assert_eq!(
            split_host("http://HellV0.local:7125/"),
            Some(("HellV0.local".into(), Some(7125), Some("http".into())))
        );
        assert_eq!(split_host("  "), None);
        assert_eq!(split_host("a b"), None);
        assert_eq!(split_host("host:notaport"), None);
    }

    #[test]
    fn moonraker_refusal_shape() {
        assert!(is_moonraker_refusal(
            r#"{"error": {"code": 401, "message": "Unauthorized"}}"#
        ));
        assert!(!is_moonraker_refusal(
            "<html><title>401 Unauthorized</title></html>"
        ));
        assert!(!is_moonraker_refusal(r#"{"detail": "Not authenticated"}"#));
    }

    #[test]
    fn moonraker_urls() {
        assert_eq!(moonraker_url("h", None, None), "http://h");
        assert_eq!(moonraker_url("h", Some(80), None), "http://h");
        assert_eq!(moonraker_url("h", Some(7125), None), "http://h:7125");
        assert_eq!(moonraker_url("h", Some(443), Some("https")), "https://h");
    }

    #[test]
    fn serial_and_code_rules() {
        assert!(valid_bambu_serial("03919C461200718"));
        assert!(!valid_bambu_serial("03919c461200718"));
        assert!(!valid_bambu_serial("0391"));
        assert!(valid_access_code("12ab34cd"));
        assert!(!valid_access_code("1234567"));
        assert!(!valid_access_code("1234-678"));
    }

    #[test]
    fn ssdp_notify_parses() {
        let msg = "NOTIFY * HTTP/1.1\r\nHOST: 239.255.255.250:1990\r\nServer: UPnP/1.0\r\nLocation: 10.0.0.249\r\nNT: urn:bambulab-com:device:3dprinter:1\r\nUSN: 03919C461200718\r\nCache-Control: max-age=1800\r\nDevModel.bambu.com: N2S\r\nDevName.bambu.com: bambu\r\nDevConnect.bambu.com: cloud\r\nDevBind.bambu.com: occupied\r\n\r\n";
        let b = parse_ssdp(msg).expect("parsed");
        assert_eq!(b.ip, "10.0.0.249");
        assert_eq!(b.serial, "03919C461200718");
        assert_eq!(b.model.as_deref(), Some("A1"));
        assert_eq!(b.name.as_deref(), Some("bambu"));
        assert_eq!(b.connect.as_deref(), Some("cloud"));
        assert!(parse_ssdp(
            "NOTIFY * HTTP/1.1\r\nNT: upnp:rootdevice\r\nLocation: x\r\nUSN: y\r\n"
        )
        .is_none());
    }

    #[test]
    fn der_cn_issuer_then_subject() {
        // Minimal fragments: issuer CN "BBL CA", then subject CN = serial.
        let mut der = vec![0x30, 0x00];
        der.extend([0x06, 0x03, 0x55, 0x04, 0x03, 0x0C, 6]);
        der.extend(b"BBL CA");
        der.extend([0x30, 0x00, 0x06, 0x03, 0x55, 0x04, 0x03, 0x13, 15]);
        der.extend(b"03919C461200718");
        let cns = bambu::der_common_names(&der);
        assert_eq!(
            cns,
            vec!["BBL CA".to_string(), "03919C461200718".to_string()]
        );
        let id = bambu::TlsIdentity {
            issuer_cn: cns.first().cloned(),
            subject_cn: cns.get(1).cloned(),
        };
        assert_eq!(id.bambu_serial(), Some("03919C461200718"));
        let other = bambu::TlsIdentity {
            issuer_cn: Some("Let's Encrypt".into()),
            subject_cn: Some("example.com".into()),
        };
        assert_eq!(other.bambu_serial(), None);
    }

    #[test]
    fn mock_probe_rules() {
        let key = ProbeRequest {
            firmware: FirmwareKind::Moonraker,
            host: "192.0.2.11".into(),
            api_key: Some("wrong".into()),
            serial: None,
            access_code: None,
        };
        assert!(!mock_probe(&key).ok);
        let key = ProbeRequest {
            api_key: Some("sample-key".into()),
            ..key
        };
        assert!(mock_probe(&key).ok);
        let bambu = ProbeRequest {
            firmware: FirmwareKind::Bambu,
            host: "192.0.2.20".into(),
            access_code: Some("123".into()),
            api_key: None,
            serial: None,
        };
        assert!(!mock_probe(&bambu).ok);
        let bambu = ProbeRequest {
            access_code: Some("12345678".into()),
            ..bambu
        };
        assert!(mock_probe(&bambu).ok);
    }
}
