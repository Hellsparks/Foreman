//! Axum router for the spoolman-rs HTTP API.

pub mod error;
pub mod filament;
pub mod spool;
pub mod vendor;

use axum::{routing::get, Router};

use crate::state::AppState;

/// Build the API router mounted at `{base_path}/api/v1`.
pub fn router(base_path: &str) -> Router<AppState> {
    let api = Router::new()
        // Vendor
        .route("/vendor", get(vendor::list).post(vendor::create))
        .route(
            "/vendor/{id}",
            get(vendor::get_one)
                .patch(vendor::update)
                .delete(vendor::delete),
        )
        // Filament
        .route("/filament", get(filament::list).post(filament::create))
        .route(
            "/filament/{id}",
            get(filament::get_one)
                .patch(filament::update)
                .delete(filament::delete),
        )
        // Spool
        .route("/spool", get(spool::list).post(spool::create))
        .route(
            "/spool/{id}",
            get(spool::get_one)
                .patch(spool::update)
                .delete(spool::delete),
        );

    if base_path.is_empty() {
        Router::new().nest("/api/v1", api)
    } else {
        Router::new().nest(&format!("{base_path}/api/v1"), api)
    }
}
