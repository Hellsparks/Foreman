//! Vendor REST + WebSocket endpoints — mirrors `spoolman/api/v1/vendor.py`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};

use crate::api::ws::OptWs;
use serde::Deserialize;

use crate::{
    api::error::ApiResult,
    db::vendor as db,
    models::{CreateVendorRequest, UpdateVendorRequest, VendorResponse},
    state::AppState,
};

// ---------------------------------------------------------------------------
// Query params
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub struct VendorQuery {
    pub name: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET {base_path}/api/v1/vendor  (also accepts WS upgrade → all vendor events)
pub async fn list(
    OptWs(ws): OptWs,
    State(state): State<AppState>,
    uri: Uri,
    Query(q): Query<VendorQuery>,
) -> Response {
    if let Some(ws) = ws {
        let rx = state.event_tx.subscribe();
        return ws.on_upgrade(move |socket| {
            crate::api::ws::handle_entity_ws(socket, rx, "vendor", None)
        });
    }
    list_inner(state, uri, q).await.into_response()
}

async fn list_inner(
    state: AppState,
    uri: Uri,
    q: VendorQuery,
) -> ApiResult<(StatusCode, HeaderMap, Json<Vec<VendorResponse>>)> {
    let extra_filters = crate::api::parse_extra_params(&uri);
    let filter = db::VendorFilter {
        name: q.name,
        sort: q.sort,
        limit: q.limit,
        offset: q.offset,
        extra_filters,
    };
    let (vendors, total) = db::find(&state.db.pool, &state.db.engine, &filter).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-total-count",
        HeaderValue::from_str(&total.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );
    Ok((StatusCode::OK, headers, Json(vendors)))
}

/// GET {base_path}/api/v1/vendor/{id}  (also accepts WS upgrade → events for this vendor)
pub async fn get_one(
    OptWs(ws): OptWs,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    if let Some(ws) = ws {
        let rx = state.event_tx.subscribe();
        return ws.on_upgrade(move |socket| {
            crate::api::ws::handle_entity_ws(socket, rx, "vendor", Some(id))
        });
    }
    get_one_inner(state, id).await.into_response()
}

async fn get_one_inner(state: AppState, id: i64) -> ApiResult<Json<VendorResponse>> {
    let vendor = db::find_by_id(&state.db.pool, id).await?;
    Ok(Json(vendor))
}

/// POST {base_path}/api/v1/vendor
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateVendorRequest>,
) -> ApiResult<(StatusCode, Json<VendorResponse>)> {
    let vendor = db::create(&state.db.pool, &state.db.engine, &req).await?;
    crate::ws::broadcast(&state.event_tx, "added", "vendor", vendor.id, &vendor);
    Ok((StatusCode::CREATED, Json(vendor)))
}

/// PATCH {base_path}/api/v1/vendor/{id}
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateVendorRequest>,
) -> ApiResult<Json<VendorResponse>> {
    let vendor = db::update(&state.db.pool, id, &req).await?;
    crate::ws::broadcast(&state.event_tx, "updated", "vendor", vendor.id, &vendor);
    Ok(Json(vendor))
}

/// DELETE {base_path}/api/v1/vendor/{id}
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let vendor = db::find_by_id(&state.db.pool, id).await?;
    db::delete(&state.db.pool, id).await?;
    crate::ws::broadcast(&state.event_tx, "deleted", "vendor", vendor.id, &vendor);
    Ok(StatusCode::NO_CONTENT)
}
