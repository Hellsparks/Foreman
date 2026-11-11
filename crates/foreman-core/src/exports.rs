//! Exports (W11): OrcaSlicer filament profiles and the HueForge catalogue.
//!
//! ## OrcaSlicer (checked against OrcaSlicer v2.5.0 source)
//!
//! `PresetBundle::import_json_presets` loads a user filament preset from JSON
//! when it has `name`, a semver `version`, `filament_settings_id`, and an
//! `inherits` that names an existing preset (otherwise the preset is skipped).
//! Values are arrays of strings. Keys listed in Orca's
//! `filament_options_with_variant` hold one value per extruder variant: 6 for
//! every OrcaFilamentLibrary parent used here.
//!
//! Each profile inherits an OrcaFilamentLibrary `@System` preset (available on
//! every printer): the Bambu product preset for Bambu Lab filaments, else
//! `Generic <type>`. Orca 2.5 matches an AMS tray to a preset through the
//! preset's root, so the parent decides the match; its Orca `filament_id`
//! (`OF…`) is copied into the profile as Orca does for its own user presets,
//! and the Bambu id (`GF…`, from Orca's `bambu_filament_ids.json`) is reported
//! and written to the notes.
//!
//! A zip without `bundle_structure.json` imports into the user preset folder
//! (File → Import → Import Configs…). Every entry must be a JSON preset.
//!
//! ## HueForge
//!
//! `{"Filaments": [{Brand, Color, Name, Owned, Tags, Transmissivity, Type,
//! uuid}]}` — the format of HueForge's own `Filaments/*.json` libraries
//! (lower-case `#rrggbb`, `uuid` in braces). One TD per filament, so
//! multi-colour filaments are left out.

use std::io::Write;

use serde::Serialize;
use serde_json::{json, Map, Value};
use ts_rs::TS;

use crate::{calibration::orca_num, error::AppError, materials};

/// Orca version written into profiles (`Semver`).
pub const ORCA_VERSION: &str = "2.5.0.0";
/// Extruder variants of the OrcaFilamentLibrary parents (Orca 2.5).
pub const ORCA_VARIANTS: usize = 6;

