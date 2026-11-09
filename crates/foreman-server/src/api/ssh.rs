//! SSH settings, SFTP file API and shell macros (task 2.5).
//!
//! Every SFTP call opens a short-lived SSH session to the printer's *active* connection
//! path host, authenticates (Foreman key or encrypted password) and enforces TOFU host-key
//! pinning. A changed host key is refused, stored as pending, and raised in Attention.

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use foreman_core::{
    domain::{CreateShellMacro, PrinterSsh, UpdateShellMacro, UpsertPrinterSsh},
    driver::PrinterCommand,
    error::AppError,
    events::{AttentionItemEvent, PrinterState, PrinterStatus, ServerEvent},
    remote_fs::{self, Roots, SftpFs},
    repo,
    ssh_client::{self, ConnectError, SshConnection},
};
use futures_util::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tokio::io::AsyncWriteExt as _;
use tracing::{info, warn};

use crate::{ssh_pool::PooledSftp, state::AppState};

pub(crate) const HOST_KEY_ATTENTION_KIND: &str = "ssh_host_key";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// JSON error response `{ "error": "...", "code": "..." }`.
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    pub code: Option<&'static str>,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            code: None,
        }
    }
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        let status = match &e {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::InvalidInput(_) => StatusCode::BAD_REQUEST,
            AppError::Driver(_) => StatusCode::BAD_GATEWAY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(status, e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "error": self.message, "code": self.code })),
        )
            .into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

// ---------------------------------------------------------------------------
// Session setup
// ---------------------------------------------------------------------------

/// An authenticated session plus what the UI needs to label it.
pub(crate) struct Session {
    pub conn: SshConnection,
    pub cfg: PrinterSsh,
    pub host: String,
}

/// Host of the printer's active connection path, falling back to `primary_url`.
async fn resolve_host(state: &AppState, printer_id: i64) -> ApiResult<String> {
    if let Some((_, path)) = state.driver_manager.get_active_path(printer_id).await {
        if let Some(h) = ssh_client::host_from_url(&path.url) {
            return Ok(h);
        }
    }
    let printer = repo::printer::get(&state.foreman_db.pool, printer_id).await?;
    printer
        .primary_url
        .as_deref()
        .and_then(ssh_client::host_from_url)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "printer has no address"))
}

async fn raise_host_key_attention(state: &AppState, printer_id: i64, presented: &str) {
    let pool = &state.foreman_db.pool;
    match repo::attention::get_active_by_printer_kind(pool, printer_id, HOST_KEY_ATTENTION_KIND)
        .await
    {
        Ok(Some(_)) => return,
        Ok(None) => {}
        Err(e) => warn!(printer_id, "attention lookup: {e}"),
    }
    let name = repo::printer::get(pool, printer_id)
        .await
        .map(|p| p.name)
        .unwrap_or_else(|_| format!("printer {printer_id}"));
    let title = format!("{name}: SSH host key changed");
    let body = format!(
        "The printer presented a different SSH host key ({presented}). Foreman refuses to \
         connect until you trust it in Config → SSH."
    );
    match repo::attention::raise(
        pool,
        Some(printer_id),
        HOST_KEY_ATTENTION_KIND,
        &title,
        Some(&body),
    )
    .await
    {
        Ok(item) => {
            let _ = state.bus_tx.send(ServerEvent::AttentionAdded {
                item: AttentionItemEvent {
                    id: item.id,
                    printer_id: Some(printer_id),
                    kind: HOST_KEY_ATTENTION_KIND.to_owned(),
                    title: item.title,
                    body: item.body,
                },
            });
        }
        Err(e) => warn!(printer_id, "raise host-key attention: {e}"),
    }
}

