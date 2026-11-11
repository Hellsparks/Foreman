//! Templates and projects from a template (task 5.1, W9).
//!
//! - A template's plates are **copies** of Library files under
//!   `Templates/<template name>/`, so deleting the originals is safe.
//! - A project copies the template's plates and slots, with one spool per slot and
//!   `copies = copies per set × sets` prints per plate.
//! - Prints, not plates, are what everything counts: progress, grams,
//!   reservations, and "print all remaining" (one fleet-queue item per print).
//! - The scheduler calls [`print_ended`] when a queue item finishes; a complete
//!   print marks its plate, and the project closes when every print is done.

use std::collections::HashMap;

use foreman_core::{
    domain::{JobEnd, QueueItem, QueueSource, QueueStatus, ToolUsage},
    error::AppError,
    filament::SpoolFacts,
    inventory::{JobDemand, ToolDemand},
    projects::{
        self, hex_distance, plate_counts, slot_key, PlateCounts, TemplateForm, ToolSlot,
        MAX_COPIES, MAX_SETS,
    },
    repo::{self, project::PlateRow},
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::{info, warn};
use ts_rs::TS;

use crate::{
    inventory,
    spools::{self, library_meta::MetaSource},
    state::AppState,
};

/// Library folder that holds template plate copies.
pub const TEMPLATE_FOLDER: &str = "Templates";

fn bad(m: impl Into<String>) -> AppError {
    AppError::InvalidInput(m.into())
}

fn clean_hex(h: &Option<String>) -> Option<String> {
    h.as_deref()
        .map(|h| h.trim().trim_start_matches('#').to_uppercase())
        .filter(|h| h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()))
}

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

/// Facts about a plate file, for the editor and the project pages.
#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
pub struct PlateFile {
    #[ts(type = "number")]
    pub file_id: i64,
    pub file_name: String,
    pub thumb_url: Option<String>,
    #[ts(type = "number | null")]
    pub print_time_secs: Option<i64>,
    pub tools: Vec<ToolUsage>,
    /// The file is gone from the Library (or in the trash).
    pub missing: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct TemplateView {
    #[ts(type = "number")]
    pub id: i64,
    pub template: TemplateForm,
    /// One entry per plate, same order.
    pub files: Vec<PlateFile>,
    /// What keeps it from being used; empty = ready.
    pub problems: Vec<String>,
    pub prints_per_set: u32,
    #[ts(type = "number | null")]
    pub print_time_secs: Option<i64>,
    pub updated_at: String,
}

pub async fn plate_file(state: &AppState, file_id: i64) -> PlateFile {
    match state.library.get(file_id).await {
        Ok(e) if e.deleted_at.is_none() => PlateFile {
            file_id,
            file_name: e.name,
            thumb_url: e.thumb_url,
            print_time_secs: e.print_time_secs,
            tools: e.tools,
            missing: false,
        },
        _ => PlateFile {
            file_id,
            missing: true,
            ..Default::default()
        },
    }
}

async fn template_view(state: &AppState, row: repo::project::TemplateRow) -> TemplateView {
    let mut files = Vec::new();
    for p in &row.form.plates {
        files.push(plate_file(state, p.file_id).await);
    }
    let mut problems = projects::template_problems(&row.form);
    for (p, f) in row.form.plates.iter().zip(&files) {
        if f.missing {
            problems.push(format!("{}: its file is missing from the Library.", p.name));
        }
    }
    let time: i64 = row
        .form
        .plates
        .iter()
        .zip(&files)
        .filter_map(|(p, f)| f.print_time_secs.map(|s| s * p.copies as i64))
        .sum();
    TemplateView {
        id: row.id,
        prints_per_set: row.form.plates.iter().map(|p| p.copies).sum(),
        print_time_secs: (time > 0).then_some(time),
        template: row.form,
        files,
        problems,
        updated_at: row.updated_at,
    }
}

pub async fn templates(state: &AppState) -> Result<Vec<TemplateView>, AppError> {
    let mut out = Vec::new();
    for r in repo::project::list_templates(&state.foreman_db.pool).await? {
        out.push(template_view(state, r).await);
    }
    Ok(out)
}

pub async fn template(state: &AppState, id: i64) -> Result<TemplateView, AppError> {
    let r = repo::project::get_template(&state.foreman_db.pool, id).await?;
    Ok(template_view(state, r).await)
}

/// Copy a Library file into the template's folder (unless it already lives under
/// `Templates/`). Returns the id to store.
async fn own_copy(state: &AppState, file_id: i64, template_name: &str) -> Result<i64, AppError> {
    let entry = state.library.get(file_id).await?;
    if entry.deleted_at.is_some() {
        return Err(bad(format!("{} is in the Library trash.", entry.name)));
    }
    if entry.path.starts_with(&format!("{TEMPLATE_FOLDER}/")) {
        return Ok(file_id);
    }
    let folder_name: String = template_name
        .chars()
        .map(|c| if c == '/' || c == '\\' { '-' } else { c })
        .collect();
    let folder = format!("{TEMPLATE_FOLDER}/{}", folder_name.trim());
    let src = state.library.file_path(file_id).await?;
    let tmp = state.library.tmp_dir().join(format!(
        "tpl-{}-{}",
        file_id,
        chrono::Utc::now().timestamp_micros()
    ));
    tokio::fs::copy(&src, &tmp)
        .await
        .map_err(|e| AppError::Internal(format!("copy {}: {e}", entry.name)))?;
    let copy = state.library.add_file(&folder, &entry.name, &tmp).await?;
    info!(from = file_id, to = copy.id, path = %copy.path, "template plate copied");
    Ok(copy.id)
}

fn tidy(form: &TemplateForm) -> Result<TemplateForm, AppError> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err(bad("Give the template a name."));
    }
    let mut f = form.clone();
    f.name = name.to_owned();
    f.category = f
        .category
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty());
    f.notes = f
        .notes
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty());
    for s in &mut f.slots {
        s.key = slot_key(&s.key);
        s.hex = clean_hex(&s.hex);
        s.label = s
            .label
            .take()
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty());
    }
    for p in &mut f.plates {
        p.name = p.name.trim().to_owned();
        if p.name.is_empty() {
            return Err(bad("Every plate needs a name."));
        }
        if p.copies == 0 || p.copies > MAX_COPIES {
            return Err(bad(format!(
                "{}: copies per set 1 to {MAX_COPIES}.",
                p.name
            )));
        }
        for t in &mut p.tools {
            t.slot = slot_key(&t.slot);
        }
        p.tools.sort_by_key(|t| t.tool);
        p.tools.dedup_by_key(|t| t.tool);
    }
    Ok(f)
}

