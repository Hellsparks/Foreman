//! First-run setup (task 6.4, W1).
//!
//! - `GET  /api/setup` — wizard state + getting-started checklist
//! - `POST /api/setup/step` — `{step}`: move between steps (validated)
//! - `POST /api/setup/skip` — leave the wizard; the empty Fleet page keeps the checklist
//! - `POST /api/setup/restart` — run the wizard again (Settings)
//! - `POST /api/setup/data/fresh` — start with the built-in Spoolman as it is
//! - `POST /api/setup/data/detect-path` — `{path}` of a Spoolman SQLite file on this machine
//! - `POST /api/setup/data/detect-upload` — the same file, uploaded (multipart `file`)
//! - `POST /api/setup/data/detect-url` — `{url}` of a running Spoolman: read it once
//! - `POST /api/setup/data/import` — `{replace_existing}`: copy the detected data in
//! - `POST /api/setup/data/discard` — forget the detected database
//! - `POST /api/setup/discover` — scan the LAN (sample printers when drivers are off)
//! - `POST /api/setup/probe` — check one printer + its credentials
//! - `POST /api/setup/printers` — validate, probe and add one printer
//! - `POST /api/setup/slicer-key` — make the slicer API key (secret in this response only)
//! - `POST /api/setup/backup` — `{enable, dir?}`: the 6.2 default schedule (daily 03:00)
//! - `POST /api/setup/finish` — wizard done

use std::path::PathBuf;
use std::time::Duration;

use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use foreman_core::{
    auth as core_auth,
    domain::{ApiKey, CreatePrinter, FirmwareKind, Printer},
    repo,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spoolman_rs::db::{extra_field, import as sm_import};
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};
use ts_rs::TS;

use super::ssh::{ApiError, ApiResult};
use crate::{
    auth::{AuthCtx, AuthKind},
    backup::Schedule,
    discovery::{self, FoundPrinter, ProbeRequest, ProbeResult, Readiness, ScanResult},
    setup::{
        self, Checklist, DataSource, DataSourceKind, ImportCounts, PendingImport, SetupStep,
        WizardState, SLICER_KEY_NAME,
    },
    state::AppState,
};

/// Largest Spoolman file accepted by upload (real ones are a few MB).
pub const UPLOAD_LIMIT: usize = 512 * 1024 * 1024;

/// One import or scan at a time.
static BUSY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, msg)
}

