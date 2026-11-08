//! Axum router for the spoolman-rs HTTP API.

pub mod error;
pub mod extra_field;
pub mod filament;
pub mod other;
pub mod setting;
pub mod spool;
pub mod vendor;

use axum::{
    routing::{get, patch, post, put},
    Router,
};

use crate::state::AppState;

/// Build the API router mounted at `{base_path}/api/v1`.
pub fn router(base_path: &str) -> Router<AppState> {
    let api = Router::new()
        // Health / Info
        .route("/health", get(other::health))
        .route("/info", get(other::info))
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
        )
        .route("/spool/{id}/use", put(spool::use_spool))
        .route("/spool/{id}/measure", put(spool::measure_spool))
        // Extra fields (definitions)
        .route("/field/{entity_type}", get(extra_field::get_definitions))
        .route(
            "/field/{entity_type}/{key}",
            post(extra_field::upsert_definition).delete(extra_field::delete_definition),
        )
        .route(
            "/field/{entity_type}/{key}/values",
            get(extra_field::get_values),
        )
        // Settings
        .route("/setting", get(setting::get_all))
        .route(
            "/setting/{key}",
            get(setting::get_one)
                .post(setting::set_one)
                .delete(setting::delete_one),
        )
        // Find helpers
        .route("/material", get(other::find_materials))
        .route("/article-number", get(other::find_article_numbers))
        .route("/location", get(other::find_locations))
        .route("/lot-number", get(other::find_lot_numbers))
        // Location rename
        .route("/location/{location}", patch(other::rename_location));

    if base_path.is_empty() {
        Router::new().nest("/api/v1", api)
    } else {
        Router::new().nest(&format!("{base_path}/api/v1"), api)
    }
}
