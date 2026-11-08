//! Miscellaneous endpoints — mirrors `spoolman/api/v1/other.py` and the
//! router-level `/info` and `/health` endpoints in `spoolman/api/v1/router.py`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

use crate::{
    api::error::ApiResult,
    db::{filament as db_filament, spool as db_spool},
    models::{HealthCheck, InfoResponse, RenameLocationBody},
    state::AppState,
};

// ---------------------------------------------------------------------------
// Info / Health
// ---------------------------------------------------------------------------

pub async fn health() -> Json<HealthCheck> {
    Json(HealthCheck { status: "healthy" })
}

pub async fn info(State(state): State<AppState>) -> Json<InfoResponse> {
    let db_type = format!("{:?}", state.db.engine).to_lowercase();
    Json(InfoResponse {
        version: env!("CARGO_PKG_VERSION"),
        db_type,
    })
}

// ---------------------------------------------------------------------------
// Find helpers
// ---------------------------------------------------------------------------

pub async fn find_materials(State(state): State<AppState>) -> ApiResult<Json<Vec<String>>> {
    let materials = db_filament::find_materials(&state.db.pool).await?;
    Ok(Json(materials))
}

pub async fn find_article_numbers(State(state): State<AppState>) -> ApiResult<Json<Vec<String>>> {
    let numbers = db_filament::find_article_numbers(&state.db.pool).await?;
    Ok(Json(numbers))
}

pub async fn find_locations(State(state): State<AppState>) -> ApiResult<Json<Vec<String>>> {
    let locations = db_spool::find_locations(&state.db.pool).await?;
    Ok(Json(locations))
}

pub async fn find_lot_numbers(State(state): State<AppState>) -> ApiResult<Json<Vec<String>>> {
    let lot_numbers = db_spool::find_lot_numbers(&state.db.pool).await?;
    Ok(Json(lot_numbers))
}

// ---------------------------------------------------------------------------
// Location rename
// ---------------------------------------------------------------------------

/// PATCH {base_path}/api/v1/location/{location}
pub async fn rename_location(
    State(state): State<AppState>,
    Path(location): Path<String>,
    Json(body): Json<RenameLocationBody>,
) -> ApiResult<(StatusCode, Json<String>)> {
    db_spool::rename_location(&state.db.pool, &location, &body.name).await?;
    Ok((StatusCode::OK, Json(body.name)))
}
