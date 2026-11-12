//! Exports + swatches (task 5.5, W11).
//!
//! - `GET  /api/exports` — every filament with what the Orca / HueForge exports do with it
//! - `POST /api/exports/orca` — `{filament_ids?}` → zip of OrcaSlicer filament profiles
//! - `POST /api/exports/hueforge` — `{filament_ids?}` → HueForge library JSON
//! - `POST /api/swatch/fit` — `{line1, line2}` → sizes (preview; builds no mesh)
//! - `POST /api/swatch/stl` — `{line1, line2}` → binary STL
//! - `POST /api/swatch/batch` — `{filament_ids}` → zip of STLs with each filament's default text

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use super::ssh::{ApiError, ApiResult};
use crate::{
    exports::{self as svc, ExportRequest, ExportRow, SwatchFit, SwatchLines},
    spools::Spoolman,
    state::AppState,
};

fn sm(s: &AppState) -> ApiResult<&Spoolman> {
    s.spoolman
        .get()
        .ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "Spoolman is not running"))
}

fn download(bytes: Vec<u8>, content_type: &str, file: &str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type.to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{file}\""),
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        ],
        bytes,
    )
        .into_response()
}

pub async fn list(State(s): State<AppState>) -> ApiResult<Json<Vec<ExportRow>>> {
    Ok(Json(svc::rows(sm(&s)?).await?))
}

pub async fn orca(State(s): State<AppState>, Json(b): Json<ExportRequest>) -> ApiResult<Response> {
    let (zip, infos) = svc::orca_zip(sm(&s)?, b.filament_ids.as_deref()).await?;
    let file = match infos.as_slice() {
        [one] => format!("orca-{}.zip", svc::file_slug(&one.name)),
        _ => "foreman-orca-filaments.zip".to_owned(),
    };
    Ok(download(zip, "application/zip", &file))
}

pub async fn hueforge(
    State(s): State<AppState>,
    Json(b): Json<ExportRequest>,
) -> ApiResult<Response> {
    let (v, _) = svc::hueforge(sm(&s)?, b.filament_ids.as_deref()).await?;
    let text = serde_json::to_vec_pretty(&v)
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(download(text, "application/json", "foreman-hueforge.json"))
}

pub async fn swatch_fit(Json(b): Json<SwatchLines>) -> Json<SwatchFit> {
    Json(svc::swatch_fit(&b))
}

pub async fn swatch_stl(Json(b): Json<SwatchLines>) -> ApiResult<Response> {
    let name = svc::file_slug(&format!("{} {}", b.line1, b.line2));
    let stl = svc::swatch_stl(b).await?;
    Ok(download(stl, "model/stl", &format!("swatch-{name}.stl")))
}

pub async fn swatch_batch(
    State(s): State<AppState>,
    Json(b): Json<ExportRequest>,
) -> ApiResult<Response> {
    let ids = b.filament_ids.unwrap_or_default();
    let zip = svc::swatch_zip(sm(&s)?, &ids).await?;
    Ok(download(zip, "application/zip", "foreman-swatches.zip"))
}

/// `GET /api/exports/spools.csv` — every spool for spreadsheets.
pub async fn spools_csv(State(s): State<AppState>) -> ApiResult<Response> {
    let (csv, _) = svc::spools_csv(sm(&s)?).await?;
    let day = chrono::Local::now().format("%Y-%m-%d");
    Ok(download(
        csv,
        "text/csv; charset=utf-8",
        &format!("foreman-spools-{day}.csv"),
    ))
}

/// `GET /api/exports/spoolman.json` — vendors, filaments, spools in Spoolman's API shape.
pub async fn spoolman_json(State(s): State<AppState>) -> ApiResult<Response> {
    let v = svc::spoolman_json(sm(&s)?).await?;
    let text = serde_json::to_vec_pretty(&v)
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let day = chrono::Local::now().format("%Y-%m-%d");
    Ok(download(
        text,
        "application/json",
        &format!("foreman-spoolman-{day}.json"),
    ))
}
