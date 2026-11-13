//! Compatibility engine (task 3.3).
//!
//! One function that takes file facts + printer capabilities + printer state +
//! loaded spools (AMS trays) and returns a per-printer verdict.
//!
//! Used by Send, fleet queue, and (future) MCP.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    domain::{FileMeta, FirmwareKind, Printer, ToolUsage},
    driver::PrinterCapabilities,
    events::{AmsTray, PrinterState, PrinterStatus},
    filament::{self, LoadedSlot, SpoolFacts},
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
    /// Foreman can't start or control prints on this printer (Bambu in cloud mode).
    Unsupported,
}

/// Reason shown wherever a cloud-mode printer's print controls are refused.
pub const CLOUD_MODE: &str = "Not supported (cloud mode)";

/// Why Foreman can't send, start, pause, resume or cancel prints on `p`, if it can't.
pub fn control_unsupported(p: &Printer) -> Option<&'static str> {
    // Bambu runs read-only until the cloud transport (doc 05, 2C.4) exists.
    (p.firmware == FirmwareKind::Bambu).then_some(CLOUD_MODE)
}

/// How well what is loaded matches the file's filament, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum FilamentMatch {
    /// The loaded spool is the one the file's preset names (`(#ID)`).
    Exact,
    /// Loaded spool / tray matches material and colour.
    Colour,
    /// Loaded spool / tray matches material, colour differs or unknown.
    Material,
    /// Nothing loaded is known; the file's `(#ID)` preset is the only hint.
    Guess,
    /// No filament information (printer doesn't report, file has no material).
    Unknown,
    /// Loaded material doesn't match.
    Mismatch,
    /// Printer tracks spools and nothing is loaded.
    Empty,
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
    /// Filament match level (independent of the verdict's other checks).
    pub filament: FilamentMatch,
}

// ---------------------------------------------------------------------------
// Input snapshot (everything the engine needs, collected by the caller)
// ---------------------------------------------------------------------------

/// Everything the compatibility check needs for one printer.
pub struct PrinterSnapshot {
    pub printer: Printer,
    pub caps: Option<PrinterCapabilities>,
    pub status: Option<PrinterStatus>,
    /// Loaded spools per tool / slot (Klipper: Moonraker's active spool).
    pub loaded: Vec<LoadedSlot>,
}

impl PrinterSnapshot {
    /// The printer tracks a loaded spool (Moonraker with a `[spoolman]` link).
    fn tracks_spools(&self) -> bool {
        self.loaded.iter().any(|s| s.writable)
    }

    /// Spool loaded for a file tool: same tool index, else tool 0 on a
    /// single-tool printer.
    fn spool_for(&self, tool: u32) -> Option<&SpoolFacts> {
        let with: Vec<&LoadedSlot> = self.loaded.iter().filter(|s| s.spool.is_some()).collect();
        with.iter()
            .find(|s| s.tool == tool)
            .or_else(|| if with.len() == 1 { with.first() } else { None })
            .and_then(|s| s.spool.as_ref())
    }
}

/// Match one file tool against a loaded spool.
fn spool_match(t: &ToolUsage, sp: &SpoolFacts) -> FilamentMatch {
    if t.spool_id.is_some() && t.spool_id == Some(sp.id) {
        return FilamentMatch::Exact;
    }
    let Some(want) = t.material.as_deref() else {
        return FilamentMatch::Unknown;
    };
    let have = sp.material.as_deref().unwrap_or("");
    if !filament::materials_match(want, have) {
        return FilamentMatch::Mismatch;
    }
    let colour_ok = t.colour.as_deref().is_some_and(|c| {
        sp.colours()
            .iter()
            .filter_map(|h| filament::hex_distance(h, c))
            .any(|d| d <= filament::COLOUR_MATCH_DISTANCE)
    });
    if colour_ok {
        FilamentMatch::Colour
    } else {
        FilamentMatch::Material
    }
}

/// Worst match across the file's tools (one mismatching tool spoils it).
fn worst(ms: impl Iterator<Item = FilamentMatch>) -> Option<FilamentMatch> {
    ms.max_by_key(|m| match m {
        FilamentMatch::Exact => 0,
        FilamentMatch::Colour => 1,
        FilamentMatch::Material => 2,
        FilamentMatch::Guess => 3,
        FilamentMatch::Unknown => 4,
        FilamentMatch::Mismatch => 5,
        FilamentMatch::Empty => 6,
    })
}

