//! Moonraker WebSocket JSON-RPC driver.
//!
//! Connects via WebSocket, subscribes to Klipper object updates, merges
//! partial status updates into a full `PrinterStatus`, and publishes events
//! on the Foreman event bus.
//!
//! Key Moonraker JSON-RPC methods used:
//! - `server.info` — server state, klipper version
//! - `printer.objects.list` — available Klipper objects
//! - `printer.objects.subscribe` — subscribe to object changes
//! - `printer.objects.query` — one-shot query for config data
//! - `server.webcams.list` — webcam list
//!
//! Subscribed objects: print_stats, extruder, heater_bed, virtual_sdcard,
//! display_status, toolhead, webhooks.
//!
//! Partial updates arrive as `notify_status_update` notifications and are
//! merged into the cached status. Events are throttled to max 2/s per printer.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, watch, Mutex, RwLock};
use tracing::{debug, error, info, warn};

use crate::driver::{PrinterCapabilities, PrinterCommand, PrinterDriver, PrinterFile, WebcamInfo};
use crate::drivers::moonraker_files::MoonrakerFiles;
use crate::error::AppError;
use crate::events::{
    ConnectionEvent, ConnectionEventKind, PrintEnd, PrinterState, PrinterStatus, ServerEvent,
    TempReading,
};
use crate::storage::PrinterStorage;

// ---------------------------------------------------------------------------
// JSON-RPC types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
    id: u64,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    id: Option<u64>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
    method: Option<String>,
    params: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

// ---------------------------------------------------------------------------
// Subscribed objects
// ---------------------------------------------------------------------------

/// The Klipper objects we subscribe to.
const SUBSCRIBED_OBJECTS: &[(&str, Option<&[&str]>)] = &[
    ("print_stats", None),
    ("extruder", Some(&["temperature", "target"])),
    ("heater_bed", Some(&["temperature", "target"])),
    ("virtual_sdcard", Some(&["progress", "file_path"])),
    ("display_status", Some(&["message", "progress"])),
    ("toolhead", Some(&["position", "homed_axes"])),
    ("webhooks", Some(&["state", "state_message"])),
];

