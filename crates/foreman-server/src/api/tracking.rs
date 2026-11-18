//! Settings → Filament: tracking mode and the Spoolman API switch (doc 07 §A).
//!
//! - `GET /api/settings/filament` — policy, API switch, whether 7912 listens now
//! - `GET /api/settings/filament/preview?mode=off` — what switching would do,
//!   with the confirm text
//! - `PUT /api/settings/filament` — `{mode?, serve_api?}`; a mode change needs
//!   `confirm: true`

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use foreman_core::{
    error::AppError,
    repo,
    tracking::{FilamentMode, FilamentPolicy},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use crate::{
    state::AppState,
    tracking::{self, SETTING_MODE, SETTING_SERVE_API},
};

type ApiErr = (StatusCode, Json<serde_json::Value>);

fn err(e: AppError) -> ApiErr {
    let status = match e {
        AppError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({ "error": e.to_string() })))
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct FilamentSettings {
    pub policy: FilamentPolicy,
    /// Serve the Spoolman API on its port (explicit setting, else the mode's default).
    pub serve_api: bool,
    /// The Spoolman API is listening right now.
    pub api_running: bool,
    #[ts(type = "number | null")]
    pub api_port: Option<u16>,
}

async fn settings(s: &AppState) -> Result<FilamentSettings, AppError> {
    let mode = s.tracking.mode();
    Ok(FilamentSettings {
        policy: mode.policy(),
        serve_api: tracking::serve_api(&s.foreman_db.pool, mode).await?,
        api_running: s.tracking.api_running().await,
        api_port: s.tracking.store().map(|sm| sm.config.port),
    })
}

/// `GET /api/settings/filament`.
pub async fn get_settings(State(s): State<AppState>) -> Result<Json<FilamentSettings>, ApiErr> {
    settings(&s).await.map(Json).map_err(err)
}

#[derive(Debug, Deserialize)]
pub struct PreviewQuery {
    mode: FilamentMode,
}

/// What switching to a mode would do.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct SwitchPreview {
    pub from: FilamentMode,
    pub to: FilamentMode,
    /// Open reservations released (queued jobs and projects holding grams).
    pub reservations: usize,
    pub reserved_grams: f64,
    /// Project slots that lose their spool, and in how many projects.
    #[ts(type = "number")]
    pub project_slots: i64,
    #[ts(type = "number")]
    pub projects: i64,
    /// The confirm dialog's text, one paragraph per entry.
    pub confirm: Vec<String>,
}

fn plural(n: impl Into<i64>, one: &str, many: &str) -> String {
    let n = n.into();
    format!("{n} {}", if n == 1 { one } else { many })
}

async fn preview(s: &AppState, to: FilamentMode) -> Result<SwitchPreview, AppError> {
    let from = s.tracking.mode();
    let pool = &s.foreman_db.pool;
    let off = from.tracks_spools() && !to.tracks_spools();
    let on = !from.tracks_spools() && to.tracks_spools();
    let (mut reservations, mut reserved_grams, mut project_slots, mut projects) = (0, 0.0, 0, 0);
    let mut confirm = Vec::new();
    if off {
        if let Some(sm) = s.tracking.spools() {
            let all = crate::spools::all(sm, false).await?;
            let res = crate::inventory::reservations(
                pool,
                &s.driver_manager,
                sm,
                s.library.as_ref(),
                &all,
            )
            .await?;
            reservations = res.len();
            reserved_grams = (res.iter().map(|r| r.grams).sum::<f64>() * 10.0).round() / 10.0;
        }
        (project_slots, projects) = repo::project::count_slot_spools(pool).await?;
        confirm.push(
            "Turn filament tracking off? The Filament section, spool pickers, the load-spool \
             check and the spool verdicts in Send go away. Jobs still record the grams each \
             file uses; nothing is subtracted from any spool."
                .into(),
        );
        confirm.push(if reservations == 0 {
            "No open reservations to release.".into()
        } else {
            format!(
                "{} ({reserved_grams} g) will be released.",
                plural(reservations as i64, "open reservation", "open reservations")
            )
        });
        confirm.push(if project_slots == 0 {
            "No project has a spool assigned.".into()
        } else {
            format!(
                "{} in {} will lose their spool assignment. Turning tracking back on does not \
                 restore them.",
                plural(project_slots, "slot", "slots"),
                plural(projects, "project", "projects")
            )
        });
        confirm.push("Spools, filaments and job history are kept, not deleted.".into());
        if tracking::serve_api_setting(pool).await?.is_none() && s.tracking.api_running().await {
            confirm.push(
                "The Spoolman API keeps answering on its port, because Moonraker or slicers \
                 may point at it. Switch it off separately if nothing uses it."
                    .into(),
            );
        }
    } else if on {
        confirm.push(format!(
            "Turn filament tracking on ({})? Spools, filaments and their weights come back \
             exactly as they were.",
            if to == FilamentMode::External {
                "External"
            } else {
                "Full"
            }
        ));
        confirm.push(
            "Project spool assignments are not restored: pick a spool for each slot again. \
             Jobs printed while tracking was off stay without a spool, and nothing is \
             deducted for them afterwards."
                .into(),
        );
    } else if from != to {
        confirm.push(format!(
            "Switch filament tracking to {}? Spools and assignments are unchanged.",
            if to == FilamentMode::External {
                "External"
            } else {
                "Full"
            }
        ));
    }
    Ok(SwitchPreview {
        from,
        to,
        reservations,
        reserved_grams,
        project_slots,
        projects,
        confirm,
    })
}

