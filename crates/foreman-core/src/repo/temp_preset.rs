//! Repository functions for the `temp_preset` table.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TempPreset {
    pub id: i64,
    pub printer_id: i64,
    pub name: String,
    pub extruder_temp: Option<i64>,
    pub bed_temp: Option<i64>,
    pub chamber_temp: Option<i64>,
    pub sort_order: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateTempPreset {
    pub name: String,
    pub extruder_temp: Option<i64>,
    pub bed_temp: Option<i64>,
    pub chamber_temp: Option<i64>,
}

/// List all temp presets for a printer, ordered by sort_order.
pub async fn list(pool: &SqlitePool, printer_id: i64) -> Result<Vec<TempPreset>, AppError> {
    let rows: Vec<TempPreset> = sqlx::query_as(
        "SELECT id, printer_id, name, extruder_temp, bed_temp, chamber_temp, sort_order \
         FROM temp_preset WHERE printer_id = ? ORDER BY sort_order, id",
    )
    .bind(printer_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(rows)
}

/// Create a new temp preset.
pub async fn create(
    pool: &SqlitePool,
    printer_id: i64,
    req: CreateTempPreset,
) -> Result<TempPreset, AppError> {
    let id = sqlx::query(
        "INSERT INTO temp_preset (printer_id, name, extruder_temp, bed_temp, chamber_temp, sort_order) \
         VALUES (?, ?, ?, ?, ?, 0)",
    )
    .bind(printer_id)
    .bind(&req.name)
    .bind(req.extruder_temp)
    .bind(req.bed_temp)
    .bind(req.chamber_temp)
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?
    .last_insert_rowid();

    let row: TempPreset = sqlx::query_as(
        "SELECT id, printer_id, name, extruder_temp, bed_temp, chamber_temp, sort_order \
         FROM temp_preset WHERE id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(row)
}

/// Delete a temp preset by ID.
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<(), AppError> {
    sqlx::query("DELETE FROM temp_preset WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}
