//! Standalone `spoolman-rs` binary.
//!
//! Reads SPOOLMAN_* environment variables, opens (or creates) the SQLite
//! database, runs the alembic-compatible startup check, then prints the
//! connection info and exits cleanly.  The full HTTP API is added in
//! sessions 6–10.

use spoolman_rs::{config::SpoolmanConfig, db::Db};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() {
    // Initialise tracing; RUST_LOG controls verbosity.
    fmt().with_env_filter(EnvFilter::from_default_env()).init();

    let cfg = match SpoolmanConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("configuration error: {e}");
            std::process::exit(1);
        }
    };

    tracing::info!(
        db = %cfg.sqlite_path().display(),
        host = %cfg.host,
        port = cfg.port,
        "spoolman-rs starting"
    );

    match Db::connect(&cfg).await {
        Ok(_db) => {
            tracing::info!("database ready");
            println!(
                "spoolman-rs v{} — database open at {}",
                env!("CARGO_PKG_VERSION"),
                cfg.sqlite_path().display()
            );
        }
        Err(e) => {
            tracing::error!("failed to open database: {e}");
            std::process::exit(1);
        }
    }
}
