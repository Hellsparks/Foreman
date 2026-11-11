//! Calibration maths (W10, task 5.4). Pure functions, no I/O.
//!
//! Two flows that never mix:
//! - **Skew** belongs to a printer (Klipper `[skew_correction]`). The factor is
//!   computed with Klipper's own `calc_skew_factor` (klippy/extras/skew_correction.py).
//! - **Shrinkage** belongs to a filament. Ported from Calistar / fleur_de_cali
//!   (`calcDimensionality`): mean relative error over measured spans, and the
//!   OrcaSlicer value `shrinkage = 100 / (1 + err)` %.
//!
//! Plus the text patch that adds `SKEW_PROFILE LOAD=foreman` to PRINT_START,
//! `SET_SKEW CLEAR=1` to PRINT_END and `[skew_correction]` to printer.cfg, and a
//! small line diff for the approval step.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::AppError;

// ---------------------------------------------------------------------------
// Skew
// ---------------------------------------------------------------------------

/// The Klipper profile name Foreman saves and loads.
pub const SKEW_PROFILE: &str = "foreman";

/// A plane Klipper can correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Plane {
    XY,
    XZ,
    YZ,
}

impl Plane {
    pub fn as_str(self) -> &'static str {
        match self {
            Plane::XY => "XY",
            Plane::XZ => "XZ",
            Plane::YZ => "YZ",
        }
    }
}

/// Measured lengths of one plane of the skew test piece, in mm:
/// corner A→C, B→D and side A→D (without the corner flats).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlaneMeasure {
    pub plane: Plane,
    pub ac: f64,
    pub bd: f64,
    pub ad: f64,
}

/// Result for one plane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlaneSkew {
    pub plane: Plane,
    pub ac: f64,
    pub bd: f64,
    pub ad: f64,
    /// Klipper's skew factor (what `[skew_correction foreman] xy_skew` stores).
    pub factor: f64,
    /// `atan(factor)` in degrees; negative = the A corner is under 90°.
    pub degrees: f64,
    /// Offset over 100 mm of travel, mm (`factor × 100`).
    pub mm_per_100: f64,
}

/// Klipper's `calc_skew_factor(ac, bd, ad)`, verbatim:
///
/// ```python
/// side = math.sqrt(2*ac*ac + 2*bd*bd - 4*ad*ad) / 2.
/// return math.tan(math.pi/2 - math.acos(
///     (ac*ac - side*side - ad*ad) / (2*side*ad)))
/// ```
///
/// Klipper would raise a Python `ValueError` on impossible numbers; here they
/// are an `InvalidInput` naming the problem.
pub fn skew_factor(ac: f64, bd: f64, ad: f64) -> Result<f64, AppError> {
    for (k, v) in [("AC", ac), ("BD", bd), ("AD", ad)] {
        if !v.is_finite() || v <= 0.0 {
            return Err(AppError::InvalidInput(format!(
                "{k} must be a positive length in mm"
            )));
        }
    }
    let sq = 2.0 * ac * ac + 2.0 * bd * bd - 4.0 * ad * ad;
    if sq <= 0.0 {
        return Err(AppError::InvalidInput(
            "AD is too long for these diagonals — check which side you measured".into(),
        ));
    }
    let side = sq.sqrt() / 2.0;
    let cos = (ac * ac - side * side - ad * ad) / (2.0 * side * ad);
    if !(-1.0..=1.0).contains(&cos) {
        return Err(AppError::InvalidInput(
            "these lengths can't form the test piece — re-measure AC, BD and AD".into(),
        ));
    }
    let f = (std::f64::consts::FRAC_PI_2 - cos.acos()).tan();
    if !f.is_finite() {
        return Err(AppError::InvalidInput("skew is out of range".into()));
    }
    Ok(f)
}

/// Skew of one plane.
pub fn plane_skew(m: &PlaneMeasure) -> Result<PlaneSkew, AppError> {
    let factor = skew_factor(m.ac, m.bd, m.ad)
        .map_err(|e| AppError::InvalidInput(format!("{}: {}", m.plane.as_str(), msg(&e))))?;
    Ok(PlaneSkew {
        plane: m.plane,
        ac: m.ac,
        bd: m.bd,
        ad: m.ad,
        factor,
        degrees: factor.atan().to_degrees(),
        mm_per_100: factor * 100.0,
    })
}

fn msg(e: &AppError) -> String {
    match e {
        AppError::InvalidInput(m) => m.clone(),
        e => e.to_string(),
    }
}

