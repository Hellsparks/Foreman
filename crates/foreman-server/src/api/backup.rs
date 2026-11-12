//! Backup + restore (task 6.2, W14).
//!
//! - `GET  /api/backup` — settings, folder contents, last run, next run, Library size
//! - `PUT  /api/backup/settings` — `{settings, passphrase?}` (`""` clears the passphrase)
//! - `POST /api/backup/check-dir` — `{dir}` → can Foreman write there?
//! - `POST /api/backup/run` — "Run now" with the saved settings (`{passphrase?}` if none saved)
//! - `POST /api/backup/inspect` — `{path, passphrase?}` → manifest + compatibility
//! - `POST /api/backup/restore` — `{path, passphrase?}` → stage, then restart the server

use std::{path::PathBuf, time::Duration};

use axum::{extract::State, http::StatusCode, Json};
use chrono::{DateTime, Local, Utc};
use foreman_core::{
    error::AppError,
    events::{AttentionItemEvent, ServerEvent},
    repo,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use ts_rs::TS;

use super::ssh::{ApiError, ApiResult};
use crate::{
    backup::{
        self as svc, BackupCounts, BackupEntry, BackupKind, BackupSettings, LastRun, Manifest,
    },
    state::AppState,
};

/// One backup or restore at a time.
static BUSY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Where this server's data lives (Spoolman only when it runs on SQLite).
pub fn sources(state: &AppState) -> svc::Sources {
    let spoolman_db = state.spoolman.get().and_then(|sm| {
        (sm.config.db_type == spoolman_rs::config::DbType::Sqlite).then(|| sm.config.sqlite_path())
    });
    svc::Sources {
        data_dir: state.data_dir.clone(),
        spoolman_db,
    }
}

pub async fn load_settings(state: &AppState) -> Result<BackupSettings, AppError> {
    Ok(repo::setting::get(&state.foreman_db.pool, svc::SETTING_KEY)
        .await?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default())
}

async fn last_run(state: &AppState) -> Option<LastRun> {
    repo::setting::get(&state.foreman_db.pool, svc::SETTING_LAST)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
}

async fn has_passphrase(state: &AppState) -> bool {
    repo::setting::get(&state.foreman_db.pool, svc::SETTING_PASSPHRASE)
        .await
        .ok()
        .flatten()
        .is_some_and(|v| !v.is_empty())
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct BackupOverview {
    pub settings: BackupSettings,
    pub has_passphrase: bool,
    pub default_dir: String,
    pub data_dir: String,
    #[ts(type = "number")]
    pub library_files: u64,
    #[ts(type = "number")]
    pub library_bytes: u64,
    pub last: Option<LastRun>,
    pub next_run: Option<String>,
    pub backups: Vec<BackupEntry>,
    /// Pre-restore snapshots in `<data_dir>/restore-snapshots`.
    pub snapshots: Vec<BackupEntry>,
    pub spoolman_included: bool,
}

pub async fn overview(State(s): State<AppState>) -> ApiResult<Json<BackupOverview>> {
    Ok(Json(build_overview(&s).await?))
}

async fn build_overview(s: &AppState) -> Result<BackupOverview, AppError> {
    let settings = load_settings(s).await?;
    let (library_files, library_bytes) = {
        let dd = s.data_dir.clone();
        tokio::task::spawn_blocking(move || svc::library_size(&dd))
            .await
            .unwrap_or((0, 0))
    };
    let next_run = svc::next_slot(&settings, &Local::now()).map(|d| d.to_rfc3339());
    let dir = PathBuf::from(&settings.dir);
    let snap_dir = s.data_dir.join(svc::SNAPSHOT_DIR);
    let (backups, snapshots) =
        tokio::task::spawn_blocking(move || (svc::list(&dir), svc::list(&snap_dir)))
            .await
            .unwrap_or_default();
    Ok(BackupOverview {
        has_passphrase: has_passphrase(s).await,
        default_dir: svc::default_dir().to_string_lossy().into_owned(),
        data_dir: s.data_dir.to_string_lossy().into_owned(),
        library_files,
        library_bytes,
        last: last_run(s).await,
        next_run,
        backups,
        snapshots,
        spoolman_included: sources(s).spoolman_db.is_some(),
        settings,
    })
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct PutBackupSettings {
    pub settings: BackupSettings,
    /// `None` keeps the saved passphrase, `""` removes it.
    #[ts(optional)]
    pub passphrase: Option<String>,
}

pub async fn put_settings(
    State(s): State<AppState>,
    Json(b): Json<PutBackupSettings>,
) -> ApiResult<Json<BackupOverview>> {
    Ok(Json(save_settings(&s, b.settings, b.passphrase).await?))
}

/// Validate and store backup settings (also used by first-run setup).
pub async fn save_settings(
    s: &AppState,
    settings: BackupSettings,
    passphrase: Option<String>,
) -> ApiResult<BackupOverview> {
    let mut next = settings;
    next.validate()?;
    let pool = &s.foreman_db.pool;
    if let Some(p) = &passphrase {
        if p.is_empty() {
            repo::setting::set(pool, svc::SETTING_PASSPHRASE, "").await?;
        } else {
            if p.chars().count() < 8 {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Use a passphrase of at least 8 characters.",
                ));
            }
            repo::setting::set(pool, svc::SETTING_PASSPHRASE, &s.secrets.seal(p)?).await?;
        }
    }
    if next.encrypt && !has_passphrase(s).await {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Encryption is on: enter a passphrase.",
        ));
    }
    let prev = load_settings(s).await?;
    // Re-anchor the schedule when it changes, so a slot that already passed today
    // does not fire the moment the schedule is saved.
    let changed =
        prev.schedule != next.schedule || prev.time != next.time || prev.weekday != next.weekday;
    next.schedule_since = if changed || prev.schedule_since.is_none() {
        Some(Utc::now().to_rfc3339())
    } else {
        prev.schedule_since
    };
    let json = serde_json::to_string(&next)
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    repo::setting::set(pool, svc::SETTING_KEY, &json).await?;
    Ok(build_overview(s).await?)
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct CheckDirRequest {
    pub dir: String,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct CheckDirResult {
    pub ok: bool,
    pub message: String,
}

/// Can Foreman create the folder and write a file in it?
pub async fn check_dir(Json(b): Json<CheckDirRequest>) -> Json<CheckDirResult> {
    Json(
        match check_dir_writable(PathBuf::from(b.dir.trim())).await {
            Ok(message) => CheckDirResult { ok: true, message },
            Err(message) => CheckDirResult { ok: false, message },
        },
    )
}

/// Create the folder if needed and prove a file can be written and deleted there.
pub async fn check_dir_writable(dir: PathBuf) -> Result<String, String> {
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        if !dir.is_absolute() {
            return Err("Use a full path.".into());
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create the folder: {e}"))?;
        let probe = dir.join(format!(".foreman-write-test-{}", uuid::Uuid::new_v4()));
        std::fs::write(&probe, b"ok").map_err(|e| format!("Cannot write there: {e}"))?;
        std::fs::remove_file(&probe).map_err(|e| format!("Cannot delete the test file: {e}"))?;
        let n = svc::list(&dir).len();
        Ok(format!(
            "Foreman can write here. {n} backup{} in this folder.",
            if n == 1 { "" } else { "s" }
        ))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()))
}

