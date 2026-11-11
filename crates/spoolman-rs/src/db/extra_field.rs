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
    db::{query::tp, setting, Engine},
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
    use crate::models::ExtraFieldType;

    let mut fields = get_definitions(pool, entity_type).await?;

    // 1. choice fields require choices + multi_choice; non-choice must not have them.
    if matches!(params.field_type, ExtraFieldType::Choice) {
        if params.choices.is_none() {
            return Err(SpoolmanError::BadRequest(
                "Choices must be set for field type choice.".into(),
            ));
        }
        if params.multi_choice.is_none() {
            return Err(SpoolmanError::BadRequest(
                "Multi choice must be set for field type choice.".into(),
            ));
        }
    } else {
        if params.choices.is_some() {
            return Err(SpoolmanError::BadRequest(
                "Choices must not be set for field types other than choice.".into(),
            ));
        }
        if params.multi_choice.is_some() {
            return Err(SpoolmanError::BadRequest(
                "Multi choice must not be set for field types other than choice.".into(),
            ));
        }
    }

    // 2. Validate default_value type if provided.
    if let Some(ref dv) = params.default_value {
        validate_default_value(&params.field_type, dv, params.multi_choice)?;
    }

    // 3. Check conflicts with the existing field definition (if one exists).
    if let Some(existing) = fields.iter().find(|f| f.key == field_key) {
        if existing.field_type != params.field_type {
            return Err(SpoolmanError::BadRequest(
                "Field type cannot be changed.".into(),
            ));
        }
        if matches!(params.field_type, ExtraFieldType::Choice) {
            if existing.multi_choice != params.multi_choice {
                return Err(SpoolmanError::BadRequest(
                    "Multi choice cannot be changed.".into(),
                ));
            }
            // New choices list must be a superset of the existing choices.
            if let (Some(old_choices), Some(new_choices)) = (&existing.choices, &params.choices) {
                let removed: Vec<_> = old_choices
                    .iter()
                    .filter(|c| !new_choices.contains(c))
                    .collect();
                if !removed.is_empty() {
                    return Err(SpoolmanError::BadRequest(
                        "Cannot remove existing choices.".into(),
                    ));
                }
            }
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

/// Validate that `default_value` (a JSON-encoded string) is a valid value for `field_type`.
fn validate_default_value(
    field_type: &crate::models::ExtraFieldType,
    default_value: &str,
    multi_choice: Option<bool>,
) -> Result<()> {
    use crate::models::ExtraFieldType;

    let decoded = serde_json::from_str::<Value>(default_value)
        .map_err(|e| SpoolmanError::BadRequest(format!("Default value is not valid: {e}")))?;

    let not_string =
        || SpoolmanError::BadRequest("Default value is not valid: Value is not a string.".into());

    match field_type {
        ExtraFieldType::Text | ExtraFieldType::Datetime => {
            if !decoded.is_string() {
                return Err(not_string());
            }
        }
        ExtraFieldType::Integer => {
            let ok = decoded.as_f64().is_some_and(|f| f.fract() == 0.0);
            if !ok {
                return Err(SpoolmanError::BadRequest(
                    "Default value is not valid: Value is not an integer.".into(),
                ));
            }
        }
        ExtraFieldType::IntegerRange => {
            let ok = matches!(&decoded, Value::Array(a)
                if a.len() == 2 && a.iter().all(|v| v.as_f64().is_some_and(|f| f.fract() == 0.0)));
            if !ok {
                return Err(SpoolmanError::BadRequest(
                    "Default value is not valid: Value is not an integer range.".into(),
                ));
            }
        }
        ExtraFieldType::Float => {
            if !decoded.is_number() {
                return Err(SpoolmanError::BadRequest(
                    "Default value is not valid: Value is not a number.".into(),
                ));
            }
        }
        ExtraFieldType::FloatRange => {
            let ok = matches!(&decoded, Value::Array(a)
                if a.len() == 2 && a.iter().all(|v| v.is_number()));
            if !ok {
                return Err(SpoolmanError::BadRequest(
                    "Default value is not valid: Value is not a float range.".into(),
                ));
            }
        }
        ExtraFieldType::Boolean => {
            if !decoded.is_boolean() {
                return Err(SpoolmanError::BadRequest(
                    "Default value is not valid: Value is not a boolean.".into(),
                ));
            }
        }
        ExtraFieldType::Choice => {
            let is_multi = multi_choice.unwrap_or(false);
            if is_multi {
                if !matches!(decoded, Value::Array(_)) {
                    return Err(SpoolmanError::BadRequest(
                        "Default value is not valid: Value is not a list.".into(),
                    ));
                }
            } else if !decoded.is_string() {
                return Err(not_string());
            }
        }
    }
    Ok(())
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
    let del_sql = tp(&format!("DELETE FROM {field_table} WHERE key = ?"));
    sqlx::query(sqlx::AssertSqlSafe(&*del_sql))
        .bind(field_key)
        .execute(pool)
        .await?;

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
    let sql = tp(&format!(
        "SELECT key, value FROM {table} WHERE {id_col} = ?"
    ));
    let rows = sqlx::query(sqlx::AssertSqlSafe(&*sql))
        .bind(id)
        .fetch_all(pool)
        .await?;
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
    let sql = tp(&format!(
        "SELECT {id_col}, key, value FROM {table} WHERE {id_col} IN ({placeholders})"
    ));
    let mut q = sqlx::query(sqlx::AssertSqlSafe(&*sql));
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

/// Validate that every key in `extra` has a corresponding field definition for
/// `entity_type`.  Returns `Err(BadRequest)` if any unknown key is present.
///
/// Must be called before `set_values` on PATCH/PUT to match upstream behaviour
/// where even a null value for a non-existent field is a 422/400 error.
/// Validate that all keys in `extra` are defined for `entity_type`.
///
/// `for_create` controls the error variant:
/// - `true`  → `ValidationError` (HTTP 422), matching upstream's Pydantic behaviour on POST.
/// - `false` → `BadRequest`      (HTTP 400), matching upstream's explicit check on PATCH.
pub async fn validate_keys(
    pool: &AnyPool,
    entity_type: &str,
    extra: &serde_json::Map<String, Value>,
    for_create: bool,
) -> Result<()> {
    if extra.is_empty() {
        return Ok(());
    }
    let defs = get_definitions(pool, entity_type).await?;
    let defined: std::collections::HashSet<&str> = defs.iter().map(|f| f.key.as_str()).collect();
    let unknown: Vec<&str> = extra
        .keys()
        .map(String::as_str)
        .filter(|k| !defined.contains(k))
        .collect();
    if !unknown.is_empty() {
        let msg = format!("Unknown extra field key(s): {}", unknown.join(", "));
        return Err(if for_create {
            SpoolmanError::ValidationError(msg)
        } else {
            SpoolmanError::BadRequest(msg)
        });
    }
    Ok(())
}

/// Apply per-key extra field changes for an entity (upstream PATCH semantics).
///
/// For each key in `values`:
/// - `Value::Null`   → delete that key (leave all other keys unchanged).
/// - Any other value → upsert that key (leave all other keys unchanged).
///
/// Keys *not* present in `values` are untouched. This matches upstream's PATCH
/// behaviour where `{"extra": {"k": null}}` removes only `k` while leaving every
/// other key on the entity intact.
///
/// Safe to call on a fresh entity (no existing rows) — the per-key DELETE is a
/// no-op when the key doesn't exist yet.
pub async fn set_values(
    pool: &AnyPool,
    table: &str,
    id_col: &str,
    id: i64,
    values: &Map<String, Value>,
) -> Result<()> {
    let del_sql = tp(&format!(
        "DELETE FROM {table} WHERE {id_col} = ? AND key = ?"
    ));
    let ins_sql = tp(&format!(
        "INSERT INTO {table} ({id_col}, key, value) VALUES (?,?,?)"
    ));

    for (key, val) in values {
        // Always remove the current row for this key (if it exists).
        sqlx::query(sqlx::AssertSqlSafe(&*del_sql))
            .bind(id)
            .bind(key.as_str())
            .execute(pool)
            .await?;

        if val.is_null() {
            continue; // null = "delete this key" — don't re-insert.
        }

        // Upstream contract: clients send JSON-encoded strings for all values.
        // Store the string content directly — do NOT re-encode with to_string(),
        // which would double-encode (e.g. "\"hello\"" → "\"\\\"hello\\\"\"").
        // Fall back to to_string() for bare numbers/bools sent by non-compliant clients.
        let raw = match val {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        sqlx::query(sqlx::AssertSqlSafe(&*ins_sql))
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
/// | abrasive           | boolean |      |  (Foreman 4.1: filament guard)
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
        DefaultField {
            key: "abrasive",
            name: "Abrasive",
            field_type: ExtraFieldType::Boolean,
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

// ---------------------------------------------------------------------------
// Extra-field filtering and sorting for list queries
// ---------------------------------------------------------------------------

/// Pre-built SQL fragments for extra-field filtering and sorting.
///
/// Returned by [`build_extra_sql`] and consumed by the entity `find()` functions.
#[derive(Default)]
pub struct ExtraSql {
    /// Additional WHERE conditions (complete SQL predicates using EXISTS subqueries).
    pub where_clauses: Vec<String>,
    /// Binds for [`where_clauses`], in order of `?` placeholders.
    pub where_binds: Vec<String>,
    /// Complete ORDER BY fragments (ready to append), one per extra sort field.
    pub order_frags: Vec<String>,
    /// Binds for [`order_frags`] subqueries.
    pub order_binds: Vec<String>,
}

/// Escape LIKE wildcards using `/` as the escape character.
///
/// `/` avoids the ambiguity that `\` causes in MySQL/MariaDB string parsing.
/// Pair the result with `ESCAPE '/'` in the SQL clause.
pub fn escape_like(s: &str) -> String {
    s.replace('/', "//").replace('%', "/%").replace('_', "/_")
}

/// Build a JSON first-element expression (for integer_range / float_range filtering/sorting).
fn json_first_elem(col: &str, engine: &Engine) -> String {
    match engine {
        Engine::Postgres => format!("CAST(CAST({col} AS JSON) ->> 0 AS FLOAT)"),
        _ => format!("CAST(JSON_EXTRACT({col}, '$[0]') AS REAL)"),
    }
}

/// Build a JSON second-element expression.
fn json_second_elem(col: &str, engine: &Engine) -> String {
    match engine {
        Engine::Postgres => format!("CAST(CAST({col} AS JSON) ->> 1 AS FLOAT)"),
        _ => format!("CAST(JSON_EXTRACT({col}, '$[1]') AS REAL)"),
    }
}

/// LIKE/ILIKE operator appropriate for the engine (case-insensitive fuzzy match).
fn like_op(engine: &Engine) -> &'static str {
    match engine {
        Engine::Postgres => "ILIKE",
        _ => "LIKE",
    }
}

/// Build the SQL EXISTS condition for one filter term on one extra field.
///
/// Returns `(sql_fragment, binds)`.
fn build_term_condition(
    engine: &Engine,
    field: &ExtraField,
    entity_alias: &str,
    field_table: &str,
    id_col: &str,
    term: &crate::db::query::FilterTerm,
) -> Result<(String, Vec<String>)> {
    use crate::db::query::FilterTerm;
    use crate::models::ExtraFieldType;

    // Helper: wrap a value condition into a correlated EXISTS subquery.
    let exists = |cond: &str, binds: Vec<String>| -> (String, Vec<String>) {
        let sql = format!(
            "EXISTS (SELECT 1 FROM {field_table} WHERE {id_col} = {entity_alias}.id AND key = ? AND ({cond}))"
        );
        // key bind comes first, then condition binds
        let mut all = vec![field.key.clone()];
        all.extend(binds);
        (sql, all)
    };

    // Empty-filter: no value set (or false for booleans).
    if matches!(term, FilterTerm::IsNull) {
        let mut empty_conds: Vec<&str> = vec!["value IS NULL", "value = 'null'", "value = '\"\"'"];
        let bool_extra;
        if matches!(field.field_type, ExtraFieldType::Boolean) {
            bool_extra = "value = 'false'";
            empty_conds.push(bool_extra);
        }
        let empty_cond_sql = format!("({})", empty_conds.join(" OR "));
        // EXISTS with empty value  OR  NOT EXISTS (no row at all)
        let sql = format!(
            "(NOT EXISTS (SELECT 1 FROM {field_table} WHERE {id_col} = {entity_alias}.id AND key = ?) \
             OR EXISTS (SELECT 1 FROM {field_table} WHERE {id_col} = {entity_alias}.id AND key = ? AND {empty_cond_sql}))"
        );
        let binds = vec![field.key.clone(), field.key.clone()];
        return Ok((sql, binds));
    }

    let (is_exact, raw_term) = match term {
        FilterTerm::Exact(v) => (true, v.as_str()),
        FilterTerm::Fuzzy(v) => (false, v.as_str()),
        FilterTerm::IsNull => unreachable!(),
    };

    match &field.field_type {
        ExtraFieldType::Text => {
            if is_exact {
                // Exact: compare against stored JSON string (which wraps in double quotes).
                let stored = format!("\"{}\"", raw_term);
                Ok(exists("value = ?", vec![stored]))
            } else {
                // Fuzzy: LIKE/ILIKE with wildcard escaping.
                let op = like_op(engine);
                let pattern = format!("%{}%", escape_like(raw_term));
                let cond = format!("value {op} ? ESCAPE '/'");
                Ok(exists(&cond, vec![pattern]))
            }
        }

        ExtraFieldType::Integer => {
            if raw_term.contains(':') {
                // Range filter: min:max (either may be empty).
                let (min_s, max_s) = raw_term.split_once(':').unwrap();
                let mut conds = Vec::new();
                let mut binds = Vec::new();
                if !min_s.is_empty() {
                    min_s.trim().parse::<i64>().map_err(|_| {
                        SpoolmanError::BadRequest(format!(
                            "Invalid integer range filter value for '{}': {}",
                            field.key, raw_term
                        ))
                    })?;
                    conds.push("CAST(value AS REAL) >= CAST(? AS REAL)".to_owned());
                    binds.push(min_s.trim().to_owned());
                }
                if !max_s.is_empty() {
                    max_s.trim().parse::<i64>().map_err(|_| {
                        SpoolmanError::BadRequest(format!(
                            "Invalid integer range filter value for '{}': {}",
                            field.key, raw_term
                        ))
                    })?;
                    conds.push("CAST(value AS REAL) <= CAST(? AS REAL)".to_owned());
                    binds.push(max_s.trim().to_owned());
                }
                if conds.is_empty() {
                    return Err(SpoolmanError::BadRequest(format!(
                        "Invalid range filter value for '{}': '{}'. Expected '<min>:<max>'.",
                        field.key, raw_term
                    )));
                }
                Ok(exists(&conds.join(" AND "), binds))
            } else {
                // Exact integer: validate and store as string.
                let n = raw_term.trim().parse::<i64>().map_err(|_| {
                    SpoolmanError::BadRequest(format!(
                        "Invalid integer filter value for '{}': {}",
                        field.key, raw_term
                    ))
                })?;
                Ok(exists("value = ?", vec![n.to_string()]))
            }
        }

        ExtraFieldType::Float => {
            if raw_term.contains(':') {
                let (min_s, max_s) = raw_term.split_once(':').unwrap();
                let mut conds = Vec::new();
                let mut binds = Vec::new();
                if !min_s.is_empty() {
                    min_s.trim().parse::<f64>().map_err(|_| {
                        SpoolmanError::BadRequest(format!(
                            "Invalid float range filter value for '{}': {}",
                            field.key, raw_term
                        ))
                    })?;
                    conds.push("CAST(value AS REAL) >= CAST(? AS REAL)".to_owned());
                    binds.push(min_s.trim().to_owned());
                }
                if !max_s.is_empty() {
                    max_s.trim().parse::<f64>().map_err(|_| {
                        SpoolmanError::BadRequest(format!(
                            "Invalid float range filter value for '{}': {}",
                            field.key, raw_term
                        ))
                    })?;
                    conds.push("CAST(value AS REAL) <= CAST(? AS REAL)".to_owned());
                    binds.push(max_s.trim().to_owned());
                }
                if conds.is_empty() {
                    return Err(SpoolmanError::BadRequest(format!(
                        "Invalid range filter value for '{}': '{}'. Expected '<min>:<max>'.",
                        field.key, raw_term
                    )));
                }
                Ok(exists(&conds.join(" AND "), binds))
            } else {
                // Exact float: numeric comparison to handle int-stored floats (e.g. "2" == 2.0).
                raw_term.trim().parse::<f64>().map_err(|_| {
                    SpoolmanError::BadRequest(format!(
                        "Invalid float filter value for '{}': {}",
                        field.key, raw_term
                    ))
                })?;
                Ok(exists(
                    "CAST(value AS REAL) = CAST(? AS REAL)",
                    vec![raw_term.trim().to_owned()],
                ))
            }
        }

        ExtraFieldType::Boolean => {
            // Only "true" / "false" are accepted; numeric tokens and anything else → 400.
            let stored = match raw_term.trim().to_ascii_lowercase().as_str() {
                "true" => "true",
                "false" => "false",
                _ => {
                    return Err(SpoolmanError::BadRequest(format!(
                        "Invalid boolean filter value for '{}': {}",
                        field.key, raw_term
                    )));
                }
            };
            Ok(exists("value = ?", vec![stored.to_owned()]))
        }

        ExtraFieldType::Choice => {
            let is_multi = field.multi_choice.unwrap_or(false);
            if is_multi {
                // Multi-choice: JSON array; match the JSON-encoded token as a substring.
                // Escape the JSON-quoted token to treat % and _ literally.
                let json_token = format!("\"{}\"", raw_term);
                let pattern = format!("%{}%", escape_like(&json_token));
                let cond = "value LIKE ? ESCAPE '/'".to_string();
                Ok(exists(&cond, vec![pattern]))
            } else {
                // Single-choice: compare exact stored JSON string value.
                let stored = format!("\"{}\"", raw_term);
                Ok(exists("value = ?", vec![stored]))
            }
        }

        ExtraFieldType::Datetime => {
            // Datetimes are stored as JSON strings: `"2023-01-01T00:00:00"`.
            // Range separator is `|` (ISO 8601 timestamps already contain `:`).
            if raw_term.contains('|') {
                let (start_s, end_s) = raw_term.split_once('|').unwrap();
                let mut conds = Vec::new();
                let mut binds = Vec::new();
                if !start_s.is_empty() {
                    conds.push("value >= ?".to_owned());
                    binds.push(format!("\"{}\"", start_s.trim()));
                }
                if !end_s.is_empty() {
                    conds.push("value <= ?".to_owned());
                    binds.push(format!("\"{}\"", end_s.trim()));
                }
                if conds.is_empty() {
                    return Err(SpoolmanError::BadRequest(format!(
                        "Invalid datetime range filter for '{}': '{}'. \
                         Expected '<start>|<end>' with at least one end given.",
                        field.key, raw_term
                    )));
                }
                Ok(exists(&conds.join(" AND "), binds))
            } else {
                // Exact match against stored JSON string.
                let stored = format!("\"{}\"", raw_term.trim());
                Ok(exists("value = ?", vec![stored]))
            }
        }

        ExtraFieldType::IntegerRange | ExtraFieldType::FloatRange => {
            // Range fields must use the colon syntax.
            if !raw_term.contains(':') {
                return Err(SpoolmanError::BadRequest(format!(
                    "Invalid range filter value for '{}': '{}'. Expected '<min>:<max>'.",
                    field.key, raw_term
                )));
            }
            let (min_s, max_s) = raw_term.split_once(':').unwrap();
            let is_int = matches!(field.field_type, ExtraFieldType::IntegerRange);
            let mut conds = Vec::new();
            let mut binds = Vec::new();
            if !min_s.is_empty() {
                if is_int {
                    min_s.trim().parse::<i64>().map_err(|_| {
                        SpoolmanError::BadRequest(format!(
                            "Invalid integer range filter value for '{}': {}",
                            field.key, raw_term
                        ))
                    })?;
                } else {
                    min_s.trim().parse::<f64>().map_err(|_| {
                        SpoolmanError::BadRequest(format!(
                            "Invalid float range filter value for '{}': {}",
                            field.key, raw_term
                        ))
                    })?;
                }
                let expr = json_first_elem("value", engine);
                conds.push(format!("{expr} >= CAST(? AS REAL)"));
                binds.push(min_s.trim().to_owned());
            }
            if !max_s.is_empty() {
                if is_int {
                    max_s.trim().parse::<i64>().map_err(|_| {
                        SpoolmanError::BadRequest(format!(
                            "Invalid integer range filter value for '{}': {}",
                            field.key, raw_term
                        ))
                    })?;
                } else {
                    max_s.trim().parse::<f64>().map_err(|_| {
                        SpoolmanError::BadRequest(format!(
                            "Invalid float range filter value for '{}': {}",
                            field.key, raw_term
                        ))
                    })?;
                }
                let expr = json_second_elem("value", engine);
                conds.push(format!("{expr} <= CAST(? AS REAL)"));
                binds.push(max_s.trim().to_owned());
            }
            if conds.is_empty() {
                return Err(SpoolmanError::BadRequest(format!(
                    "Invalid range filter value for '{}': '{}'. Expected '<min>:<max>'.",
                    field.key, raw_term
                )));
            }
            Ok(exists(&conds.join(" AND "), binds))
        }
    }
}

/// Build the ORDER BY fragment for one extra sort field.
///
/// Returns `(order_by_fragment, binds)`.
/// The fragment is a complete `(nullcheck) ASC, sort_expr DIR` string.
fn build_sort_fragment(
    engine: &Engine,
    field: &ExtraField,
    entity_alias: &str,
    field_table: &str,
    id_col: &str,
    ascending: bool,
) -> (String, Vec<String>) {
    use crate::models::ExtraFieldType;

    let dir = if ascending { "ASC" } else { "DESC" };
    let subq =
        format!("(SELECT value FROM {field_table} WHERE {id_col} = {entity_alias}.id AND key = ?)");

    let sort_expr = match &field.field_type {
        ExtraFieldType::Integer => format!("CAST({subq} AS REAL)"),
        ExtraFieldType::Float => format!("CAST({subq} AS REAL)"),
        ExtraFieldType::IntegerRange | ExtraFieldType::FloatRange => json_first_elem(&subq, engine),
        // Text, Datetime, Boolean, Choice: sort lexicographically by raw stored value.
        _ => subq.clone(),
    };

    let frag = format!("({subq} IS NULL) ASC, {sort_expr} {dir}");
    // Each subquery reference needs one bind; there are two subq refs: one for IS NULL, one for sort.
    let binds = vec![field.key.clone(), field.key.clone()];
    (frag, binds)
}

/// Build extra-field SQL fragments for filtering and sorting.
///
/// # Arguments
/// * `entity_type` — `"spool"`, `"filament"`, or `"vendor"` (for loading definitions).
/// * `entity_alias` — SQL table alias used in the main query (`"s"`, `"f"`, `"v"`).
/// * `field_table` — extra-field join table (`"spool_field"`, etc.).
/// * `id_col` — ID column in `field_table` (`"spool_id"`, etc.).
/// * `extra_filters` — `(key, raw_filter_value)` pairs from the query string.
/// * `extra_sorts` — `(key, ascending)` pairs from the `sort=extra.*:asc/desc` param.
#[allow(clippy::too_many_arguments)]
pub async fn build_extra_sql(
    pool: &AnyPool,
    engine: &Engine,
    entity_type: &str,
    entity_alias: &str,
    field_table: &str,
    id_col: &str,
    extra_filters: &[(String, String)],
    extra_sorts: &[(String, bool)],
) -> Result<ExtraSql> {
    if extra_filters.is_empty() && extra_sorts.is_empty() {
        return Ok(ExtraSql::default());
    }

    let defs = get_definitions(pool, entity_type).await?;
    let def_map: std::collections::HashMap<&str, &ExtraField> =
        defs.iter().map(|f| (f.key.as_str(), f)).collect();

    let mut result = ExtraSql::default();

    // --- WHERE conditions ---
    for (key, raw_filter) in extra_filters {
        let Some(field) = def_map.get(key.as_str()) else {
            continue; // Unknown key: silently ignored per spec.
        };

        // Split on commas (the existing parse_filter_terms handles quoted terms).
        use crate::db::query::parse_filter_terms;
        let terms = parse_filter_terms(raw_filter);

        let mut parts = Vec::new();
        let mut all_binds = Vec::new();
        for term in &terms {
            let (part_sql, part_binds) =
                build_term_condition(engine, field, entity_alias, field_table, id_col, term)?;
            parts.push(part_sql);
            all_binds.extend(part_binds);
        }

        if parts.is_empty() {
            continue;
        }
        let clause = if parts.len() == 1 {
            parts.remove(0)
        } else {
            format!("({})", parts.join(" OR "))
        };
        result.where_clauses.push(clause);
        result.where_binds.extend(all_binds);
    }

    // --- ORDER BY ---
    for (key, ascending) in extra_sorts {
        let Some(field) = def_map.get(key.as_str()) else {
            continue; // Unknown key: silently ignored.
        };
        let (frag, binds) =
            build_sort_fragment(engine, field, entity_alias, field_table, id_col, *ascending);
        result.order_frags.push(frag);
        result.order_binds.extend(binds);
    }

    Ok(result)
}

// ---------------------------------------------------------------------------

/// Get all distinct values stored for a specific extra field key across all entities.
pub async fn get_distinct_values(pool: &AnyPool, table: &str, key: &str) -> Result<Vec<String>> {
    let sql = tp(&format!(
        "SELECT DISTINCT value FROM {table} WHERE key = ? AND value IS NOT NULL"
    ));
    let rows = sqlx::query(sqlx::AssertSqlSafe(&*sql))
        .bind(key)
        .fetch_all(pool)
        .await?;
    let mut result = Vec::new();
    for row in rows {
        use sqlx::Row;
        let raw: String = row.try_get("value")?;
        // Decode JSON string scalars (e.g. `'"A1"'` → `'A1'`).  Non-string JSON values
        // (numbers, booleans, arrays) are returned as-is.  Skip null / empty.
        let decoded = match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(serde_json::Value::String(s)) => {
                if s.is_empty() {
                    continue;
                }
                s
            }
            Ok(serde_json::Value::Null) => continue,
            _ => {
                if raw.is_empty() || raw == "null" {
                    continue;
                }
                raw
            }
        };
        result.push(decoded);
    }
    result.sort();
    result.dedup();
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
        let path_str = path.to_string_lossy().replace('\\', "/");
        let url = if path_str.starts_with('/') {
            format!("sqlite://{path_str}")
        } else {
            format!("sqlite:///{path_str}")
        };
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