/// Skew of every given plane. XY is required; each plane at most once.
pub fn compute_skew(planes: &[PlaneMeasure]) -> Result<Vec<PlaneSkew>, AppError> {
    if !planes.iter().any(|p| p.plane == Plane::XY) {
        return Err(AppError::InvalidInput("XY is required".into()));
    }
    for (i, p) in planes.iter().enumerate() {
        if planes[..i].iter().any(|q| q.plane == p.plane) {
            return Err(AppError::InvalidInput(format!(
                "{} given twice",
                p.plane.as_str()
            )));
        }
    }
    let mut out = planes
        .iter()
        .map(plane_skew)
        .collect::<Result<Vec<_>, _>>()?;
    out.sort_by_key(|p| p.plane as u8);
    Ok(out)
}

/// A length as Klipper's SET_SKEW takes it: up to 3 decimals, no trailing zeros.
fn len(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_owned()
}

/// The `SET_SKEW` line for these measurements.
pub fn set_skew_line(planes: &[PlaneSkew]) -> String {
    let mut s = String::from("SET_SKEW");
    for p in planes {
        s.push_str(&format!(
            " {}={},{},{}",
            p.plane.as_str(),
            len(p.ac),
            len(p.bd),
            len(p.ad)
        ));
    }
    s
}

/// The exact commands "Apply" sends, in order. The last one, SAVE_CONFIG,
/// writes printer.cfg and restarts Klipper.
pub fn apply_commands(planes: &[PlaneSkew]) -> Vec<String> {
    vec![
        set_skew_line(planes),
        format!("SKEW_PROFILE SAVE={SKEW_PROFILE}"),
        "SAVE_CONFIG".to_owned(),
    ]
}

/// Sent before printing the test piece so the old correction doesn't hide the error.
pub const CLEAR_COMMAND: &str = "SET_SKEW CLEAR=1";

// ---------------------------------------------------------------------------
// Shrinkage (Calistar / fleur_de_cali port)
// ---------------------------------------------------------------------------

/// One measured span: its nominal length and the caliper readings (mm).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Span {
    pub nominal: f64,
    pub readings: Vec<f64>,
}

/// Mean and sample standard deviation of the positive readings (Calistar `computeRowStats`).
pub fn row_stats(readings: &[f64]) -> Option<(f64, f64)> {
    let v: Vec<f64> = readings
        .iter()
        .copied()
        .filter(|r| r.is_finite() && *r > 0.0)
        .collect();
    if v.is_empty() {
        return None;
    }
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    let sigma = if v.len() > 1 {
        (v.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
    } else {
        0.0
    };
    Some((mean, sigma))
}

/// Dimensional error of one axis (Calistar `calcDimensionality`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AxisError {
    /// Mean relative error `(measured − nominal) / nominal`; negative = shrank.
    pub err: f64,
    /// Propagated 1σ of `err`.
    pub sigma: f64,
    /// Spans used.
    pub n: u32,
}

/// `cal_err` is the caliper's own uncertainty (Calistar default 0.01 mm), added
/// in quadrature to each span's scatter.
pub fn axis_error(spans: &[Span], cal_err: f64) -> Option<AxisError> {
    let (mut sum, mut sumsq, mut n) = (0.0, 0.0, 0u32);
    for s in spans {
        let Some((mean, sigma)) = row_stats(&s.readings) else {
            continue;
        };
        if !s.nominal.is_finite() || s.nominal <= 0.0 {
            continue;
        }
        sum += (mean - s.nominal) / s.nominal;
        sumsq += (sigma.powi(2) + cal_err.powi(2)) / s.nominal.powi(2);
        n += 1;
    }
    if n == 0 {
        return None;
    }
    let nf = f64::from(n);
    Some(AxisError {
        err: sum / nf,
        sigma: (sumsq / (nf * nf)).sqrt(),
        n,
    })
}

/// OrcaSlicer shrinkage % for a relative error: `100 / (1 + err)`.
/// A part that came out 0.5 % small needs 100.503 %.
pub fn shrinkage_pct(err: f64) -> f64 {
    100.0 / (1.0 + err)
}

/// Calistar nominal spans for a star of width `n` mm with `m` measuring points,
/// printed at `scale`: `max((N − N·i/M)·S, 0)` for i in 0..M.
pub fn calistar_nominals(n: f64, m: u32, scale: f64) -> Vec<f64> {
    (0..m)
        .map(|i| ((n - n * f64::from(i) / f64::from(m)) * scale).max(0.0))
        .collect()
}

