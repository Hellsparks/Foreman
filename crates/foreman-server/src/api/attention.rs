//! REST endpoints for attention items.
//!
//! - `GET  /api/attention`           — list active (uncleared) items
//! - `POST /api/attention/:id/dismiss` — mark an item cleared

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::state::AppState;
use foreman_core::events::ServerEvent;

/// `GET /api/attention`
pub async fn list_attention(State(state): State<AppState>) -> impl IntoResponse {
    match foreman_core::repo::attention::list_active(&state.foreman_db.pool).await {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::to_value(&items).unwrap_or_default()),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `POST /api/attention/:id/dismiss`
pub async fn dismiss_attention(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    // Load item before dismissing so we can broadcast the cleared event.
    let items = foreman_core::repo::attention::list_active(&state.foreman_db.pool).await;
    let printer_id = items
        .ok()
        .and_then(|v| v.into_iter().find(|i| i.id == id))
        .and_then(|i| i.printer_id);

    match foreman_core::repo::attention::dismiss(&state.foreman_db.pool, id).await {
        Ok(()) => {
            let _ = state.bus_tx.send(ServerEvent::AttentionCleared {
                item_id: id,
                printer_id,
            });
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/attention/count` — lightweight count for the topbar badge.
pub async fn attention_count(State(state): State<AppState>) -> impl IntoResponse {
    match foreman_core::repo::attention::list_active(&state.foreman_db.pool).await {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({ "count": items.len() })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
