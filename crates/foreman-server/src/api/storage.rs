//! Printer G-code storage as Library locations, and transfers between locations (task 3.2).

use std::sync::{atomic::AtomicU64, Arc};

use axum::{
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use foreman_core::{
    domain::FirmwareKind,
    repo,
    storage::{clean_path, covers, join, name_of, parent_of, PrinterStorage},
};
use serde::Deserialize;
use serde_json::json;
use tracing::info;

use super::library::{bad, spool_files, ApiError};
use crate::{
    state::AppState,
    transfers::{copy_name, is_stl, StartOutcome, TransferRequest},
};

type ApiResult<T> = Result<T, ApiError>;

async fn storage_of(state: &AppState, id: i64) -> ApiResult<(String, Arc<dyn PrinterStorage>)> {
    state.driver_manager.storage(id).await.ok_or_else(|| {
        ApiError(
            StatusCode::NOT_FOUND,
            format!("printer {id} has no browsable storage (Klipper printers only)"),
        )
    })
}

fn one_name(name: &str) -> ApiResult<String> {
    let n = clean_path(name.trim())?;
    if n.is_empty() || n.contains('/') {
        return Err(bad("use a name without slashes"));
    }
    Ok(n)
}

/// `GET /api/locations` — Library + every Klipper printer's G-code storage.
pub async fn locations(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let printers = repo::printer::list(&state.foreman_db.pool).await?;
    let mut out = Vec::new();
    for p in printers
        .into_iter()
        .filter(|p| p.firmware == FirmwareKind::Moonraker)
    {
        let status = state.driver_manager.get_status(p.id).await;
        out.push(json!({
            "printer_id": p.id,
            "name": p.name,
            "browsable": state.driver_manager.storage(p.id).await.is_some(),
            "state": status.as_ref().map(|s| &s.state),
            "printing_file": status.and_then(|s| s.filename),
        }));
    }
    Ok(Json(json!({ "printers": out })))
}

#[derive(Deserialize)]
pub struct PathQuery {
    #[serde(default)]
    path: String,
}

/// `GET /api/printers/{id}/storage?path=` — one folder of the gcodes root.
pub async fn list(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<PathQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let (name, storage) = storage_of(&state, id).await?;
    let path = clean_path(&q.path)?;
    let entries = storage.list_dir(&path).await?;
    let active = storage.active_file().await.ok().flatten();
    Ok(Json(json!({
        "printer_id": id,
        "name": name,
        "path": path,
        "entries": entries,
        "active_file": active,
    })))
}

async fn stream_file(
    state: &AppState,
    id: i64,
    path: &str,
    inline_image: bool,
) -> ApiResult<Response> {
    let (_, storage) = storage_of(state, id).await?;
    let path = clean_path(path)?;
    let (len, stream) = storage.read_stream(&path).await?;
    let name = name_of(&path).replace('"', "");
    let lower = name.to_ascii_lowercase();
    let mut resp = Body::from_stream(stream).into_response();
    let h = resp.headers_mut();
    if inline_image {
        let ct = if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
            "image/jpeg"
        } else {
            "image/png"
        };
        h.insert(header::CONTENT_TYPE, header::HeaderValue::from_static(ct));
        h.insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("private, max-age=300"),
        );
    } else {
        h.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/octet-stream"),
        );
        if let Ok(v) = header::HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
            h.insert(header::CONTENT_DISPOSITION, v);
        }
    }
    if let Some(n) = len {
        h.insert(header::CONTENT_LENGTH, n.into());
    }
    Ok(resp)
}

/// `GET /api/printers/{id}/storage/thumb?path=<thumbnail path>` (URL-token aware).
pub async fn thumb(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<PathQuery>,
) -> ApiResult<Response> {
    let lower = q.path.to_ascii_lowercase();
    if !(lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg")) {
        return Err(bad("not an image"));
    }
    stream_file(&state, id, &q.path, true).await
}

/// `GET /api/printers/{id}/storage/download?path=` (URL-token aware).
pub async fn download(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<PathQuery>,
) -> ApiResult<Response> {
    stream_file(&state, id, &q.path, false).await
}

#[derive(Deserialize)]
pub struct NameBody {
    #[serde(default)]
    path: String,
    name: String,
}

