//! Calibration (task 5.4, W10): skew per printer, shrinkage per filament.
//!
//! Skew: Foreman computes the factor with Klipper's formula and shows the exact
//! commands. Sending them is the owner's action (the Apply button), refused
//! unless the printer is a Klipper printer that is known to be idle, and only
//! with `confirm_restart` because SAVE_CONFIG restarts Klipper.
//!
//! Shrinkage: Calistar maths, written into the filament's `orcaslicer_config`
//! extra field as `shrinkage_xy` / `shrinkage_z` (the keys Marathon used) and
//! logged in `shrinkage_calibration`.

use foreman_core::{
    calibration::{self as cal, PlaneMeasure, PlaneSkew, ShrinkageInput, ShrinkageResult},
    domain::{FirmwareKind, Printer},
    driver::PrinterCommand,
    error::AppError,
    events::{PrinterState, PrinterStatus},
    repo::{
        self,
        calibration::{ShrinkageRecord, SkewRecord},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::SqlitePool;
use tracing::{info, warn};
use ts_rs::TS;

use crate::{driver_manager::DriverManager, spools::Spoolman};

// ---------------------------------------------------------------------------
// Skew
// ---------------------------------------------------------------------------

/// Why a printer can't take software skew correction, or `None` if it can.
pub fn skew_unsupported(p: &Printer) -> Option<String> {
    match p.firmware {
        FirmwareKind::Moonraker => None,
        _ => Some(format!(
            "{}: no software skew correction — only Klipper is supported. Square the frame \
             mechanically; shrinkage measured on this printer carries any skew error into the number.",
            p.name
        )),
    }
}

/// Error code the UI keys on for "printing / paused".
pub const CODE_PRINTING: &str = "printing";

/// Refuse sending skew commands unless the printer is Klipper and known idle.
/// Printing or paused → refused (SAVE_CONFIG would restart Klipper mid-job, and a
/// new SET_SKEW would bend the rest of the print). Unknown/offline/error → refused.
pub fn skew_send_allowed(p: &Printer, status: Option<&PrinterStatus>) -> Result<(), AppError> {
    if let Some(why) = skew_unsupported(p) {
        return Err(AppError::InvalidInput(why));
    }
    match status.map(|s| &s.state) {
        Some(PrinterState::Idle) => Ok(()),
        Some(PrinterState::Printing) | Some(PrinterState::Paused) => {
            Err(AppError::InvalidInput(format!(
                "{} is printing or paused — skew commands refused until the job ends",
                p.name
            )))
        }
        Some(PrinterState::Error) => Err(AppError::InvalidInput(format!(
            "{} is in an error state — fix that first",
            p.name
        ))),
        None | Some(PrinterState::Offline) => Err(AppError::InvalidInput(format!(
            "{} state unknown (offline?) — not sending",
            p.name
        ))),
    }
}

/// What the skew page needs per printer.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SkewPrinter {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub firmware: String,
    pub supported: bool,
    /// Set when not supported.
    pub unsupported: Option<String>,
    pub state: Option<PrinterState>,
    /// Newest applied measurement (the correction Foreman last saved on it).
    pub applied: Option<SkewRecord>,
    /// All measurements, newest first.
    pub history: Vec<SkewRecord>,
}

pub async fn skew_printers(
    pool: &SqlitePool,
    drivers: &DriverManager,
) -> Result<Vec<SkewPrinter>, AppError> {
    let printers = repo::printer::list(pool).await?;
    let all = repo::calibration::list_skew(pool, None).await?;
    let mut out = vec![];
    for p in printers {
        let history: Vec<SkewRecord> = all
            .iter()
            .filter(|r| r.printer_id == p.id)
            .cloned()
            .collect();
        let unsupported = skew_unsupported(&p);
        out.push(SkewPrinter {
            id: p.id,
            name: p.name.clone(),
            firmware: serde_json::to_value(&p.firmware)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            supported: unsupported.is_none(),
            unsupported,
            state: drivers.get_status(p.id).await.map(|s| s.state),
            applied: history.iter().find(|r| r.applied_at.is_some()).cloned(),
            history,
        });
    }
    Ok(out)
}

/// Computed skew + the exact commands Apply would send.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SkewPreview {
    pub planes: Vec<PlaneSkew>,
    pub commands: Vec<String>,
    /// Sent before printing the test piece.
    pub clear_command: String,
    pub profile: String,
}

