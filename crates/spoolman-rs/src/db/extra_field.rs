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
        // Upstream contract: every value in the `extra` map is a JSON-encoded string.
        // We return the raw stored string so consumers can JSON-decode it themselves,
        // exactly as the upstream Python Spoolman does.
        map.insert(key, Value::String(raw_val));
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
        // Same as get_values: return the raw stored JSON-encoded string.
        result
            .entry(entity_id)
            .or_default()
            .insert(key, Value::String(raw_val));
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
        // Upstream contract: clients send JSON-encoded strings for all values.
        // Store the string content directly — do NOT re-encode with to_string(),
        // which would double-encode (e.g. "\"hello\"" → "\"\\\"hello\\\"\"").
        // Fall back to to_string() for bare numbers/bools sent by non-compliant clients.
        let raw = match val {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        sqlx::query(&ins_sql)
            .bind(id)
            .bind(key.as_str())
            .bind(raw)
            .execute(pool)
            .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Foreman-specific defaults
// ---------------------------------------------------------------------------

/// Ensure Foreman's built-in extra fields exist on the `filament` entity.
///
/// Called once by `foreman-server` after connecting to the spoolman-rs database.
/// Each field is created only if it is not already present — existing definitions
/// are never overwritten so users can rename/reorder without losing their changes.
///
/// Fields (from the original Marathon `setup.js`):
/// | key                | type    | unit |
/// |--------------------|---------|------|
/// | url                | text    |      |
/// | hue_td             | float   | TD   |
/// | swatch             | boolean |      |
/// | orcaslicer_config  | text    |      |
/// | material_modifier  | text    |      |
pub async fn ensure_foreman_defaults(pool: &AnyPool, engine: &Engine) -> crate::error::Result<()> {
    use crate::models::ExtraFieldType;

    struct DefaultField {
        key: &'static str,
        name: &'static str,
        field_type: ExtraFieldType,
        unit: Option<&'static str>,
    }

    let defaults = [
        DefaultField {
            key: "url",
            name: "Link",
            field_type: ExtraFieldType::Text,
            unit: None,
        },
        DefaultField {
            key: "hue_td",
            name: "Hueforge TD",
            field_type: ExtraFieldType::Float,
            unit: Some("TD"),
        },
        DefaultField {
            key: "swatch",
            name: "Has printed swatch",
            field_type: ExtraFieldType::Boolean,
            unit: None,
        },
        DefaultField {
            key: "orcaslicer_config",
            name: "OrcaSlicer Config",
            field_type: ExtraFieldType::Text,
            unit: None,
        },
        DefaultField {
            key: "material_modifier",
            name: "Material Modifier",
            field_type: ExtraFieldType::Text,
            unit: None,
        },
    ];

    let existing = get_definitions(pool, "filament").await?;
    let existing_keys: std::collections::HashSet<&str> =
        existing.iter().map(|f| f.key.as_str()).collect();

    for d in &defaults {
        if existing_keys.contains(d.key) {
            continue;
        }
        let params = ExtraFieldParams {
            name: d.name.to_owned(),
            order: 0,
            field_type: d.field_type.clone(),
            unit: d.unit.map(str::to_owned),
            default_value: None,
            choices: None,
            multi_choice: None,
        };
        upsert_definition(pool, engine, "filament", d.key, &params).await?;
        tracing::info!(key = d.key, "created Foreman default extra field");
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
        // Return the raw stored JSON-encoded string, consistent with the extra map.
        result.push(raw);
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{install_drivers, Engine};
    use sqlx::AnyPool;
    use tempfile::TempDir;

    async fn open_fresh_db(dir: &std::path::Path) -> AnyPool {
        install_drivers();
        let path = dir.join("test.db");
        std::fs::File::create(&path).unwrap();
        let url = format!("sqlite://{}", path.display());
        let pool = AnyPool::connect(&url).await.unwrap();
        // Create the tables needed by extra_field helpers.
        sqlx::query(
            "CREATE TABLE setting (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL,
                first_created DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now')),
                last_updated DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now'))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE filament_field (
                filament_id INTEGER NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    /// Round-trip a text extra field: set it with a JSON-encoded string value
    /// (as the upstream Spoolman client does), then read it back.
    ///
    /// Regression for the double-encoding bug: before the fix, `set_values`
    /// called `val.to_string()` on a `Value::String`, which re-encoded the
    /// already-JSON-encoded string — producing `"\"\\\"url\\\"\""` instead of
    /// the correct `"\"url\""`.
    #[tokio::test]
    async fn extra_field_values_round_trip_as_raw_json_strings() {
        let dir = TempDir::new().unwrap();
        let pool = open_fresh_db(dir.path()).await;
        let engine = Engine::Sqlite;

        // Create a text extra field definition.
        let params = crate::models::ExtraFieldParams {
            name: "Link".to_owned(),
            order: 1,
            field_type: crate::models::ExtraFieldType::Text,
            unit: None,
            default_value: None,
            choices: None,
            multi_choice: None,
        };
        upsert_definition(&pool, &engine, "filament", "url", &params)
            .await
            .unwrap();

        // Simulate what the Spoolman web client sends: a JSON-encoded string
        // value for a text field.  Over the wire the PATCH body contains
        // `"extra": {"url": "\"https://example.com\""}`.  serde_json parses
        // `"\"https://example.com\""` as Value::String whose Rust content is
        // `"https://example.com"` — a string that includes the outer double-
        // quote characters, because those are the JSON encoding of the actual
        // text field value.  We construct it directly here to avoid confusion
        // with the from_str string-delimiter stripping.
        let val = Value::String(r#""https://example.com""#.to_owned());
        assert!(val.is_string(), "pre-condition: val must be Value::String");

        let mut values = Map::new();
        values.insert("url".to_owned(), val);
        set_values(&pool, "filament_field", "filament_id", 1, &values)
            .await
            .unwrap();

        // Read back via get_values.
        let got = get_values(&pool, "filament_field", "filament_id", 1)
            .await
            .unwrap();

        // The returned value MUST be the raw stored string, not a decoded URL.
        // Upstream Spoolman returns `"\"https://example.com\""` in the JSON
        // response body — a JSON string whose content is `"https://example.com"`
        // (with the outer double-quote characters preserved).
        assert_eq!(
            got.get("url"),
            Some(&Value::String(r#""https://example.com""#.to_owned())),
            "get_values must return the raw JSON-encoded string, not a decoded URL"
        );

        // Verify the DB column stores the raw JSON-encoded string (single encoding).
        let stored: String =
            sqlx::query_scalar("SELECT value FROM filament_field WHERE key = 'url'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            stored, r#""https://example.com""#,
            "stored value must be the JSON string literal, not double-encoded"
        );
    }

    /// A float extra field stored by upstream Python as a bare number (`4.3`, not `"4.3"`)
    /// must round-trip correctly through spoolman-rs.
    #[tokio::test]
    async fn extra_field_float_stored_by_upstream_survives_read() {
        let dir = TempDir::new().unwrap();
        let pool = open_fresh_db(dir.path()).await;

        // Insert exactly the value the upstream Python app stored.
        sqlx::query(
            "INSERT INTO filament_field (filament_id, key, value) VALUES (1, 'hue_td', '4.3')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let got = get_values(&pool, "filament_field", "filament_id", 1)
            .await
            .unwrap();

        // Returned as the raw stored string "4.3".
        assert_eq!(
            got.get("hue_td"),
            Some(&Value::String("4.3".to_owned())),
            "float stored as bare number must come back as the raw stored string"
        );
    }
}