/// Create (id None) or update a template. Incomplete templates save fine (the
/// editor saves drafts); [`TemplateView::problems`] says what is missing.
pub async fn save_template(
    state: &AppState,
    id: Option<i64>,
    form: &TemplateForm,
) -> Result<TemplateView, AppError> {
    let mut f = tidy(form)?;
    for p in &mut f.plates {
        p.file_id = own_copy(state, p.file_id, &f.name).await?;
    }
    let pool = &state.foreman_db.pool;
    let row = match id {
        Some(id) => repo::project::update_template(pool, id, &f).await?,
        None => repo::project::create_template(pool, &f).await?,
    };
    Ok(template_view(state, row).await)
}

// ---------------------------------------------------------------------------
// Planning: grams per slot, spool options
// ---------------------------------------------------------------------------

/// A spool that could feed a slot, with how well it fits.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SpoolOption {
    pub spool: SpoolFacts,
    /// Remaining minus what queued jobs and other projects reserve.
    pub free: f64,
    pub reserved: f64,
    /// `free ≥ need`.
    pub fits: bool,
    /// The spool's material matches what the plate files were sliced for.
    pub material_ok: bool,
    /// 0 = same colour as the slot's preferred one; `null` = no preference.
    pub colour_distance: Option<f64>,
    /// Printers it is loaded on ("HellV0 T0").
    pub loaded: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SlotPlan {
    pub key: String,
    pub label: Option<String>,
    pub hex: Option<String>,
    pub allow_swap: bool,
    /// Materials the plate files were sliced with for this slot.
    pub materials: Vec<String>,
    /// Plates that use it ("Pawns ×2").
    pub plates: Vec<String>,
    /// Grams for every print of the project.
    pub need: f64,
    /// Best first: material, fits, colour.
    pub options: Vec<SpoolOption>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ProjectPlan {
    #[ts(type = "number")]
    pub template_id: i64,
    pub sets: u32,
    /// Total prints (copies × sets summed over plates).
    pub prints: u32,
    pub slots: Vec<SlotPlan>,
}

/// Tool grams of a file: (tool, grams).
fn tool_grams(tools: &[ToolUsage]) -> Vec<(u32, f64)> {
    tools
        .iter()
        .filter_map(|t| Some((t.index, t.grams.filter(|g| *g > 0.0)?)))
        .collect()
}

fn same_material(a: Option<&str>, b: &str) -> bool {
    a.is_some_and(|a| a.trim().eq_ignore_ascii_case(b.trim()))
}

/// Grams, materials and spool options per slot for `sets` of a template.
pub async fn plan(state: &AppState, template_id: i64, sets: u32) -> Result<ProjectPlan, AppError> {
    let sets = sets.clamp(1, MAX_SETS);
    let view = template(state, template_id).await?;
    let t = &view.template;
    let sm = state
        .spoolman
        .get()
        .ok_or_else(|| AppError::NotImplemented("Spoolman is not running".into()))?;
    let all = spools::all(sm, false).await?;
    let res = inventory::reservations(
        &state.foreman_db.pool,
        &state.driver_manager,
        sm,
        state.library.as_ref(),
        &all,
    )
    .await?;
    let reserved = foreman_core::inventory::reserved_by_spool(&res);
    let slots_loaded = spools::all_slots(&state.foreman_db.pool, &state.driver_manager, Some(sm))
        .await
        .unwrap_or_default();

    let mut out = Vec::new();
    for s in &t.slots {
        let key = slot_key(&s.key);
        let mut need = 0.0;
        let mut materials: Vec<String> = Vec::new();
        let mut plates = Vec::new();
        for (p, f) in t.plates.iter().zip(&view.files) {
            let per = projects::grams_per_slot(&p.tools, &tool_grams(&f.tools));
            let used = p.tools.iter().any(|ts| slot_key(&ts.slot) == key);
            if !used {
                continue;
            }
            plates.push(if p.copies > 1 {
                format!("{} ×{}", p.name, p.copies)
            } else {
                p.name.clone()
            });
            need += per
                .iter()
                .filter(|(k, _)| *k == key)
                .map(|(_, g)| g)
                .sum::<f64>()
                * f64::from(p.copies * sets);
            for ts in p.tools.iter().filter(|ts| slot_key(&ts.slot) == key) {
                if let Some(m) = f
                    .tools
                    .iter()
                    .find(|u| u.index == ts.tool)
                    .and_then(|u| u.material.clone())
                {
                    if !materials.iter().any(|x| x.eq_ignore_ascii_case(&m)) {
                        materials.push(m);
                    }
                }
            }
        }
        let mut options: Vec<SpoolOption> = all
            .iter()
            .filter(|sp| !sp.archived)
            .map(|sp| {
                let r = reserved.get(&sp.id).copied().unwrap_or(0.0);
                let free = sp.remaining_weight.unwrap_or(0.0) - r;
                SpoolOption {
                    free,
                    reserved: r,
                    fits: free >= need,
                    material_ok: materials.is_empty()
                        || materials
                            .iter()
                            .any(|m| same_material(sp.material.as_deref(), m)),
                    colour_distance: match (&s.hex, &sp.color_hex) {
                        (Some(a), Some(b)) => hex_distance(a, b),
                        _ => None,
                    },
                    loaded: slots_loaded
                        .iter()
                        .filter(|l| l.spool_id == Some(sp.id))
                        .map(|l| format!("{} {}", l.printer_name, l.label))
                        .collect(),
                    spool: sp.clone(),
                }
            })
            .collect();
        options.sort_by(|a, b| {
            b.material_ok
                .cmp(&a.material_ok)
                .then(b.fits.cmp(&a.fits))
                .then(
                    a.colour_distance
                        .unwrap_or(f64::MAX)
                        .total_cmp(&b.colour_distance.unwrap_or(f64::MAX)),
                )
                .then(b.free.total_cmp(&a.free))
        });
        out.push(SlotPlan {
            key,
            label: s.label.clone(),
            hex: s.hex.clone(),
            allow_swap: s.allow_swap,
            materials,
            plates,
            need,
            options,
        });
    }
    Ok(ProjectPlan {
        template_id,
        sets,
        prints: t.plates.iter().map(|p| p.copies * sets).sum(),
        slots: out,
    })
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct SlotSpool {
    pub key: String,
    #[ts(type = "number | null")]
    pub spool_id: Option<i64>,
}

/// `POST /api/projects`.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct CreateProject {
    #[ts(type = "number")]
    pub template_id: i64,
    #[serde(default)]
    pub name: Option<String>,
    /// `YYYY-MM-DD`.
    #[serde(default)]
    pub due_date: Option<String>,
    pub sets: u32,
    pub spools: Vec<SlotSpool>,
    /// Also put every print on the fleet queue.
    #[serde(default)]
    pub queue_all: bool,
}

fn check_date(d: &Option<String>) -> Result<Option<String>, AppError> {
    match d.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        None => Ok(None),
        Some(d) => chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .map(|_| Some(d.to_owned()))
            .map_err(|_| bad("Due date must be YYYY-MM-DD.")),
    }
}

