//! Driver manager — spawns and manages one printer driver per registered printer.
//!
//! On server start, loads all printers from the Foreman DB and spawns a driver
//! for each one that has a `primary_url`. Adding or removing a printer via the
//! REST API starts or stops its driver.
//!
//! Background tasks also raise/clear attention items automatically:
//! - `error` kind: when a PrinterStatus event arrives with state=Error
//! - `offline` kind: when no status has been received for >60 s

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use foreman_core::{
    domain::{FirmwareKind, Printer},
    driver::{PrinterCapabilities, PrinterDriver},
    drivers::{
        bambu::{BambuConfig, BambuDriver},
        moonraker::{MoonrakerConfig, MoonrakerDriver},
    },
    events::{AttentionItemEvent, PrinterState, PrinterStatus, ServerEvent},
};
use sqlx::SqlitePool;
use tokio::sync::{broadcast, RwLock};
use tracing::{info, warn};

/// A running driver handle (type-erased).
struct DriverHandle {
    driver: Arc<dyn PrinterDriver>,
}

/// Manages the lifecycle of printer drivers.
pub struct DriverManager {
    drivers: RwLock<HashMap<i64, DriverHandle>>,
    bus_tx: Arc<broadcast::Sender<ServerEvent>>,
    pool: SqlitePool,
    /// Last-known status per printer (updated by drivers via events).
    status_cache: RwLock<HashMap<i64, PrinterStatus>>,
    /// Cached capabilities per printer.
    capabilities_cache: RwLock<HashMap<i64, PrinterCapabilities>>,
    /// When the last status event was received for each printer.
    last_status_at: RwLock<HashMap<i64, Instant>>,
    /// Map of (printer_id, kind) → attention_item_id for live items we raised.
    open_attention: RwLock<HashMap<(i64, String), i64>>,
}

