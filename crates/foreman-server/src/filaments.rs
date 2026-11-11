//! Vendors, filaments and new spools (task 4.2).
//!
//! The filament form writes Spoolman's own columns plus Foreman's extra fields:
//! - `material` = base material ("PETG"), `material_modifier` = modifiers ("CF HF"),
//! - `abrasive` = the explicit flag (auto-ticked from the modifiers, user-overridable),
//! - colour: single → `color_hex`; coextruded / gradient → `multi_color_hexes`
//!   + `multi_color_direction` (`coaxial` / `longitudinal`),
//! - `hue_td` (single colour only), `url`.
//!
//! Vendors get a `url` extra field (website / store). A new spool with no empty
//! weight takes the filament's, else the vendor's `empty_spool_weight`.

use std::collections::HashMap;

use foreman_core::{error::AppError, filament::SpoolFacts, materials};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use spoolman_rs::{
    db::{filament as sm_filament, spool as sm_spool, vendor as sm_vendor},
    error::SpoolmanError,
    models::{FilamentResponse, VendorResponse},
};
use ts_rs::TS;

use crate::spools::{self, Spoolman};

/// Most spools one "how many" request may create.
pub const MAX_QUANTITY: u32 = 50;

fn sm_err(what: &str) -> impl Fn(SpoolmanError) -> AppError + '_ {
    move |e| match e {
        SpoolmanError::NotFound(_) => AppError::NotFound(what.to_owned()),
        SpoolmanError::ValidationError(m) | SpoolmanError::BadRequest(m) => {
            AppError::InvalidInput(m)
        }
        e => AppError::Internal(format!("spoolman: {e}")),
    }
}

fn bad(msg: impl Into<String>) -> AppError {
    AppError::InvalidInput(msg.into())
}

/// Spoolman stores extra values as JSON text.
fn enc(v: Value) -> Value {
    Value::String(v.to_string())
}

fn dec(map: &Map<String, Value>, key: &str) -> Option<Value> {
    match map.get(key)? {
        Value::String(s) => serde_json::from_str(s)
            .ok()
            .or(Some(Value::String(s.clone()))),
        v => Some(v.clone()),
    }
}

fn dec_str(map: &Map<String, Value>, key: &str) -> Option<String> {
    match dec(map, key)? {
        Value::String(s) if !s.trim().is_empty() => Some(s),
        _ => None,
    }
}

fn trimmed(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

// ---------------------------------------------------------------------------
// Vendors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct VendorRow {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    /// Grams; prefills new spools of this vendor.
    pub empty_spool_weight: Option<f64>,
    pub website: Option<String>,
    pub comment: Option<String>,
    pub filament_count: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct VendorForm {
    pub name: String,
    #[serde(default)]
    pub empty_spool_weight: Option<f64>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
}

fn vendor_row(v: &VendorResponse, filament_count: u32) -> VendorRow {
    VendorRow {
        id: v.id,
        name: v.name.clone(),
        empty_spool_weight: v.empty_spool_weight,
        website: dec_str(&v.extra, "url"),
        comment: v.comment.clone().filter(|c| !c.trim().is_empty()),
        filament_count,
    }
}

async fn all_vendors(sm: &Spoolman) -> Result<Vec<VendorResponse>, AppError> {
    let filter = sm_vendor::VendorFilter::default();
    sm_vendor::find(&sm.db.pool, &sm.db.engine, &filter)
        .await
        .map(|(v, _)| v)
        .map_err(sm_err("vendors"))
}

async fn all_filaments(sm: &Spoolman) -> Result<Vec<FilamentResponse>, AppError> {
    let filter = sm_filament::FilamentFilter::default();
    sm_filament::find(&sm.db.pool, &sm.db.engine, &filter)
        .await
        .map(|(v, _)| v)
        .map_err(sm_err("filaments"))
}

/// Every vendor, by name, with how many filaments each has.
pub async fn vendors(sm: &Spoolman) -> Result<Vec<VendorRow>, AppError> {
    let mut counts: HashMap<i64, u32> = HashMap::new();
    for f in all_filaments(sm).await? {
        if let Some(v) = &f.vendor {
            *counts.entry(v.id).or_default() += 1;
        }
    }
    let mut rows: Vec<VendorRow> = all_vendors(sm)
        .await?
        .iter()
        .map(|v| vendor_row(v, counts.get(&v.id).copied().unwrap_or(0)))
        .collect();
    rows.sort_by_key(|v| v.name.to_lowercase());
    Ok(rows)
}

fn check_vendor(form: &VendorForm) -> Result<String, AppError> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err(bad("Vendor needs a name."));
    }
    if form
        .empty_spool_weight
        .is_some_and(|w| !(0.0..=5000.0).contains(&w))
    {
        return Err(bad("Empty spool weight must be between 0 and 5000 g."));
    }
    Ok(name.to_owned())
}

