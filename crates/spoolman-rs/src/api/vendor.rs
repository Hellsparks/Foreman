//! Vendor REST endpoints — mirrors `spoolman/api/v1/vendor.py`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    Json,
};
use serde::Deserialize;

use crate::{
    api::error::ApiResult,
    db::vendor as db,
    models::{CreateVendorRequest, UpdateVendorRequest},
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

/// GET {base_path}/api/v1/vendor
pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<VendorQuery>,
) -> ApiResult<(
    StatusCode,
    HeaderMap,
    Json<Vec<crate::models::VendorResponse>>,
)> {
    let filter = db::VendorFilter {
        name: q.name,
        sort: q.sort,
        limit: q.limit,
        offset: q.offset,
    };
    let (vendors, total) = db::find(&state.db.pool, &state.db.engine, &filter).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-total-count",
        HeaderValue::from_str(&total.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );
    Ok((StatusCode::OK, headers, Json(vendors)))
}

/// GET {base_path}/api/v1/vendor/{id}
pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<crate::models::VendorResponse>> {
    let vendor = db::find_by_id(&state.db.pool, id).await?;
    Ok(Json(vendor))
}

/// POST {base_path}/api/v1/vendor
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateVendorRequest>,
) -> ApiResult<(StatusCode, Json<crate::models::VendorResponse>)> {
    let vendor = db::create(&state.db.pool, &state.db.engine, &req).await?;
    Ok((StatusCode::CREATED, Json(vendor)))
}

/// PATCH {base_path}/api/v1/vendor/{id}
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateVendorRequest>,
) -> ApiResult<Json<crate::models::VendorResponse>> {
    let vendor = db::update(&state.db.pool, id, &req).await?;
    Ok(Json(vendor))
}

/// DELETE {base_path}/api/v1/vendor/{id}
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    db::delete(&state.db.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