/// Measurements for a filament's shrinkage. X and Y feed `shrinkage_xy`, Z feeds
/// `shrinkage_z`. Each axis may have several spans (Calistar) or one (a cube).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ShrinkageInput {
    #[serde(default)]
    pub x: Vec<Span>,
    #[serde(default)]
    pub y: Vec<Span>,
    #[serde(default)]
    pub z: Vec<Span>,
    /// Caliper uncertainty, mm. Default 0.01.
    #[serde(default)]
    pub cal_err: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ShrinkageResult {
    pub x: Option<AxisError>,
    pub y: Option<AxisError>,
    pub z: Option<AxisError>,
    /// Per-axis correction % (what each axis alone would need).
    pub x_pct: Option<f64>,
    pub y_pct: Option<f64>,
    pub z_pct: Option<f64>,
    /// OrcaSlicer `shrinkage_xy`: from the mean of the X and Y errors, 3 decimals.
    pub shrinkage_xy: Option<f64>,
    /// OrcaSlicer `shrinkage_z`, 3 decimals.
    pub shrinkage_z: Option<f64>,
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Plausible range for a stored shrinkage %. Outside it the measurement is wrong.
pub const SHRINKAGE_RANGE: (f64, f64) = (90.0, 110.0);

pub fn compute_shrinkage(input: &ShrinkageInput) -> Result<ShrinkageResult, AppError> {
    let ce = input
        .cal_err
        .filter(|c| c.is_finite() && *c >= 0.0)
        .unwrap_or(0.01);
    let x = axis_error(&input.x, ce);
    let y = axis_error(&input.y, ce);
    let z = axis_error(&input.z, ce);
    if x.is_none() && y.is_none() && z.is_none() {
        return Err(AppError::InvalidInput(
            "enter at least one measured length".into(),
        ));
    }
    // Calistar: shrinkage_xy = 100 / (1 + mean of the available X/Y errors).
    let xy: Vec<f64> = [x, y].iter().flatten().map(|a| a.err).collect();
    let shrinkage_xy =
        (!xy.is_empty()).then(|| round3(shrinkage_pct(xy.iter().sum::<f64>() / xy.len() as f64)));
    let shrinkage_z = z.map(|a| round3(shrinkage_pct(a.err)));
    for v in [shrinkage_xy, shrinkage_z].into_iter().flatten() {
        if !(SHRINKAGE_RANGE.0..=SHRINKAGE_RANGE.1).contains(&v) {
            return Err(AppError::InvalidInput(format!(
                "{v:.3} % is outside {:.0}–{:.0} % — check the measurements",
                SHRINKAGE_RANGE.0, SHRINKAGE_RANGE.1
            )));
        }
    }
    Ok(ShrinkageResult {
        x_pct: x.map(|a| round3(shrinkage_pct(a.err))),
        y_pct: y.map(|a| round3(shrinkage_pct(a.err))),
        z_pct: z.map(|a| round3(shrinkage_pct(a.err))),
        x,
        y,
        z,
        shrinkage_xy,
        shrinkage_z,
    })
}

/// Merge shrinkage into the text of an `orcaslicer_config` value (a JSON object
/// as a string, the same shape Marathon wrote). Other keys are kept; a value
/// that isn't a JSON object is replaced (returned `true` = it was replaced).
pub fn merge_orca_config(
    existing: Option<&str>,
    shrinkage_xy: Option<f64>,
    shrinkage_z: Option<f64>,
) -> (String, bool) {
    let mut replaced = false;
    let mut obj = match existing.map(str::trim).filter(|s| !s.is_empty()) {
        None => serde_json::Map::new(),
        Some(s) => match serde_json::from_str::<serde_json::Value>(s) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => {
                replaced = true;
                serde_json::Map::new()
            }
        },
    };
    // Strings, like the values already in these profiles ("98.85"): Orca's own
    // profile format keeps numbers as strings.
    if let Some(v) = shrinkage_xy {
        obj.insert("shrinkage_xy".into(), serde_json::json!(orca_num(v)));
    }
    if let Some(v) = shrinkage_z {
        obj.insert("shrinkage_z".into(), serde_json::json!(orca_num(v)));
    }
    (serde_json::Value::Object(obj).to_string(), replaced)
}

/// `100.402`, `100.2`, `100` — up to 3 decimals, no trailing zeros.
pub fn orca_num(v: f64) -> String {
    len(v)
}

/// Read `shrinkage_xy` / `shrinkage_z` from an `orcaslicer_config` text.
pub fn read_orca_shrinkage(text: Option<&str>) -> (Option<f64>, Option<f64>) {
    let Some(v) = text.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()) else {
        return (None, None);
    };
    let num = |k: &str| match v.get(k) {
        Some(serde_json::Value::Number(n)) => n.as_f64(),
        Some(serde_json::Value::String(s)) => s.trim().trim_end_matches('%').parse().ok(),
        _ => None,
    };
    (num("shrinkage_xy"), num("shrinkage_z"))
}

