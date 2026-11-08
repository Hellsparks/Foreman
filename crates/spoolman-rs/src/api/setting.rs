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

/// GET {base_path}/api/v1/setting/{key}
pub async fn get_one(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> ApiResult<Json<SettingResponse>> {
    let def = db::find_def(&key)
        .ok_or_else(|| SpoolmanError::NotFound(format!("setting '{key}' does not exist")))?;

    let (value, is_set) = match db::get(&state.db.pool, &key).await? {
        Some(raw) => {
            let v: Value = serde_json::from_str(&raw)?;
            (v, true)
        }
        None => {
            let v: Value = serde_json::from_str(def.default)?;
            (v, false)
        }
    };

    Ok(Json(SettingResponse {
        value,
        is_set,
        type_: def.type_name.to_owned(),
    }))
}

/// GET {base_path}/api/v1/setting/
pub async fn get_all(
    State(state): State<AppState>,
) -> ApiResult<Json<serde_json::Map<String, Value>>> {
    let set_values = db::get_all(&state.db.pool).await?;
    let set_map: std::collections::HashMap<String, String> = set_values.into_iter().collect();

    let mut result = serde_json::Map::new();
    for def in db::SETTINGS {
        let (value, is_set) = if let Some(raw) = set_map.get(def.key) {
            let v: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
            (v, true)
        } else {
            let v: Value = serde_json::from_str(def.default).unwrap_or(Value::Null);
            (v, false)
        };
        result.insert(
            def.key.to_owned(),
            serde_json::to_value(SettingResponse {
                value,
                is_set,
                type_: def.type_name.to_owned(),
            })
            .unwrap_or(Value::Null),
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
        Some(Json(_)) => false,
    };

    if reset {
        db::delete(&state.db.pool, &key).await?;
    } else {
        let raw = body.unwrap().0.to_string();
        if matches!(state.db.engine, crate::db::Engine::Mysql) {
            db::set_mysql(&state.db.pool, &key, &raw).await?;
        } else {
            db::set(&state.db.pool, &key, &raw).await?;
        }
    }

    // Return the new value (or the default if reset).
    let (value, is_set) = match db::get(&state.db.pool, &key).await? {
        Some(raw) => {
            let v: Value = serde_json::from_str(&raw)?;
            (v, true)
        }
        None => {
            let v: Value = serde_json::from_str(def.default)?;
            (v, false)
        }
    };

    Ok(Json(SettingResponse {
        value,
        is_set,
        type_: def.type_name.to_owned(),
    }))
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