/// Connect + pin/verify host key + authenticate.
pub(crate) async fn open_session(state: &AppState, printer_id: i64) -> ApiResult<Session> {
    let pool = &state.foreman_db.pool;
    let cfg = repo::printer_ssh::get(pool, printer_id)
        .await
        .map_err(|_| ApiError {
            status: StatusCode::NOT_FOUND,
            message: "SSH is not set up for this printer — add it in Config → SSH".into(),
            code: Some("ssh_not_configured"),
        })?;
    let host = resolve_host(state, printer_id).await?;
    let port = u16::try_from(cfg.ssh_port)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid SSH port"))?;

    let mut conn = match ssh_client::connect(&host, port, cfg.host_key_fingerprint.as_deref()).await
    {
        Ok(c) => c,
        Err(ConnectError::HostKeyMismatch { presented }) => {
            warn!(printer_id, %presented, "SSH host key changed — refusing");
            if let Err(e) =
                repo::printer_ssh::set_pending_host_key(pool, printer_id, &presented).await
            {
                warn!(printer_id, "store pending host key: {e}");
            }
            raise_host_key_attention(state, printer_id, &presented).await;
            return Err(ApiError {
                status: StatusCode::CONFLICT,
                message: format!(
                    "SSH host key changed (now {presented}). Refusing to connect until you \
                     trust the new key in Config → SSH."
                ),
                code: Some("host_key_changed"),
            });
        }
        Err(ConnectError::Other(e)) => return Err(e.into()),
    };

    if cfg.host_key_fingerprint.is_none() {
        if let Some(fp) = conn.received_fingerprint() {
            repo::printer_ssh::store_host_key(pool, printer_id, &fp).await?;
            info!(printer_id, %fp, "TOFU: pinned SSH host key");
        }
    }

    let auth = if cfg.auth_type == "password" {
        let sealed = repo::printer_ssh::get_sealed_password(pool, printer_id)
            .await?
            .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "no SSH password stored"))?;
        let password = state.secrets.open(&sealed)?;
        ssh_client::auth_with_password(&mut conn, &cfg.ssh_user, &password).await
    } else {
        ssh_client::auth_with_key(&mut conn, &cfg.ssh_user, Arc::clone(&state.ssh_identity.0)).await
    };
    if let Err(e) = auth {
        conn.disconnect().await;
        return Err(ApiError {
            status: StatusCode::BAD_GATEWAY,
            message: e.to_string(),
            code: Some("ssh_auth_failed"),
        });
    }

    Ok(Session { conn, cfg, host })
}

/// The printer's pooled SFTP session, opening (and pinning/authenticating) one if needed.
async fn open_fs(state: &AppState, printer_id: i64) -> ApiResult<Arc<PooledSftp>> {
    if let Some(s) = state.ssh_pool.get(printer_id).await {
        return Ok(s);
    }
    let _guard = state.ssh_pool.open_lock.lock().await;
    if let Some(s) = state.ssh_pool.get(printer_id).await {
        return Ok(s);
    }
    let session = open_session(state, printer_id).await?;
    let fs = match ssh_client::open_sftp(&session.conn).await {
        Ok(s) => SftpFs::new(s),
        Err(e) => {
            session.conn.disconnect().await;
            return Err(e.into());
        }
    };
    let roots = match Roots::discover(&fs).await {
        Ok(r) => r,
        Err(e) => {
            session.conn.disconnect().await;
            return Err(e.into());
        }
    };
    info!(printer_id, host = %session.host, "SSH session opened");
    let pooled = PooledSftp::new(
        session.conn,
        fs,
        roots,
        session.cfg.ssh_user,
        session.host,
        session.cfg.ssh_port,
    );
    Ok(state.ssh_pool.insert(printer_id, pooled).await)
}

async fn audit(state: &AppState, printer_id: i64, user: &str, command: &str) {
    if let Err(e) = repo::audit_log::insert(
        &state.foreman_db.pool,
        Some(printer_id),
        Some(user),
        command,
    )
    .await
    {
        warn!(printer_id, "audit_log insert failed: {e}");
    }
}

// ---------------------------------------------------------------------------
// SSH settings
// ---------------------------------------------------------------------------

/// `GET /api/ssh/public-key`
pub async fn get_public_key(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({ "public_key": state.ssh_identity.1 }))
}

