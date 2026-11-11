//! Loaded spools + load spool flow (task 4.1).
//!
//! - `GET  /api/spools`                           — every spool + where it is loaded
//! - `GET  /api/spools/loaded`                    — loaded slots of every printer
//! - `GET  /api/printers/{id}/spools`             — that printer's slots
//! - `GET  /api/printers/{id}/spools/candidates?tool=` — spools to load, guarded, recent first
//! - `POST /api/printers/{id}/spools/check`       — `{tool, spool_id}` → guard warnings
//! - `POST /api/printers/{id}/spools/load`        — `{tool, spool_id, override_reason?}`
//! - `POST /api/printers/{id}/spools/unload`      — `{tool}`
//! - `PATCH /api/spools/{id}`                     — `{location?, lot_nr?, comment?, archived?, price?}`
//!
//! Blocking guard warnings need `override_reason`; the override goes to `audit_log`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use foreman_core::{
    error::AppError,
    filament::{self, GuardWarning, LoadedSlot, SpoolFacts},
    repo,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use crate::{
    auth::AuthCtx,
    spools::{self, Spoolman},
    state::AppState,
};

pub(crate) type ApiErr = (StatusCode, Json<serde_json::Value>);

pub(crate) fn err(e: AppError) -> ApiErr {
    let status = match &e {
        AppError::NotFound(_) => StatusCode::NOT_FOUND,
        AppError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        AppError::NotImplemented(_) => StatusCode::CONFLICT,
        AppError::Driver(_) => StatusCode::BAD_GATEWAY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({ "error": e.to_string() })))
}

pub(crate) fn sm(state: &AppState) -> Result<&Spoolman, ApiErr> {
    state.spoolman.get().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "Spoolman is not running" })),
        )
    })
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// Where a spool is loaded.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct LoadedAt {
    #[ts(type = "number")]
    pub printer_id: i64,
    pub printer_name: String,
    pub tool: u32,
    pub label: String,
}

impl From<&LoadedSlot> for LoadedAt {
    fn from(s: &LoadedSlot) -> Self {
        Self {
            printer_id: s.printer_id,
            printer_name: s.printer_name.clone(),
            tool: s.tool,
            label: s.label.clone(),
        }
    }
}

/// One row of the Spools list.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct SpoolRow {
    pub spool: SpoolFacts,
    pub loaded: Vec<LoadedAt>,
}

