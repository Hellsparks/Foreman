//! Loaded filament + filament guard (W8, task 4.1).
//!
//! - [`LoadedSlot`]: which spool sits in which tool / AMS slot of a printer.
//!   Klipper: Moonraker's active spool (its `[spoolman]` link), tool 0 only.
//!   Bambu: the AMS trays the printer reports, read-only (cloud mode).
//! - [`guard`]: checks a spool against a printer slot before it is loaded and
//!   returns warnings with a severity and a plain-language reason. Blocking
//!   warnings need an explicit override with a reason (written to `audit_log`
//!   by the caller).
//!
//! Spoolman data is passed in as [`SpoolFacts`] so this crate does not depend
//! on spoolman-rs.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{domain::Printer, driver::PrinterCapabilities, events::AmsTray};

// ---------------------------------------------------------------------------
// Spool facts (a flattened Spoolman spool + filament + vendor)
// ---------------------------------------------------------------------------

/// What Foreman needs to know about one Spoolman spool.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SpoolFacts {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub filament_id: i64,
    /// Filament name, e.g. "PLA Basic Black".
    pub name: String,
    pub vendor: Option<String>,
    /// Base material, e.g. "PLA", "PA12".
    pub material: Option<String>,
    /// `material_modifier` extra field, e.g. "CF", "Silk".
    pub modifier: Option<String>,
    /// Abrasive: the explicit `abrasive` extra field when set, else derived from
    /// the material / modifier (CF, GF, AF, Metal, Glow).
    pub abrasive: bool,
    /// `RRGGBB`, no `#`.
    pub color_hex: Option<String>,
    /// Multi-colour filaments: every colour, `RRGGBB`.
    pub multi_color_hexes: Vec<String>,
    /// `coaxial` (coextruded, side by side) or `longitudinal` (gradient).
    pub multi_color_direction: Option<String>,
    pub remaining_weight: Option<f64>,
    pub initial_weight: Option<f64>,
    pub used_weight: f64,
    pub spool_weight: Option<f64>,
    pub location: Option<String>,
    pub lot_nr: Option<String>,
    pub price: Option<f64>,
    pub diameter: f64,
    /// Filament nozzle temperature, °C.
    #[ts(type = "number | null")]
    pub extruder_temp: Option<i64>,
    #[ts(type = "number | null")]
    pub bed_temp: Option<i64>,
    pub first_used: Option<String>,
    pub last_used: Option<String>,
    pub registered: String,
    pub archived: bool,
    pub comment: Option<String>,
}

impl SpoolFacts {
    /// "PLA" / "PA12-CF" / "PLA Silk" — material with its modifier.
    pub fn material_label(&self) -> String {
        match (&self.material, &self.modifier) {
            (Some(m), Some(x)) if !x.is_empty() => format!("{m} {x}"),
            (Some(m), _) => m.clone(),
            (None, Some(x)) => x.clone(),
            (None, None) => "unknown material".into(),
        }
    }

    /// "#1 PLA Basic Black".
    pub fn label(&self) -> String {
        format!("#{} {}", self.id, self.name)
    }

    /// Every colour of the spool (`RRGGBB`, upper case).
    pub fn colours(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .multi_color_hexes
            .iter()
            .map(|c| norm_hex(c))
            .filter(|c| !c.is_empty())
            .collect();
        if let Some(c) = &self.color_hex {
            let c = norm_hex(c);
            if !c.is_empty() && !v.contains(&c) {
                v.insert(0, c);
            }
        }
        v
    }
}

/// `#rrggbb` / `rrggbbaa` → `RRGGBB`.
pub fn norm_hex(c: &str) -> String {
    let c = c.trim().trim_start_matches('#').to_ascii_uppercase();
    if c.len() >= 6 && c.chars().take(6).all(|ch| ch.is_ascii_hexdigit()) {
        c[..6].to_owned()
    } else {
        String::new()
    }
}

