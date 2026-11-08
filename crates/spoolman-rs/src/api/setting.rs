//! Settings endpoints — mirrors `spoolman/api/v1/setting.py`.
//!
//! Settings are stored as JSON-encoded values in the `setting` table.
//! Unknown keys return 404; the body must be a valid JSON value of the
//! correct type (or null/empty to reset to default).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::Value;

use crate::{
    api::error::ApiResult, db::setting as db, error::SpoolmanError, models::SettingResponse,
    state::AppState,
};

/// Validate the content of an `extra_fields_<entity>` setting value.
/// `raw` is the already-type-validated string containing a JSON array.
/// Returns `Err(BadRequest)` if the content is malformed.
fn validate_extra_fields_setting(entity_type: &str, raw: &str) -> crate::api::error::ApiResult<()> {
    let fields: Vec<crate::models::ExtraField> = serde_json::from_str(raw).map_err(|e| {
        crate::error::SpoolmanError::BadRequest(format!("Invalid extra field definition: {e}"))
    })?;
    for (i, field) in fields.iter().enumerate() {
        if field.entity_type != entity_type {
            return Err(crate::error::SpoolmanError::BadRequest(format!(
                "Extra field at index {i} has entity type {:?}, expected {:?}.",
                field.entity_type, entity_type
            ))
            .into());
        }
    }
    Ok(())
}

// Helper: build SettingResponse from an optional raw stored value and the setting definition.
fn make_response(def: &db::SettingDef, raw: Option<String>) -> SettingResponse {
    match raw {
        Some(v) => SettingResponse {
            value: v,
            is_set: true,
            type_: def.type_name.to_owned(),
        },
        None => SettingResponse {
            value: def.default.to_owned(),
            is_set: false,
            type_: def.type_name.to_owned(),
        },
    }
}

/// GET {base_path}/api/v1/setting/{key}
pub async fn get_one(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> ApiResult<Json<SettingResponse>> {
    let def = db::find_def(&key)
        .ok_or_else(|| SpoolmanError::NotFound(format!("setting '{key}' does not exist")))?;
    let raw = db::get(&state.db.pool, &key).await?;
    Ok(Json(make_response(def, raw)))
}

/// GET {base_path}/api/v1/setting/
pub async fn get_all(
    State(state): State<AppState>,
) -> ApiResult<Json<serde_json::Map<String, Value>>> {
    let set_values = db::get_all(&state.db.pool).await?;
    let set_map: std::collections::HashMap<String, String> = set_values.into_iter().collect();

    let mut result = serde_json::Map::new();
    for def in db::SETTINGS {
        let raw = set_map.get(def.key).cloned();
        let resp = make_response(def, raw);
        result.insert(
            def.key.to_owned(),
            serde_json::to_value(resp).unwrap_or(Value::Null),
        );
    }

    Ok(Json(result))
}

/// POST {base_path}/api/v1/setting/{key}
/// Body: a JSON value, or null/empty to reset to default.
pub async fn set_one(
    State(state): State<AppState>,
    Path(key): Path<String>,
    body: Option<Json<Value>>,
) -> ApiResult<Json<SettingResponse>> {
    let def = db::find_def(&key)
        .ok_or_else(|| SpoolmanError::NotFound(format!("setting '{key}' does not exist")))?;

    let reset = match &body {
        None => true,
        Some(Json(Value::Null)) => true,
        // An empty string body resets the setting (mirrors upstream Python `if body`).
        Some(Json(Value::String(s))) if s.is_empty() => true,
        Some(Json(_)) => false,
    };

    if reset {
        db::delete(&state.db.pool, &key).await?;
    } else {
        // For string values: store the raw string chars (not the JSON encoding).
        // For other types: use the JSON representation.
        let raw = match body.unwrap().0 {
            Value::String(s) => s,
            other => other.to_string(),
        };
        // Validate type before storing: parse `raw` as JSON and check against def.type_name.
        if let Some(msg) = db::validate_type(def, &raw) {
            return Err(crate::error::SpoolmanError::ValidationError(msg).into());
        }
        // For extra_fields_* settings, validate that each element is a well-formed ExtraField.
        if let Some(entity_type) = key.strip_prefix("extra_fields_") {
            validate_extra_fields_setting(entity_type, &raw)?;
        }
        if matches!(state.db.engine, crate::db::Engine::Mysql) {
            db::set_mysql(&state.db.pool, &key, &raw).await?;
        } else {
            db::set(&state.db.pool, &key, &raw).await?;
        }
    }

    // Return the new value (or the default if reset).
    let raw = db::get(&state.db.pool, &key).await?;
    Ok(Json(make_response(def, raw)))
}

/// DELETE {base_path}/api/v1/setting/{key}  (reset to default)
pub async fn delete_one(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> ApiResult<StatusCode> {
    db::find_def(&key)
        .ok_or_else(|| SpoolmanError::NotFound(format!("setting '{key}' does not exist")))?;
    db::delete(&state.db.pool, &key).await?;
    Ok(StatusCode::NO_CONTENT)
}