/// A spool offered in the load flow, already guarded for the target slot.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct LoadCandidate {
    pub spool: SpoolFacts,
    pub loaded: Vec<LoadedAt>,
    pub warnings: Vec<GuardWarning>,
    pub blocked: bool,
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct SpoolCheck {
    pub tool: u32,
    #[ts(type = "number")]
    pub spool_id: i64,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct GuardResult {
    pub warnings: Vec<GuardWarning>,
    pub blocked: bool,
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct LoadSpool {
    pub tool: u32,
    #[ts(type = "number")]
    pub spool_id: i64,
    /// Required when the guard blocks; written to the audit log.
    #[serde(default)]
    pub override_reason: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct UnloadSpool {
    pub tool: u32,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct LoadResult {
    pub slot: LoadedSlot,
    pub warnings: Vec<GuardWarning>,
    pub overridden: bool,
    /// Slots the spool was unloaded from.
    pub unloaded_from: Vec<LoadedAt>,
}

/// Spool fields the inspector can edit (the filament form is separate).
#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct EditSpool {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lot_nr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct CandidateQuery {
    #[serde(default)]
    pub tool: u32,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub archived: bool,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn printer(state: &AppState, id: i64) -> Result<foreman_core::domain::Printer, ApiErr> {
    repo::printer::get(&state.foreman_db.pool, id)
        .await
        .map_err(err)
}

/// `GET /api/spools`.
pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<SpoolRow>>, ApiErr> {
    let sm = sm(&state)?;
    let spools = spools::all(sm, q.archived).await.map_err(err)?;
    let slots = spools::all_slots(&state.foreman_db.pool, &state.driver_manager, Some(sm))
        .await
        .map_err(err)?;
    Ok(Json(
        spools
            .into_iter()
            .map(|s| SpoolRow {
                loaded: slots
                    .iter()
                    .filter(|l| l.spool_id == Some(s.id))
                    .map(LoadedAt::from)
                    .collect(),
                spool: s,
            })
            .collect(),
    ))
}

/// `GET /api/spools/loaded`.
pub async fn loaded(State(state): State<AppState>) -> Result<Json<Vec<LoadedSlot>>, ApiErr> {
    let sm = state.spoolman.get();
    spools::all_slots(&state.foreman_db.pool, &state.driver_manager, sm)
        .await
        .map(Json)
        .map_err(err)
}

/// `GET /api/printers/{id}/spools`.
pub async fn printer_slots(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<LoadedSlot>>, ApiErr> {
    let p = printer(&state, id).await?;
    Ok(Json(
        spools::slots_for(&state.driver_manager, state.spoolman.get(), &p).await,
    ))
}

/// `GET /api/printers/{id}/spools/candidates?tool=0`.
pub async fn candidates(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<CandidateQuery>,
) -> Result<Json<Vec<LoadCandidate>>, ApiErr> {
    let sm = sm(&state)?;
    let p = printer(&state, id).await?;
    let pool = &state.foreman_db.pool;
    let drivers = &state.driver_manager;
    let slot = spools::slots_for(drivers, Some(sm), &p)
        .await
        .into_iter()
        .find(|s| s.tool == q.tool)
        .ok_or_else(|| {
            err(AppError::NotFound(format!(
                "{} has no tool {}",
                p.name, q.tool
            )))
        })?;
    let all_slots = spools::all_slots(pool, drivers, Some(sm))
        .await
        .map_err(err)?;
    let caps = drivers.get_capabilities(id).await;
    let need = spools::job_need(pool, drivers, state.library.as_ref(), id, q.tool).await;
    let mut out = Vec::new();
    for s in spools::all(sm, false).await.map_err(err)? {
        if slot.spool_id == Some(s.id) {
            continue;
        }
        let elsewhere: Vec<LoadedSlot> = all_slots
            .iter()
            .filter(|l| l.spool_id == Some(s.id))
            .cloned()
            .collect();
        let warnings = filament::guard(&filament::GuardInput {
            printer: &p,
            caps: caps.as_ref(),
            spool: &s,
            tray: slot.tray.as_ref(),
            need: need.clone(),
            loaded_elsewhere: &elsewhere,
        });
        out.push(LoadCandidate {
            blocked: filament::is_blocked(&warnings),
            loaded: elsewhere.iter().map(LoadedAt::from).collect(),
            spool: s,
            warnings,
        });
    }
    Ok(Json(out))
}

/// `POST /api/printers/{id}/spools/check`.
pub async fn check(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<SpoolCheck>,
) -> Result<Json<GuardResult>, ApiErr> {
    let sm = sm(&state)?;
    let p = printer(&state, id).await?;
    let plan = spools::plan(
        &state.foreman_db.pool,
        &state.driver_manager,
        sm,
        state.library.as_ref(),
        &p,
        req.tool,
        req.spool_id,
    )
    .await
    .map_err(err)?;
    Ok(Json(GuardResult {
        blocked: filament::is_blocked(&plan.warnings),
        warnings: plan.warnings,
    }))
}

/// `POST /api/printers/{id}/spools/load`.
pub async fn load(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<i64>,
    Json(req): Json<LoadSpool>,
) -> Result<Json<LoadResult>, ApiErr> {
    let sm = sm(&state)?;
    let p = printer(&state, id).await?;
    let pool = &state.foreman_db.pool;
    let drivers = &state.driver_manager;
    let plan = spools::plan(
        pool,
        drivers,
        sm,
        state.library.as_ref(),
        &p,
        req.tool,
        req.spool_id,
    )
    .await
    .map_err(err)?;
    if !plan.slot.writable {
        let reason = plan
            .slot
            .read_only_reason
            .clone()
            .unwrap_or_else(|| "This slot is read-only.".into());
        return Err((
            StatusCode::CONFLICT,
            Json(json!({ "error": reason, "read_only": true })),
        ));
    }
    let blocked = filament::is_blocked(&plan.warnings);
    let reason = req
        .override_reason
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    if blocked && reason.is_none() {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({
                "error": "The filament guard blocks this spool. Give a reason to load it anyway.",
                "warnings": plan.warnings,
                "blocked": true,
            })),
        ));
    }

    // One spool, one slot: unload it wherever else Foreman can.
    let mut unloaded_from = Vec::new();
    for other in plan.elsewhere.iter().filter(|s| s.writable) {
        match spools::set_slot(drivers, other.printer_id, other.tool, None).await {
            Ok(()) => {
                audit(
                    &state,
                    other.printer_id,
                    &ctx.username,
                    &format!(
                        "spool.unload {} {} (moved to {} {})",
                        plan.spool.label(),
                        other.label,
                        p.name,
                        plan.slot.label
                    ),
                )
                .await;
                unloaded_from.push(LoadedAt::from(other));
            }
            Err(e) => tracing::warn!(
                printer_id = other.printer_id,
                "unload spool #{} elsewhere: {e}",
                req.spool_id
            ),
        }
    }

    spools::set_slot(drivers, id, req.tool, Some(req.spool_id))
        .await
        .map_err(err)?;

    let line = format!("spool.load {} → {}", plan.spool.label(), plan.slot.label);
    if blocked {
        let codes: Vec<String> = plan
            .warnings
            .iter()
            .filter(|w| w.severity == filament::GuardSeverity::Block)
            .map(|w| w.reason.clone())
            .collect();
        audit(
            &state,
            id,
            &ctx.username,
            &format!(
                "{line} OVERRIDE: {} | guard: {}",
                reason.unwrap_or_default(),
                codes.join(" / ")
            ),
        )
        .await;
    } else {
        audit(&state, id, &ctx.username, &line).await;
    }

    let slot = spools::slots_for(drivers, Some(sm), &p)
        .await
        .into_iter()
        .find(|s| s.tool == req.tool)
        .unwrap_or(plan.slot);
    Ok(Json(LoadResult {
        slot,
        warnings: plan.warnings,
        overridden: blocked,
        unloaded_from,
    }))
}

/// `POST /api/printers/{id}/spools/unload`.
pub async fn unload(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<i64>,
    Json(req): Json<UnloadSpool>,
) -> Result<Json<LoadedSlot>, ApiErr> {
    let p = printer(&state, id).await?;
    let drivers = &state.driver_manager;
    let sm = state.spoolman.get();
    let slot = spools::slots_for(drivers, sm, &p)
        .await
        .into_iter()
        .find(|s| s.tool == req.tool)
        .ok_or_else(|| {
            err(AppError::NotFound(format!(
                "{} has no tool {}",
                p.name, req.tool
            )))
        })?;
    if !slot.writable {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({ "error": slot.read_only_reason, "read_only": true })),
        ));
    }
    spools::set_slot(drivers, id, req.tool, None)
        .await
        .map_err(err)?;
    let what = slot
        .spool
        .as_ref()
        .map(|s| s.label())
        .or(slot.spool_id.map(|i| format!("#{i}")))
        .unwrap_or_else(|| "nothing".into());
    audit(
        &state,
        id,
        &ctx.username,
        &format!("spool.unload {what} {}", slot.label),
    )
    .await;
    let slot = spools::slots_for(drivers, sm, &p)
        .await
        .into_iter()
        .find(|s| s.tool == req.tool)
        .unwrap_or(slot);
    Ok(Json(slot))
}

