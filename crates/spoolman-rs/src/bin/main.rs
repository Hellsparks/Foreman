//! Standalone `spoolman-rs` binary.
//!
//! Reads SPOOLMAN_* environment variables, opens (or creates) the database,
//! runs the alembic-compatible startup check, then serves the REST API.

use std::net::SocketAddr;

use axum::{
    extract::Request,
    http::{header, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
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
    let app = Router::new()
        .route("/", get(root_handler))
        .merge(api::router(&cfg.base_path).with_state(state));

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

/// `GET /` — returns JSON when the client accepts it, otherwise a minimal HTML
/// page directing users to Foreman's Filament section.
async fn root_handler(req: Request) -> Response {
    let wants_json = req
        .headers()
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("application/json"))
        .unwrap_or(false);

    if wants_json {
        let body = serde_json::json!({
            "name": "spoolman-rs",
            "version": env!("CARGO_PKG_VERSION"),
            "note": "Spoolman-compatible API. For the full Foreman interface see your Foreman server."
        });
        (
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            body.to_string(),
        )
            .into_response()
    } else {
        Html(format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
             <title>spoolman-rs</title></head><body>\
             <h1>spoolman-rs v{}</h1>\
             <p>This is the Spoolman-compatible filament inventory API.</p>\
             <p>For the full Foreman interface, navigate to your \
             <a href=\"/\">Foreman server</a>.</p>\
             <p><a href=\"/api/v1/health\">API health</a> &middot; \
             <a href=\"/api/v1/info\">API info</a></p>\
             </body></html>",
            env!("CARGO_PKG_VERSION")
        ))
        .into_response()
    }
}
