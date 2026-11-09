//! Shared application state for all axum handlers.

use std::sync::Arc;

use foreman_core::{db::ForemanDb, events::ServerEvent, secrets::SecretBox};
use tokio::sync::broadcast;

use crate::driver_manager::DriverManager;

/// Shared state available to all route handlers via `State<AppState>`.
#[derive(Clone)]
pub struct AppState {
    pub foreman_db: ForemanDb,
    pub bus_tx: Arc<broadcast::Sender<ServerEvent>>,
    pub driver_manager: Arc<DriverManager>,
    /// Shared HTTP client for proxying snapshot URLs.
    pub http_client: reqwest::Client,
    /// The Foreman SSH identity: (private key, OpenSSH public key string).
    pub ssh_identity: Arc<(Arc<russh::keys::PrivateKey>, String)>,
    /// Seals/opens secrets stored in the DB (SSH passwords).
    pub secrets: Arc<SecretBox>,
}
