//! Loaded spools (task 4.1): Spoolman lookups, which spool is in which printer
//! slot, the filament guard's inputs, and load / unload.
//!
//! Klipper: Moonraker's active spool (its `[spoolman]` link), tool 0 only.
//! Bambu: AMS trays as the printer reports them, read-only (cloud mode).

use foreman_core::{
    compat::PrinterSnapshot,
    domain::{FirmwareKind, Printer},
    error::AppError,
    events::PrinterState,
    filament::{
        self, ams_slot_label, GuardInput, GuardWarning, JobNeed, LoadedSlot, SpoolFacts,
        BAMBU_READ_ONLY,
    },
    materials, repo,
};
use library_meta::MetaSource;
use serde_json::{Map, Value};
use spoolman_rs::models::SpoolResponse;
use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::driver_manager::DriverManager;

/// The built-in Spoolman's state.
pub type Spoolman = spoolman_rs::state::AppState;

// ---------------------------------------------------------------------------
// Spoolman → SpoolFacts
// ---------------------------------------------------------------------------

/// Spoolman stores extra values as JSON text: `"\"CF\""`, `"true"`.
fn extra(map: &Map<String, Value>, key: &str) -> Option<Value> {
    match map.get(key)? {
        Value::String(s) => serde_json::from_str(s)
            .ok()
            .or(Some(Value::String(s.clone()))),
        v => Some(v.clone()),
    }
}

fn extra_str(map: &Map<String, Value>, key: &str) -> Option<String> {
    match extra(map, key)? {
        Value::String(s) if !s.trim().is_empty() => Some(s),
        _ => None,
    }
}