pub fn preview(planes: &[PlaneMeasure]) -> Result<SkewPreview, AppError> {
    let planes = cal::compute_skew(planes)?;
    Ok(SkewPreview {
        commands: cal::apply_commands(&planes),
        clear_command: cal::CLEAR_COMMAND.to_owned(),
        profile: cal::SKEW_PROFILE.to_owned(),
        planes,
    })
}

/// Result of pressing Apply.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SkewApplied {
    pub record: SkewRecord,
    /// Commands sent, in order.
    pub sent: Vec<String>,
    /// Klipper restarting after SAVE_CONFIG (the request may drop as it restarts).
    pub restarting: bool,
}

/// Send SET_SKEW, SKEW_PROFILE SAVE=foreman, SAVE_CONFIG to the printer — only
/// ever from the owner's Apply button. The state is re-checked before each command.
pub async fn apply(
    pool: &SqlitePool,
    drivers: &DriverManager,
    printer_id: i64,
    record_id: i64,
    confirm_restart: bool,
    user: &str,
) -> Result<SkewApplied, AppError> {
    let printer = repo::printer::get(pool, printer_id).await?;
    let rec = repo::calibration::get_skew(pool, record_id).await?;
    if rec.printer_id != printer_id {
        return Err(AppError::InvalidInput(
            "that measurement belongs to another printer".into(),
        ));
    }
    if !confirm_restart {
        return Err(AppError::InvalidInput(
            "SAVE_CONFIG restarts Klipper — confirm the restart to apply".into(),
        ));
    }
    let commands = cal::apply_commands(&rec.planes);
    let mut sent = vec![];
    let mut restarting = false;
    for (i, c) in commands.iter().enumerate() {
        skew_send_allowed(&printer, drivers.get_status(printer_id).await.as_ref())?;
        let last = i + 1 == commands.len();
        let r = drivers
            .send_command_as(
                printer_id,
                PrinterCommand::Gcode { command: c.clone() },
                None,
                Some(user),
            )
            .await;
        match r {
            Ok(()) => {}
            // SAVE_CONFIG restarts Klipper; the HTTP call can fail as it goes down.
            Err(AppError::Driver(e)) if last => {
                warn!(
                    printer_id,
                    "SAVE_CONFIG returned an error (Klipper restarting?): {e}"
                );
            }
            Err(e) => {
                return Err(AppError::Driver(format!(
                    "`{c}` failed after sending {sent:?}: {e}"
                )))
            }
        }
        sent.push(c.clone());
        restarting = last;
    }
    info!(printer_id, record_id, user, "skew applied: {sent:?}");
    let record = repo::calibration::mark_skew_applied(pool, record_id, user, None).await?;
    Ok(SkewApplied {
        record,
        sent,
        restarting,
    })
}

/// SET_SKEW CLEAR=1 before printing the test piece (owner's button).
pub async fn clear(
    pool: &SqlitePool,
    drivers: &DriverManager,
    printer_id: i64,
    user: &str,
) -> Result<String, AppError> {
    let printer = repo::printer::get(pool, printer_id).await?;
    skew_send_allowed(&printer, drivers.get_status(printer_id).await.as_ref())?;
    drivers
        .send_command_as(
            printer_id,
            PrinterCommand::Gcode {
                command: cal::CLEAR_COMMAND.into(),
            },
            None,
            Some(user),
        )
        .await?;
    Ok(cal::CLEAR_COMMAND.to_owned())
}

// ---------------------------------------------------------------------------
// Shrinkage
// ---------------------------------------------------------------------------

pub const ORCA_FIELD: &str = "orcaslicer_config";

