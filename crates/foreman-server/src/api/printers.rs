//! REST endpoints for printer management.
//!
//! - `GET  /api/printers`                     — list all printers with last-known status
//! - `POST /api/printers`                     — add a new printer (starts its driver)
//! - `DELETE /api/printers/:id`               — remove a printer (stops its driver)
//! - `POST /api/printers/:id/trust-new-cert`  — clear TLS fingerprint (TOFU re-pin)

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use foreman_core::{
    domain::{CreatePrinter, Printer},
    events::PrinterStatus,
};
use serde::Serialize;

use crate::state::AppState;

/// A printer record enriched with live status and active path info.
#[derive(Debug, Serialize)]
pub struct PrinterWithStatus {
    #[serde(flatten)]
    pub printer: Printer,
    pub status: Option<PrinterStatus>,
    /// Index of the currently active connection path (0 = primary).
    pub active_path_index: Option<i64>,
    /// Display name of the currently active connection path.
    pub active_path_name: Option<String>,
}

/// `GET /api/printers`
pub async fn list_printers(State(state): State<AppState>) -> impl IntoResponse {
    let pool = &state.foreman_db.pool;
    match foreman_core::repo::printer::list(pool).await {
        Ok(printers) => {
            let statuses = state.driver_manager.get_all_statuses().await;
            let mut result: Vec<PrinterWithStatus> = Vec::with_capacity(printers.len());
            for p in printers {
                let status = statuses.get(&p.id).cloned();
                let active = state.driver_manager.get_active_path(p.id).await;
                let active_path_index = active.as_ref().map(|(i, _)| *i as i64);
                let active_path_name = active.map(|(_, path)| path.name);
                result.push(PrinterWithStatus {
                    printer: p,
                    status,
                    active_path_index,
                    active_path_name,
                });
            }
            (
                StatusCode::OK,
                Json(serde_json::to_value(result).unwrap_or_default()),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `POST /api/printers`
pub async fn create_printer(
    State(state): State<AppState>,
    Json(req): Json<CreatePrinter>,
) -> impl IntoResponse {
    let pool = &state.foreman_db.pool;
    match foreman_core::repo::printer::create(pool, req).await {
        Ok(printer) => {
            // Start the driver for the new printer.
            if let Err(e) = state.driver_manager.start_driver(&printer).await {
                tracing::warn!(
                    printer_id = printer.id,
                    "driver start failed (printer created): {e}"
                );
            }
            (
                StatusCode::CREATED,
                Json(serde_json::to_value(&printer).unwrap_or_default()),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `DELETE /api/printers/:id`
pub async fn delete_printer(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    // Stop the driver first.
    state.driver_manager.stop_driver(id).await;
    state.ssh_pool.close(id).await;

    let pool = &state.foreman_db.pool;
    match foreman_core::repo::printer::delete(pool, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/printers/:id/snapshot`
///
/// Proxies the first webcam snapshot URL found in the printer's cached
/// capabilities.  Returns 404 if no snapshot URL is available.
/// Used by the UI to refresh camera thumbnails every 10 s without exposing
/// the raw camera URL to the browser (avoids CORS issues).
pub async fn snapshot(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let Some(snapshot_url) = state.driver_manager.get_snapshot_url(id).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no snapshot URL for this printer" })),
        )
            .into_response();
    };

    match state
        .http_client
        .get(&snapshot_url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) => {
            let content_type = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("image/jpeg")
                .to_owned();
            match resp.bytes().await {
                Ok(bytes) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, content_type)],
                    bytes,
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response(),
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `POST /api/printers/:id/trust-new-cert`
///
/// Clears the stored TLS certificate fingerprint so the next connection
/// will TOFU-pin the new cert. Use when a printer's cert has changed
/// (e.g. firmware update).
pub async fn trust_new_cert(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let pool = &state.foreman_db.pool;

    // Stop the driver so it reconnects with the new cert.
    state.driver_manager.stop_driver(id).await;

    match foreman_core::repo::printer::clear_tls_fingerprint(pool, id).await {
        Ok(()) => {
            // Restart the driver — it will TOFU-pin the new cert.
            match foreman_core::repo::printer::get(pool, id).await {
                Ok(printer) => {
                    if let Err(e) = state.driver_manager.start_driver(&printer).await {
                        tracing::warn!(printer_id = id, "driver restart after trust-new-cert: {e}");
                    }
                }
                Err(e) => {
                    tracing::warn!(printer_id = id, "could not reload printer: {e}");
                }
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
