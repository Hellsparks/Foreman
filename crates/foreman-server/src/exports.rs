//! Exports + swatches (task 5.5, W11): reads Spoolman, hands the data to the
//! pure generators in `foreman_core::exports` and `foreman_swatch`.

use foreman_core::{
    error::AppError,
    exports::{self as ex, ExportFilament, HueForgeSkipped, OrcaProfileInfo},
    materials,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    calibration::orca_text,
    filaments::{self, FilamentRow},
    spools::{self, Spoolman},
};

/// Every filament (or the ids asked for, in that order) flattened for export.
pub async fn export_filaments(
    sm: &Spoolman,
    ids: Option<&[i64]>,
) -> Result<Vec<(FilamentRow, ExportFilament)>, AppError> {
    let rows = filaments::filaments(sm).await?;
    let raw = filaments::all_filaments(sm).await?;
    let spools = spools::all(sm, false).await?;
    let one = |f: &FilamentRow| {
        let r = raw.iter().find(|r| r.id == f.id);
        let owned = spools.iter().any(|s| {
            s.filament_id == f.id && !s.archived && s.remaining_weight.is_none_or(|w| w > 0.0)
        });
        ExportFilament {
            id: f.id,
            name: f.name.clone(),
            vendor: f.vendor.clone(),
            material: f.material.clone(),
            modifiers: f.modifiers.clone(),
            colours: f.colours.clone(),
            density: f.density,
            diameter: f.diameter,
            extruder_temp: f.extruder_temp,
            bed_temp: f.bed_temp,
            price: f.price,
            weight: f.weight,
            orca_config: r.and_then(|r| orca_text(&r.extra)),
            hue_td: f.hue_td,
            owned,
        }
    };
    match ids {
        None => Ok(rows.iter().map(|f| (f.clone(), one(f))).collect()),
        Some(ids) => {
            let mut out = Vec::with_capacity(ids.len());
            for id in ids {
                let f = rows
                    .iter()
                    .find(|f| f.id == *id)
                    .ok_or_else(|| AppError::NotFound(format!("filament {id}")))?;
                out.push((f.clone(), one(f)));
            }
            if out.is_empty() {
                return Err(AppError::InvalidInput("Pick at least one filament.".into()));
            }
            Ok(out)
        }
    }
}

/// Body of the export / swatch-batch requests: `null` / missing = all.
#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct ExportRequest {
    #[serde(default)]
    #[ts(type = "number[] | null")]
    pub filament_ids: Option<Vec<i64>>,
}

/// One row of the Export tab: the filament and what each export does with it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ExportRow {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub vendor: Option<String>,
    pub material: Option<String>,
    pub colours: Vec<String>,
    pub hue_td: Option<f64>,
    pub owned: bool,
    pub orca: OrcaProfileInfo,
    /// Why HueForge leaves it out; `None` = included.
    pub hueforge_skip: Option<foreman_core::exports::HueForgeSkip>,
    /// Default swatch text.
    pub swatch_line1: String,
    pub swatch_line2: String,
}

/// Default swatch lines, as Marathon made them: "Vendor Material" / name.
pub fn swatch_lines(f: &FilamentRow) -> (String, String) {
    let mut l1: Vec<String> = Vec::new();
    if let Some(v) = f.vendor.as_deref().filter(|v| !v.trim().is_empty()) {
        l1.push(v.trim().to_owned());
    }
    if let Some(m) = f.material.as_deref().filter(|m| !m.trim().is_empty()) {
        l1.push(m.trim().to_owned());
    }
    let mods = materials::join_modifiers(&f.modifiers);
    if !mods.is_empty() && !l1.join(" ").contains(&mods) {
        l1.push(mods);
    }
    (l1.join(" "), f.name.trim().to_owned())
}

pub async fn rows(sm: &Spoolman) -> Result<Vec<ExportRow>, AppError> {
    let all = export_filaments(sm, None).await?;
    let flat: Vec<ExportFilament> = all.iter().map(|(_, e)| e.clone()).collect();
    let (_, skipped) = ex::hueforge_catalogue(&flat);
    Ok(all
        .into_iter()
        .map(|(f, e)| {
            let (swatch_line1, swatch_line2) = swatch_lines(&f);
            ExportRow {
                id: f.id,
                name: f.name,
                vendor: f.vendor,
                material: f.material,
                colours: f.colours,
                hue_td: f.hue_td,
                owned: e.owned,
                orca: ex::orca_profile(&e).1,
                hueforge_skip: skipped
                    .iter()
                    .find(|s| s.filament_id == e.id)
                    .map(|s| s.reason),
                swatch_line1,
                swatch_line2,
            }
        })
        .collect())
}

