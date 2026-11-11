//! Shared application state for all axum handlers.

use std::sync::Arc;

use foreman_core::{db::ForemanDb, events::ServerEvent, library::LibraryStore, secrets::SecretBox};
use tokio::sync::broadcast;

use crate::{
    auth::{AuthState, ServerInfo},
    driver_manager::DriverManager,
    scheduler::Scheduler,
    ssh_pool::SshPool,
    transfers::Transfers,
};

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
    /// Pooled SSH/SFTP sessions, one per printer.
    pub ssh_pool: Arc<SshPool>,
    /// Setup code + login rate limiting.
    pub auth: Arc<AuthState>,
    /// Server identity and bind facts.
    pub info: Arc<ServerInfo>,
    /// Library files, index and parsed facts.
    pub library: Arc<LibraryStore>,
    /// Copy/move jobs between the Library and printers.
    pub transfers: Arc<Transfers>,
    /// Printer queues + fleet queue dispatcher.
    pub scheduler: Arc<Scheduler>,
}
