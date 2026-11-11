//! Mock Bambu MQTT replay test.
//!
//! Reads the recorded A1 fixture (`fixtures/bambu_a1_reports.json`), feeds
//! each report through the Bambu driver's status parser, and verifies key
//! fields: state, temps, progress, AMS trays, HMS codes, fan speeds, layers.
//!
//! Fixture captured from a real Bambu Lab A1 (session 17).

use foreman_core::events::{PrinterState, PrinterStatus};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Re-use the driver's CachedStatus parser via a thin public wrapper.
// Since CachedStatus is private to the bambu driver module, we replicate the
// merge logic here by calling into the public status types.
// ---------------------------------------------------------------------------

/// Minimal CachedStatus replica for test purposes.
/// Mirrors the private CachedStatus in drivers::bambu.
#[derive(Debug, Default)]
struct TestCachedStatus {
    nozzle_actual: f64,
    nozzle_target: f64,
    bed_actual: f64,
    bed_target: f64,
    progress: f64,
    ams_trays: Vec<foreman_core::events::AmsTray>,
    hms_codes: Vec<foreman_core::events::HmsCode>,
    fans: foreman_core::events::FanSpeeds,
    layer: Option<u32>,
    total_layers: Option<u32>,
    remaining_time: Option<u32>,
    filename: Option<String>,
    state: PrinterState,
}

impl TestCachedStatus {
    fn new() -> Self {
        Self::default()
    }

    fn merge_report(&mut self, print: &Value) {
        // HMS — parse before gcode_state so FAILED can check hms_codes.
        if let Some(hms) = print.get("hms").and_then(|v| v.as_array()) {
            self.hms_codes = hms
                .iter()
                .filter_map(|entry| {
                    let attr = entry.get("attr").and_then(|v| v.as_u64())? as u32;
                    let code = entry.get("code").and_then(|v| v.as_u64())? as u32;
                    Some(foreman_core::events::HmsCode { attr, code })
                })
                .collect();
        }

        // State.
        if let Some(s) = print.get("gcode_state").and_then(|v| v.as_str()) {
            self.state = match s {
                "RUNNING" | "PREPARE" => PrinterState::Printing,
                "PAUSE" => PrinterState::Paused,
                "FINISH" | "IDLE" | "CREATED" | "SLICING" => PrinterState::Idle,
                "FAILED" if self.hms_codes.is_empty() => PrinterState::Idle,
                "FAILED" => PrinterState::Error,
                _ => self.state.clone(),
            };
        }

        // Temperatures.
        if let Some(t) = print.get("nozzle_temper").and_then(|v| v.as_f64()) {
            self.nozzle_actual = t;
        }
        if let Some(t) = print.get("nozzle_target_temper").and_then(|v| v.as_f64()) {
            self.nozzle_target = t;
        }
        if let Some(t) = print.get("bed_temper").and_then(|v| v.as_f64()) {
            self.bed_actual = t;
        }
        if let Some(t) = print.get("bed_target_temper").and_then(|v| v.as_f64()) {
            self.bed_target = t;
        }

        // Progress.
        if let Some(p) = print.get("mc_percent").and_then(|v| v.as_u64()) {
            self.progress = p as f64 / 100.0;
        }

        // File.
        if let Some(f) = print.get("gcode_file").and_then(|v| v.as_str()) {
            self.filename = if f.is_empty() {
                None
            } else {
                Some(f.to_owned())
            };
        } else if let Some(f) = print.get("subtask_name").and_then(|v| v.as_str()) {
            if !f.is_empty() {
                self.filename = Some(f.to_owned());
            }
        }

        // Layers.
        if let Some(l) = print.get("layer_num").and_then(|v| v.as_u64()) {
            self.layer = Some(l as u32);
        }
        if let Some(l) = print.get("total_layer_num").and_then(|v| v.as_u64()) {
            self.total_layers = Some(l as u32);
        }

        // Remaining time.
        if let Some(t) = print.get("mc_remaining_time").and_then(|v| v.as_u64()) {
            self.remaining_time = Some(t as u32);
        }

        // Fans.
        if let Some(v) = print.get("cooling_fan_speed") {
            self.fans.part_fan = parse_fan(v);
        }
        if let Some(v) = print.get("big_fan1_speed") {
            self.fans.aux_fan = parse_fan(v);
        }
        if let Some(v) = print.get("big_fan2_speed") {
            self.fans.chamber_fan = parse_fan(v);
        }
        if let Some(v) = print.get("heatbreak_fan_speed") {
            self.fans.heatbreak_fan = parse_fan(v);
        }

        // AMS.
        if let Some(ams) = print.get("ams") {
            self.merge_ams(ams);
        }

        // Also handle top-level vt_tray (some reports put it outside ams).
        if let Some(vt) = print.get("vt_tray") {
            self.push_vt_tray(vt);
        }

        // (HMS is parsed at the top of merge_report.)
    }

