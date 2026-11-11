//! Repository functions for the `tracked_filament` table (task 4.3).

use chrono::Utc;
use sqlx::{FromRow, SqlitePool};

use crate::error::AppError;

/// Stock targets for one Spoolman filament.
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRow)]
pub struct Tracked {
    pub filament_id: i64,
    pub target: i64,
    pub buy_below: i64,
}

fn db(e: sqlx::Error) -> AppError {
    AppError::Database(e.to_string())
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Tracked>, AppError> {
    sqlx::query_as(
        "SELECT filament_id, target, buy_below FROM tracked_filament ORDER BY filament_id",
    )
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// Track a filament, or change its numbers.
pub async fn upsert(pool: &SqlitePool, t: Tracked) -> Result<Tracked, AppError> {
    sqlx::query(
        "INSERT INTO tracked_filament (filament_id, target, buy_below, updated_at) VALUES (?, ?, ?, ?) \
         ON CONFLICT(filament_id) DO UPDATE SET target = excluded.target, \
         buy_below = excluded.buy_below, updated_at = excluded.updated_at",
    )
    .bind(t.filament_id)
    .bind(t.target)
    .bind(t.buy_below)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(t)
}

/// Stop tracking. Returns whether it was tracked.
pub async fn remove(pool: &SqlitePool, filament_id: i64) -> Result<bool, AppError> {
    let n = sqlx::query("DELETE FROM tracked_filament WHERE filament_id = ?")
        .bind(filament_id)
        .execute(pool)
        .await
        .map_err(db)?
        .rows_affected();
    Ok(n > 0)
}
