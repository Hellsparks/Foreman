//! Route-list diff test — asserts our router exposes every path in the
//! committed fixture `tests/fixtures/routes-v0.27.1.json`.
//!
//! Methodology: build an in-process `Router` with the spoolman-rs `api::router`
//! (base_path = ""), spin up a one-shot listener, then fire a HEAD (or GET)
//! request against each expected path.  A 404 is a gap; anything else (200,
//! 405 Method Not Allowed, 400, etc.) counts as "route present".
//!
//! WebSocket-only paths (GET /api/v1/ and /api/v1/tag/scan*) will return 400
//! "missing Upgrade header" rather than 200, which is still "present".

use std::net::SocketAddr;

use axum::Router;
use serde::Deserialize;
use spoolman_rs::{api, config::SpoolmanConfig, db::Db, state::AppState};
use tempfile::TempDir;
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RouteEntry {
    method: String,
    path: String,
}

fn load_fixture() -> Vec<RouteEntry> {
    let json = include_str!("fixtures/routes-v0.27.1.json");
    serde_json::from_str(json).expect("routes-v0.27.1.json is valid")
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_expected_routes_are_registered() {
    spoolman_rs::db::install_drivers();

    let dir = TempDir::new().unwrap();
    let cfg = test_config(dir.path());
    let db = Db::connect(&cfg).await.expect("connect failed");
    let state = AppState::new(db, cfg.clone());

    let app: Router = api::router("").with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let routes = load_fixture();
    let mut missing: Vec<String> = Vec::new();

    for entry in &routes {
        // Replace Axum-style {param} with a dummy value for the test URL.
        let path = entry
            .path
            .replace("{id}", "1")
            .replace("{key}", "test_key")
            .replace("{uid}", "AABBCCDD")
            .replace("{field}", "location")
            .replace("{entity_type}", "spool")
            .replace("{reader_id}", "reader1")
            .replace("{location}", "shelf1");

        let url = format!("{base}{path}");

        // Use HEAD when possible; fall back to GET for WS-only paths.
        let method = reqwest::Method::from_bytes(entry.method.as_bytes()).unwrap();
        let resp = client.request(method, &url).send().await;

        match resp {
            Ok(r) if r.status() == 404 => {
                // Distinguish Axum routing 404 (empty body) from handler 404
                // (JSON body with "message" key — entity not found, route *does* exist).
                let body = r.text().await.unwrap_or_default();
                let is_handler_404 = body.contains("\"message\"");
                if !is_handler_404 {
                    missing.push(format!("{} {}", entry.method, entry.path));
                }
            }
            Ok(_) => {} // any non-404 means the route exists
            Err(e) => {
                missing.push(format!("{} {} (request error: {e})", entry.method, entry.path));
            }
        }
    }

    if !missing.is_empty() {
        panic!(
            "Routes missing from spoolman-rs router:\n{}",
            missing.join("\n")
        );
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_config(data_dir: &std::path::Path) -> SpoolmanConfig {
    SpoolmanConfig {
        db_type: spoolman_rs::config::DbType::Sqlite,
        db_host: None,
        db_port: None,
        db_name: None,
        db_username: None,
        db_password: None,
        data_dir: data_dir.to_path_buf(),
        logs_dir: data_dir.to_path_buf(),
        backups_dir: data_dir.join("backups"),
        host: "127.0.0.1".to_owned(),
        port: 7912,
        base_path: String::new(),
        automatic_backup: false,
    }
}
