//! Filament REST endpoints — mirrors `spoolman/api/v1/filament.py`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    Json,
};
use serde::Deserialize;

use crate::{
    api::error::ApiResult,
    db::filament as db,
    models::{CreateFilamentRequest, FilamentResponse, UpdateFilamentRequest},
    state::AppState,
};

// ---------------------------------------------------------------------------
// Query params (upstream FilamentParameters)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub struct FilamentQuery {
    pub name: Option<String>,
    #[serde(rename = "vendor_id")]
    pub vendor_id: Option<String>,
    #[serde(rename = "vendor.name")]
    pub vendor_name: Option<String>,
    pub material: Option<String>,
    pub article_number: Option<String>,
    pub external_id: Option<String>,
    pub color_hex: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET {base_path}/api/v1/filament
pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<FilamentQuery>,
) -> ApiResult<(StatusCode, HeaderMap, Json<Vec<FilamentResponse>>)> {
    let filter = db::FilamentFilter {
        name: q.name,
        vendor_id: q.vendor_id,
        vendor_name: q.vendor_name,
        material: q.material,
        article_number: q.article_number,
        external_id: q.external_id,
        color_hex: q.color_hex,
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

/// GET {base_path}/api/v1/filament/{id}
pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<FilamentResponse>> {
    let filament = db::find_by_id(&state.db.pool, id).await?;
    Ok(Json(filament))
}

/// POST {base_path}/api/v1/filament
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateFilamentRequest>,
) -> ApiResult<(StatusCode, Json<FilamentResponse>)> {
    let filament = db::create(&state.db.pool, &state.db.engine, &req).await?;
    Ok((StatusCode::CREATED, Json(filament)))
}

/// PATCH {base_path}/api/v1/filament/{id}
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateFilamentRequest>,
) -> ApiResult<Json<FilamentResponse>> {
    let filament = db::update(&state.db.pool, id, &req).await?;
    Ok(Json(filament))
}

/// DELETE {base_path}/api/v1/filament/{id}
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    db::delete(&state.db.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
