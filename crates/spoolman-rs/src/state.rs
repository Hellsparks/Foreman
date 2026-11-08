//! Shared application state threaded through axum handlers.

use std::sync::Arc;

use crate::{config::SpoolmanConfig, db::Db};

/// Shared state passed to every handler via `axum::extract::State`.
#[derive(Clone, Debug)]
pub struct AppState {
    pub db: Arc<Db>,
    pub config: Arc<SpoolmanConfig>,
}

impl AppState {
    pub fn new(db: Db, config: SpoolmanConfig) -> Self {
        AppState {
            db: Arc::new(db),
            config: Arc::new(config),
        }
    }
}
