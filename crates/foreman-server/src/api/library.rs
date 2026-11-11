//! Library REST (task 3.1/3.2 basics) and the OctoPrint-compatible slicer upload (task 3.4).

use std::path::PathBuf;

use axum::{
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use foreman_core::{
    domain::{LibraryEntry, LibraryFileDetails},
    error::AppError,
    repo,
};
use serde::Deserialize;
use serde_json::json;
use tokio::io::AsyncWriteExt as _;
use tracing::{info, warn};

use crate::{auth::AuthCtx, state::AppState};

/// Library folder that slicer uploads land in.
pub const SLICER_INBOX: &str = "Slicer inbox";
pub const SETTING_TRASH_DAYS: &str = "library.trash_days";
pub const DEFAULT_TRASH_DAYS: i64 = 30;
/// The OctoPrint release whose upload API we mirror (`/api/version`).
const OCTOPRINT_COMPAT: &str = "1.10.3";

pub struct ApiError(pub(crate) StatusCode, pub(crate) String);

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        let status = match &e {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::InvalidInput(_) => StatusCode::BAD_REQUEST,
            AppError::Driver(_) => StatusCode::BAD_GATEWAY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::BAD_GATEWAY {
            warn!("library: {e}");
        }
        ApiError(status, e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

pub(crate) fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

pub async fn trash_days(state: &AppState) -> i64 {
    repo::setting::get(&state.foreman_db.pool, SETTING_TRASH_DAYS)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_TRASH_DAYS)
}

// ---------------------------------------------------------------------------
// Uploads (shared by the Library and the OctoPrint endpoint)
// ---------------------------------------------------------------------------

struct Spooled {
    name: String,
    tmp: PathBuf,
}

/// A spooled upload: (file name, temp path). The caller owns the temp file.
pub(crate) async fn spool_files(
    state: &AppState,
    mp: Multipart,
) -> ApiResult<Vec<(String, PathBuf)>> {
    let (files, _) = spool_multipart(state, mp).await?;
    Ok(files.into_iter().map(|f| (f.name, f.tmp)).collect())
}

/// Read a multipart body: files are streamed to temp files, text fields collected.
async fn spool_multipart(
    state: &AppState,
    mut mp: Multipart,
) -> ApiResult<(Vec<Spooled>, Vec<(String, String)>)> {
    let mut files = Vec::new();
    let mut fields = Vec::new();
    while let Some(mut field) = mp.next_field().await.map_err(|e| bad(e.to_string()))? {
        let field_name = field.name().unwrap_or_default().to_owned();
        let Some(file_name) = field.file_name().map(str::to_owned) else {
            let text = field.text().await.map_err(|e| bad(e.to_string()))?;
            fields.push((field_name, text));
            continue;
        };
        // Browsers/slicers may send a path; keep only the last component.
        let name = file_name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_owned();
        let tmp = state
            .library
            .tmp_dir()
            .join(format!(".foreman-upload-{}", uuid::Uuid::new_v4()));
        let mut out = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| ApiError::from(AppError::Internal(e.to_string())))?;
        let written = async {
            while let Some(chunk) = field.chunk().await.map_err(|e| bad(e.to_string()))? {
                out.write_all(&chunk)
                    .await
                    .map_err(|e| ApiError::from(AppError::Internal(e.to_string())))?;
            }
            out.flush()
                .await
                .map_err(|e| ApiError::from(AppError::Internal(e.to_string())))
        }
        .await;
        if let Err(e) = written {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e);
        }
        files.push(Spooled { name, tmp });
    }
    Ok((files, fields))
}

