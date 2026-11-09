//! Key-value access to the `setting` table.

use chrono::Utc;
use sqlx::SqlitePool;

use crate::error::AppError;

pub async fn get(pool: &SqlitePool, key: &str) -> Result<Option<String>, AppError> {
    sqlx::query_scalar("SELECT value FROM setting WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))
}

pub async fn set(pool: &SqlitePool, key: &str, value: &str) -> Result<(), AppError> {
    sqlx::query("INSERT OR REPLACE INTO setting (key, value, updated_at) VALUES (?, ?, ?)")
        .bind(key)
        .bind(value)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}