impl DriverManager {
    pub fn new(bus_tx: Arc<broadcast::Sender<ServerEvent>>, pool: SqlitePool) -> Arc<Self> {
        let mgr = Arc::new(Self {
            drivers: RwLock::new(HashMap::new()),
            bus_tx: Arc::clone(&bus_tx),
            pool,
            status_cache: RwLock::new(HashMap::new()),
            capabilities_cache: RwLock::new(HashMap::new()),
            last_status_at: RwLock::new(HashMap::new()),
            open_attention: RwLock::new(HashMap::new()),
        });

        // ── Task 1: status event listener ────────────────────────────────────
        {
            let mgr2 = Arc::clone(&mgr);
            let mut rx = bus_tx.subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(ServerEvent::PrinterStatus { printer_id, status }) => {
                            mgr2.status_cache
                                .write()
                                .await
                                .insert(printer_id, status.clone());
                            mgr2.last_status_at
                                .write()
                                .await
                                .insert(printer_id, Instant::now());

                            // ── Clear offline attention if printer is back ───────
                            let offline_key = (printer_id, "offline".to_owned());
                            if let Some(att_id) =
                                mgr2.open_attention.read().await.get(&offline_key).copied()
                            {
                                if let Err(e) =
                                    foreman_core::repo::attention::dismiss(&mgr2.pool, att_id).await
                                {
                                    warn!(printer_id, "attention clear offline: {e}");
                                } else {
                                    mgr2.open_attention.write().await.remove(&offline_key);
                                    let _ = mgr2.bus_tx.send(ServerEvent::AttentionCleared {
                                        item_id: att_id,
                                        printer_id: Some(printer_id),
                                    });
                                    info!(printer_id, "offline attention cleared (printer back)");
                                }
                            }

                            // ── Error state handling ─────────────────────────────
                            let error_key = (printer_id, "error".to_owned());
                            if status.state == PrinterState::Error {
                                if !mgr2.open_attention.read().await.contains_key(&error_key) {
                                    let title = mgr2
                                        .printer_name(printer_id)
                                        .await
                                        .map(|n| format!("{n}: printer error"))
                                        .unwrap_or_else(|| format!("Printer {printer_id}: error"));
                                    let body = status.display_message.as_deref().map(str::to_owned);
                                    match foreman_core::repo::attention::raise(
                                        &mgr2.pool,
                                        Some(printer_id),
                                        "error",
                                        &title,
                                        body.as_deref(),
                                    )
                                    .await
                                    {
                                        Ok(item) => {
                                            let att_id = item.id;
                                            mgr2.open_attention
                                                .write()
                                                .await
                                                .insert(error_key, att_id);
                                            let _ = mgr2.bus_tx.send(ServerEvent::AttentionAdded {
                                                item: AttentionItemEvent {
                                                    id: att_id,
                                                    printer_id: Some(printer_id),
                                                    kind: "error".to_owned(),
                                                    title: item.title,
                                                    body: item.body,
                                                },
                                            });
                                            info!(printer_id, att_id, "error attention raised");
                                        }
                                        Err(e) => {
                                            warn!(printer_id, "raise error attention: {e}");
                                        }
                                    }
                                }
                            } else {
                                // Non-error: clear any open error attention.
                                if let Some(att_id) =
                                    mgr2.open_attention.read().await.get(&error_key).copied()
                                {
                                    if let Err(e) =
                                        foreman_core::repo::attention::dismiss(&mgr2.pool, att_id)
                                            .await
                                    {
                                        warn!(printer_id, "attention clear error: {e}");
                                    } else {
                                        mgr2.open_attention.write().await.remove(&error_key);
                                        let _ = mgr2.bus_tx.send(ServerEvent::AttentionCleared {
                                            item_id: att_id,
                                            printer_id: Some(printer_id),
                                        });
                                        info!(printer_id, att_id, "error attention cleared");
                                    }
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!("driver_manager: event listener lagged by {n}");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }

        // ── Task 2: offline watcher ───────────────────────────────────────────
        {
            let mgr3 = Arc::clone(&mgr);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(15));
                interval.tick().await; // skip first immediate tick
                loop {
                    interval.tick().await;

                    // Collect printer_ids that have an active driver.
                    let driver_ids: Vec<i64> =
                        { mgr3.drivers.read().await.keys().copied().collect() };

                    for printer_id in driver_ids {
                        let offline_key = (printer_id, "offline".to_owned());

                        // Skip if already raised.
                        if mgr3.open_attention.read().await.contains_key(&offline_key) {
                            continue;
                        }

                        // Check if status is overdue (>60 s).
                        let overdue = {
                            let at = mgr3.last_status_at.read().await;
                            match at.get(&printer_id) {
                                None => true, // never received a status
                                Some(t) => t.elapsed() > Duration::from_secs(60),
                            }
                        };

                        if overdue {
                            let title = mgr3
                                .printer_name(printer_id)
                                .await
                                .map(|n| format!("{n}: offline"))
                                .unwrap_or_else(|| format!("Printer {printer_id}: offline"));

                            match foreman_core::repo::attention::raise(
                                &mgr3.pool,
                                Some(printer_id),
                                "offline",
                                &title,
                                Some("No status received for >60 seconds"),
                            )
                            .await
                            {
                                Ok(item) => {
                                    let att_id = item.id;
                                    mgr3.open_attention
                                        .write()
                                        .await
                                        .insert(offline_key, att_id);
                                    let _ = mgr3.bus_tx.send(ServerEvent::AttentionAdded {
                                        item: AttentionItemEvent {
                                            id: att_id,
                                            printer_id: Some(printer_id),
                                            kind: "offline".to_owned(),
                                            title: item.title,
                                            body: item.body,
                                        },
                                    });
                                    info!(printer_id, att_id, "offline attention raised");
                                }
                                Err(e) => {
                                    warn!(printer_id, "raise offline attention: {e}");
                                }
                            }
                        }
                    }
                }
            });
        }

        mgr
    }

    /// Look up a printer's display name from the DB.
    async fn printer_name(&self, printer_id: i64) -> Option<String> {
        foreman_core::repo::printer::get(&self.pool, printer_id)
            .await
            .ok()
            .map(|p| p.name)
    }

    /// Load all printers from the DB and start drivers for each.
    pub async fn start_all(&self, pool: &SqlitePool) {
        match foreman_core::repo::printer::list(pool).await {
            Ok(printers) => {
                for printer in printers {
                    if let Err(e) = self.start_driver(&printer).await {
                        warn!(
                            printer_id = printer.id,
                            name = %printer.name,
                            "failed to start driver: {e}"
                        );
                    }
                }
            }
            Err(e) => {
                warn!("failed to load printers from DB: {e}");
            }
        }
    }

