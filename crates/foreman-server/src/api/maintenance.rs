//! Maintenance (task 5.3, W12).
//!
//! - `GET  /api/maintenance?printer_id=` — tasks with progress, due work first
//! - `PUT  /api/maintenance/settings` — `{due_soon_pct}`
//! - `GET  /api/maintenance/templates` — ready-made tasks
//! - `POST /api/maintenance/from-templates` — `{printer_id, keys}`
//! - `POST /api/maintenance/tasks` · `PUT/DELETE /api/maintenance/tasks/{id}`
//! - `POST /api/maintenance/tasks/{id}/done` — `{notes}`: log + reset counter
//! - `POST /api/maintenance/tasks/{id}/snooze` — `{snooze}` (fraction of interval)
//! - `GET  /api/maintenance/history?task_id=&printer_id=` — newest first
//! - `DELETE /api/maintenance/history/{id}` — undo the task's latest service

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use foreman_core::{
    error::AppError,
    maintenance::{self, TaskForm, TaskTemplate},
    repo::{self, maintenance::ServiceLog},
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::spools::{err, ApiErr};
use crate::{
    auth::AuthCtx,
    maintenance::{self as svc, Overview, TaskView},
    state::AppState,
};

#[derive(Deserialize)]
pub struct PrinterQuery {
    #[serde(default)]
    printer_id: Option<i64>,
}

pub async fn overview(
    State(s): State<AppState>,
    Query(q): Query<PrinterQuery>,
) -> Result<Json<Overview>, ApiErr> {
    svc::overview(&s.foreman_db.pool, q.printer_id)
        .await
        .map(Json)
        .map_err(err)
}

#[derive(Deserialize)]
pub struct SettingsBody {
    due_soon_pct: f64,
}

pub async fn put_settings(
    State(s): State<AppState>,
    Json(b): Json<SettingsBody>,
) -> Result<Json<Value>, ApiErr> {
    let pool = &s.foreman_db.pool;
    let pct = svc::set_soon_pct(pool, b.due_soon_pct).await.map_err(err)?;
    Ok(Json(json!({ "due_soon_pct": pct })))
}

pub async fn templates() -> Json<&'static [TaskTemplate]> {
    Json(maintenance::TEMPLATES)
}

#[derive(Deserialize)]
pub struct FromTemplates {
    printer_id: i64,
    keys: Vec<String>,
    /// Start counters at zero (a printer that was just serviced or is new).
    #[serde(default)]
    start_now: bool,
}

pub async fn from_templates(
    State(s): State<AppState>,
    Json(b): Json<FromTemplates>,
) -> Result<(StatusCode, Json<Vec<TaskView>>), ApiErr> {
    let pool = &s.foreman_db.pool;
    repo::printer::get(pool, b.printer_id).await.map_err(err)?;
    let mut out = vec![];
    for key in &b.keys {
        let t = maintenance::template(key)
            .ok_or_else(|| err(AppError::InvalidInput(format!("unknown template {key}"))))?;
        let form = TaskForm {
            printer_id: Some(b.printer_id),
            name: t.name.into(),
            notes: Some(t.notes.into()),
            basis: t.basis,
            interval: t.interval,
            every_days: t.every_days,
            start_now: b.start_now,
        };
        let task = repo::maintenance::create_task(pool, b.printer_id, &form)
            .await
            .map_err(err)?;
        out.push(
            svc::evaluate_task(pool, &s.bus_tx, task.id)
                .await
                .map_err(err)?,
        );
    }
    Ok((StatusCode::CREATED, Json(out)))
}