async fn vendor_by_name(sm: &Spoolman, name: &str) -> Result<Option<VendorResponse>, AppError> {
    Ok(all_vendors(sm)
        .await?
        .into_iter()
        .find(|v| v.name.trim().eq_ignore_ascii_case(name.trim())))
}

/// Create a vendor. Names are unique (case-insensitive).
pub async fn create_vendor(sm: &Spoolman, form: &VendorForm) -> Result<VendorRow, AppError> {
    let name = check_vendor(form)?;
    if vendor_by_name(sm, &name).await?.is_some() {
        return Err(bad(format!("\u{201c}{name}\u{201d} already exists.")));
    }
    let mut extra = Map::new();
    if let Some(url) = trimmed(&form.website) {
        extra.insert("url".into(), enc(json!(url)));
    }
    let req = serde_json::from_value(json!({
        "name": name,
        "empty_spool_weight": form.empty_spool_weight,
        "comment": trimmed(&form.comment),
        "extra": extra,
    }))
    .map_err(|e| bad(e.to_string()))?;
    let v = sm_vendor::create(&sm.db.pool, &sm.db.engine, &req)
        .await
        .map_err(sm_err("vendor"))?;
    spoolman_rs::ws::broadcast(&sm.event_tx, "added", "vendor", v.id, &v);
    Ok(vendor_row(&v, 0))
}

/// Edit a vendor; empty fields are cleared.
pub async fn update_vendor(
    sm: &Spoolman,
    id: i64,
    form: &VendorForm,
) -> Result<VendorRow, AppError> {
    let name = check_vendor(form)?;
    if let Some(other) = vendor_by_name(sm, &name).await? {
        if other.id != id {
            return Err(bad(format!("\u{201c}{name}\u{201d} already exists.")));
        }
    }
    let what = format!("vendor #{id}");
    let comment = trimmed(&form.comment);
    let req = serde_json::from_value(json!({
        "name": name,
        "empty_spool_weight": form.empty_spool_weight,
        "comment": comment,
        "extra": { "url": trimmed(&form.website).map(|u| enc(json!(u))).unwrap_or(Value::Null) },
    }))
    .map_err(|e| bad(e.to_string()))?;
    sm_vendor::update(&sm.db.pool, id, &req)
        .await
        .map_err(sm_err(&what))?;
    let mut clear = vec![];
    if form.empty_spool_weight.is_none() {
        clear.push("empty_spool_weight");
    }
    if comment.is_none() {
        clear.push("comment");
    }
    sm_vendor::clear_columns(&sm.db.pool, id, &clear)
        .await
        .map_err(sm_err(&what))?;
    let v = sm_vendor::find_by_id(&sm.db.pool, id)
        .await
        .map_err(sm_err(&what))?;
    spoolman_rs::ws::broadcast(&sm.event_tx, "updated", "vendor", v.id, &v);
    let count = all_filaments(sm)
        .await?
        .iter()
        .filter(|f| f.vendor.as_ref().is_some_and(|x| x.id == id))
        .count() as u32;
    Ok(vendor_row(&v, count))
}

// ---------------------------------------------------------------------------
// Filaments
// ---------------------------------------------------------------------------

