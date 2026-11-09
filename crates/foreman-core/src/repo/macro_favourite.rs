//! Repository functions for the `macro_favourite` table.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MacroFavourite {
    pub id: i64,
    pub printer_id: i64,
    pub macro_name: String,
}

/// List all macro favourites for a printer.
pub async fn list(pool: &SqlitePool, printer_id: i64) -> Result<Vec<MacroFavourite>, AppError> {
    let rows: Vec<MacroFavourite> = sqlx::query_as(
        "SELECT id, printer_id, macro_name FROM macro_favourite WHERE printer_id = ? ORDER BY id",
    )
    .bind(printer_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(rows)
}

/// Add a macro to favourites. Returns the new row (or error if duplicate).
pub async fn add(
    pool: &SqlitePool,
    printer_id: i64,
    macro_name: &str,
) -> Result<MacroFavourite, AppError> {
    let id =
        sqlx::query("INSERT OR IGNORE INTO macro_favourite (printer_id, macro_name) VALUES (?, ?)")
            .bind(printer_id)
            .bind(macro_name)
            .execute(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
            .last_insert_rowid();

    if id == 0 {
        // Already existed (INSERT OR IGNORE returned 0 rows affected).
        let row: MacroFavourite = sqlx::query_as(
            "SELECT id, printer_id, macro_name FROM macro_favourite \
             WHERE printer_id = ? AND macro_name = ?",
        )
        .bind(printer_id)
        .bind(macro_name)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
        return Ok(row);
    }

    Ok(MacroFavourite {
        id,
        printer_id,
        macro_name: macro_name.to_owned(),
    })
}

/// Remove a macro from favourites by ID.
pub async fn remove(pool: &SqlitePool, id: i64) -> Result<(), AppError> {
    sqlx::query("DELETE FROM macro_favourite WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}