/// `POST /api/printers/{id}/storage/folders {path, name}`
pub async fn new_folder(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<NameBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let (_, storage) = storage_of(&state, id).await?;
    let folder = clean_path(&body.path)?;
    let name = one_name(&body.name)?;
    if storage
        .list_dir(&folder)
        .await?
        .iter()
        .any(|e| e.name.eq_ignore_ascii_case(&name))
    {
        return Err(bad("That name already exists in this folder."));
    }
    let path = join(&folder, &name);
    storage.mkdir(&path).await?;
    Ok((StatusCode::CREATED, Json(json!({ "path": path }))))
}

/// `POST /api/printers/{id}/storage/rename {path, name}` — never renames a printing file.
pub async fn rename(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<NameBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let (_, storage) = storage_of(&state, id).await?;
    let from = clean_path(&body.path)?;
    if from.is_empty() {
        return Err(bad("nothing to rename"));
    }
    let name = one_name(&body.name)?;
    let to = join(parent_of(&from), &name);
    if to == from {
        return Ok(Json(json!({ "path": to })));
    }
    if let Some(a) = storage.active_file().await.ok().flatten() {
        if covers(&from, &a) {
            return Err(ApiError(StatusCode::CONFLICT, "printing now".into()));
        }
    }
    if storage
        .list_dir(parent_of(&from))
        .await?
        .iter()
        .any(|e| e.name == name)
    {
        return Err(bad("That name already exists in this folder."));
    }
    storage.move_entry(&from, &to).await?;
    Ok(Json(json!({ "path": to })))
}

#[derive(Deserialize)]
pub struct PathsBody {
    paths: Vec<String>,
}

/// `POST /api/printers/{id}/storage/delete {paths}` — permanent; printing files are skipped.
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<PathsBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let (deleted, blocked) = state.transfers.delete_on_printer(id, &body.paths).await?;
    Ok(Json(json!({ "deleted": deleted, "blocked": blocked })))
}

/// `POST /api/printers/{id}/storage/upload?path=` (multipart). STL is refused; a name
/// already in use is kept as ` (copy)`.
pub async fn upload(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<PathQuery>,
    mp: Multipart,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let files = spool_files(&state, mp).await?;
    let cleanup = |files: &[(String, std::path::PathBuf)]| {
        for (_, tmp) in files {
            let _ = std::fs::remove_file(tmp);
        }
    };
    let result = async {
        let (name, storage) = storage_of(&state, id).await?;
        let folder = clean_path(&q.path)?;
        if files.is_empty() {
            return Err(bad("no file in the upload"));
        }
        if let Some((n, _)) = files.iter().find(|(n, _)| is_stl(n)) {
            return Err(bad(format!("{n} is an STL: slice first")));
        }
        let mut taken: std::collections::HashSet<String> = storage
            .list_dir(&folder)
            .await?
            .into_iter()
            .map(|e| e.name.to_lowercase())
            .collect();
        let mut paths = Vec::new();
        for (file, tmp) in &files {
            let file = one_name(file)?;
            let file = if taken.contains(&file.to_lowercase()) {
                copy_name(&file, false, &taken)
            } else {
                file
            };
            taken.insert(file.to_lowercase());
            let dest = join(&folder, &file);
            storage
                .upload(&dest, tmp, Arc::new(AtomicU64::new(0)))
                .await?;
            info!(printer = %name, path = %dest, "uploaded to printer");
            paths.push(dest);
        }
        Ok(paths)
    }
    .await;
    cleanup(&files);
    let paths = result?;
    Ok((StatusCode::CREATED, Json(json!({ "paths": paths }))))
}

// ---------------------------------------------------------------------------
// Transfers
// ---------------------------------------------------------------------------

/// `POST /api/transfers` — 202 with started transfers, or 409 with the conflicts to resolve.
pub async fn start_transfer(
    State(state): State<AppState>,
    Json(req): Json<TransferRequest>,
) -> ApiResult<Response> {
    let outcome = state.transfers.start(req).await?;
    let status = match outcome {
        StartOutcome::Conflicts { .. } => StatusCode::CONFLICT,
        StartOutcome::Started { .. } => StatusCode::ACCEPTED,
    };
    Ok((status, Json(outcome)).into_response())
}

/// `GET /api/transfers`
pub async fn list_transfers(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "transfers": state.transfers.list() }))
}

/// `POST /api/transfers/{id}/cancel`
pub async fn cancel_transfer(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<foreman_core::events::TransferInfo>> {
    Ok(Json(state.transfers.cancel(&id)?))
}

/// `POST /api/transfers/clear` — drop finished transfers from the list.
pub async fn clear_transfers(State(state): State<AppState>) -> StatusCode {
    state.transfers.clear_finished();
    StatusCode::NO_CONTENT
}