#[derive(Debug, Default, Deserialize, TS)]
#[ts(export)]
pub struct RunBackupRequest {
    /// Only needed when encryption is on and no passphrase is saved.
    #[ts(optional)]
    pub passphrase: Option<String>,
}

pub async fn run_now(
    State(s): State<AppState>,
    body: Option<Json<RunBackupRequest>>,
) -> ApiResult<Json<LastRun>> {
    let pass = body.and_then(|Json(b)| b.passphrase);
    let run = run_backup(&s, BackupKind::Manual, pass).await?;
    if run.ok {
        Ok(Json(run))
    } else {
        Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Backup failed: {}", run.error.unwrap_or_default()),
        ))
    }
}

/// Take a backup with the saved settings, prune, record the result, and raise or
/// clear the "Backup failed" Attention item. `Err` only when another backup runs.
pub async fn run_backup(
    s: &AppState,
    kind: BackupKind,
    passphrase: Option<String>,
) -> Result<LastRun, AppError> {
    let Ok(_lock) = BUSY.try_lock() else {
        return Err(AppError::InvalidInput(
            "A backup or restore is already running.".into(),
        ));
    };
    let settings = load_settings(s).await?;
    let started = Utc::now();
    let result = async {
        let pass = if settings.encrypt {
            match passphrase.filter(|p| !p.is_empty()) {
                Some(p) => Some(p),
                None => {
                    let sealed = repo::setting::get(&s.foreman_db.pool, svc::SETTING_PASSPHRASE)
                        .await?
                        .filter(|v| !v.is_empty())
                        .ok_or_else(|| {
                            AppError::InvalidInput(
                                "Encryption is on but no passphrase is saved.".into(),
                            )
                        })?;
                    Some(s.secrets.open(&sealed)?)
                }
            }
        } else {
            None
        };
        let entry = svc::create(
            &sources(s),
            &svc::BackupOptions {
                dest_dir: PathBuf::from(&settings.dir),
                include_library: settings.include_library,
                passphrase: pass,
                kind,
            },
        )
        .await?;
        let dir = PathBuf::from(&settings.dir);
        let keep = settings.retention as usize;
        let pruned = tokio::task::spawn_blocking(move || svc::prune(&dir, keep))
            .await
            .unwrap_or_default();
        Ok::<_, AppError>((entry, pruned))
    }
    .await;

    let run = match result {
        Ok((entry, pruned)) => {
            info!(file = %entry.path, bytes = entry.size, pruned = pruned.len(), "backup written");
            LastRun {
                at: started.to_rfc3339(),
                kind,
                ok: true,
                file: Some(entry.path),
                size: Some(entry.size),
                error: None,
                pruned,
            }
        }
        Err(e) => {
            warn!("backup failed: {e}");
            LastRun {
                at: started.to_rfc3339(),
                kind,
                ok: false,
                file: None,
                size: None,
                error: Some(e.to_string()),
                pruned: Vec::new(),
            }
        }
    };
    if let Ok(json) = serde_json::to_string(&run) {
        repo::setting::set(&s.foreman_db.pool, svc::SETTING_LAST, &json).await?;
    }
    let pool = &s.foreman_db.pool;
    if run.ok {
        if let Some(id) = repo::attention::clear_open(pool, None, svc::ATTENTION_KIND).await? {
            let _ = s.bus_tx.send(ServerEvent::AttentionCleared {
                item_id: id,
                printer_id: None,
            });
        }
    } else {
        let body = format!(
            "{} · folder {}. Fix it in Settings → Backup, then Run now.",
            run.error.as_deref().unwrap_or("unknown error"),
            settings.dir
        );
        raise(s, svc::ATTENTION_KIND, "Backup failed", &body).await;
    }
    Ok(run)
}

