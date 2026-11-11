//! Send API (task 3.4): print a Library file on a printer.
//!
//! Modes:
//! - `print_now`: start the print immediately (upload to printer + start).
//! - `queue`: add to the printer's queue (session 24).
//! - `fleet_queue`: add to the fleet queue (session 24).
//!
//! Both queue modes return the item's position and the lane's order.
//!
//! `GET /api/send/check/{file_id}` returns per-printer compatibility results.

use axum::Extension;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use foreman_core::{
    compat::{self, CompatResult, PrinterSnapshot, Verdict},
    domain::{FileMeta, QueueSource},
    error::AppError,
    repo,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::info;
use ts_rs::TS;

use crate::{
    api::queue::{enqueue, QueuePositions},
    auth::AuthCtx,
    state::AppState,
};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// Request body for `POST /api/send`.
#[derive(Debug, Deserialize)]
pub struct SendRequest {
    /// Library entry id of the file to print.
    pub file_id: i64,
    /// Target printer id (required for `print_now` and `queue`).
    pub printer_id: Option<i64>,
    /// `print_now` | `queue` | `fleet_queue`.
    pub mode: String,
    /// For `print_now`: the user confirms the plate is clear.
    #[serde(default)]
    pub plate_confirm: bool,
    /// For `print_now`: run compat and the upload, then stop before the start
    /// command and report what would have happened. Changes no printer state.
    #[serde(default)]
    pub dry_run: bool,
    /// The file swaps filament by hand (M600); send / queue it anyway.
    #[serde(default)]
    pub accept_manual_swaps: bool,
}

/// Successful `print_now` response.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct SendResult {
    pub ok: bool,
    pub message: String,
    /// True when this was a dry run: the file was uploaded but no print started.
    pub dry_run: bool,
    /// Name of the file on the printer that was (or, on a dry run, would be) started.
    pub file: String,
    /// Printer the file was sent to (null for the fleet queue).
    #[ts(type = "number | null")]
    pub printer_id: Option<i64>,
    /// Queue modes: where the item landed and the lane's new order.
    pub queue: Option<QueuePositions>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/send/check/{file_id}` — compatibility check for every printer.
pub async fn check_file(
    State(state): State<AppState>,
    Path(file_id): Path<i64>,
) -> Result<Json<Vec<CompatResult>>, (StatusCode, Json<serde_json::Value>)> {
    let meta = get_file_meta(&state, file_id).await.map_err(api_err)?;

    let snapshots = build_snapshots(&state).await;
    let results = compat::check_all(&meta, &snapshots);
    Ok(Json(results))
}

/// `POST /api/send` — send a file to a printer.
pub async fn send_file(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Json(req): Json<SendRequest>,
) -> Result<(StatusCode, Json<SendResult>), (StatusCode, Json<serde_json::Value>)> {
    if !req.accept_manual_swaps {
        if let Some(n) = crate::projects::manual_swap_colours(&state, req.file_id).await {
            let name = state
                .library
                .get(req.file_id)
                .await
                .map(|e| e.name)
                .unwrap_or_default();
            return Err((
                StatusCode::CONFLICT,
                Json(crate::projects::manual_swaps_body(&name, n)),
            ));
        }
    }
    match req.mode.as_str() {
        "print_now" => print_now(state, req).await,
        "queue" | "fleet_queue" => {
            let lane = if req.mode == "queue" {
                Some(req.printer_id.ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": "printer_id required for queue" })),
                    )
                })?)
            } else {
                None
            };
            let entry = state.library.get(req.file_id).await.map_err(api_err)?;
            let p = enqueue(
                &state,
                req.file_id,
                lane,
                QueueSource::Manual,
                &ctx.username,
            )
            .await
            .map_err(crate::api::queue::err)?;
            let lane_name = match lane {
                Some(id) => repo::printer::get(&state.foreman_db.pool, id)
                    .await
                    .map(|p| format!("{}'s queue", p.name))
                    .unwrap_or_else(|_| "the printer queue".into()),
                None => "the fleet queue".into(),
            };
            Ok((
                StatusCode::OK,
                Json(SendResult {
                    ok: true,
                    message: format!(
                        "Queued {} in {lane_name} at position {}",
                        entry.name, p.position
                    ),
                    dry_run: false,
                    file: entry.name,
                    printer_id: lane,
                    queue: Some(p),
                }),
            ))
        }
        other => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("unknown mode: {other}") })),
        )),
    }
}

/// Print now: upload the file to the printer's storage and start printing.
async fn print_now(
    state: AppState,
    req: SendRequest,
) -> Result<(StatusCode, Json<SendResult>), (StatusCode, Json<serde_json::Value>)> {
    let printer_id = req.printer_id.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "printer_id required for print_now" })),
        )
    })?;

    // No scheduler tick may dispatch to this printer while Print now runs.
    let _hold = state.scheduler.hold().await;
    if let Ok(Some(a)) = repo::queue::active_for_printer(&state.foreman_db.pool, printer_id).await {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("printer already has a queued job: {} is {}", a.file_name, a.status.as_str()),
            })),
        ));
    }

    // 1. Fetch file details.
    let meta = get_file_meta(&state, req.file_id).await.map_err(api_err)?;

    // 2. Run compatibility check.
    let snap = build_snapshot_for(&state, printer_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "printer not found" })),
            )
        })?;
    let compat_result = compat::check(&meta, &snap);

    // Allow Ready, or Plate if the user confirmed.
    match &compat_result.verdict {
        Verdict::Ready => {}
        Verdict::Plate if req.plate_confirm && req.dry_run => {}
        Verdict::Plate if req.plate_confirm => {
            // User says plate is clear — mark it and close the "clear the plate" item.
            let _ = repo::printer::set_plate_clear(&state.foreman_db.pool, printer_id, true).await;
            let _ = state.scheduler.clear_plate_attention(printer_id).await;
        }
        other => {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": format!("printer not ready: {}", compat_result.reason),
                    "verdict": other,
                    "reason": compat_result.reason,
                })),
            ));
        }
    }

    // 3. Read the file from the Library.
    let entry = state.library.get(req.file_id).await.map_err(api_err)?;
    let abs_path = state
        .library
        .file_path(req.file_id)
        .await
        .map_err(api_err)?;

    // 4. Get the printer's storage and upload the file.
    let (_name, storage) = state
        .driver_manager
        .storage(printer_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "printer has no accessible storage" })),
            )
        })?;

    let dest_name = entry.name.clone();
    let progress = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    storage
        .upload(&dest_name, &abs_path, progress)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": format!("upload failed: {e}") })),
            )
        })?;

    if req.dry_run {
        info!(printer_id, file = %dest_name, "print_now dry run: uploaded, start skipped");
        return Ok((
            StatusCode::OK,
            Json(SendResult {
                ok: true,
                message: format!(
                    "Dry run: uploaded {dest_name} to printer {printer_id}; would start printing it (start command not sent)"
                ),
                dry_run: true,
                file: dest_name,
                printer_id: Some(printer_id),
                queue: None,
            }),
        ));
    }

    // 5. Start the print.
    let driver = state
        .driver_manager
        .get_driver(printer_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "no driver for printer" })),
            )
        })?;
    state.scheduler.expect_start(printer_id).await;
    driver.start_print(&dest_name).await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": format!("start_print failed: {e}") })),
        )
    })?;

    info!(printer_id, file = %dest_name, "print_now: print started");

    // 6. Mark plate as not clear (a print is starting).
    let _ = repo::printer::set_plate_clear(&state.foreman_db.pool, printer_id, false).await;
    state
        .scheduler
        .job_sent(printer_id, req.file_id, &dest_name)
        .await;
    state.scheduler.queue_changed(Some(printer_id));

    Ok((
        StatusCode::OK,
        Json(SendResult {
            ok: true,
            message: format!("Printing {} on printer {}", dest_name, printer_id),
            dry_run: false,
            file: dest_name,
            printer_id: Some(printer_id),
            queue: None,
        }),
    ))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the full parsed metadata for a library file.
async fn get_file_meta(state: &AppState, file_id: i64) -> Result<FileMeta, AppError> {
    let details = state.library.details(file_id).await?;
    details.meta.ok_or_else(|| {
        AppError::InvalidInput("file has no parsed metadata (not a printable file?)".into())
    })
}

/// Build a compatibility snapshot for a single printer.
pub(crate) async fn build_snapshot_for(
    state: &AppState,
    printer_id: i64,
) -> Option<PrinterSnapshot> {
    let printer = repo::printer::get(&state.foreman_db.pool, printer_id)
        .await
        .ok()?;
    Some(crate::spools::snapshot(&state.driver_manager, state.spoolman.get(), printer).await)
}

/// Build compatibility snapshots for all printers.
pub(crate) async fn build_snapshots(state: &AppState) -> Vec<PrinterSnapshot> {
    let printers = repo::printer::list(&state.foreman_db.pool)
        .await
        .unwrap_or_default();
    let mut snaps = Vec::with_capacity(printers.len());
    for p in printers {
        snaps.push(crate::spools::snapshot(&state.driver_manager, state.spoolman.get(), p).await);
    }
    snaps
}

fn api_err(e: AppError) -> (StatusCode, Json<serde_json::Value>) {
    let status = match &e {
        AppError::NotFound(_) => StatusCode::NOT_FOUND,
        AppError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({ "error": e.to_string() })))
}