pub async fn create_task(
    State(s): State<AppState>,
    Json(f): Json<TaskForm>,
) -> Result<(StatusCode, Json<TaskView>), ApiErr> {
    let pool = &s.foreman_db.pool;
    let printer_id = f
        .printer_id
        .ok_or_else(|| err(AppError::InvalidInput("printer_id is required".into())))?;
    repo::printer::get(pool, printer_id).await.map_err(err)?;
    let task = repo::maintenance::create_task(pool, printer_id, &f)
        .await
        .map_err(err)?;
    svc::evaluate_task(pool, &s.bus_tx, task.id)
        .await
        .map(|v| (StatusCode::CREATED, Json(v)))
        .map_err(err)
}

pub async fn update_task(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(f): Json<TaskForm>,
) -> Result<Json<TaskView>, ApiErr> {
    let pool = &s.foreman_db.pool;
    repo::maintenance::update_task(pool, id, &f)
        .await
        .map_err(err)?;
    svc::evaluate_task(pool, &s.bus_tx, id)
        .await
        .map(Json)
        .map_err(err)
}

pub async fn delete_task(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiErr> {
    let pool = &s.foreman_db.pool;
    let task = repo::maintenance::get_task(pool, id).await.map_err(err)?;
    svc::clear_attention(pool, &s.bus_tx, task.printer_id, id)
        .await
        .map_err(err)?;
    repo::maintenance::delete_task(pool, id)
        .await
        .map_err(err)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Default)]
pub struct DoneBody {
    #[serde(default)]
    notes: Option<String>,
}

pub async fn mark_done(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Extension(ctx): Extension<AuthCtx>,
    body: Option<Json<DoneBody>>,
) -> Result<Json<Value>, ApiErr> {
    let pool = &s.foreman_db.pool;
    let before = svc::task_view(pool, id).await.map_err(err)?;
    let notes = body.and_then(|Json(b)| b.notes);
    let log = repo::maintenance::mark_done(
        pool,
        &before.task,
        &repo::maintenance::Done {
            counter: before.progress.value,
            days: before.progress.days,
            by: &ctx.username,
            notes: notes.as_deref(),
        },
    )
    .await
    .map_err(err)?;
    let after = svc::evaluate_task(pool, &s.bus_tx, id).await.map_err(err)?;
    tracing::info!(
        task = id,
        printer_id = after.task.printer_id,
        counter = log.counter,
        by = %log.done_by,
        "maintenance done"
    );
    Ok(Json(json!({ "task": after, "log": log })))
}

#[derive(Deserialize)]
pub struct SnoozeBody {
    /// Absolute snooze (fraction of the interval). Omit to add one step.
    #[serde(default)]
    snooze: Option<f64>,
}

pub async fn snooze(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(b): Json<SnoozeBody>,
) -> Result<Json<TaskView>, ApiErr> {
    let pool = &s.foreman_db.pool;
    let task = repo::maintenance::get_task(pool, id).await.map_err(err)?;
    let value = b
        .snooze
        .unwrap_or(task.snooze + maintenance::snooze_step(&task));
    if !(value.is_finite() && (0.0..=10.0).contains(&value)) {
        return Err(err(AppError::InvalidInput("snooze out of range".into())));
    }
    repo::maintenance::set_snooze(pool, id, value)
        .await
        .map_err(err)?;
    svc::evaluate_task(pool, &s.bus_tx, id)
        .await
        .map(Json)
        .map_err(err)
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    #[serde(default)]
    task_id: Option<i64>,
    #[serde(default)]
    printer_id: Option<i64>,
}

pub async fn history(
    State(s): State<AppState>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Vec<ServiceLog>>, ApiErr> {
    repo::maintenance::history(&s.foreman_db.pool, q.task_id, q.printer_id)
        .await
        .map(Json)
        .map_err(err)
}

pub async fn undo_done(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiErr> {
    let pool = &s.foreman_db.pool;
    if let Some(task_id) = repo::maintenance::undo_done(pool, id).await.map_err(err)? {
        svc::evaluate_task(pool, &s.bus_tx, task_id)
            .await
            .map_err(err)?;
    }
    Ok(StatusCode::NO_CONTENT)
}
