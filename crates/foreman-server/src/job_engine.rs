//! Job engine (task 3.6, W7).
//!
//! The scheduler feeds it printer transitions:
//! - **start** (a print begins, sent by Foreman or started on the printer): open a
//!   `job` row with the file's per-tool plan and who reports filament use;
//! - **end** (printing/paused → idle or error): close the row once with how it ended
//!   (`complete` / `cancelled` / `error`, from the firmware's own end state), work
//!   out the grams used, and deduct them from Spoolman when Foreman is the reporter.
//!
//! Deduction runs at most once per job: [`repo::job::claim_deduction`] is a
//! conditional `none → pending` update, the job row survives restarts, and a
//! printer that is still printing after a restart re-attaches to its running job
//! instead of opening a new one.
//!
//! Reporter: `usage_reporter = auto` means Moonraker when its `[spoolman]` link is
//! connected (it reports usage itself), else Foreman. Set per printer.

use chrono::Utc;
use foreman_core::{
    compat::PrinterSnapshot,
    domain::{FirmwareKind, Job, JobEnd, JobOutcome, JobTool, QueueItem, UsageReporter},
    error::AppError,
    events::{PrintEnd, PrinterState, PrinterStatus, ServerEvent},
    repo::{
        self,
        job::{JobClose, NewJob},
    },
};
use serde_json::json;
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::scheduler::SchedEnv;

/// Who reports this printer's filament use, given its setting and live link.
pub fn resolve_reporter(snap: &PrinterSnapshot) -> &'static str {
    match snap.printer.usage_reporter {
        UsageReporter::Moonraker => "moonraker",
        UsageReporter::Foreman => "foreman",
        UsageReporter::Auto => {
            let linked = snap.printer.firmware == FirmwareKind::Moonraker
                && snap
                    .caps
                    .as_ref()
                    .and_then(|c| c.spoolman_connected)
                    .unwrap_or(false);
            if linked {
                "moonraker"
            } else {
                "foreman"
            }
        }
    }
}

fn emit(bus: &broadcast::Sender<ServerEvent>, printer_id: i64, kind: &str, job_id: i64) {
    let _ = bus.send(ServerEvent::Job {
        printer_id: Some(printer_id),
        payload: json!({ "kind": kind, "job_id": job_id }),
    });
}

/// Open a running job for `printer_id` unless one is already open (a restart or a
/// printer coming back online mid-print re-attaches to it). `file_id` is known when
/// Foreman sent the file; for prints started on the printer only the name is.
pub async fn job_started(
    pool: &sqlx::SqlitePool,
    env: &dyn SchedEnv,
    bus: &broadcast::Sender<ServerEvent>,
    printer_id: i64,
    file_id: Option<i64>,
    file_name: Option<&str>,
    queue_item: Option<&QueueItem>,
) -> Result<Job, AppError> {
    if let Some(j) = repo::job::running_for_printer(pool, printer_id).await? {
        return Ok(j);
    }
    let snap = env
        .snapshots()
        .await
        .into_iter()
        .find(|s| s.printer.id == printer_id);
    let printer_name = match &snap {
        Some(s) => s.printer.name.clone(),
        None => repo::printer::get(pool, printer_id).await?.name,
    };
    let reporter = snap.as_ref().map(resolve_reporter).unwrap_or("foreman");
    let file_id = file_id.or(queue_item.map(|q| q.file_id));
    let meta = match file_id {
        Some(id) => env.file_meta(id).await.ok(),
        None => None,
    };
    let mut tools: Vec<JobTool> = meta
        .as_ref()
        .map(|m| {
            m.tools
                .iter()
                .map(|t| JobTool {
                    index: t.index,
                    material: t.material.clone(),
                    colour: t.colour.clone(),
                    grams_planned: t.grams,
                    grams_used: None,
                    grams_deducted: 0.0,
                    spool_id: t.spool_id,
                })
                .collect()
        })
        .unwrap_or_default();
    // A single-tool file whose preset names a Spoolman spool.
    if let (Some(m), [t]) = (&meta, tools.as_mut_slice()) {
        if t.spool_id.is_none() {
            t.spool_id = m.suggested_spool_id;
        }
    }
    // Moonraker's active spool (recorded for history; it reports use itself).
    if let Some(active) = snap
        .as_ref()
        .and_then(|s| s.caps.as_ref())
        .and_then(|c| c.spoolman_spool_id)
    {
        match tools.first_mut() {
            Some(t) if t.spool_id.is_none() => t.spool_id = Some(active),
            None => tools.push(JobTool {
                spool_id: Some(active),
                ..Default::default()
            }),
            _ => {}
        }
    }
    let name = file_name
        .map(str::to_owned)
        .or(queue_item.map(|q| q.file_name.clone()))
        .unwrap_or_else(|| "(unknown file)".into());
    let thumb_url = match file_id {
        Some(id) => env.thumb_url(id).await,
        None => None,
    };
    let job = repo::job::open(
        pool,
        &NewJob {
            printer_id,
            printer_name,
            file_id,
            file_name: name,
            queue_item_id: queue_item.map(|q| q.id),
            usage_reporter: reporter.into(),
            tools,
            thumb_url,
        },
    )
    .await?;
    info!(job = job.id, printer_id, file = %job.file_name, reporter, "job started");
    emit(bus, printer_id, "started", job.id);
    Ok(job)
}