// ---------------------------------------------------------------------------
// Klipper config patch (PRINT_START / PRINT_END / [skew_correction])
// ---------------------------------------------------------------------------

pub const LOAD_LINE: &str = "SKEW_PROFILE LOAD=foreman";
const START_NAMES: [&str; 2] = ["PRINT_START", "START_PRINT"];
const END_NAMES: [&str; 2] = ["PRINT_END", "END_PRINT"];
const AUTOSAVE_MARK: &str = "#*#";

/// One config file as read from the printer.
#[derive(Debug, Clone)]
pub struct CfgFile {
    pub path: String,
    pub content: String,
}

/// What the patch would change.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CfgPatch {
    /// (path, new content) for each file that changes.
    pub changed: Vec<(String, String)>,
    /// Human-readable notes, one per decision.
    pub notes: Vec<String>,
    /// `[skew_correction]` was missing and is added (Klipper must restart before SET_SKEW works).
    pub adds_section: bool,
}

fn section_name(line: &str) -> Option<String> {
    let t = line.trim_end();
    if !t.starts_with('[') {
        return None;
    }
    let end = t.find(']')?;
    Some(t[1..end].trim().to_owned())
}

fn macro_name(section: &str) -> Option<String> {
    let mut it = section.split_whitespace();
    if !it.next()?.eq_ignore_ascii_case("gcode_macro") {
        return None;
    }
    Some(it.next()?.to_ascii_uppercase())
}

/// Line range of the `gcode:` body of `[gcode_macro <name>]`: (gcode_line, body lines).
/// Body = following lines that are indented, blank, or column-0 comments,
/// trimmed of trailing blanks/comments.
fn find_macro_body(lines: &[&str], names: &[&str]) -> Option<(String, usize, Vec<usize>)> {
    let mut i = 0;
    while i < lines.len() {
        let Some(sec) = section_name(lines[i]) else {
            i += 1;
            continue;
        };
        let Some(name) = macro_name(&sec).filter(|n| names.contains(&n.as_str())) else {
            i += 1;
            continue;
        };
        // Find `gcode:` inside this section.
        let mut j = i + 1;
        while j < lines.len() && section_name(lines[j]).is_none() {
            let l = lines[j];
            if !l.starts_with([' ', '\t'])
                && l.split(':')
                    .next()
                    .is_some_and(|k| k.trim().eq_ignore_ascii_case("gcode"))
                && l.contains(':')
            {
                let mut body = vec![];
                let mut k = j + 1;
                while k < lines.len() {
                    let l = lines[k];
                    if l.starts_with(AUTOSAVE_MARK) || section_name(l).is_some() {
                        break;
                    }
                    let is_body = l.trim().is_empty()
                        || l.starts_with([' ', '\t'])
                        || l.starts_with('#')
                        || l.starts_with(';');
                    if !is_body {
                        break;
                    }
                    body.push(k);
                    k += 1;
                }
                while body.last().is_some_and(|&b| {
                    !lines[b].starts_with([' ', '\t']) || lines[b].trim().is_empty()
                }) {
                    body.pop();
                }
                return Some((name, j, body));
            }
            j += 1;
        }
        i += 1;
    }
    None
}

fn indent_of(lines: &[&str], body: &[usize]) -> String {
    body.iter()
        .map(|&b| lines[b])
        .find(|l| !l.trim().is_empty())
        .map(|l| l[..l.len() - l.trim_start().len()].to_owned())
        .unwrap_or_else(|| "  ".to_owned())
}

fn body_has(lines: &[&str], body: &[usize], needle: &str) -> bool {
    body.iter().any(|&b| {
        let l = lines[b].trim();
        !l.starts_with('#') && l.to_ascii_uppercase().contains(needle)
    })
}

fn join(lines: &[String], trailing_newline: bool) -> String {
    let mut s = lines.join("\n");
    if trailing_newline {
        s.push('\n');
    }
    s
}

