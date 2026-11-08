//! Shared application state for all axum handlers.

use std::sync::Arc;

use foreman_core::{db::ForemanDb, events::ServerEvent};
use tokio::sync::broadcast;

use crate::driver_manager::DriverManager;

/// Shared state available to all route handlers via `State<AppState>`.
#[derive(Clone)]
pub struct AppState {
    pub foreman_db: ForemanDb,
    pub bus_tx: Arc<broadcast::Sender<ServerEvent>>,
    pub driver_manager: Arc<DriverManager>,
}