/// How the print ended: the firmware's own end state when it reports one, else
/// idle = complete and error = error.
pub fn end_of(status: &PrinterStatus) -> JobEnd {
    match status.print_end {
        Some(PrintEnd::Complete) => JobEnd::Complete,
        Some(PrintEnd::Cancelled) => JobEnd::Cancelled,
        Some(PrintEnd::Error) => JobEnd::Error,
        None if status.state == PrinterState::Error => JobEnd::Error,
        None => JobEnd::Complete,
    }
}

/// Share of the planned filament this print used (1.0 for a full print).
fn used_fraction(end: JobEnd, status: &PrinterStatus, planned_mm: Option<f64>) -> f64 {
    if end == JobEnd::Complete {
        return 1.0;
    }
    let by_mm = status
        .filament_used_mm
        .zip(planned_mm)
        .filter(|(_, p)| *p > 0.0)
        .map(|(u, p)| u / p);
    by_mm.or(status.progress).unwrap_or(0.0).clamp(0.0, 1.0)
}

/// Close the printer's running job and deduct filament once. Returns the job (or
/// None when no job was running).
pub async fn job_ended(
    pool: &sqlx::SqlitePool,
    env: &dyn SchedEnv,
    bus: &broadcast::Sender<ServerEvent>,
    printer_id: i64,
    status: &PrinterStatus,
) -> Result<Option<Job>, AppError> {
    let Some(job) = repo::job::running_for_printer(pool, printer_id).await? else {
        return Ok(None);
    };
    let end = end_of(status);
    let planned_mm = match job.file_id {
        Some(id) => env
            .file_meta(id)
            .await
            .ok()
            .map(|m| m.tools.iter().filter_map(|t| t.mm).sum::<f64>()),
        None => None,
    };
    let frac = used_fraction(end, status, planned_mm);
    let tools: Vec<JobTool> = job
        .tools
        .iter()
        .cloned()
        .map(|mut t| {
            t.grams_used = t.grams_planned.map(|g| round2(g * frac));
            t
        })
        .collect();
    let duration_secs = chrono::DateTime::parse_from_rfc3339(&job.started_at)
        .ok()
        .map(|s| (Utc::now() - s.with_timezone(&Utc)).num_seconds().max(0));
    let (outcome, error) = match end {
        JobEnd::Cancelled => (Some(JobOutcome::Cancelled), None),
        JobEnd::Error => (
            Some(JobOutcome::Failed),
            Some(
                status
                    .print_message
                    .clone()
                    .unwrap_or_else(|| "printer reported an error".into()),
            ),
        ),
        _ => (None, None),
    };
    let closed = repo::job::close(
        pool,
        job.id,
        &JobClose {
            end,
            duration_secs,
            error,
            outcome,
            tools: tools.clone(),
        },
    )
    .await?;
    if !closed {
        return Ok(Some(repo::job::get(pool, job.id).await?));
    }
    info!(
        job = job.id,
        printer_id,
        end = end.as_str(),
        frac,
        "job ended"
    );
    deduct_once(pool, env, job.id, &job.usage_reporter, tools).await?;
    emit(bus, printer_id, "ended", job.id);
    Ok(Some(repo::job::get(pool, job.id).await?))
}

/// Deduct the job's used grams from its spools, at most once per job.
pub async fn deduct_once(
    pool: &sqlx::SqlitePool,
    env: &dyn SchedEnv,
    job_id: i64,
    reporter: &str,
    mut tools: Vec<JobTool>,
) -> Result<(), AppError> {
    if !repo::job::claim_deduction(pool, job_id).await? {
        info!(
            job = job_id,
            "deduction already claimed; not deducting again"
        );
        return Ok(());
    }
    if reporter != "foreman" {
        info!(
            job = job_id,
            reporter, "filament use reported by the printer; Foreman does not deduct"
        );
        return repo::job::finish_deduction(pool, job_id, &tools, "skipped").await;
    }
    for t in tools.iter_mut() {
        let (Some(spool), Some(g)) = (t.spool_id, t.grams_used) else {
            continue;
        };
        if g <= 0.0 {
            continue;
        }
        match env.deduct(spool, g).await {
            Ok(()) => {
                t.grams_deducted = g;
                info!(
                    job = job_id,
                    spool,
                    grams = g,
                    tool = t.index,
                    "filament deducted"
                );
            }
            Err(e) => warn!(
                job = job_id,
                spool,
                grams = g,
                "filament deduction failed: {e}"
            ),
        }
    }
    repo::job::finish_deduction(pool, job_id, &tools, "done").await
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
