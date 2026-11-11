//! Repository functions for the `attention_item` table.
//!
//! Only "active" items (cleared_at IS NULL) are exposed to callers.
//! Items raised automatically by the driver manager (kind "offline" / "error")
//! are keyed by (printer_id, kind) so duplicates are suppressed.

use chrono::Utc;
use sqlx::{FromRow, SqlitePool};

use crate::{domain::AttentionItem, error::AppError};

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

#[derive(FromRow)]
struct AttentionRow {
    id: i64,
    printer_id: Option<i64>,
    kind: String,
    title: String,
    body: Option<String>,
    job_id: Option<i64>,
    created_at: String,
}

impl AttentionRow {
    fn into_domain(self) -> AttentionItem {
        AttentionItem {
            id: self.id,
            printer_id: self.printer_id,
            kind: self.kind,
            title: self.title,
            body: self.body,
            job_id: self.job_id,
            created_at: self.created_at.parse().unwrap_or_else(|_| Utc::now()),
        }
    }
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

/// List all active (not-yet-cleared) attention items, newest first.
pub async fn list_active(pool: &SqlitePool) -> Result<Vec<AttentionItem>, AppError> {
    let rows: Vec<AttentionRow> = sqlx::query_as(
        "SELECT id, printer_id, kind, title, body, job_id, created_at \
         FROM attention_item \
         WHERE cleared_at IS NULL \
         ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;

    Ok(rows.into_iter().map(|r| r.into_domain()).collect())
}

/// Return the first active item for (printer_id, kind), if any.
pub async fn get_active_by_printer_kind(
    pool: &SqlitePool,
    printer_id: i64,
    kind: &str,
) -> Result<Option<AttentionItem>, AppError> {
    let row: Option<AttentionRow> = sqlx::query_as(
        "SELECT id, printer_id, kind, title, body, job_id, created_at \
         FROM attention_item \
         WHERE printer_id = ? AND kind = ? AND cleared_at IS NULL \
         LIMIT 1",
    )
    .bind(printer_id)
    .bind(kind)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;

    Ok(row.map(|r| r.into_domain()))
}

/// Insert a new attention item and return it.
pub async fn raise(
    pool: &SqlitePool,
    printer_id: Option<i64>,
    kind: &str,
    title: &str,
    body: Option<&str>,
) -> Result<AttentionItem, AppError> {
    let now = Utc::now().to_rfc3339();
    let id = sqlx::query(
        "INSERT INTO attention_item (printer_id, kind, title, body, actions, created_at) \
         VALUES (?, ?, ?, ?, '[]', ?)",
    )
    .bind(printer_id)
    .bind(kind)
    .bind(title)
    .bind(body)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?
    .last_insert_rowid();

    let row: AttentionRow = sqlx::query_as(
        "SELECT id, printer_id, kind, title, body, job_id, created_at \
         FROM attention_item WHERE id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;

    Ok(row.into_domain())
}

/// Link an attention item to a job (the "print finished" item → its job).
pub async fn set_job(pool: &SqlitePool, id: i64, job_id: i64) -> Result<(), AppError> {
    sqlx::query("UPDATE attention_item SET job_id = ? WHERE id = ?")
        .bind(job_id)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}

/// Mark a single attention item as cleared.
pub async fn dismiss(pool: &SqlitePool, id: i64) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE attention_item SET cleared_at = ? WHERE id = ?")
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}

/// Clear all active items for a given (printer_id, kind).
pub async fn clear_by_printer_kind(
    pool: &SqlitePool,
    printer_id: i64,
    kind: &str,
) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE attention_item SET cleared_at = ? \
         WHERE printer_id = ? AND kind = ? AND cleared_at IS NULL",
    )
    .bind(&now)
    .bind(printer_id)
    .bind(kind)
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}

/// Change an item's title (e.g. a maintenance counter that kept accruing).
pub async fn set_title(pool: &SqlitePool, id: i64, title: &str) -> Result<(), AppError> {
    sqlx::query("UPDATE attention_item SET title = ? WHERE id = ?")
        .bind(title)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}
