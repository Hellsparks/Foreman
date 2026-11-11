//! Repository functions for the `audit_log` table.

use chrono::Utc;
use sqlx::SqlitePool;

use crate::error::AppError;

/// Insert an audit log entry.
pub async fn insert(
    pool: &SqlitePool,
    printer_id: Option<i64>,
    user: Option<&str>,
    command: &str,
) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO audit_log (printer_id, user, command, occurred_at) VALUES (?, ?, ?, ?)",
    )
    .bind(printer_id)
    .bind(user)
    .bind(command)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}

/// One `audit_log` row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AuditEntry {
    pub printer_id: Option<i64>,
    pub user: Option<String>,
    pub command: String,
    pub occurred_at: String,
}

/// Newest entries first.
pub async fn recent(pool: &SqlitePool, limit: i64) -> Result<Vec<AuditEntry>, AppError> {
    sqlx::query_as::<_, AuditEntry>(
        "SELECT printer_id, user, command, occurred_at FROM audit_log ORDER BY id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))
}