/// Filament match for a file on a printer, plus a short "what's loaded" note.
fn filament_check(file: &FileMeta, snap: &PrinterSnapshot) -> (FilamentMatch, Option<String>) {
    let tools: Vec<&ToolUsage> = file.tools.iter().filter(|t| t.material.is_some()).collect();
    let has_guess =
        file.tools.iter().any(|t| t.spool_id.is_some()) || file.suggested_spool_id.is_some();
    let ams = snap
        .status
        .as_ref()
        .map(|s| &s.ams_trays[..])
        .unwrap_or(&[]);

    // Loaded spools (Klipper active spool) win over everything else.
    if snap.loaded.iter().any(|s| s.spool.is_some()) {
        let first = snap.loaded.iter().find_map(|s| s.spool.as_ref());
        let note = first.map(|sp| format!("{} {} loaded", sp.label(), sp.material_label()));
        if tools.is_empty() {
            return (FilamentMatch::Unknown, note);
        }
        let m = worst(tools.iter().map(|t| match snap.spool_for(t.index) {
            Some(sp) => spool_match(t, sp),
            None => FilamentMatch::Empty,
        }))
        .unwrap_or(FilamentMatch::Unknown);
        return (m, note);
    }
    if !ams.is_empty() {
        if tools.is_empty() {
            return (FilamentMatch::Unknown, None);
        }
        let m = worst(tools.iter().map(|t| {
            let mat = t.material.as_deref().unwrap_or("");
            let want = t.colour.as_deref();
            let same: Vec<&AmsTray> = ams.iter().filter(|tr| tray_has_material(tr, mat)).collect();
            let colour_ok = same.iter().any(|tr| match (want, tr.color.as_deref()) {
                (Some(w), Some(c)) => filament::hex_distance(w, c)
                    .is_some_and(|d| d <= filament::COLOUR_MATCH_DISTANCE),
                _ => false,
            });
            if same.is_empty() {
                FilamentMatch::Mismatch
            } else if colour_ok {
                FilamentMatch::Colour
            } else {
                FilamentMatch::Material
            }
        }))
        .unwrap_or(FilamentMatch::Unknown);
        return (m, None);
    }
    if snap.tracks_spools() && !tools.is_empty() {
        return (FilamentMatch::Empty, None);
    }
    if has_guess {
        return (FilamentMatch::Guess, None);
    }
    (FilamentMatch::Unknown, None)
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
    let (fil, loaded_note) = filament_check(file, snap);
    if let Some(reason) = control_unsupported(p) {
        return CompatResult {
            printer_id: id,
            printer_name: name,
            verdict: Verdict::Unsupported,
            reason: reason.into(),
            quick_fix: None,
            filament: fil,
        };
    }
    if *state == PrinterState::Offline {
        return CompatResult {
            printer_id: id,
            printer_name: name,
            verdict: Verdict::Offline,
            reason: "offline".into(),
            quick_fix: None,
            filament: fil,
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
                        filament: fil,
                    };
                }
            }
        }
    }

    // 3. Nozzle: abrasive filament needs a hardened nozzle. The loaded spool's
    //    stored `abrasive` flag decides; the file's material string is only used
    //    when no spool is known for that tool (it carries no flag).
    let needs_hardened = file.tools.iter().any(|t| match snap.spool_for(t.index) {
        Some(sp) => sp.abrasive,
        None => crate::materials::is_abrasive_filament(t.material.as_deref(), None),
    });
    if needs_hardened && !p.hardened_nozzle {
        return CompatResult {
            printer_id: id,
            printer_name: name,
            verdict: Verdict::Nozzle,
            reason: "file uses abrasive filament; printer has no hardened nozzle".into(),
            quick_fix: None,
            filament: fil,
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
                        filament: fil,
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
            filament: fil,
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
            filament: fil,
        };
    }

    // 7. Material / spool match: loaded spool (Klipper), AMS trays (Bambu).
    let wanted = || {
        file.tools
            .iter()
            .filter_map(|t| t.material.as_deref())
            .next()
            .unwrap_or("filament")
            .to_owned()
    };
    match fil {
        FilamentMatch::Mismatch => {
            let reason = match &loaded_note {
                Some(n) => format!("file wants {}; {n}", wanted()),
                None => format!("{} not loaded", wanted()),
            };
            return CompatResult {
                printer_id: id,
                printer_name: name,
                verdict: Verdict::Material,
                reason,
                quick_fix: Some("Load matching spool".into()),
                filament: fil,
            };
        }
        FilamentMatch::Empty => {
            return CompatResult {
                printer_id: id,
                printer_name: name,
                verdict: Verdict::NoSpool,
                reason: format!("no spool loaded · file wants {}", wanted()),
                quick_fix: Some("Load spool".into()),
                filament: fil,
            };
        }
        _ => {}
    }

    // 8. Ready.
    let mut reason = match (&loaded_note, fil) {
        (Some(n), FilamentMatch::Exact) => format!("idle · {n} · the file's spool"),
        (Some(n), FilamentMatch::Colour) => format!("idle · {n} · material and colour match"),
        (Some(n), FilamentMatch::Material) => format!("idle · {n} · colour differs"),
        (Some(n), _) => format!("idle · {n}"),
        (None, FilamentMatch::Guess) => {
            let guess = file
                .tools
                .iter()
                .find_map(|t| t.spool_id)
                .or(file.suggested_spool_id)
                .map(|g| format!("idle · preset names spool #{g}, nothing loaded is known"))
                .unwrap_or_else(|| "idle".into());
            guess
        }
        (None, _) => snap
            .status
            .as_ref()
            .and_then(|s| {
                s.ams_trays
                    .first()
                    .and_then(|t| t.material.as_deref())
                    .map(|m| format!("idle · {m} loaded"))
            })
            .unwrap_or_else(|| "idle".into()),
    };
    let mut quick_fix = None;
    // Enough filament left on the loaded spool?
    for t in &file.tools {
        if let (Some(g), Some(sp)) = (t.grams, snap.spool_for(t.index)) {
            if let Some(left) = sp.remaining_weight {
                if left < g {
                    reason.push_str(&format!(" · only {left:.0} g left, needs {g:.0} g"));
                    quick_fix = Some("Load a fuller spool".into());
                }
            }
        }
    }

    CompatResult {
        printer_id: id,
        printer_name: name,
        verdict: Verdict::Ready,
        reason,
        quick_fix,
        filament: fil,
    }
}

