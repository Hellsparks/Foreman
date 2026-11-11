//! Filament slots from a sliced file (task 5.2, W9): one model for toolchangers,
//! AMS / MMU, and single-extruder colour changes.
//!
//! How slicers write it (checked against upstream source, 2026-11-11):
//! - **PrusaSlicer** (`GCode.cpp`, `emit_custom_color_change_gcode_per_print_z`):
//!   `;COLOR_CHANGE,T<extruder>,<#colour>` then `color_change_gcode` (default `M600`).
//! - **OrcaSlicer** (`GCode.cpp`): `;` + reserved tag + `,T<n>,<#colour>`; the tag is
//!   `" COLOR_CHANGE"` for Bambu printers and `"COLOR_CHANGE"` otherwise, then
//!   `color_change_gcode`.
//! - Neither writes grams per colour (`; filament used [g]` is per extruder), so
//!   the body is scanned and extrusion is split at each change.
//! - A bare `M600` without the tag (hand-inserted, Cura plugins) also counts.
//! - **AMS / MMU**: `single_extruder_multi_material = 1` in the config block, Bambu
//!   `M620` tool changes, or a Bambu 3MF (`slice_info.config` `<filament id
//!   tray_info_idx type color used_m used_g …>`). The 3MF holds no AMS tray map;
//!   the tray is picked when the print is sent.
//! - **Toolchanger**: several filaments used, none of the above.

use crate::domain::{FilamentSlot, SlotKind, ToolUsage};

/// Streams the G-code body: colour changes and extrusion per segment.
#[derive(Debug, Clone)]
pub struct BodyScan {
    relative_e: bool,
    last_e: f64,
    /// Extruded mm per segment (segment 0 = before the first change).
    pub segments: Vec<f64>,
    /// Colour set at each change (segments 1..).
    pub change_colours: Vec<Option<String>>,
    /// A `COLOR_CHANGE` tag was seen; the next `M600` belongs to it.
    pending_tag: bool,
    /// Bambu AMS tool change (`M620 S…`).
    pub m620: bool,
    carry: Vec<u8>,
}

impl Default for BodyScan {
    fn default() -> Self {
        Self {
            relative_e: false,
            last_e: 0.0,
            segments: vec![0.0],
            change_colours: vec![],
            pending_tag: false,
            m620: false,
            carry: vec![],
        }
    }
}

fn hex_colour(v: &str) -> Option<String> {
    let h = v.trim().trim_matches('"').strip_prefix('#')?;
    (h.len() >= 6 && h[..6].chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("#{}", h[..6].to_ascii_uppercase()))
}

fn word(code: &str, letter: char) -> Option<f64> {
    code.split_whitespace()
        .find_map(|w| {
            w.strip_prefix(letter)
                .or_else(|| w.strip_prefix(letter.to_ascii_lowercase()))
        })
        .and_then(|v| v.parse().ok())
}

impl BodyScan {
    /// Feed raw bytes (any chunking).
    pub fn feed(&mut self, bytes: &[u8]) {
        self.carry.extend_from_slice(bytes);
        let mut start = 0;
        while let Some(pos) = self.carry[start..].iter().position(|b| *b == b'\n') {
            let end = start + pos;
            let line = String::from_utf8_lossy(&self.carry[start..end]).into_owned();
            self.line(&line);
            start = end + 1;
        }
        self.carry.drain(..start);
    }

    /// Flush a last line without a newline.
    pub fn finish(&mut self) {
        if !self.carry.is_empty() {
            let line = String::from_utf8_lossy(&self.carry).into_owned();
            self.carry.clear();
            self.line(&line);
        }
    }

    fn change(&mut self, colour: Option<String>) {
        self.segments.push(0.0);
        self.change_colours.push(colour);
    }