/// (OrcaFilamentLibrary preset, Orca filament_id, Bambu filament id) — OrcaSlicer v2.5.0
/// `resources/profiles/OrcaFilamentLibrary/filament` + `resources/printers/bambu_filament_ids.json`.
pub const PARENTS: &[(&str, &str, Option<&str>)] = &[
    ("Bambu ABS @System", "OFhuaUQB", Some("GFB00")),
    ("Bambu ABS-GF @System", "OFknl9Iz", Some("GFB50")),
    ("Bambu ASA @System", "OFfBpSRI", Some("GFB01")),
    ("Bambu ASA-Aero @System", "OFXzQ4yL", Some("GFB02")),
    ("Bambu ASA-CF @System", "OF0wBGNx", Some("GFB51")),
    ("Bambu PA-CF @System", "OFFNYwWR", Some("GFN03")),
    ("Bambu PA6-CF @System", "OFniMuTN", Some("GFN05")),
    ("Bambu PA6-GF @System", "OFNk8bxk", Some("GFN08")),
    ("Bambu PAHT-CF @System", "OF2VFe4J", Some("GFN04")),
    ("Bambu PC @System", "OFKhMPeX", Some("GFC00")),
    ("Bambu PC FR @System", "OFg57Nmc", Some("GFC01")),
    ("Bambu PET-CF @System", "OFHa48An", Some("GFT01")),
    ("Bambu PETG Basic @System", "OFFvzqcd", Some("GFG00")),
    ("Bambu PETG HF @System", "OFovEIbw", Some("GFG02")),
    ("Bambu PETG Matte @System", "OFdyfQvU", Some("GFG03")),
    ("Bambu PETG Translucent @System", "OFwPrlCM", Some("GFG01")),
    ("Bambu PETG-CF @System", "OFTRZ8Y4", Some("GFG50")),
    ("Bambu PLA Aero @System", "OFBSW57R", Some("GFA11")),
    ("Bambu PLA Basic @System", "OFoiVqVM", Some("GFA00")),
    ("Bambu PLA Dynamic @System", "OFDGigEI", Some("GFA13")),
    ("Bambu PLA Galaxy @System", "OFQ3e56w", Some("GFA15")),
    ("Bambu PLA Glow @System", "OFEkPBwx", Some("GFA12")),
    ("Bambu PLA Impact @System", "OFTGHyNC", None),
    ("Bambu PLA Lite @System", "OFnfxTvi", Some("GFA18")),
    ("Bambu PLA Marble @System", "OF9OlTWH", Some("GFA07")),
    ("Bambu PLA Matte @System", "OFXIbw5D", Some("GFA01")),
    ("Bambu PLA Metal @System", "OFrKLeE3", Some("GFA02")),
    ("Bambu PLA Pure @System", "OFTHDCqA", Some("GFA19")),
    ("Bambu PLA Silk @System", "OFEGNJD4", Some("GFA05")),
    ("Bambu PLA Silk+ @System", "OFUanySo", Some("GFA06")),
    ("Bambu PLA Sparkle @System", "OFDxfPgH", Some("GFA08")),
    ("Bambu PLA Tough @System", "OFaQMgRH", Some("GFA09")),
    ("Bambu PLA Tough+ @System", "OF342jVN", Some("GFA10")),
    ("Bambu PLA Translucent @System", "OFFbSnCD", Some("GFA17")),
    ("Bambu PLA Wood @System", "OFMjtqTC", Some("GFA16")),
    ("Bambu PLA-CF @System", "OFEswT5W", Some("GFA50")),
    ("Bambu PPA-CF @System", "OF54B0S0", Some("GFN06")),
    ("Bambu PVA @System", "OFzyIxba", Some("GFS04")),
    ("Bambu TPU 85A @System", "OFlfuj2k", Some("GFU04")),
    ("Bambu TPU 90A @System", "OFtkLO6q", Some("GFU03")),
    ("Bambu TPU 95A @System", "OFVCkX5w", Some("GFU01")),
    ("Bambu TPU 95A HF @System", "OFvKUnLh", Some("GFU00")),
    ("Bambu TPU for AMS @System", "OFIBbYO5", Some("GFU02")),
    ("Generic ABS @System", "OFY9muEs", Some("GFB99")),
    ("Generic ASA @System", "OFLPAxz3", Some("GFB98")),
    ("Generic BVOH @System", "OFo2UF2C", Some("GFS97")),
    ("Generic CoPE @System", "OFjt9en1", None),
    ("Generic EVA @System", "OFCD8qiU", Some("GFR99")),
    ("Generic HIPS @System", "OFsFon5l", Some("GFS98")),
    ("Generic PA @System", "OFg8ndtj", Some("GFN99")),
    ("Generic PA-CF @System", "OFQLcbps", Some("GFN98")),
    ("Generic PC @System", "OFLakOUI", Some("GFC99")),
    ("Generic PCTG @System", "OFu1evlr", Some("GFG97")),
    ("Generic PE @System", "OFPklMI1", Some("GFP99")),
    ("Generic PE-CF @System", "OFd0Fv0k", Some("GFP98")),
    ("Generic PETG @System", "OFYPdQJh", Some("GFG99")),
    ("Generic PETG HF @System", "OF7lOgYF", Some("GFG96")),
    ("Generic PETG-CF @System", "OFoYSJKi", Some("GFG98")),
    ("Generic PHA @System", "OF74KeQR", Some("GFR98")),
    ("Generic PLA @System", "OFDSrzZ8", Some("GFL99")),
    ("Generic PLA High Speed @System", "OFmpMwxS", Some("GFL95")),
    ("Generic PLA Matte @System", "OFmY0l6t", None),
    ("Generic PLA Silk @System", "OFesA6rF", Some("GFL96")),
    ("Generic PLA-CF @System", "OFWbdGsC", Some("GFL98")),
    ("Generic PP @System", "OF1UNk9P", Some("GFP97")),
    ("Generic PP-CF @System", "OFXkm8q1", Some("GFP96")),
    ("Generic PP-GF @System", "OFsijjtH", Some("GFP95")),
    ("Generic PPA-CF @System", "OF8tPByX", Some("GFN97")),
    ("Generic PPA-GF @System", "OF8Tlg47", Some("GFN96")),
    ("Generic PVA @System", "OFDvXujf", Some("GFS99")),
    ("Generic SBS @System", "OFhQfUQY", None),
    ("Generic TPU @System", "OFgbpcy9", Some("GFU99")),
];