/// Check all printers for one file.
pub fn check_all(file: &FileMeta, printers: &[PrinterSnapshot]) -> Vec<CompatResult> {
    let mut results: Vec<CompatResult> = printers.iter().map(|snap| check(file, snap)).collect();
    // Sort: Ready first, then Busy, Plate, Material, NoSpool, TooBig, Nozzle, Offline, Unsupported.
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
        Verdict::Unsupported => 8,
    }
}

/// How well the loaded filament matches the file, per tool: +4 exact spool,
/// +3 material and colour, +2 material. 0 when nothing loaded is known.
/// The fleet queue prefers the highest score.
pub fn filament_score(file: &FileMeta, snap: &PrinterSnapshot) -> u32 {
    let trays = snap
        .status
        .as_ref()
        .map(|s| &s.ams_trays[..])
        .unwrap_or(&[]);
    let mut score = 0;
    for tool in &file.tools {
        if let Some(sp) = snap.spool_for(tool.index) {
            score += match spool_match(tool, sp) {
                FilamentMatch::Exact => 4,
                FilamentMatch::Colour => 3,
                FilamentMatch::Material => 2,
                _ => 0,
            };
            continue;
        }
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
            usage_reporter: Default::default(),
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
            loaded: vec![],
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
            loaded: vec![],
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
            loaded: vec![],
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
            loaded: vec![],
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
            loaded: vec![],
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
            loaded: vec![],
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
            loaded: vec![],
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
            loaded: vec![],
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
            loaded: vec![],
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
                loaded: vec![],
            },
            PrinterSnapshot {
                printer: make_printer(2, "Idle"),
                caps: Some(make_caps(120.0, 120.0)),
                status: Some(make_status(PrinterState::Idle)),
                loaded: vec![],
            },
            PrinterSnapshot {
                printer: make_printer(3, "Printing"),
                caps: Some(make_caps(120.0, 120.0)),
                status: Some(make_status(PrinterState::Printing)),
                loaded: vec![],
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
            loaded: vec![],
        };
        let r = check(&meta, &snap);
        assert_eq!(r.verdict, Verdict::Nozzle);
    }

    fn klipper_slot(spool: Option<SpoolFacts>) -> LoadedSlot {
        LoadedSlot {
            printer_id: 1,
            printer_name: "V0".into(),
            tool: 0,
            label: "T0".into(),
            spool_id: spool.as_ref().map(|s| s.id),
            spool,
            tray: None,
            writable: true,
            read_only_reason: None,
        }
    }

    fn pla_white(id: i64, grams: f64) -> SpoolFacts {
        SpoolFacts {
            id,
            name: "PLA Basic White".into(),
            material: Some("PLA".into()),
            color_hex: Some("FAFAFA".into()),
            remaining_weight: Some(grams),
            ..Default::default()
        }
    }

    fn idle_klipper(loaded: Vec<LoadedSlot>) -> PrinterSnapshot {
        PrinterSnapshot {
            printer: make_printer(1, "V0"),
            caps: Some(make_caps(120.0, 120.0)),
            status: Some(make_status(PrinterState::Idle)),
            loaded,
        }
    }

    #[test]
    fn loaded_spool_improves_on_the_preset_guess() {
        let mut meta = make_meta(); // PLA #FFFFFF 50 g
        meta.tools[0].spool_id = Some(9);
        // No [spoolman] link: only the preset hint.
        let before = check(&meta, &idle_klipper(vec![]));
        assert_eq!(before.verdict, Verdict::Ready);
        assert_eq!(before.filament, FilamentMatch::Guess);
        assert!(before.reason.contains("#9"), "{}", before.reason);
        // Load a matching spool: better verdict, reason names it.
        let after = check(
            &meta,
            &idle_klipper(vec![klipper_slot(Some(pla_white(1, 800.0)))]),
        );
        assert_eq!(after.verdict, Verdict::Ready);
        assert_eq!(after.filament, FilamentMatch::Colour);
        assert!(
            after.reason.contains("#1 PLA Basic White"),
            "{}",
            after.reason
        );
        let snap_before = idle_klipper(vec![]);
        let snap_after = idle_klipper(vec![klipper_slot(Some(pla_white(1, 800.0)))]);
        assert!(filament_score(&meta, &snap_after) > filament_score(&meta, &snap_before));
        // The preset's own spool loaded: exact.
        let exact = check(
            &meta,
            &idle_klipper(vec![klipper_slot(Some(pla_white(9, 800.0)))]),
        );
        assert_eq!(exact.filament, FilamentMatch::Exact);
        assert!(
            filament_score(
                &meta,
                &idle_klipper(vec![klipper_slot(Some(pla_white(9, 800.0)))])
            ) > filament_score(&meta, &snap_after)
        );
    }

    #[test]
    fn nozzle_check_reads_the_loaded_spools_abrasive_flag() {
        // File says plain PLA, but the loaded spool is flagged abrasive (e.g. Glow).
        let meta = make_meta();
        let mut glow = pla_white(1, 800.0);
        glow.abrasive = true;
        let r = check(&meta, &idle_klipper(vec![klipper_slot(Some(glow))]));
        assert_eq!(r.verdict, Verdict::Nozzle);
        // File says PETG-CF, loaded spool's flag was unticked by the user: no block.
        let mut meta = make_meta();
        meta.tools[0].material = Some("PETG-CF".into());
        let mut petg = pla_white(2, 800.0);
        petg.material = Some("PETG".into());
        petg.modifier = Some("CF".into());
        petg.abrasive = false;
        let r = check(&meta, &idle_klipper(vec![klipper_slot(Some(petg))]));
        assert_ne!(r.verdict, Verdict::Nozzle, "{}", r.reason);
    }

    #[test]
    fn no_spool_only_when_nothing_is_loaded() {
        let meta = make_meta();
        let r = check(&meta, &idle_klipper(vec![klipper_slot(None)]));
        assert_eq!(r.verdict, Verdict::NoSpool);
        assert_eq!(r.filament, FilamentMatch::Empty);
        assert_eq!(r.quick_fix.as_deref(), Some("Load spool"));
        let r = check(
            &meta,
            &idle_klipper(vec![klipper_slot(Some(pla_white(1, 800.0)))]),
        );
        assert_eq!(r.verdict, Verdict::Ready);
    }

    #[test]
    fn loaded_spool_of_another_material_is_a_material_verdict() {
        let mut meta = make_meta();
        meta.tools[0].material = Some("PETG".into());
        let r = check(
            &meta,
            &idle_klipper(vec![klipper_slot(Some(pla_white(1, 800.0)))]),
        );
        assert_eq!(r.verdict, Verdict::Material);
        assert_eq!(r.filament, FilamentMatch::Mismatch);
        assert!(
            r.reason.contains("PETG") && r.reason.contains("#1"),
            "{}",
            r.reason
        );
    }

    #[test]
    fn loaded_spool_colour_differs_and_low_remaining() {
        let mut meta = make_meta();
        meta.tools[0].colour = Some("#000000".into());
        let r = check(
            &meta,
            &idle_klipper(vec![klipper_slot(Some(pla_white(1, 30.0)))]),
        );
        assert_eq!(r.verdict, Verdict::Ready);
        assert_eq!(r.filament, FilamentMatch::Material);
        assert!(
            r.reason.contains("only 30 g left, needs 50 g"),
            "{}",
            r.reason
        );
        assert_eq!(r.quick_fix.as_deref(), Some("Load a fuller spool"));
    }
}