pub async fn create_project(
    state: &AppState,
    req: &CreateProject,
    user: &str,
) -> Result<ProjectView, AppError> {
    if req.sets == 0 || req.sets > MAX_SETS {
        return Err(bad(format!("Sets: 1 to {MAX_SETS}.")));
    }
    let view = template(state, req.template_id).await?;
    if !view.problems.is_empty() {
        return Err(bad(format!(
            "Template {} isn't ready: {}",
            view.template.name,
            view.problems.join(" ")
        )));
    }
    let t = &view.template;
    let due = check_date(&req.due_date)?;
    if let Some(sm) = state.spoolman.get() {
        for s in &req.spools {
            if let Some(id) = s.spool_id {
                spools::spool(sm, id).await?;
            }
        }
    }
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if req.sets > 1 {
                format!("{} ×{}", t.name, req.sets)
            } else {
                t.name.clone()
            }
        });
    let slots = t
        .slots
        .iter()
        .map(|s| {
            let spool = req
                .spools
                .iter()
                .find(|x| slot_key(&x.key) == s.key)
                .and_then(|x| x.spool_id);
            (s.key.clone(), s.label.clone(), s.hex.clone(), spool)
        })
        .collect();
    let plates = t
        .plates
        .iter()
        .map(|p| {
            (
                p.name.clone(),
                p.file_id,
                p.copies * req.sets,
                p.tools.clone(),
            )
        })
        .collect();
    let pool = &state.foreman_db.pool;
    let id = repo::project::create_project(
        pool,
        &repo::project::NewProject {
            name: &name,
            template_id: Some(req.template_id),
            template_name: Some(&t.name),
            sets: req.sets,
            due_date: due.as_deref(),
            slots,
            plates,
        },
    )
    .await?;
    info!(
        project = id,
        template = req.template_id,
        sets = req.sets,
        "project created"
    );
    if req.queue_all {
        print_all_remaining(state, id, user).await?;
    }
    project(state, id).await
}

