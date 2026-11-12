//! First-run setup (task 6.4, W1): the wizard's saved state and the getting-started
//! checklist shared by the wizard's last step and the empty Fleet page.
//!
//! The state lives in the `setup.wizard` setting, so leaving halfway and coming back
//! (even after a restart) continues at the saved step. A detected-but-not-imported
//! Spoolman database waits in `<data_dir>/setup/staged/spoolman.db`.

use std::path::{Path, PathBuf};

use chrono::Utc;
use foreman_core::{error::AppError, repo};
use serde::{Deserialize, Serialize};
use spoolman_rs::db::import as sm_import;
use tracing::info;
use ts_rs::TS;

use crate::{backup::Schedule, state::AppState};

pub const SETTING_KEY: &str = "setup.wizard";
/// Name of the API key made by "Generate a key" on the slicer row.
pub const SLICER_KEY_NAME: &str = "Slicer (setup)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SetupStep {
    Data,
    Printers,
    Finish,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceKind {
    Fresh,
    SpoolmanFile,
    SpoolmanUrl,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ImportCounts {
    #[ts(type = "number")]
    pub vendors: i64,
    #[ts(type = "number")]
    pub filaments: i64,
    #[ts(type = "number")]
    pub spools: i64,
    #[ts(type = "number")]
    pub archived_spools: i64,
    #[ts(type = "number")]
    pub extra_fields: i64,
}

impl From<sm_import::Counts> for ImportCounts {
    fn from(c: sm_import::Counts) -> Self {
        Self {
            vendors: c.vendors,
            filaments: c.filaments,
            spools: c.spools,
            archived_spools: c.archived_spools,
            extra_fields: c.extra_fields,
        }
    }
}

impl ImportCounts {
    pub fn is_empty(&self) -> bool {
        self.vendors == 0 && self.filaments == 0 && self.spools == 0
    }
}

/// The data source the user settled on.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DataSource {
    pub kind: DataSourceKind,
    /// File name, URL, or "Fresh start".
    pub label: String,
    pub revision: Option<String>,
    pub counts: ImportCounts,
    pub at: String,
}