async fn raise(s: &AppState, kind: &str, title: &str, body: &str) {
    match repo::attention::raise(&s.foreman_db.pool, None, kind, title, Some(body)).await {
        Ok(r) => {
            let _ = s.bus_tx.send(ServerEvent::attention_raised(
                r.created,
                AttentionItemEvent {
                    id: r.item.id,
                    printer_id: None,
                    kind: kind.to_owned(),
                    title: r.item.title,
                    body: r.item.body,
                },
            ));
        }
        Err(e) => warn!("raise attention {kind}: {e}"),
    }
}

/// Startup note about a restore that was applied (or failed) before the DB opened.
pub async fn report_restore(s: &AppState, outcome: Result<svc::Applied, String>) {
    match outcome {
        Ok(a) => {
            let undo = a
                .snapshot
                .map(|p| {
                    format!(
                        " The state before it is saved as {}; restore that file to undo.",
                        p.display()
                    )
                })
                .unwrap_or_default();
            let body = format!(
                "Backup from {} (Foreman {}) is now live.{undo}",
                a.manifest.created_at, a.manifest.foreman_version
            );
            raise(s, "backup_restored", "Backup restored", &body).await;
        }
        Err(e) => raise(s, "backup_restore_failed", "Restore failed", &e).await,
    }
}