/// One print of a plate that is on a queue now.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct LivePrint {
    #[ts(type = "number")]
    pub queue_item_id: i64,
    pub status: QueueStatus,
    #[ts(type = "number | null")]
    pub printer_id: Option<i64>,
    /// Printer name, or "fleet" while it waits on the fleet queue.
    pub where_: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ProjectPlate {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub file: PlateFile,
    pub tools: Vec<ToolSlot>,
    pub counts: PlateCounts,
    /// Lowest copies allowed now (printed + printing, at least 1).
    pub min_copies: u32,
    /// Grams one print takes, per slot key.
    pub grams: Vec<SlotGrams>,
    pub live: Vec<LivePrint>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SlotGrams {
    pub key: String,
    pub grams: f64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ProjectSlot {
    pub key: String,
    pub label: Option<String>,
    pub hex: Option<String>,
    pub spool: Option<SpoolFacts>,
    /// Grams still to print (all prints not done yet).
    pub need: f64,
    /// Grams this project holds on its spool now.
    pub reserved: f64,
    /// Reserved on the same spool by others (queue, other projects).
    pub reserved_elsewhere: f64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ProjectTotals {
    pub prints: u32,
    pub printed: u32,
    pub printing: u32,
    pub queued: u32,
    pub remaining: u32,
    #[ts(type = "number")]
    pub secs_used: i64,
    pub grams_used: f64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ProjectEvent {
    pub at: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ProjectView {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    #[ts(type = "number | null")]
    pub template_id: Option<i64>,
    pub template_name: Option<String>,
    pub sets: u32,
    pub due_date: Option<String>,
    /// `active` | `done` | `archived`.
    pub status: String,
    pub created_at: String,
    pub closed_at: Option<String>,
    pub plates: Vec<ProjectPlate>,
    pub slots: Vec<ProjectSlot>,
    pub totals: ProjectTotals,
    pub events: Vec<ProjectEvent>,
}

/// Plate counts from its row and live queue items.
async fn counts_of(
    pool: &SqlitePool,
    p: &PlateRow,
) -> Result<(PlateCounts, Vec<(i64, QueueStatus, Option<i64>)>), AppError> {
    let items = repo::project::plate_items(pool, p.id).await?;
    let st: Vec<QueueStatus> = items.iter().map(|i| i.1).collect();
    Ok((
        plate_counts(p.copies.max(0) as u32, p.printed.max(0) as u32, &st),
        items,
    ))
}

pub async fn project(state: &AppState, id: i64) -> Result<ProjectView, AppError> {
    let pool = &state.foreman_db.pool;
    let row = repo::project::get_project(pool, id).await?;
    let printers: HashMap<i64, String> = repo::printer::list(pool)
        .await?
        .into_iter()
        .map(|p| (p.id, p.name))
        .collect();
    let mut plates = Vec::new();
    let mut need: HashMap<String, f64> = HashMap::new();
    for p in repo::project::plates(pool, id).await? {
        let (counts, items) = counts_of(pool, &p).await?;
        let file = plate_file(state, p.file_id).await;
        let grams: Vec<SlotGrams> = projects::grams_per_slot(&p.tools, &tool_grams(&file.tools))
            .into_iter()
            .map(|(key, grams)| SlotGrams { key, grams })
            .collect();
        let left = counts.copies.saturating_sub(counts.printed);
        for g in &grams {
            *need.entry(g.key.clone()).or_insert(0.0) += g.grams * f64::from(left);
        }
        plates.push(ProjectPlate {
            id: p.id,
            name: p.name.clone(),
            min_copies: projects::min_copies(&counts),
            live: items
                .iter()
                .map(|(qid, st, pid)| LivePrint {
                    queue_item_id: *qid,
                    status: *st,
                    printer_id: *pid,
                    where_: pid
                        .and_then(|p| printers.get(&p).cloned())
                        .unwrap_or_else(|| "fleet".into()),
                })
                .collect(),
            file,
            tools: p.tools,
            counts,
            grams,
        });
    }
    let sm = state.spoolman.get();
    let (spool_list, reservations) = match sm {
        Some(sm) => {
            let all = spools::all(sm, false).await?;
            let res = inventory::reservations(
                pool,
                &state.driver_manager,
                sm,
                state.library.as_ref(),
                &all,
            )
            .await?;
            (all, res)
        }
        None => (vec![], vec![]),
    };
    let plate_ids: Vec<i64> = plates.iter().map(|p| p.id).collect();
    let mut slots = Vec::new();
    for s in repo::project::slots(pool, id).await? {
        let spool = s
            .spool_id
            .and_then(|sid| spool_list.iter().find(|x| x.id == sid).cloned());
        let (mine, other): (f64, f64) = reservations
            .iter()
            .filter(|r| Some(r.spool_id) == s.spool_id)
            .fold((0.0, 0.0), |(m, o), r| {
                if r.project_plate_id.is_some_and(|p| plate_ids.contains(&p)) {
                    (m + r.grams, o)
                } else {
                    (m, o + r.grams)
                }
            });
        slots.push(ProjectSlot {
            need: need.get(&s.key).copied().unwrap_or(0.0),
            key: s.key,
            label: s.label,
            hex: s.hex,
            spool,
            reserved: mine,
            reserved_elsewhere: other,
        });
    }
    let (secs, grams) = repo::project::job_totals(pool, id).await?;
    let sum = |f: fn(&PlateCounts) -> u32| plates.iter().map(|p| f(&p.counts)).sum::<u32>();
    let totals = ProjectTotals {
        prints: sum(|c| c.copies),
        printed: sum(|c| c.printed),
        printing: sum(|c| c.printing),
        queued: sum(|c| c.queued),
        remaining: sum(|c| c.remaining),
        secs_used: secs,
        grams_used: grams,
    };
    Ok(ProjectView {
        id: row.id,
        name: row.name,
        template_id: row.template_id,
        template_name: row.template_name,
        sets: row.sets.max(0) as u32,
        due_date: row.due_date,
        status: row.status,
        created_at: row.created_at,
        closed_at: row.closed_at,
        plates,
        slots,
        totals,
        events: repo::project::events(pool, id)
            .await?
            .into_iter()
            .map(|(at, text)| ProjectEvent { at, text })
            .collect(),
    })
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ProjectSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub template_name: Option<String>,
    pub due_date: Option<String>,
    pub status: String,
    pub closed_at: Option<String>,
    pub totals: ProjectTotals,
    /// Slot colours (`RRGGBB`): the assigned spool's, else the preferred one.
    pub swatches: Vec<String>,
}

pub async fn list(state: &AppState) -> Result<Vec<ProjectSummary>, AppError> {
    let pool = &state.foreman_db.pool;
    let mut out = Vec::new();
    for r in repo::project::list_projects(pool).await? {
        let mut t = ProjectTotals {
            prints: 0,
            printed: 0,
            printing: 0,
            queued: 0,
            remaining: 0,
            secs_used: 0,
            grams_used: 0.0,
        };
        for p in repo::project::plates(pool, r.id).await? {
            let (c, _) = counts_of(pool, &p).await?;
            t.prints += c.copies;
            t.printed += c.printed;
            t.printing += c.printing;
            t.queued += c.queued;
            t.remaining += c.remaining;
        }
        let mut swatches = Vec::new();
        for s in repo::project::slots(pool, r.id).await? {
            let from_spool = match (state.spoolman.get(), s.spool_id) {
                (Some(sm), Some(id)) => spools::spool(sm, id).await.ok().and_then(|f| f.color_hex),
                _ => None,
            };
            if let Some(h) = from_spool.or(s.hex) {
                swatches.push(h);
            }
        }
        out.push(ProjectSummary {
            id: r.id,
            name: r.name,
            template_name: r.template_name,
            due_date: r.due_date,
            status: r.status,
            closed_at: r.closed_at,
            totals: t,
            swatches,
        });
    }
    Ok(out)
}

fn active(row: &repo::project::ProjectRow) -> Result<(), AppError> {
    if row.status == "archived" {
        return Err(bad("The project is archived (read-only)."));
    }
    Ok(())
}

/// Put every print not yet printed or queued on the fleet queue. Returns how
/// many were added.
pub async fn print_all_remaining(state: &AppState, id: i64, user: &str) -> Result<u32, AppError> {
    let pool = &state.foreman_db.pool;
    let row = repo::project::get_project(pool, id).await?;
    active(&row)?;
    let mut added = 0;
    for p in repo::project::plates(pool, id).await? {
        let (c, _) = counts_of(pool, &p).await?;
        if c.remaining == 0 {
            continue;
        }
        let entry = state.library.get(p.file_id).await?;
        if entry.deleted_at.is_some() {
            return Err(bad(format!(
                "{}: its file is in the Library trash.",
                p.name
            )));
        }
        for _ in 0..c.remaining {
            let item = repo::queue::add(
                pool,
                p.file_id,
                &entry.name,
                None,
                QueueSource::Project,
                user,
            )
            .await?;
            repo::project::link_queue_item(pool, item.id, p.id).await?;
            added += 1;
        }
    }
    if added > 0 {
        repo::project::add_event(
            pool,
            id,
            &format!(
                "{added} print{} added to the fleet queue",
                if added == 1 { "" } else { "s" }
            ),
        )
        .await?;
        if row.status == "done" {
            repo::project::set_status(pool, id, "active").await?;
        }
        state.scheduler.queue_changed(None);
    }
    info!(project = id, added, "print all remaining");
    Ok(added)
}

/// Change a plate's copies. Never below printed + printing; queued prints above
/// the new count come off the queue (newest first).
pub async fn set_copies(state: &AppState, plate_id: i64, copies: u32) -> Result<(), AppError> {
    let pool = &state.foreman_db.pool;
    let p = repo::project::plate(pool, plate_id).await?;
    let row = repo::project::get_project(pool, p.project_id).await?;
    active(&row)?;
    let (c, items) = counts_of(pool, &p).await?;
    let min = projects::min_copies(&c);
    if copies < min {
        return Err(bad(format!(
            "{}: at least {min} — {} printed, {} printing.",
            p.name, c.printed, c.printing
        )));
    }
    if copies > MAX_COPIES * MAX_SETS {
        return Err(bad("That is too many copies."));
    }
    let drop = projects::unqueue_for(&c, copies);
    let mut queued: Vec<i64> = items
        .iter()
        .filter(|(_, st, _)| matches!(st, QueueStatus::Waiting | QueueStatus::Failed))
        .map(|(id, _, _)| *id)
        .collect();
    queued.sort_unstable_by(|a, b| b.cmp(a));
    for qid in queued.into_iter().take(drop as usize) {
        repo::queue::remove(pool, qid).await?;
    }
    repo::project::set_copies(pool, plate_id, copies).await?;
    repo::project::add_event(
        pool,
        p.project_id,
        &format!("{}: copies {} → {copies}", p.name, c.copies),
    )
    .await?;
    if drop > 0 {
        state.scheduler.queue_changed(None);
    }
    settle(pool, p.project_id).await?;
    Ok(())
}

/// Count one print of a plate as done without a job (printed elsewhere).
pub async fn mark_done(state: &AppState, plate_id: i64) -> Result<(), AppError> {
    let pool = &state.foreman_db.pool;
    let p = repo::project::plate(pool, plate_id).await?;
    active(&repo::project::get_project(pool, p.project_id).await?)?;
    let (c, _) = counts_of(pool, &p).await?;
    if c.printed + c.printing >= c.copies {
        return Err(bad(format!(
            "{}: every copy is printed or printing.",
            p.name
        )));
    }
    repo::project::add_printed(pool, plate_id).await?;
    repo::project::add_event(
        pool,
        p.project_id,
        &format!("{} marked done by hand", p.name),
    )
    .await?;
    settle(pool, p.project_id).await?;
    Ok(())
}

/// Close an active project whose prints are all done; reopen a done one that got
/// more copies.
pub async fn settle(pool: &SqlitePool, project_id: i64) -> Result<(), AppError> {
    let row = repo::project::get_project(pool, project_id).await?;
    if row.status == "archived" {
        return Ok(());
    }
    let mut all = Vec::new();
    for p in repo::project::plates(pool, project_id).await? {
        all.push(counts_of(pool, &p).await?.0);
    }
    let done = projects::all_done(&all);
    if done && row.status == "active" {
        repo::project::set_status(pool, project_id, "done").await?;
        repo::project::add_event(pool, project_id, "Every print is done · project closed").await?;
        info!(project = project_id, "project closed: every print done");
    } else if !done && row.status == "done" {
        repo::project::set_status(pool, project_id, "active").await?;
        repo::project::add_event(pool, project_id, "Reopened: more prints to do").await?;
    }
    Ok(())
}

/// Job engine hook: a queue item finished. A complete print counts for its plate;
/// a cancelled or failed one leaves the copy outstanding.
pub async fn print_ended(
    pool: &SqlitePool,
    item: &QueueItem,
    end: JobEnd,
    job_id: Option<i64>,
    printer_name: &str,
) -> Result<(), AppError> {
    let Some(plate_id) = item.project_plate_id else {
        return Ok(());
    };
    let p = match repo::project::plate(pool, plate_id).await {
        Ok(p) => p,
        Err(AppError::NotFound(_)) => return Ok(()),
        Err(e) => return Err(e),
    };
    if let Some(j) = job_id {
        repo::project::link_job(pool, j, plate_id).await?;
    }
    let text = match end {
        JobEnd::Complete | JobEnd::Running => {
            repo::project::add_printed(pool, plate_id).await?;
            let printed = repo::project::plate(pool, plate_id).await?.printed;
            format!(
                "{} finished on {printer_name} · {printed} of {} printed",
                p.name, p.copies
            )
        }
        JobEnd::Cancelled => format!("{} cancelled on {printer_name} · copy not counted", p.name),
        JobEnd::Error => format!("{} failed on {printer_name} · copy not counted", p.name),
    };
    repo::project::add_event(pool, p.project_id, &text).await?;
    settle(pool, p.project_id).await
}

pub async fn set_slot_spool(
    state: &AppState,
    project_id: i64,
    key: &str,
    spool_id: Option<i64>,
) -> Result<(), AppError> {
    let pool = &state.foreman_db.pool;
    active(&repo::project::get_project(pool, project_id).await?)?;
    let label = match (state.spoolman.get(), spool_id) {
        (Some(sm), Some(id)) => {
            let s = spools::spool(sm, id).await?;
            format!("#{} {}", s.id, s.name)
        }
        _ => "no spool".into(),
    };
    let key = slot_key(key);
    if !repo::project::set_slot_spool(pool, project_id, &key, spool_id).await? {
        return Err(AppError::NotFound(format!("slot {key}")));
    }
    repo::project::add_event(pool, project_id, &format!("{key} → {label}")).await?;
    Ok(())
}

/// `PATCH /api/projects/{id}`.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct UpdateProject {
    #[serde(default)]
    pub name: Option<String>,
    /// `""` clears it.
    #[serde(default)]
    pub due_date: Option<String>,
    /// `archived` or `active` (unarchive).
    #[serde(default)]
    pub status: Option<String>,
}

pub async fn update(state: &AppState, id: i64, req: &UpdateProject) -> Result<(), AppError> {
    let pool = &state.foreman_db.pool;
    let row = repo::project::get_project(pool, id).await?;
    let name = req.name.as_deref().map(str::trim);
    if name == Some("") {
        return Err(bad("Give the project a name."));
    }
    let due = match &req.due_date {
        None => None,
        Some(d) => Some(check_date(&Some(d.clone()))?),
    };
    if name.is_some() || due.is_some() {
        active(&row)?;
    }
    repo::project::update_project(pool, id, name, due.as_ref().map(|d| d.as_deref())).await?;
    match req.status.as_deref() {
        None => {}
        Some("archived") => {
            repo::project::set_status(pool, id, "archived").await?;
            repo::project::add_event(pool, id, "Archived").await?;
        }
        Some("active") if row.status == "archived" => {
            repo::project::set_status(pool, id, "active").await?;
            repo::project::add_event(pool, id, "Unarchived").await?;
            settle(pool, id).await?;
        }
        Some("active") => {}
        Some(s) => return Err(bad(format!("Unknown status {s}."))),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Reservations
// ---------------------------------------------------------------------------

/// Which spool each plate's tool draws from: plate id → (tool → spool).
pub async fn plate_spools(pool: &SqlitePool) -> Result<HashMap<i64, HashMap<u32, i64>>, AppError> {
    let mut out = HashMap::new();
    for pr in repo::project::list_projects(pool).await? {
        let slots: HashMap<String, i64> = repo::project::slots(pool, pr.id)
            .await?
            .into_iter()
            .filter_map(|s| Some((s.key, s.spool_id?)))
            .collect();
        for p in repo::project::plates(pool, pr.id).await? {
            let m: HashMap<u32, i64> = p
                .tools
                .iter()
                .filter_map(|t| Some((t.tool, *slots.get(&slot_key(&t.slot))?)))
                .collect();
            out.insert(p.id, m);
        }
    }
    Ok(out)
}

/// Project prints that are not on a queue yet, as demands on their slot spools
/// (active projects only, due date first).
pub async fn unqueued_demands(
    pool: &SqlitePool,
    lib: &dyn MetaSource,
    map: &HashMap<i64, HashMap<u32, i64>>,
) -> Result<Vec<JobDemand>, AppError> {
    let mut out = Vec::new();
    for pr in repo::project::list_projects(pool).await? {
        if pr.status != "active" {
            continue;
        }
        for p in repo::project::plates(pool, pr.id).await? {
            let (c, _) = counts_of(pool, &p).await?;
            if c.remaining == 0 {
                continue;
            }
            let meta = match lib.meta(p.file_id).await {
                Ok(Some(m)) => m,
                Ok(None) => continue,
                Err(e) => {
                    warn!(plate = p.id, "project plate file: {e}");
                    continue;
                }
            };
            let spools = map.get(&p.id);
            let tools: Vec<ToolDemand> = tool_grams(&meta.tools)
                .into_iter()
                .filter_map(|(tool, g)| {
                    Some(ToolDemand {
                        tool,
                        grams: g * f64::from(c.remaining),
                        spool_id: Some(*spools?.get(&tool)?),
                    })
                })
                .collect();
            if tools.is_empty() {
                continue;
            }
            out.push(JobDemand {
                queue_item_id: None,
                project_plate_id: Some(p.id),
                file_name: format!("{} · {}", pr.name, p.name),
                printer_id: None,
                tools,
                left: 1.0,
            });
        }
    }
    Ok(out)
}