/// Plan the config change. `files[0]` must be printer.cfg; the others are its
/// includes. Nothing is written here.
pub fn plan_skew_cfg_patch(files: &[CfgFile]) -> Result<CfgPatch, AppError> {
    let Some(main) = files.first() else {
        return Err(AppError::InvalidInput("printer.cfg not found".into()));
    };
    let mut patch = CfgPatch::default();
    let mut edited: Vec<(String, Vec<String>, bool)> = files
        .iter()
        .map(|f| {
            (
                f.path.clone(),
                f.content.lines().map(str::to_owned).collect(),
                f.content.ends_with('\n') || f.content.is_empty(),
            )
        })
        .collect();
    let mut dirty = vec![false; files.len()];

    // [skew_correction] anywhere (not a "skew_correction <profile>" section).
    let has_section = files.iter().any(|f| {
        f.content
            .lines()
            .filter_map(section_name)
            .any(|s| s.eq_ignore_ascii_case("skew_correction"))
    });

    // PRINT_START: append the load as the last line, i.e. after homing and any
    // purge / wipe moves near the edges (Klipper docs).
    let mut start_done = false;
    for (fi, f) in files.iter().enumerate() {
        let lines: Vec<&str> = f.content.lines().collect();
        if let Some((name, _, body)) = find_macro_body(&lines, &START_NAMES) {
            if body_has(&lines, &body, "SKEW_PROFILE LOAD") {
                patch.notes.push(format!(
                    "{name} in {} already loads a skew profile — left as is",
                    f.path
                ));
            } else {
                let at = body.last().map(|b| b + 1).ok_or_else(|| {
                    AppError::InvalidInput(format!("{name} in {} has an empty gcode body", f.path))
                })?;
                let ind = indent_of(&lines, &body);
                edited[fi].1.insert(at, format!("{ind}{LOAD_LINE}"));
                dirty[fi] = true;
                patch.notes.push(format!(
                    "{name} ({}): add `{LOAD_LINE}` as its last line, after homing and purge",
                    f.path
                ));
            }
            start_done = true;
            break;
        }
    }
    if !start_done {
        patch.notes.push(
            "No PRINT_START / START_PRINT macro found — add `SKEW_PROFILE LOAD=foreman` to your slicer's start gcode after homing".into(),
        );
    }

    // PRINT_END: clear first, so parking moves aren't corrected past the bed edges.
    let mut end_done = false;
    for (fi, f) in files.iter().enumerate() {
        // Use the current (maybe already edited) lines of this file.
        let owned = edited[fi].1.clone();
        let lines: Vec<&str> = owned.iter().map(String::as_str).collect();
        if let Some((name, gcode_line, body)) = find_macro_body(&lines, &END_NAMES) {
            if body_has(&lines, &body, "SET_SKEW CLEAR") {
                patch.notes.push(format!(
                    "{name} in {} already clears skew — left as is",
                    f.path
                ));
            } else {
                let ind = indent_of(&lines, &body);
                edited[fi]
                    .1
                    .insert(gcode_line + 1, format!("{ind}{CLEAR_COMMAND}"));
                dirty[fi] = true;
                patch.notes.push(format!(
                    "{name} ({}): add `{CLEAR_COMMAND}` as its first line",
                    f.path
                ));
            }
            end_done = true;
            break;
        }
    }
    if !end_done {
        patch.notes.push(
            "No PRINT_END / END_PRINT macro found — add `SET_SKEW CLEAR=1` to your slicer's end gcode".into(),
        );
    }

    if !has_section {
        let lines = &mut edited[0].1;
        let at = lines
            .iter()
            .position(|l| l.starts_with(AUTOSAVE_MARK))
            .unwrap_or(lines.len());
        let mut add = vec!["[skew_correction]".to_owned(), String::new()];
        if at > 0 && !lines[at - 1].trim().is_empty() {
            add.insert(0, String::new());
        }
        for (k, l) in add.into_iter().enumerate() {
            lines.insert(at + k, l);
        }
        dirty[0] = true;
        patch.adds_section = true;
        patch.notes.push(format!(
            "{}: add `[skew_correction]` (missing) — Klipper must restart before SET_SKEW works",
            main.path
        ));
    }

    for (i, (path, lines, nl)) in edited.into_iter().enumerate() {
        if dirty[i] {
            patch.changed.push((path, join(&lines, nl)));
        }
    }
    Ok(patch)
}

