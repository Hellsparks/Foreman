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
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One health-check result entry: (path_id, reachable, latency_ms).
type PathCheckEntry = (i64, bool, u64);

use foreman_core::{
    domain::{FirmwareKind, Printer, PrinterPath},
    driver::{PrinterCapabilities, PrinterCommand, PrinterDriver},
    drivers::{
        bambu::{BambuConfig, BambuDriver},
        moonraker::{MoonrakerConfig, MoonrakerDriver},
    },
    error::AppError,
    events::{
        AttentionItemEvent, ConnectionEvent, ConnectionEventKind, PrinterState, PrinterStatus,
        ServerEvent,
    },
};
use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::{broadcast, RwLock};
use tracing::{info, warn};

/// A running driver handle (type-erased).
struct DriverHandle {
    driver: Arc<dyn PrinterDriver>,
}

/// Result of a single path TCP-reachability test.
#[derive(Debug, Clone, Serialize)]
pub struct PathTestResult {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub reachable: bool,
    pub latency_ms: Option<u64>,
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
    /// When each driver was started (used for 60 s offline grace period).
    driver_started_at: RwLock<HashMap<i64, Instant>>,
    /// Map of (printer_id, kind) → attention_item_id for live items we raised.
    open_attention: RwLock<HashMap<(i64, String), i64>>,
    /// Cancel confirm tokens: printer_id → (token, expiry).
    cancel_tokens: RwLock<HashMap<i64, (String, Instant)>>,
    // ── Path failover state ───────────────────────────────────────────────
    /// Active path index per printer (0 = highest priority).
    active_path_idx: RwLock<HashMap<i64, usize>>,
    /// Consecutive health-check failures on the active path.
    path_fail_counts: RwLock<HashMap<i64, u32>>,
    /// When a higher-priority path first became consistently healthy again.
    /// keyed by printer_id → (target_path_index, since_instant).
    higher_path_healthy_since: RwLock<HashMap<i64, (usize, Instant)>>,
    /// Last health-check results: printer_id → Vec<PathCheckEntry>.
    path_health_cache: RwLock<HashMap<i64, Vec<PathCheckEntry>>>,
    /// Open "path_switched" attention items: printer_id → attention_item_id.
    path_switched_attention: RwLock<HashMap<i64, i64>>,
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
            driver_started_at: RwLock::new(HashMap::new()),
            open_attention: RwLock::new(HashMap::new()),
            cancel_tokens: RwLock::new(HashMap::new()),
            active_path_idx: RwLock::new(HashMap::new()),
            path_fail_counts: RwLock::new(HashMap::new()),
            higher_path_healthy_since: RwLock::new(HashMap::new()),
            path_health_cache: RwLock::new(HashMap::new()),
            path_switched_attention: RwLock::new(HashMap::new()),
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
                            // Only refresh the heartbeat timer when the printer is
                            // actually reachable. Offline events are self-published
                            // by the driver on disconnect — counting them would
                            // prevent the offline watcher from ever triggering.
                            if status.state != PrinterState::Offline {
                                mgr2.last_status_at
                                    .write()
                                    .await
                                    .insert(printer_id, Instant::now());
                            }

