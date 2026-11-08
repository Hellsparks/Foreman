//! Extra field helpers — definitions (stored in `setting`) and values (stored in `*_field` tables).
//!
//! Extra field DEFINITIONS live in the `setting` table as JSON arrays for the keys
//! `extra_fields_vendor`, `extra_fields_filament`, `extra_fields_spool`.
//!
//! Extra field VALUES live in `vendor_field(vendor_id, key, value TEXT)`,
//! `filament_field(filament_id, key, value TEXT)`,
//! `spool_field(spool_id, key, value TEXT)`.
//! Values are stored as JSON-encoded strings (e.g. `"\"red\""`, `"42"`, `"true"`).

use std::collections::HashMap;

use serde_json::{Map, Value};
use sqlx::AnyPool;

use crate::{
    db::{setting, Engine},
    error::{Result, SpoolmanError},
    models::{ExtraField, ExtraFieldParams},
};

// ---------------------------------------------------------------------------
// Definitions — stored as JSON arrays in `setting`
// ---------------------------------------------------------------------------

fn setting_key(entity_type: &str) -> String {
    format!("extra_fields_{entity_type}")
}

/// Get all extra field definitions for an entity type.
pub async fn get_definitions(pool: &AnyPool, entity_type: &str) -> Result<Vec<ExtraField>> {
    let key = setting_key(entity_type);
    let raw = match setting::get(pool, &key).await? {
        Some(v) => v,
        None => return Ok(vec![]),
    };
    let fields: Vec<ExtraField> = serde_json::from_str(&raw)?;
    Ok(fields)
}

/// Add or update an extra field definition.  Returns the full updated list.
pub async fn upsert_definition(
    pool: &AnyPool,
    engine: &Engine,
    entity_type: &str,
    field_key: &str,
    params: &ExtraFieldParams,
) -> Result<Vec<ExtraField>> {
    let mut fields = get_definitions(pool, entity_type).await?;

    // Validate: for choice fields, choices must be provided; for others, must not be.
    if matches!(params.field_type, crate::models::ExtraFieldType::Choice) {
        if params.choices.is_none() {
            return Err(SpoolmanError::BadRequest(
                "choices must be set for field_type choice".into(),
            ));
        }
        if params.multi_choice.is_none() {
            return Err(SpoolmanError::BadRequest(
                "multi_choice must be set for field_type choice".into(),
            ));
        }
    } else {
        if params.choices.is_some() {
            return Err(SpoolmanError::BadRequest(
                "choices must not be set for field types other than choice".into(),
            ));
        }
        if params.multi_choice.is_some() {
            return Err(SpoolmanError::BadRequest(
                "multi_choice must not be set for field types other than choice".into(),
            ));
        }
    }

    // Check for type/multi_choice change conflicts on existing field.
    if let Some(existing) = fields.iter().find(|f| f.key == field_key) {
        if existing.field_type != params.field_type {
            return Err(SpoolmanError::BadRequest(
                "field_type cannot be changed".into(),
            ));
        }
        if matches!(params.field_type, crate::models::ExtraFieldType::Choice)
            && existing.multi_choice != params.multi_choice
        {
            return Err(SpoolmanError::BadRequest(
                "multi_choice cannot be changed".into(),
            ));
        }
    }

    // Replace or append.
    let new_field = ExtraField {
        key: field_key.to_owned(),
        entity_type: entity_type.to_owned(),
        name: params.name.clone(),
        order: params.order,
        unit: params.unit.clone(),
        field_type: params.field_type.clone(),
        default_value: params.default_value.clone(),
        choices: params.choices.clone(),
        multi_choice: params.multi_choice,
    };

    fields.retain(|f| f.key != field_key);
    fields.push(new_field);

    let json = serde_json::to_string(&fields)?;
    let key = setting_key(entity_type);
    if matches!(engine, Engine::Mysql) {
        setting::set_mysql(pool, &key, &json).await?;
    } else {
        setting::set(pool, &key, &json).await?;
    }

    Ok(fields)
}

