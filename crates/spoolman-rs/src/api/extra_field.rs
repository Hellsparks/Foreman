//! Extra field endpoints — mirrors `spoolman/api/v1/field.py`.
//!
//! Extra field DEFINITIONS live in the `setting` table as JSON arrays.
//! Extra field VALUES live in `{entity}_field` tables.

use axum::{
    extract::{Path, State},
    Json,
};

use crate::{
    api::error::ApiResult,
    db::extra_field as db,
    error::SpoolmanError,
    models::{ExtraField, ExtraFieldParams},
    state::AppState,
};

fn validate_entity_type(entity_type: &str) -> ApiResult<()> {
    if matches!(entity_type, "vendor" | "filament" | "spool") {
        Ok(())
    } else {
        Err(SpoolmanError::NotFound(format!(
            "unknown entity type '{entity_type}'; must be vendor, filament, or spool"
        ))
        .into())
    }
}

/// GET {base_path}/api/v1/field/{entity_type}
pub async fn get_definitions(
    State(state): State<AppState>,
    Path(entity_type): Path<String>,
) -> ApiResult<Json<Vec<ExtraField>>> {
    validate_entity_type(&entity_type)?;
    let fields = db::get_definitions(&state.db.pool, &entity_type).await?;
    Ok(Json(fields))
}

/// GET {base_path}/api/v1/field/{entity_type}/{key}/values
pub async fn get_values(
    State(state): State<AppState>,
    Path((entity_type, key)): Path<(String, String)>,
) -> ApiResult<Json<Vec<String>>> {
    validate_entity_type(&entity_type)?;
    let fields = db::get_definitions(&state.db.pool, &entity_type).await?;
    if !fields.iter().any(|f| f.key == key) {
        return Err(SpoolmanError::NotFound(format!(
            "extra field '{key}' not found for entity type '{entity_type}'"
        ))
        .into());
    }
    let table = format!("{entity_type}_field");
    let values = db::get_distinct_values(&state.db.pool, &table, &key).await?;
    Ok(Json(values))
}

/// POST {base_path}/api/v1/field/{entity_type}/{key}
pub async fn upsert_definition(
    State(state): State<AppState>,
    Path((entity_type, key)): Path<(String, String)>,
    Json(params): Json<ExtraFieldParams>,
) -> ApiResult<Json<Vec<ExtraField>>> {
    validate_entity_type(&entity_type)?;
    let fields = db::upsert_definition(
        &state.db.pool,
        &state.db.engine,
        &entity_type,
        &key,
        &params,
    )
    .await?;
    Ok(Json(fields))
}

/// DELETE {base_path}/api/v1/field/{entity_type}/{key}
pub async fn delete_definition(
    State(state): State<AppState>,
    Path((entity_type, key)): Path<(String, String)>,
) -> ApiResult<Json<Vec<ExtraField>>> {
    validate_entity_type(&entity_type)?;
    let fields =
        db::delete_definition(&state.db.pool, &state.db.engine, &entity_type, &key).await?;
    Ok(Json(fields))
}