/// One filament as the exporters need it (a flattened Spoolman filament).
#[derive(Debug, Clone, Default)]
pub struct ExportFilament {
    pub id: i64,
    pub name: String,
    pub vendor: Option<String>,
    /// Base material as stored ("PETG", "PETG HF", "TPU95A").
    pub material: Option<String>,
    /// `material_modifier` split ("CF", "Silk").
    pub modifiers: Vec<String>,
    /// `RRGGBB`; more than one = multi-colour.
    pub colours: Vec<String>,
    pub density: f64,
    pub diameter: f64,
    pub extruder_temp: Option<i64>,
    pub bed_temp: Option<i64>,
    /// Spool price and net weight (g), for cost per kg.
    pub price: Option<f64>,
    pub weight: Option<f64>,
    /// `orcaslicer_config` extra field (JSON object text).
    pub orca_config: Option<String>,
    pub hue_td: Option<f64>,
    /// Has a non-archived spool with filament left.
    pub owned: bool,
}

impl ExportFilament {
    fn material_label(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(m) = self
            .material
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            parts.push(m.to_owned());
        }
        for x in &self.modifiers {
            let lower = parts.join(" ").to_ascii_lowercase();
            if !lower.split(' ').any(|t| t.eq_ignore_ascii_case(x)) {
                parts.push(x.clone());
            }
        }
        parts.join(" ")
    }
    fn multi_colour(&self) -> bool {
        self.colours.len() > 1
    }
}

// ---------------------------------------------------------------------------
// OrcaSlicer
// ---------------------------------------------------------------------------

/// What an Orca profile was built from, for the UI and the summary.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct OrcaProfileInfo {
    #[ts(type = "number")]
    pub filament_id: i64,
    /// Preset name in Orca.
    pub name: String,
    /// File name inside the zip.
    pub file: String,
    /// Parent preset (`inherits`).
    pub inherits: String,
    /// Orca `filament_id` of the parent (`OF…`).
    pub orca_filament_id: String,
    /// Bambu filament id (`GF…`) when Orca maps one.
    pub bambu_filament_id: Option<String>,
    pub shrinkage_xy: Option<f64>,
    pub shrinkage_z: Option<f64>,
    pub flow_ratio: Option<f64>,
    pub multi_colour: bool,
    /// Plain-language notes (fallback parent, unknown keys…).
    pub notes: Vec<String>,
}

fn parent(name: &str) -> Option<&'static (&'static str, &'static str, Option<&'static str>)> {
    PARENTS.iter().find(|p| p.0.eq_ignore_ascii_case(name))
}

/// Upper-case word tokens of a string ("PETG-CF HF" → PETG, CF, HF).
fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_ascii_alphanumeric() && c != '+')
        .filter(|t| !t.is_empty())
        .map(str::to_ascii_uppercase)
        .collect()
}

/// The OrcaFilamentLibrary preset a filament inherits, plus a note when it is
/// a fallback.
fn pick_parent(
    f: &ExportFilament,
    cfg: &Map<String, Value>,
) -> (
    &'static (&'static str, &'static str, Option<&'static str>),
    Option<String>,
) {
    // 1. An explicit id in orcaslicer_config (Orca `OF…` or Bambu `GF…`).
    for key in ["filament_id", "bambu_filament_id"] {
        if let Some(id) = cfg.get(key).and_then(Value::as_str).map(str::trim) {
            if let Some(p) = PARENTS
                .iter()
                .find(|p| p.1 == id || p.2.is_some_and(|b| b.eq_ignore_ascii_case(id)))
            {
                return (p, None);
            }
        }
    }
    let label = f.material_label();
    let all = tokens(&format!("{label} {}", f.name));
    // 2. Bambu Lab products: the longest series whose words all appear.
    let bambu = f
        .vendor
        .as_deref()
        .is_some_and(|v| v.to_ascii_lowercase().starts_with("bambu"));
    if bambu {
        let best = PARENTS
            .iter()
            .filter(|p| p.0.starts_with("Bambu "))
            .filter(|p| {
                let series =
                    p.0.trim_start_matches("Bambu ")
                        .trim_end_matches(" @System");
                tokens(series).iter().all(|t| all.contains(t))
            })
            .max_by_key(|p| p.0.len());
        if let Some(p) = best {
            return (p, None);
        }
    }
    // 3. Generic by base polymer and modifiers.
    let base = materials::normalize_filament_type(f.material.as_deref().unwrap_or(""));
    let base = match base.as_str() {
        "" => "PLA".to_owned(),
        b if b.starts_with("PA") && b != "PA" && !b.starts_with("PAHT") => {
            // PA6 / PA12 / PA11 → PA
            if b[2..].chars().all(|c| c.is_ascii_digit()) {
                "PA".into()
            } else {
                b.to_owned()
            }
        }
        "TPE" => "TPU".into(),
        b => b.to_owned(),
    };
    let lt = tokens(&label);
    let has = |t: &str| {
        lt.iter()
            .any(|x| x == t || x.starts_with(t) && x[t.len()..].chars().all(|c| c.is_ascii_digit()))
    };
    let mut candidates: Vec<String> = Vec::new();
    for fill in ["CF", "GF"] {
        if has(fill) {
            candidates.push(format!("Generic {base}-{fill} @System"));
        }
    }
    if has("HF") {
        candidates.push(format!("Generic {base} HF @System"));
    }
    if has("SILK") {
        candidates.push(format!("Generic {base} Silk @System"));
    }
    if has("MATTE") {
        candidates.push(format!("Generic {base} Matte @System"));
    }
    if has("HS") || (has("HIGH") && has("SPEED")) {
        candidates.push(format!("Generic {base} High Speed @System"));
    }
    candidates.push(format!("Generic {base} @System"));
    // "TPU95A" → "TPU": the leading letters.
    let letters: String = base
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    if !letters.is_empty() && letters != base {
        candidates.push(format!("Generic {letters} @System"));
    }
    for c in &candidates {
        if let Some(p) = parent(c) {
            return (p, None);
        }
    }
    let fallback = parent("Generic PLA @System").unwrap_or(&PARENTS[0]);
    (
        fallback,
        Some(format!(
            "No Orca preset for \u{201c}{label}\u{201d}; exported on Generic PLA. Check the material in Orca."
        )),
    )
}