/// Euclidean RGB distance between two hex colours (0 = same, ~441 = black/white).
pub fn hex_distance(a: &str, b: &str) -> Option<f64> {
    let (a, b) = (norm_hex(a), norm_hex(b));
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let rgb = |s: &str| -> Option<[f64; 3]> {
        let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok().map(f64::from);
        Some([p(0)?, p(2)?, p(4)?])
    };
    let (x, y) = (rgb(&a)?, rgb(&b)?);
    Some(((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt())
}

/// Colours closer than this count as "the same colour" for compat.
pub const COLOUR_MATCH_DISTANCE: f64 = 40.0;

/// Base material of a slicer / tray string: "PLA Basic" → "PLA", "PA12-CF" → "PA12".
fn base_material(m: &str) -> String {
    m.split(['-', ' ', '+', '_'])
        .find(|t| !t.is_empty())
        .unwrap_or("")
        .to_ascii_uppercase()
}

/// Do two material strings name the same material family?
/// "PLA" ~ "PLA Basic" ~ "PLA+"; "PA12-CF" ~ "PA12"; "PA" ~ "PA12"; "PETG" ≁ "PLA".
pub fn materials_match(a: &str, b: &str) -> bool {
    let (a, b) = (base_material(a), base_material(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    // Nylon variants: "PA" matches "PA6" / "PA12"; "PA6" does not match "PA12".
    let fam = |s: &str| s.trim_end_matches(|c: char| c.is_ascii_digit()).to_owned();
    (fam(&a) == b || fam(&b) == a) && !fam(&a).is_empty()
}

// ---------------------------------------------------------------------------
// Loaded slots
// ---------------------------------------------------------------------------

/// One tool / AMS slot on a printer and what is in it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LoadedSlot {
    #[ts(type = "number")]
    pub printer_id: i64,
    pub printer_name: String,
    /// Tool index (Klipper extruder) or global AMS slot (`ams * 4 + tray`, 254 = external).
    pub tool: u32,
    /// "T0", "AMS 1 · slot 2", "External spool".
    pub label: String,
    /// The Spoolman spool in this slot (Klipper: Moonraker's active spool).
    #[ts(type = "number | null")]
    pub spool_id: Option<i64>,
    pub spool: Option<SpoolFacts>,
    /// What the printer itself reports in this slot (Bambu AMS).
    pub tray: Option<AmsTray>,
    /// Foreman can load / unload this slot.
    pub writable: bool,
    /// Why not, when `writable` is false.
    pub read_only_reason: Option<String>,
}

/// Label for an AMS global slot index.
pub fn ams_slot_label(slot: u32) -> String {
    if slot == 254 || slot == 255 {
        "External spool".into()
    } else {
        format!("AMS {} · slot {}", slot / 4 + 1, slot % 4 + 1)
    }
}

/// Why a Bambu tray can't be changed from Foreman.
pub const BAMBU_READ_ONLY: &str =
    "Bambu trays are set on the printer. Read-only while the printer is in cloud mode.";

// ---------------------------------------------------------------------------
// Guard
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum GuardSeverity {
    /// Needs an explicit override with a reason.
    Block,
    /// Shown, but loading goes ahead.
    Warn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum GuardCode {
    /// Material needs more than the printer can do (nozzle temperature, enclosure).
    Material,
    /// Abrasive filament on a non-hardened nozzle.
    Abrasive,
    /// Not enough filament left for the current or next job (or under 150 g).
    LowRemaining,
    /// The Bambu tray reports a different material / colour than the spool.
    TrayMismatch,
    /// The spool is already loaded on another printer / slot.
    LoadedElsewhere,
    /// The spool is archived in Spoolman.
    Archived,
}

/// One guard finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GuardWarning {
    pub code: GuardCode,
    pub severity: GuardSeverity,
    /// Plain-language reason, shown as-is.
    pub reason: String,
}

/// How much filament a job on this slot needs.
#[derive(Debug, Clone, PartialEq)]
pub struct JobNeed {
    pub grams: f64,
    /// "the current print (benchy.gcode)" / "the next queued job (clip.gcode)".
    pub what: String,
}

/// Everything the guard looks at.
pub struct GuardInput<'a> {
    pub printer: &'a Printer,
    pub caps: Option<&'a PrinterCapabilities>,
    pub spool: &'a SpoolFacts,
    /// Bambu: what the printer reports in the target tray.
    pub tray: Option<&'a AmsTray>,
    pub need: Option<JobNeed>,
    /// Other slots this spool is loaded in (not the target slot).
    pub loaded_elsewhere: &'a [LoadedSlot],
}

/// Under this, a spool is "low" even when no job needs it.
pub const LOW_GRAMS: f64 = 150.0;

/// Materials that warp or crack without an enclosure.
const NEEDS_ENCLOSURE: &[&str] = &["ABS", "ASA", "PC", "PA", "PPS", "PEI", "PEEK", "PEKK"];

/// Typical minimum nozzle temperature when the filament has none set.
fn default_nozzle_temp(base: &str) -> Option<f64> {
    let t = match base {
        "PLA" | "PVA" | "BVOH" => 190.0,
        "TPU" | "TPE" | "POM" => 210.0,
        "PETG" | "PET" | "HIPS" | "CPE" => 220.0,
        "ABS" | "ASA" => 230.0,
        "PA" | "PA6" | "PA12" | "NYLON" | "PC" => 250.0,
        "PPS" => 300.0,
        "PEI" | "ULTEM" => 350.0,
        "PEEK" | "PEKK" => 370.0,
        _ => return None,
    };
    Some(t)
}

/// Run the guard. Order: blocks first, then warnings.
pub fn guard(input: &GuardInput<'_>) -> Vec<GuardWarning> {
    let mut out = Vec::new();
    let sp = input.spool;
    let printer = &input.printer.name;
    let mat_label = sp.material_label();
    let base = sp
        .material
        .as_deref()
        .map(base_material)
        .unwrap_or_default();

    // Abrasive vs nozzle.
    if sp.abrasive && !input.printer.hardened_nozzle {
        out.push(GuardWarning {
            code: GuardCode::Abrasive,
            severity: GuardSeverity::Block,
            reason: format!(
                "{mat_label} is abrasive. {printer} has no hardened nozzle; a brass nozzle wears out within hours."
            ),
        });
    }

    // Material vs printer: nozzle temperature.
    let need_temp = sp
        .extruder_temp
        .map(|t| t as f64)
        .or_else(|| default_nozzle_temp(&base));
    let max_temp = input.caps.and_then(|c| c.max_nozzle_temp);
    if let (Some(need), Some(max)) = (need_temp, max_temp) {
        if need > max {
            out.push(GuardWarning {
                code: GuardCode::Material,
                severity: GuardSeverity::Block,
                reason: format!(
                    "{mat_label} needs {need:.0} °C; {printer}'s nozzle tops out at {max:.0} °C."
                ),
            });
        }
    }

    // Material vs printer: enclosure.
    let fam = base.trim_end_matches(|c: char| c.is_ascii_digit());
    if input.caps.and_then(|c| c.enclosed) == Some(false)
        && NEEDS_ENCLOSURE.iter().any(|m| *m == base || *m == fam)
    {
        out.push(GuardWarning {
            code: GuardCode::Material,
            severity: GuardSeverity::Warn,
            reason: format!(
                "{mat_label} warps and cracks without an enclosure; {printer} is open-frame."
            ),
        });
    }

    // Remaining weight vs the job that will use it.
    if let Some(left) = sp.remaining_weight {
        match &input.need {
            Some(n) if left < n.grams => out.push(GuardWarning {
                code: GuardCode::LowRemaining,
                severity: GuardSeverity::Warn,
                reason: format!("Only {left:.0} g left; {} needs {:.0} g.", n.what, n.grams),
            }),
            _ if left < LOW_GRAMS => out.push(GuardWarning {
                code: GuardCode::LowRemaining,
                severity: GuardSeverity::Warn,
                reason: format!("Only {left:.0} g left. Weigh it to be sure before a long print."),
            }),
            _ => {}
        }
    }

    // Bambu tray mismatch.
    if let Some(tray) = input.tray {
        if let (Some(tm), Some(sm)) = (tray.material.as_deref(), sp.material.as_deref()) {
            if !materials_match(tm, sm) {
                out.push(GuardWarning {
                    code: GuardCode::TrayMismatch,
                    severity: GuardSeverity::Warn,
                    reason: format!(
                        "The tray reports {tm}, this spool is {mat_label}. Set the tray on the printer to match."
                    ),
                });
            }
        }
        if let Some(tc) = tray.color.as_deref() {
            let colours = sp.colours();
            let close = colours
                .iter()
                .filter_map(|c| hex_distance(c, tc))
                .any(|d| d <= COLOUR_MATCH_DISTANCE);
            if !colours.is_empty() && !close {
                out.push(GuardWarning {
                    code: GuardCode::TrayMismatch,
                    severity: GuardSeverity::Warn,
                    reason: format!(
                        "The tray reports colour #{}, this spool is #{}.",
                        norm_hex(tc),
                        colours[0]
                    ),
                });
            }
        }
    }

    // Already loaded elsewhere.
    for other in input.loaded_elsewhere {
        out.push(GuardWarning {
            code: GuardCode::LoadedElsewhere,
            severity: GuardSeverity::Warn,
            reason: if other.writable {
                format!(
                    "Loaded on {} {}. Loading here unloads it there.",
                    other.printer_name, other.label
                )
            } else {
                format!("Also shown on {} {}.", other.printer_name, other.label)
            },
        });
    }

    if sp.archived {
        out.push(GuardWarning {
            code: GuardCode::Archived,
            severity: GuardSeverity::Warn,
            reason: "This spool is archived in Spoolman.".into(),
        });
    }

    out.sort_by_key(|w| match w.severity {
        GuardSeverity::Block => 0,
        GuardSeverity::Warn => 1,
    });
    out
}

/// True when any warning blocks loading without an override.
pub fn is_blocked(ws: &[GuardWarning]) -> bool {
    ws.iter().any(|w| w.severity == GuardSeverity::Block)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::FirmwareKind;

    fn printer(hardened: bool) -> Printer {
        Printer {
            id: 1,
            name: "HellV0".into(),
            firmware: FirmwareKind::Moonraker,
            primary_url: None,
            api_key: None,
            serial_number: None,
            access_code: None,
            tls_fingerprint: None,
            tags: vec![],
            hardened_nozzle: hardened,
            slicer_profiles: vec![],
            sort_order: 0,
            fail_after_checks: 3,
            return_after_secs: 60,
            plate_clear: true,
            auto_start_from_queue: true,
            usage_reporter: Default::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn spool(material: &str, grams: f64) -> SpoolFacts {
        SpoolFacts {
            id: 1,
            name: format!("{material} Black"),
            material: Some(material.into()),
            abrasive: crate::materials::is_abrasive_filament(Some(material), None),
            color_hex: Some("1D1D22".into()),
            remaining_weight: Some(grams),
            initial_weight: Some(1000.0),
            diameter: 1.75,
            ..Default::default()
        }
    }

    fn run(
        p: &Printer,
        caps: Option<&PrinterCapabilities>,
        s: &SpoolFacts,
        tray: Option<&AmsTray>,
        need: Option<JobNeed>,
        elsewhere: &[LoadedSlot],
    ) -> Vec<GuardWarning> {
        guard(&GuardInput {
            printer: p,
            caps,
            spool: s,
            tray,
            need,
            loaded_elsewhere: elsewhere,
        })
    }

    fn codes(ws: &[GuardWarning]) -> Vec<(GuardCode, GuardSeverity)> {
        ws.iter().map(|w| (w.code, w.severity)).collect()
    }

    #[test]
    fn clean_spool_has_no_warnings() {
        let p = printer(false);
        let ws = run(&p, None, &spool("PLA", 800.0), None, None, &[]);
        assert!(ws.is_empty(), "{ws:?}");
        assert!(!is_blocked(&ws));
    }

    #[test]
    fn abrasive_on_brass_blocks_and_hardened_is_fine() {
        let s = spool("PA12-CF", 480.0);
        assert!(s.abrasive);
        let ws = run(&printer(false), None, &s, None, None, &[]);
        assert_eq!(
            codes(&ws),
            vec![(GuardCode::Abrasive, GuardSeverity::Block)]
        );
        assert!(is_blocked(&ws));
        assert!(ws[0].reason.contains("hardened"));
        let ws = run(&printer(true), None, &s, None, None, &[]);
        assert!(ws.is_empty(), "{ws:?}");
    }

    #[test]
    fn abrasive_reads_the_flag() {
        // The guard reads the stored flag, not the material string.
        let mut s = spool("PLA", 800.0);
        s.abrasive = true; // explicit flag wins
        let ws = run(&printer(false), None, &s, None, None, &[]);
        assert_eq!(
            codes(&ws),
            vec![(GuardCode::Abrasive, GuardSeverity::Block)]
        );
    }

    #[test]
    fn nozzle_temperature_beyond_max_blocks() {
        let caps = PrinterCapabilities {
            max_nozzle_temp: Some(280.0),
            ..Default::default()
        };
        let ws = run(
            &printer(true),
            Some(&caps),
            &spool("PEEK", 500.0),
            None,
            None,
            &[],
        );
        assert_eq!(
            codes(&ws),
            vec![(GuardCode::Material, GuardSeverity::Block)]
        );
        assert!(ws[0].reason.contains("280"));
        // A filament temperature set in Spoolman wins over the default.
        let mut s = spool("PLA", 800.0);
        s.extruder_temp = Some(300);
        let ws = run(&printer(true), Some(&caps), &s, None, None, &[]);
        assert_eq!(
            codes(&ws),
            vec![(GuardCode::Material, GuardSeverity::Block)]
        );
        let ws = run(
            &printer(true),
            Some(&caps),
            &spool("PETG", 800.0),
            None,
            None,
            &[],
        );
        assert!(ws.is_empty());
    }

    #[test]
    fn enclosure_material_on_open_frame_warns() {
        let open = PrinterCapabilities {
            enclosed: Some(false),
            ..Default::default()
        };
        let ws = run(
            &printer(false),
            Some(&open),
            &spool("ABS", 800.0),
            None,
            None,
            &[],
        );
        assert_eq!(codes(&ws), vec![(GuardCode::Material, GuardSeverity::Warn)]);
        let ws = run(
            &printer(false),
            Some(&open),
            &spool("PA12", 800.0),
            None,
            None,
            &[],
        );
        assert_eq!(codes(&ws), vec![(GuardCode::Material, GuardSeverity::Warn)]);
        // Unknown enclosure (Klipper): no warning.
        let ws = run(&printer(false), None, &spool("ABS", 800.0), None, None, &[]);
        assert!(ws.is_empty());
    }

    #[test]
    fn low_remaining_against_job_need_and_floor() {
        let p = printer(false);
        let need = JobNeed {
            grams: 58.0,
            what: "the next queued job (chess_4.gcode)".into(),
        };
        let ws = run(&p, None, &spool("PLA", 40.0), None, Some(need.clone()), &[]);
        assert_eq!(
            codes(&ws),
            vec![(GuardCode::LowRemaining, GuardSeverity::Warn)]
        );
        assert!(ws[0].reason.contains("40 g"), "{}", ws[0].reason);
        assert!(ws[0].reason.contains("58 g"));
        assert!(!is_blocked(&ws));
        // Enough for the job and above the floor: fine.
        let ws = run(&p, None, &spool("PLA", 400.0), None, Some(need), &[]);
        assert!(ws.is_empty());
        // No job, under 150 g: still warns.
        let ws = run(&p, None, &spool("PLA", 120.0), None, None, &[]);
        assert_eq!(
            codes(&ws),
            vec![(GuardCode::LowRemaining, GuardSeverity::Warn)]
        );
    }

    #[test]
    fn bambu_tray_mismatch_material_and_colour() {
        let p = printer(false);
        let tray = AmsTray {
            slot: 0,
            color: Some("FFFFFF".into()),
            material: Some("PETG".into()),
            remaining: Some(80),
        };
        let ws = run(&p, None, &spool("PLA", 800.0), Some(&tray), None, &[]);
        assert_eq!(
            codes(&ws),
            vec![
                (GuardCode::TrayMismatch, GuardSeverity::Warn),
                (GuardCode::TrayMismatch, GuardSeverity::Warn)
            ]
        );
        let ok = AmsTray {
            slot: 0,
            color: Some("1E1E20".into()),
            material: Some("PLA".into()),
            remaining: Some(80),
        };
        let ws = run(&p, None, &spool("PLA", 800.0), Some(&ok), None, &[]);
        assert!(ws.is_empty(), "{ws:?}");
    }

    #[test]
    fn loaded_elsewhere_and_archived_warn() {
        let p = printer(false);
        let other = LoadedSlot {
            printer_id: 2,
            printer_name: "Sovol".into(),
            tool: 0,
            label: "T0".into(),
            spool_id: Some(1),
            spool: None,
            tray: None,
            writable: true,
            read_only_reason: None,
        };
        let mut s = spool("PLA", 800.0);
        s.archived = true;
        let ws = run(&p, None, &s, None, None, std::slice::from_ref(&other));
        assert_eq!(
            codes(&ws),
            vec![
                (GuardCode::LoadedElsewhere, GuardSeverity::Warn),
                (GuardCode::Archived, GuardSeverity::Warn)
            ]
        );
        assert!(ws[0].reason.contains("unloads it there"));
    }

    #[test]
    fn blocks_sort_before_warnings() {
        let s = spool("PA12-CF", 40.0);
        let ws = run(&printer(false), None, &s, None, None, &[]);
        assert_eq!(
            codes(&ws),
            vec![
                (GuardCode::Abrasive, GuardSeverity::Block),
                (GuardCode::LowRemaining, GuardSeverity::Warn)
            ]
        );
    }

    #[test]
    fn material_families() {
        assert!(materials_match("PLA", "PLA Basic"));
        assert!(materials_match("PLA+", "PLA"));
        assert!(materials_match("PA12-CF", "PA12"));
        assert!(materials_match("PA", "PA12"));
        assert!(!materials_match("PA6", "PA12"));
        assert!(!materials_match("PETG", "PLA"));
        assert!(!materials_match("", "PLA"));
    }

    #[test]
    fn colours_and_distance() {
        assert_eq!(norm_hex("#1d1d22ff"), "1D1D22");
        assert_eq!(hex_distance("000000", "000000"), Some(0.0));
        assert!(hex_distance("FFFFFF", "000000").is_some_and(|d| d > 400.0));
        assert_eq!(hex_distance("xyz", "000000"), None);
        let s = SpoolFacts {
            color_hex: Some("b87333".into()),
            multi_color_hexes: vec!["B87333".into(), "c0c0c0".into()],
            ..Default::default()
        };
        assert_eq!(s.colours(), vec!["B87333".to_owned(), "C0C0C0".to_owned()]);
    }
}