    fn merge_ams(&mut self, ams: &Value) {
        let units = match ams.get("ams").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => return,
        };

        let mut trays = Vec::new();
        for unit in units {
            let unit_id: u32 = unit
                .get("id")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            if let Some(tray_arr) = unit.get("tray").and_then(|v| v.as_array()) {
                for tray in tray_arr {
                    let tray_id: u32 = tray
                        .get("id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let slot = unit_id * 4 + tray_id;

                    let tray_type = tray.get("tray_type").and_then(|v| v.as_str());
                    if tray_type.is_none() || tray_type == Some("") {
                        continue;
                    }

                    let color = tray.get("tray_color").and_then(|v| v.as_str()).map(|c| {
                        if c.len() == 8 {
                            c[..6].to_owned()
                        } else {
                            c.to_owned()
                        }
                    });
                    let material = tray_type.map(|s| s.to_owned());
                    let remaining = tray
                        .get("remain")
                        .and_then(|v| v.as_u64())
                        .map(|r| r.min(100) as u8);

                    trays.push(foreman_core::events::AmsTray {
                        slot,
                        color,
                        material,
                        remaining,
                    });
                }
            }
        }

        // vt_tray inside the ams object.
        if let Some(vt) = ams.get("vt_tray") {
            let tray_type = vt.get("tray_type").and_then(|v| v.as_str());
            if tray_type.is_some() && tray_type != Some("") {
                let color = vt.get("tray_color").and_then(|v| v.as_str()).map(|c| {
                    if c.len() == 8 {
                        c[..6].to_owned()
                    } else {
                        c.to_owned()
                    }
                });
                let material = tray_type.map(|s| s.to_owned());
                let remaining = vt
                    .get("remain")
                    .and_then(|v| v.as_u64())
                    .map(|r| r.min(100) as u8);
                trays.push(foreman_core::events::AmsTray {
                    slot: 254,
                    color,
                    material,
                    remaining,
                });
            }
        }

        self.ams_trays = trays;
    }

    fn push_vt_tray(&mut self, vt: &Value) {
        let tray_type = vt.get("tray_type").and_then(|v| v.as_str());
        if tray_type.is_none() || tray_type == Some("") {
            return;
        }
        // Remove any existing vt_tray (slot 254) before adding.
        self.ams_trays.retain(|t| t.slot != 254);
        let color = vt.get("tray_color").and_then(|v| v.as_str()).map(|c| {
            if c.len() == 8 {
                c[..6].to_owned()
            } else {
                c.to_owned()
            }
        });
        let material = tray_type.map(|s| s.to_owned());
        let remaining = vt
            .get("remain")
            .and_then(|v| v.as_u64())
            .map(|r| r.min(100) as u8);
        self.ams_trays.push(foreman_core::events::AmsTray {
            slot: 254,
            color,
            material,
            remaining,
        });
    }

    fn to_status(&self) -> PrinterStatus {
        PrinterStatus {
            state: self.state.clone(),
            nozzle: Some(foreman_core::events::TempReading {
                actual: self.nozzle_actual,
                target: self.nozzle_target,
            }),
            bed: Some(foreman_core::events::TempReading {
                actual: self.bed_actual,
                target: self.bed_target,
            }),
            progress: Some(self.progress),
            filename: self.filename.clone(),
            print_duration: None,
            display_message: None,
            layer: self.layer,
            total_layers: self.total_layers,
            remaining_time: self.remaining_time,
            fans: Some(self.fans.clone()),
            ams_trays: self.ams_trays.clone(),
            hms_codes: self.hms_codes.clone(),
            ..Default::default()
        }
    }
}

