//! Compatibility engine (task 3.3).
//!
//! One function that takes file facts + printer capabilities + printer state +
//! loaded spools (AMS trays) and returns a per-printer verdict.
//!
//! Used by Send, fleet queue, and (future) MCP.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    domain::{FileMeta, Printer},
    driver::PrinterCapabilities,
    events::{AmsTray, PrinterState, PrinterStatus},
};

// ---------------------------------------------------------------------------
// Verdict
// ---------------------------------------------------------------------------

/// Compatibility verdict for one printer × one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Printer is idle, compatible, has matching filament.
    Ready,
    /// Printer is busy (printing/paused) but otherwise compatible.
    Busy,
    /// Printer needs its plate cleared first.
    Plate,
    /// Material type mismatch (e.g. file wants PETG, printer has PLA loaded).
    Material,
    /// No spool / AMS tray detected for the needed material.
    NoSpool,
    /// File exceeds the printer's bed dimensions.
    TooBig,
    /// Nozzle mismatch (e.g. file needs hardened nozzle for abrasive material).
    Nozzle,
    /// Printer is offline / unreachable.
    Offline,
}

/// Full compatibility result for one printer.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CompatResult {
    pub printer_id: i64,
    pub printer_name: String,
    pub verdict: Verdict,
    /// Human-readable reason, e.g. "idle · PLA loaded" or "offline".
    pub reason: String,
    /// Optional quick-fix action, e.g. "Plate cleared" or "Load matching spool".
    pub quick_fix: Option<String>,
}

// ---------------------------------------------------------------------------
// Input snapshot (everything the engine needs, collected by the caller)
// ---------------------------------------------------------------------------