fn subscription_objects() -> Value {
    let mut map = serde_json::Map::new();
    for (name, fields) in SUBSCRIBED_OBJECTS {
        let val = match fields {
            Some(fs) => Value::Array(fs.iter().map(|f| Value::String(f.to_string())).collect()),
            None => Value::Null,
        };
        map.insert(name.to_string(), val);
    }
    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Internal cached state
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
    print_duration: f64,
    display_message: Option<String>,
    print_end: Option<PrintEnd>,
    print_message: Option<String>,
    filament_used_mm: Option<f64>,
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
            print_duration: Some(self.print_duration),
            display_message: self.display_message.clone(),
            layer: None,
            total_layers: None,
            remaining_time: None,
            fans: None,
            ams_trays: Vec::new(),
            hms_codes: Vec::new(),
            print_end: self.print_end,
            print_message: self.print_message.clone(),
            filament_used_mm: self.filament_used_mm,
        }
    }

    /// Merge a partial `notify_status_update` payload into this status.
    fn merge_update(&mut self, objects: &Value) {
        if let Some(obj) = objects.as_object() {
            for (key, val) in obj {
                match key.as_str() {
                    "extruder" => {
                        if let Some(t) = val.get("temperature").and_then(|v| v.as_f64()) {
                            self.nozzle_actual = t;
                        }
                        if let Some(t) = val.get("target").and_then(|v| v.as_f64()) {
                            self.nozzle_target = t;
                        }
                    }
                    "heater_bed" => {
                        if let Some(t) = val.get("temperature").and_then(|v| v.as_f64()) {
                            self.bed_actual = t;
                        }
                        if let Some(t) = val.get("target").and_then(|v| v.as_f64()) {
                            self.bed_target = t;
                        }
                    }
                    "virtual_sdcard" => {
                        if let Some(p) = val.get("progress").and_then(|v| v.as_f64()) {
                            self.progress = p;
                        }
                        if let Some(f) = val.get("file_path").and_then(|v| v.as_str()) {
                            self.filename = if f.is_empty() {
                                None
                            } else {
                                Some(f.to_owned())
                            };
                        }
                    }
                    "print_stats" => {
                        if let Some(s) = val.get("state").and_then(|v| v.as_str()) {
                            self.state = match s {
                                "printing" => PrinterState::Printing,
                                "paused" => PrinterState::Paused,
                                "error" => PrinterState::Error,
                                "complete" | "cancelled" | "standby" => PrinterState::Idle,
                                _ => PrinterState::Idle,
                            };
                            // Klipper keeps complete/cancelled/error until the next
                            // print or SDCARD_RESET_FILE (→ standby).
                            self.print_end = match s {
                                "complete" => Some(PrintEnd::Complete),
                                "cancelled" => Some(PrintEnd::Cancelled),
                                "error" => Some(PrintEnd::Error),
                                _ => None,
                            };
                        }
                        if let Some(m) = val.get("message").and_then(|v| v.as_str()) {
                            self.print_message = if m.is_empty() {
                                None
                            } else {
                                Some(m.to_owned())
                            };
                        }
                        if let Some(f) = val.get("filament_used").and_then(|v| v.as_f64()) {
                            self.filament_used_mm = Some(f);
                        }
                        if let Some(d) = val.get("print_duration").and_then(|v| v.as_f64()) {
                            self.print_duration = d;
                        }
                        if let Some(f) = val.get("filename").and_then(|v| v.as_str()) {
                            self.filename = if f.is_empty() {
                                None
                            } else {
                                Some(f.to_owned())
                            };
                        }
                    }
                    "display_status" => {
                        if let Some(m) = val.get("message").and_then(|v| v.as_str()) {
                            self.display_message = if m.is_empty() {
                                None
                            } else {
                                Some(m.to_owned())
                            };
                        }
                    }
                    "webhooks" => {
                        if let Some(s) = val.get("state").and_then(|v| v.as_str()) {
                            if s == "shutdown" || s == "error" {
                                self.state = PrinterState::Error;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// WebSocket send helper
// ---------------------------------------------------------------------------

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::Message,
>;

async fn ws_send_rpc(
    tx: &mut WsSink,
    id: u64,
    method: &str,
    params: Option<Value>,
) -> Result<(), AppError> {
    use tokio_tungstenite::tungstenite;
    let req = JsonRpcRequest {
        jsonrpc: "2.0",
        method: method.to_owned(),
        params,
        id,
    };
    let json = serde_json::to_string(&req).unwrap_or_else(|_| "{}".to_owned());
    tx.send(tungstenite::Message::Text(json.into()))
        .await
        .map_err(|e| AppError::Driver(format!("ws send: {e}")))
}

// ---------------------------------------------------------------------------
// MoonrakerDriver
// ---------------------------------------------------------------------------

/// Configuration for a Moonraker driver instance.
#[derive(Debug, Clone)]
pub struct MoonrakerConfig {
    /// Printer database ID.
    pub printer_id: i64,
    /// Full Moonraker URL, e.g. `http://10.0.0.177` (ws:// will be derived).
    pub url: String,
    /// Optional API key.
    pub api_key: Option<String>,
}

/// A Moonraker printer driver.
///
/// Internally spawns a background task that maintains the WebSocket connection,
/// subscribes to objects, and publishes events on the bus.
pub struct MoonrakerDriver {
    config: MoonrakerConfig,
    bus_tx: Arc<broadcast::Sender<ServerEvent>>,
    status: Arc<RwLock<CachedStatus>>,
    capabilities: Arc<RwLock<Option<PrinterCapabilities>>>,
    /// HTTP client for sending commands to Moonraker.
    http: reqwest::Client,
    /// Signals the background loop to stop.
    shutdown_tx: watch::Sender<bool>,
    /// Handle to the background task.
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// `/server/files/*` for the gcodes root.
    files: Arc<MoonrakerFiles>,
}

impl MoonrakerDriver {
    pub fn new(config: MoonrakerConfig, bus_tx: Arc<broadcast::Sender<ServerEvent>>) -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        let files = Arc::new(MoonrakerFiles::new(&config.url, config.api_key.clone()));
        Self {
            config,
            bus_tx,
            status: Arc::new(RwLock::new(CachedStatus::default())),
            capabilities: Arc::new(RwLock::new(None)),
            http,
            shutdown_tx,
            task: Mutex::new(None),
            files,
        }
    }

    /// Derive the WebSocket URL from the configured HTTP URL.
    fn ws_url(&self) -> Result<String, AppError> {
        let parsed = url::Url::parse(&self.config.url)
            .map_err(|e| AppError::Driver(format!("invalid URL: {e}")))?;
        let scheme = match parsed.scheme() {
            "http" | "ws" => "ws",
            "https" | "wss" => "wss",
            s => return Err(AppError::Driver(format!("unsupported scheme: {s}"))),
        };
        let host = parsed
            .host_str()
            .ok_or_else(|| AppError::Driver("URL has no host".into()))?;
        let port = parsed.port().map(|p| format!(":{p}")).unwrap_or_default();
        Ok(format!("{scheme}://{host}{port}/websocket"))
    }

    /// Spawn the background connection loop.
    fn spawn_loop(&self) -> tokio::task::JoinHandle<()> {
        let ws_url = match self.ws_url() {
            Ok(u) => u,
            Err(e) => {
                error!(printer_id = self.config.printer_id, "bad ws url: {e}");
                return tokio::spawn(async {});
            }
        };
        let printer_id = self.config.printer_id;
        let api_key = self.config.api_key.clone();
        let http_base = self.config.url.clone();
        let bus_tx = Arc::clone(&self.bus_tx);
        let status = Arc::clone(&self.status);
        let capabilities = Arc::clone(&self.capabilities);
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            let max_backoff = Duration::from_secs(30);

            loop {
                // Check shutdown before attempting connect.
                if *shutdown_rx.borrow() {
                    break;
                }

                info!(printer_id, url = %ws_url, "moonraker: connecting");

                match Self::run_connection(
                    printer_id,
                    &ws_url,
                    &http_base,
                    api_key.as_deref(),
                    &bus_tx,
                    &status,
                    &capabilities,
                    &mut shutdown_rx,
                )
                .await
                {
                    Ok(()) => {
                        info!(printer_id, "moonraker: disconnected cleanly");
                        break;
                    }
                    Err(e) => {
                        warn!(printer_id, "moonraker: connection lost: {e}");
                        // Publish offline event.
                        let _ = bus_tx.send(ServerEvent::PrinterConnection {
                            printer_id,
                            event: ConnectionEvent {
                                kind: ConnectionEventKind::Offline,
                                active_path: None,
                            },
                        });
                        // Set status to offline.
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

                // Check shutdown before sleeping.
                if *shutdown_rx.borrow() {
                    break;
                }

                info!(
                    printer_id,
                    backoff_secs = backoff.as_secs(),
                    "moonraker: reconnecting after backoff"
                );
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {},
                    _ = shutdown_rx.changed() => { break; }
                }
                backoff = (backoff * 2).min(max_backoff);
            }
        })
    }

    /// Run a single WebSocket connection session. Returns Ok(()) on clean
    /// shutdown, Err on connection loss.
    #[allow(clippy::too_many_arguments)]
    async fn run_connection(
        printer_id: i64,
        ws_url: &str,
        http_base: &str,
        api_key: Option<&str>,
        bus_tx: &broadcast::Sender<ServerEvent>,
        status: &RwLock<CachedStatus>,
        capabilities: &RwLock<Option<PrinterCapabilities>>,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) -> Result<(), AppError> {
        use tokio_tungstenite::tungstenite;

        // Connect — API key, if present, is passed as a query parameter.
        // Moonraker accepts `?token=<key>` on the WebSocket URL.
        let connect_url = match api_key {
            Some(key) => format!("{ws_url}?token={key}"),
            None => ws_url.to_owned(),
        };

        let (ws, _) = tokio_tungstenite::connect_async(&connect_url)
            .await
            .map_err(|e| AppError::Driver(format!("ws connect: {e}")))?;

        let (mut ws_tx, mut ws_rx) = ws.split();
        let mut rpc_id: u64 = 1;

        // Publish online event.
        let _ = bus_tx.send(ServerEvent::PrinterConnection {
            printer_id,
            event: ConnectionEvent {
                kind: ConnectionEventKind::Online,
                active_path: None,
            },
        });

        // 1) server.info
        let server_info_id = rpc_id;
        rpc_id += 1;
        ws_send_rpc(&mut ws_tx, server_info_id, "server.info", None).await?;

        // 2) printer.objects.subscribe
        let subscribe_id = rpc_id;
        rpc_id += 1;
        ws_send_rpc(
            &mut ws_tx,
            subscribe_id,
            "printer.objects.subscribe",
            Some(json!({ "objects": subscription_objects() })),
        )
        .await?;

        // 3) Query config for capabilities.
        let config_query_id = rpc_id;
        rpc_id += 1;
        ws_send_rpc(
            &mut ws_tx,
            config_query_id,
            "printer.objects.query",
            Some(json!({
                "objects": {
                    "configfile": ["settings"],
                    "toolhead": ["axis_maximum", "axis_minimum"]
                }
            })),
        )
        .await?;

        // 4) webcams
        let webcams_id = rpc_id;
        rpc_id += 1;
        ws_send_rpc(&mut ws_tx, webcams_id, "server.webcams.list", None).await?;

        // 5) macros
        let macros_id = rpc_id;
        rpc_id += 1;
        ws_send_rpc(&mut ws_tx, macros_id, "printer.objects.list", None).await?;

        // 6) [spoolman] link (errors "method not found" when not configured).
        let spoolman_id = rpc_id;
        ws_send_rpc(&mut ws_tx, spoolman_id, "server.spoolman.status", None).await?;

        // Throttle: max 2 status events per second.
        let min_interval = Duration::from_millis(500);
        let mut last_publish = Instant::now() - min_interval;

        // Message loop.
        loop {
            tokio::select! {
                msg = ws_rx.next() => {
                    let msg = match msg {
                        Some(Ok(m)) => m,
                        Some(Err(e)) => return Err(AppError::Driver(format!("ws read: {e}"))),
                        None => return Err(AppError::Driver("ws closed".into())),
                    };

                    let text = match msg {
                        tungstenite::Message::Text(t) => t,
                        tungstenite::Message::Ping(d) => {
                            let _ = ws_tx.send(tungstenite::Message::Pong(d)).await;
                            continue;
                        }
                        tungstenite::Message::Close(_) => {
                            return Err(AppError::Driver("ws server closed".into()));
                        }
                        _ => continue,
                    };

                    let resp: JsonRpcResponse = match serde_json::from_str(&text) {
                        Ok(r) => r,
                        Err(e) => {
                            debug!(printer_id, "moonraker: unparseable msg: {e}");
                            continue;
                        }
                    };

                    // Handle notifications (no id, has method).
                    if resp.id.is_none() {
                        if let Some(method) = &resp.method {
                            if method == "notify_status_update" {
                                if let Some(params) = &resp.params {
                                    if let Some(objects) = params.first() {
                                        let mut s = status.write().await;
                                        let before = (s.state.clone(), s.print_end);
                                        s.merge_update(objects);
                                        let state_changed = before != (s.state.clone(), s.print_end);
                                        drop(s);

                                        // Throttled publish; state changes always go out
                                        // (the job engine needs every start/end).
                                        let now = Instant::now();
                                        if state_changed || now.duration_since(last_publish) >= min_interval {
                                            last_publish = now;
                                            let ps = status.read().await.to_printer_status();
                                            let _ = bus_tx.send(ServerEvent::PrinterStatus {
                                                printer_id,
                                                status: ps,
                                            });
                                        }
                                    }
                                }
                            } else if method == "notify_klippy_disconnected" {
                                let mut s = status.write().await;
                                s.state = PrinterState::Error;
                                drop(s);
                                let ps = status.read().await.to_printer_status();
                                let _ = bus_tx.send(ServerEvent::PrinterStatus {
                                    printer_id,
                                    status: ps,
                                });
                            } else if method == "notify_spoolman_status_changed" {
                                let connected = resp.params.as_ref()
                                    .and_then(|p| p.first())
                                    .and_then(|o| o.get("spoolman_connected"))
                                    .and_then(|v| v.as_bool());
                                let mut caps = capabilities.write().await;
                                caps.get_or_insert_with(PrinterCapabilities::default).spoolman_connected = connected;
                                debug!(printer_id, ?connected, "moonraker: spoolman status changed");
                            } else if method == "notify_active_spool_set" {
                                let spool = resp.params.as_ref()
                                    .and_then(|p| p.first())
                                    .and_then(|o| o.get("spool_id"))
                                    .and_then(|v| v.as_i64());
                                let mut caps = capabilities.write().await;
                                caps.get_or_insert_with(PrinterCapabilities::default).spoolman_spool_id = spool;
                            } else if method == "notify_klippy_ready" {
                                // Klippy came back — re-subscribe.
                                let mut s = status.write().await;
                                s.state = PrinterState::Idle;
                                drop(s);
                            }
                        }
                        continue;
                    }

                    // Handle RPC responses.
                    let id = resp.id.unwrap_or(0);

                    if let Some(err) = resp.error {
                        if id == spoolman_id {
                            debug!(printer_id, msg = %err.message, "moonraker: no [spoolman] component");
                            let mut caps = capabilities.write().await;
                            caps.get_or_insert_with(PrinterCapabilities::default).spoolman_connected = None;
                            continue;
                        }
                        warn!(printer_id, id, code = err.code, msg = %err.message, "moonraker rpc error");
                        continue;
                    }

                    let result = match resp.result {
                        Some(r) => r,
                        None => continue,
                    };

                    if id == server_info_id {
                        // Extract klipper version, hostname.
                        let version = result.get("klippy_state")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_owned());
                        let hostname = result.get("hostname")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_owned());
                        let mut caps = capabilities.write().await;
                        let c = caps.get_or_insert_with(PrinterCapabilities::default);
                        c.hostname = hostname;
                        if let Some(v) = version {
                            debug!(printer_id, klippy_state = %v, "server.info");
                        }
                    } else if id == subscribe_id {
                        // Initial status snapshot from subscribe response.
                        if let Some(status_obj) = result.get("status") {
                            let mut s = status.write().await;
                            s.merge_update(status_obj);
                            drop(s);
                            let ps = status.read().await.to_printer_status();
                            let _ = bus_tx.send(ServerEvent::PrinterStatus {
                                printer_id,
                                status: ps,
                            });
                        }
                    } else if id == config_query_id {
                        // Parse bed size + extruder count from config.
                        Self::parse_config_capabilities(&result, capabilities).await;
                    } else if id == webcams_id {
                        // Parse webcam list.
                        Self::parse_webcams(&result, capabilities, http_base).await;
                    } else if id == macros_id {
                        // Parse macro names from printer.objects.list.
                        Self::parse_macros(&result, capabilities).await;
                    } else if id == spoolman_id {
                        let connected = result.get("spoolman_connected").and_then(|v| v.as_bool());
                        let spool = result.get("spool_id").and_then(|v| v.as_i64());
                        let mut caps = capabilities.write().await;
                        let c = caps.get_or_insert_with(PrinterCapabilities::default);
                        c.spoolman_connected = connected;
                        c.spoolman_spool_id = spool;
                        info!(printer_id, ?connected, ?spool, "moonraker: [spoolman] link");
                    }
                }

                _ = shutdown_rx.changed() => {
                    info!(printer_id, "moonraker: shutdown signal received");
                    let _ = ws_tx.close().await;
                    return Ok(());
                }
            }
        }
    }

    async fn parse_config_capabilities(
        result: &Value,
        capabilities: &RwLock<Option<PrinterCapabilities>>,
    ) {
        let status = match result.get("status") {
            Some(s) => s,
            None => return,
        };

        let mut caps = capabilities.write().await;
        let c = caps.get_or_insert_with(PrinterCapabilities::default);

        // Bed size from toolhead axis limits.
        if let Some(toolhead) = status.get("toolhead") {
            if let Some(max) = toolhead.get("axis_maximum").and_then(|v| v.as_array()) {
                if max.len() >= 2 {
                    let x = max[0].as_f64().unwrap_or(0.0);
                    let y = max[1].as_f64().unwrap_or(0.0);
                    if x > 0.0 && y > 0.0 {
                        c.bed_size = Some((x, y));
                    }
                }
            }
        }

        // Extruder count + nozzle diameter from configfile.
        if let Some(configfile) = status.get("configfile") {
            if let Some(settings) = configfile.get("settings").and_then(|v| v.as_object()) {
                let mut ext_count: u32 = 0;
                for key in settings.keys() {
                    if key == "extruder" || key.starts_with("extruder") {
                        let suffix = &key["extruder".len()..];
                        if suffix.is_empty() || suffix.chars().all(|c| c.is_ascii_digit()) {
                            ext_count += 1;
                        }
                    }
                }
                if ext_count > 0 {
                    c.extruder_count = ext_count;
                }
                // Nozzle diameter from the primary extruder.
                if let Some(nozzle) = settings
                    .get("extruder")
                    .and_then(|e| e.get("nozzle_diameter"))
                    .and_then(|v| v.as_f64())
                {
                    if nozzle > 0.0 {
                        c.nozzle_diameter = Some(nozzle);
                    }
                }
            }
        }
    }

    async fn parse_webcams(
        result: &Value,
        capabilities: &RwLock<Option<PrinterCapabilities>>,
        http_base: &str,
    ) {
        let webcams_arr = match result.get("webcams").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => return,
        };

        // Relative webcam URLs (e.g. /webcam/...) are served by the web frontend (nginx),
        // not Moonraker, so resolve them against scheme://host without Moonraker's port.
        let origin = {
            let s = http_base
                .replace("ws://", "http://")
                .replace("wss://", "https://");
            if let Some(pos) = s.find("://") {
                let after = &s[pos + 3..];
                let authority = after.split('/').next().unwrap_or(after);
                let host = match authority.rfind(':') {
                    Some(i) if !authority[i..].contains(']') => &authority[..i],
                    _ => authority,
                };
                format!("{}{}", &s[..pos + 3], host)
            } else {
                s
            }
        };

        let resolve = |raw: &str| -> String {
            if raw.starts_with("http://") || raw.starts_with("https://") {
                raw.to_owned()
            } else if raw.starts_with('/') {
                format!("{origin}{raw}")
            } else {
                raw.to_owned()
            }
        };

        let mut caps = capabilities.write().await;
        let c = caps.get_or_insert_with(PrinterCapabilities::default);
        c.webcams.clear();

        for wc in webcams_arr {
            let name = wc
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_owned();
            let stream_url = wc.get("stream_url").and_then(|v| v.as_str()).map(resolve);
            let snapshot_url = wc.get("snapshot_url").and_then(|v| v.as_str()).map(resolve);
            c.webcams.push(WebcamInfo {
                name,
                stream_url,
                snapshot_url,
            });
        }
    }

    /// POST to a Moonraker endpoint (no body). Used for emergency_stop etc.
    async fn moonraker_post(&self, url: &str) -> Result<(), AppError> {
        self.http
            .post(url)
            .send()
            .await
            .map_err(|e| AppError::Driver(format!("http post: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::Driver(format!("http rejected: {e}")))?;
        Ok(())
    }

    async fn parse_macros(result: &Value, capabilities: &RwLock<Option<PrinterCapabilities>>) {
        let objects = match result.get("objects").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => return,
        };

        let mut caps = capabilities.write().await;
        let c = caps.get_or_insert_with(PrinterCapabilities::default);
        c.macros.clear();

        for obj in objects {
            if let Some(name) = obj.as_str() {
                if let Some(macro_name) = name.strip_prefix("gcode_macro ") {
                    // Skip internal macros starting with underscore.
                    if !macro_name.starts_with('_') {
                        c.macros.push(macro_name.to_owned());
                    }
                }
            }
        }
        c.macros.sort();
    }
}

// ---------------------------------------------------------------------------
// PrinterDriver impl
// ---------------------------------------------------------------------------

#[async_trait]
impl PrinterDriver for MoonrakerDriver {
    async fn connect(&self) -> Result<PrinterCapabilities, AppError> {
        let handle = self.spawn_loop();
        let mut task = self.task.lock().await;
        *task = Some(handle);

        // Wait briefly for initial data to arrive.
        tokio::time::sleep(Duration::from_secs(2)).await;

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

    async fn send_command(&self, cmd: PrinterCommand) -> Result<(), AppError> {
        // Emergency stop uses its own Moonraker endpoint.
        if matches!(cmd, PrinterCommand::EmergencyStop) {
            let url = format!("{}/printer/emergency_stop", self.config.url);
            self.moonraker_post(&url).await?;
            return Ok(());
        }

        let gcode = match cmd {
            PrinterCommand::Pause => "PAUSE".to_owned(),
            PrinterCommand::Resume => "RESUME".to_owned(),
            PrinterCommand::Cancel => "CANCEL_PRINT".to_owned(),
            PrinterCommand::SetNozzleTemp { target } => format!("M104 S{target:.0}"),
            PrinterCommand::SetBedTemp { target } => format!("M140 S{target:.0}"),
            PrinterCommand::Home { axes } => {
                if axes.is_empty() {
                    "G28".to_owned()
                } else {
                    format!("G28 {}", axes.join(" ").to_uppercase())
                }
            }
            PrinterCommand::Jog { x, y, z, speed } => {
                let mut parts = vec!["G91".to_owned(), "G1".to_owned()];
                if let Some(v) = x {
                    parts.push(format!("X{v:.3}"));
                }
                if let Some(v) = y {
                    parts.push(format!("Y{v:.3}"));
                }
                if let Some(v) = z {
                    parts.push(format!("Z{v:.3}"));
                }
                if let Some(s) = speed {
                    parts.push(format!("F{s:.0}"));
                }
                parts.push("\nG90".to_owned());
                parts.join(" ")
            }
            PrinterCommand::BabyStep { offset } => {
                format!("SET_GCODE_OFFSET Z_ADJUST={offset:.3} MOVE=1")
            }
            PrinterCommand::MotorsOff => "M84".to_owned(),
            PrinterCommand::Extrude { length, speed } => {
                // Relative extrude: switch to relative E, extrude, switch back.
                format!("M83\nG1 E{length:.1} F{speed:.0}\nM82")
            }
            PrinterCommand::SetLight { brightness } => {
                let value = brightness.clamp(0.0, 1.0);
                format!("SET_PIN PIN=light VALUE={value:.2}")
            }
            PrinterCommand::Macro { name } => name,
            PrinterCommand::Gcode { command } => command,
            PrinterCommand::EmergencyStop => unreachable!(),
        };

        let url = format!("{}/printer/gcode/script", self.config.url,);
        self.http
            .post(&url)
            .query(&[("script", &gcode)])
            .send()
            .await
            .map_err(|e| AppError::Driver(format!("command http: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::Driver(format!("command rejected: {e}")))?;

        Ok(())
    }

    async fn list_files(&self, path: Option<&str>) -> Result<Vec<PrinterFile>, AppError> {
        Ok(self
            .files
            .list_dir(path.unwrap_or(""))
            .await?
            .into_iter()
            .filter(|e| !e.is_dir)
            .map(|e| PrinterFile {
                path: e.path,
                size: Some(e.size),
                modified: Some(e.modified),
            })
            .collect())
    }

    async fn upload_file(&self, path: &str, data: Vec<u8>) -> Result<(), AppError> {
        self.files.upload_bytes(path, data).await
    }

    async fn delete_file(&self, path: &str) -> Result<(), AppError> {
        self.files.delete_file(path).await
    }

    async fn start_print(&self, path: &str) -> Result<(), AppError> {
        let url = format!("{}/printer/print/start", self.config.url);
        self.http
            .post(&url)
            .query(&[("filename", path)])
            .send()
            .await
            .map_err(|e| AppError::Driver(format!("start_print http: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::Driver(format!("start_print rejected: {e}")))?;
        Ok(())
    }

    async fn current_status(&self) -> Result<PrinterStatus, AppError> {
        Ok(self.status.read().await.to_printer_status())
    }

    fn storage(&self) -> Option<Arc<dyn PrinterStorage>> {
        Some(self.files.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_stats_end_states_are_told_apart() {
        let mut c = CachedStatus::default();
        c.merge_update(&json!({"print_stats": {"state": "printing", "filename": "a.gcode"}}));
        assert_eq!(c.state, PrinterState::Printing);
        assert_eq!(c.print_end, None);
        c.merge_update(&json!({"print_stats": {"state": "cancelled", "filament_used": 812.5}}));
        assert_eq!(c.state, PrinterState::Idle);
        assert_eq!(c.print_end, Some(PrintEnd::Cancelled));
        assert_eq!(c.to_printer_status().filament_used_mm, Some(812.5));
        c.merge_update(&json!({"print_stats": {"state": "complete"}}));
        assert_eq!(c.print_end, Some(PrintEnd::Complete));
        c.merge_update(&json!({"print_stats": {"state": "error", "message": "Move out of range"}}));
        assert_eq!(c.state, PrinterState::Error);
        let st = c.to_printer_status();
        assert_eq!(st.print_end, Some(PrintEnd::Error));
        assert_eq!(st.print_message.as_deref(), Some("Move out of range"));
        c.merge_update(&json!({"print_stats": {"state": "standby", "message": ""}}));
        assert_eq!(c.print_end, None);
        assert_eq!(c.print_message, None);
    }

    #[tokio::test]
    async fn relative_webcam_urls_use_frontend_origin() {
        let caps = RwLock::new(None);
        let result = serde_json::json!({ "webcams": [
            { "name": "cam", "stream_url": "/webcam/?action=stream",
              "snapshot_url": "/webcam/?action=snapshot" },
            { "name": "abs", "snapshot_url": "http://cam.local:8080/snap" }
        ]});
        MoonrakerDriver::parse_webcams(&result, &caps, "http://10.0.0.177:7125").await;
        let c = caps.read().await;
        let w = &c.as_ref().expect("caps").webcams;
        assert_eq!(
            w[0].snapshot_url.as_deref(),
            Some("http://10.0.0.177/webcam/?action=snapshot")
        );
        assert_eq!(
            w[0].stream_url.as_deref(),
            Some("http://10.0.0.177/webcam/?action=stream")
        );
        assert_eq!(
            w[1].snapshot_url.as_deref(),
            Some("http://cam.local:8080/snap")
        );
    }
}