/// How the colours of a filament are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ColourKind {
    /// One colour (`color_hex`).
    Single,
    /// Coextruded: 2–3 colours side by side (`multi_color_direction = coaxial`).
    Coaxial,
    /// Gradient: 2–6 colours along the spool (`longitudinal`).
    Longitudinal,
}

impl ColourKind {
    fn range(self) -> (usize, usize) {
        match self {
            Self::Single => (1, 1),
            Self::Coaxial => (2, 3),
            Self::Longitudinal => (2, 6),
        }
    }
    fn direction(self) -> Option<&'static str> {
        match self {
            Self::Single => None,
            Self::Coaxial => Some("coaxial"),
            Self::Longitudinal => Some("longitudinal"),
        }
    }
}

/// One filament as the Filaments tab and the form show it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct FilamentRow {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    #[ts(type = "number | null")]
    pub vendor_id: Option<i64>,
    pub vendor: Option<String>,
    pub material: Option<String>,
    pub modifiers: Vec<String>,
    pub abrasive: bool,
    pub colour_kind: ColourKind,
    /// `RRGGBB`, no `#`; one for single colour.
    pub colours: Vec<String>,
    pub hue_td: Option<f64>,
    #[ts(type = "number | null")]
    pub extruder_temp: Option<i64>,
    #[ts(type = "number | null")]
    pub bed_temp: Option<i64>,
    pub density: f64,
    pub diameter: f64,
    pub price: Option<f64>,
    pub url: Option<String>,
    pub article_number: Option<String>,
    pub comment: Option<String>,
    /// Empty spool weight a new spool gets: filament's, else the vendor's.
    pub spool_weight: Option<f64>,
    /// Non-archived spools of it.
    pub spool_count: u32,
    pub grams_left: f64,
    /// Net filament weight of a full spool, g.
    pub weight: Option<f64>,
}

/// What the filament form sends (create and edit).
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct FilamentForm {
    /// An existing vendor…
    #[serde(default)]
    #[ts(type = "number | null")]
    pub vendor_id: Option<i64>,
    /// …or a name typed inline: an existing vendor with that name is used,
    /// otherwise one is created with the filament.
    #[serde(default)]
    pub vendor_name: Option<String>,
    pub name: String,
    /// Base material, e.g. "PETG".
    pub material: String,
    #[serde(default)]
    pub modifiers: Vec<String>,
    /// `None` → auto from the modifiers (CF, GF, AF, Metal, Glow).
    #[serde(default)]
    pub abrasive: Option<bool>,
    pub colour_kind: ColourKind,
    /// `RRGGBB` or `#RRGGBB`.
    pub colours: Vec<String>,
    /// HueForge TD; single colour only.
    #[serde(default)]
    pub hue_td: Option<f64>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub extruder_temp: Option<i64>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub bed_temp: Option<i64>,
    /// Defaults from the material when missing.
    #[serde(default)]
    pub density: Option<f64>,
    #[serde(default)]
    pub diameter: Option<f64>,
    #[serde(default)]
    pub price: Option<f64>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub article_number: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
}

/// `#rrggbb` → `RRGGBB`, or an error naming the bad value.
fn hex(c: &str) -> Result<String, AppError> {
    let h = c.trim().trim_start_matches('#').to_ascii_uppercase();
    if h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(h)
    } else {
        Err(bad(format!(
            "\u{201c}{c}\u{201d} is not a colour (RRGGBB)."
        )))
    }
}