/// A detected Spoolman database waiting for "Import".
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PendingImport {
    pub kind: DataSourceKind,
    pub label: String,
    /// `sqlite` for a file, `spoolman <version>` for a URL.
    pub engine: String,
    pub revision: Option<String>,
    /// `head` | `older` | `unknown` | `missing`
    pub status: String,
    #[ts(type = "number")]
    pub steps_to_head: u32,
    pub head: String,
    pub accepted: bool,
    pub refusal: Option<String>,
    pub counts: ImportCounts,
    pub detected_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WizardState {
    pub step: SetupStep,
    pub data_source: Option<DataSource>,
    pub pending: Option<PendingImport>,
    /// Left the printer step with no printer added.
    pub printers_skipped: bool,
    /// "Skip setup" from the first step: the Fleet checklist takes over.
    pub skipped: bool,
    /// The server had data before setup existed: never sent to the wizard.
    pub existing_install: bool,
    pub started_at: String,
    pub finished_at: Option<String>,
}

impl WizardState {
    fn fresh() -> Self {
        Self {
            step: SetupStep::Data,
            data_source: None,
            pending: None,
            printers_skipped: false,
            skipped: false,
            existing_install: false,
            started_at: Utc::now().to_rfc3339(),
            finished_at: None,
        }
    }
}

/// Built-in Spoolman's current rows (zeros when it is off).
pub async fn current_counts(state: &AppState) -> ImportCounts {
    match state.spoolman.get() {
        Some(sm) => sm_import::counts(&sm.db)
            .await
            .map(Into::into)
            .unwrap_or_default(),
        None => ImportCounts::default(),
    }
}

/// Load the wizard state. The first call decides: a server that already has printers,
/// spools or API keys is an existing install (wizard done); otherwise the wizard starts.
pub async fn load(state: &AppState) -> Result<WizardState, AppError> {
    let pool = &state.foreman_db.pool;
    if let Some(v) = repo::setting::get(pool, SETTING_KEY).await? {
        if let Ok(s) = serde_json::from_str::<WizardState>(&v) {
            return Ok(s);
        }
    }
    let printers = repo::printer::list(pool).await?.len();
    let keys = foreman_core::auth::list_api_keys(pool).await?.len();
    let counts = current_counts(state).await;
    let mut s = WizardState::fresh();
    if printers > 0 || keys > 0 || !counts.is_empty() {
        s.step = SetupStep::Done;
        s.existing_install = true;
        s.finished_at = Some(Utc::now().to_rfc3339());
        info!(
            printers,
            keys,
            spools = counts.spools,
            "setup: existing install, wizard marked done"
        );
    }
    save(state, &s).await?;
    Ok(s)
}

pub async fn save(state: &AppState, s: &WizardState) -> Result<(), AppError> {
    let json = serde_json::to_string(s).map_err(|e| AppError::Internal(e.to_string()))?;
    repo::setting::set(&state.foreman_db.pool, SETTING_KEY, &json).await
}

/// `<data_dir>/setup/<name>`.
pub fn work_dir(state: &AppState, name: &str) -> PathBuf {
    state.data_dir.join("setup").join(name)
}

/// The staged copy of a detected database.
pub fn staged_db(state: &AppState) -> PathBuf {
    work_dir(state, "staged").join("spoolman.db")
}

/// Remove a work dir (ignore "not found").
pub fn clear_dir(dir: &Path) {
    if dir.exists() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Turn an inspection into what the UI shows.
pub fn pending_from(
    kind: DataSourceKind,
    label: String,
    engine: String,
    insp: &sm_import::Inspection,
) -> PendingImport {
    let status = match insp.status {
        sm_import::RevisionStatus::Head => "head",
        sm_import::RevisionStatus::Older => "older",
        sm_import::RevisionStatus::Unknown => "unknown",
        sm_import::RevisionStatus::Missing => "missing",
    };
    PendingImport {
        kind,
        label,
        engine,
        revision: insp.revision.clone(),
        status: status.into(),
        steps_to_head: insp.steps_to_head as u32,
        head: insp.head.clone(),
        accepted: insp.accepted(),
        refusal: insp.refusal(),
        counts: insp.counts.clone().into(),
        detected_at: Utc::now().to_rfc3339(),
    }
}

// ---------------------------------------------------------------------------
// Checklist
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChecklistItem {
    /// `server` | `filament` | `printers` | `slicer` | `backup`
    pub id: String,
    pub done: bool,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SlicerInfo {
    /// OctoPrint-compatible base URLs a slicer can use (`http://10.0.0.10:9090`).
    pub urls: Vec<String>,
    #[ts(type = "number")]
    pub keys: u32,
    /// The one-click key exists already (its secret was shown once, when it was made).
    pub setup_key_exists: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Checklist {
    pub items: Vec<ChecklistItem>,
    #[ts(type = "number")]
    pub done: u32,
    #[ts(type = "number")]
    pub total: u32,
    pub slicer: SlicerInfo,
}

pub fn plural(n: i64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub async fn checklist(state: &AppState, ws: &WizardState) -> Result<Checklist, AppError> {
    let pool = &state.foreman_db.pool;
    let counts = current_counts(state).await;
    let printers = repo::printer::list(pool).await?;
    let keys = foreman_core::auth::list_api_keys(pool).await?;
    let backup = crate::api::backup::load_settings(state).await?;

    let mut items = vec![ChecklistItem {
        id: "server".into(),
        done: true,
        title: "Server running".into(),
        detail: format!("Foreman {} on port {}", state.info.version, state.info.port),
    }];

    let spools = plural(counts.spools, "spool", "spools");
    let filament_done = ws.data_source.is_some() || !counts.is_empty();
    let filament_detail = match (&ws.data_source, state.spoolman.get().is_some()) {
        (_, false) => "Built-in Spoolman is off (FOREMAN__SPOOLMAN_ENABLED=false)".into(),
        (Some(ds), _) => match ds.kind {
            DataSourceKind::Fresh if counts.is_empty() => "Fresh start · no spools yet".into(),
            DataSourceKind::Fresh => format!("{spools} in the built-in Spoolman"),
            DataSourceKind::SpoolmanFile => {
                format!(
                    "{spools} found in existing Spoolman database ({})",
                    ds.label
                )
            }
            DataSourceKind::SpoolmanUrl => format!("{spools} copied from {}", ds.label),
        },
        (None, _) if !counts.is_empty() => format!("{spools} in the built-in Spoolman"),
        (None, _) => "Start fresh or bring an existing Spoolman database".into(),
    };
    items.push(ChecklistItem {
        id: "filament".into(),
        done: filament_done,
        title: if filament_done {
            "Filament inventory connected".into()
        } else {
            "Choose where filament data comes from".into()
        },
        detail: filament_detail,
    });

    let n = printers.len() as i64;
    items.push(ChecklistItem {
        id: "printers".into(),
        done: n > 0,
        title: if n > 0 {
            format!("{} added", plural(n, "printer", "printers"))
        } else {
            "Add your first printer".into()
        },
        detail: printers
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>()
            .join(", "),
    });

    let setup_key_exists = keys.iter().any(|k| k.name == SLICER_KEY_NAME);
    items.push(ChecklistItem {
        id: "slicer".into(),
        done: !keys.is_empty(),
        title: "Point your slicer at Foreman".into(),
        detail: if keys.is_empty() {
            "OctoPrint-compatible upload; needs an API key".into()
        } else {
            format!(
                "{} · {}",
                plural(keys.len() as i64, "API key", "API keys"),
                if keys.iter().any(|k| k.last_used_at.is_some()) {
                    "used by a slicer"
                } else {
                    "not used yet"
                }
            )
        },
    });

    let backup_on = backup.schedule != Schedule::Off;
    items.push(ChecklistItem {
        id: "backup".into(),
        done: backup_on,
        title: if backup_on {
            "Backups scheduled".into()
        } else {
            "Turn on backups".into()
        },
        detail: if backup_on {
            let when = match backup.schedule {
                Schedule::Daily => format!("Daily at {}", backup.time),
                _ => format!("Weekly at {}", backup.time),
            };
            format!("{when} · keep {} · {}", backup.retention, backup.dir)
        } else {
            "Off".into()
        },
    });

    let urls = slicer_urls(state);
    let done = items.iter().filter(|i| i.done).count() as u32;
    let total = items.len() as u32;
    Ok(Checklist {
        items,
        done,
        total,
        slicer: SlicerInfo {
            urls,
            keys: keys.len() as u32,
            setup_key_exists,
        },
    })
}

/// `http://<address>` for each way a slicer on the LAN can reach this server.
pub fn slicer_urls(state: &AppState) -> Vec<String> {
    crate::net::own_addresses(
        &state.info.bound_host(),
        state.info.port,
        &crate::net::interface_ips(),
        &state.info.hostname,
        state.info.magic_dns.as_deref(),
    )
    .into_iter()
    .map(|a| format!("http://{}", a.address))
    .collect()
}