/// `GET /api/printers/{id}/ssh`
pub async fn get_ssh(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<PrinterSsh>> {
    Ok(Json(
        repo::printer_ssh::get(&state.foreman_db.pool, id).await?,
    ))
}

/// `PUT /api/printers/{id}/ssh`
pub async fn upsert_ssh(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<UpsertPrinterSsh>,
) -> ApiResult<Json<PrinterSsh>> {
    let pool = &state.foreman_db.pool;
    let printer = repo::printer::get(pool, id).await?;
    if printer.firmware == foreman_core::domain::FirmwareKind::Bambu {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Bambu printers have no SSH",
        ));
    }
    let sealed = match body.password.as_deref() {
        Some(pw) if !pw.is_empty() => Some(state.secrets.seal(pw)?),
        _ => None,
    };
    let cfg = repo::printer_ssh::upsert(pool, id, body, sealed).await?;
    state.ssh_pool.close(id).await;
    Ok(Json(cfg))
}

/// `DELETE /api/printers/{id}/ssh`
pub async fn delete_ssh(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    repo::printer_ssh::delete(&state.foreman_db.pool, id).await?;
    state.ssh_pool.close(id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/printers/{id}/ssh/trust` — pin the pending (changed) host key.
pub async fn trust_host_key(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<PrinterSsh>> {
    let pool = &state.foreman_db.pool;
    let fp = repo::printer_ssh::trust_pending_host_key(pool, id).await?;
    state.ssh_pool.close(id).await;
    info!(printer_id = id, %fp, "user trusted new SSH host key");
    if let Ok(Some(item)) =
        repo::attention::get_active_by_printer_kind(pool, id, HOST_KEY_ATTENTION_KIND).await
    {
        repo::attention::clear_by_printer_kind(pool, id, HOST_KEY_ATTENTION_KIND).await?;
        let _ = state.bus_tx.send(ServerEvent::AttentionCleared {
            item_id: item.id,
            printer_id: Some(id),
        });
    }
    Ok(Json(repo::printer_ssh::get(pool, id).await?))
}

/// `POST /api/printers/{id}/ssh/test` — fresh connect + authenticate + resolve home. Runs no command.
pub async fn test_ssh(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    state.ssh_pool.close(id).await;
    let s = open_fs(&state, id).await?;
    Ok(Json(json!({
        "ok": true,
        "user": s.user,
        "host": s.host,
        "home": s.roots.home,
        "host_key_fingerprint": s.fingerprint,
    })))
}

/// `GET /api/printers/{id}/ssh/session` — is a pooled session open, and for how long.
pub async fn session_status(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Json<serde_json::Value> {
    match state.ssh_pool.info(id).await {
        Some(i) => {
            Json(json!({ "open": true, "open_secs": i.open_secs, "idle_secs": i.idle_secs }))
        }
        None => Json(json!({ "open": false })),
    }
}

/// `DELETE /api/printers/{id}/ssh/session` — close the pooled SFTP session now.
pub async fn close_session(State(state): State<AppState>, Path(id): Path<i64>) -> StatusCode {
    state.ssh_pool.close(id).await;
    StatusCode::NO_CONTENT
}

// ---------------------------------------------------------------------------
// SFTP
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PathQuery {
    path: String,
}

#[derive(Debug, Deserialize)]
pub struct WriteBody {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
pub struct MkdirBody {
    path: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameBody {
    from: String,
    to: String,
}

/// `GET /api/printers/{id}/files/roots`
pub async fn roots(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    let s = open_fs(&state, id).await?;
    Ok(Json(json!({
        "user": s.user,
        "host": s.host,
        "port": s.port,
        "roots": s.roots,
    })))
}

/// `GET /api/printers/{id}/files?path=`
pub async fn list_files(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<PathQuery>,
) -> ApiResult<Response> {
    let s = open_fs(&state, id).await?;
    Ok(Json(remote_fs::list(&s.fs, &q.path).await?).into_response())
}

/// `GET /api/printers/{id}/files/read?path=` — UTF-8 text (tail if large).
pub async fn read_file(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<PathQuery>,
) -> ApiResult<Response> {
    let s = open_fs(&state, id).await?;
    Ok(Json(remote_fs::read_text(&s.fs, &s.roots, &q.path).await?).into_response())
}

/// `GET /api/printers/{id}/files/download?path=` — streamed raw bytes.
pub async fn download_file(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<PathQuery>,
) -> ApiResult<Response> {
    let s = open_fs(&state, id).await?;
    let path = remote_fs::normalize(&q.path)?;
    let file =
        s.fs.session()
            .open(path.clone())
            .await
            .map_err(|e| AppError::Driver(format!("sftp open '{path}': {e}")))?;
    let name: String = path
        .rsplit('/')
        .next()
        .unwrap_or("download")
        .chars()
        .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
        .collect();
    // The stream holds the pooled session so it can't be dropped mid-download.
    let keep = s;
    let stream = tokio_util::io::ReaderStream::new(file).map(move |chunk| {
        let _ = &keep;
        chunk
    });
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

/// `PUT /api/printers/{id}/files/write` — whole-file write, `.bak` first.
pub async fn write_file(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<WriteBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let s = open_fs(&state, id).await?;
    let backup =
        remote_fs::write_with_backup(&s.fs, &s.roots, &body.path, body.content.as_bytes()).await?;
    audit(
        &state,
        id,
        &s.user,
        &format!(
            "sftp write {}{}",
            body.path,
            if backup { " (.bak kept)" } else { "" }
        ),
    )
    .await;
    Ok(Json(json!({ "ok": true, "backup": backup })))
}

/// Refuse a firmware restart unless the printer is known to be idle (not printing/paused).
pub(crate) fn firmware_restart_allowed(status: Option<&PrinterStatus>) -> Result<(), AppError> {
    match status.map(|s| &s.state) {
        None | Some(PrinterState::Offline) => Err(AppError::InvalidInput(
            "printer state unknown — not restarting".into(),
        )),
        Some(PrinterState::Printing) | Some(PrinterState::Paused) => Err(AppError::InvalidInput(
            "printer is printing — FIRMWARE_RESTART refused; save now and restart after the job"
                .into(),
        )),
        Some(_) => Ok(()),
    }
}

/// `POST /api/printers/{id}/files/save-restart` — save a Klipper `.cfg`, then FIRMWARE_RESTART.
/// Refused (nothing written) while printing.
pub async fn save_and_restart(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<WriteBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let status = state.driver_manager.get_status(id).await;
    firmware_restart_allowed(status.as_ref()).map_err(|e| ApiError {
        code: Some("printing"),
        ..ApiError::from(e)
    })?;

    let s = open_fs(&state, id).await?;
    if !remote_fs::is_klipper_cfg(&s.roots, &body.path) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "only .cfg files under printer_data/config can be saved with FIRMWARE_RESTART",
        ));
    }
    let backup =
        remote_fs::write_with_backup(&s.fs, &s.roots, &body.path, body.content.as_bytes()).await?;
    let user = s.user.clone();
    audit(
        &state,
        id,
        &user,
        &format!("sftp write {} (.bak kept)", body.path),
    )
    .await;

    // Re-check: a print may have started while we were saving.
    firmware_restart_allowed(state.driver_manager.get_status(id).await.as_ref())?;
    state
        .driver_manager
        .send_command(
            id,
            PrinterCommand::Gcode {
                command: "FIRMWARE_RESTART".into(),
            },
            None,
        )
        .await?;
    Ok(Json(
        json!({ "ok": true, "backup": backup, "restarted": true }),
    ))
}

/// `POST /api/printers/{id}/files/upload?path=` — raw body streamed to the printer.
pub async fn upload_file(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<PathQuery>,
    body: Body,
) -> ApiResult<Json<serde_json::Value>> {
    let s = open_fs(&state, id).await?;
    let (fs, roots) = (&s.fs, &s.roots);
    let result: Result<(String, u64), ApiError> = async {
        let path = remote_fs::prepare_upload(fs, roots, &q.path).await?;
        let mut file = fs
            .session()
            .create(path.clone())
            .await
            .map_err(|e| AppError::Driver(format!("sftp create '{path}': {e}")))?;
        let mut stream = body.into_data_stream();
        let mut total = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.to_string()))?;
            total += chunk.len() as u64;
            file.write_all(&chunk)
                .await
                .map_err(|e| AppError::Driver(format!("sftp write '{path}': {e}")))?;
        }
        file.shutdown()
            .await
            .map_err(|e| AppError::Driver(format!("sftp close '{path}': {e}")))?;
        Ok((path, total))
    }
    .await;
    let (path, total) = result?;
    audit(
        &state,
        id,
        &s.user,
        &format!("sftp upload {path} ({total} bytes)"),
    )
    .await;
    Ok(Json(json!({ "ok": true, "path": path, "size": total })))
}

/// `POST /api/printers/{id}/files/mkdir`
pub async fn mkdir(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<MkdirBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let s = open_fs(&state, id).await?;
    let path = remote_fs::mkdir(&s.fs, &s.roots, &body.path).await?;
    audit(&state, id, &s.user, &format!("sftp mkdir {path}")).await;
    Ok(Json(json!({ "ok": true, "path": path })))
}

/// `POST /api/printers/{id}/files/rename`
pub async fn rename_file(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<RenameBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let s = open_fs(&state, id).await?;
    let (from, to) = remote_fs::rename(&s.fs, &s.roots, &body.from, &body.to).await?;
    audit(&state, id, &s.user, &format!("sftp rename {from} -> {to}")).await;
    Ok(Json(json!({ "ok": true, "path": to })))
}

/// `DELETE /api/printers/{id}/files?path=`
pub async fn delete_file(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<PathQuery>,
) -> ApiResult<StatusCode> {
    let s = open_fs(&state, id).await?;
    let path = remote_fs::delete(&s.fs, &s.roots, &q.path).await?;
    audit(&state, id, &s.user, &format!("sftp delete {path}")).await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Shell macros (run happens over the terminal WebSocket so output shows there)
// ---------------------------------------------------------------------------

/// `GET /api/printers/{id}/shell-macros`
pub async fn list_macros(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    Ok(
        Json(repo::shell_macro::list_for_printer(&state.foreman_db.pool, id).await?)
            .into_response(),
    )
}

/// `POST /api/printers/{id}/shell-macros`
pub async fn create_macro(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<CreateShellMacro>,
) -> ApiResult<Response> {
    let m = repo::shell_macro::create(&state.foreman_db.pool, id, body).await?;
    Ok((StatusCode::CREATED, Json(m)).into_response())
}

async fn visible_macro(
    state: &AppState,
    printer_id: i64,
    mid: i64,
) -> ApiResult<foreman_core::domain::ShellMacro> {
    let m = repo::shell_macro::get(&state.foreman_db.pool, mid).await?;
    if m.scope == "all_ssh" || m.printer_id == Some(printer_id) {
        Ok(m)
    } else {
        Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "shell macro not found",
        ))
    }
}

/// `PUT /api/printers/{id}/shell-macros/{mid}`
pub async fn update_macro(
    State(state): State<AppState>,
    Path((id, mid)): Path<(i64, i64)>,
    Json(body): Json<UpdateShellMacro>,
) -> ApiResult<Response> {
    visible_macro(&state, id, mid).await?;
    let m = repo::shell_macro::update(&state.foreman_db.pool, id, mid, body).await?;
    Ok(Json(m).into_response())
}

/// `DELETE /api/printers/{id}/shell-macros/{mid}`
pub async fn delete_macro(
    State(state): State<AppState>,
    Path((id, mid)): Path<(i64, i64)>,
) -> ApiResult<StatusCode> {
    visible_macro(&state, id, mid).await?;
    repo::shell_macro::delete(&state.foreman_db.pool, mid).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn macro_for_run(
    state: &AppState,
    printer_id: i64,
    mid: i64,
) -> ApiResult<foreman_core::domain::ShellMacro> {
    visible_macro(state, printer_id, mid).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: PrinterState) -> PrinterStatus {
        PrinterStatus {
            state,
            ..Default::default()
        }
    }

    #[test]
    fn restart_refused_while_printing_or_unknown() {
        assert!(firmware_restart_allowed(Some(&status(PrinterState::Printing))).is_err());
        assert!(firmware_restart_allowed(Some(&status(PrinterState::Paused))).is_err());
        assert!(firmware_restart_allowed(Some(&status(PrinterState::Offline))).is_err());
        assert!(firmware_restart_allowed(None).is_err());
        assert!(firmware_restart_allowed(Some(&status(PrinterState::Idle))).is_ok());
        assert!(firmware_restart_allowed(Some(&status(PrinterState::Error))).is_ok());
    }
}