/// Spoolman's colour columns: (color_hex, multi_color_hexes, direction).
type ColourColumns = (Option<String>, Option<String>, Option<&'static str>);

/// Colour columns from a form.
fn colour_columns(form: &FilamentForm) -> Result<ColourColumns, AppError> {
    let cols = form
        .colours
        .iter()
        .map(|c| hex(c))
        .collect::<Result<Vec<_>, _>>()?;
    let (min, max) = form.colour_kind.range();
    if cols.len() < min || cols.len() > max {
        let kind = match form.colour_kind {
            ColourKind::Single => "Single colour needs exactly 1 colour.".to_owned(),
            ColourKind::Coaxial => "Coextruded needs 2 or 3 colours.".to_owned(),
            ColourKind::Longitudinal => "Gradient needs 2 to 6 colours.".to_owned(),
        };
        return Err(bad(kind));
    }
    Ok(match form.colour_kind {
        ColourKind::Single => (cols.first().cloned(), None, None),
        k => (None, Some(cols.join(",")), k.direction()),
    })
}

fn row(f: &FilamentResponse, spools: &[SpoolFacts]) -> FilamentRow {
    let modifiers = dec_str(&f.extra, "material_modifier")
        .map(|m| materials::split_modifiers(&m))
        .unwrap_or_default();
    let abrasive = match dec(&f.extra, "abrasive") {
        Some(Value::Bool(b)) => b,
        _ => materials::is_abrasive_filament(
            f.material.as_deref(),
            Some(&materials::join_modifiers(&modifiers)),
        ),
    };
    let multi: Vec<String> = f
        .multi_color_hexes
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(foreman_core::filament::norm_hex)
        .filter(|h| !h.is_empty())
        .collect();
    let (colour_kind, colours) = if multi.len() > 1 {
        let k = if f.multi_color_direction.as_deref() == Some("coaxial") {
            ColourKind::Coaxial
        } else {
            ColourKind::Longitudinal
        };
        (k, multi)
    } else {
        let c = f
            .color_hex
            .as_deref()
            .map(foreman_core::filament::norm_hex)
            .filter(|h| !h.is_empty());
        (ColourKind::Single, c.into_iter().collect())
    };
    let mine: Vec<&SpoolFacts> = spools
        .iter()
        .filter(|s| s.filament_id == f.id && !s.archived)
        .collect();
    FilamentRow {
        id: f.id,
        name: f
            .name
            .clone()
            .unwrap_or_else(|| format!("Filament #{}", f.id)),
        vendor_id: f.vendor.as_ref().map(|v| v.id),
        vendor: f.vendor.as_ref().map(|v| v.name.clone()),
        material: f.material.clone(),
        modifiers,
        abrasive,
        colour_kind,
        colours,
        hue_td: match dec(&f.extra, "hue_td") {
            Some(Value::Number(n)) => n.as_f64(),
            _ => None,
        },
        extruder_temp: f.settings_extruder_temp,
        bed_temp: f.settings_bed_temp,
        density: f.density,
        diameter: f.diameter,
        price: f.price,
        url: dec_str(&f.extra, "url"),
        article_number: f.article_number.clone(),
        comment: f.comment.clone().filter(|c| !c.trim().is_empty()),
        spool_weight: f
            .spool_weight
            .or_else(|| f.vendor.as_ref().and_then(|v| v.empty_spool_weight)),
        spool_count: mine.len() as u32,
        grams_left: mine.iter().filter_map(|s| s.remaining_weight).sum(),
        weight: f.weight,
    }
}

/// Every filament, vendor then name.
pub async fn filaments(sm: &Spoolman) -> Result<Vec<FilamentRow>, AppError> {
    let spools = spools::all(sm, false).await?;
    let mut rows: Vec<FilamentRow> = all_filaments(sm)
        .await?
        .iter()
        .map(|f| row(f, &spools))
        .collect();
    rows.sort_by_key(|f| {
        (
            f.vendor.clone().unwrap_or_default().to_lowercase(),
            f.name.to_lowercase(),
        )
    });
    Ok(rows)
}

/// One filament.
pub async fn filament(sm: &Spoolman, id: i64) -> Result<FilamentRow, AppError> {
    let what = format!("filament #{id}");
    let f = sm_filament::find_by_id(&sm.db.pool, id)
        .await
        .map_err(sm_err(&what))?;
    let spools = spools::all(sm, false).await?;
    Ok(row(&f, &spools))
}

/// Vendor id for a form: the given id, an existing vendor by name, or a new one.
async fn resolve_vendor(sm: &Spoolman, form: &FilamentForm) -> Result<Option<i64>, AppError> {
    if let Some(id) = form.vendor_id {
        sm_vendor::find_by_id(&sm.db.pool, id)
            .await
            .map_err(sm_err(&format!("vendor #{id}")))?;
        return Ok(Some(id));
    }
    let Some(name) = trimmed(&form.vendor_name) else {
        return Ok(None);
    };
    if let Some(v) = vendor_by_name(sm, &name).await? {
        return Ok(Some(v.id));
    }
    let v = create_vendor(
        sm,
        &VendorForm {
            name,
            empty_spool_weight: None,
            website: None,
            comment: None,
        },
    )
    .await?;
    Ok(Some(v.id))
}

struct Checked {
    name: String,
    material: String,
    modifiers: String,
    abrasive: bool,
    colours: ColourColumns,
    hue_td: Option<f64>,
    density: f64,
    diameter: f64,
}

fn check_filament(form: &FilamentForm) -> Result<Checked, AppError> {
    let name = form.name.trim().to_owned();
    if name.is_empty() {
        return Err(bad("Filament needs a name."));
    }
    let material = form.material.trim().to_owned();
    if material.is_empty() {
        return Err(bad("Pick a material."));
    }
    let mods: Vec<String> = form
        .modifiers
        .iter()
        .flat_map(|m| materials::split_modifiers(m))
        .collect();
    let modifiers = materials::join_modifiers(&mods);
    let abrasive = form
        .abrasive
        .unwrap_or_else(|| materials::auto_abrasive(&mods));
    let colours = colour_columns(form)?;
    // HueForge needs one colour per filament: TD only for single colour.
    let hue_td = match form.colour_kind {
        ColourKind::Single => form.hue_td,
        _ => None,
    };
    if hue_td.is_some_and(|t| !(0.0..=100.0).contains(&t)) {
        return Err(bad("HueForge TD must be between 0 and 100."));
    }
    let info = materials::material_info(&material);
    let density = form
        .density
        .or(info.as_ref().map(|i| i.density))
        .unwrap_or(1.24);
    let diameter = form.diameter.unwrap_or(1.75);
    if density <= 0.0 || diameter <= 0.0 {
        return Err(bad("Density and diameter must be more than 0."));
    }
    if form.price.is_some_and(|p| p < 0.0) {
        return Err(bad("Price can't be negative."));
    }
    Ok(Checked {
        name,
        material,
        modifiers,
        abrasive,
        colours,
        hue_td,
        density,
        diameter,
    })
}

/// Foreman's extra fields for a form. `nulls` → removed values are sent as
/// `null` (edit), else left out (create).
fn filament_extra(c: &Checked, form: &FilamentForm, nulls: bool) -> Map<String, Value> {
    let mut m = Map::new();
    let mut put = |k: &str, v: Option<Value>| match v {
        Some(v) => {
            m.insert(k.into(), enc(v));
        }
        None if nulls => {
            m.insert(k.into(), Value::Null);
        }
        None => {}
    };
    put("abrasive", Some(json!(c.abrasive)));
    put(
        "material_modifier",
        (!c.modifiers.is_empty()).then(|| json!(c.modifiers)),
    );
    put("hue_td", c.hue_td.map(|t| json!(t)));
    put("url", trimmed(&form.url).map(|u| json!(u)));
    m
}

/// Create a filament (and its vendor when named inline).
pub async fn create_filament(sm: &Spoolman, form: &FilamentForm) -> Result<FilamentRow, AppError> {
    let c = check_filament(form)?;
    let vendor_id = resolve_vendor(sm, form).await?;
    let (color_hex, multi, dir) = &c.colours;
    let req = serde_json::from_value(json!({
        "name": c.name,
        "vendor_id": vendor_id,
        "material": c.material,
        "density": c.density,
        "diameter": c.diameter,
        "price": form.price,
        "article_number": trimmed(&form.article_number),
        "comment": trimmed(&form.comment),
        "settings_extruder_temp": form.extruder_temp,
        "settings_bed_temp": form.bed_temp,
        "color_hex": color_hex,
        "multi_color_hexes": multi,
        "multi_color_direction": dir,
        "extra": filament_extra(&c, form, false),
    }))
    .map_err(|e| bad(e.to_string()))?;
    let f = sm_filament::create(&sm.db.pool, &sm.db.engine, &req)
        .await
        .map_err(sm_err("filament"))?;
    spoolman_rs::ws::broadcast(&sm.event_tx, "added", "filament", f.id, &f);
    filament(sm, f.id).await
}

/// Edit a filament; every spool of it changes with it.
pub async fn update_filament(
    sm: &Spoolman,
    id: i64,
    form: &FilamentForm,
) -> Result<FilamentRow, AppError> {
    let what = format!("filament #{id}");
    let c = check_filament(form)?;
    let vendor_id = resolve_vendor(sm, form).await?;
    let article = trimmed(&form.article_number);
    let comment = trimmed(&form.comment);
    let req = serde_json::from_value(json!({
        "name": c.name,
        "vendor_id": vendor_id,
        "material": c.material,
        "density": c.density,
        "diameter": c.diameter,
        "price": form.price,
        "article_number": article,
        "comment": comment,
        "settings_extruder_temp": form.extruder_temp,
        "settings_bed_temp": form.bed_temp,
        "extra": filament_extra(&c, form, true),
    }))
    .map_err(|e| bad(e.to_string()))?;
    sm_filament::update(&sm.db.pool, id, &req)
        .await
        .map_err(sm_err(&what))?;
    let (color_hex, multi, dir) = &c.colours;
    sm_filament::set_colours(
        &sm.db.pool,
        id,
        color_hex.as_deref(),
        multi.as_deref(),
        *dir,
    )
    .await
    .map_err(sm_err(&what))?;
    let mut clear = vec![];
    for (none, col) in [
        (vendor_id.is_none(), "vendor_id"),
        (form.price.is_none(), "price"),
        (article.is_none(), "article_number"),
        (comment.is_none(), "comment"),
        (form.extruder_temp.is_none(), "settings_extruder_temp"),
        (form.bed_temp.is_none(), "settings_bed_temp"),
    ] {
        if none {
            clear.push(col);
        }
    }
    sm_filament::clear_columns(&sm.db.pool, id, &clear)
        .await
        .map_err(sm_err(&what))?;
    let f = sm_filament::find_by_id(&sm.db.pool, id)
        .await
        .map_err(sm_err(&what))?;
    spoolman_rs::ws::broadcast(&sm.event_tx, "updated", "filament", f.id, &f);
    filament(sm, id).await
}

// ---------------------------------------------------------------------------
// New spools
// ---------------------------------------------------------------------------

/// `POST /api/spools`: N identical spools of an existing filament, or of a new
/// one created in the same request.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct CreateSpools {
    #[serde(default)]
    #[ts(type = "number | null")]
    pub filament_id: Option<i64>,
    /// Create this filament first (when the search found no match).
    #[serde(default)]
    pub new_filament: Option<FilamentForm>,
    /// Net filament weight when full, g.
    #[serde(default)]
    pub initial_weight: Option<f64>,
    /// Empty spool weight, g. Missing → filament's, else the vendor's.
    #[serde(default)]
    pub spool_weight: Option<f64>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub lot_nr: Option<String>,
    #[serde(default)]
    pub price: Option<f64>,
    #[serde(default)]
    pub comment: Option<String>,
    /// How many identical spools (1–50). Default 1.
    #[serde(default)]
    pub quantity: Option<u32>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct CreatedSpools {
    pub filament: FilamentRow,
    pub spools: Vec<SpoolFacts>,
    /// A filament was created by this request.
    pub filament_created: bool,
}

/// `POST /api/spools/{id}/clone`.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct CloneSpool {
    #[serde(default)]
    pub quantity: Option<u32>,
}