async fn store_files(
    state: &AppState,
    folder: &str,
    files: Vec<Spooled>,
) -> ApiResult<Vec<LibraryEntry>> {
    let mut out = Vec::new();
    let mut pending = files.into_iter();
    while let Some(f) = pending.next() {
        match state.library.add_file(folder, &f.name, &f.tmp).await {
            Ok(e) => out.push(e),
            Err(e) => {
                let _ = tokio::fs::remove_file(&f.tmp).await;
                for rest in pending {
                    let _ = tokio::fs::remove_file(&rest.tmp).await;
                }
                return Err(e.into());
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Library REST
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct FolderQuery {
    #[serde(default)]
    path: String,
}

/// `GET /api/library?path=<folder>`
pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<FolderQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let entries = state.library.list(&q.path).await?;
    Ok(Json(
        json!({ "path": q.path.trim_matches('/'), "entries": entries }),
    ))
}

/// `GET /api/library/trash`
pub async fn list_trash(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let entries = state.library.list_trash().await?;
    Ok(Json(
        json!({ "entries": entries, "purge_after_days": trash_days(&state).await }),
    ))
}

/// `GET /api/library/files/{id}`
pub async fn details(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<LibraryFileDetails>> {
    Ok(Json(state.library.details(id).await?))
}

/// `GET /api/library/files/{id}/thumb` (URL-token aware for `<img src>`).
pub async fn thumb(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Response> {
    let bytes = tokio::fs::read(state.library.thumb_path(id))
        .await
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "no thumbnail".into()))?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            // The URL carries ?v=<hash prefix>, so a new file gets a new URL.
            (header::CACHE_CONTROL, "private, max-age=86400"),
        ],
        bytes,
    )
        .into_response())
}

/// `GET /api/library/files/{id}/download` (URL-token aware for `<a href>`).
pub async fn download(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Response> {
    let path = state.library.file_path(id).await?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .replace('"', "");
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| ApiError::from(AppError::Internal(e.to_string())))?;
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file));
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        body,
    )
        .into_response())
}

/// `POST /api/library/upload?path=<folder>` (multipart; any number of `file` parts).
pub async fn upload(
    State(state): State<AppState>,
    Query(q): Query<FolderQuery>,
    mp: Multipart,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let (files, _) = spool_multipart(&state, mp).await?;
    if files.is_empty() {
        return Err(bad("no file in the upload"));
    }
    let entries = store_files(&state, &q.path, files).await?;
    Ok((StatusCode::CREATED, Json(json!({ "entries": entries }))))
}

#[derive(Deserialize)]
pub struct NewFolder {
    #[serde(default)]
    path: String,
    name: String,
}

/// `POST /api/library/folders {path, name}`
pub async fn new_folder(
    State(state): State<AppState>,
    Json(body): Json<NewFolder>,
) -> ApiResult<(StatusCode, Json<LibraryEntry>)> {
    Ok((
        StatusCode::CREATED,
        Json(state.library.mkdir(&body.path, &body.name).await?),
    ))
}

#[derive(Deserialize)]
pub struct EntryPatch {
    name: Option<String>,
    starred: Option<bool>,
}

/// `PATCH /api/library/entries/{id} {name?, starred?}`
pub async fn patch_entry(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<EntryPatch>,
) -> ApiResult<Json<LibraryEntry>> {
    let mut entry = state.library.get(id).await?;
    if let Some(name) = body.name {
        entry = state.library.rename(id, &name).await?;
    }
    if let Some(starred) = body.starred {
        entry = state.library.set_starred(id, starred).await?;
    }
    Ok(Json(entry))
}