/// `PATCH /api/spools/{id}`.
pub async fn edit(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<EditSpool>,
) -> Result<Json<SpoolFacts>, ApiErr> {
    let sm = sm(&state)?;
    if req.price.is_some_and(|p| p < 0.0) {
        return Err(err(AppError::InvalidInput(
            "price can't be negative".into(),
        )));
    }
    let upd: spoolman_rs::models::UpdateSpoolRequest = serde_json::from_value(json!(req))
        .map_err(|e| err(AppError::InvalidInput(e.to_string())))?;
    let s = spoolman_rs::db::spool::update(&sm.db.pool, id, &upd)
        .await
        .map_err(|e| match e {
            spoolman_rs::error::SpoolmanError::NotFound(_) => {
                err(AppError::NotFound(format!("spool #{id}")))
            }
            e => err(AppError::Internal(format!("spoolman: {e}"))),
        })?;
    spoolman_rs::ws::broadcast(&sm.event_tx, "updated", "spool", s.id, &s);
    Ok(Json(spools::facts(&s)))
}

async fn audit(state: &AppState, printer_id: i64, user: &str, line: &str) {
    if let Err(e) =
        repo::audit_log::insert(&state.foreman_db.pool, Some(printer_id), Some(user), line).await
    {
        tracing::warn!(printer_id, "audit_log insert failed: {e}");
    }
}