/// Everything the compatibility check needs for one printer.
pub struct PrinterSnapshot {
    pub printer: Printer,
    pub caps: Option<PrinterCapabilities>,
    pub status: Option<PrinterStatus>,
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

/// Check one file against one printer.
pub fn check(file: &FileMeta, snap: &PrinterSnapshot) -> CompatResult {
    let p = &snap.printer;
    let name = p.name.clone();
    let id = p.id;

    // 1. Offline: no status or state == Offline.
    let state = snap
        .status
        .as_ref()
        .map(|s| &s.state)
        .unwrap_or(&PrinterState::Offline);
    if *state == PrinterState::Offline {
        return CompatResult {
            printer_id: id,
            printer_name: name,
            verdict: Verdict::Offline,
            reason: "offline".into(),
            quick_fix: None,
        };
    }

    // 2. Bed size check.
    if let (Some(fw), Some(fd)) = (file.bed_width, file.bed_depth) {
        if let Some(caps) = &snap.caps {
            if let Some((bw, bd)) = caps.bed_size {
                if fw > bw || fd > bd {
                    return CompatResult {
                        printer_id: id,
                        printer_name: name,
                        verdict: Verdict::TooBig,
                        reason: format!("file needs {fw:.0}×{fd:.0} mm, bed is {bw:.0}×{bd:.0} mm"),
                        quick_fix: None,
                    };
                }
            }
        }
    }

    // 3. Nozzle: abrasive materials (CF, GF) need a hardened nozzle.
    let needs_hardened = file.tools.iter().any(|t| {
        t.material
            .as_deref()
            .map(|m| {
                let m = m.to_uppercase();
                m.contains("CF") || m.contains("GF")
            })
            .unwrap_or(false)
    });
    if needs_hardened && !p.hardened_nozzle {
        return CompatResult {
            printer_id: id,
            printer_name: name,
            verdict: Verdict::Nozzle,
            reason: "file uses abrasive filament; printer has no hardened nozzle".into(),
            quick_fix: None,
        };
    }

    // 4. Nozzle diameter check.
    if let Some(file_nozzle) = file.nozzle_diameter {
        if let Some(caps) = &snap.caps {
            if let Some(printer_nozzle) = caps.nozzle_diameter {
                if (file_nozzle - printer_nozzle).abs() > 0.05 {
                    return CompatResult {
                        printer_id: id,
                        printer_name: name,
                        verdict: Verdict::Nozzle,
                        reason: format!(
                            "file needs {file_nozzle:.2} mm nozzle, printer has {printer_nozzle:.2} mm"
                        ),
                        quick_fix: None,
                    };
                }
            }
        }
    }

    // 5. Plate gate: printer isn't plate-clear.
    if !p.plate_clear {
        return CompatResult {
            printer_id: id,
            printer_name: name,
            verdict: Verdict::Plate,
            reason: "clear plate first".into(),
            quick_fix: Some("Plate cleared".into()),
        };
    }

    // 6. Busy: printing or paused.
    if *state == PrinterState::Printing || *state == PrinterState::Paused {
        let eta = snap
            .status
            .as_ref()
            .and_then(|s| s.remaining_time)
            .map(|secs| {
                let m = secs / 60;
                if m >= 60 {
                    format!("free in {} h {} m", m / 60, m % 60)
                } else {
                    format!("free in {m} m")
                }
            })
            .unwrap_or_else(|| "busy".into());
        return CompatResult {
            printer_id: id,
            printer_name: name,
            verdict: Verdict::Busy,
            reason: eta,
            quick_fix: None,
        };
    }

    // 7. Material / spool match (AMS trays for Bambu, or best-effort).
    let needed_materials: Vec<&str> = file
        .tools
        .iter()
        .filter_map(|t| t.material.as_deref())
        .collect();

    if !needed_materials.is_empty() {
        let ams = snap
            .status
            .as_ref()
            .map(|s| &s.ams_trays[..])
            .unwrap_or(&[]);

        if !ams.is_empty() {
            // Check every needed material can be found in AMS.
            for mat in &needed_materials {
                let loaded = ams_has_material(ams, mat);
                if !loaded {
                    return CompatResult {
                        printer_id: id,
                        printer_name: name,
                        verdict: Verdict::Material,
                        reason: format!("{mat} not loaded"),
                        quick_fix: Some("Load matching spool".into()),
                    };
                }
            }
        }
        // If no AMS data, we can't check — assume ready (Klipper printers
        // don't report loaded filament via the driver).
    }

    // 8. Ready.
    let loaded_hint = snap
        .status
        .as_ref()
        .and_then(|s| {
            s.ams_trays
                .first()
                .and_then(|t| t.material.as_deref())
                .map(|m| format!("idle · {m} loaded"))
        })
        .unwrap_or_else(|| "idle".into());

    CompatResult {
        printer_id: id,
        printer_name: name,
        verdict: Verdict::Ready,
        reason: loaded_hint,
        quick_fix: None,
    }
}

/// Check all printers for one file.
pub fn check_all(file: &FileMeta, printers: &[PrinterSnapshot]) -> Vec<CompatResult> {
    let mut results: Vec<CompatResult> = printers.iter().map(|snap| check(file, snap)).collect();
    // Sort: Ready first, then Busy, Plate, Material, NoSpool, TooBig, Nozzle, Offline.
    results.sort_by_key(|r| verdict_order(&r.verdict));
    results
}

fn verdict_order(v: &Verdict) -> u8 {
    match v {
        Verdict::Ready => 0,
        Verdict::Busy => 1,
        Verdict::Plate => 2,
        Verdict::Material => 3,
        Verdict::NoSpool => 4,
        Verdict::TooBig => 5,
        Verdict::Nozzle => 6,
        Verdict::Offline => 7,
    }
}

/// How well the loaded filament matches the file: +2 per tool whose material is
/// loaded in a tray, +1 more when the colour matches too. 0 when the printer reports
/// no trays (Klipper). The fleet queue prefers the highest score.
pub fn filament_score(file: &FileMeta, snap: &PrinterSnapshot) -> u32 {
    let trays = snap
        .status
        .as_ref()
        .map(|s| &s.ams_trays[..])
        .unwrap_or(&[]);
    let mut score = 0;
    for tool in &file.tools {
        let Some(mat) = tool.material.as_deref() else {
            continue;
        };
        let want_colour = tool
            .colour
            .as_deref()
            .map(|c| c.trim_start_matches('#').to_uppercase());
        let mut best = 0;
        for t in trays.iter().filter(|t| tray_has_material(t, mat)) {
            let colour_ok = match (&want_colour, t.color.as_deref()) {
                (Some(w), Some(c)) => c.trim_start_matches('#').to_uppercase().starts_with(w),
                _ => false,
            };
            best = best.max(if colour_ok { 3 } else { 2 });
        }
        score += best;
    }
    score
}

fn tray_has_material(t: &AmsTray, needed: &str) -> bool {
    let needed_up = needed.to_uppercase();
    t.material
        .as_deref()
        .map(|m| {
            let m_up = m.to_uppercase();
            m_up.starts_with(&needed_up) || needed_up.starts_with(&m_up)
        })
        .unwrap_or(false)
}

/// Check if any AMS tray has a material matching the needed one (case-insensitive prefix).
fn ams_has_material(trays: &[AmsTray], needed: &str) -> bool {
    let needed_up = needed.to_uppercase();
    trays.iter().any(|t| {
        t.material
            .as_deref()
            .map(|m| {
                let m_up = m.to_uppercase();
                // "PLA" matches "PLA", "PLA-CF", etc.; "PETG" matches "PETG".
                // Also match the reverse: file says "PLA Basic" and tray says "PLA".
                m_up.starts_with(&needed_up) || needed_up.starts_with(&m_up)
            })
            .unwrap_or(false)
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{FirmwareKind, ToolUsage};

    fn make_printer(id: i64, name: &str) -> Printer {
        Printer {
            id,
            name: name.into(),
            firmware: FirmwareKind::Moonraker,
            primary_url: None,
            api_key: None,
            serial_number: None,
            access_code: None,
            tls_fingerprint: None,
            tags: vec![],
            hardened_nozzle: false,
            slicer_profiles: vec![],
            sort_order: 0,
            fail_after_checks: 3,
            return_after_secs: 60,
            plate_clear: true,
            auto_start_from_queue: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn make_caps(bw: f64, bd: f64) -> PrinterCapabilities {
        PrinterCapabilities {
            bed_size: Some((bw, bd)),
            ..Default::default()
        }
    }

    fn make_status(state: PrinterState) -> PrinterStatus {
        PrinterStatus {
            state,
            ..Default::default()
        }
    }

    fn make_meta() -> FileMeta {
        FileMeta {
            format: "gcode".into(),
            parse_status: "ok".into(),
            tools: vec![ToolUsage {
                index: 0,
                material: Some("PLA".into()),
                colour: Some("#FFFFFF".into()),
                grams: Some(50.0),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn offline_printer() {
        let meta = make_meta();
        let snap = PrinterSnapshot {
            printer: make_printer(1, "V0"),
            caps: Some(make_caps(120.0, 120.0)),
            status: None,
        };
        let r = check(&meta, &snap);
        assert_eq!(r.verdict, Verdict::Offline);
    }

    #[test]
    fn bed_too_small() {
        let mut meta = make_meta();
        meta.bed_width = Some(300.0);
        meta.bed_depth = Some(300.0);
        let snap = PrinterSnapshot {
            printer: make_printer(1, "V0"),
            caps: Some(make_caps(120.0, 120.0)),
            status: Some(make_status(PrinterState::Idle)),
        };
        let r = check(&meta, &snap);
        assert_eq!(r.verdict, Verdict::TooBig);
    }

    #[test]
    fn hardened_nozzle_needed() {
        let mut meta = make_meta();
        meta.tools[0].material = Some("PA12-CF".into());
        let snap = PrinterSnapshot {
            printer: make_printer(1, "V0"),
            caps: Some(make_caps(120.0, 120.0)),
            status: Some(make_status(PrinterState::Idle)),
        };
        let r = check(&meta, &snap);
        assert_eq!(r.verdict, Verdict::Nozzle);
    }

    #[test]
    fn hardened_nozzle_ok() {
        let mut meta = make_meta();
        meta.tools[0].material = Some("PA12-CF".into());
        let mut printer = make_printer(1, "V0");
        printer.hardened_nozzle = true;
        let snap = PrinterSnapshot {
            printer,
            caps: Some(make_caps(120.0, 120.0)),
            status: Some(make_status(PrinterState::Idle)),
        };
        let r = check(&meta, &snap);
        assert_eq!(r.verdict, Verdict::Ready);
    }

    #[test]
    fn plate_not_clear() {
        let meta = make_meta();
        let mut printer = make_printer(1, "V0");
        printer.plate_clear = false;
        let snap = PrinterSnapshot {
            printer,
            caps: Some(make_caps(120.0, 120.0)),
            status: Some(make_status(PrinterState::Idle)),
        };
        let r = check(&meta, &snap);
        assert_eq!(r.verdict, Verdict::Plate);
        assert!(r.quick_fix.is_some());
    }

    #[test]
    fn busy_printer() {
        let meta = make_meta();
        let snap = PrinterSnapshot {
            printer: make_printer(1, "V0"),
            caps: Some(make_caps(120.0, 120.0)),
            status: Some(make_status(PrinterState::Printing)),
        };
        let r = check(&meta, &snap);
        assert_eq!(r.verdict, Verdict::Busy);
    }

    #[test]
    fn ams_material_mismatch() {
        let meta = make_meta(); // needs PLA
        let mut status = make_status(PrinterState::Idle);
        status.ams_trays = vec![AmsTray {
            slot: 0,
            color: Some("FF0000".into()),
            material: Some("PETG".into()),
            remaining: Some(80),
        }];
        let snap = PrinterSnapshot {
            printer: make_printer(1, "X1C"),
            caps: Some(make_caps(256.0, 256.0)),
            status: Some(status),
        };
        let r = check(&meta, &snap);
        assert_eq!(r.verdict, Verdict::Material);
    }

    #[test]
    fn ams_material_match() {
        let meta = make_meta(); // needs PLA
        let mut status = make_status(PrinterState::Idle);
        status.ams_trays = vec![AmsTray {
            slot: 0,
            color: Some("FFFFFF".into()),
            material: Some("PLA".into()),
            remaining: Some(90),
        }];
        let snap = PrinterSnapshot {
            printer: make_printer(1, "X1C"),
            caps: Some(make_caps(256.0, 256.0)),
            status: Some(status),
        };
        let r = check(&meta, &snap);
        assert_eq!(r.verdict, Verdict::Ready);
    }

    #[test]
    fn ready_idle_klipper() {
        let meta = make_meta();
        let snap = PrinterSnapshot {
            printer: make_printer(1, "V0"),
            caps: Some(make_caps(120.0, 120.0)),
            status: Some(make_status(PrinterState::Idle)),
        };
        let r = check(&meta, &snap);
        assert_eq!(r.verdict, Verdict::Ready);
    }

    #[test]
    fn check_all_sorted() {
        let meta = make_meta();
        let snaps = vec![
            PrinterSnapshot {
                printer: make_printer(1, "Offline"),
                caps: None,
                status: None,
            },
            PrinterSnapshot {
                printer: make_printer(2, "Idle"),
                caps: Some(make_caps(120.0, 120.0)),
                status: Some(make_status(PrinterState::Idle)),
            },
            PrinterSnapshot {
                printer: make_printer(3, "Printing"),
                caps: Some(make_caps(120.0, 120.0)),
                status: Some(make_status(PrinterState::Printing)),
            },
        ];
        let results = check_all(&meta, &snaps);
        assert_eq!(results[0].verdict, Verdict::Ready);
        assert_eq!(results[1].verdict, Verdict::Busy);
        assert_eq!(results[2].verdict, Verdict::Offline);
    }

    #[test]
    fn nozzle_diameter_mismatch() {
        let mut meta = make_meta();
        meta.nozzle_diameter = Some(0.6);
        let snap = PrinterSnapshot {
            printer: make_printer(1, "V0"),
            caps: Some(PrinterCapabilities {
                bed_size: Some((120.0, 120.0)),
                nozzle_diameter: Some(0.4),
                ..Default::default()
            }),
            status: Some(make_status(PrinterState::Idle)),
        };
        let r = check(&meta, &snap);
        assert_eq!(r.verdict, Verdict::Nozzle);
    }
}