    /// Start a driver for a single printer.
    pub async fn start_driver(&self, printer: &Printer) -> Result<(), String> {
        let driver: Arc<dyn PrinterDriver> = match printer.firmware {
            FirmwareKind::Moonraker => {
                let url = match &printer.primary_url {
                    Some(u) if !u.is_empty() => u.clone(),
                    _ => return Err("moonraker printer has no primary_url".into()),
                };
                let config = MoonrakerConfig {
                    printer_id: printer.id,
                    url,
                    api_key: printer.api_key.clone(),
                };
                info!(printer_id = printer.id, name = %printer.name, "starting moonraker driver");
                Arc::new(MoonrakerDriver::new(config, Arc::clone(&self.bus_tx)))
            }
            FirmwareKind::Bambu => {
                let host = match &printer.primary_url {
                    Some(u) if !u.is_empty() => u.clone(),
                    _ => return Err("bambu printer has no host (primary_url)".into()),
                };
                let serial = match &printer.serial_number {
                    Some(s) if !s.is_empty() => s.clone(),
                    _ => return Err("bambu printer has no serial_number".into()),
                };
                let access_code = match &printer.access_code {
                    Some(c) if !c.is_empty() => c.clone(),
                    _ => return Err("bambu printer has no access_code".into()),
                };
                let config = BambuConfig {
                    printer_id: printer.id,
                    host,
                    serial,
                    access_code,
                    tls_fingerprint: printer.tls_fingerprint.clone(),
                    pool: self.pool.clone(),
                };
                info!(printer_id = printer.id, name = %printer.name, "starting bambu driver");
                Arc::new(BambuDriver::new(config, Arc::clone(&self.bus_tx)))
            }
            _ => {
                return Err(format!("unsupported firmware: {:?}", printer.firmware));
            }
        };

        match driver.connect().await {
            Ok(caps) => {
                info!(
                    printer_id = printer.id,
                    bed_size = ?caps.bed_size,
                    extruders = caps.extruder_count,
                    "driver connected"
                );
                self.capabilities_cache
                    .write()
                    .await
                    .insert(printer.id, caps);
            }
            Err(e) => {
                warn!(
                    printer_id = printer.id,
                    "driver connect returned error (will retry in background): {e}"
                );
            }
        }

        self.drivers
            .write()
            .await
            .insert(printer.id, DriverHandle { driver });

        Ok(())
    }

    /// Stop the driver for a printer and clear any attention items it raised.
    pub async fn stop_driver(&self, printer_id: i64) {
        if let Some(handle) = self.drivers.write().await.remove(&printer_id) {
            info!(printer_id, "stopping driver");
            if let Err(e) = handle.driver.disconnect().await {
                warn!(printer_id, "disconnect error: {e}");
            }
        }
        self.status_cache.write().await.remove(&printer_id);
        self.capabilities_cache.write().await.remove(&printer_id);
        self.last_status_at.write().await.remove(&printer_id);

        // Clear any auto-raised attention items for this printer.
        for kind in &["offline", "error"] {
            let key = (printer_id, kind.to_string());
            if let Some(att_id) = self.open_attention.write().await.remove(&key) {
                if let Err(e) = foreman_core::repo::attention::dismiss(&self.pool, att_id).await {
                    warn!(printer_id, att_id, "clear attention on stop: {e}");
                }
            }
        }
    }

    /// Get the last-known status for a printer.
    #[allow(dead_code)]
    pub async fn get_status(&self, printer_id: i64) -> Option<PrinterStatus> {
        self.status_cache.read().await.get(&printer_id).cloned()
    }

    /// Get all cached statuses.
    pub async fn get_all_statuses(&self) -> HashMap<i64, PrinterStatus> {
        self.status_cache.read().await.clone()
    }

    /// Get cached capabilities for a printer.
    #[allow(dead_code)]
    pub async fn get_capabilities(&self, printer_id: i64) -> Option<PrinterCapabilities> {
        self.capabilities_cache
            .read()
            .await
            .get(&printer_id)
            .cloned()
    }

    /// Return the first webcam snapshot URL for a printer (from cached capabilities).
    pub async fn get_snapshot_url(&self, printer_id: i64) -> Option<String> {
        let caps = self.capabilities_cache.read().await;
        caps.get(&printer_id)
            .and_then(|c| c.webcams.iter().find_map(|w| w.snapshot_url.clone()))
    }
}
