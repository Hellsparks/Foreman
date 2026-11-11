//! Calibration (task 5.4, W10).
//!
//! Skew (per printer, Klipper only):
//! - `GET  /api/calibration/skew` — printers with support, state, history
//! - `POST /api/calibration/skew/preview` — `{planes}` → angles + exact commands (sends nothing)
//! - `POST /api/calibration/skew/{printer_id}/measurements` — `{planes}` → stored record
//! - `POST /api/calibration/skew/{printer_id}/apply` — `{record_id, confirm_restart}`:
//!   SET_SKEW, SKEW_PROFILE SAVE=foreman, SAVE_CONFIG. Refused unless Klipper + idle.
//! - `POST /api/calibration/skew/{printer_id}/clear` — SET_SKEW CLEAR=1 (before the test print)
//! - `GET  /api/calibration/skew/{printer_id}/macros` — planned PRINT_START / PRINT_END /
//!   `[skew_correction]` change with a diff (writes nothing)
//! - `POST /api/calibration/skew/{printer_id}/macros` — `{approved:[{path,sha}], restart}`:
//!   re-plans, writes only if every file is unchanged since the approved diff, `.bak` first.
//!
//! Shrinkage (per filament):
//! - `GET  /api/calibration/shrinkage` — filaments with current values + history
//! - `POST /api/calibration/shrinkage/preview` — `ShrinkageInput` → result
//! - `POST /api/calibration/shrinkage/{filament_id}` — `{input, printer_id}` → saved

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use foreman_core::{
    calibration::{self as cal, CfgFile, PlaneMeasure, ShrinkageInput, ShrinkageResult},
    domain::SftpEntryKind,
    driver::PrinterCommand,
    error::AppError,
    remote_fs,
    repo::{self, calibration::SkewRecord},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use super::ssh::{audit, firmware_restart_allowed, open_fs, ApiError, ApiResult};
use crate::{
    auth::AuthCtx,
    calibration::{
        self as svc, SaveShrinkage, ShrinkFilament, ShrinkSaved, SkewApplied, SkewPreview,
        SkewPrinter,
    },
    state::AppState,
};

fn refused(e: AppError) -> ApiError {
    let printing = matches!(&e, AppError::InvalidInput(m) if m.contains("printing or paused"));
    let mut a = ApiError::from(e);
    if printing {
        a.status = StatusCode::CONFLICT;
        a.code = Some(svc::CODE_PRINTING);
    }
    a
}

// ---------------------------------------------------------------------------
// Skew
// ---------------------------------------------------------------------------

pub async fn skew_printers(State(s): State<AppState>) -> ApiResult<Json<Vec<SkewPrinter>>> {
    Ok(Json(
        svc::skew_printers(&s.foreman_db.pool, &s.driver_manager).await?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct PlanesBody {
    planes: Vec<PlaneMeasure>,
}

pub async fn skew_preview(Json(b): Json<PlanesBody>) -> ApiResult<Json<SkewPreview>> {
    Ok(Json(svc::preview(&b.planes)?))
}

pub async fn skew_measure(
    State(s): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<i64>,
    Json(b): Json<PlanesBody>,
) -> ApiResult<(StatusCode, Json<SkewRecord>)> {
    let pool = &s.foreman_db.pool;
    let printer = repo::printer::get(pool, id).await?;
    if let Some(why) = svc::skew_unsupported(&printer) {
        return Err(AppError::InvalidInput(why).into());
    }
    let planes = cal::compute_skew(&b.planes)?;
    let rec = repo::calibration::insert_skew(pool, id, &planes, &ctx.username).await?;
    Ok((StatusCode::CREATED, Json(rec)))
}

#[derive(Debug, Deserialize)]
pub struct ApplyBody {
    record_id: i64,
    #[serde(default)]
    confirm_restart: bool,
}

pub async fn skew_apply(
    State(s): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<i64>,
    Json(b): Json<ApplyBody>,
) -> ApiResult<Json<SkewApplied>> {
    svc::apply(
        &s.foreman_db.pool,
        &s.driver_manager,
        id,
        b.record_id,
        b.confirm_restart,
        &ctx.username,
    )
    .await
    .map(Json)
    .map_err(refused)
}

pub async fn skew_clear(
    State(s): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    let sent = svc::clear(&s.foreman_db.pool, &s.driver_manager, id, &ctx.username)
        .await
        .map_err(refused)?;
    Ok(Json(serde_json::json!({ "sent": sent })))
}

// ---------------------------------------------------------------------------
// PRINT_START / PRINT_END / [skew_correction] patch
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct MacroFileChange {
    pub path: String,
    /// sha256 of the file as read; apply refuses if it changed since.
    pub sha: String,
    /// Lines prefixed "  ", "- ", "+ "; "…" between hunks.
    pub diff: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct MacroPatch {
    pub files: Vec<MacroFileChange>,
    pub notes: Vec<String>,
    /// `[skew_correction]` is added: Klipper must restart before SET_SKEW works.
    pub adds_section: bool,
    /// Files read (printer.cfg + includes).
    pub read: Vec<String>,
}

fn sha(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn glob_match(pat: &str, name: &str) -> bool {
    fn m(p: &[u8], n: &[u8]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some(b'*'), _) => m(&p[1..], n) || (!n.is_empty() && m(p, &n[1..])),
            (Some(b'?'), Some(_)) => m(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a == b => m(&p[1..], &n[1..]),
            _ => false,
        }
    }
    m(pat.as_bytes(), name.as_bytes())
}

/// printer.cfg plus its includes (glob in the file name only), up to 3 levels deep.
async fn read_config(state: &AppState, printer_id: i64) -> ApiResult<Vec<CfgFile>> {
    let s = open_fs(state, printer_id).await?;
    let root = format!("{}/printer.cfg", s.roots.config);
    let mut out: Vec<CfgFile> = vec![];
    let mut queue: Vec<(String, u8)> = vec![(root, 0)];
    while let Some((path, depth)) = queue.pop() {
        if out.iter().any(|f| f.path == path) || out.len() >= 64 {
            continue;
        }
        let t = remote_fs::read_text(&s.fs, &s.roots, &path).await?;
        if t.truncated {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("{path} is too large to patch safely"),
            ));
        }
        let dir = path
            .rsplit_once('/')
            .map(|(d, _)| d.to_owned())
            .unwrap_or_default();
        if depth < 3 {
            for inc in cal::includes(&t.content) {
                let full = if inc.starts_with('/') {
                    inc.clone()
                } else {
                    format!("{dir}/{inc}")
                };
                let (idir, ifile) = full.rsplit_once('/').unwrap_or(("", &full));
                if ifile.contains(['*', '?']) || idir.contains(['*', '?']) {
                    if idir.contains(['*', '?']) {
                        continue; // directory globs: not followed
                    }
                    let mut names: Vec<String> = remote_fs::list(&s.fs, idir)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|e| !matches!(e.kind, SftpEntryKind::Directory))
                        .filter(|e| glob_match(ifile, &e.name))
                        .map(|e| format!("{idir}/{}", e.name))
                        .collect();
                    names.sort();
                    for n in names.into_iter().rev() {
                        queue.push((n, depth + 1));
                    }
                } else {
                    queue.push((full, depth + 1));
                }
            }
        }
        out.push(CfgFile {
            path: t.path,
            content: t.content,
        });
    }
    Ok(out)
}

fn macro_patch(files: &[CfgFile]) -> ApiResult<(MacroPatch, cal::CfgPatch)> {
    let plan = cal::plan_skew_cfg_patch(files)?;
    let changes = plan
        .changed
        .iter()
        .map(|(path, new)| {
            let old = files
                .iter()
                .find(|f| &f.path == path)
                .map(|f| f.content.as_str())
                .unwrap_or("");
            MacroFileChange {
                path: path.clone(),
                sha: sha(old),
                diff: cal::line_diff(old, new, 3),
            }
        })
        .collect();
    Ok((
        MacroPatch {
            files: changes,
            notes: plan.notes.clone(),
            adds_section: plan.adds_section,
            read: files.iter().map(|f| f.path.clone()).collect(),
        },
        plan,
    ))
}

async fn klipper(state: &AppState, id: i64) -> ApiResult<()> {
    let p = repo::printer::get(&state.foreman_db.pool, id).await?;
    match svc::skew_unsupported(&p) {
        Some(why) => Err(AppError::InvalidInput(why).into()),
        None => Ok(()),
    }
}

/// Read-only: plan the change and show the diff.
pub async fn macros_preview(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<MacroPatch>> {
    klipper(&s, id).await?;
    let files = read_config(&s, id).await?;
    Ok(Json(macro_patch(&files)?.0))
}

#[derive(Debug, Deserialize)]
pub struct Approved {
    path: String,
    sha: String,
}

#[derive(Debug, Deserialize)]
pub struct MacrosApplyBody {
    approved: Vec<Approved>,
    #[serde(default)]
    restart: bool,
}

pub async fn macros_apply(
    State(s): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<i64>,
    Json(b): Json<MacrosApplyBody>,
) -> ApiResult<Json<serde_json::Value>> {
    klipper(&s, id).await?;
    if b.restart {
        firmware_restart_allowed(s.driver_manager.get_status(id).await.as_ref()).map_err(|e| {
            let mut a = ApiError::from(e);
            a.status = StatusCode::CONFLICT;
            a.code = Some(svc::CODE_PRINTING);
            a
        })?;
    }
    let files = read_config(&s, id).await?;
    let (view, plan) = macro_patch(&files)?;
    // The approved set must be exactly what we'd write now, file contents unchanged.
    let same = view.files.len() == b.approved.len()
        && view.files.iter().all(|f| {
            b.approved
                .iter()
                .any(|a| a.path == f.path && a.sha == f.sha)
        });
    if !same {
        return Err(ApiError {
            status: StatusCode::CONFLICT,
            message: "the config changed since you reviewed the diff — review it again".into(),
            code: Some("stale"),
        });
    }
    if plan.changed.is_empty() {
        return Ok(Json(
            serde_json::json!({ "written": [], "restarted": false }),
        ));
    }
    let fs = open_fs(&s, id).await?;
    let mut written = vec![];
    for (path, content) in &plan.changed {
        if !remote_fs::is_klipper_cfg(&fs.roots, path) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("{path} is not a .cfg under printer_data/config"),
            ));
        }
        remote_fs::write_with_backup(&fs.fs, &fs.roots, path, content.as_bytes()).await?;
        audit(
            &s,
            id,
            &ctx.username,
            &format!("calibration: patch {path} (.bak kept)"),
        )
        .await;
        written.push(path.clone());
    }
    let mut restarted = false;
    if b.restart {
        firmware_restart_allowed(s.driver_manager.get_status(id).await.as_ref())?;
        s.driver_manager
            .send_command_as(
                id,
                PrinterCommand::Gcode {
                    command: "FIRMWARE_RESTART".into(),
                },
                None,
                Some(&ctx.username),
            )
            .await?;
        restarted = true;
    }
    Ok(Json(
        serde_json::json!({ "written": written, "restarted": restarted }),
    ))
}