/// `GET /api/settings/filament/preview?mode=`.
pub async fn get_preview(
    State(s): State<AppState>,
    Query(q): Query<PreviewQuery>,
) -> Result<Json<SwitchPreview>, ApiErr> {
    preview(&s, q.mode).await.map(Json).map_err(err)
}

#[derive(Debug, Deserialize)]
pub struct PutSettings {
    mode: Option<FilamentMode>,
    serve_api: Option<bool>,
    /// Required for a mode change: the user saw the preview's confirm text.
    #[serde(default)]
    confirm: bool,
}

/// What a switch did, with the settings after it.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct SwitchResult {
    pub settings: FilamentSettings,
    pub released_reservations: usize,
    #[ts(type = "number")]
    pub cleared_project_slots: u64,
}

/// `PUT /api/settings/filament`.
pub async fn put_settings(
    State(s): State<AppState>,
    Json(b): Json<PutSettings>,
) -> Result<Json<SwitchResult>, ApiErr> {
    let pool = &s.foreman_db.pool;
    let mut released = 0;
    let mut cleared = 0;
    if let Some(to) = b.mode.filter(|m| *m != s.tracking.mode()) {
        if !b.confirm {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "changing filament tracking needs confirm: true (see /preview)"
                })),
            ));
        }
        let from = s.tracking.mode();
        // Turning tracking off must not silently drop the API that Moonraker may
        // use: pin the switch to what it is now unless it was set explicitly.
        if repo::setting::get(pool, SETTING_SERVE_API)
            .await
            .map_err(err)?
            .is_none()
        {
            let now = tracking::serve_api(pool, from).await.map_err(err)?;
            repo::setting::set(pool, SETTING_SERVE_API, if now { "true" } else { "false" })
                .await
                .map_err(err)?;
        }
        if from.tracks_spools() && !to.tracks_spools() {
            released = preview(&s, to).await.map_err(err)?.reservations;
            cleared = repo::project::clear_slot_spools(pool).await.map_err(err)?;
        }
        repo::setting::set(pool, SETTING_MODE, to.as_str())
            .await
            .map_err(err)?;
        s.tracking.set_mode(to);
        tracing::info!(
            from = from.as_str(),
            to = to.as_str(),
            released,
            cleared,
            "filament tracking mode changed"
        );
        // Queues and projects re-read their reservations and verdicts.
        let _ = s
            .bus_tx
            .send(foreman_core::events::ServerEvent::QueueChanged { printer_id: None });
    }
    if let Some(on) = b.serve_api {
        repo::setting::set(pool, SETTING_SERVE_API, if on { "true" } else { "false" })
            .await
            .map_err(err)?;
        s.tracking.serve_api(on).await.map_err(err)?;
    }
    Ok(Json(SwitchResult {
        settings: settings(&s).await.map_err(err)?,
        released_reservations: released,
        cleared_project_slots: cleared,
    }))
}
