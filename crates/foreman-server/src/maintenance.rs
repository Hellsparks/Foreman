//! Maintenance (task 5.3, W12): task views, and keeping the Attention item in step.
//!
//! A task's Attention item (kind `maintenance:<task id>`) is raised when it
//! reaches 100% of its interval and cleared once it drops below — after Mark done,
//! an interval edit or snooze, or deletion. "Due soon" only shows in Service.
//! [`evaluate_printer`] runs after every job ends; [`evaluate_all`] at startup and
//! hourly, so calendar intervals come due without a print.

use std::collections::HashMap;

use chrono::Utc;
use foreman_core::{
    error::AppError,
    events::{AttentionItemEvent, ServerEvent},
    maintenance::{
        self, Counters, DueState, MaintenanceTask, Progress, ATTENTION_PREFIX,
        DEFAULT_DUE_SOON_PCT, DUE_SOON_SETTING,
    },
    repo,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::broadcast;
use tracing::{info, warn};
use ts_rs::TS;

/// A task with where it stands.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TaskView {
    pub task: MaintenanceTask,
    pub printer_name: String,
    pub counters: Counters,
    pub progress: Progress,
    /// When it was last marked done, if ever.
    pub last_done: Option<String>,
    /// How much one snooze adds, in the task's unit.
    pub snooze_amount: f64,
}

/// `GET /api/maintenance`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Overview {
    pub due_soon_pct: f64,
    /// Overdue first (most overdue on top), then due soon, then the rest.
    pub tasks: Vec<TaskView>,
}

pub fn attention_kind(task_id: i64) -> String {
    format!("{ATTENTION_PREFIX}{task_id}")
}

pub async fn soon_pct(pool: &SqlitePool) -> f64 {
    repo::setting::get(pool, DUE_SOON_SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| (1.0..=100.0).contains(v))
        .unwrap_or(DEFAULT_DUE_SOON_PCT)
}

pub async fn set_soon_pct(pool: &SqlitePool, pct: f64) -> Result<f64, AppError> {
    if !(1.0..=100.0).contains(&pct) {
        return Err(AppError::InvalidInput(
            "due-soon threshold must be 1–100 %".into(),
        ));
    }
    repo::setting::set(pool, DUE_SOON_SETTING, &pct.to_string()).await?;
    Ok(pct)
}

async fn printer_names(pool: &SqlitePool) -> Result<HashMap<i64, String>, AppError> {
    Ok(repo::printer::list(pool)
        .await?
        .into_iter()
        .map(|p| (p.id, p.name))
        .collect())
}

pub async fn view(
    pool: &SqlitePool,
    task: MaintenanceTask,
    printer_name: String,
    pct: f64,
) -> Result<TaskView, AppError> {
    let counters =
        repo::maintenance::counters(pool, task.printer_id, task.since.as_deref()).await?;
    let progress = maintenance::progress(&task, &counters, Utc::now(), pct);
    let last_done = repo::maintenance::history(pool, Some(task.id), None)
        .await?
        .into_iter()
        .next()
        .map(|l| l.done_at);
    let snooze_amount = task.interval * maintenance::snooze_step(&task);
    Ok(TaskView {
        task,
        printer_name,
        counters,
        progress,
        last_done,
        snooze_amount,
    })
}

pub async fn task_view(pool: &SqlitePool, id: i64) -> Result<TaskView, AppError> {
    let task = repo::maintenance::get_task(pool, id).await?;
    let name = repo::printer::get(pool, task.printer_id).await?.name;
    view(pool, task, name, soon_pct(pool).await).await
}

fn rank(s: DueState) -> u8 {
    match s {
        DueState::Overdue => 0,
        DueState::Soon => 1,
        DueState::Ok => 2,
    }
}

pub async fn overview(pool: &SqlitePool, printer_id: Option<i64>) -> Result<Overview, AppError> {
    let pct = soon_pct(pool).await;
    let names = printer_names(pool).await?;
    let mut tasks = vec![];
    for t in repo::maintenance::list_tasks(pool, printer_id).await? {
        let name = names.get(&t.printer_id).cloned().unwrap_or_default();
        tasks.push(view(pool, t, name, pct).await?);
    }
    tasks.sort_by(|a, b| {
        rank(a.progress.state)
            .cmp(&rank(b.progress.state))
            .then(b.progress.ratio.total_cmp(&a.progress.ratio))
    });
    Ok(Overview {
        due_soon_pct: pct,
        tasks,
    })
}

