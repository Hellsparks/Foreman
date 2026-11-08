//! Settings database queries — mirrors upstream `spoolman/database/setting.py`.
//!
//! Settings are stored as JSON-encoded values in the `setting` table.
//! The settings registry (types + defaults) is defined in this module,
//! mirroring `spoolman/settings.py`.

use sqlx::AnyPool;

use crate::{db::vendor::utc_now_str, error::Result};

// ---------------------------------------------------------------------------
// Settings registry (mirrors spoolman/settings.py)
// ---------------------------------------------------------------------------

pub struct SettingDef {
    pub key: &'static str,
    pub type_name: &'static str, // "boolean" | "number" | "string" | "array" | "object"
    pub default: &'static str,   // JSON-encoded default value
}

pub static SETTINGS: &[SettingDef] = &[
    SettingDef {
        key: "currency",
        type_name: "string",
        default: "\"EUR\"",
    },
    SettingDef {
        key: "round_prices",
        type_name: "boolean",
        default: "false",
    },
    SettingDef {
        key: "print_presets",
        type_name: "array",
        default: "[]",
    },
    SettingDef {
        key: "label_designs",
        type_name: "array",
        default: "[]",
    },
    SettingDef {
        key: "extra_fields_vendor",
        type_name: "array",
        default: "[]",
    },
    SettingDef {
        key: "extra_fields_filament",
        type_name: "array",
        default: "[]",
    },
    SettingDef {
        key: "extra_fields_spool",
        type_name: "array",
        default: "[]",
    },
    SettingDef {
        key: "base_url",
        type_name: "string",
        default: "\"\"",
    },
    SettingDef {
        key: "locations",
        type_name: "array",
        default: "[]",
    },
    SettingDef {
        key: "locations_spoolorders",
        type_name: "object",
        default: "{}",
    },
    SettingDef {
        key: "dashboard_groups",
        type_name: "object",
        default: "{}",
    },
    SettingDef {
        key: "dashboard_spoolorders",
        type_name: "object",
        default: "{}",
    },
];

pub fn find_def(key: &str) -> Option<&'static SettingDef> {
    SETTINGS.iter().find(|s| s.key == key)
}

/// Check that `raw_value` is valid for `def`'s type.
/// Returns `Some(error_message)` on failure, `None` on success.
pub fn validate_type(def: &SettingDef, raw_value: &str) -> Option<String> {
    let parsed: serde_json::Value = match serde_json::from_str(raw_value) {
        Ok(v) => v,
        Err(e) => return Some(e.to_string()),
    };
    let ok = match def.type_name {
        "string" => parsed.is_string(),
        "boolean" => parsed.is_boolean(),
        "number" => parsed.is_number(),
        "array" => parsed.is_array(),
        "object" => parsed.is_object(),
        _ => true,
    };
    if ok {
        None
    } else {
        Some(format!("Setting {} must be a {}.", def.key, def.type_name))
    }
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

/// Get a setting's JSON value from the DB, or `None` if not set.
pub async fn get(pool: &AnyPool, key: &str) -> Result<Option<String>> {
    let row: Option<String> = sqlx::query_scalar("SELECT value FROM setting WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// Set (upsert) a setting.  `value` must be a JSON-encoded string.
pub async fn set(pool: &AnyPool, key: &str, value: &str) -> Result<()> {
    let now = utc_now_str();
    // SQLite and PostgreSQL: INSERT OR REPLACE / ON CONFLICT DO UPDATE.
    // MySQL: INSERT ... ON DUPLICATE KEY UPDATE.
    // Use separate branches detected at runtime.
    sqlx::query(
        "INSERT INTO setting (key, value, last_updated) VALUES (?,?,?) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, last_updated=excluded.last_updated",
    )
    .bind(key)
    .bind(value)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Set a setting using MySQL's ON DUPLICATE KEY UPDATE syntax.
pub async fn set_mysql(pool: &AnyPool, key: &str, value: &str) -> Result<()> {
    let now = utc_now_str();
    sqlx::query(
        "INSERT INTO setting (key, value, last_updated) VALUES (?,?,?) \
         ON DUPLICATE KEY UPDATE value=VALUES(value), last_updated=VALUES(last_updated)",
    )
    .bind(key)
    .bind(value)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete a setting (reset to default).
pub async fn delete(pool: &AnyPool, key: &str) -> Result<()> {
    sqlx::query("DELETE FROM setting WHERE key = ?")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get all settings that have been explicitly set (not defaults).
pub async fn get_all(pool: &AnyPool) -> Result<Vec<(String, String)>> {
    let rows = sqlx::query("SELECT key, value FROM setting")
        .fetch_all(pool)
        .await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        use sqlx::Row;
        let key: String = row.try_get("key")?;
        let value: String = row.try_get("value")?;
        result.push((key, value));
    }
    Ok(result)
}