/// `DELETE /api/library/entries/{id}` → trash.
pub async fn trash_entry(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    state.library.trash(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/library/entries/{id}/restore`
pub async fn restore_entry(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<LibraryEntry>> {
    Ok(Json(state.library.restore(id).await?))
}

#[derive(Deserialize)]
pub struct Ids {
    ids: Vec<i64>,
}

/// `POST /api/library/trash-many {ids}` → `{trashed, errors}` (multi-select delete).
pub async fn trash_many(
    State(state): State<AppState>,
    Json(body): Json<Ids>,
) -> Json<serde_json::Value> {
    let mut trashed = Vec::new();
    let mut errors = Vec::new();
    for id in body.ids {
        match state.library.trash(id).await {
            Ok(()) => trashed.push(id),
            Err(e) => errors.push(json!({ "id": id, "error": e.to_string() })),
        }
    }
    Json(json!({ "trashed": trashed, "errors": errors }))
}

/// `POST /api/library/restore-many {ids}` → `{restored, errors}` (undo of a multi-delete).
pub async fn restore_many(
    State(state): State<AppState>,
    Json(body): Json<Ids>,
) -> Json<serde_json::Value> {
    let mut restored = Vec::new();
    let mut errors = Vec::new();
    for id in body.ids {
        match state.library.restore(id).await {
            Ok(e) => restored.push(e),
            Err(e) => errors.push(json!({ "id": id, "error": e.to_string() })),
        }
    }
    Json(json!({ "restored": restored, "errors": errors }))
}

#[derive(Deserialize)]
pub struct LibrarySettings {
    trash_days: i64,
}

/// `GET /api/settings/library`
pub async fn get_settings(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "trash_days": trash_days(&state).await }))
}

/// `PUT /api/settings/library {trash_days}` (1–365).
pub async fn put_settings(
    State(state): State<AppState>,
    Json(body): Json<LibrarySettings>,
) -> ApiResult<Json<serde_json::Value>> {
    if !(1..=365).contains(&body.trash_days) {
        return Err(bad("trash_days must be 1–365"));
    }
    repo::setting::set(
        &state.foreman_db.pool,
        SETTING_TRASH_DAYS,
        &body.trash_days.to_string(),
    )
    .await
    .map_err(ApiError::from)?;
    Ok(Json(json!({ "trash_days": body.trash_days })))
}

// ---------------------------------------------------------------------------
// OctoPrint-compatible endpoints for slicers (OrcaSlicer / PrusaSlicer "OctoPrint" host)
// ---------------------------------------------------------------------------

/// `GET /api/version` — slicers check `api` exists and `text` starts with "OctoPrint".
pub async fn octoprint_version(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "api": "0.1",
        "server": OCTOPRINT_COMPAT,
        "text": format!("OctoPrint {OCTOPRINT_COMPAT} (Foreman {})", state.info.version),
    }))
}

/// `GET /api/server`
pub async fn octoprint_server() -> Json<serde_json::Value> {
    Json(json!({ "version": OCTOPRINT_COMPAT, "safemode": null }))
}

/// `POST /api/files/local` — multipart `file` (+ `path`, `print`, `select`).
/// Files land in the Library's "Slicer inbox"; `print=true` adds them to the fleet queue.
pub async fn octoprint_upload(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    mp: Multipart,
) -> ApiResult<Response> {
    let (files, fields) = spool_multipart(&state, mp).await?;
    let field = |k: &str| {
        fields
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.trim().to_owned())
    };
    if files.is_empty() {
        return Err(bad("No file included"));
    }
    let sub = field("path").unwrap_or_default();
    let folder = format!("{SLICER_INBOX}/{}", sub.trim_matches('/'));
    let entries = store_files(&state, folder.trim_end_matches('/'), files).await?;
    let print = field("print").is_some_and(|v| v == "true");
    let plate = field("plateindex");
    for e in &entries {
        info!(path = %e.path, user = %ctx.username, print, plate = plate.as_deref().unwrap_or("-"), "slicer upload");
    }
    if print {
        // "Upload and print" from the slicer: the file joins the fleet queue; the
        // scheduler starts it on a free, compatible printer with a clear plate.
        for e in &entries {
            match crate::api::queue::enqueue(
                &state,
                e.id,
                None,
                foreman_core::domain::QueueSource::Slicer,
                &ctx.username,
            )
            .await
            {
                Ok(p) => {
                    info!(path = %e.path, position = p.position, "slicer print: added to the fleet queue")
                }
                Err(err) => warn!(path = %e.path, "slicer print: could not queue: {err}"),
            }
        }
    }
    let first = &entries[0];
    let resource = format!("/api/library/files/{}", first.id);
    let body = json!({
        "done": true,
        "files": {
            "local": {
                "name": first.name,
                "path": first.path,
                "display": first.name,
                "origin": "local",
                "refs": {
                    "resource": resource,
                    "download": format!("/api/library/files/{}/download", first.id),
                },
            }
        },
        "effectivePrint": false,
        "effectiveSelect": false,
    });
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, resource)],
        Json(body),
    )
        .into_response())
}