/// The `orcaslicer_config` text from a filament's extra map. Spoolman stores a
/// text field as a JSON string literal holding the config's JSON object.
pub fn orca_text(extra: &Map<String, Value>) -> Option<String> {
    match extra.get(ORCA_FIELD)? {
        Value::String(raw) => match serde_json::from_str::<Value>(raw) {
            Ok(Value::String(inner)) => Some(inner),
            Ok(v @ Value::Object(_)) => Some(v.to_string()),
            _ => Some(raw.clone()),
        },
        Value::Object(o) => Some(Value::Object(o.clone()).to_string()),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ShrinkFilament {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub vendor: Option<String>,
    pub material: Option<String>,
    /// `RRGGBB` values, first = main colour.
    pub colours: Vec<String>,
    pub shrinkage_xy: Option<f64>,
    pub shrinkage_z: Option<f64>,
    pub history: Vec<ShrinkageRecord>,
}

pub async fn shrink_filaments(
    pool: &SqlitePool,
    sm: &Spoolman,
) -> Result<Vec<ShrinkFilament>, AppError> {
    let all = repo::calibration::list_shrinkage(pool, None).await?;
    let rows = crate::filaments::filaments(sm).await?;
    let raw = crate::filaments::all_filaments(sm).await?;
    Ok(rows
        .into_iter()
        .map(|f| {
            let extra = raw.iter().find(|r| r.id == f.id).map(|r| &r.extra);
            let (xy, z) = cal::read_orca_shrinkage(extra.and_then(orca_text).as_deref());
            ShrinkFilament {
                history: all
                    .iter()
                    .filter(|r| r.filament_id == f.id)
                    .cloned()
                    .collect(),
                id: f.id,
                name: f.name,
                vendor: f.vendor,
                material: f.material,
                colours: f.colours,
                shrinkage_xy: xy,
                shrinkage_z: z,
            }
        })
        .collect())
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct SaveShrinkage {
    pub input: ShrinkageInput,
    /// The printer the test piece was printed on.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub printer_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ShrinkSaved {
    pub result: ShrinkageResult,
    pub record: ShrinkageRecord,
    /// The `orcaslicer_config` text now on the filament.
    pub orcaslicer_config: String,
    /// The old value wasn't a JSON object and was replaced.
    pub replaced_invalid: bool,
}

/// Compute, merge into `orcaslicer_config`, write through spoolman-rs, log.
pub async fn save_shrinkage(
    pool: &SqlitePool,
    sm: &Spoolman,
    filament_id: i64,
    req: &SaveShrinkage,
    user: &str,
) -> Result<ShrinkSaved, AppError> {
    let result = cal::compute_shrinkage(&req.input)?;
    let what = format!("filament #{filament_id}");
    let f = spoolman_rs::db::filament::find_by_id(&sm.db.pool, filament_id)
        .await
        .map_err(|e| match e {
            spoolman_rs::error::SpoolmanError::NotFound(_) => AppError::NotFound(what.clone()),
            e => AppError::Internal(format!("spoolman: {e}")),
        })?;
    let (text, replaced) = cal::merge_orca_config(
        orca_text(&f.extra).as_deref(),
        result.shrinkage_xy,
        result.shrinkage_z,
    );
    let req_sm: spoolman_rs::models::UpdateFilamentRequest = serde_json::from_value(json!({
        "extra": { ORCA_FIELD: Value::String(text.clone()).to_string() }
    }))
    .map_err(|e| AppError::Internal(e.to_string()))?;
    let updated = spoolman_rs::db::filament::update(&sm.db.pool, filament_id, &req_sm)
        .await
        .map_err(|e| AppError::InvalidInput(format!("spoolman: {e}")))?;
    spoolman_rs::ws::broadcast(&sm.event_tx, "updated", "filament", updated.id, &updated);

    let printer = match req.printer_id {
        Some(id) => Some(repo::printer::get(pool, id).await?),
        None => None,
    };
    let corrected = match &printer {
        Some(p) => repo::calibration::list_skew(pool, Some(p.id))
            .await?
            .iter()
            .any(|r| r.applied_at.is_some()),
        None => false,
    };
    let record = repo::calibration::insert_shrinkage(
        pool,
        filament_id,
        &f.name
            .clone()
            .unwrap_or_else(|| format!("Filament {filament_id}")),
        printer.as_ref().map(|p| (p.id, p.name.as_str())),
        corrected,
        &req.input,
        result.shrinkage_xy,
        result.shrinkage_z,
        user,
    )
    .await?;
    info!(filament_id, xy = ?result.shrinkage_xy, z = ?result.shrinkage_z, "shrinkage saved");
    Ok(ShrinkSaved {
        result,
        record,
        orcaslicer_config: text,
        replaced_invalid: replaced,
    })
}
