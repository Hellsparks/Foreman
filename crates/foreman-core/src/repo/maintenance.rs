//! Repository functions for `maintenance_task` and `maintenance_log` (task 5.3).

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use ts_rs::TS;

use crate::{
    error::AppError,
    maintenance::{Basis, Counters, MaintenanceTask, TaskForm},
};

fn db(e: sqlx::Error) -> AppError {
    AppError::Database(e.to_string())
}

#[derive(FromRow)]
struct TaskRow {
    id: i64,
    printer_id: i64,
    name: String,
    notes: Option<String>,
    basis: String,
    interval: f64,
    every_days: Option<i64>,
    snooze: f64,
    since: Option<String>,
    created_at: String,
    updated_at: String,
}

impl TaskRow {
    fn into_domain(self) -> MaintenanceTask {
        MaintenanceTask {
            id: self.id,
            printer_id: self.printer_id,
            name: self.name,
            notes: self.notes,
            basis: Basis::parse(&self.basis).unwrap_or(Basis::Hours),
            interval: self.interval,
            every_days: self.every_days,
            snooze: self.snooze,
            since: self.since,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

const TASK_COLS: &str = "id, printer_id, name, notes, basis, interval, every_days, snooze, \
     since, created_at, updated_at";

fn clean_notes(n: &Option<String>) -> Option<&str> {
    n.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// All tasks, or one printer's, by printer then name.
pub async fn list_tasks(
    pool: &SqlitePool,
    printer_id: Option<i64>,
) -> Result<Vec<MaintenanceTask>, AppError> {
    let sql = format!(
        "SELECT {TASK_COLS} FROM maintenance_task \
         WHERE (? IS NULL OR printer_id = ?) ORDER BY printer_id, name COLLATE NOCASE, id"
    );
    let rows: Vec<TaskRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(printer_id)
        .bind(printer_id)
        .fetch_all(pool)
        .await
        .map_err(db)?;
    Ok(rows.into_iter().map(TaskRow::into_domain).collect())
}

pub async fn get_task(pool: &SqlitePool, id: i64) -> Result<MaintenanceTask, AppError> {
    let sql = format!("SELECT {TASK_COLS} FROM maintenance_task WHERE id = ?");
    let row: Option<TaskRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db)?;
    row.map(TaskRow::into_domain)
        .ok_or_else(|| AppError::NotFound(format!("maintenance task {id}")))
}

/// Insert a task for `printer_id`. `start_now` starts its counter at zero.
pub async fn create_task(
    pool: &SqlitePool,
    printer_id: i64,
    f: &TaskForm,
) -> Result<MaintenanceTask, AppError> {
    f.validate()?;
    let now = Utc::now().to_rfc3339();
    let since = f.start_now.then(|| now.clone());
    let id = sqlx::query(
        "INSERT INTO maintenance_task (printer_id, name, notes, basis, interval, every_days, \
         since, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(printer_id)
    .bind(f.name.trim())
    .bind(clean_notes(&f.notes))
    .bind(f.basis.as_str())
    .bind(f.interval)
    .bind(f.every_days)
    .bind(since)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(db)?
    .last_insert_rowid();
    get_task(pool, id).await
}

/// Edit name, notes and intervals (the counter is untouched).
pub async fn update_task(
    pool: &SqlitePool,
    id: i64,
    f: &TaskForm,
) -> Result<MaintenanceTask, AppError> {
    f.validate()?;
    let n = sqlx::query(
        "UPDATE maintenance_task SET name = ?, notes = ?, basis = ?, interval = ?, \
         every_days = ?, updated_at = ? WHERE id = ?",
    )
    .bind(f.name.trim())
    .bind(clean_notes(&f.notes))
    .bind(f.basis.as_str())
    .bind(f.interval)
    .bind(f.every_days)
    .bind(Utc::now().to_rfc3339())
    .bind(id)
    .execute(pool)
    .await
    .map_err(db)?
    .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound(format!("maintenance task {id}")));
    }
    get_task(pool, id).await
}

pub async fn delete_task(pool: &SqlitePool, id: i64) -> Result<bool, AppError> {
    let n = sqlx::query("DELETE FROM maintenance_task WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(db)?
        .rows_affected();
    Ok(n == 1)
}

pub async fn set_snooze(pool: &SqlitePool, id: i64, snooze: f64) -> Result<(), AppError> {
    sqlx::query("UPDATE maintenance_task SET snooze = ?, updated_at = ? WHERE id = ?")
        .bind(snooze.max(0.0))
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await
        .map_err(db)?;
    Ok(())
}

/// What accrued on `printer_id` from jobs that ended after `since` (None = all).
///
/// Hours and grams count jobs that ran to the end or errored out; the job count
/// counts completed jobs. Cancelled jobs — by the firmware or the user's verdict —
/// never count, nor do running ones.
///
/// `ended_at` and `since` are both written by Foreman as `Utc::now().to_rfc3339()`
/// (UTC, `+00:00`), so text order is time order — to the nanosecond, where SQLite's
/// `julianday()` would round to the millisecond.
pub async fn counters(
    pool: &SqlitePool,
    printer_id: i64,
    since: Option<&str>,
) -> Result<Counters, AppError> {
    let (secs, grams, jobs): (f64, f64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(COALESCE(duration_secs, 0)), 0) * 1.0, \
                COALESCE(SUM(COALESCE(grams_used, 0)), 0) * 1.0, \
                COALESCE(SUM(CASE WHEN end_state = 'complete' THEN 1 ELSE 0 END), 0) \
         FROM job \
         WHERE printer_id = ? AND end_state IN ('complete', 'error') \
           AND COALESCE(outcome, '') != 'cancelled' \
           AND (? IS NULL OR ended_at > ?)",
    )
    .bind(printer_id)
    .bind(since)
    .bind(since)
    .fetch_one(pool)
    .await
    .map_err(db)?;
    Ok(Counters {
        hours: secs / 3600.0,
        grams,
        jobs,
    })
}

/// One "mark done".
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ServiceLog {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number | null")]
    pub task_id: Option<i64>,
    #[ts(type = "number")]
    pub printer_id: i64,
    pub task_name: String,
    pub basis: Basis,
    pub interval: f64,
    /// Counter value when it was done (what accrued since the previous service).
    pub counter: f64,
    /// Days since the previous service (or since the counter started).
    pub days: f64,
    /// True when no earlier service is on record (the gap runs from task creation
    /// or the printer's first job).
    pub first: bool,
    pub done_at: String,
    pub done_by: String,
    pub notes: Option<String>,
}

#[derive(FromRow)]
struct LogRow {
    id: i64,
    task_id: Option<i64>,
    printer_id: i64,
    task_name: String,
    basis: String,
    interval: f64,
    counter: f64,
    days: f64,
    done_at: String,
    done_by: String,
    notes: Option<String>,
    prev_since: Option<String>,
    prev_snooze: f64,
}

impl LogRow {
    fn into_domain(self) -> ServiceLog {
        ServiceLog {
            id: self.id,
            task_id: self.task_id,
            printer_id: self.printer_id,
            task_name: self.task_name,
            basis: Basis::parse(&self.basis).unwrap_or(Basis::Hours),
            interval: self.interval,
            counter: self.counter,
            days: self.days,
            first: self.prev_since.is_none(),
            done_at: self.done_at,
            done_by: self.done_by,
            notes: self.notes,
        }
    }
}

const LOG_COLS: &str = "id, task_id, printer_id, task_name, basis, interval, counter, days, \
     done_at, done_by, notes, prev_since, prev_snooze";

/// What [`mark_done`] records.
pub struct Done<'a> {
    pub counter: f64,
    pub days: f64,
    pub by: &'a str,
    pub notes: Option<&'a str>,
}

/// Log a service and reset the task's counter (and snooze) in one transaction.
pub async fn mark_done(
    pool: &SqlitePool,
    task: &MaintenanceTask,
    d: &Done<'_>,
) -> Result<ServiceLog, AppError> {
    let now = Utc::now().to_rfc3339();
    let mut tx = pool.begin().await.map_err(db)?;
    let id = sqlx::query(
        "INSERT INTO maintenance_log (task_id, printer_id, task_name, basis, interval, counter, \
         days, done_at, done_by, notes, prev_since, prev_snooze) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(task.id)
    .bind(task.printer_id)
    .bind(&task.name)
    .bind(task.basis.as_str())
    .bind(task.interval)
    .bind(d.counter)
    .bind(d.days)
    .bind(&now)
    .bind(d.by)
    .bind(d.notes.map(str::trim).filter(|s| !s.is_empty()))
    .bind(&task.since)
    .bind(task.snooze)
    .execute(&mut *tx)
    .await
    .map_err(db)?
    .last_insert_rowid();
    sqlx::query("UPDATE maintenance_task SET since = ?, snooze = 0, updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(&now)
        .bind(task.id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    get_log(pool, id).await
}

pub async fn get_log(pool: &SqlitePool, id: i64) -> Result<ServiceLog, AppError> {
    get_log_row(pool, id).await.map(LogRow::into_domain)
}

async fn get_log_row(pool: &SqlitePool, id: i64) -> Result<LogRow, AppError> {
    let sql = format!("SELECT {LOG_COLS} FROM maintenance_log WHERE id = ?");
    let row: Option<LogRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db)?;
    row.ok_or_else(|| AppError::NotFound(format!("service log {id}")))
}

/// Service history, newest first: for one task, one printer, or everything.
pub async fn history(
    pool: &SqlitePool,
    task_id: Option<i64>,
    printer_id: Option<i64>,
) -> Result<Vec<ServiceLog>, AppError> {
    let sql = format!(
        "SELECT {LOG_COLS} FROM maintenance_log \
         WHERE (? IS NULL OR task_id = ?) AND (? IS NULL OR printer_id = ?) \
         ORDER BY julianday(done_at) DESC, id DESC"
    );
    let rows: Vec<LogRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(task_id)
        .bind(task_id)
        .bind(printer_id)
        .bind(printer_id)
        .fetch_all(pool)
        .await
        .map_err(db)?;
    Ok(rows.into_iter().map(LogRow::into_domain).collect())
}

/// Undo a service: only the task's latest one, restoring its counter start and
/// snooze. Returns the task id it belonged to (None if the task was deleted).
pub async fn undo_done(pool: &SqlitePool, log_id: i64) -> Result<Option<i64>, AppError> {
    let row = get_log_row(pool, log_id).await?;
    let mut tx = pool.begin().await.map_err(db)?;
    if let Some(task_id) = row.task_id {
        let latest: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM maintenance_log WHERE task_id = ? \
             ORDER BY julianday(done_at) DESC, id DESC LIMIT 1",
        )
        .bind(task_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        if latest != Some(log_id) {
            return Err(AppError::InvalidInput(
                "only the latest service of a task can be undone".into(),
            ));
        }
        sqlx::query(
            "UPDATE maintenance_task SET since = ?, snooze = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&row.prev_since)
        .bind(row.prev_snooze)
        .bind(Utc::now().to_rfc3339())
        .bind(task_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    }
    sqlx::query("DELETE FROM maintenance_log WHERE id = ?")
        .bind(log_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(row.task_id)
}