fn quantity(q: Option<u32>) -> Result<u32, AppError> {
    let q = q.unwrap_or(1);
    if q == 0 || q > MAX_QUANTITY {
        return Err(bad(format!("How many spools: 1 to {MAX_QUANTITY}.")));
    }
    Ok(q)
}

struct SpoolFields {
    filament_id: i64,
    initial_weight: Option<f64>,
    spool_weight: Option<f64>,
    location: Option<String>,
    lot_nr: Option<String>,
    price: Option<f64>,
    comment: Option<String>,
}

async fn insert_spools(
    sm: &Spoolman,
    s: &SpoolFields,
    n: u32,
) -> Result<Vec<SpoolFacts>, AppError> {
    for (v, what) in [
        (s.initial_weight, "Net weight"),
        (s.spool_weight, "Empty spool weight"),
        (s.price, "Price"),
    ] {
        if v.is_some_and(|v| v < 0.0) {
            return Err(bad(format!("{what} can't be negative.")));
        }
    }
    let req = serde_json::from_value(json!({
        "filament_id": s.filament_id,
        "initial_weight": s.initial_weight,
        "remaining_weight": s.initial_weight,
        "spool_weight": s.spool_weight,
        "location": s.location,
        "lot_nr": s.lot_nr,
        "price": s.price,
        "comment": s.comment,
    }))
    .map_err(|e| bad(e.to_string()))?;
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let sp = sm_spool::create(&sm.db.pool, &sm.db.engine, &req)
            .await
            .map_err(sm_err("spool"))?;
        spoolman_rs::ws::broadcast(&sm.event_tx, "added", "spool", sp.id, &sp);
        out.push(spools::facts(&sp));
    }
    Ok(out)
}