/// Delete an extra field definition.  Also clears all stored values for that key.
/// Returns the remaining list of definitions.
pub async fn delete_definition(
    pool: &AnyPool,
    engine: &Engine,
    entity_type: &str,
    field_key: &str,
) -> Result<Vec<ExtraField>> {
    let mut fields = get_definitions(pool, entity_type).await?;
    if !fields.iter().any(|f| f.key == field_key) {
        return Err(SpoolmanError::NotFound(format!(
            "extra field '{field_key}' not found for entity type '{entity_type}'"
        )));
    }
    fields.retain(|f| f.key != field_key);

    let json = serde_json::to_string(&fields)?;
    let key = setting_key(entity_type);
    if matches!(engine, Engine::Mysql) {
        setting::set_mysql(pool, &key, &json).await?;
    } else {
        setting::set(pool, &key, &json).await?;
    }

    // Clear all stored values for this field across all entities.
    let field_table = format!("{entity_type}_field");
    let del_sql = format!("DELETE FROM {field_table} WHERE key = ?");
    sqlx::query(&del_sql).bind(field_key).execute(pool).await?;

    Ok(fields)
}

// ---------------------------------------------------------------------------
// Values — stored in `{entity}_field` tables
// ---------------------------------------------------------------------------

/// Get extra field values for a single entity.
pub async fn get_values(
    pool: &AnyPool,
    table: &str,
    id_col: &str,
    id: i64,
) -> Result<Map<String, Value>> {
    let sql = format!("SELECT key, value FROM {table} WHERE {id_col} = ?");
    let rows = sqlx::query(&sql).bind(id).fetch_all(pool).await?;
    let mut map = Map::new();
    for row in rows {
        use sqlx::Row;
        let key: String = row.try_get("key")?;
        let raw_val: String = row.try_get("value")?;
        // Values are stored as JSON: parse them back so the API returns the
        // native JSON type (string, number, bool, array, etc.), not a JSON string.
        let json_val: Value = serde_json::from_str(&raw_val).unwrap_or(Value::String(raw_val));
        map.insert(key, json_val);
    }
    Ok(map)
}

/// Get extra field values for a batch of entity IDs.
/// Returns a map from entity_id to its extra field map.
pub async fn get_values_batch(
    pool: &AnyPool,
    table: &str,
    id_col: &str,
    ids: &[i64],
) -> Result<HashMap<i64, Map<String, Value>>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql =
        format!("SELECT {id_col}, key, value FROM {table} WHERE {id_col} IN ({placeholders})");
    let mut q = sqlx::query(&sql);
    for id in ids {
        q = q.bind(*id);
    }
    let rows = q.fetch_all(pool).await?;

    let mut result: HashMap<i64, Map<String, Value>> = HashMap::new();
    for row in rows {
        use sqlx::Row;
        let entity_id: i64 = row.try_get(id_col)?;
        let key: String = row.try_get("key")?;
        let raw_val: String = row.try_get("value")?;
        let json_val: Value = serde_json::from_str(&raw_val).unwrap_or(Value::String(raw_val));
        result.entry(entity_id).or_default().insert(key, json_val);
    }
    Ok(result)
}

/// Set (replace all) extra field values for an entity.
/// A `Value::Null` means "delete this key" (upstream behaviour).
pub async fn set_values(
    pool: &AnyPool,
    table: &str,
    id_col: &str,
    id: i64,
    values: &Map<String, Value>,
) -> Result<()> {
    // Delete all existing values for this entity.
    let del_sql = format!("DELETE FROM {table} WHERE {id_col} = ?");
    sqlx::query(&del_sql).bind(id).execute(pool).await?;

    // Insert non-null values.
    let ins_sql = format!("INSERT INTO {table} ({id_col}, key, value) VALUES (?,?,?)");
    for (key, val) in values {
        if val.is_null() {
            continue;
        }
        let raw = val.to_string(); // JSON encoding
        sqlx::query(&ins_sql)
            .bind(id)
            .bind(key.as_str())
            .bind(raw)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Get all distinct values stored for a specific extra field key across all entities.
pub async fn get_distinct_values(pool: &AnyPool, table: &str, key: &str) -> Result<Vec<String>> {
    let sql = format!("SELECT DISTINCT value FROM {table} WHERE key = ?");
    let rows = sqlx::query(&sql).bind(key).fetch_all(pool).await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        use sqlx::Row;
        let raw: String = row.try_get("value")?;
        // Values are JSON-encoded; decode to get the plain string for text fields.
        let decoded = serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or(raw);
        result.push(decoded);
    }
    Ok(result)
}