fn coded(status: StatusCode, code: &'static str, msg: impl Into<String>) -> ApiError {
    ApiError {
        status,
        message: msg.into(),
        code: Some(code),
    }
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct SetupOverview {
    pub state: WizardState,
    /// Send the user to the wizard after sign-in.
    pub needs_wizard: bool,
    pub checklist: Checklist,
    /// What the built-in Spoolman holds now.
    pub current: ImportCounts,
    /// The built-in Spoolman is running (imports need it).
    pub spoolman_running: bool,
    /// Port Moonraker / slicers use for the Spoolman API.
    #[ts(type = "number | null")]
    pub spoolman_port: Option<u16>,
    /// `false` = sample printers in scans, no connections.
    pub drivers_enabled: bool,
    /// The 6.2 default schedule the finish step offers.
    pub backup_default_dir: String,
}

async fn overview(s: &AppState) -> ApiResult<SetupOverview> {
    let ws = setup::load(s).await?;
    let checklist = setup::checklist(s, &ws).await?;
    Ok(SetupOverview {
        needs_wizard: ws.step != SetupStep::Done,
        checklist,
        current: setup::current_counts(s).await,
        spoolman_running: s.spoolman.get().is_some(),
        spoolman_port: s.spoolman.get().map(|sm| sm.config.port),
        drivers_enabled: s.drivers_enabled,
        backup_default_dir: crate::api::backup::load_settings(s).await?.dir,
        state: ws,
    })
}

pub async fn get(State(s): State<AppState>) -> ApiResult<Json<SetupOverview>> {
    Ok(Json(overview(&s).await?))
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct StepRequest {
    pub step: SetupStep,
}

/// Rules for moving between steps; the data step must be settled before printers.
pub fn check_step(ws: &WizardState, to: SetupStep, printers: usize) -> Result<WizardState, String> {
    let mut next = ws.clone();
    match to {
        SetupStep::Data => {}
        SetupStep::Printers => {
            if ws.data_source.is_none() {
                return Err("Choose where filament data comes from first.".into());
            }
        }
        SetupStep::Finish => {
            if ws.data_source.is_none() {
                return Err("Choose where filament data comes from first.".into());
            }
            next.printers_skipped = printers == 0;
        }
        SetupStep::Done => return Err("Use Finish to complete setup.".into()),
    }
    next.step = to;
    Ok(next)
}

pub async fn step(
    State(s): State<AppState>,
    Json(b): Json<StepRequest>,
) -> ApiResult<Json<SetupOverview>> {
    let ws = setup::load(&s).await?;
    let printers = repo::printer::list(&s.foreman_db.pool).await?.len();
    let next = check_step(&ws, b.step, printers).map_err(bad)?;
    setup::save(&s, &next).await?;
    Ok(Json(overview(&s).await?))
}

pub async fn skip(State(s): State<AppState>) -> ApiResult<Json<SetupOverview>> {
    let mut ws = setup::load(&s).await?;
    ws.step = SetupStep::Done;
    ws.skipped = true;
    ws.finished_at = Some(Utc::now().to_rfc3339());
    setup::save(&s, &ws).await?;
    info!("setup: skipped");
    Ok(Json(overview(&s).await?))
}

pub async fn restart(State(s): State<AppState>) -> ApiResult<Json<SetupOverview>> {
    let mut ws = setup::load(&s).await?;
    ws.step = SetupStep::Data;
    ws.skipped = false;
    ws.finished_at = None;
    setup::save(&s, &ws).await?;
    info!("setup: wizard restarted");
    Ok(Json(overview(&s).await?))
}

pub async fn finish(State(s): State<AppState>) -> ApiResult<Json<SetupOverview>> {
    let mut ws = setup::load(&s).await?;
    if ws.data_source.is_none() {
        return Err(bad("Choose where filament data comes from first."));
    }
    ws.step = SetupStep::Done;
    ws.finished_at = Some(Utc::now().to_rfc3339());
    setup::save(&s, &ws).await?;
    info!("setup: finished");
    Ok(Json(overview(&s).await?))
}

// ---------------------------------------------------------------------------
// Data source
// ---------------------------------------------------------------------------

/// The data step is done: move on to printers.
async fn settle(s: &AppState, ds: DataSource) -> ApiResult<SetupOverview> {
    let mut ws = setup::load(s).await?;
    ws.data_source = Some(ds);
    ws.pending = None;
    if ws.step == SetupStep::Data {
        ws.step = SetupStep::Printers;
    }
    setup::save(s, &ws).await?;
    overview(s).await
}

pub async fn data_fresh(State(s): State<AppState>) -> ApiResult<Json<SetupOverview>> {
    setup::clear_dir(&setup::work_dir(&s, "staged"));
    let counts = setup::current_counts(&s).await;
    let ds = DataSource {
        kind: DataSourceKind::Fresh,
        label: "Fresh start".into(),
        revision: None,
        counts,
        at: Utc::now().to_rfc3339(),
    };
    Ok(Json(settle(&s, ds).await?))
}

fn need_spoolman(s: &AppState) -> ApiResult<&crate::spools::Spoolman> {
    s.spoolman.get().ok_or_else(|| {
        coded(
            StatusCode::CONFLICT,
            "spoolman_off",
            "The built-in Spoolman is off (FOREMAN__SPOOLMAN_ENABLED=false), so there is nowhere to import into.",
        )
    })
}

/// Inspect the staged file and remember the result.
async fn record_detection(
    s: &AppState,
    kind: DataSourceKind,
    label: String,
    engine: String,
) -> ApiResult<PendingImport> {
    let staged = setup::staged_db(s);
    let insp = sm_import::inspect_sqlite(&staged)
        .await
        .map_err(|e| bad(e.to_string()))?;
    let pending = setup::pending_from(kind, label, engine, &insp);
    info!(
        revision = ?pending.revision,
        status = %pending.status,
        spools = pending.counts.spools,
        "setup: Spoolman database detected"
    );
    let mut ws = setup::load(s).await?;
    ws.pending = Some(pending.clone());
    setup::save(s, &ws).await?;
    Ok(pending)
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct DetectPathRequest {
    pub path: String,
}

pub async fn detect_path(
    State(s): State<AppState>,
    Json(b): Json<DetectPathRequest>,
) -> ApiResult<Json<PendingImport>> {
    need_spoolman(&s)?;
    let _g = BUSY.lock().await;
    let src = PathBuf::from(b.path.trim());
    if !src.is_absolute() {
        return Err(bad(
            "Use the full path to the file, e.g. /home/pi/.local/share/spoolman/spoolman.db.",
        ));
    }
    if !src.is_file() {
        return Err(bad(format!("No file at {}.", src.display())));
    }
    let staged = setup::staged_db(&s);
    let dir = setup::work_dir(&s, "staged");
    setup::clear_dir(&dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| bad(format!("Cannot create {}: {e}", dir.display())))?;
    tokio::fs::copy(&src, &staged)
        .await
        .map_err(|e| bad(format!("Cannot read {}: {e}", src.display())))?;
    let label = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| src.display().to_string());
    Ok(Json(
        record_detection(&s, DataSourceKind::SpoolmanFile, label, "sqlite".into()).await?,
    ))
}

pub async fn detect_upload(
    State(s): State<AppState>,
    mut mp: Multipart,
) -> ApiResult<Json<PendingImport>> {
    need_spoolman(&s)?;
    let _g = BUSY.lock().await;
    let dir = setup::work_dir(&s, "staged");
    setup::clear_dir(&dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| bad(format!("Cannot create {}: {e}", dir.display())))?;
    let staged = setup::staged_db(&s);
    let mut label = None;
    while let Some(mut field) = mp
        .next_field()
        .await
        .map_err(|e| bad(format!("Upload failed: {e}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        label = Some(field.file_name().unwrap_or("spoolman.db").to_owned());
        let mut out = tokio::fs::File::create(&staged)
            .await
            .map_err(|e| bad(format!("Cannot write the upload: {e}")))?;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| bad(format!("Upload failed: {e}")))?
        {
            out.write_all(&chunk)
                .await
                .map_err(|e| bad(format!("Cannot write the upload: {e}")))?;
        }
        out.flush().await.ok();
    }
    let label = label.ok_or_else(|| bad("Pick a Spoolman database file."))?;
    Ok(Json(
        record_detection(&s, DataSourceKind::SpoolmanFile, label, "sqlite".into()).await?,
    ))
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct DetectUrlRequest {
    pub url: String,
}

/// `http://host:7912/`, `host:7912`, `.../api/v1` → `http://host:7912`.
pub fn spoolman_base(input: &str) -> Option<String> {
    let mut t = input.trim().trim_end_matches('/').to_owned();
    if t.is_empty() || t.contains(char::is_whitespace) {
        return None;
    }
    if !t.contains("://") {
        t = format!("http://{t}");
    }
    let t = t
        .trim_end_matches("/api/v1")
        .trim_end_matches('/')
        .to_owned();
    url::Url::parse(&t).ok()?;
    Some(t)
}

async fn fetch_json(http: &reqwest::Client, url: &str) -> Result<Value, String> {
    let resp = http
        .get(url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "timed out".to_string()
            } else if e.is_connect() {
                "nothing answered".to_string()
            } else {
                e.to_string()
            }
        })?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("{status}"));
    }
    serde_json::from_str(&text).map_err(|_| "the answer was not JSON".to_string())
}

