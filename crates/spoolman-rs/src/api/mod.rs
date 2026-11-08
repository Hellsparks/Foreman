//! Axum router for the spoolman-rs HTTP API.

pub mod error;
pub mod external;
pub mod extra_field;
pub mod filament;
pub mod other;
pub mod search;
pub mod setting;
pub mod spool;
pub mod tag;
pub mod vendor;
pub mod ws;

use axum::{
    extract::{State, WebSocketUpgrade},
    response::IntoResponse,
    routing::{delete, get, patch, post, put},
    Router,
};

use crate::state::AppState;

/// Build the API router mounted at `{base_path}/api/v1`.
pub fn router(base_path: &str) -> Router<AppState> {
    let api = Router::new()
        // Root WebSocket — subscribe to all entity events
        .route("/", get(root_ws))
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
        // Tag link / unlink
        .route("/spool/{id}/tag", post(tag::link_tag))
        .route("/spool/{id}/tag/{uid}", delete(tag::unlink_tag))
        // Tag scan relay
        .route("/tag/scan", post(tag::scan_post).get(tag::scan_ws_any))
        .route("/tag/scan/{reader_id}", get(tag::scan_ws_reader))
        .route("/tag/reader", get(tag::list_readers))
        // Search
        .route("/search", get(search::search_handler))
        // External DB stubs
        .route("/external/filament", get(external::get_filaments))
        .route("/external/filament/search", get(external::search_filaments))
        .route("/external/material", get(external::get_materials))
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

/// GET/WS {base_path}/api/v1/  — subscribe to all entity events (all types).
async fn root_ws(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.event_tx.subscribe();
    ws.on_upgrade(move |socket| ws::handle_entity_ws(socket, rx, "", None))
}
