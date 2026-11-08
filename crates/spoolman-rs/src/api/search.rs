//! Search endpoint — GET {base_path}/api/v1/search.
//!
//! Mirrors `spoolman/api/v1/search.py`.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;

use crate::{api::error::ApiResult, db::search, models::SearchResults, state::AppState};

// ---------------------------------------------------------------------------
// Query params (upstream SearchParameters)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    /// The search query string.
    pub q: Option<String>,
    /// Max colour distance (0–100) for colour-similarity hits.  Default: 10.
    pub color_similarity_threshold: Option<f64>,
    /// Max results per entity type.  Default: 10.
    pub limit: Option<i64>,
    /// Whether to include archived spools.  Default: false.
    pub allow_archived: Option<bool>,
    /// Spools to embed per filament result (0 = none).  Default: 1.
    pub spools_per_filament: Option<i64>,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// GET {base_path}/api/v1/search
pub async fn search_handler(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<SearchResults>> {
    let results = search::search(
        &state.db.pool,
        &state.db.engine,
        q.q.as_deref().unwrap_or(""),
        q.color_similarity_threshold.unwrap_or(10.0),
        q.limit.unwrap_or(10),
        q.allow_archived.unwrap_or(false),
        q.spools_per_filament.unwrap_or(1),
    )
    .await?;
    Ok(Json(results))
}
