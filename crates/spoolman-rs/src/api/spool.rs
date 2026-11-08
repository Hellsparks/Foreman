//! Spool REST endpoints — mirrors `spoolman/api/v1/spool.py`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    Json,
};
use serde::Deserialize;

use crate::{
    api::error::ApiResult,
    db::spool as db,
    models::{CreateSpoolRequest, SpoolResponse, UpdateSpoolRequest},
    state::AppState,
};

// ---------------------------------------------------------------------------
// Query params (upstream SpoolParameters — dot-notation flattened)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub struct SpoolQuery {
    #[serde(rename = "filament.name")]
    pub filament_name: Option<String>,
    #[serde(rename = "filament.id")]
    pub filament_id: Option<String>,
    #[serde(rename = "filament.material")]
    pub filament_material: Option<String>,
    #[serde(rename = "filament.vendor.name")]
    pub filament_vendor_name: Option<String>,
    #[serde(rename = "filament.vendor.id")]
    pub filament_vendor_id: Option<String>,
    pub location: Option<String>,
    pub lot_nr: Option<String>,
    pub allow_archived: Option<bool>,
    pub first_used: Option<String>,
    pub last_used: Option<String>,
    pub registered: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET {base_path}/api/v1/spool
pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<SpoolQuery>,
) -> ApiResult<(StatusCode, HeaderMap, Json<Vec<SpoolResponse>>)> {
    let filter = db::SpoolFilter {
        filament_name: q.filament_name,
        filament_id: q.filament_id,
        filament_material: q.filament_material,
        filament_vendor_name: q.filament_vendor_name,
        filament_vendor_id: q.filament_vendor_id,
        location: q.location,
        lot_nr: q.lot_nr,
        allow_archived: q.allow_archived.unwrap_or(false),
        first_used: q.first_used,
        last_used: q.last_used,
        registered: q.registered,
        sort: q.sort,
        limit: q.limit,
        offset: q.offset,
    };
    let (items, total) = db::find(&state.db.pool, &state.db.engine, &filter).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-total-count",
        HeaderValue::from_str(&total.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );
    Ok((StatusCode::OK, headers, Json(items)))
}

/// GET {base_path}/api/v1/spool/{id}
pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<SpoolResponse>> {
    let spool = db::find_by_id(&state.db.pool, id).await?;
    Ok(Json(spool))
}

/// POST {base_path}/api/v1/spool
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateSpoolRequest>,
) -> ApiResult<(StatusCode, Json<SpoolResponse>)> {
    let spool = db::create(&state.db.pool, &state.db.engine, &req).await?;
    Ok((StatusCode::CREATED, Json(spool)))
}

/// PATCH {base_path}/api/v1/spool/{id}
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateSpoolRequest>,
) -> ApiResult<Json<SpoolResponse>> {
    let spool = db::update(&state.db.pool, id, &req).await?;
    Ok(Json(spool))
}

/// DELETE {base_path}/api/v1/spool/{id}
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    db::delete(&state.db.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
