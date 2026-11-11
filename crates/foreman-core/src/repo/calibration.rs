//! Calibration records (task 5.4, W10): skew per printer, shrinkage per filament.
//! The current shrinkage lives on the filament (Spoolman `orcaslicer_config`);
//! these rows are the history behind it.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use ts_rs::TS;

use crate::{
    calibration::{PlaneSkew, ShrinkageInput},
    error::AppError,
};

fn db(e: sqlx::Error) -> AppError {
    AppError::Database(e.to_string())
}

/// One skew measurement of a printer. `applied_at` is set once the owner pressed
/// Apply and the commands went to the printer.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SkewRecord {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub printer_id: i64,
    pub planes: Vec<PlaneSkew>,
    pub measured_at: String,
    pub measured_by: String,
    pub applied_at: Option<String>,
    pub applied_by: Option<String>,
    pub notes: Option<String>,
}

#[derive(FromRow)]
struct SkewRow {
    id: i64,
    printer_id: i64,
    planes_json: String,
    measured_at: String,
    measured_by: String,
    applied_at: Option<String>,
    applied_by: Option<String>,
    notes: Option<String>,
}

impl SkewRow {
    fn into_domain(self) -> SkewRecord {
        SkewRecord {
            id: self.id,
            printer_id: self.printer_id,
            planes: serde_json::from_str(&self.planes_json).unwrap_or_default(),
            measured_at: self.measured_at,
            measured_by: self.measured_by,
            applied_at: self.applied_at,
            applied_by: self.applied_by,
            notes: self.notes,
        }
    }
}

const SKEW_COLS: &str =
    "id, printer_id, planes_json, measured_at, measured_by, applied_at, applied_by, notes";

pub async fn insert_skew(
    pool: &SqlitePool,
    printer_id: i64,
    planes: &[PlaneSkew],
    by: &str,
) -> Result<SkewRecord, AppError> {
    let json = serde_json::to_string(planes).map_err(|e| AppError::Internal(e.to_string()))?;
    let id = sqlx::query(
        "INSERT INTO skew_calibration (printer_id, planes_json, measured_at, measured_by) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(printer_id)
    .bind(json)
    .bind(Utc::now().to_rfc3339())
    .bind(by)
    .execute(pool)
    .await
    .map_err(db)?
    .last_insert_rowid();
    get_skew(pool, id).await
}

pub async fn get_skew(pool: &SqlitePool, id: i64) -> Result<SkewRecord, AppError> {
    sqlx::query_as::<_, SkewRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {SKEW_COLS} FROM skew_calibration WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(db)?
    .map(SkewRow::into_domain)
    .ok_or_else(|| AppError::NotFound(format!("skew measurement #{id}")))
}

/// Newest first; all printers or one.
pub async fn list_skew(
    pool: &SqlitePool,
    printer_id: Option<i64>,
) -> Result<Vec<SkewRecord>, AppError> {
    let rows: Vec<SkewRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {SKEW_COLS} FROM skew_calibration \
         WHERE (? IS NULL OR printer_id = ?) ORDER BY id DESC"
    )))
    .bind(printer_id)
    .bind(printer_id)
    .fetch_all(pool)
    .await
    .map_err(db)?;
    Ok(rows.into_iter().map(SkewRow::into_domain).collect())
}

pub async fn mark_skew_applied(
    pool: &SqlitePool,
    id: i64,
    by: &str,
    notes: Option<&str>,
) -> Result<SkewRecord, AppError> {
    sqlx::query(
        "UPDATE skew_calibration SET applied_at = ?, applied_by = ?, notes = ? WHERE id = ?",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(by)
    .bind(notes)
    .bind(id)
    .execute(pool)
    .await
    .map_err(db)?;
    get_skew(pool, id).await
}

/// One shrinkage result saved to a filament.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ShrinkageRecord {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub filament_id: i64,
    pub filament_name: String,
    #[ts(type = "number | null")]
    pub printer_id: Option<i64>,
    pub printer_name: Option<String>,
    /// The test printer had an applied Foreman skew correction at the time.
    pub printer_skew_corrected: bool,
    pub input: ShrinkageInput,
    pub shrinkage_xy: Option<f64>,
    pub shrinkage_z: Option<f64>,
    pub saved_at: String,
    pub saved_by: String,
}

#[derive(FromRow)]
struct ShrinkRow {
    id: i64,
    filament_id: i64,
    filament_name: String,
    printer_id: Option<i64>,
    printer_name: Option<String>,
    printer_skew_corrected: bool,
    input_json: String,
    shrinkage_xy: Option<f64>,
    shrinkage_z: Option<f64>,
    saved_at: String,
    saved_by: String,
}

impl ShrinkRow {
    fn into_domain(self) -> ShrinkageRecord {
        ShrinkageRecord {
            id: self.id,
            filament_id: self.filament_id,
            filament_name: self.filament_name,
            printer_id: self.printer_id,
            printer_name: self.printer_name,
            printer_skew_corrected: self.printer_skew_corrected,
            input: serde_json::from_str(&self.input_json).unwrap_or(ShrinkageInput {
                x: vec![],
                y: vec![],
                z: vec![],
                cal_err: None,
            }),
            shrinkage_xy: self.shrinkage_xy,
            shrinkage_z: self.shrinkage_z,
            saved_at: self.saved_at,
            saved_by: self.saved_by,
        }
    }
}

const SHRINK_COLS: &str = "id, filament_id, filament_name, printer_id, printer_name, \
     printer_skew_corrected, input_json, shrinkage_xy, shrinkage_z, saved_at, saved_by";

#[allow(clippy::too_many_arguments)]
pub async fn insert_shrinkage(
    pool: &SqlitePool,
    filament_id: i64,
    filament_name: &str,
    printer: Option<(i64, &str)>,
    printer_skew_corrected: bool,
    input: &ShrinkageInput,
    shrinkage_xy: Option<f64>,
    shrinkage_z: Option<f64>,
    by: &str,
) -> Result<ShrinkageRecord, AppError> {
    let json = serde_json::to_string(input).map_err(|e| AppError::Internal(e.to_string()))?;
    let id = sqlx::query(
        "INSERT INTO shrinkage_calibration (filament_id, filament_name, printer_id, printer_name, \
         printer_skew_corrected, input_json, shrinkage_xy, shrinkage_z, saved_at, saved_by) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(filament_id)
    .bind(filament_name)
    .bind(printer.map(|p| p.0))
    .bind(printer.map(|p| p.1))
    .bind(printer_skew_corrected)
    .bind(json)
    .bind(shrinkage_xy)
    .bind(shrinkage_z)
    .bind(Utc::now().to_rfc3339())
    .bind(by)
    .execute(pool)
    .await
    .map_err(db)?
    .last_insert_rowid();
    let row: ShrinkRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {SHRINK_COLS} FROM shrinkage_calibration WHERE id = ?"
    )))
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(db)?;
    Ok(row.into_domain())
}

/// Newest first; all filaments or one.
pub async fn list_shrinkage(
    pool: &SqlitePool,
    filament_id: Option<i64>,
) -> Result<Vec<ShrinkageRecord>, AppError> {
    let rows: Vec<ShrinkRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {SHRINK_COLS} FROM shrinkage_calibration \
         WHERE (? IS NULL OR filament_id = ?) ORDER BY id DESC"
    )))
    .bind(filament_id)
    .bind(filament_id)
    .fetch_all(pool)
    .await
    .map_err(db)?;
    Ok(rows.into_iter().map(ShrinkRow::into_domain).collect())
}