    pub fn line(&mut self, raw: &str) {
        let line = raw.trim();
        if let Some(comment) = line.strip_prefix(';') {
            // `;COLOR_CHANGE,T0,#E2621F` (Prusa / Orca generic) or `; COLOR_CHANGE,…` (Orca BBL).
            if let Some(rest) = comment.trim_start().strip_prefix("COLOR_CHANGE") {
                let colour = rest.split(',').find_map(hex_colour);
                self.change(colour);
                self.pending_tag = true;
            }
            return;
        }
        let code = line.split(';').next().unwrap_or("").trim();
        if code.is_empty() {
            return;
        }
        let cmd = code
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        match cmd.as_str() {
            "G0" | "G1" | "G2" | "G3" => {
                if let Some(e) = word(code, 'E') {
                    let d = if self.relative_e { e } else { e - self.last_e };
                    if !self.relative_e {
                        self.last_e = e;
                    }
                    if let Some(s) = self.segments.last_mut() {
                        *s += d;
                    }
                }
            }
            "G92" => {
                if let Some(e) = word(code, 'E') {
                    self.last_e = e;
                }
            }
            "M82" => self.relative_e = false,
            "M83" => self.relative_e = true,
            "M600" => {
                if self.pending_tag {
                    self.pending_tag = false;
                } else {
                    self.change(None);
                }
            }
            // Bambu AMS tool change: `M620 S1A` (tray + `A`); `M620.1` etc. are not.
            "M620"
                if code
                    .split_whitespace()
                    .skip(1)
                    .any(|w| w.starts_with(['S', 's'])) =>
            {
                self.m620 = true
            }
            _ => {}
        }
    }

    pub fn changes(&self) -> usize {
        self.change_colours.len()
    }
}

/// Tools that print (unknown usage counts as used).
fn used(tools: &[ToolUsage]) -> Vec<&ToolUsage> {
    tools
        .iter()
        .filter(|t| t.grams.map(|g| g > 0.0).unwrap_or(true))
        .collect()
}

fn slot_of(t: &ToolUsage, kind: SlotKind, tray: Option<&String>) -> FilamentSlot {
    FilamentSlot {
        index: t.index,
        tool: t.index,
        label: match kind {
            SlotKind::Ams => format!("AMS {}", t.index + 1),
            _ => format!("T{}", t.index),
        },
        material: t.material.clone(),
        colour: t.colour.clone(),
        grams: t.grams,
        mm: t.mm,
        preset: t.preset.clone(),
        spool_id: t.spool_id,
        tray: tray.cloned(),
    }
}

