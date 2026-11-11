//! Inventory, shopping list and storage locations (task 4.3).
//!
//! - Targets (target / buy-below per filament) are Foreman data
//!   (`tracked_filament`), not Spoolman extra fields: they are a Foreman
//!   stock policy, other Spoolman clients have no use for them, and an extra
//!   field would need a schema entry plus JSON-text encoding for two numbers.
//! - Reserved weight comes from the queue: jobs waiting, sending or printing
//!   hold grams on the spool they will use (see [`inventory::reservations`]).
//! - Currency is Spoolman's `currency` setting.
//! - Locations are Spoolman's: spool `location` plus the `locations` setting
//!   (Spoolman's own UI keeps empty locations and their order there).

use std::collections::HashMap;

use foreman_core::{
    domain::QueueStatus,
    error::AppError,
    events::PrinterState,
    filament::SpoolFacts,
    inventory::{self, JobDemand, Reservation, SlotSpool, StockStatus, ToolDemand},
    repo::{self, tracked_filament::Tracked},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spoolman_rs::db::{setting as sm_setting, spool as sm_spool, Engine};
use sqlx::SqlitePool;
use tracing::{info, warn};
use ts_rs::TS;

use crate::{
    driver_manager::DriverManager,
    filaments::{self, FilamentRow},
    spools::{self, library_meta::MetaSource, Spoolman},
};

/// Highest target / buy-below a filament may have.
pub const MAX_TARGET: u32 = 99;
/// Spoolman's `location` column length.
const LOCATION_MAX_LEN: usize = 64;

fn sm_err(e: spoolman_rs::error::SpoolmanError) -> AppError {
    AppError::Internal(format!("spoolman: {e}"))
}

// ---------------------------------------------------------------------------
// Currency
// ---------------------------------------------------------------------------

/// Spoolman's `currency` setting (ISO code), else its default.
pub async fn currency(sm: &Spoolman) -> Result<String, AppError> {
    let raw = match sm_setting::get(&sm.db.pool, "currency")
        .await
        .map_err(sm_err)?
    {
        Some(v) => v,
        None => sm_setting::find_def("currency")
            .map(|d| d.default.to_owned())
            .unwrap_or_default(),
    };
    Ok(match serde_json::from_str::<Value>(&raw) {
        Ok(Value::String(s)) if !s.trim().is_empty() => s.trim().to_owned(),
        _ => raw.trim().trim_matches('"').to_owned(),
    })
}

// ---------------------------------------------------------------------------
// Reservations
// ---------------------------------------------------------------------------

/// Jobs on the queue that still need filament, with what each tool uses.
pub async fn demands(
    pool: &SqlitePool,
    drivers: &DriverManager,
    lib: &dyn MetaSource,
) -> Result<Vec<JobDemand>, AppError> {
    let mut items = repo::queue::list_board(pool).await?;
    // Printing first (they are using the spool now), then lane order.
    items.sort_by_key(|i| {
        (
            match i.status {
                QueueStatus::Printing => 0,
                QueueStatus::Sending => 1,
                _ => 2,
            },
            i.position,
            i.id,
        )
    });
    // Project prints draw from their slot spool, whatever the file was sliced for.
    let plate_spools = crate::projects::plate_spools(pool).await?;
    let mut out = Vec::new();
    for item in items {
        if !matches!(
            item.status,
            QueueStatus::Waiting | QueueStatus::Sending | QueueStatus::Printing
        ) {
            continue;
        }
        let Ok(Some(meta)) = lib.meta(item.file_id).await else {
            continue;
        };
        let tools: Vec<ToolDemand> = meta
            .tools
            .iter()
            .filter_map(|t| {
                let slot_spool = item
                    .project_plate_id
                    .and_then(|p| plate_spools.get(&p))
                    .and_then(|m| m.get(&t.index).copied());
                Some(ToolDemand {
                    tool: t.index,
                    grams: t.grams.filter(|g| *g > 0.0)?,
                    spool_id: slot_spool.or(t.spool_id),
                })
            })
            .collect();
        if tools.is_empty() {
            continue;
        }
        let left = match (item.status, item.printer_id) {
            (QueueStatus::Printing, Some(pid)) => {
                let st = drivers.get_status(pid).await;
                let printing = st.as_ref().is_some_and(|s| {
                    matches!(s.state, PrinterState::Printing | PrinterState::Paused)
                });
                if printing {
                    1.0 - st.and_then(|s| s.progress).unwrap_or(0.0).clamp(0.0, 1.0)
                } else {
                    1.0
                }
            }
            _ => 1.0,
        };
        out.push(JobDemand {
            queue_item_id: Some(item.id),
            project_plate_id: item.project_plate_id,
            file_name: item.file_name,
            printer_id: item.printer_id,
            tools,
            left,
        });
    }
    // Then project prints not on a queue yet.
    out.extend(crate::projects::unqueued_demands(pool, lib, &plate_spools).await?);
    Ok(out)
}

/// Grams reserved by queued jobs, per job and spool.
pub async fn reservations(
    pool: &SqlitePool,
    drivers: &DriverManager,
    sm: &Spoolman,
    lib: &dyn MetaSource,
    spools: &[SpoolFacts],
) -> Result<Vec<Reservation>, AppError> {
    let jobs = demands(pool, drivers, lib).await?;
    if jobs.is_empty() {
        return Ok(vec![]);
    }
    let slots: Vec<SlotSpool> = spools::all_slots(pool, drivers, Some(sm))
        .await?
        .into_iter()
        .filter_map(|s| {
            Some(SlotSpool {
                printer_id: s.printer_id,
                tool: s.tool,
                spool_id: s.spool_id?,
            })
        })
        .collect();
    let remaining: HashMap<i64, f64> = spools
        .iter()
        .filter(|s| !s.archived)
        .map(|s| (s.id, s.remaining_weight.unwrap_or(0.0).max(0.0)))
        .collect();
    Ok(inventory::reservations(&jobs, &slots, &remaining))
}

// ---------------------------------------------------------------------------
// Inventory
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct InventoryRow {
    pub filament: FilamentRow,
    pub target: u32,
    pub buy_below: u32,
    /// Spools that count as stock: more than 50 g left after reservations.
    pub stock: u32,
    /// Every unarchived spool of the filament.
    pub spools: u32,
    pub grams: f64,
    pub grams_reserved: f64,
    pub status: StockStatus,
    /// Spools to buy: target − stock while below buy-below, else 0.
    pub buy: u32,
    /// `buy × filament price`, when the filament has a price.
    pub line_cost: Option<f64>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Inventory {
    /// Spoolman's currency setting (ISO code).
    pub currency: String,
    pub tracked: Vec<InventoryRow>,
    /// Unarchived spools.
    pub spool_count: u32,
    pub grams_on_hand: f64,
    pub grams_reserved: f64,
    /// Tracked filaments on the shopping list.
    pub need_count: u32,
    /// Sum of the shopping list's line costs.
    pub est_cost: f64,
    /// Shopping lines with no price (left out of `est_cost`).
    pub unpriced: u32,
    pub reservations: Vec<Reservation>,
}

/// `PUT /api/inventory/{filament_id}`.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct TrackFilament {
    pub target: u32,
    pub buy_below: u32,
}

/// Sum that is `0.0` (not `-0.0`, which an empty f64 sum gives) when empty.
fn gsum(it: impl Iterator<Item = f64>) -> f64 {
    it.fold(0.0, |a, b| a + b)
}

/// One inventory row from a filament, its spools and their reservations.
pub fn row(
    f: FilamentRow,
    t: &Tracked,
    spools: &[SpoolFacts],
    reserved: &HashMap<i64, f64>,
) -> InventoryRow {
    let mine: Vec<&SpoolFacts> = spools
        .iter()
        .filter(|s| s.filament_id == f.id && !s.archived)
        .collect();
    let grams = gsum(mine.iter().filter_map(|s| s.remaining_weight));
    let grams_reserved = gsum(mine.iter().filter_map(|s| reserved.get(&s.id).copied()));
    let stock =
        inventory::usable(mine.iter().map(|s| {
            s.remaining_weight.unwrap_or(0.0) - reserved.get(&s.id).copied().unwrap_or(0.0)
        }));
    let target = t.target.clamp(0, MAX_TARGET as i64) as u32;
    let buy_below = t.buy_below.clamp(0, target as i64) as u32;
    let (status, buy) = inventory::stock_status(stock, target, buy_below);
    InventoryRow {
        line_cost: f.price.filter(|_| buy > 0).map(|p| p * buy as f64),
        filament: f,
        target,
        buy_below,
        stock,
        spools: mine.len() as u32,
        grams,
        grams_reserved,
        status,
        buy,
    }
}

/// The whole Inventory tab.
pub async fn inventory(
    pool: &SqlitePool,
    drivers: &DriverManager,
    sm: &Spoolman,
    lib: &dyn MetaSource,
) -> Result<Inventory, AppError> {
    let spools = spools::all(sm, false).await?;
    let fils = filaments::filaments(sm).await?;
    let res = reservations(pool, drivers, sm, lib, &spools).await?;
    let reserved = inventory::reserved_by_spool(&res);
    let mut by_id: HashMap<i64, FilamentRow> = fils.into_iter().map(|f| (f.id, f)).collect();
    let mut tracked = Vec::new();
    for t in repo::tracked_filament::list(pool).await? {
        match by_id.remove(&t.filament_id) {
            Some(f) => tracked.push(row(f, &t, &spools, &reserved)),
            None => {
                // The filament was deleted in Spoolman.
                warn!(
                    filament_id = t.filament_id,
                    "tracked filament is gone; untracking"
                );
                repo::tracked_filament::remove(pool, t.filament_id).await?;
            }
        }
    }
    tracked.sort_by_key(|r| {
        (
            match r.status {
                StockStatus::Out => 0,
                StockStatus::Buy => 1,
                StockStatus::Ok => 2,
            },
            r.filament.vendor.clone().unwrap_or_default().to_lowercase(),
            r.filament.name.to_lowercase(),
        )
    });
    let shop: Vec<&InventoryRow> = tracked.iter().filter(|r| r.buy > 0).collect();
    let live: Vec<&SpoolFacts> = spools.iter().filter(|s| !s.archived).collect();
    Ok(Inventory {
        currency: currency(sm).await?,
        spool_count: live.len() as u32,
        grams_on_hand: gsum(live.iter().filter_map(|s| s.remaining_weight)),
        grams_reserved: gsum(res.iter().map(|r| r.grams)),
        need_count: shop.len() as u32,
        est_cost: gsum(shop.iter().filter_map(|r| r.line_cost)),
        unpriced: shop.iter().filter(|r| r.line_cost.is_none()).count() as u32,
        tracked,
        reservations: res,
    })
}

/// Track a filament (or change its numbers).
pub async fn track(
    pool: &SqlitePool,
    sm: &Spoolman,
    filament_id: i64,
    req: &TrackFilament,
) -> Result<(), AppError> {
    if req.target > MAX_TARGET {
        return Err(AppError::InvalidInput(format!(
            "Target: 0 to {MAX_TARGET} spools."
        )));
    }
    if req.buy_below > req.target {
        return Err(AppError::InvalidInput(
            "Buy below can't be more than the target.".into(),
        ));
    }
    filaments::filament(sm, filament_id).await?;
    repo::tracked_filament::upsert(
        pool,
        Tracked {
            filament_id,
            target: req.target as i64,
            buy_below: req.buy_below as i64,
        },
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Locations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct LocationRow {
    pub name: String,
    /// Unarchived spools here.
    pub spool_count: u32,
    pub grams: f64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Locations {
    pub locations: Vec<LocationRow>,
    /// Unarchived spools with no location.
    pub unplaced: u32,
}

/// `PATCH /api/spool-locations/{name}`.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct RenameLocation {
    pub name: String,
}

/// Spoolman's `locations` setting (ordered list, may hold empty locations).
async fn location_setting(sm: &Spoolman) -> Result<Vec<String>, AppError> {
    let raw = sm_setting::get(&sm.db.pool, "locations")
        .await
        .map_err(sm_err)?;
    Ok(raw
        .and_then(|r| serde_json::from_str::<Vec<String>>(&r).ok())
        .unwrap_or_default())
}

async fn set_setting(sm: &Spoolman, key: &str, value: &Value) -> Result<(), AppError> {
    let v = value.to_string();
    if matches!(sm.db.engine, Engine::Mysql) {
        sm_setting::set_mysql(&sm.db.pool, key, &v).await
    } else {
        sm_setting::set(&sm.db.pool, key, &v).await
    }
    .map_err(sm_err)
}

/// Every location: from spools and from Spoolman's list, with counts.
/// Names are exact (Spoolman compares them case-sensitively).
pub async fn locations(sm: &Spoolman) -> Result<Locations, AppError> {
    let spools = spools::all(sm, false).await?;
    let mut map: HashMap<String, LocationRow> = HashMap::new();
    let entry = |map: &mut HashMap<String, LocationRow>, name: &str| {
        map.entry(name.to_owned()).or_insert_with(|| LocationRow {
            name: name.to_owned(),
            spool_count: 0,
            grams: 0.0,
        });
    };
    for l in location_setting(sm).await? {
        if !l.trim().is_empty() {
            entry(&mut map, &l);
        }
    }
    let mut unplaced = 0;
    for s in &spools {
        match s.location.as_deref().filter(|l| !l.trim().is_empty()) {
            None => unplaced += 1,
            Some(l) => {
                entry(&mut map, l);
                if let Some(r) = map.get_mut(l) {
                    r.spool_count += 1;
                    r.grams += s.remaining_weight.unwrap_or(0.0);
                }
            }
        }
    }
    let mut locations: Vec<LocationRow> = map.into_values().collect();
    locations.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(Locations {
        locations,
        unplaced,
    })
}

/// Rename a location: every spool on it (archived too, as Spoolman does) and
/// Spoolman's location list. Renaming onto an existing name merges the two.
/// Returns the number of spools moved.
pub async fn rename_location(sm: &Spoolman, old: &str, new: &str) -> Result<u32, AppError> {
    let new = new.trim();
    if new.is_empty() {
        return Err(AppError::InvalidInput("Give the location a name.".into()));
    }
    if new.chars().count() > LOCATION_MAX_LEN {
        return Err(AppError::InvalidInput(format!(
            "Location names are at most {LOCATION_MAX_LEN} characters."
        )));
    }
    let all = spools::all(sm, true).await?;
    let moved: Vec<i64> = all
        .iter()
        .filter(|s| s.location.as_deref() == Some(old))
        .map(|s| s.id)
        .collect();
    let mut listed = location_setting(sm).await?;
    if moved.is_empty() && !listed.iter().any(|l| l == old) {
        return Err(AppError::NotFound(format!("location {old:?}")));
    }
    if old == new {
        return Ok(0);
    }
    sm_spool::rename_location(&sm.db.pool, old, new)
        .await
        .map_err(sm_err)?;

    // Spoolman's ordered list: replace in place, no duplicates.
    if !listed.is_empty() {
        let had_new = listed.iter().any(|l| l == new);
        listed = listed
            .into_iter()
            .filter_map(|l| match l == old {
                true if had_new => None,
                true => Some(new.to_owned()),
                false => Some(l),
            })
            .collect();
        set_setting(sm, "locations", &serde_json::json!(listed)).await?;
    }
    // Spoolman's per-location spool order: move the old key's order over.
    if let Some(raw) = sm_setting::get(&sm.db.pool, "locations_spoolorders")
        .await
        .map_err(sm_err)?
    {
        if let Ok(Value::Object(mut m)) = serde_json::from_str::<Value>(&raw) {
            if let Some(Value::Array(old_order)) = m.remove(old) {
                let merged = match m.remove(new) {
                    Some(Value::Array(mut cur)) => {
                        cur.extend(old_order);
                        cur
                    }
                    _ => old_order,
                };
                m.insert(new.to_owned(), Value::Array(merged));
                set_setting(sm, "locations_spoolorders", &Value::Object(m)).await?;
            }
        }
    }
    for id in &moved {
        if let Ok(sp) = sm_spool::find_by_id(&sm.db.pool, *id).await {
            spoolman_rs::ws::broadcast(&sm.event_tx, "updated", "spool", sp.id, &sp);
        }
    }
    info!(old, new, spools = moved.len(), "location renamed");
    Ok(moved.len() as u32)
}