fn cfg_num(cfg: &Map<String, Value>, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| match cfg.get(*k) {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().trim_end_matches('%').trim().parse().ok(),
        _ => None,
    })
}

fn cfg_bool(cfg: &Map<String, Value>, key: &str) -> Option<bool> {
    match cfg.get(key) {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::String(s)) => match s.trim() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        },
        Some(Value::Number(n)) => n.as_f64().map(|v| v != 0.0),
        _ => None,
    }
}

fn arr(v: impl Into<String>) -> Value {
    json!([v.into()])
}

fn variants(v: impl Into<String>) -> Value {
    let v = v.into();
    Value::Array(vec![Value::String(v); ORCA_VARIANTS])
}

/// A file/preset name Orca accepts (no path separators or reserved characters).
fn safe_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Orca preset name: "NextShapes PETG HF Silver (Foreman #236)".
pub fn orca_preset_name(f: &ExportFilament) -> String {
    let label = f.material_label();
    let mut parts: Vec<String> = Vec::new();
    if let Some(v) = f.vendor.as_deref().filter(|v| !v.trim().is_empty()) {
        parts.push(v.trim().to_owned());
    }
    if !label.is_empty()
        && !f
            .name
            .to_ascii_lowercase()
            .contains(&label.to_ascii_lowercase())
    {
        parts.push(label);
    }
    parts.push(f.name.trim().to_owned());
    safe_name(&format!("{} (Foreman #{})", parts.join(" "), f.id))
}