fn extra_bool(map: &Map<String, Value>, key: &str) -> Option<bool> {
    match extra(map, key)? {
        Value::Bool(b) => Some(b),
        Value::String(s) => match s.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Flatten a Spoolman spool for Foreman.
pub fn facts(s: &SpoolResponse) -> SpoolFacts {
    let f = &s.filament;
    let modifier = extra_str(&f.extra, "material_modifier");
    // The stored flag decides. Filaments made before the flag existed fall back
    // to the same rule the form uses to auto-tick it.
    let abrasive = extra_bool(&f.extra, "abrasive").unwrap_or_else(|| {
        materials::is_abrasive_filament(f.material.as_deref(), modifier.as_deref())
    });
    let multi: Vec<String> = f
        .multi_color_hexes
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(filament::norm_hex)
        .filter(|h| !h.is_empty())
        .collect();
    SpoolFacts {
        id: s.id,
        filament_id: f.id,
        name: f
            .name
            .clone()
            .unwrap_or_else(|| format!("Filament #{}", f.id)),
        vendor: f.vendor.as_ref().map(|v| v.name.clone()),
        material: f.material.clone(),
        modifier,
        abrasive,
        color_hex: f
            .color_hex
            .as_deref()
            .map(filament::norm_hex)
            .filter(|h| !h.is_empty()),
        multi_color_direction: if multi.len() > 1 {
            f.multi_color_direction.clone()
        } else {
            None
        },
        multi_color_hexes: if multi.len() > 1 { multi } else { vec![] },
        remaining_weight: s.remaining_weight,
        initial_weight: s.initial_weight.or(f.weight),
        used_weight: s.used_weight,
        spool_weight: s.spool_weight.or(f.spool_weight),
        location: s.location.clone(),
        lot_nr: s.lot_nr.clone(),
        price: s.price.or(f.price),
        diameter: f.diameter,
        extruder_temp: f.settings_extruder_temp,
        bed_temp: f.settings_bed_temp,
        first_used: s.first_used.clone(),
        last_used: s.last_used.clone(),
        registered: s.registered.clone(),
        archived: s.archived,
        comment: s.comment.clone(),
    }
}

/// One spool by id.
pub async fn spool(sm: &Spoolman, id: i64) -> Result<SpoolFacts, AppError> {
    spoolman_rs::db::spool::find_by_id(&sm.db.pool, id)
        .await
        .map(|s| facts(&s))
        .map_err(|e| match e {
            spoolman_rs::error::SpoolmanError::NotFound(_) => {
                AppError::NotFound(format!("spool #{id}"))
            }
            e => AppError::Internal(format!("spoolman: {e}")),
        })
}

/// Every spool (archived too when asked), newest-used first.
pub async fn all(sm: &Spoolman, archived: bool) -> Result<Vec<SpoolFacts>, AppError> {
    let filter = spoolman_rs::db::spool::SpoolFilter {
        allow_archived: archived,
        ..Default::default()
    };
    let (rows, _) = spoolman_rs::db::spool::find(&sm.db.pool, &sm.db.engine, &filter)
        .await
        .map_err(|e| AppError::Internal(format!("spoolman: {e}")))?;
    let mut v: Vec<SpoolFacts> = rows.iter().map(facts).collect();
    sort_recent_first(&mut v);
    Ok(v)
}

/// Recently used first (never-used spools after, newest registered first).
pub fn sort_recent_first(v: &mut [SpoolFacts]) {
    v.sort_by(|a, b| {
        b.last_used
            .cmp(&a.last_used)
            .then_with(|| b.registered.cmp(&a.registered))
            .then_with(|| b.id.cmp(&a.id))
    });
}

// ---------------------------------------------------------------------------
// Loaded slots
// ---------------------------------------------------------------------------

/// Which spool sits in which slot of `printer`.
pub async fn slots_for(
    drivers: &DriverManager,
    sm: Option<&Spoolman>,
    printer: &Printer,
) -> Vec<LoadedSlot> {
    match printer.firmware {
        FirmwareKind::Moonraker => {
            let caps = drivers.get_capabilities(printer.id).await;
            let online = drivers
                .get_status(printer.id)
                .await
                .is_some_and(|s| s.state != PrinterState::Offline);
            let link = caps.as_ref().and_then(|c| c.spoolman_connected);
            let spool_id = caps.as_ref().and_then(|c| c.spoolman_spool_id);
            let spool = match (sm, spool_id) {
                (Some(sm), Some(id)) => spool(sm, id)
                    .await
                    .map_err(|e| warn!(printer_id = printer.id, "active spool #{id}: {e}"))
                    .ok(),
                _ => None,
            };
            let (writable, reason) = match (online, link) {
                (false, _) => (false, Some("Printer is offline.".to_owned())),
                (true, None) => (
                    false,
                    Some("Moonraker has no [spoolman] section, so it has no active spool.".into()),
                ),
                (true, Some(_)) => (true, None),
            };
            vec![LoadedSlot {
                printer_id: printer.id,
                printer_name: printer.name.clone(),
                tool: 0,
                label: "T0".into(),
                spool_id,
                spool,
                tray: None,
                writable,
                read_only_reason: reason,
            }]
        }
        FirmwareKind::Bambu => {
            let status = drivers.get_status(printer.id).await;
            let caps = drivers.get_capabilities(printer.id).await;
            let trays = status.map(|s| s.ams_trays).unwrap_or_default();
            let n = caps.and_then(|c| c.ams_slot_count).unwrap_or(0);
            let mut slots: Vec<LoadedSlot> = Vec::new();
            for slot in 0..n {
                let tray = trays.iter().find(|t| t.slot == slot).cloned();
                slots.push(bambu_slot(printer, slot, tray));
            }
            for t in trays.iter().filter(|t| t.slot >= n) {
                slots.push(bambu_slot(printer, t.slot, Some(t.clone())));
            }
            slots
        }
        _ => vec![],
    }
}

fn bambu_slot(
    printer: &Printer,
    slot: u32,
    tray: Option<foreman_core::events::AmsTray>,
) -> LoadedSlot {
    LoadedSlot {
        printer_id: printer.id,
        printer_name: printer.name.clone(),
        tool: slot,
        label: ams_slot_label(slot),
        spool_id: None,
        spool: None,
        tray,
        writable: false,
        read_only_reason: Some(BAMBU_READ_ONLY.into()),
    }
}

/// Loaded slots of every printer.
pub async fn all_slots(
    pool: &SqlitePool,
    drivers: &DriverManager,
    sm: Option<&Spoolman>,
) -> Result<Vec<LoadedSlot>, AppError> {
    let mut out = Vec::new();
    for p in repo::printer::list(pool).await? {
        out.extend(slots_for(drivers, sm, &p).await);
    }
    Ok(out)
}

/// Compat snapshot for one printer, with its loaded spools.
pub async fn snapshot(
    drivers: &DriverManager,
    sm: Option<&Spoolman>,
    printer: Printer,
) -> PrinterSnapshot {
    let caps = drivers.get_capabilities(printer.id).await;
    let status = drivers.get_status(printer.id).await;
    let loaded = slots_for(drivers, sm, &printer).await;
    PrinterSnapshot {
        printer,
        caps,
        status,
        loaded,
    }
}

// ---------------------------------------------------------------------------
// Guard
// ---------------------------------------------------------------------------

/// Library metadata access (so tests can run without a LibraryStore).
pub mod library_meta {
    use async_trait::async_trait;
    use foreman_core::{domain::FileMeta, error::AppError, library::LibraryStore};

    #[async_trait]
    pub trait MetaSource: Send + Sync {
        async fn meta(&self, file_id: i64) -> Result<Option<FileMeta>, AppError>;
    }

    #[async_trait]
    impl MetaSource for LibraryStore {
        async fn meta(&self, file_id: i64) -> Result<Option<FileMeta>, AppError> {
            Ok(self.details(file_id).await?.meta)
        }
    }
}

/// Grams a file needs on `tool` (that tool, else the only tool, else all tools).
fn grams_for_tool(meta: &foreman_core::domain::FileMeta, tool: u32) -> Option<f64> {
    let used: Vec<_> = meta.tools.iter().filter(|t| t.grams.is_some()).collect();
    used.iter()
        .find(|t| t.index == tool)
        .and_then(|t| t.grams)
        .or_else(|| (used.len() == 1).then(|| used[0].grams).flatten())
}

/// What the current print (remaining part) or the next queued job on this
/// printer needs from the spool in `tool`.
pub async fn job_need(
    pool: &SqlitePool,
    drivers: &DriverManager,
    lib: &dyn MetaSource,
    printer_id: i64,
    tool: u32,
) -> Option<JobNeed> {
    let status = drivers.get_status(printer_id).await;
    let printing = status
        .as_ref()
        .is_some_and(|s| matches!(s.state, PrinterState::Printing | PrinterState::Paused));
    if printing {
        if let Ok(Some(item)) = repo::queue::active_for_printer(pool, printer_id).await {
            if let Ok(Some(meta)) = lib.meta(item.file_id).await {
                if let Some(g) = grams_for_tool(&meta, tool) {
                    let done = status
                        .and_then(|s| s.progress)
                        .unwrap_or(0.0)
                        .clamp(0.0, 1.0);
                    return Some(JobNeed {
                        grams: g * (1.0 - done),
                        what: format!("the rest of the current print ({})", item.file_name),
                    });
                }
            }
        }
    }
    let next = repo::queue::list_waiting(pool, Some(printer_id))
        .await
        .ok()?
        .into_iter()
        .next()?;
    let meta = lib.meta(next.file_id).await.ok()??;
    Some(JobNeed {
        grams: grams_for_tool(&meta, tool)?,
        what: format!("the next queued job ({})", next.file_name),
    })
}

/// Everything needed to guard and load one spool into one slot.
pub struct LoadPlan {
    pub slot: LoadedSlot,
    pub spool: SpoolFacts,
    pub warnings: Vec<GuardWarning>,
    /// Writable slots elsewhere that hold this spool (unloaded on load).
    pub elsewhere: Vec<LoadedSlot>,
}

/// Run the guard for `spool_id` in `printer`'s `tool`.
pub async fn plan(
    pool: &SqlitePool,
    drivers: &DriverManager,
    sm: &Spoolman,
    lib: &dyn MetaSource,
    printer: &Printer,
    tool: u32,
    spool_id: i64,
) -> Result<LoadPlan, AppError> {
    let slots = slots_for(drivers, Some(sm), printer).await;
    let slot = slots
        .into_iter()
        .find(|s| s.tool == tool)
        .ok_or_else(|| AppError::NotFound(format!("{} has no tool {tool}", printer.name)))?;
    let sp = spool(sm, spool_id).await?;
    let elsewhere: Vec<LoadedSlot> = all_slots(pool, drivers, Some(sm))
        .await?
        .into_iter()
        .filter(|s| s.spool_id == Some(spool_id))
        .filter(|s| !(s.printer_id == printer.id && s.tool == tool))
        .collect();
    let caps = drivers.get_capabilities(printer.id).await;
    let need = job_need(pool, drivers, lib, printer.id, tool).await;
    let warnings = filament::guard(&GuardInput {
        printer,
        caps: caps.as_ref(),
        spool: &sp,
        tray: slot.tray.as_ref(),
        need,
        loaded_elsewhere: &elsewhere,
    });
    Ok(LoadPlan {
        slot,
        spool: sp,
        warnings,
        elsewhere,
    })
}

/// Set (or clear) the spool in a slot through the printer's driver.
pub async fn set_slot(
    drivers: &DriverManager,
    printer_id: i64,
    tool: u32,
    spool_id: Option<i64>,
) -> Result<(), AppError> {
    let driver = drivers
        .get_driver(printer_id)
        .await
        .ok_or_else(|| AppError::Driver("printer is not connected".into()))?;
    driver.set_active_spool(tool, spool_id).await?;
    info!(printer_id, tool, ?spool_id, "loaded spool set");
    Ok(())
}
