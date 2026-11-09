//! Driver manager — spawns and manages one printer driver per registered printer.
//!
//! On server start, loads all printers from the Foreman DB and spawns a driver
//! for each one that has a `primary_url`. Adding or removing a printer via the
//! REST API starts or stops its driver.

use std::collections::HashMap;
use std::sync::Arc;

use foreman_core::{
    domain::{FirmwareKind, Printer},
    driver::{PrinterCapabilities, PrinterDriver},
    drivers::{
        bambu::{BambuConfig, BambuDriver},
        moonraker::{MoonrakerConfig, MoonrakerDriver},
    },
    events::{PrinterStatus, ServerEvent},
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
}

impl DriverManager {
    pub fn new(bus_tx: Arc<broadcast::Sender<ServerEvent>>, pool: SqlitePool) -> Arc<Self> {
        let mgr = Arc::new(Self {
            drivers: RwLock::new(HashMap::new()),
            bus_tx: Arc::clone(&bus_tx),
            pool,
            status_cache: RwLock::new(HashMap::new()),
            capabilities_cache: RwLock::new(HashMap::new()),
        });

        // Spawn a background task to listen for status events and update the cache.
        let mgr2 = Arc::clone(&mgr);
        let mut rx = bus_tx.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ServerEvent::PrinterStatus { printer_id, status }) => {
                        mgr2.status_cache.write().await.insert(printer_id, status);
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("driver_manager: event listener lagged by {n}");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        mgr
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

    /// Stop the driver for a printer.
    pub async fn stop_driver(&self, printer_id: i64) {
        if let Some(handle) = self.drivers.write().await.remove(&printer_id) {
            info!(printer_id, "stopping driver");
            if let Err(e) = handle.driver.disconnect().await {
                warn!(printer_id, "disconnect error: {e}");
            }
        }
        self.status_cache.write().await.remove(&printer_id);
        self.capabilities_cache.write().await.remove(&printer_id);
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
}
