//! Axum router for the spoolman-rs HTTP API.

pub mod error;
pub mod export;
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
    http::Uri,
    response::IntoResponse,
    routing::{delete, get, patch, post, put},
    Router,
};

use crate::state::AppState;

/// Extract `extra.*` query parameters from the request URI.
///
/// Serde cannot capture dynamic `extra.<key>` params in a struct because the keys
/// are not known at compile time.  We parse the raw query string directly instead.
///
/// Returns a `Vec<(key, value)>` where `key` is the part after `"extra."`.
pub fn parse_extra_params(uri: &Uri) -> Vec<(String, String)> {
    parse_prefixed_extra_params(uri, "extra.")
}

/// Extract `filament.extra.*` query parameters from the request URI.
///
/// Returns a `Vec<(key, value)>` where `key` is the part after `"filament.extra."`.
pub fn parse_filament_extra_params(uri: &Uri) -> Vec<(String, String)> {
    parse_prefixed_extra_params(uri, "filament.extra.")
}

fn parse_prefixed_extra_params(uri: &Uri, prefix: &str) -> Vec<(String, String)> {
    let query = uri.query().unwrap_or("");
    // URL-decode the query string and split into key=value pairs.
    let mut result = Vec::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = if let Some(pos) = pair.find('=') {
            (&pair[..pos], &pair[pos + 1..])
        } else {
            (pair, "")
        };
        // Percent-decode both key and value.
        let key = percent_decode(k);
        if let Some(field_key) = key.strip_prefix(prefix) {
            let value = percent_decode(v);
            result.push((field_key.to_owned(), value));
        }
    }
    result
}

/// Minimal percent-decoder: handles `%XX` sequences (including multi-byte UTF-8) and `+` → space.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                decoded.push(h * 16 + l);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            decoded.push(b' ');
            i += 1;
            continue;
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(decoded)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Build the API router mounted at `{base_path}/api/v1`.
pub fn router(base_path: &str) -> Router<AppState> {
    // Note: the root WebSocket route ("/") is NOT placed inside the nest because
    // Axum 0.8's nest() does not reliably match the stripped root path "/" when
    // accessed as `{prefix}/`.  It is registered directly on the outer router.
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
        .route("/spool/group", get(spool::find_groups))
        .route("/spool/field/{field}", patch(spool::rename_field_value))
        .route(
            "/spool/{id}",
            get(spool::get_one)
                .patch(spool::update)
                .delete(spool::delete),
        )
        .route("/spool/{id}/use", put(spool::use_spool))
        .route("/spool/{id}/measure", put(spool::measure_spool))
        // Spool tag link / unlink
        .route("/spool/{id}/tag", post(tag::link_tag))
        .route("/spool/{id}/tag/{uid}", delete(tag::unlink_tag))
        // Filament tag link / unlink
        .route("/filament/{id}/tag", post(filament::link_tag))
        .route("/filament/{id}/tag/{uid}", delete(filament::unlink_tag))
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
        // Settings (both with and without trailing slash to match upstream's FastAPI behaviour)
        .route("/setting", get(setting::get_all))
        .route("/setting/", get(setting::get_all))
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
        .route("/location/{location}", patch(other::rename_location))
        // Backup
        .route("/backup", post(other::backup))
        // Export
        .route("/export/spools", get(export::export_spools))
        .route("/export/filaments", get(export::export_filaments))
        .route("/export/vendors", get(export::export_vendors));

    // Register root WebSocket outside the nest so "/api/v1/" matches reliably.
    if base_path.is_empty() {
        Router::new()
            .route("/api/v1/", get(root_ws))
            .nest("/api/v1", api)
    } else {
        Router::new()
            .route(&format!("{base_path}/api/v1/"), get(root_ws))
            .nest(&format!("{base_path}/api/v1"), api)
    }
}

/// GET/WS {base_path}/api/v1/  — subscribe to all entity events (all types).
async fn root_ws(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.event_tx.subscribe();
    ws.on_upgrade(move |socket| ws::handle_entity_ws(socket, rx, "", None))
}