/// One Orca filament profile (JSON object) and what it was built from.
pub fn orca_profile(f: &ExportFilament) -> (Value, OrcaProfileInfo) {
    let cfg: Map<String, Value> = f
        .orca_config
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default();
    let (p, fallback_note) = pick_parent(f, &cfg);
    let name = orca_preset_name(f);
    let mut notes: Vec<String> = fallback_note.into_iter().collect();
    let mut o = Map::new();
    // Metadata, not a setting: Orca's JSON loader keeps it out of the config.
    // The GUI import doesn't need it; the CLI uses it to tell the preset type.
    o.insert("type".into(), json!("filament"));
    o.insert("name".into(), json!(name));
    o.insert("filament_settings_id".into(), arr(name.clone()));
    o.insert("from".into(), json!("User"));
    o.insert("inherits".into(), json!(p.0));
    o.insert("is_custom_defined".into(), json!("0"));
    o.insert("version".into(), json!(ORCA_VERSION));
    o.insert("filament_id".into(), json!(p.1));
    if let Some(v) = f.vendor.as_deref().filter(|v| !v.trim().is_empty()) {
        o.insert("filament_vendor".into(), arr(v.trim()));
    }
    if f.density > 0.0 {
        o.insert("filament_density".into(), arr(orca_num(f.density)));
    }
    if f.diameter > 0.0 {
        o.insert("filament_diameter".into(), arr(orca_num(f.diameter)));
    }
    if let (Some(price), Some(w)) = (f.price, f.weight.filter(|w| *w > 0.0)) {
        o.insert(
            "filament_cost".into(),
            arr(orca_num((price / (w / 1000.0) * 100.0).round() / 100.0)),
        );
    }
    if let Some(c) = f.colours.first() {
        o.insert(
            "default_filament_colour".into(),
            arr(format!("#{}", c.to_ascii_uppercase())),
        );
    }

    // Temperatures: orcaslicer_config first, then Spoolman's own columns.
    let nozzle = cfg_num(&cfg, &["nozzle_temperature"]).or(f.extruder_temp.map(|t| t as f64));
    if let Some(t) = nozzle {
        o.insert("nozzle_temperature".into(), variants(orca_num(t.round())));
        let first = cfg_num(
            &cfg,
            &[
                "nozzle_temp_initial_layer",
                "nozzle_temperature_initial_layer",
            ],
        )
        .unwrap_or(t);
        o.insert(
            "nozzle_temperature_initial_layer".into(),
            variants(orca_num(first.round())),
        );
    }
    if let Some(t) = cfg_num(&cfg, &["nozzle_temp_min", "nozzle_temperature_range_low"]) {
        o.insert(
            "nozzle_temperature_range_low".into(),
            arr(orca_num(t.round())),
        );
    }
    if let Some(t) = cfg_num(&cfg, &["nozzle_temp_max", "nozzle_temperature_range_high"]) {
        o.insert(
            "nozzle_temperature_range_high".into(),
            arr(orca_num(t.round())),
        );
    }
    let bed = cfg_num(&cfg, &["bed_temp", "hot_plate_temp"]).or(f.bed_temp.map(|t| t as f64));
    if let Some(t) = bed {
        let first = cfg_num(
            &cfg,
            &["bed_temp_initial_layer", "hot_plate_temp_initial_layer"],
        )
        .unwrap_or(t);
        for k in ["hot_plate_temp", "textured_plate_temp"] {
            o.insert(k.into(), arr(orca_num(t.round())));
            o.insert(format!("{k}_initial_layer"), arr(orca_num(first.round())));
        }
    }
    if let Some(t) = cfg_num(&cfg, &["chamber_temperature"]) {
        o.insert("chamber_temperature".into(), arr(orca_num(t.round())));
    }

    // Flow, volumetric speed, pressure advance (per extruder variant).
    let flow = cfg_num(&cfg, &["filament_flow_ratio", "flow_ratio"]);
    if let Some(v) = flow {
        o.insert("filament_flow_ratio".into(), variants(orca_num(v)));
    }
    if let Some(v) = cfg_num(
        &cfg,
        &["max_volumetric_speed", "filament_max_volumetric_speed"],
    ) {
        o.insert(
            "filament_max_volumetric_speed".into(),
            variants(orca_num(v)),
        );
    }
    if let Some(v) = cfg_num(&cfg, &["pressure_advance"]) {
        o.insert("pressure_advance".into(), variants(orca_num(v)));
    }
    if let Some(b) = cfg_bool(&cfg, "enable_pressure_advance") {
        o.insert(
            "enable_pressure_advance".into(),
            variants(if b { "1" } else { "0" }),
        );
    }
    for (from, to) in [
        ("fan_min_speed", "fan_min_speed"),
        ("fan_max_speed", "fan_max_speed"),
    ] {
        if let Some(v) = cfg_num(&cfg, &[from]) {
            o.insert(to.into(), variants(orca_num(v.round())));
        }
    }

    // Shrinkage (session 32 stores it in orcaslicer_config).
    let (sxy, sz) = (
        cfg_num(&cfg, &["shrinkage_xy", "filament_shrink"]),
        cfg_num(&cfg, &["shrinkage_z", "filament_shrinkage_compensation_z"]),
    );
    if let Some(v) = sxy {
        o.insert("filament_shrink".into(), arr(format!("{}%", orca_num(v))));
    }
    if let Some(v) = sz {
        o.insert(
            "filament_shrinkage_compensation_z".into(),
            arr(format!("{}%", orca_num(v))),
        );
    }

    let mut note = format!("Exported by Foreman from Spoolman filament #{}.", f.id);
    match p.2 {
        Some(b) => note.push_str(&format!(
            " Bambu filament id {b} ({}).",
            p.0.trim_end_matches(" @System")
        )),
        None => note.push_str(" No Bambu filament id for this material."),
    }
    if f.multi_colour() {
        let list: Vec<String> = f.colours.iter().map(|c| format!("#{c}")).collect();
        note.push_str(&format!(" Colours: {}.", list.join(", ")));
        notes.push(format!(
            "Multi-colour: all {} colours listed in the profile notes.",
            f.colours.len()
        ));
    }
    o.insert("filament_notes".into(), arr(note));

    let file = format!("{name}.json");
    (
        Value::Object(o),
        OrcaProfileInfo {
            filament_id: f.id,
            name,
            file,
            inherits: p.0.into(),
            orca_filament_id: p.1.into(),
            bambu_filament_id: p.2.map(Into::into),
            shrinkage_xy: sxy,
            shrinkage_z: sz,
            flow_ratio: flow,
            multi_colour: f.multi_colour(),
            notes,
        },
    )
}

