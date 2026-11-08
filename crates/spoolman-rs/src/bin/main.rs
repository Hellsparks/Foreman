//! Standalone `spoolman-rs` binary.
//!
//! Reads SPOOLMAN_* environment variables, opens (or creates) the database,
//! runs the alembic-compatible startup check, then serves the REST API.

use std::net::SocketAddr;

use spoolman_rs::{api, config::SpoolmanConfig, db::Db, state::AppState};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() {
    fmt().with_env_filter(EnvFilter::from_default_env()).init();

    let cfg = match SpoolmanConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("configuration error: {e}");
            std::process::exit(1);
        }
    };

    tracing::info!(
        host = %cfg.host,
        port = cfg.port,
        "spoolman-rs starting"
    );

    let db = match Db::connect(&cfg).await {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("failed to open database: {e}");
            std::process::exit(1);
        }
    };

    tracing::info!("database ready");

    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port)
        .parse()
        .expect("invalid listen address");

    let state = AppState::new(db, cfg.clone());
    let app = api::router(&cfg.base_path).with_state(state);

    tracing::info!(%addr, "listening");

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("failed to bind {addr}: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!("server error: {e}");
        std::process::exit(1);
    }
}