/// `[include …]` targets in a config file, as written.
pub fn includes(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(section_name)
        .filter_map(|s| {
            let mut it = s.splitn(2, char::is_whitespace);
            it.next()
                .filter(|k| k.eq_ignore_ascii_case("include"))
                .and(it.next())
                .map(|p| p.trim().to_owned())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Line diff (for the approval step)
// ---------------------------------------------------------------------------

/// Unified-style line diff with `context` lines around each change.
/// Lines start with `"  "`, `"- "` or `"+ "`; `"…"` separates hunks.
pub fn line_diff(old: &str, new: &str, context: usize) -> Vec<String> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    // Trim the common prefix/suffix, LCS the middle.
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf]
    {
        suf += 1;
    }
    let am = &a[pre..a.len() - suf];
    let bm = &b[pre..b.len() - suf];
    let (n, m) = (am.len(), bm.len());
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if am[i] == bm[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut ops: Vec<(char, &str)> = a[..pre].iter().map(|l| (' ', *l)).collect();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && am[i] == bm[j] {
            ops.push((' ', am[i]));
            i += 1;
            j += 1;
        } else if j < m && (i == n || dp[i][j + 1] >= dp[i + 1][j]) {
            ops.push(('+', bm[j]));
            j += 1;
        } else {
            ops.push(('-', am[i]));
            i += 1;
        }
    }
    ops.extend(a[a.len() - suf..].iter().map(|l| (' ', *l)));

    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, (c, _))| *c != ' ')
        .map(|(k, _)| k)
        .collect();
    let keep = |k: usize| {
        changed
            .iter()
            .any(|&c| k + context >= c && k <= c + context)
    };
    let mut out = vec![];
    let mut last: Option<usize> = None;
    for (k, (c, l)) in ops.iter().enumerate() {
        if !keep(k) {
            continue;
        }
        if last.is_some_and(|p| k > p + 1) {
            out.push("…".to_owned());
        }
        out.push(format!("{c} {l}"));
        last = Some(k);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    // --- skew --------------------------------------------------------------

    #[test]
    fn klipper_docs_example_is_minus_0_97_degrees() {
        // docs/Skew_Correction.md: SET_SKEW XY=140.4,142.8,99.8
        let p = plane_skew(&PlaneMeasure {
            plane: Plane::XY,
            ac: 140.4,
            bd: 142.8,
            ad: 99.8,
        })
        .unwrap();
        assert!(close(p.degrees, -0.97, 0.005), "got {}", p.degrees);
        assert_eq!(format!("{:.2}", p.degrees), "-0.97");
        // Klipper's GET_CURRENT_SKEW prints degrees(factor): same to 2 decimals.
        assert_eq!(format!("{:.2}", p.factor.to_degrees()), "-0.97");
        assert!(close(p.factor, -0.016949, 0.000005), "factor {}", p.factor);
    }

    #[test]
    fn docs_example_all_three_planes_and_commands() {
        // SET_SKEW XY=140.4,142.8,99.8 XZ=141.6,141.4,99.8 YZ=142.4,140.5,99.5
        let r = compute_skew(&[
            PlaneMeasure {
                plane: Plane::YZ,
                ac: 142.4,
                bd: 140.5,
                ad: 99.5,
            },
            PlaneMeasure {
                plane: Plane::XY,
                ac: 140.4,
                bd: 142.8,
                ad: 99.8,
            },
            PlaneMeasure {
                plane: Plane::XZ,
                ac: 141.6,
                bd: 141.4,
                ad: 99.8,
            },
        ])
        .unwrap();
        assert_eq!(
            r.iter().map(|p| p.plane).collect::<Vec<_>>(),
            vec![Plane::XY, Plane::XZ, Plane::YZ]
        );
        // AC > BD → positive, AC < BD → negative.
        assert!(r[1].degrees > 0.0 && r[2].degrees > 0.0 && r[0].degrees < 0.0);
        assert_eq!(
            apply_commands(&r),
            vec![
                "SET_SKEW XY=140.4,142.8,99.8 XZ=141.6,141.4,99.8 YZ=142.4,140.5,99.5",
                "SKEW_PROFILE SAVE=foreman",
                "SAVE_CONFIG",
            ]
        );
    }

    #[test]
    fn square_part_has_zero_skew() {
        let d = 100.0 * std::f64::consts::SQRT_2;
        let f = skew_factor(d, d, 100.0).unwrap();
        assert!(f.abs() < 1e-12);
    }

    #[test]
    fn swapping_diagonals_flips_the_sign() {
        let a = skew_factor(140.4, 142.8, 99.8).unwrap();
        let b = skew_factor(142.8, 140.4, 99.8).unwrap();
        assert!(close(a, -b, 1e-12));
    }

    #[test]
    fn bad_lengths_are_refused() {
        assert!(skew_factor(0.0, 141.0, 100.0).is_err());
        assert!(skew_factor(-140.0, 141.0, 100.0).is_err());
        assert!(skew_factor(f64::NAN, 141.0, 100.0).is_err());
        assert!(skew_factor(140.0, 141.0, f64::INFINITY).is_err());
        // AD longer than the diagonals allow.
        assert!(skew_factor(140.0, 141.0, 200.0).is_err());
        // Diagonals that can't close the shape.
        assert!(skew_factor(10.0, 300.0, 100.0).is_err());
        // XY required; no duplicates.
        let xz = PlaneMeasure {
            plane: Plane::XZ,
            ac: 141.6,
            bd: 141.4,
            ad: 99.8,
        };
        assert!(compute_skew(&[xz]).is_err());
        let xy = PlaneMeasure {
            plane: Plane::XY,
            ac: 140.4,
            bd: 142.8,
            ad: 99.8,
        };
        assert!(compute_skew(&[xy, xy]).is_err());
    }

    #[test]
    fn set_skew_formats_lengths_like_typed() {
        let r = compute_skew(&[PlaneMeasure {
            plane: Plane::XY,
            ac: 141.0,
            bd: 141.25,
            ad: 99.875,
        }])
        .unwrap();
        assert_eq!(set_skew_line(&r), "SET_SKEW XY=141,141.25,99.875");
    }

    // --- shrinkage ----------------------------------------------------------

    fn span(nominal: f64, r: &[f64]) -> Span {
        Span {
            nominal,
            readings: r.to_vec(),
        }
    }

    #[test]
    fn cube_shrinkage_per_axis() {
        let r = compute_shrinkage(&ShrinkageInput {
            x: vec![span(100.0, &[99.5])],
            y: vec![span(100.0, &[99.7])],
            z: vec![span(100.0, &[99.8])],
            cal_err: None,
        })
        .unwrap();
        assert_eq!(r.x_pct, Some(100.503)); // 100/0.995
        assert_eq!(r.y_pct, Some(100.301));
        assert_eq!(r.z_pct, Some(100.2));
        // XY from the mean error (−0.004): 100/0.996
        assert_eq!(r.shrinkage_xy, Some(100.402));
        assert_eq!(r.shrinkage_z, Some(100.2));
    }

    #[test]
    fn calistar_dimensionality_matches_the_js() {
        // 120×3 star: nominals 120, 80, 40 (getNominals).
        assert_eq!(calistar_nominals(120.0, 3, 1.0), vec![120.0, 80.0, 40.0]);
        assert_eq!(calistar_nominals(100.0, 2, 1.5), vec![150.0, 75.0]);
        // Hand-computed from calcDimensionality:
        //   rows 120:[119.4,119.6] 80:[79.6] 40:[39.8,39.8,39.8]
        //   errs  -0.004166.. , -0.005, -0.005 → mean -0.0047222
        let a = axis_error(
            &[
                span(120.0, &[119.4, 119.6]),
                span(80.0, &[79.6]),
                span(40.0, &[39.8, 39.8, 39.8]),
            ],
            0.01,
        )
        .unwrap();
        assert_eq!(a.n, 3);
        assert!(close(a.err, -0.0047222, 1e-6), "err {}", a.err);
        // σ: row1 sigma = 0.141421; (σ²+0.01²)/120², 0.01²/80², 0.01²/40² ; sqrt(sum/9)
        let want: f64 = ((0.02 + 0.0001) / 14400.0 + 0.0001 / 6400.0 + 0.0001 / 1600.0) / 9.0;
        assert!(close(a.sigma, want.sqrt(), 1e-9));
        assert!(close(shrinkage_pct(a.err), 100.4745, 1e-3));
    }

    #[test]
    fn shrinkage_edge_cases() {
        // Nothing measured.
        assert!(compute_shrinkage(&ShrinkageInput {
            x: vec![],
            y: vec![],
            z: vec![],
            cal_err: None
        })
        .is_err());
        // Zero / negative readings ignored → nothing left.
        assert!(compute_shrinkage(&ShrinkageInput {
            x: vec![span(100.0, &[0.0, -1.0])],
            y: vec![],
            z: vec![],
            cal_err: None
        })
        .is_err());
        // Implausible (typed 10 instead of 100).
        assert!(compute_shrinkage(&ShrinkageInput {
            x: vec![span(100.0, &[10.0])],
            y: vec![],
            z: vec![],
            cal_err: None
        })
        .is_err());
        // Only Z: no xy.
        let r = compute_shrinkage(&ShrinkageInput {
            x: vec![],
            y: vec![],
            z: vec![span(50.0, &[49.9])],
            cal_err: None,
        })
        .unwrap();
        assert_eq!(r.shrinkage_xy, None);
        assert_eq!(r.shrinkage_z, Some(100.2));
        // Exact part: 100 %.
        let r = compute_shrinkage(&ShrinkageInput {
            x: vec![span(100.0, &[100.0])],
            y: vec![],
            z: vec![],
            cal_err: None,
        })
        .unwrap();
        assert_eq!(r.shrinkage_xy, Some(100.0));
    }

    #[test]
    fn orca_config_merge_keeps_other_keys() {
        let (s, replaced) = merge_orca_config(
            Some(r#"{"filament_flow_ratio":0.95,"shrinkage_xy":99}"#),
            Some(100.402),
            Some(100.2),
        );
        assert!(!replaced);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["filament_flow_ratio"], 0.95);
        assert_eq!(v["shrinkage_xy"], "100.402");
        assert_eq!(v["shrinkage_z"], "100.2");
        assert_eq!(read_orca_shrinkage(Some(&s)), (Some(100.402), Some(100.2)));
        // Only z: xy untouched.
        let (s2, _) = merge_orca_config(Some(&s), None, Some(100.1));
        assert_eq!(read_orca_shrinkage(Some(&s2)), (Some(100.402), Some(100.1)));
        // Garbage → replaced.
        let (s3, replaced) = merge_orca_config(Some("not json"), Some(100.0), None);
        assert!(replaced);
        assert_eq!(read_orca_shrinkage(Some(&s3)), (Some(100.0), None));
        assert_eq!(
            read_orca_shrinkage(Some(r#"{"shrinkage_xy":"100.5%"}"#)),
            (Some(100.5), None)
        );
    }

    // --- config patch -------------------------------------------------------

    const CFG: &str = "\
[include macros.cfg]

[printer]
kinematics: corexy

#*# <---------------------- SAVE_CONFIG ---------------------->
#*# DO NOT EDIT THIS BLOCK OR BELOW. The contents are auto-generated.
#*#
#*# [extruder]
#*# pid_Kp = 1
";
    const MACROS: &str = "\
[gcode_macro PRINT_START]
description: start
gcode:
    G28
    BED_MESH_CALIBRATE
    {% if params.PURGE|default(1)|int %}
      PURGE_LINE
    {% endif %}

# end of start
[gcode_macro PRINT_END]
variable_park: 1
gcode:
    G91
    G1 E-2
    TURN_OFF_HEATERS
";

    fn files() -> Vec<CfgFile> {
        vec![
            CfgFile {
                path: "/c/printer.cfg".into(),
                content: CFG.into(),
            },
            CfgFile {
                path: "/c/macros.cfg".into(),
                content: MACROS.into(),
            },
        ]
    }

    #[test]
    fn patch_adds_load_clear_and_section() {
        assert_eq!(includes(CFG), vec!["macros.cfg"]);
        let p = plan_skew_cfg_patch(&files()).unwrap();
        assert!(p.adds_section);
        assert_eq!(p.changed.len(), 2);
        let (_, macros) = p
            .changed
            .iter()
            .find(|(f, _)| f.ends_with("macros.cfg"))
            .unwrap();
        assert!(macros.contains("    {% endif %}\n    SKEW_PROFILE LOAD=foreman\n\n# end of start"));
        assert!(macros.contains("gcode:\n    SET_SKEW CLEAR=1\n    G91"));
        let (_, printer) = p
            .changed
            .iter()
            .find(|(f, _)| f.ends_with("printer.cfg"))
            .unwrap();
        assert!(printer.contains("kinematics: corexy\n\n[skew_correction]\n\n#*# <----"));
        // Re-planning on the patched files changes nothing.
        let again: Vec<CfgFile> = files()
            .into_iter()
            .map(|f| {
                let c = p
                    .changed
                    .iter()
                    .find(|(x, _)| *x == f.path)
                    .map(|(_, c)| c.clone())
                    .unwrap_or(f.content);
                CfgFile {
                    path: f.path,
                    content: c,
                }
            })
            .collect();
        let p2 = plan_skew_cfg_patch(&again).unwrap();
        assert!(p2.changed.is_empty(), "{:?}", p2.notes);
        assert!(!p2.adds_section);
    }

    #[test]
    fn patch_without_macros_only_notes() {
        let p = plan_skew_cfg_patch(&[CfgFile {
            path: "/c/printer.cfg".into(),
            content: "[printer]\nkinematics: corexy\n[skew_correction]\n".into(),
        }])
        .unwrap();
        assert!(p.changed.is_empty());
        assert_eq!(p.notes.len(), 2);
        assert!(p.notes[0].contains("No PRINT_START"));
    }

    #[test]
    fn profile_section_is_not_the_main_section() {
        let p = plan_skew_cfg_patch(&[CfgFile {
            path: "/c/printer.cfg".into(),
            content: "[printer]\n#*# [skew_correction foreman]\n#*# xy_skew = 0.01\n".into(),
        }])
        .unwrap();
        assert!(p.adds_section);
    }

    #[test]
    fn diff_shows_inserts_with_context() {
        let d = line_diff("a\nb\nc\nd\ne\nf\ng\n", "a\nb\nc\nX\nd\ne\nf\ng\n", 1);
        assert_eq!(d, vec!["  c", "+ X", "  d"]);
        let d = line_diff(
            "1\n2\n3\n4\n5\n6\n7\n8\n",
            "0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n",
            1,
        );
        assert_eq!(d, vec!["+ 0", "  1", "…", "  8", "+ 9"]);
        assert!(line_diff("same\n", "same\n", 3).is_empty());
    }
}
