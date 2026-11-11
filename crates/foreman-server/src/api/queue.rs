//! Queue REST (task 3.5): the Jobs → Queue board, add / move / remove, start next now,
//! and "Plate cleared".
//!
//! - `GET    /api/queue`                          — board: fleet lane + one lane per printer
//! - `POST   /api/queue`                          — `{file_id, printer_id?}` add (null = fleet)
//! - `POST   /api/queue/{id}/move`                — `{printer_id?, index}` reorder / move lanes
//! - `DELETE /api/queue/{id}`                     — remove (waiting/failed) or cancel (sending)
//! - `POST   /api/printers/{id}/queue/start-next` — `{plate_confirm, dry_run}`
//! - `POST   /api/printers/{id}/plate-cleared`    — open the plate gate
//!
//! Every change broadcasts `queue_changed`; clients refetch the board.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use foreman_core::{
    compat::{self, CompatResult, Verdict},
    domain::{QueueItem, QueueSource, QueueStatus},
    error::AppError,
    events::PrinterStatus,
    repo,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use crate::{auth::AuthCtx, state::AppState};

use super::send::{build_snapshot_for, build_snapshots};

type ApiErr = (StatusCode, Json<serde_json::Value>);

pub(crate) fn err(e: AppError) -> ApiErr {
    let status = match &e {
        AppError::NotFound(_) => StatusCode::NOT_FOUND,
        AppError::InvalidInput(_) => StatusCode::CONFLICT,
        AppError::Driver(_) => StatusCode::BAD_GATEWAY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({ "error": e.to_string() })))
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// One card on the queue board.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct QueueCard {
    pub item: QueueItem,
    pub thumb_url: Option<String>,
    /// First tool's material and colour (`#RRGGBB`).
    pub material: Option<String>,
    pub colour: Option<String>,
    #[ts(type = "number | null")]
    pub print_time_secs: Option<i64>,
    pub grams: Option<f64>,
    /// Chip tone: `ok` | `warn` | `bad` | `off` | `info`.
    pub tone: String,
    /// Chip text, e.g. `✓ Ready · next`, `✓ 2 printers can take it`.
    pub tag: String,
}

/// A printer's queue, or the fleet queue (`printer_id` null).
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct QueueLane {
    #[ts(type = "number | null")]
    pub printer_id: Option<i64>,
    pub name: String,
    pub plate_clear: bool,
    pub auto_start: bool,
    /// Last known printer status (clients prefer newer `printer_status` events).
    pub status: Option<PrinterStatus>,
    pub items: Vec<QueueCard>,
}

/// `GET /api/queue`.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct QueueBoard {
    pub lanes: Vec<QueueLane>,
}

/// Where an item sits after an add or a move.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct QueuePositions {
    #[ts(type = "number")]
    pub item_id: i64,
    /// Lane (null = fleet queue).
    #[ts(type = "number | null")]
    pub printer_id: Option<i64>,
    /// 1-based position of the item in its lane.
    pub position: u32,
    /// The lane's waiting items in order.
    #[ts(type = "number[]")]
    pub item_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AddBody {
    pub file_id: i64,
    #[serde(default)]
    pub printer_id: Option<i64>,
    /// The file swaps filament by hand (M600); queue it anyway.
    #[serde(default)]
    pub accept_manual_swaps: bool,
}

#[derive(Debug, Deserialize)]
pub struct MoveBody {
    #[serde(default)]
    pub printer_id: Option<i64>,
    pub index: usize,
}

#[derive(Debug, Deserialize, Default)]
pub struct StartNextBody {
    #[serde(default)]
    pub plate_confirm: bool,
    #[serde(default)]
    pub dry_run: bool,
}

/// `POST /api/printers/{id}/queue/start-next` result.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct StartNextResult {
    pub ok: bool,
    pub message: String,
    pub dry_run: bool,
    pub file: String,
    #[ts(type = "number")]
    pub item_id: i64,
}

// ---------------------------------------------------------------------------
// Shared with Send and the slicer upload
// ---------------------------------------------------------------------------

/// Refuse a printer lane the file can never print on (too big, wrong nozzle).
fn hard_mismatch(r: &CompatResult) -> bool {
    matches!(r.verdict, Verdict::TooBig | Verdict::Nozzle)
}

async fn check_lane(
    state: &AppState,
    file_id: i64,
    printer_id: Option<i64>,
) -> Result<(), AppError> {
    let Some(pid) = printer_id else {
        return Ok(());
    };
    let snap = build_snapshot_for(state, pid)
        .await
        .ok_or_else(|| AppError::NotFound(format!("printer {pid}")))?;
    let Some(meta) = state.library.details(file_id).await?.meta else {
        return Err(AppError::InvalidInput(
            "file has no parsed metadata (not a printable file?)".into(),
        ));
    };
    let r = compat::check(&meta, &snap);
    if hard_mismatch(&r) {
        return Err(AppError::InvalidInput(format!(
            "{}: {}",
            r.printer_name, r.reason
        )));
    }
    Ok(())
}

