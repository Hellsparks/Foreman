//! Shared application state threaded through axum handlers.

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::{
    config::SpoolmanConfig,
    db::Db,
    ws::{BroadcastEvent, ScanBroadcastEvent, ScanRelay},
};

/// Shared state passed to every handler via `axum::extract::State`.
#[derive(Clone, Debug)]
pub struct AppState {
    pub db: Arc<Db>,
    pub config: Arc<SpoolmanConfig>,
    /// Broadcast channel for entity events (vendor/filament/spool added/updated/deleted).
    pub event_tx: broadcast::Sender<BroadcastEvent>,
    /// Broadcast channel for tag scan relay events.
    pub scan_tx: broadcast::Sender<ScanBroadcastEvent>,
    /// In-memory reader registry for the scan relay.
    pub scan_relay: ScanRelay,
}

impl AppState {
    pub fn new(db: Db, config: SpoolmanConfig) -> Self {
        let (event_tx, _) = broadcast::channel(128);
        let (scan_tx, _) = broadcast::channel(128);
        AppState {
            db: Arc::new(db),
            config: Arc::new(config),
            event_tx,
            scan_tx,
            scan_relay: ScanRelay::default(),
        }
    }
}