/// A zip of Orca profiles (plain zip, JSON entries only: Orca imports every
/// entry as a preset).
pub fn orca_zip(filaments: &[ExportFilament]) -> Result<(Vec<u8>, Vec<OrcaProfileInfo>), AppError> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut infos = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        for f in filaments {
            let (v, info) = orca_profile(f);
            let text = serde_json::to_string_pretty(&v)
                .map_err(|e| AppError::Internal(format!("orca profile json: {e}")))?;
            zip.start_file(info.file.as_str(), opts)
                .map_err(|e| AppError::Internal(format!("zip: {e}")))?;
            zip.write_all(text.as_bytes())
                .map_err(|e| AppError::Internal(format!("zip: {e}")))?;
            infos.push(info);
        }
        zip.finish()
            .map_err(|e| AppError::Internal(format!("zip: {e}")))?;
    }
    Ok((buf.into_inner(), infos))
}

// ---------------------------------------------------------------------------
// HueForge
// ---------------------------------------------------------------------------

/// Why a filament is not in the HueForge catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum HueForgeSkip {
    /// No `hue_td` value: measure it.
    MissingTd,
    /// Several colours, one TD can't describe it.
    MultiColour,
    /// No colour stored.
    NoColour,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct HueForgeSkipped {
    #[ts(type = "number")]
    pub filament_id: i64,
    pub label: String,
    pub reason: HueForgeSkip,
    pub owned: bool,
}

/// Stable uuid per filament, so a re-import updates instead of duplicating.
fn hueforge_uuid(id: i64) -> String {
    let ns = uuid::Uuid::from_u128(0x6f72_656d_616e_2d68_7565_666f_7267_6500);
    format!("{{{}}}", uuid::Uuid::new_v5(&ns, id.to_string().as_bytes()))
}

/// HueForge library JSON and the filaments left out.
pub fn hueforge_catalogue(filaments: &[ExportFilament]) -> (Value, Vec<HueForgeSkipped>) {
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for f in filaments {
        let label = format!(
            "{} {}",
            f.vendor.as_deref().unwrap_or("").trim(),
            f.name.trim()
        )
        .trim()
        .to_owned();
        let skip = |reason| HueForgeSkipped {
            filament_id: f.id,
            label: label.clone(),
            reason,
            owned: f.owned,
        };
        if f.multi_colour() {
            skipped.push(skip(HueForgeSkip::MultiColour));
            continue;
        }
        let Some(td) = f.hue_td else {
            skipped.push(skip(HueForgeSkip::MissingTd));
            continue;
        };
        let Some(colour) = f.colours.first() else {
            skipped.push(skip(HueForgeSkip::NoColour));
            continue;
        };
        let base = materials::normalize_filament_type(f.material.as_deref().unwrap_or(""));
        out.push(json!({
            "Brand": f.vendor.as_deref().unwrap_or("").trim(),
            "Color": format!("#{}", colour.to_ascii_lowercase()),
            "Name": f.name.trim(),
            "Owned": f.owned,
            "Tags": [],
            "Transmissivity": td,
            "Type": if base.is_empty() { "PLA".to_owned() } else { base },
            "uuid": hueforge_uuid(f.id),
        }));
    }
    (json!({ "Filaments": out }), skipped)
}

#[cfg(test)]
mod tests;
