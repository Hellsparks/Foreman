//! External filament database endpoints — stub returning empty results.
//!
//! The upstream Spoolman supports downloading filament profiles from an
//! external DB hosted on GitHub (`Donkie/SpoolmanDB`).  Fetching and caching
//! that external data is deferred to session 10.  These stubs return empty
//! arrays so existing clients that query these endpoints do not error out.

use axum::{http::StatusCode, Json};
use serde::Deserialize;
use serde_json::Value;

use crate::api::error::ApiResult;

#[derive(Debug, Deserialize)]
pub struct FilamentSearchQuery {
    pub q: Option<String>,
    pub limit: Option<i64>,
}

/// GET {base_path}/api/v1/external/filament
pub async fn get_filaments() -> ApiResult<(StatusCode, Json<Vec<Value>>)> {
    Ok((StatusCode::OK, Json(vec![])))
}

/// GET {base_path}/api/v1/external/filament/search
pub async fn search_filaments(
    _query: axum::extract::Query<FilamentSearchQuery>,
) -> ApiResult<(StatusCode, Json<Vec<Value>>)> {
    Ok((StatusCode::OK, Json(vec![])))
}

/// GET {base_path}/api/v1/external/material
pub async fn get_materials() -> ApiResult<(StatusCode, Json<Vec<Value>>)> {
    Ok((StatusCode::OK, Json(vec![])))
}
