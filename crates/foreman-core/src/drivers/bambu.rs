//! Bambu Lab LAN MQTT driver.
//!
//! Connects via MQTT over TLS (port 8883), user `bblp`, password = LAN access
//! code.  Subscribes to `device/<serial>/report`, sends `pushall` on connect,
//! and merges partial status reports into a full `PrinterStatus`.
//!
//! TLS: TOFU cert pinning. First connect stores the cert SHA-256 fingerprint
//! in the DB (printer row); later connects must match; on mismatch the driver
//! refuses and publishes `printer.connection` with reason "certificate changed".
//!
//! FTPS: implicit TLS on port 990, same cert pinning. File list/upload/delete
//! on the SD card.
//!
//! Camera: A1/P1 JPEG stream discovery — logged for proxy wiring.
//!
//! Reference sources (read, not copied):
//! - OpenBambuAPI: github.com/Doridian/OpenBambuAPI/blob/main/mqtt.md
//! - Bambu wiki: wiki.bambulab.com/en/knowledge-sharing/developer-resources
//! - ha-bambulab: github.com/greghesp/ha-bambulab (Home Assistant integration)

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, TlsConfiguration, Transport};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tokio::sync::{broadcast, watch, Mutex, RwLock};
use tracing::{debug, error, info, warn};

use crate::driver::{PrinterCapabilities, PrinterCommand, PrinterDriver, PrinterFile};
use crate::error::AppError;
use crate::events::{
    AmsTray, ConnectionEvent, ConnectionEventKind, FanSpeeds, HmsCode, PrinterState, PrinterStatus,
    ServerEvent, TempReading,
};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Configuration for a Bambu LAN driver instance.
#[derive(Debug, Clone)]
pub struct BambuConfig {
    /// Printer database ID.
    pub printer_id: i64,
    /// Printer IP address or hostname.
    pub host: String,
    /// Device serial number (topic prefix).
    pub serial: String,
    /// LAN access code (8-digit).
    pub access_code: String,
    /// Stored TLS cert fingerprint (None = first connect, TOFU pin).
    pub tls_fingerprint: Option<String>,
    /// Database pool for persisting the fingerprint on first connect.
    pub pool: SqlitePool,
}

// ---------------------------------------------------------------------------
// Serial → model mapping
// ---------------------------------------------------------------------------

/// Derive printer model from serial number prefix.
fn model_from_serial(serial: &str) -> Option<&'static str> {
    if serial.len() < 3 {
        return None;
    }
    match &serial[..3] {
        "00M" => Some("X1 Carbon"),
        "00W" => Some("A1"),
        "01P" => Some("P1P"),
        "01S" => Some("A1 Mini"),
        "03P" => Some("P1S"),
        "030" => Some("X1E"),
        "039" => Some("A1"), // newer A1 serial prefix
        _ => None,
    }
}