/// Create N spools (and the filament first when asked).
pub async fn create_spools(sm: &Spoolman, req: &CreateSpools) -> Result<CreatedSpools, AppError> {
    let n = quantity(req.quantity)?;
    let (fil, created) = match (req.filament_id, &req.new_filament) {
        (Some(_), Some(_)) => {
            return Err(bad("Send filament_id or new_filament, not both."));
        }
        (Some(id), None) => (filament(sm, id).await?, false),
        (None, Some(form)) => (create_filament(sm, form).await?, true),
        (None, None) => return Err(bad("Pick a filament or create one.")),
    };
    let spools = insert_spools(
        sm,
        &SpoolFields {
            filament_id: fil.id,
            initial_weight: req.initial_weight,
            spool_weight: req.spool_weight.or(fil.spool_weight),
            location: trimmed(&req.location),
            lot_nr: trimmed(&req.lot_nr),
            price: req.price.or(fil.price),
            comment: trimmed(&req.comment),
        },
        n,
    )
    .await?;
    Ok(CreatedSpools {
        filament: filament(sm, fil.id).await?,
        spools,
        filament_created: created,
    })
}

/// Copy a spool N times: same filament, weights, location, lot and price; full.
pub async fn clone_spool(
    sm: &Spoolman,
    id: i64,
    req: &CloneSpool,
) -> Result<Vec<SpoolFacts>, AppError> {
    let n = quantity(req.quantity)?;
    let src = sm_spool::find_by_id(&sm.db.pool, id)
        .await
        .map_err(sm_err(&format!("spool #{id}")))?;
    insert_spools(
        sm,
        &SpoolFields {
            filament_id: src.filament.id,
            initial_weight: src.initial_weight.or(src.filament.weight),
            spool_weight: src.spool_weight,
            location: src.location.clone(),
            lot_nr: src.lot_nr.clone(),
            price: src.price,
            comment: src.comment.clone(),
        },
        n,
    )
    .await
}

/// Every spool location in use.
pub async fn locations(sm: &Spoolman) -> Result<Vec<String>, AppError> {
    let mut v = sm_spool::find_locations(&sm.db.pool)
        .await
        .map_err(sm_err("locations"))?;
    v.retain(|l| !l.trim().is_empty());
    v.sort_by_key(|l| l.to_lowercase());
    Ok(v)
}