/// Check every minute whether a scheduled backup is due (and once at start, so a
/// slot missed while the server was off runs as soon as it is back).
pub fn spawn_schedule(s: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            let Ok(settings) = load_settings(&s).await else {
                continue;
            };
            let last = last_run(&s)
                .await
                .and_then(|r| DateTime::parse_from_rfc3339(&r.at).ok())
                .map(|d| d.with_timezone(&Utc));
            if svc::is_due(&settings, last, Local::now()) {
                info!("scheduled backup due");
                if let Err(e) = run_backup(&s, BackupKind::Scheduled, None).await {
                    warn!("scheduled backup skipped: {e}");
                }
            }
        }
    });
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct BackupFileRequest {
    pub path: String,
    #[ts(optional)]
    pub passphrase: Option<String>,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct InspectResult {
    pub manifest: Manifest,
    pub current: svc::Current,
    /// Counts of the live data, to compare with the backup's.
    pub current_counts: BackupCounts,
    /// Why this backup cannot be restored here, if it cannot.
    pub problem: Option<String>,
}

async fn live_counts(s: &AppState) -> BackupCounts {
    let src = sources(s);
    let mut c = svc::foreman_facts(&src.data_dir.join("foreman.db"))
        .await
        .map(|(_, c)| c)
        .unwrap_or_default();
    if let Some(sm) = &src.spoolman_db {
        let _ = svc::spoolman_facts(sm, &mut c).await;
    }
    c
}

pub async fn inspect(
    State(s): State<AppState>,
    Json(b): Json<BackupFileRequest>,
) -> ApiResult<Json<InspectResult>> {
    let manifest = svc::inspect(&PathBuf::from(&b.path), b.passphrase.as_deref()).await?;
    let problem = svc::check_compatible(
        manifest.format,
        &manifest.foreman_version,
        manifest.foreman_schema,
        manifest.spoolman_revision.as_deref(),
    )
    .err();
    Ok(Json(InspectResult {
        manifest,
        current: svc::current(),
        current_counts: live_counts(&s).await,
        problem,
    }))
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct RestoreStarted {
    pub manifest: Manifest,
    pub restarting: bool,
}

/// Stage the backup, answer, then restart: the swap happens on the next start before
/// any database is opened, after the current state is saved as a pre-restore backup.
pub async fn restore(
    State(s): State<AppState>,
    Json(b): Json<BackupFileRequest>,
) -> ApiResult<(StatusCode, Json<RestoreStarted>)> {
    let Ok(lock) = BUSY.try_lock() else {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "A backup or restore is already running.",
        ));
    };
    let manifest = svc::stage(
        &s.data_dir,
        &PathBuf::from(&b.path),
        b.passphrase.as_deref(),
    )
    .await?;
    info!(file = %b.path, created = %manifest.created_at, "restore staged; restarting");
    drop(lock);
    if !cfg!(test) {
        let state = s.clone();
        tokio::spawn(async move {
            // Let the response go out, then stop writing and restart.
            tokio::time::sleep(Duration::from_millis(500)).await;
            let _guard = BUSY.lock().await;
            state.foreman_db.pool.close().await;
            if let Some(sm) = state.spoolman.get() {
                sm.db.pool.close().await;
            }
            svc::restart_process();
        });
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(RestoreStarted {
            manifest,
            restarting: !cfg!(test),
        }),
    ))
}