/// Derive bed size from model.
fn bed_size_from_model(model: &str) -> Option<(f64, f64)> {
    match model {
        "X1 Carbon" | "X1E" | "P1P" | "P1S" => Some((256.0, 256.0)),
        "A1" => Some((256.0, 256.0)),
        "A1 Mini" => Some((180.0, 180.0)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Fan speed normalisation
// ---------------------------------------------------------------------------

/// Bambu reports fan speeds as string values 0–15, mapping to 0–100%.
fn parse_fan_percent(val: &Value) -> Option<u8> {
    let raw = match val {
        Value::String(s) => s.parse::<f64>().ok()?,
        Value::Number(n) => n.as_f64()?,
        _ => return None,
    };
    // Map 0–15 → 0–100%.
    let pct = ((raw / 15.0) * 100.0).round().clamp(0.0, 100.0) as u8;
    Some(pct)
}

// ---------------------------------------------------------------------------
// Cached state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct CachedStatus {
    state: PrinterState,
    nozzle_actual: f64,
    nozzle_target: f64,
    bed_actual: f64,
    bed_target: f64,
    progress: f64,
    filename: Option<String>,
    layer: Option<u32>,
    total_layers: Option<u32>,
    remaining_time: Option<u32>,
    fans: FanSpeeds,
    ams_trays: Vec<AmsTray>,
    hms_codes: Vec<HmsCode>,
    // Capabilities extracted from reports.
    nozzle_diameter: Option<f64>,
    has_camera: Option<bool>,
    has_xcam: bool,
    ams_count: u32,
    // Camera info extracted from ipcam report.
    camera_resolution: Option<String>,
}

impl CachedStatus {
    fn to_printer_status(&self) -> PrinterStatus {
        PrinterStatus {
            state: self.state.clone(),
            nozzle: Some(TempReading {
                actual: self.nozzle_actual,
                target: self.nozzle_target,
            }),
            bed: Some(TempReading {
                actual: self.bed_actual,
                target: self.bed_target,
            }),
            progress: Some(self.progress),
            filename: self.filename.clone(),
            print_duration: None,
            display_message: None,
            layer: self.layer,
            total_layers: self.total_layers,
            remaining_time: self.remaining_time,
            fans: Some(self.fans.clone()),
            ams_trays: self.ams_trays.clone(),
            hms_codes: self.hms_codes.clone(),
        }
    }

    /// Merge a partial `print` report payload into this cached status.
    fn merge_report(&mut self, print: &Value) {
        // HMS error codes — parse before gcode_state so FAILED can check them.
        if let Some(hms) = print.get("hms").and_then(|v| v.as_array()) {
            self.hms_codes = hms
                .iter()
                .filter_map(|entry| {
                    let attr = entry.get("attr").and_then(|v| v.as_u64())? as u32;
                    let code = entry.get("code").and_then(|v| v.as_u64())? as u32;
                    Some(HmsCode { attr, code })
                })
                .collect();
        }

        // State.  "FAILED" is the stale gcode_state after a print failure;
        // it does NOT mean the printer is currently faulted.  Only treat it
        // as Error when there are active HMS codes.
        if let Some(s) = print.get("gcode_state").and_then(|v| v.as_str()) {
            self.state = match s {
                "RUNNING" | "PREPARE" => PrinterState::Printing,
                "PAUSE" => PrinterState::Paused,
                "FINISH" | "IDLE" | "CREATED" | "SLICING" => PrinterState::Idle,
                "FAILED" if self.hms_codes.is_empty() => PrinterState::Idle,
                "FAILED" => PrinterState::Error,
                _ => self.state.clone(),
            };
        }

        // Temperatures.
        if let Some(t) = print.get("nozzle_temper").and_then(|v| v.as_f64()) {
            self.nozzle_actual = t;
        }
        if let Some(t) = print.get("nozzle_target_temper").and_then(|v| v.as_f64()) {
            self.nozzle_target = t;
        }
        if let Some(t) = print.get("bed_temper").and_then(|v| v.as_f64()) {
            self.bed_actual = t;
        }
        if let Some(t) = print.get("bed_target_temper").and_then(|v| v.as_f64()) {
            self.bed_target = t;
        }

        // Progress.
        if let Some(p) = print.get("mc_percent").and_then(|v| v.as_u64()) {
            self.progress = p as f64 / 100.0;
        }

        // File.
        if let Some(f) = print.get("gcode_file").and_then(|v| v.as_str()) {
            self.filename = if f.is_empty() {
                None
            } else {
                Some(f.to_owned())
            };
        } else if let Some(f) = print.get("subtask_name").and_then(|v| v.as_str()) {
            if !f.is_empty() {
                self.filename = Some(f.to_owned());
            }
        }

        // Layers.
        if let Some(l) = print.get("layer_num").and_then(|v| v.as_u64()) {
            self.layer = Some(l as u32);
        }
        if let Some(l) = print.get("total_layer_num").and_then(|v| v.as_u64()) {
            self.total_layers = Some(l as u32);
        }

        // Remaining time.
        if let Some(t) = print.get("mc_remaining_time").and_then(|v| v.as_u64()) {
            self.remaining_time = Some(t as u32);
        }

        // Fans.
        if let Some(v) = print.get("cooling_fan_speed") {
            self.fans.part_fan = parse_fan_percent(v);
        }
        if let Some(v) = print.get("big_fan1_speed") {
            self.fans.aux_fan = parse_fan_percent(v);
        }
        if let Some(v) = print.get("big_fan2_speed") {
            self.fans.chamber_fan = parse_fan_percent(v);
        }
        if let Some(v) = print.get("heatbreak_fan_speed") {
            self.fans.heatbreak_fan = parse_fan_percent(v);
        }

        // AMS.
        if let Some(ams) = print.get("ams") {
            self.merge_ams(ams);
        }

        // (HMS error codes are parsed at the top of merge_report.)

        // Nozzle diameter.
        if let Some(d) = print.get("nozzle_diameter").and_then(|v| v.as_str()) {
            if let Ok(f) = d.parse::<f64>() {
                self.nozzle_diameter = Some(f);
            }
        }

        // Camera detection.
        if let Some(ipcam) = print.get("ipcam") {
            if let Some(dev) = ipcam.get("ipcam_dev").and_then(|v| v.as_str()) {
                self.has_camera = Some(dev == "1");
            }
            if let Some(res) = ipcam.get("resolution").and_then(|v| v.as_str()) {
                self.camera_resolution = Some(res.to_owned());
            }
        }
        if let Some(xcam) = print.get("xcam") {
            if xcam.is_object() && xcam.as_object().is_some_and(|m| !m.is_empty()) {
                self.has_xcam = true;
            }
        }
    }

    fn merge_ams(&mut self, ams: &Value) {
        // Parse AMS unit array.
        let units = match ams.get("ams").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => return,
        };

        let mut trays = Vec::new();
        let mut unit_count: u32 = 0;

        for unit in units {
            let unit_id: u32 = unit
                .get("id")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            unit_count = unit_count.max(unit_id + 1);

            if let Some(tray_arr) = unit.get("tray").and_then(|v| v.as_array()) {
                for tray in tray_arr {
                    let tray_id: u32 = tray
                        .get("id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);

                    let slot = unit_id * 4 + tray_id;

                    // If a tray has only "id" and nothing else, it's empty — skip.
                    let tray_type = tray.get("tray_type").and_then(|v| v.as_str());
                    if tray_type.is_none() || tray_type == Some("") {
                        continue;
                    }

                    let color = tray.get("tray_color").and_then(|v| v.as_str()).map(|c| {
                        // Strip alpha if present (RRGGBBAA → RRGGBB).
                        if c.len() == 8 {
                            c[..6].to_owned()
                        } else {
                            c.to_owned()
                        }
                    });

                    let material = tray_type.map(|s| s.to_owned());

                    let remaining = tray
                        .get("remain")
                        .and_then(|v| v.as_u64())
                        .map(|r| r.min(100) as u8);

                    trays.push(AmsTray {
                        slot,
                        color,
                        material,
                        remaining,
                    });
                }
            }
        }

        // Also parse vt_tray (external spool, slot 254) — check ams root level
        // first, then the top-level print object.
        if let Some(vt) = ams.get("vt_tray") {
            Self::push_vt_tray(&mut trays, vt);
        }

        self.ams_trays = trays;
        self.ams_count = unit_count;
    }

    fn push_vt_tray(trays: &mut Vec<AmsTray>, vt: &Value) {
        let tray_type = vt.get("tray_type").and_then(|v| v.as_str());
        if tray_type.is_none() || tray_type == Some("") {
            return;
        }
        let color = vt.get("tray_color").and_then(|v| v.as_str()).map(|c| {
            if c.len() == 8 {
                c[..6].to_owned()
            } else {
                c.to_owned()
            }
        });
        let material = tray_type.map(|s| s.to_owned());
        let remaining = vt
            .get("remain")
            .and_then(|v| v.as_u64())
            .map(|r| r.min(100) as u8);
        trays.push(AmsTray {
            slot: 254,
            color,
            material,
            remaining,
        });
    }
}

// ---------------------------------------------------------------------------
// TLS: TOFU cert pinning
// ---------------------------------------------------------------------------

/// Compute SHA-256 fingerprint of a DER-encoded certificate.
pub fn cert_fingerprint(der: &[u8]) -> String {
    let hash = Sha256::digest(der);
    hex::encode(hash)
}

/// A rustls certificate verifier that accepts any cert but captures its
/// SHA-256 fingerprint for TOFU verification after the handshake.
#[derive(Debug)]
struct TofuCertVerifier {
    captured: Arc<std::sync::Mutex<Option<String>>>,
}

impl rumqttc::tokio_rustls::rustls::client::danger::ServerCertVerifier for TofuCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rumqttc::tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rumqttc::tokio_rustls::rustls::pki_types::CertificateDer<'_>],
        _server_name: &rumqttc::tokio_rustls::rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rumqttc::tokio_rustls::rustls::pki_types::UnixTime,
    ) -> Result<
        rumqttc::tokio_rustls::rustls::client::danger::ServerCertVerified,
        rumqttc::tokio_rustls::rustls::Error,
    > {
        let fp = cert_fingerprint(end_entity.as_ref());
        if let Ok(mut guard) = self.captured.lock() {
            *guard = Some(fp);
        }
        Ok(rumqttc::tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rumqttc::tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _dss: &rumqttc::tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<
        rumqttc::tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        rumqttc::tokio_rustls::rustls::Error,
    > {
        Ok(rumqttc::tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rumqttc::tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _dss: &rumqttc::tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<
        rumqttc::tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        rumqttc::tokio_rustls::rustls::Error,
    > {
        Ok(rumqttc::tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rumqttc::tokio_rustls::rustls::SignatureScheme> {
        use rumqttc::tokio_rustls::rustls::SignatureScheme;
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}

/// Build a TLS config with TOFU cert capture.
/// Returns (TlsConfiguration, captured_fingerprint_handle).
fn build_tls_config() -> (TlsConfiguration, Arc<std::sync::Mutex<Option<String>>>) {
    use rumqttc::tokio_rustls::rustls::ClientConfig;

    let captured: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));

    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(TofuCertVerifier {
            captured: Arc::clone(&captured),
        }))
        .with_no_client_auth();

    (TlsConfiguration::Rustls(Arc::new(config)), captured)
}

// ---------------------------------------------------------------------------
// FTPS helpers (sync FTP in spawn_blocking — suppaftp async uses async-std)
// ---------------------------------------------------------------------------

/// Connect to the printer's implicit FTPS (port 990) using sync suppaftp
/// wrapped in spawn_blocking. Uses native-tls with danger_accept_invalid_certs
/// since Bambu printers use self-signed certs.
fn ftps_connect_sync(
    host: &str,
    access_code: &str,
) -> Result<suppaftp::NativeTlsFtpStream, AppError> {
    use suppaftp::{NativeTlsConnector, NativeTlsFtpStream};

    let connector = NativeTlsConnector::from(
        native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|e| AppError::Driver(format!("ftps tls: {e}")))?,
    );

    let addr = format!("{host}:990");
    let mut ftp = NativeTlsFtpStream::connect_secure_implicit(addr, connector, host)
        .map_err(|e| AppError::Driver(format!("ftps connect: {e}")))?;

    ftp.login("bblp", access_code)
        .map_err(|e| AppError::Driver(format!("ftps login: {e}")))?;

    Ok(ftp)
}

// ---------------------------------------------------------------------------
// BambuDriver
// ---------------------------------------------------------------------------

/// A Bambu Lab LAN MQTT printer driver.
pub struct BambuDriver {
    config: BambuConfig,
    bus_tx: Arc<broadcast::Sender<ServerEvent>>,
    status: Arc<RwLock<CachedStatus>>,
    capabilities: Arc<RwLock<Option<PrinterCapabilities>>>,
    shutdown_tx: watch::Sender<bool>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl BambuDriver {
    pub fn new(config: BambuConfig, bus_tx: Arc<broadcast::Sender<ServerEvent>>) -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            config,
            bus_tx,
            status: Arc::new(RwLock::new(CachedStatus::default())),
            capabilities: Arc::new(RwLock::new(None)),
            shutdown_tx,
            task: Mutex::new(None),
        }
    }

    fn spawn_loop(&self) -> tokio::task::JoinHandle<()> {
        let printer_id = self.config.printer_id;
        let host = self.config.host.clone();
        let serial = self.config.serial.clone();
        let access_code = self.config.access_code.clone();
        let tls_fingerprint = self.config.tls_fingerprint.clone();
        let pool = self.config.pool.clone();
        let bus_tx = Arc::clone(&self.bus_tx);
        let status = Arc::clone(&self.status);
        let capabilities = Arc::clone(&self.capabilities);
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(2);
            let max_backoff = Duration::from_secs(60);

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                info!(printer_id, host = %host, "bambu: connecting");

                match Self::run_connection(
                    printer_id,
                    &host,
                    &serial,
                    &access_code,
                    &tls_fingerprint,
                    &pool,
                    &bus_tx,
                    &status,
                    &capabilities,
                    &mut shutdown_rx,
                )
                .await
                {
                    Ok(()) => {
                        info!(printer_id, "bambu: disconnected cleanly");
                        break;
                    }
                    Err(e) => {
                        warn!(printer_id, "bambu: connection lost: {e}");
                        let _ = bus_tx.send(ServerEvent::PrinterConnection {
                            printer_id,
                            event: ConnectionEvent {
                                kind: ConnectionEventKind::Offline,
                                active_path: None,
                            },
                        });
                        {
                            let mut s = status.write().await;
                            s.state = PrinterState::Offline;
                        }
                        let _ = bus_tx.send(ServerEvent::PrinterStatus {
                            printer_id,
                            status: status.read().await.to_printer_status(),
                        });
                    }
                }

                if *shutdown_rx.borrow() {
                    break;
                }

                info!(
                    printer_id,
                    backoff_secs = backoff.as_secs(),
                    "bambu: reconnecting after backoff"
                );
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {},
                    _ = shutdown_rx.changed() => { break; }
                }
                backoff = (backoff * 2).min(max_backoff);
            }
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_connection(
        printer_id: i64,
        host: &str,
        serial: &str,
        access_code: &str,
        tls_fingerprint: &Option<String>,
        pool: &SqlitePool,
        bus_tx: &broadcast::Sender<ServerEvent>,
        status: &RwLock<CachedStatus>,
        capabilities: &RwLock<Option<PrinterCapabilities>>,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) -> Result<(), AppError> {
        let report_topic = format!("device/{serial}/report");
        let request_topic = format!("device/{serial}/request");

        // Build MQTT options with TOFU cert capture.
        let client_id = format!("foreman-{printer_id}-{}", uuid::Uuid::new_v4().simple());
        let mut opts = MqttOptions::new(&client_id, host, 8883);
        opts.set_credentials("bblp", access_code);
        opts.set_keep_alive(Duration::from_secs(30));

        let (tls_config, captured_fp) = build_tls_config();
        opts.set_transport(Transport::tls_with_config(tls_config));

        let (client, mut eventloop) = AsyncClient::new(opts, 64);

        // Subscribe to report topic.
        client
            .subscribe(&report_topic, QoS::AtMostOnce)
            .await
            .map_err(|e| AppError::Driver(format!("mqtt subscribe: {e}")))?;

        // Drive the event loop briefly to complete the TLS handshake + CONNACK.
        // We need to poll until we see the SubAck (which means TLS + MQTT are done).
        let handshake_deadline = Instant::now() + Duration::from_secs(10);
        let mut connected = false;
        while Instant::now() < handshake_deadline {
            tokio::select! {
                event = eventloop.poll() => {
                    let event = event
                        .map_err(|e| AppError::Driver(format!("mqtt handshake: {e}")))?;
                    if let Event::Incoming(Packet::SubAck(_)) = event {
                        connected = true;
                        break;
                    }
                }
                _ = shutdown_rx.changed() => {
                    let _ = client.disconnect().await;
                    return Ok(());
                }
            }
        }
        if !connected {
            return Err(AppError::Driver(
                "mqtt handshake timeout: no SubAck received".into(),
            ));
        }

        // --- TOFU fingerprint check ---
        let observed_fp = captured_fp
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .ok_or_else(|| {
                AppError::Driver("TLS cert fingerprint not captured during handshake".into())
            })?;

        match tls_fingerprint {
            Some(stored) if stored != &observed_fp => {
                // Mismatch — refuse connection.
                error!(
                    printer_id,
                    stored = %stored,
                    observed = %observed_fp,
                    "bambu: TLS certificate changed! Refusing connection."
                );
                let _ = bus_tx.send(ServerEvent::PrinterConnection {
                    printer_id,
                    event: ConnectionEvent {
                        kind: ConnectionEventKind::Offline,
                        active_path: Some("certificate changed".to_owned()),
                    },
                });
                let _ = client.disconnect().await;
                return Err(AppError::Driver(
                    "TLS certificate fingerprint mismatch — certificate changed. \
                     Clear the pin via 'Trust new certificate' to reconnect."
                        .into(),
                ));
            }
            Some(_stored) => {
                info!(printer_id, "bambu: TLS cert fingerprint matches stored pin");
            }
            None => {
                // First connect — store the fingerprint (TOFU).
                info!(
                    printer_id,
                    fingerprint = %observed_fp,
                    "bambu: TOFU — pinning TLS cert fingerprint"
                );
                if let Err(e) =
                    crate::repo::printer::update_tls_fingerprint(pool, printer_id, &observed_fp)
                        .await
                {
                    warn!(printer_id, "bambu: failed to store TLS fingerprint: {e}");
                }
            }
        }

        // Publish online event.
        let _ = bus_tx.send(ServerEvent::PrinterConnection {
            printer_id,
            event: ConnectionEvent {
                kind: ConnectionEventKind::Online,
                active_path: None,
            },
        });

        // Send pushall to get full state dump.
        let pushall = serde_json::json!({
            "pushing": {
                "sequence_id": "0",
                "command": "pushall",
                "version": 1,
                "push_target": 1
            }
        });
        client
            .publish(
                &request_topic,
                QoS::AtMostOnce,
                false,
                serde_json::to_vec(&pushall).unwrap_or_default(),
            )
            .await
            .map_err(|e| AppError::Driver(format!("mqtt pushall: {e}")))?;

        // Init capabilities from serial.
        {
            let model = model_from_serial(serial);
            let mut caps = capabilities.write().await;
            let c = caps.get_or_insert_with(PrinterCapabilities::default);
            c.printer_model = model.map(|s| s.to_owned());
            if let Some(m) = model {
                c.bed_size = bed_size_from_model(m);
            }
            c.extruder_count = 1;
        }

        // Throttle: max 2 status events per second.
        let min_interval = Duration::from_millis(500);
        let mut last_publish = Instant::now() - min_interval;
        let mut camera_logged = false;

        // Event loop.
        loop {
            tokio::select! {
                event = eventloop.poll() => {
                    let event = event
                        .map_err(|e| AppError::Driver(format!("mqtt poll: {e}")))?;

                    if let Event::Incoming(Packet::Publish(publish)) = event {
                        if publish.topic == report_topic {
                            let payload: Value = match serde_json::from_slice(&publish.payload) {
                                Ok(v) => v,
                                Err(e) => {
                                    debug!(printer_id, "bambu: bad JSON: {e}");
                                    continue;
                                }
                            };

                            if let Some(print) = payload.get("print") {
                                let mut s = status.write().await;
                                s.merge_report(print);

                                // Log camera info once.
                                if !camera_logged {
                                    if let Some(has_cam) = s.has_camera {
                                        info!(
                                            printer_id,
                                            has_camera = has_cam,
                                            has_xcam = s.has_xcam,
                                            resolution = ?s.camera_resolution,
                                            "bambu: camera info from report"
                                        );
                                        camera_logged = true;
                                    }
                                }

                                drop(s);

                                // Throttled status publish.
                                let now = Instant::now();
                                if now.duration_since(last_publish) >= min_interval {
                                    last_publish = now;

                                    let s = status.read().await;
                                    let ps = s.to_printer_status();

                                    // Update capabilities from dynamic status fields.
                                    {
                                        let mut caps = capabilities.write().await;
                                        let c = caps.get_or_insert_with(PrinterCapabilities::default);
                                        if let Some(d) = s.nozzle_diameter {
                                            c.nozzle_diameter = Some(d);
                                        }
                                        if s.ams_count > 0 {
                                            c.ams_slot_count = Some(s.ams_count * 4);
                                        }
                                        if let Some(cam) = s.has_camera {
                                            if cam {
                                                c.camera_type = Some(if s.has_xcam {
                                                    "ai".to_owned()
                                                } else {
                                                    "basic".to_owned()
                                                });
                                            }
                                        }
                                    }

                                    let _ = bus_tx.send(ServerEvent::PrinterStatus {
                                        printer_id,
                                        status: ps,
                                    });
                                }
                            }
                        }
                    }
                }

                _ = shutdown_rx.changed() => {
                    info!(printer_id, "bambu: shutdown signal");
                    let _ = client.disconnect().await;
                    return Ok(());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PrinterDriver impl
// ---------------------------------------------------------------------------

#[async_trait]
impl PrinterDriver for BambuDriver {
    async fn connect(&self) -> Result<PrinterCapabilities, AppError> {
        let handle = self.spawn_loop();
        let mut task = self.task.lock().await;
        *task = Some(handle);

        // Wait briefly for initial pushall response.
        tokio::time::sleep(Duration::from_secs(3)).await;

        let caps = self.capabilities.read().await;
        Ok(caps.clone().unwrap_or_default())
    }

    async fn disconnect(&self) -> Result<(), AppError> {
        let _ = self.shutdown_tx.send(true);
        let mut task = self.task.lock().await;
        if let Some(handle) = task.take() {
            let _ = handle.await;
        }
        Ok(())
    }

    async fn probe_capabilities(&self) -> Result<PrinterCapabilities, AppError> {
        let caps = self.capabilities.read().await;
        Ok(caps.clone().unwrap_or_default())
    }

    async fn send_command(&self, _cmd: PrinterCommand) -> Result<(), AppError> {
        Err(AppError::NotImplemented(
            "Bambu commands deferred to a future session".into(),
        ))
    }

    async fn list_files(&self, path: Option<&str>) -> Result<Vec<PrinterFile>, AppError> {
        let host = self.config.host.clone();
        let access_code = self.config.access_code.clone();
        let dir = path.unwrap_or("/").to_owned();

        tokio::task::spawn_blocking(move || {
            let mut ftp = ftps_connect_sync(&host, &access_code)?;

            let entries = ftp
                .nlst(Some(&dir))
                .map_err(|e| AppError::Driver(format!("ftps list: {e}")))?;

            let mut files = Vec::new();
            for entry in entries {
                let name = entry.trim().to_owned();
                if name.is_empty() || name == "." || name == ".." {
                    continue;
                }
                let full_path = if dir == "/" {
                    format!("/{name}")
                } else {
                    format!("{dir}/{name}")
                };
                let size = ftp.size(&full_path).ok().map(|s| s as u64);
                files.push(PrinterFile {
                    path: full_path,
                    size,
                    modified: None,
                });
            }

            let _ = ftp.quit();
            Ok(files)
        })
        .await
        .map_err(|e| AppError::Driver(format!("ftps task join: {e}")))?
    }

    async fn upload_file(&self, path: &str, data: Vec<u8>) -> Result<(), AppError> {
        let host = self.config.host.clone();
        let access_code = self.config.access_code.clone();
        let path = path.to_owned();

        tokio::task::spawn_blocking(move || {
            let mut ftp = ftps_connect_sync(&host, &access_code)?;

            let mut cursor = std::io::Cursor::new(data);
            ftp.put_file(&path, &mut cursor)
                .map_err(|e| AppError::Driver(format!("ftps upload: {e}")))?;

            let _ = ftp.quit();
            Ok(())
        })
        .await
        .map_err(|e| AppError::Driver(format!("ftps task join: {e}")))?
    }

    async fn delete_file(&self, path: &str) -> Result<(), AppError> {
        let host = self.config.host.clone();
        let access_code = self.config.access_code.clone();
        let path = path.to_owned();

        tokio::task::spawn_blocking(move || {
            let mut ftp = ftps_connect_sync(&host, &access_code)?;

            ftp.rm(&path)
                .map_err(|e| AppError::Driver(format!("ftps delete: {e}")))?;

            let _ = ftp.quit();
            Ok(())
        })
        .await
        .map_err(|e| AppError::Driver(format!("ftps task join: {e}")))?
    }

    async fn start_print(&self, _path: &str) -> Result<(), AppError> {
        Err(AppError::NotImplemented(
            "Bambu commands deferred to a future session".into(),
        ))
    }

    async fn current_status(&self) -> Result<PrinterStatus, AppError> {
        Ok(self.status.read().await.to_printer_status())
    }
}