                            // ── Clear offline attention if printer is back ───────
                            // Only clear when the printer is genuinely reachable
                            // (not when the driver itself reports Offline on disconnect).
                            let offline_key = (printer_id, "offline".to_owned());
                            let offline_att_id = if status.state != PrinterState::Offline {
                                mgr2.open_attention.read().await.get(&offline_key).copied()
                            } else {
                                None
                            };
                            if let Some(att_id) = offline_att_id {
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

                        // Check if status is overdue (>60 s since last status,
                        // or >60 s since driver start if no status ever received).
                        let overdue = {
                            let at = mgr3.last_status_at.read().await;
                            match at.get(&printer_id) {
                                Some(t) => t.elapsed() > Duration::from_secs(60),
                                None => {
                                    // No status yet — use driver start time as baseline.
                                    mgr3.driver_started_at
                                        .read()
                                        .await
                                        .get(&printer_id)
                                        .is_some_and(|t| t.elapsed() > Duration::from_secs(60))
                                }
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

        // ── Task 3: path health-check + failover ─────────────────────────────
        {
            let mgr4 = Arc::clone(&mgr);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(2));
                interval.tick().await; // skip first immediate tick
                loop {
                    interval.tick().await;

                    let driver_ids: Vec<i64> =
                        { mgr4.drivers.read().await.keys().copied().collect() };

                    for printer_id in driver_ids {
                        // Load paths from DB.
                        let paths =
                            match foreman_core::repo::printer_path::list(&mgr4.pool, printer_id)
                                .await
                            {
                                Ok(p) => p,
                                Err(e) => {
                                    warn!(printer_id, "path health: load paths: {e}");
                                    continue;
                                }
                            };

                        if paths.is_empty() {
                            continue;
                        }

                        // Load printer for fail_after_checks / return_after_secs.
                        let printer =
                            match foreman_core::repo::printer::get(&mgr4.pool, printer_id).await {
                                Ok(p) => p,
                                Err(e) => {
                                    warn!(printer_id, "path health: load printer: {e}");
                                    continue;
                                }
                            };

                        // TCP-ping each path.
                        let mut health: Vec<(i64, bool, u64)> = Vec::new();
                        for path in &paths {
                            let (reachable, latency_ms) =
                                tcp_check(&path.url, Duration::from_secs(2)).await;
                            health.push((path.id, reachable, latency_ms));
                        }
                        mgr4.path_health_cache
                            .write()
                            .await
                            .insert(printer_id, health.clone());

                        let active_idx = *mgr4
                            .active_path_idx
                            .read()
                            .await
                            .get(&printer_id)
                            .unwrap_or(&0);

                        // Find health of active path (by position in sorted paths list).
                        let active_path_id = paths.get(active_idx).map(|p| p.id).unwrap_or(-1);
                        let active_healthy = health
                            .iter()
                            .find(|(id, _, _)| *id == active_path_id)
                            .map(|(_, ok, _)| *ok)
                            .unwrap_or(false);

                        // ── Active path failure tracking ──────────────────
                        if active_healthy {
                            mgr4.path_fail_counts.write().await.insert(printer_id, 0);
                        } else {
                            let fail_count = {
                                let mut counts = mgr4.path_fail_counts.write().await;
                                let c = counts.entry(printer_id).or_insert(0);
                                *c += 1;
                                *c
                            };
                            if fail_count >= printer.fail_after_checks as u32 {
                                // Try to switch to next healthy path.
                                mgr4.switch_to_next_path(printer_id, &paths, &health).await;
                            }
                        }

                        // Re-read active_idx after potential switch.
                        let active_idx = *mgr4
                            .active_path_idx
                            .read()
                            .await
                            .get(&printer_id)
                            .unwrap_or(&0);

                        // ── Higher-path return tracking ───────────────────
                        if active_idx > 0 {
                            // Find the best (lowest index) healthy path < active_idx.
                            let best_higher = health
                                .iter()
                                .enumerate()
                                .filter(|(i, (_, ok, _))| *i < active_idx && *ok)
                                .map(|(i, _)| i)
                                .next();

                            match best_higher {
                                Some(target_idx) => {
                                    let return_after =
                                        Duration::from_secs(printer.return_after_secs as u64);
                                    let should_return = {
                                        let hps = mgr4.higher_path_healthy_since.read().await;
                                        match hps.get(&printer_id) {
                                            Some((stored_idx, since))
                                                if *stored_idx == target_idx =>
                                            {
                                                since.elapsed() >= return_after
                                            }
                                            _ => false,
                                        }
                                    };

                                    if should_return {
                                        mgr4.higher_path_healthy_since
                                            .write()
                                            .await
                                            .remove(&printer_id);
                                        mgr4.switch_to_path(printer_id, target_idx, &paths).await;
                                    } else {
                                        // Record if not already tracking this target.
                                        let already_tracking = mgr4
                                            .higher_path_healthy_since
                                            .read()
                                            .await
                                            .get(&printer_id)
                                            .map(|(i, _)| *i == target_idx)
                                            .unwrap_or(false);
                                        if !already_tracking {
                                            mgr4.higher_path_healthy_since
                                                .write()
                                                .await
                                                .insert(printer_id, (target_idx, Instant::now()));
                                        }
                                    }
                                }
                                None => {
                                    // No higher path healthy — clear tracking.
                                    mgr4.higher_path_healthy_since
                                        .write()
                                        .await
                                        .remove(&printer_id);
                                }
                            }
                        } else {
                            // On primary — clear any higher-path tracking.
                            mgr4.higher_path_healthy_since
                                .write()
                                .await
                                .remove(&printer_id);
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

    // ── Path failover methods ─────────────────────────────────────────────

    /// Switch to the specified path index (0-based position in priority-sorted list).
    async fn switch_to_path(&self, printer_id: i64, new_idx: usize, paths: &[PrinterPath]) {
        let Some(path) = paths.get(new_idx) else {
            warn!(printer_id, new_idx, "switch_to_path: index out of range");
            return;
        };

        // Update active index and reset fail count.
        self.active_path_idx
            .write()
            .await
            .insert(printer_id, new_idx);
        self.path_fail_counts.write().await.insert(printer_id, 0);

        // Restart driver with new URL.
        if let Some(handle) = self.drivers.read().await.get(&printer_id) {
            if let Err(e) = handle.driver.disconnect().await {
                warn!(printer_id, "switch_to_path disconnect: {e}");
            }
        }

        // Build a minimal Printer to restart with override URL.
        match foreman_core::repo::printer::get(&self.pool, printer_id).await {
            Ok(mut printer) => {
                printer.primary_url = Some(path.url.clone());
                if let Err(e) = self.start_driver(&printer).await {
                    warn!(printer_id, "switch_to_path: driver restart failed: {e}");
                }
            }
            Err(e) => {
                warn!(printer_id, "switch_to_path: load printer: {e}");
                return;
            }
        }

        // Emit connection event.
        let _ = self.bus_tx.send(ServerEvent::PrinterConnection {
            printer_id,
            event: ConnectionEvent {
                kind: ConnectionEventKind::PathSwitched,
                active_path: Some(path.name.clone()),
            },
        });

        // Manage path_switched attention.
        // Dismiss any existing path_switched attention.
        if let Some(att_id) = self
            .path_switched_attention
            .read()
            .await
            .get(&printer_id)
            .copied()
        {
            if let Err(e) = foreman_core::repo::attention::dismiss(&self.pool, att_id).await {
                warn!(
                    printer_id,
                    att_id, "switch_to_path: dismiss old attention: {e}"
                );
            } else {
                let _ = self.bus_tx.send(ServerEvent::AttentionCleared {
                    item_id: att_id,
                    printer_id: Some(printer_id),
                });
                self.path_switched_attention
                    .write()
                    .await
                    .remove(&printer_id);
            }
        }

        if new_idx > 0 {
            // Raise new path_switched attention.
            let title = self
                .printer_name(printer_id)
                .await
                .map(|n| format!("{n}: using backup path: {}", path.name))
                .unwrap_or_else(|| {
                    format!("Printer {printer_id}: using backup path: {}", path.name)
                });

            match foreman_core::repo::attention::raise(
                &self.pool,
                Some(printer_id),
                "path_switched",
                &title,
                None,
            )
            .await
            {
                Ok(item) => {
                    let att_id = item.id;
                    self.path_switched_attention
                        .write()
                        .await
                        .insert(printer_id, att_id);
                    let _ = self.bus_tx.send(ServerEvent::AttentionAdded {
                        item: AttentionItemEvent {
                            id: att_id,
                            printer_id: Some(printer_id),
                            kind: "path_switched".to_owned(),
                            title: item.title,
                            body: item.body,
                        },
                    });
                    info!(printer_id, att_id, path = %path.name, "path_switched attention raised");
                }
                Err(e) => {
                    warn!(printer_id, "switch_to_path: raise attention: {e}");
                }
            }
        } else {
            info!(printer_id, "switched back to primary path");
        }
    }

    /// Find the next healthy path (index > current) and switch to it.
    async fn switch_to_next_path(
        &self,
        printer_id: i64,
        paths: &[PrinterPath],
        health: &[(i64, bool, u64)],
    ) {
        let current_idx = *self
            .active_path_idx
            .read()
            .await
            .get(&printer_id)
            .unwrap_or(&0);

        // Find first healthy path after the current one.
        let next = health
            .iter()
            .enumerate()
            .filter(|(i, (_, ok, _))| *i > current_idx && *ok)
            .map(|(i, _)| i)
            .next();

        if let Some(next_idx) = next {
            info!(
                printer_id,
                current_idx, next_idx, "switching to next path after failures"
            );
            self.switch_to_path(printer_id, next_idx, paths).await;
        } else {
            warn!(
                printer_id,
                current_idx, "no healthy next path found; staying on current"
            );
        }
    }

    /// Return the current active path index + record for a printer.
    pub async fn get_active_path(&self, printer_id: i64) -> Option<(usize, PrinterPath)> {
        let idx = *self
            .active_path_idx
            .read()
            .await
            .get(&printer_id)
            .unwrap_or(&0);

        let paths = foreman_core::repo::printer_path::list(&self.pool, printer_id)
            .await
            .ok()?;

        paths.into_iter().nth(idx).map(|p| (idx, p))
    }

    /// TCP-ping all paths for a printer and return results.
    pub async fn test_all_paths(&self, printer_id: i64) -> Vec<PathTestResult> {
        let paths = foreman_core::repo::printer_path::list(&self.pool, printer_id)
            .await
            .unwrap_or_default();

        let mut results = Vec::new();
        for path in paths {
            let (reachable, latency_ms) = tcp_check(&path.url, Duration::from_secs(5)).await;
            results.push(PathTestResult {
                id: path.id,
                name: path.name,
                url: path.url,
                reachable,
                latency_ms: if reachable { Some(latency_ms) } else { None },
            });
        }
        results
    }

    /// Manually switch back to path 0, only if path 0 is healthy.
    pub async fn switch_back_to_primary(&self, printer_id: i64) -> Result<(), AppError> {
        let paths = foreman_core::repo::printer_path::list(&self.pool, printer_id).await?;
        if paths.is_empty() {
            return Err(AppError::InvalidInput(
                "no paths defined for this printer".into(),
            ));
        }

        let first = &paths[0];
        let (reachable, _) = tcp_check(&first.url, Duration::from_secs(5)).await;
        if !reachable {
            return Err(AppError::InvalidInput(
                "path 0 is not reachable; cannot switch back".into(),
            ));
        }

        self.switch_to_path(printer_id, 0, &paths).await;
        Ok(())
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
        // Load paths from DB; use the active-path URL if available.
        let active_idx = *self
            .active_path_idx
            .read()
            .await
            .get(&printer.id)
            .unwrap_or(&0);

        let paths = foreman_core::repo::printer_path::list(&self.pool, printer.id)
            .await
            .unwrap_or_default();

        let effective_url: Option<String> = paths
            .get(active_idx)
            .map(|p| p.url.clone())
            .or_else(|| printer.primary_url.clone());

        let driver: Arc<dyn PrinterDriver> = match printer.firmware {
            FirmwareKind::Moonraker => {
                let url = match effective_url {
                    Some(u) if !u.is_empty() => u,
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
                let host = match effective_url {
                    Some(u) if !u.is_empty() => u,
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

        self.driver_started_at
            .write()
            .await
            .insert(printer.id, Instant::now());
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
        self.driver_started_at.write().await.remove(&printer_id);

        // Clear any auto-raised attention items for this printer.
        for kind in &["offline", "error"] {
            let key = (printer_id, kind.to_string());
            if let Some(att_id) = self.open_attention.write().await.remove(&key) {
                if let Err(e) = foreman_core::repo::attention::dismiss(&self.pool, att_id).await {
                    warn!(printer_id, att_id, "clear attention on stop: {e}");
                }
            }
        }

        // Clear path failover state.
        self.active_path_idx.write().await.remove(&printer_id);
        self.path_fail_counts.write().await.remove(&printer_id);
        self.higher_path_healthy_since
            .write()
            .await
            .remove(&printer_id);
        self.path_health_cache.write().await.remove(&printer_id);

        if let Some(att_id) = self
            .path_switched_attention
            .write()
            .await
            .remove(&printer_id)
        {
            if let Err(e) = foreman_core::repo::attention::dismiss(&self.pool, att_id).await {
                warn!(
                    printer_id,
                    att_id, "clear path_switched attention on stop: {e}"
                );
            }
        }
    }

    /// Get the last-known status for a printer.
    pub async fn get_status(&self, printer_id: i64) -> Option<PrinterStatus> {
        self.status_cache.read().await.get(&printer_id).cloned()
    }

    /// Get all cached statuses.
    pub async fn get_all_statuses(&self) -> HashMap<i64, PrinterStatus> {
        self.status_cache.read().await.clone()
    }

    /// Get cached capabilities for a printer.
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

    // ── Command dispatch with safety checks ──────────────────────────────

    /// Check whether a command is blocked while the printer is printing.
    /// Movement, homing, motors-off are refused. BabyStep is allowed.
    fn is_blocked_while_printing(cmd: &PrinterCommand) -> bool {
        matches!(
            cmd,
            PrinterCommand::Home { .. }
                | PrinterCommand::Jog { .. }
                | PrinterCommand::MotorsOff
                | PrinterCommand::Extrude { .. }
        )
    }

    /// Request a cancel-confirm token for a printer. Returns the token string.
    /// The caller must send this token back with the actual cancel command
    /// within 30 seconds.
    pub async fn request_cancel_token(&self, printer_id: i64) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        let expiry = Instant::now() + Duration::from_secs(30);
        self.cancel_tokens
            .write()
            .await
            .insert(printer_id, (token.clone(), expiry));
        token
    }

    /// Validate and consume a cancel token.
    async fn validate_cancel_token(&self, printer_id: i64, token: &str) -> Result<(), AppError> {
        let mut tokens = self.cancel_tokens.write().await;
        match tokens.remove(&printer_id) {
            Some((stored, expiry)) if stored == token && Instant::now() < expiry => Ok(()),
            Some(_) => Err(AppError::InvalidInput(
                "cancel token expired or invalid".into(),
            )),
            None => Err(AppError::InvalidInput(
                "no cancel token issued — request one first".into(),
            )),
        }
    }

    /// Send a command to a printer with safety checks and audit logging.
    ///
    /// `confirm_token` is required for Cancel commands. Pass `None` for all others.
    #[cfg(test)]
    pub async fn send_command(
        &self,
        printer_id: i64,
        cmd: PrinterCommand,
        confirm_token: Option<&str>,
    ) -> Result<(), AppError> {
        self.send_command_as(printer_id, cmd, confirm_token, None)
            .await
    }

    /// [`send_command`](Self::send_command) recording `user` in the audit log.
    pub async fn send_command_as(
        &self,
        printer_id: i64,
        cmd: PrinterCommand,
        confirm_token: Option<&str>,
        user: Option<&str>,
    ) -> Result<(), AppError> {
        // Look up the driver.
        let driver = {
            let drivers = self.drivers.read().await;
            drivers
                .get(&printer_id)
                .map(|h| Arc::clone(&h.driver))
                .ok_or_else(|| AppError::NotFound(format!("no driver for printer {printer_id}")))?
        };

        // Safety: refuse movement commands while printing.
        if Self::is_blocked_while_printing(&cmd) {
            let status = self.status_cache.read().await;
            if let Some(s) = status.get(&printer_id) {
                if s.state == PrinterState::Printing || s.state == PrinterState::Paused {
                    return Err(AppError::InvalidInput(
                        "command blocked: printer is printing".into(),
                    ));
                }
            }
        }

        // Cancel requires a confirm token.
        if matches!(cmd, PrinterCommand::Cancel) {
            match confirm_token {
                Some(tok) => self.validate_cancel_token(printer_id, tok).await?,
                None => {
                    return Err(AppError::InvalidInput(
                        "cancel requires a confirm_token — POST to /confirm-cancel first".into(),
                    ))
                }
            }
        }

        // Audit log.
        let cmd_desc = format!("{cmd:?}");
        if let Err(e) =
            foreman_core::repo::audit_log::insert(&self.pool, Some(printer_id), user, &cmd_desc)
                .await
        {
            warn!(printer_id, "audit_log insert failed: {e}");
        }

        // Dispatch.
        driver.send_command(cmd).await
    }
}

// ---------------------------------------------------------------------------
// TCP health-check helpers
// ---------------------------------------------------------------------------

/// Parse `host:port` from a URL string.
/// Falls back to bare host:port or host:80.
fn parse_host_port(url: &str) -> Option<String> {
    let url = url.trim();
    if let Ok(parsed) = url::Url::parse(url) {
        let host = parsed.host_str()?;
        let port = parsed.port_or_known_default()?;
        return Some(format!("{host}:{port}"));
    }
    if url.contains(':') {
        return Some(url.to_owned());
    }
    Some(format!("{url}:80"))
}

/// Attempt a TCP connection to `url`, returning `(reachable, latency_ms)`.
async fn tcp_check(url: &str, timeout: Duration) -> (bool, u64) {
    let Some(addr_str) = parse_host_port(url) else {
        return (false, 0);
    };

    // Resolve DNS synchronously on a blocking thread to avoid blocking the runtime.
    let addrs: Vec<std::net::SocketAddr> = match tokio::task::spawn_blocking({
        let addr_str = addr_str.clone();
        move || addr_str.to_socket_addrs().map(|a| a.collect::<Vec<_>>())
    })
    .await
    {
        Ok(Ok(a)) if !a.is_empty() => a,
        _ => return (false, 0),
    };

    let start = Instant::now();
    for addr in addrs {
        match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await {
            Ok(Ok(_)) => {
                return (true, start.elapsed().as_millis() as u64);
            }
            _ => continue,
        }
    }
    (false, 0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use foreman_core::{
        db::ForemanDb,
        driver::{PrinterCapabilities, PrinterCommand, PrinterDriver, PrinterFile},
        events::{PrinterState, PrinterStatus, ServerEvent},
    };
    use tempfile::TempDir;
    use tokio::sync::broadcast;

    /// Mock driver that records commands it receives.
    struct MockDriver {
        commands: tokio::sync::Mutex<Vec<String>>,
    }

    impl MockDriver {
        fn new() -> Self {
            Self {
                commands: tokio::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl PrinterDriver for MockDriver {
        async fn connect(&self) -> Result<PrinterCapabilities, AppError> {
            Ok(PrinterCapabilities::default())
        }
        async fn disconnect(&self) -> Result<(), AppError> {
            Ok(())
        }
        async fn probe_capabilities(&self) -> Result<PrinterCapabilities, AppError> {
            Ok(PrinterCapabilities::default())
        }
        async fn send_command(&self, cmd: PrinterCommand) -> Result<(), AppError> {
            self.commands.lock().await.push(format!("{cmd:?}"));
            Ok(())
        }
        async fn list_files(&self, _path: Option<&str>) -> Result<Vec<PrinterFile>, AppError> {
            Ok(vec![])
        }
        async fn upload_file(&self, _path: &str, _data: Vec<u8>) -> Result<(), AppError> {
            Ok(())
        }
        async fn delete_file(&self, _path: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn start_print(&self, _path: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn current_status(&self) -> Result<PrinterStatus, AppError> {
            Ok(make_status(PrinterState::Idle))
        }
    }

    fn make_status(state: PrinterState) -> PrinterStatus {
        PrinterStatus {
            state,
            nozzle: None,
            bed: None,
            progress: None,
            filename: None,
            print_duration: None,
            display_message: None,
            layer: None,
            total_layers: None,
            remaining_time: None,
            fans: None,
            ams_trays: vec![],
            hms_codes: vec![],
        }
    }

    #[tokio::test]
    async fn test_error_attention_raised_and_cleared() {
        let tmp = TempDir::new().unwrap();
        let db = ForemanDb::connect(&tmp.path().join("test.db"))
            .await
            .unwrap();

        let (tx, _) = broadcast::channel::<ServerEvent>(256);
        let tx = Arc::new(tx);

        // Insert a minimal printer row so the FK constraint in attention_item is satisfied.
        sqlx::query(
            "INSERT INTO printer (id, name, firmware, tags, created_at, updated_at) \
             VALUES (999, 'test-printer', 'moonraker', '[]', '2026-01-01', '2026-01-01')",
        )
        .execute(&db.pool)
        .await
        .unwrap();

        let _mgr = DriverManager::new(Arc::clone(&tx), db.pool.clone());

        // Trigger an error state.
        tx.send(ServerEvent::PrinterStatus {
            printer_id: 999,
            status: make_status(PrinterState::Error),
        })
        .unwrap();

        // Let the event listener task process it.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let items = foreman_core::repo::attention::list_active(&db.pool)
            .await
            .unwrap();
        assert!(
            items
                .iter()
                .any(|i| i.printer_id == Some(999) && i.kind == "error"),
            "error attention should have been raised"
        );

        // Send a non-error state to clear the attention.
        tx.send(ServerEvent::PrinterStatus {
            printer_id: 999,
            status: make_status(PrinterState::Idle),
        })
        .unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;

        let items = foreman_core::repo::attention::list_active(&db.pool)
            .await
            .unwrap();
        assert!(
            !items
                .iter()
                .any(|i| i.printer_id == Some(999) && i.kind == "error"),
            "error attention should have been cleared after recovery"
        );
    }

    // ── Safety tests ─────────────────────────────────────────────────

    #[test]
    fn blocked_while_printing_covers_movement() {
        // These should be blocked while printing:
        assert!(DriverManager::is_blocked_while_printing(
            &PrinterCommand::Home { axes: vec![] }
        ));
        assert!(DriverManager::is_blocked_while_printing(
            &PrinterCommand::Jog {
                x: Some(10.0),
                y: None,
                z: None,
                speed: None
            }
        ));
        assert!(DriverManager::is_blocked_while_printing(
            &PrinterCommand::MotorsOff
        ));
        assert!(DriverManager::is_blocked_while_printing(
            &PrinterCommand::Extrude {
                length: 10.0,
                speed: 300.0
            }
        ));

        // These should be allowed while printing:
        assert!(!DriverManager::is_blocked_while_printing(
            &PrinterCommand::Pause
        ));
        assert!(!DriverManager::is_blocked_while_printing(
            &PrinterCommand::Resume
        ));
        assert!(!DriverManager::is_blocked_while_printing(
            &PrinterCommand::Cancel
        ));
        assert!(!DriverManager::is_blocked_while_printing(
            &PrinterCommand::BabyStep { offset: 0.01 }
        ));
        assert!(!DriverManager::is_blocked_while_printing(
            &PrinterCommand::SetNozzleTemp { target: 200.0 }
        ));
        assert!(!DriverManager::is_blocked_while_printing(
            &PrinterCommand::SetBedTemp { target: 60.0 }
        ));
        assert!(!DriverManager::is_blocked_while_printing(
            &PrinterCommand::Gcode {
                command: "M117 test".to_owned()
            }
        ));
        assert!(!DriverManager::is_blocked_while_printing(
            &PrinterCommand::Macro {
                name: "TEST".to_owned()
            }
        ));
    }

    /// Helper: create a DriverManager with a mock driver pre-inserted.
    async fn setup_with_mock() -> (Arc<DriverManager>, Arc<MockDriver>, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db = ForemanDb::connect(&tmp.path().join("test.db"))
            .await
            .unwrap();

        let (tx, _) = broadcast::channel::<ServerEvent>(256);
        let tx = Arc::new(tx);
        let mgr = DriverManager::new(Arc::clone(&tx), db.pool.clone());

        // Insert a printer row.
        sqlx::query(
            "INSERT INTO printer (id, name, firmware, tags, created_at, updated_at) \
             VALUES (1, 'test-printer', 'moonraker', '[]', '2026-01-01', '2026-01-01')",
        )
        .execute(&db.pool)
        .await
        .unwrap();

        // Insert mock driver directly.
        let mock = Arc::new(MockDriver::new());
        mgr.drivers.write().await.insert(
            1,
            DriverHandle {
                driver: Arc::clone(&mock) as Arc<dyn PrinterDriver>,
            },
        );

        (mgr, mock, tmp)
    }

    #[tokio::test]
    async fn send_command_dispatches_to_driver() {
        let (mgr, mock, _tmp) = setup_with_mock().await;

        mgr.send_command(1, PrinterCommand::SetNozzleTemp { target: 200.0 }, None)
            .await
            .unwrap();

        let cmds = mock.commands.lock().await;
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].contains("SetNozzleTemp"));
    }

    #[tokio::test]
    async fn movement_refused_while_printing() {
        let (mgr, _mock, _tmp) = setup_with_mock().await;

        // Set printing state.
        mgr.status_cache
            .write()
            .await
            .insert(1, make_status(PrinterState::Printing));

        let err = mgr
            .send_command(
                1,
                PrinterCommand::Jog {
                    x: Some(10.0),
                    y: None,
                    z: None,
                    speed: None,
                },
                None,
            )
            .await;
        assert!(err.is_err(), "jog should be refused while printing");
        assert!(
            err.unwrap_err().to_string().contains("printing"),
            "error should mention printing"
        );
    }

    #[tokio::test]
    async fn babystep_allowed_while_printing() {
        let (mgr, mock, _tmp) = setup_with_mock().await;

        mgr.status_cache
            .write()
            .await
            .insert(1, make_status(PrinterState::Printing));

        mgr.send_command(1, PrinterCommand::BabyStep { offset: 0.01 }, None)
            .await
            .unwrap();

        let cmds = mock.commands.lock().await;
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].contains("BabyStep"));
    }

    #[tokio::test]
    async fn cancel_requires_confirm_token() {
        let (mgr, _mock, _tmp) = setup_with_mock().await;

        // Cancel without token should fail.
        let err = mgr.send_command(1, PrinterCommand::Cancel, None).await;
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("confirm_token"));

        // Cancel with correct token should succeed.
        let token = mgr.request_cancel_token(1).await;
        mgr.send_command(1, PrinterCommand::Cancel, Some(&token))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cancel_token_is_single_use() {
        let (mgr, _mock, _tmp) = setup_with_mock().await;

        let token = mgr.request_cancel_token(1).await;
        // First use succeeds.
        mgr.send_command(1, PrinterCommand::Cancel, Some(&token))
            .await
            .unwrap();
        // Second use fails (consumed).
        let err = mgr
            .send_command(1, PrinterCommand::Cancel, Some(&token))
            .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn send_command_logs_to_audit() {
        let (mgr, _mock, tmp) = setup_with_mock().await;

        mgr.send_command(1, PrinterCommand::Pause, None)
            .await
            .unwrap();

        // Check audit_log table.
        let pool = &mgr.pool;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE printer_id = 1")
            .fetch_one(pool)
            .await
            .unwrap();
        assert!(count >= 1, "audit log should have an entry");

        drop(tmp);
    }

    // ── Path failover tests ───────────────────────────────────────────

    /// Helper: set up DB + DriverManager with two paths for printer 1,
    /// and printer configured with fail_after_checks=3 / return_after_secs=1.
    async fn setup_with_paths() -> (Arc<DriverManager>, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db = ForemanDb::connect(&tmp.path().join("test.db"))
            .await
            .unwrap();

        let (tx, _) = broadcast::channel::<ServerEvent>(256);
        let tx = Arc::new(tx);
        let mgr = DriverManager::new(Arc::clone(&tx), db.pool.clone());

        // Insert a printer with quick failover settings.
        sqlx::query(
            "INSERT INTO printer \
             (id, name, firmware, primary_url, tags, fail_after_checks, return_after_secs, \
              created_at, updated_at) \
             VALUES (1, 'test-printer', 'moonraker', 'http://10.0.0.1', '[]', 3, 1, \
                     '2026-01-01', '2026-01-01')",
        )
        .execute(&db.pool)
        .await
        .unwrap();

        // Insert two paths: path 0 (priority 0) and path 1 (priority 10).
        sqlx::query(
            "INSERT INTO printer_path (id, printer_id, name, url, priority, created_at, updated_at) \
             VALUES (1, 1, 'Primary', 'http://10.0.0.1', 0, '2026-01-01', '2026-01-01'), \
                    (2, 1, 'Tailscale', 'http://100.64.0.1', 10, '2026-01-01', '2026-01-01')",
        )
        .execute(&db.pool)
        .await
        .unwrap();

        // Insert mock driver.
        let mock = Arc::new(MockDriver::new());
        mgr.drivers.write().await.insert(
            1,
            DriverHandle {
                driver: Arc::clone(&mock) as Arc<dyn PrinterDriver>,
            },
        );

        (mgr, tmp)
    }

    #[tokio::test]
    async fn test_path_failover_switches_after_3_fails() {
        let (mgr, _tmp) = setup_with_paths().await;

        // Active path starts at 0.
        assert_eq!(*mgr.active_path_idx.read().await.get(&1).unwrap_or(&0), 0);

        // Build "all healthy" health vec.
        let paths = foreman_core::repo::printer_path::list(&mgr.pool, 1)
            .await
            .unwrap();
        let path0_id = paths[0].id;
        let path1_id = paths[1].id;

        // Simulate active path (path 0) failing 3 times.
        // health: path 0 DOWN, path 1 UP.
        let health = vec![(path0_id, false, 0u64), (path1_id, true, 15u64)];

        for _ in 0..3 {
            let count = {
                let mut counts = mgr.path_fail_counts.write().await;
                let c = counts.entry(1).or_insert(0);
                *c += 1;
                *c
            };

            let printer = foreman_core::repo::printer::get(&mgr.pool, 1)
                .await
                .unwrap();

            if count >= printer.fail_after_checks as u32 {
                mgr.switch_to_next_path(1, &paths, &health).await;
                break;
            }
        }

        // Should have switched to path 1.
        let active_idx = *mgr.active_path_idx.read().await.get(&1).unwrap_or(&0);
        assert_eq!(
            active_idx, 1,
            "should have switched to path 1 after 3 failures"
        );

        // Fail count should be reset.
        let fail_count = *mgr.path_fail_counts.read().await.get(&1).unwrap_or(&0);
        assert_eq!(fail_count, 0, "fail count should reset after switch");

        // path_switched attention should have been raised.
        let attn = foreman_core::repo::attention::list_active(&mgr.pool)
            .await
            .unwrap();
        assert!(
            attn.iter()
                .any(|a| a.printer_id == Some(1) && a.kind == "path_switched"),
            "path_switched attention should be raised"
        );
    }

    #[tokio::test]
    async fn test_path_return_after_healthy() {
        let (mgr, _tmp) = setup_with_paths().await;

        // Start on path 1 (simulating a prior failover).
        mgr.active_path_idx.write().await.insert(1, 1);

        let paths = foreman_core::repo::printer_path::list(&mgr.pool, 1)
            .await
            .unwrap();

        // Record that path 0 (higher priority) has been healthy for 2 seconds.
        // return_after_secs = 1 so this should trigger a return.
        let since = Instant::now() - Duration::from_secs(2);
        mgr.higher_path_healthy_since
            .write()
            .await
            .insert(1, (0, since));

        // Trigger the return logic manually.
        let return_after = Duration::from_secs(1); // matches printer's return_after_secs
        let should_return = {
            let hps = mgr.higher_path_healthy_since.read().await;
            match hps.get(&1) {
                Some((stored_idx, s)) if *stored_idx == 0 => s.elapsed() >= return_after,
                _ => false,
            }
        };

        assert!(should_return, "should decide to return to path 0");

        mgr.higher_path_healthy_since.write().await.remove(&1);
        mgr.switch_to_path(1, 0, &paths).await;

        let active_idx = *mgr.active_path_idx.read().await.get(&1).unwrap_or(&1);
        assert_eq!(active_idx, 0, "should have returned to path 0");

        // path_switched attention should be cleared (no new one for idx 0).
        let attn = foreman_core::repo::attention::list_active(&mgr.pool)
            .await
            .unwrap();
        assert!(
            !attn
                .iter()
                .any(|a| a.printer_id == Some(1) && a.kind == "path_switched"),
            "path_switched attention should be cleared after return to primary"
        );
    }

    #[tokio::test]
    async fn test_manual_switch_back() {
        let (mgr, _tmp) = setup_with_paths().await;

        // Start on path 1.
        mgr.active_path_idx.write().await.insert(1, 1);

        let paths = foreman_core::repo::printer_path::list(&mgr.pool, 1)
            .await
            .unwrap();

        // Manual switch back to 0.
        mgr.switch_to_path(1, 0, &paths).await;

        let active_idx = *mgr.active_path_idx.read().await.get(&1).unwrap_or(&1);
        assert_eq!(active_idx, 0, "manual switch_to_path(0) should work");
    }
}