/// What a file needs, from its per-tool header facts and (when scanned) its body.
///
/// - `semm`: `single_extruder_multi_material = 1` in the config.
/// - `bambu_3mf`: the file is a Bambu / Orca 3MF with `slice_info.config`.
/// - `trays`: Bambu `tray_info_idx` per filament id (0-based).
pub fn build(
    tools: &[ToolUsage],
    body: Option<&BodyScan>,
    semm: bool,
    bambu_3mf: bool,
    trays: &[(u32, String)],
) -> (SlotKind, Vec<FilamentSlot>) {
    let used = used(tools);
    let tray = |i: u32| trays.iter().find(|(t, _)| *t == i).map(|(_, v)| v);
    if let Some(b) = body.filter(|b| b.changes() > 0 && used.len() <= 1) {
        // One extruder, filaments in sequence.
        let base = used.first().copied().or(tools.first());
        let total_mm: f64 = b.segments.iter().map(|s| s.max(0.0)).sum();
        let n = b.segments.len();
        let slots = (0..n)
            .map(|i| {
                let frac = if total_mm > 0.0 {
                    b.segments[i].max(0.0) / total_mm
                } else {
                    1.0 / n as f64
                };
                let colour = if i == 0 {
                    base.and_then(|t| t.colour.clone())
                } else {
                    b.change_colours.get(i - 1).cloned().flatten()
                };
                FilamentSlot {
                    index: i as u32,
                    tool: base.map(|t| t.index).unwrap_or(0),
                    label: format!("Colour {}", i + 1),
                    material: base.and_then(|t| t.material.clone()),
                    colour,
                    grams: base.and_then(|t| t.grams).map(|g| round2(g * frac)),
                    mm: base.and_then(|t| t.mm).map(|m| round2(m * frac)),
                    preset: base.and_then(|t| t.preset.clone()),
                    // Each colour is its own spool; a preset's spool only fits the first.
                    spool_id: if i == 0 {
                        base.and_then(|t| t.spool_id)
                    } else {
                        None
                    },
                    tray: None,
                }
            })
            .collect();
        return (SlotKind::Change, slots);
    }
    if used.len() > 1 {
        let ams = semm || bambu_3mf || body.is_some_and(|b| b.m620);
        let kind = if ams { SlotKind::Ams } else { SlotKind::Tool };
        return (
            kind,
            used.iter()
                .map(|t| slot_of(t, kind, tray(t.index)))
                .collect(),
        );
    }
    let kind = if bambu_3mf {
        SlotKind::Ams
    } else {
        SlotKind::Tool
    };
    (
        kind,
        used.iter()
            .map(|t| slot_of(t, kind, tray(t.index)))
            .collect(),
    )
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(i: u32, mat: &str, col: &str, g: f64) -> ToolUsage {
        ToolUsage {
            index: i,
            material: Some(mat.into()),
            colour: Some(col.into()),
            grams: Some(g),
            mm: Some(g * 100.0),
            ..Default::default()
        }
    }

    fn scan(text: &str) -> BodyScan {
        let mut b = BodyScan::default();
        // Odd chunking on purpose.
        for c in text.as_bytes().chunks(7) {
            b.feed(c);
        }
        b.finish();
        b
    }

    #[test]
    fn prusa_colour_change_splits_by_extrusion() {
        let b = scan(
            "M83\nG1 X1 E10\nG1 X2 E-1\nG1 X3 E1\n;COLOR_CHANGE,T0,#e2621f\nM600\nG1 X4 E30\n",
        );
        assert_eq!(b.segments, vec![10.0, 30.0]);
        assert_eq!(b.change_colours, vec![Some("#E2621F".into())]);
        let (kind, slots) = build(
            &[tool(0, "PLA", "#3B3B3B", 40.0)],
            Some(&b),
            false,
            false,
            &[],
        );
        assert_eq!(kind, SlotKind::Change);
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].label, "Colour 1");
        assert_eq!(slots[0].colour.as_deref(), Some("#3B3B3B"));
        assert_eq!(slots[1].colour.as_deref(), Some("#E2621F"));
        assert_eq!((slots[0].grams, slots[1].grams), (Some(10.0), Some(30.0)));
        assert!(slots
            .iter()
            .all(|s| s.material.as_deref() == Some("PLA") && s.tool == 0));
    }

    #[test]
    fn orca_bbl_tag_absolute_e_and_bare_m600() {
        // Orca for Bambu printers writes "; COLOR_CHANGE"; absolute E with G92 resets.
        let b = scan(
            "M82\nG92 E0\nG1 E5\nG1 E10\n; COLOR_CHANGE,T0,#FFFFFF\nM600\nG92 E0\nG1 E20\nM600 ; by hand\nG92 E0\nG1 E10\n",
        );
        assert_eq!(b.segments, vec![10.0, 20.0, 10.0]);
        assert_eq!(b.change_colours, vec![Some("#FFFFFF".into()), None]);
    }

    #[test]
    fn config_lines_are_not_changes() {
        let b = scan("; color_change_gcode = M600\n; pause_print_gcode = M601\nG1 E1\n");
        assert_eq!(b.changes(), 0);
    }

    #[test]
    fn several_filaments_are_tools_or_ams() {
        let tools = [
            tool(0, "PLA", "#1F8A8A", 30.0),
            tool(1, "PLA", "#F4F4EF", 18.0),
            tool(2, "PETG", "#000000", 0.0),
        ];
        let (k, s) = build(&tools, Some(&BodyScan::default()), false, false, &[]);
        assert_eq!(k, SlotKind::Tool, "toolchanger");
        assert_eq!(
            s.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(),
            ["T0", "T1"],
            "unused tool dropped"
        );
        let (k, _) = build(&tools, None, true, false, &[]);
        assert_eq!(k, SlotKind::Ams, "single_extruder_multi_material");
        let mut m620 = BodyScan::default();
        m620.line("M620 S1A");
        assert_eq!(
            build(&tools, Some(&m620), false, false, &[]).0,
            SlotKind::Ams
        );
        let (k, s) = build(&tools, None, false, true, &[(1, "GFA00".into())]);
        assert_eq!(k, SlotKind::Ams);
        assert_eq!(s[1].label, "AMS 2");
        assert_eq!(s[1].tray.as_deref(), Some("GFA00"));
    }

    #[test]
    fn single_filament_is_one_tool_slot() {
        let (k, s) = build(
            &[tool(0, "ASA", "#26A69A", 8.23)],
            Some(&BodyScan::default()),
            false,
            false,
            &[],
        );
        assert_eq!(k, SlotKind::Tool);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].label, "T0");
    }
}