pub async fn orca_zip(
    sm: &Spoolman,
    ids: Option<&[i64]>,
) -> Result<(Vec<u8>, Vec<OrcaProfileInfo>), AppError> {
    let fs: Vec<ExportFilament> = export_filaments(sm, ids)
        .await?
        .into_iter()
        .map(|(_, e)| e)
        .collect();
    ex::orca_zip(&fs)
}

pub async fn hueforge(
    sm: &Spoolman,
    ids: Option<&[i64]>,
) -> Result<(serde_json::Value, Vec<HueForgeSkipped>), AppError> {
    let fs: Vec<ExportFilament> = export_filaments(sm, ids)
        .await?
        .into_iter()
        .map(|(_, e)| e)
        .collect();
    Ok(ex::hueforge_catalogue(&fs))
}

// ---------------------------------------------------------------------------
// Swatches
// ---------------------------------------------------------------------------

/// The two swatch lines.
#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct SwatchLines {
    #[serde(default)]
    pub line1: String,
    #[serde(default)]
    pub line2: String,
}

/// How one line fits.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SwatchLineFit {
    pub text: String,
    /// Text size, mm (6.4 at full size).
    pub font_size: f64,
    pub shrunk: bool,
    /// mm from the left inset.
    pub width: f64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SwatchFit {
    pub line1: Option<SwatchLineFit>,
    pub line2: Option<SwatchLineFit>,
    /// Plain-language error (too long, missing glyph); the STL can't be made.
    pub error: Option<String>,
    pub max_width: f64,
    pub full_size: f64,
}

fn fit_dto(f: foreman_swatch::LineFit) -> SwatchLineFit {
    SwatchLineFit {
        text: f.text,
        font_size: (f.font_size * 100.0).round() / 100.0,
        shrunk: f.shrunk,
        width: (f.width * 100.0).round() / 100.0,
    }
}

fn swatch_text(l: &SwatchLines) -> foreman_swatch::SwatchText {
    foreman_swatch::SwatchText {
        line1: l.line1.clone(),
        line2: l.line2.clone(),
    }
}

pub fn swatch_fit(l: &SwatchLines) -> SwatchFit {
    let (line1, line2, error) = match foreman_swatch::fit(&swatch_text(l)) {
        Ok((a, b)) => (a.map(fit_dto), b.map(fit_dto), None),
        Err(e) => (None, None, Some(e.to_string())),
    };
    SwatchFit {
        line1,
        line2,
        error,
        max_width: foreman_swatch::MAX_WIDTH,
        full_size: foreman_swatch::FONT_SIZE,
    }
}

fn swatch_err(e: foreman_swatch::SwatchError) -> AppError {
    match e {
        foreman_swatch::SwatchError::Text(m) => AppError::InvalidInput(m),
        e => AppError::Internal(e.to_string()),
    }
}

/// Binary STL. CPU-bound: runs on the blocking pool.
pub async fn swatch_stl(l: SwatchLines) -> Result<Vec<u8>, AppError> {
    tokio::task::spawn_blocking(move || foreman_swatch::generate(&swatch_text(&l)))
        .await
        .map_err(|e| AppError::Internal(format!("swatch task: {e}")))?
        .map(|s| s.stl)
        .map_err(swatch_err)
}

/// "Polymaker PLA Teal" → "polymaker-pla-teal".
pub fn file_slug(s: &str) -> String {
    let slug: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let parts: Vec<&str> = slug.split('-').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        "swatch".into()
    } else {
        parts.join("-")
    }
}

/// A zip of swatch STLs, one per filament, with the default lines.
pub async fn swatch_zip(sm: &Spoolman, ids: &[i64]) -> Result<Vec<u8>, AppError> {
    let rows = export_filaments(sm, Some(ids)).await?;
    let jobs: Vec<(String, SwatchLines)> = rows
        .iter()
        .map(|(f, _)| {
            let (line1, line2) = swatch_lines(f);
            (
                format!(
                    "swatch-{}-{}.stl",
                    f.id,
                    file_slug(&format!("{line1} {line2}"))
                ),
                SwatchLines { line1, line2 },
            )
        })
        .collect();
    tokio::task::spawn_blocking(move || -> Result<Vec<u8>, AppError> {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, l) in jobs {
                let s = foreman_swatch::generate(&swatch_text(&l))
                    .map_err(|e| AppError::InvalidInput(format!("{name}: {}", swatch_err(e))))?;
                zip.start_file(name.as_str(), opts)
                    .map_err(|e| AppError::Internal(format!("zip: {e}")))?;
                zip.write_all(&s.stl)
                    .map_err(|e| AppError::Internal(format!("zip: {e}")))?;
            }
            zip.finish()
                .map_err(|e| AppError::Internal(format!("zip: {e}")))?;
        }
        Ok(buf.into_inner())
    })
    .await
    .map_err(|e| AppError::Internal(format!("swatch task: {e}")))?
}