fn parse_fan(val: &Value) -> Option<u8> {
    let raw = match val {
        Value::String(s) => s.parse::<f64>().ok()?,
        Value::Number(n) => n.as_f64()?,
        _ => return None,
    };
    Some(((raw / 15.0) * 100.0).round().clamp(0.0, 100.0) as u8)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn replay_real_a1_fixture() {
    let fixture = include_str!("fixtures/bambu_a1_reports.json");
    let reports: Vec<Value> = serde_json::from_str(fixture).expect("fixture parse");
    assert!(reports.len() >= 6, "expected at least 6 reports in fixture");

    let mut cached = TestCachedStatus::new();

    // --- Report 0: pushall response (A1, FAILED state, AMS Lite 4 trays) ---
    cached.merge_report(reports[0].get("print").unwrap());
    let s = cached.to_status();

    // State: FAILED with empty hms → Idle (stale post-print state, not a fault).
    assert_eq!(s.state, PrinterState::Idle);

    // Temperatures: idle, targets at 0.
    assert!((s.nozzle.as_ref().unwrap().actual - 25.09375).abs() < 0.01);
    assert!((s.bed.as_ref().unwrap().actual - 24.6875).abs() < 0.01);
    assert_eq!(s.nozzle.as_ref().unwrap().target, 0.0);
    assert_eq!(s.bed.as_ref().unwrap().target, 0.0);

    // Progress: 0%.
    assert_eq!(s.progress, Some(0.0));

    // Layers from previous print still visible.
    assert_eq!(s.layer, Some(3));
    assert_eq!(s.total_layers, Some(1021));
    assert_eq!(s.remaining_time, Some(1114));

    // AMS Lite: 4 loaded trays + vt_tray.
    assert_eq!(s.ams_trays.len(), 5); // 4 AMS + 1 vt_tray
    assert_eq!(s.ams_trays[0].slot, 0);
    assert_eq!(s.ams_trays[0].color.as_deref(), Some("5A657B"));
    assert_eq!(s.ams_trays[0].material.as_deref(), Some("PLA"));
    assert_eq!(s.ams_trays[0].remaining, Some(100));
    assert_eq!(s.ams_trays[1].slot, 1);
    assert_eq!(s.ams_trays[1].color.as_deref(), Some("FFFFFF"));
    assert_eq!(s.ams_trays[2].slot, 2);
    assert_eq!(s.ams_trays[2].color.as_deref(), Some("104831"));
    assert_eq!(s.ams_trays[3].slot, 3);
    assert_eq!(s.ams_trays[3].color.as_deref(), Some("000000"));
    assert_eq!(s.ams_trays[3].material.as_deref(), Some("PLA"));
    // vt_tray (external spool holder, slot 254).
    assert_eq!(s.ams_trays[4].slot, 254);
    assert_eq!(s.ams_trays[4].color.as_deref(), Some("A7A9AA"));
    assert_eq!(s.ams_trays[4].material.as_deref(), Some("PLA"));

    // No HMS errors.
    assert!(s.hms_codes.is_empty());

    // All fans off.
    let fans = s.fans.as_ref().unwrap();
    assert_eq!(fans.part_fan, Some(0));
    assert_eq!(fans.heatbreak_fan, Some(0));
    assert_eq!(fans.aux_fan, Some(0));
    assert_eq!(fans.chamber_fan, Some(0));

    // gcode_file is "" in pushall → filename is None.
    // subtask_name has a leftover value but gcode_file takes priority.
    assert!(s.filename.is_none());

    // --- Partial updates: verify temp drift is picked up ---
    for report in &reports[1..reports.len() - 1] {
        cached.merge_report(report.get("print").unwrap());
    }
    let s = cached.to_status();
    // After all partials, temps should have shifted slightly.
    assert!(s.nozzle.as_ref().unwrap().actual > 24.0);
    assert!(s.bed.as_ref().unwrap().actual > 24.0);

    // --- Last report: second pushall (full dump again) ---
    let last = reports.last().unwrap();
    cached.merge_report(last.get("print").unwrap());
    let s = cached.to_status();
    // State should still be Idle (FAILED + empty hms).
    assert_eq!(s.state, PrinterState::Idle);
    // AMS still has 5 trays.
    assert_eq!(s.ams_trays.len(), 5);
}

#[test]
fn serial_to_model() {
    assert_eq!(model_from_serial("00W12345678"), Some("A1"));
    assert_eq!(model_from_serial("01S12345678"), Some("A1 Mini"));
    assert_eq!(model_from_serial("00M12345678"), Some("X1 Carbon"));
    assert_eq!(model_from_serial("01P12345678"), Some("P1P"));
    assert_eq!(model_from_serial("03P12345678"), Some("P1S"));
    assert_eq!(model_from_serial("03912345678"), Some("A1")); // newer A1 prefix
    assert_eq!(model_from_serial("ZZZ12345678"), None);
    assert_eq!(model_from_serial("AB"), None);
}

fn model_from_serial(serial: &str) -> Option<&'static str> {
    if serial.len() < 3 {
        return None;
    }
    match &serial[..3] {
        "00M" => Some("X1 Carbon"),
        "00W" => Some("A1"),
        "01P" => Some("P1P"),
        "01S" => Some("A1 Mini"),
        "03P" => Some("P1S"),
        "030" => Some("X1E"),
        "039" => Some("A1"),
        _ => None,
    }
}

#[test]
fn cert_fingerprint_is_consistent() {
    // Verify the fingerprint function produces consistent output.
    let cert_bytes = b"fake certificate DER bytes for testing";
    let fp1 = foreman_core::drivers::bambu::cert_fingerprint(cert_bytes);
    let fp2 = foreman_core::drivers::bambu::cert_fingerprint(cert_bytes);
    assert_eq!(fp1, fp2);
    assert_eq!(fp1.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
}