pub async fn detect_url(
    State(s): State<AppState>,
    Json(b): Json<DetectUrlRequest>,
) -> ApiResult<Json<PendingImport>> {
    need_spoolman(&s)?;
    let _g = BUSY.lock().await;
    let base = spoolman_base(&b.url)
        .ok_or_else(|| bad("Enter the Spoolman address, e.g. http://192.168.1.20:7912."))?;
    let http = &s.http_client;
    let info = fetch_json(http, &format!("{base}/api/v1/info"))
        .await
        .map_err(|e| bad(format!("No Spoolman at {base} ({e}).")))?;
    let version = info
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_owned();
    let get_list = |path: &'static str| {
        let url = format!("{base}{path}");
        async move {
            match fetch_json(http, &url).await {
                Ok(Value::Array(v)) => Ok(v),
                Ok(_) => Err(format!("{url}: expected a list")),
                Err(e) => Err(format!("{url}: {e}")),
            }
        }
    };
    let vendors = get_list("/api/v1/vendor").await.map_err(bad)?;
    let filaments = get_list("/api/v1/filament").await.map_err(bad)?;
    let spools = get_list("/api/v1/spool?allow_archived=true")
        .await
        .map_err(bad)?;
    let settings = match fetch_json(http, &format!("{base}/api/v1/setting/")).await {
        Ok(Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    let dump = sm_import::ApiDump {
        vendors,
        filaments,
        spools,
        settings,
    };
    let dir = setup::work_dir(&s, "staged");
    setup::clear_dir(&dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| bad(format!("Cannot create {}: {e}", dir.display())))?;
    sm_import::build_from_api(&dir, &dump)
        .await
        .map_err(|e| bad(format!("Could not copy that Spoolman's data: {e}")))?;
    Ok(Json(
        record_detection(
            &s,
            DataSourceKind::SpoolmanUrl,
            base.clone(),
            format!("Spoolman {version}"),
        )
        .await?,
    ))
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct ImportRequest {
    /// Required when the built-in Spoolman already has vendors, filaments or spools.
    #[serde(default)]
    pub replace_existing: bool,
}

pub async fn import(
    State(s): State<AppState>,
    Json(b): Json<ImportRequest>,
) -> ApiResult<Json<SetupOverview>> {
    let sm = need_spoolman(&s)?;
    let _g = BUSY.lock().await;
    let ws = setup::load(&s).await?;
    let pending = ws
        .pending
        .clone()
        .ok_or_else(|| bad("Detect a Spoolman database first."))?;
    if !pending.accepted {
        return Err(coded(
            StatusCode::UNPROCESSABLE_ENTITY,
            "refused",
            pending
                .refusal
                .unwrap_or_else(|| "This database cannot be imported.".into()),
        ));
    }
    let staged = setup::staged_db(&s);
    if !staged.is_file() {
        return Err(bad("The detected file is gone; detect it again."));
    }
    let current = setup::current_counts(&s).await;
    if !current.is_empty() && !b.replace_existing {
        return Err(coded(
            StatusCode::CONFLICT,
            "would_replace",
            format!(
                "Foreman already holds {}, {} and {}. Importing replaces them.",
                setup::plural(current.spools, "spool", "spools"),
                setup::plural(current.filaments, "filament", "filaments"),
                setup::plural(current.vendors, "vendor", "vendors"),
            ),
        ));
    }
    // Migrate a second copy so the staged file still shows what was detected.
    let work = setup::work_dir(&s, "migrate");
    setup::clear_dir(&work);
    std::fs::create_dir_all(&work)
        .map_err(|e| bad(format!("Cannot create {}: {e}", work.display())))?;
    let copy = work.join("spoolman.db");
    tokio::fs::copy(&staged, &copy)
        .await
        .map_err(|e| bad(format!("Cannot copy the staged file: {e}")))?;
    let result = async {
        sm_import::migrate_to_head(&work).await.map_err(|e| {
            bad(format!(
                "Migrating the copy to the current revision failed: {e}"
            ))
        })?;
        let counts = sm_import::replace_all_from(&sm.db, &copy)
            .await
            .map_err(|e| bad(format!("Import failed, nothing was changed: {e}")))?;
        if let Err(e) = extra_field::ensure_foreman_defaults(&sm.db.pool, &sm.db.engine).await {
            warn!("setup: Foreman extra fields after import: {e}");
        }
        Ok::<_, ApiError>(counts)
    }
    .await;
    setup::clear_dir(&work);
    let counts: ImportCounts = result?.into();
    setup::clear_dir(&setup::work_dir(&s, "staged"));
    info!(
        spools = counts.spools,
        filaments = counts.filaments,
        vendors = counts.vendors,
        from = %pending.label,
        "setup: Spoolman data imported"
    );
    let ds = DataSource {
        kind: pending.kind,
        label: pending.label,
        revision: pending.revision,
        counts,
        at: Utc::now().to_rfc3339(),
    };
    Ok(Json(settle(&s, ds).await?))
}

pub async fn discard(State(s): State<AppState>) -> ApiResult<Json<SetupOverview>> {
    setup::clear_dir(&setup::work_dir(&s, "staged"));
    let mut ws = setup::load(&s).await?;
    ws.pending = None;
    setup::save(&s, &ws).await?;
    Ok(Json(overview(&s).await?))
}

// ---------------------------------------------------------------------------
// Printers
// ---------------------------------------------------------------------------

pub async fn discover(State(s): State<AppState>) -> ApiResult<Json<ScanResult>> {
    let existing = repo::printer::list(&s.foreman_db.pool).await?;
    if !s.drivers_enabled {
        return Ok(Json(discovery::mock_scan(&existing)));
    }
    let _g = BUSY.lock().await;
    Ok(Json(
        discovery::scan(&s.http_client, &existing, Duration::from_secs(4)).await,
    ))
}

pub async fn probe(
    State(s): State<AppState>,
    Json(b): Json<ProbeRequest>,
) -> ApiResult<Json<ProbeResult>> {
    let mut r = if s.drivers_enabled {
        discovery::probe(&s.http_client, &b).await.map_err(bad)?
    } else {
        discovery::mock_probe(&b)
    };
    let existing = repo::printer::list(&s.foreman_db.pool).await?;
    discovery::mark_added(std::slice::from_mut(&mut r.printer), &existing);
    Ok(Json(r))
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct AddPrinterRequest {
    pub name: String,
    pub firmware: FirmwareKind,
    /// IP, hostname or URL.
    pub host: String,
    #[serde(default)]
    #[ts(optional)]
    pub api_key: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub serial: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub access_code: Option<String>,
    /// Add even though the probe failed (printer switched off).
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct AddPrinterResult {
    pub printer: Printer,
    pub probe: FoundPrinter,
}

fn opt(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// Field rules for adding a printer (before any network probe).
pub fn validate_add(b: &AddPrinterRequest, existing: &[Printer]) -> Result<(), ApiError> {
    let name = b.name.trim();
    if name.is_empty() {
        return Err(bad("Give the printer a name."));
    }
    if name.chars().count() > 64 {
        return Err(bad("Keep the name under 64 characters."));
    }
    if let Some(p) = existing.iter().find(|p| p.name.eq_ignore_ascii_case(name)) {
        return Err(coded(
            StatusCode::CONFLICT,
            "duplicate_name",
            format!("A printer called {} already exists.", p.name),
        ));
    }
    if discovery::split_host(&b.host).is_none() {
        return Err(bad("Enter the printer's IP address or hostname."));
    }
    match b.firmware {
        FirmwareKind::Moonraker => {}
        FirmwareKind::Bambu => {
            let serial = opt(&b.serial).unwrap_or_default();
            if !discovery::valid_bambu_serial(&serial) {
                return Err(bad(
                    "Enter the 15-character serial (printer screen → Settings → Device).",
                ));
            }
            let code = opt(&b.access_code).unwrap_or_default();
            if !discovery::valid_access_code(&code) {
                return Err(bad(
                    "Enter the 8-character access code (printer screen → Settings → Network / LAN).",
                ));
            }
        }
        _ => return Err(bad("Setup adds Klipper (Moonraker) and Bambu printers.")),
    }
    Ok(())
}

pub async fn add_printer(
    State(s): State<AppState>,
    Json(b): Json<AddPrinterRequest>,
) -> ApiResult<(StatusCode, Json<AddPrinterResult>)> {
    let pool = &s.foreman_db.pool;
    let existing = repo::printer::list(pool).await?;
    validate_add(&b, &existing)?;
    let req = ProbeRequest {
        firmware: b.firmware.clone(),
        host: b.host.clone(),
        api_key: opt(&b.api_key),
        serial: opt(&b.serial),
        access_code: opt(&b.access_code),
    };
    let mut probe = if s.drivers_enabled {
        discovery::probe(&s.http_client, &req).await.map_err(bad)?
    } else {
        discovery::mock_probe(&req)
    };
    discovery::mark_added(std::slice::from_mut(&mut probe.printer), &existing);
    if let Some(id) = probe.printer.added_as {
        let name = existing
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        return Err(coded(
            StatusCode::CONFLICT,
            "already_added",
            format!("That printer is already added as {name}."),
        ));
    }
    if !probe.ok {
        // Wrong credentials never go in; an unreachable printer may (force).
        let credentials_refused = matches!(
            probe.printer.needs,
            Readiness::ApiKey | Readiness::AccessCode
        );
        if credentials_refused || !b.force {
            let why = probe
                .printer
                .checks
                .iter()
                .filter(|c| !c.ok)
                .map(|c| format!("{}: {}", c.label, c.detail))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(coded(
                StatusCode::UNPROCESSABLE_ENTITY,
                match probe.printer.needs {
                    Readiness::ApiKey => "api_key",
                    Readiness::AccessCode => "access_code",
                    _ => "unreachable",
                },
                if why.is_empty() {
                    "The printer did not answer.".to_string()
                } else {
                    why
                },
            ));
        }
    }
    let url = match b.firmware {
        FirmwareKind::Moonraker => probe.printer.url.clone(),
        _ => discovery::split_host(&b.host)
            .map(|(h, _, _)| h)
            .unwrap_or_else(|| b.host.trim().to_owned()),
    };
    let printer = repo::printer::create(
        pool,
        CreatePrinter {
            name: b.name.trim().to_owned(),
            firmware: b.firmware.clone(),
            primary_url: Some(url),
            api_key: opt(&b.api_key),
            serial_number: opt(&b.serial),
            access_code: opt(&b.access_code),
            tags: Vec::new(),
            hardened_nozzle: false,
            slicer_profiles: Vec::new(),
            sort_order: None,
            fail_after_checks: None,
            return_after_secs: None,
        },
    )
    .await?;
    if s.drivers_enabled {
        if let Err(e) = s.driver_manager.start_driver(&printer).await {
            warn!(
                printer_id = printer.id,
                "setup: driver start failed (printer added): {e}"
            );
        }
    }
    info!(printer_id = printer.id, name = %printer.name, "setup: printer added");
    Ok((
        StatusCode::CREATED,
        Json(AddPrinterResult {
            printer,
            probe: probe.printer,
        }),
    ))
}

// ---------------------------------------------------------------------------
// Slicer key + backups
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct SlicerKeyResult {
    pub key: ApiKey,
    /// Shown once; Foreman keeps only a hash.
    pub secret: String,
    pub urls: Vec<String>,
}

pub async fn slicer_key(
    State(s): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<(StatusCode, Json<SlicerKeyResult>)> {
    if matches!(ctx.kind, AuthKind::ApiKey { .. }) {
        return Err(coded(
            StatusCode::FORBIDDEN,
            "session_required",
            "Sign in to do this (API keys can't).",
        ));
    }
    let _g = BUSY.lock().await;
    let pool = &s.foreman_db.pool;
    if core_auth::list_api_keys(pool)
        .await?
        .iter()
        .any(|k| k.name == SLICER_KEY_NAME)
    {
        return Err(coded(
            StatusCode::CONFLICT,
            "exists",
            "The slicer key was made already and its secret was shown once. \
             Revoke it in Settings → Security to make a new one.",
        ));
    }
    let (key, secret) = core_auth::create_api_key(pool, SLICER_KEY_NAME).await?;
    info!(name = %key.name, "setup: slicer API key created");
    Ok((
        StatusCode::CREATED,
        Json(SlicerKeyResult {
            key,
            secret,
            urls: setup::slicer_urls(&s),
        }),
    ))
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct SetupBackupRequest {
    pub enable: bool,
    /// Folder; empty or missing keeps the saved one (default `~/foreman-backups`).
    #[serde(default)]
    #[ts(optional)]
    pub dir: Option<String>,
}

pub async fn backup(
    State(s): State<AppState>,
    Json(b): Json<SetupBackupRequest>,
) -> ApiResult<Json<SetupOverview>> {
    let mut settings = crate::api::backup::load_settings(&s).await?;
    if let Some(d) = b.dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        settings.dir = d.to_owned();
    }
    if b.enable {
        crate::api::backup::check_dir_writable(PathBuf::from(&settings.dir))
            .await
            .map_err(bad)?;
        // The 6.2 default schedule: daily at the saved time (03:00), keep the saved count (7).
        settings.schedule = Schedule::Daily;
    } else {
        settings.schedule = Schedule::Off;
    }
    crate::api::backup::save_settings(&s, settings, None).await?;
    info!(enable = b.enable, "setup: backup schedule set");
    Ok(Json(overview(&s).await?))
}
