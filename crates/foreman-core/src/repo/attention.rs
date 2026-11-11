//! Repository functions for the `attention_item` table.
//!
//! Only "active" items (cleared_at IS NULL) are exposed to callers.
//! Open items are unique per (printer_id, kind); `kind` carries the subject
//! where needed (`maintenance:<task_id>`). [`raise`] reuses an open item
//! instead of inserting a duplicate.

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

/// Result of [`raise`]: the open item, and whether it was newly inserted
/// (`false` = an open item for the same key already existed and was updated).
#[derive(Debug, Clone)]
pub struct Raised {
    pub item: AttentionItem,
    pub created: bool,
}

/// Raise an attention item, unique per open (printer_id, kind).
///
/// `kind` carries the subject when there can be several per printer
/// (e.g. `maintenance:<task_id>`), so the key is effectively
/// (printer, kind, subject). If an open item with the same key exists it is
/// reused: its title and body are updated and no new row is inserted. A
/// partial unique index (`ux_attention_item_open`) backs this up in the DB.
pub async fn raise(
    pool: &SqlitePool,
    printer_id: Option<i64>,
    kind: &str,
    title: &str,
    body: Option<&str>,
) -> Result<Raised, AppError> {
    if let Some(existing) = find_open(pool, printer_id, kind).await? {
        sqlx::query("UPDATE attention_item SET title = ?, body = ? WHERE id = ?")
            .bind(title)
            .bind(body)
            .bind(existing)
            .execute(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        return Ok(Raised {
            item: get(pool, existing).await?,
            created: false,
        });
    }
    let now = Utc::now().to_rfc3339();
    let inserted = sqlx::query(
        "INSERT INTO attention_item (printer_id, kind, title, body, actions, created_at) \
         VALUES (?, ?, ?, ?, '[]', ?)",
    )
    .bind(printer_id)
    .bind(kind)
    .bind(title)
    .bind(body)
    .bind(&now)
    .execute(pool)
    .await;
    match inserted {
        Ok(r) => Ok(Raised {
            item: get(pool, r.last_insert_rowid()).await?,
            created: true,
        }),
        // Lost a race against a concurrent raise of the same key: reuse it.
        Err(e) if e.to_string().contains("UNIQUE") => {
            match find_open(pool, printer_id, kind).await? {
                Some(id) => Ok(Raised {
                    item: get(pool, id).await?,
                    created: false,
                }),
                None => Err(AppError::Database(e.to_string())),
            }
        }
        Err(e) => Err(AppError::Database(e.to_string())),
    }
}

async fn find_open(
    pool: &SqlitePool,
    printer_id: Option<i64>,
    kind: &str,
) -> Result<Option<i64>, AppError> {
    sqlx::query_scalar(
        "SELECT id FROM attention_item \
         WHERE printer_id IS ? AND kind = ? AND cleared_at IS NULL \
         ORDER BY id LIMIT 1",
    )
    .bind(printer_id)
    .bind(kind)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))
}

async fn get(pool: &SqlitePool, id: i64) -> Result<AttentionItem, AppError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ForemanDb;

    async fn setup(path: &std::path::Path) -> ForemanDb {
        let db = ForemanDb::connect(path).await.unwrap();
        sqlx::query(
            "INSERT OR IGNORE INTO printer (id, name, firmware, tags, created_at, updated_at) \
             VALUES (1, 'HellV0', 'moonraker', '[]', 'x', 'x')",
        )
        .execute(&db.pool)
        .await
        .unwrap();
        db
    }

    #[tokio::test]
    async fn raise_reuses_open_item_per_printer_kind() {
        let dir = tempfile::tempdir().unwrap();
        let db = setup(&dir.path().join("a.db")).await;
        let a = raise(&db.pool, Some(1), "offline", "HellV0: offline", None)
            .await
            .unwrap();
        assert!(a.created);
        let b = raise(
            &db.pool,
            Some(1),
            "offline",
            "HellV0: offline (2)",
            Some("x"),
        )
        .await
        .unwrap();
        assert!(!b.created);
        assert_eq!(a.item.id, b.item.id);
        assert_eq!(b.item.title, "HellV0: offline (2)");
        // A different subject is a different item.
        let c = raise(&db.pool, Some(1), "maintenance:7", "t", None)
            .await
            .unwrap();
        assert!(c.created);
        assert_eq!(list_active(&db.pool).await.unwrap().len(), 2);
        // After clearing, a new raise inserts a fresh item.
        dismiss(&db.pool, a.item.id).await.unwrap();
        let d = raise(&db.pool, Some(1), "offline", "again", None)
            .await
            .unwrap();
        assert!(d.created);
        assert_ne!(d.item.id, a.item.id);
    }

    #[tokio::test]
    async fn migration_clears_existing_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let db = setup(&path).await;
        // Simulate the pre-fix state: drop the index, insert four duplicates.
        sqlx::query("DROP INDEX ux_attention_item_open")
            .execute(&db.pool)
            .await
            .unwrap();
        for _ in 0..4 {
            sqlx::query(
                "INSERT INTO attention_item (printer_id, kind, title, actions, created_at) \
                 VALUES (1, 'offline', 'HellV0: offline', '[]', '2026-11-11T00:00:00Z')",
            )
            .execute(&db.pool)
            .await
            .unwrap();
        }
        db.pool.close().await;
        let db = ForemanDb::connect(&path).await.unwrap();
        let open = list_active(&db.pool).await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].kind, "offline");
    }
}