/// Add a Library file to a lane and wake the scheduler.
pub(crate) async fn enqueue(
    state: &AppState,
    file_id: i64,
    printer_id: Option<i64>,
    source: QueueSource,
    added_by: &str,
) -> Result<QueuePositions, AppError> {
    let entry = state.library.get(file_id).await?;
    if entry.format.as_deref() == Some("stl") {
        return Err(AppError::InvalidInput(
            "STL files must be sliced first".into(),
        ));
    }
    check_lane(state, file_id, printer_id).await?;
    let item = repo::queue::add(
        &state.foreman_db.pool,
        file_id,
        &entry.name,
        printer_id,
        source,
        added_by,
    )
    .await?;
    tracing::info!(item = item.id, file = %entry.name, ?printer_id, source = source.as_str(), "queued");
    let item_ids: Vec<i64> = repo::queue::list_waiting(&state.foreman_db.pool, printer_id)
        .await?
        .iter()
        .map(|i| i.id)
        .collect();
    state.scheduler.queue_changed(printer_id);
    Ok(positions(item.id, printer_id, item_ids))
}

fn positions(item_id: i64, printer_id: Option<i64>, item_ids: Vec<i64>) -> QueuePositions {
    let position = item_ids
        .iter()
        .position(|&i| i == item_id)
        .map(|p| p as u32 + 1)
        .unwrap_or(0);
    QueuePositions {
        item_id,
        printer_id,
        position,
        item_ids,
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/queue`
pub async fn board(State(state): State<AppState>) -> Result<Json<QueueBoard>, ApiErr> {
    let pool = &state.foreman_db.pool;
    let items = repo::queue::list_board(pool).await.map_err(err)?;
    let snaps = build_snapshots(&state).await;

    let mut lanes = vec![QueueLane {
        printer_id: None,
        name: "Fleet queue".into(),
        plate_clear: true,
        auto_start: true,
        status: None,
        items: Vec::new(),
    }];
    for s in &snaps {
        lanes.push(QueueLane {
            printer_id: Some(s.printer.id),
            name: s.printer.name.clone(),
            plate_clear: s.printer.plate_clear,
            auto_start: s.printer.auto_start_from_queue,
            status: s.status.clone(),
            items: Vec::new(),
        });
    }

    for item in items {
        let entry = state.library.get(item.file_id).await.ok();
        let meta = state
            .library
            .details(item.file_id)
            .await
            .ok()
            .and_then(|d| d.meta);
        let lane_idx = lanes
            .iter()
            .position(|l| l.printer_id == item.printer_id)
            .unwrap_or(0);
        let is_head = item.status == QueueStatus::Waiting
            && !lanes[lane_idx]
                .items
                .iter()
                .any(|c| c.item.status == QueueStatus::Waiting);
        let (tone, tag) = match item.status {
            QueueStatus::Printing => ("info", "▶ Printing".to_owned()),
            QueueStatus::Sending => ("info", "↑ Sending to printer".to_owned()),
            QueueStatus::Failed => (
                "bad",
                format!(
                    "✖ Failed: {}",
                    item.error.as_deref().unwrap_or("unknown error")
                ),
            ),
            QueueStatus::Done | QueueStatus::Cancelled => ("off", item.status.as_str().to_owned()),
            QueueStatus::Waiting => match &meta {
                None => ("bad", "✖ File not printable or missing".to_owned()),
                Some(m) => match item.printer_id {
                    Some(pid) => match snaps.iter().find(|s| s.printer.id == pid) {
                        None => ("off", "○ Printer removed".to_owned()),
                        Some(snap) => {
                            let r = compat::check(m, snap);
                            lane_tag(&r, is_head, snap.printer.auto_start_from_queue)
                        }
                    },
                    None => fleet_tag(&compat::check_all(m, &snaps)),
                },
            },
        };
        let first = entry.as_ref().and_then(|e| e.tools.first().cloned());
        let grams: Option<f64> = entry.as_ref().and_then(|e| {
            let g: f64 = e.tools.iter().filter_map(|t| t.grams).sum();
            (g > 0.0).then_some(g)
        });
        lanes[lane_idx].items.push(QueueCard {
            thumb_url: entry.as_ref().and_then(|e| e.thumb_url.clone()),
            material: first.as_ref().and_then(|t| t.material.clone()),
            colour: first.as_ref().and_then(|t| t.colour.clone()),
            print_time_secs: entry.as_ref().and_then(|e| e.print_time_secs),
            grams,
            tone: tone.into(),
            tag,
            item,
        });
    }
    Ok(Json(QueueBoard { lanes }))
}

fn lane_tag(r: &CompatResult, is_head: bool, auto_start: bool) -> (&'static str, String) {
    match r.verdict {
        Verdict::Ready if is_head && !auto_start => ("ok", "✓ Ready · auto-start off".into()),
        Verdict::Ready if is_head => ("ok", "✓ Ready · next".into()),
        Verdict::Ready => ("ok", "✓ Ready".into()),
        Verdict::Busy => ("info", format!("◷ Waits · {}", r.reason)),
        Verdict::Plate => ("warn", "◐ Clear plate first".into()),
        Verdict::Offline => ("off", "○ Printer offline".into()),
        Verdict::Material | Verdict::NoSpool => ("warn", format!("⚠ {}", r.reason)),
        Verdict::TooBig | Verdict::Nozzle => ("bad", format!("✖ {}", r.reason)),
    }
}

fn fleet_tag(results: &[CompatResult]) -> (&'static str, String) {
    let can = results
        .iter()
        .filter(|r| matches!(r.verdict, Verdict::Ready | Verdict::Busy | Verdict::Plate))
        .count();
    match can {
        0 if results.is_empty() => ("off", "○ No printers".into()),
        0 => match results.iter().find(|r| r.verdict != Verdict::Offline) {
            Some(r) => ("bad", format!("✖ No printer: {}", r.reason)),
            None => ("off", "○ All printers offline".into()),
        },
        1 => ("ok", "✓ 1 printer can take it".into()),
        n => ("ok", format!("✓ {n} printers can take it")),
    }
}

/// `POST /api/queue`
pub async fn add(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Json(body): Json<AddBody>,
) -> Result<impl IntoResponse, ApiErr> {
    if !body.accept_manual_swaps {
        if let Some(n) = crate::projects::manual_swap_colours(&state, body.file_id).await {
            let name = state
                .library
                .get(body.file_id)
                .await
                .map(|e| e.name)
                .unwrap_or_default();
            return Err((
                StatusCode::CONFLICT,
                Json(crate::projects::manual_swaps_body(&name, n)),
            ));
        }
    }
    let p = enqueue(
        &state,
        body.file_id,
        body.printer_id,
        QueueSource::Manual,
        &ctx.username,
    )
    .await
    .map_err(err)?;
    Ok((StatusCode::CREATED, Json(p)))
}

/// `POST /api/queue/{id}/move`
pub async fn move_item(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<MoveBody>,
) -> Result<Json<QueuePositions>, ApiErr> {
    let pool = &state.foreman_db.pool;
    let before = repo::queue::get(pool, id).await.map_err(err)?;
    if before.printer_id != body.printer_id {
        check_lane(&state, before.file_id, body.printer_id)
            .await
            .map_err(err)?;
    }
    let ids = repo::queue::move_to(pool, id, body.printer_id, body.index)
        .await
        .map_err(err)?;
    if before.printer_id != body.printer_id {
        state.scheduler.queue_changed(before.printer_id);
    }
    state.scheduler.queue_changed(body.printer_id);
    Ok(Json(positions(id, body.printer_id, ids)))
}

/// `DELETE /api/queue/{id}`
pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiErr> {
    let pool = &state.foreman_db.pool;
    let item = repo::queue::get(pool, id).await.map_err(err)?;
    let r = repo::queue::remove(pool, id).await.map_err(err)?;
    state.scheduler.queue_changed(item.printer_id);
    let what = match r {
        repo::queue::Removed::Deleted => "deleted",
        repo::queue::Removed::Cancelled => "cancelled",
    };
    Ok(Json(json!({ "removed": what })))
}

/// `POST /api/printers/{id}/queue/start-next`
pub async fn start_next(
    State(state): State<AppState>,
    Path(printer_id): Path<i64>,
    body: Option<Json<StartNextBody>>,
) -> Result<Json<StartNextResult>, ApiErr> {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let r = state
        .scheduler
        .start_next(printer_id, b.plate_confirm, b.dry_run)
        .await
        .map_err(err)?;
    Ok(Json(StartNextResult {
        ok: true,
        message: r.message,
        dry_run: r.dry_run,
        file: r.file,
        item_id: r.item_id,
    }))
}

/// `POST /api/printers/{id}/plate-cleared`
pub async fn plate_cleared(
    State(state): State<AppState>,
    Path(printer_id): Path<i64>,
) -> Result<StatusCode, ApiErr> {
    state
        .scheduler
        .plate_cleared(printer_id)
        .await
        .map_err(err)?;
    Ok(StatusCode::NO_CONTENT)
}