// ---------------------------------------------------------------------------
// Shrinkage
// ---------------------------------------------------------------------------

fn sm(s: &AppState) -> ApiResult<&crate::spools::Spoolman> {
    s.spoolman
        .get()
        .ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "Spoolman is not running"))
}

pub async fn shrink_filaments(State(s): State<AppState>) -> ApiResult<Json<Vec<ShrinkFilament>>> {
    Ok(Json(
        svc::shrink_filaments(&s.foreman_db.pool, sm(&s)?).await?,
    ))
}

pub async fn shrink_preview(Json(b): Json<ShrinkageInput>) -> ApiResult<Json<ShrinkageResult>> {
    Ok(Json(cal::compute_shrinkage(&b)?))
}

pub async fn shrink_save(
    State(s): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<i64>,
    Json(b): Json<SaveShrinkage>,
) -> ApiResult<Json<ShrinkSaved>> {
    Ok(Json(
        svc::save_shrinkage(&s.foreman_db.pool, sm(&s)?, id, &b, &ctx.username).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn include_globs() {
        assert!(glob_match("*.cfg", "macros.cfg"));
        assert!(glob_match("macro?.cfg", "macro1.cfg"));
        assert!(!glob_match("*.cfg", "macros.cfg.bak"));
        assert!(glob_match("mainsail.cfg", "mainsail.cfg"));
    }
}
