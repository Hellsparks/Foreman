//! REST endpoints for printer commands, presets, and macro favourites.
//!
//! Commands:
//! - `POST /api/printers/:id/commands/:kind`     — send a command
//! - `POST /api/printers/:id/confirm-cancel`      — get a cancel confirm token
//! - `GET  /api/printers/:id/capabilities`        — get printer capabilities
//!
//! Temp presets:
//! - `GET    /api/printers/:id/presets`            — list temp presets
//! - `POST   /api/printers/:id/presets`            — create a temp preset
//! - `DELETE /api/printers/:id/presets/:pid`        — delete a temp preset
//!
//! Macro favourites:
//! - `GET    /api/printers/:id/macro-favourites`   — list favourites
//! - `POST   /api/printers/:id/macro-favourites`   — add a favourite
//! - `DELETE /api/printers/:id/macro-favourites/:fid` — remove a favourite

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use foreman_core::driver::PrinterCommand;
use serde::Deserialize;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Command dispatch
// ---------------------------------------------------------------------------

/// Request body for command dispatch.
/// Each command kind has its own optional fields; unused fields are ignored.
#[derive(Debug, Deserialize)]
pub struct CommandBody {
    // SetNozzleTemp / SetBedTemp
    pub target: Option<f64>,
    // Home
    pub axes: Option<Vec<String>>,
    // Jog
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub z: Option<f64>,
    pub speed: Option<f64>,
    // BabyStep
    pub offset: Option<f64>,
    // Extrude
    pub length: Option<f64>,
    // SetLight
    pub brightness: Option<f64>,
    // Macro
    pub name: Option<String>,
    // Gcode
    pub command: Option<String>,
    // Cancel confirm token
    pub confirm_token: Option<String>,
}

/// `POST /api/printers/:id/commands/:kind`
pub async fn send_command(
    State(state): State<AppState>,
    Path((id, kind)): Path<(i64, String)>,
    Json(body): Json<CommandBody>,
) -> impl IntoResponse {
    let cmd = match kind.as_str() {
        "pause" => PrinterCommand::Pause,
        "resume" => PrinterCommand::Resume,
        "cancel" => PrinterCommand::Cancel,
        "emergency_stop" => PrinterCommand::EmergencyStop,
        "motors_off" => PrinterCommand::MotorsOff,
        "set_nozzle_temp" => PrinterCommand::SetNozzleTemp {
            target: body.target.unwrap_or(0.0),
        },
        "set_bed_temp" => PrinterCommand::SetBedTemp {
            target: body.target.unwrap_or(0.0),
        },
        "home" => PrinterCommand::Home {
            axes: body.axes.unwrap_or_default(),
        },
        "jog" => PrinterCommand::Jog {
            x: body.x,
            y: body.y,
            z: body.z,
            speed: body.speed,
        },
        "baby_step" => PrinterCommand::BabyStep {
            offset: body.offset.unwrap_or(0.0),
        },
        "extrude" => PrinterCommand::Extrude {
            length: body.length.unwrap_or(10.0),
            speed: body.speed.unwrap_or(300.0),
        },
        "set_light" => PrinterCommand::SetLight {
            brightness: body.brightness.unwrap_or(1.0),
        },
        "macro" => match body.name {
            Some(n) => PrinterCommand::Macro { name: n },
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "macro command requires 'name'" })),
                )
                    .into_response()
            }
        },
        "gcode" => match body.command {
            Some(c) => PrinterCommand::Gcode { command: c },
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "gcode command requires 'command'" })),
                )
                    .into_response()
            }
        },
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("unknown command kind: {kind}") })),
            )
                .into_response()
        }
    };

    match state
        .driver_manager
        .send_command(id, cmd, body.confirm_token.as_deref())
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let status = match &e {
                foreman_core::error::AppError::NotFound(_) => StatusCode::NOT_FOUND,
                foreman_core::error::AppError::InvalidInput(_) => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, Json(serde_json::json!({ "error": e.to_string() }))).into_response()
        }
    }
}

/// `POST /api/printers/:id/confirm-cancel`
pub async fn confirm_cancel(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let token = state.driver_manager.request_cancel_token(id).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({ "confirm_token": token })),
    )
        .into_response()
}

/// `GET /api/printers/:id/capabilities`
pub async fn get_capabilities(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match state.driver_manager.get_capabilities(id).await {
        Some(caps) => (
            StatusCode::OK,
            Json(serde_json::to_value(caps).unwrap_or_default()),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no capabilities for this printer" })),
        )
            .into_response(),
    }
}

/// `GET /api/printers/:id/status`
pub async fn get_status(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    match state.driver_manager.get_status(id).await {
        Some(status) => (
            StatusCode::OK,
            Json(serde_json::to_value(status).unwrap_or_default()),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no status for this printer" })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Temp presets
// ---------------------------------------------------------------------------

/// `GET /api/printers/:id/presets`
pub async fn list_presets(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let pool = &state.foreman_db.pool;
    match foreman_core::repo::temp_preset::list(pool, id).await {
        Ok(presets) => (
            StatusCode::OK,
            Json(serde_json::to_value(presets).unwrap_or_default()),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `POST /api/printers/:id/presets`
pub async fn create_preset(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<foreman_core::repo::temp_preset::CreateTempPreset>,
) -> impl IntoResponse {
    let pool = &state.foreman_db.pool;
    match foreman_core::repo::temp_preset::create(pool, id, req).await {
        Ok(preset) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(preset).unwrap_or_default()),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `DELETE /api/printers/:id/presets/:pid`
pub async fn delete_preset(
    State(state): State<AppState>,
    Path((_id, pid)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let pool = &state.foreman_db.pool;
    match foreman_core::repo::temp_preset::delete(pool, pid).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Macro favourites
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AddFavouriteBody {
    pub macro_name: String,
}

/// `GET /api/printers/:id/macro-favourites`
pub async fn list_favourites(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let pool = &state.foreman_db.pool;
    match foreman_core::repo::macro_favourite::list(pool, id).await {
        Ok(favs) => (
            StatusCode::OK,
            Json(serde_json::to_value(favs).unwrap_or_default()),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `POST /api/printers/:id/macro-favourites`
pub async fn add_favourite(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<AddFavouriteBody>,
) -> impl IntoResponse {
    let pool = &state.foreman_db.pool;
    match foreman_core::repo::macro_favourite::add(pool, id, &body.macro_name).await {
        Ok(fav) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(fav).unwrap_or_default()),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `DELETE /api/printers/:id/macro-favourites/:fid`
pub async fn remove_favourite(
    State(state): State<AppState>,
    Path((_id, fid)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let pool = &state.foreman_db.pool;
    match foreman_core::repo::macro_favourite::remove(pool, fid).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
