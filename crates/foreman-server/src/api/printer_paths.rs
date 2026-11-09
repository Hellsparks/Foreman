//! REST endpoints for printer path (tiered-connection) management.
//!
//! - `GET    /api/printers/:id/paths`              — list paths (priority order)
//! - `POST   /api/printers/:id/paths`              — create path
//! - `PUT    /api/printers/:id/paths/:pid`         — update path
//! - `DELETE /api/printers/:id/paths/:pid`         — delete path
//! - `POST   /api/printers/:id/paths/reorder`      — bulk-update priorities
//! - `POST   /api/printers/:id/paths/test-all`     — TCP-ping all paths
//! - `GET    /api/printers/:id/active-path`        — currently active path index + record
//! - `POST   /api/printers/:id/paths/switch-back`  — manually switch back to path 0
//! - `PUT    /api/printers/:id`                    — update printer fields

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use foreman_core::domain::{CreatePrinterPath, UpdatePrinter, UpdatePrinterPath};
use serde::Deserialize;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// GET /api/printers/:id/paths
// ---------------------------------------------------------------------------

pub async fn list_paths(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    match foreman_core::repo::printer_path::list(&state.foreman_db.pool, id).await {
        Ok(paths) => (
            StatusCode::OK,
            Json(serde_json::to_value(paths).unwrap_or_default()),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /api/printers/:id/paths
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreatePathBody {
    pub name: String,
    pub url: String,
    pub priority: Option<i64>,
}

pub async fn create_path(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<CreatePathBody>,
) -> impl IntoResponse {
    let req = CreatePrinterPath {
        printer_id: id,
        name: body.name,
        url: body.url,
        priority: body.priority,
    };
    match foreman_core::repo::printer_path::create(&state.foreman_db.pool, req).await {
        Ok(path) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(path).unwrap_or_default()),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// PUT /api/printers/:id/paths/:pid
// ---------------------------------------------------------------------------

pub async fn update_path(
    State(state): State<AppState>,
    Path((_, pid)): Path<(i64, i64)>,
    Json(body): Json<UpdatePrinterPath>,
) -> impl IntoResponse {
    match foreman_core::repo::printer_path::update(&state.foreman_db.pool, pid, body).await {
        Ok(path) => (
            StatusCode::OK,
            Json(serde_json::to_value(path).unwrap_or_default()),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// DELETE /api/printers/:id/paths/:pid
// ---------------------------------------------------------------------------

pub async fn delete_path(
    State(state): State<AppState>,
    Path((_, pid)): Path<(i64, i64)>,
) -> impl IntoResponse {
    match foreman_core::repo::printer_path::delete(&state.foreman_db.pool, pid).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /api/printers/:id/paths/reorder
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ReorderItem {
    pub id: i64,
    pub priority: i64,
}

pub async fn reorder_paths(
    State(state): State<AppState>,
    Path(printer_id): Path<i64>,
    Json(items): Json<Vec<ReorderItem>>,
) -> impl IntoResponse {
    let pool = &state.foreman_db.pool;
    for item in &items {
        let req = UpdatePrinterPath {
            name: None,
            url: None,
            priority: Some(item.priority),
        };
        if let Err(e) = foreman_core::repo::printer_path::update(pool, item.id, req).await {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    }

    // Return updated list.
    match foreman_core::repo::printer_path::list(pool, printer_id).await {
        Ok(paths) => (
            StatusCode::OK,
            Json(serde_json::to_value(paths).unwrap_or_default()),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /api/printers/:id/paths/test-all
// ---------------------------------------------------------------------------

pub async fn test_all_paths(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let results = state.driver_manager.test_all_paths(id).await;
    (
        StatusCode::OK,
        Json(serde_json::to_value(results).unwrap_or_default()),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// GET /api/printers/:id/active-path
// ---------------------------------------------------------------------------

pub async fn get_active_path(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match state.driver_manager.get_active_path(id).await {
        Some((index, path)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "index": index,
                "path": path,
            })),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no active path for this printer" })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /api/printers/:id/paths/switch-back
// ---------------------------------------------------------------------------

pub async fn switch_back(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    match state.driver_manager.switch_back_to_primary(id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// PUT /api/printers/:id  (update printer fields)
// ---------------------------------------------------------------------------

pub async fn update_printer(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdatePrinter>,
) -> impl IntoResponse {
    let pool = &state.foreman_db.pool;
    let url_before = foreman_core::repo::printer::get(pool, id)
        .await
        .ok()
        .and_then(|p| p.primary_url);

    match foreman_core::repo::printer::update(pool, id, req).await {
        Ok(printer) => {
            // Restart driver if primary_url changed.
            let url_after = printer.primary_url.clone();
            if url_before != url_after {
                state.driver_manager.stop_driver(id).await;
                if let Err(e) = state.driver_manager.start_driver(&printer).await {
                    tracing::warn!(printer_id = id, "driver restart after update: {e}");
                }
            }
            let active = state.driver_manager.get_active_path(id).await;
            let resp = serde_json::json!({
                "id": printer.id,
                "name": printer.name,
                "firmware": printer.firmware,
                "primary_url": printer.primary_url,
                "api_key": printer.api_key,
                "serial_number": printer.serial_number,
                "access_code": printer.access_code,
                "tls_fingerprint": printer.tls_fingerprint,
                "tags": printer.tags,
                "hardened_nozzle": printer.hardened_nozzle,
                "sort_order": printer.sort_order,
                "fail_after_checks": printer.fail_after_checks,
                "return_after_secs": printer.return_after_secs,
                "created_at": printer.created_at,
                "updated_at": printer.updated_at,
                "active_path_index": active.as_ref().map(|(i, _)| *i as i64),
                "active_path_name": active.map(|(_, p)| p.name),
                "status": null,
            });
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