/// Raise, retitle or clear the task's Attention item to match its progress.
pub async fn sync_attention(
    pool: &SqlitePool,
    bus: &broadcast::Sender<ServerEvent>,
    task: &MaintenanceTask,
    printer_name: &str,
    p: &Progress,
) -> Result<(), AppError> {
    let kind = attention_kind(task.id);
    let active = repo::attention::get_active_by_printer_kind(pool, task.printer_id, &kind).await?;
    if p.state == DueState::Overdue {
        let title = maintenance::attention_title(task, printer_name, p);
        let body = task.notes.clone();
        match active {
            None => {
                let a = repo::attention::raise(
                    pool,
                    Some(task.printer_id),
                    &kind,
                    &title,
                    body.as_deref(),
                )
                .await?;
                info!(task = task.id, printer_id = task.printer_id, "{title}");
                let _ = bus.send(ServerEvent::AttentionAdded {
                    item: AttentionItemEvent {
                        id: a.id,
                        printer_id: Some(task.printer_id),
                        kind,
                        title: a.title,
                        body: a.body,
                    },
                });
            }
            Some(a) if a.title != title => {
                repo::attention::set_title(pool, a.id, &title).await?;
                let _ = bus.send(ServerEvent::AttentionUpdated {
                    item: AttentionItemEvent {
                        id: a.id,
                        printer_id: Some(task.printer_id),
                        kind,
                        title,
                        body: a.body,
                    },
                });
            }
            Some(_) => {}
        }
    } else if active.is_some() {
        clear_attention(pool, bus, task.printer_id, task.id).await?;
    }
    Ok(())
}

/// Clear the task's Attention item(s).
pub async fn clear_attention(
    pool: &SqlitePool,
    bus: &broadcast::Sender<ServerEvent>,
    printer_id: i64,
    task_id: i64,
) -> Result<(), AppError> {
    let kind = attention_kind(task_id);
    while let Some(a) = repo::attention::get_active_by_printer_kind(pool, printer_id, &kind).await?
    {
        repo::attention::dismiss(pool, a.id).await?;
        let _ = bus.send(ServerEvent::AttentionCleared {
            item_id: a.id,
            printer_id: Some(printer_id),
        });
    }
    Ok(())
}

/// Re-check one task and sync its Attention item.
pub async fn evaluate_task(
    pool: &SqlitePool,
    bus: &broadcast::Sender<ServerEvent>,
    task_id: i64,
) -> Result<TaskView, AppError> {
    let v = task_view(pool, task_id).await?;
    sync_attention(pool, bus, &v.task, &v.printer_name, &v.progress).await?;
    Ok(v)
}

/// Re-check every task of a printer (after a job ends).
pub async fn evaluate_printer(
    pool: &SqlitePool,
    bus: &broadcast::Sender<ServerEvent>,
    printer_id: i64,
) -> Result<(), AppError> {
    let name = repo::printer::get(pool, printer_id).await?.name;
    let pct = soon_pct(pool).await;
    for t in repo::maintenance::list_tasks(pool, Some(printer_id)).await? {
        let v = view(pool, t, name.clone(), pct).await?;
        sync_attention(pool, bus, &v.task, &v.printer_name, &v.progress).await?;
    }
    Ok(())
}

/// Re-check every task (startup and hourly, for calendar intervals).
pub async fn evaluate_all(pool: &SqlitePool, bus: &broadcast::Sender<ServerEvent>) {
    let printers = match repo::printer::list(pool).await {
        Ok(p) => p,
        Err(e) => {
            warn!("maintenance: list printers: {e}");
            return;
        }
    };
    for p in printers {
        if let Err(e) = evaluate_printer(pool, bus, p.id).await {
            warn!(printer_id = p.id, "maintenance evaluate: {e}");
        }
    }
}
