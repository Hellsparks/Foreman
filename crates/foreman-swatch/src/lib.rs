//! Filament swatch STL generator (D8): a Rust port of the old
//! `swatch_generator.py` (CadQuery).
//!
//! The plate is 75 × 40 × 3 mm with its top face at Z = 3. Two lines of text
//! are debossed 0.6 mm deep:
//!
//! | line | band     | centre | left inset | max width |
//! |------|----------|--------|------------|-----------|
//! | 1    | Y 30–38  | Y 34   | X 8.5      | 62 mm     |
//! | 2    | Y 21–29  | Y 25   | X 8.5      | 62 mm     |
//!
//! Layout matches CadQuery's `text(…, halign="left", valign="center")`
//! (OCCT `Font_TextFormatter`): the pen starts at X 8.5, advances use the
//! font's `hmtx` widths plus `kern` pairs, and the baseline sits at
//! `centre + size × (½·line_height − ascender) / em`, i.e. 2.114 mm below the
//! band centre at 6.4 mm. Liberation Sans (SIL OFL) replaces Arial; the two are
//! metric-compatible, so the layout is the same.
//!
//! Differences from the Python, on purpose:
//! - text wider than 62 mm shrinks to fit (the Python cut through the plate edge),
//! - glyph contours are filled with the non-zero rule, so overlapping contours
//!   (Ø, Å) give a clean solid (the Python produced a non-manifold mesh for them).
//!
//! The base plate is `assets/swatch_base.stl`: `swatch.step` tessellated with
//! the Python exporter's own tolerances (0.05 mm / 0.3 rad). The boolean is
//! `csgrs` (BSP CSG); the result is welded, T-junctions are split, and the mesh
//! is checked to be closed and 2-manifold before it is returned.

use std::sync::OnceLock;

use csgrs::{mesh::Mesh, sketch::Sketch, traits::CSG};
use geo::{
    orient::Direction, Geometry, GeometryCollection, LineString, MultiPolygon, Orient, Polygon,
};
use i_overlay::{core::fill_rule::FillRule, float::simplify::SimplifyShape};
use thiserror::Error;
use ttf_parser::{Face, GlyphId, OutlineBuilder};

mod mesh;

pub use mesh::{IndexedMesh, MeshReport};

// ---------------------------------------------------------------------------
// Geometry constants (from swatch_generator.py)
// ---------------------------------------------------------------------------

/// Plate size, mm (X, Y, Z).
pub const PLATE: [f64; 3] = [75.0, 40.0, 3.0];
/// Top face of the plate.
pub const SURFACE_Z: f64 = 3.0;
/// Deboss depth (Z 3 → 2.4).
pub const CUT_DEPTH: f64 = 0.6;
/// How far the cutter pokes above the top face so the boolean never meets a
/// coplanar face.
pub const OVERSHOOT: f64 = 0.2;
/// Upper band (line 1) centre, Y 30–38.
pub const UPPER_Y: f64 = 34.0;
/// Lower band (line 2) centre, Y 21–29.
pub const LOWER_Y: f64 = 25.0;
/// Both bands are 8 mm tall.
pub const BAND_HEIGHT: f64 = 8.0;
/// Left margin of both lines.
pub const TEXT_X: f64 = 8.5;
/// Widest a line may be (X 8.5 – 70.5).
pub const MAX_WIDTH: f64 = 62.0;
/// Full text size: 80 % of the band height (6.4 mm em).
pub const FONT_SIZE: f64 = BAND_HEIGHT * 0.8;
/// Smallest text the generator shrinks to. Below this, strokes are thinner
/// than two 0.4 mm lines and the text does not print legibly.
pub const MIN_FONT_SIZE: f64 = 3.0;
/// Most characters per line.
pub const MAX_CHARS: usize = 64;
/// Curve flattening tolerance, mm.
const FLATTEN_TOL: f64 = 0.01;

const FONT: &[u8] = include_bytes!("../assets/LiberationSans-Regular.ttf");
const BASE_STL: &[u8] = include_bytes!("../assets/swatch_base.stl");

#[derive(Debug, Error)]
pub enum SwatchError {
    #[error("{0}")]
    Text(String),
    #[error("swatch font: {0}")]
    Font(String),
    #[error("swatch base mesh: {0}")]
    Base(String),
    #[error("swatch mesh is not watertight: {0}")]
    NotManifold(String),
}

// ---------------------------------------------------------------------------
// Font + layout
// ---------------------------------------------------------------------------

fn face() -> Result<Face<'static>, SwatchError> {
    Face::parse(FONT, 0).map_err(|e| SwatchError::Font(e.to_string()))
}

/// How one line lays out: the size it is cut at and its extent.
#[derive(Debug, Clone, PartialEq)]
pub struct LineFit {
    /// The trimmed text.
    pub text: String,
    /// Text size (em), mm. [`FONT_SIZE`] unless shrunk.
    pub font_size: f64,
    /// True when the line was shrunk to fit [`MAX_WIDTH`].
    pub shrunk: bool,
    /// Ink extent from the left inset, mm, at `font_size`.
    pub width: f64,
    /// Ink extent at full size, mm.
    pub full_width: f64,
}

struct Glyph {
    id: GlyphId,
    /// Pen position, font units.
    x: f64,
}

/// Glyphs with pen positions (font units) and the ink's right edge.
fn shape(face: &Face, text: &str) -> Result<(Vec<Glyph>, f64), SwatchError> {
    let mut missing: Vec<char> = Vec::new();
    let mut glyphs = Vec::new();
    let mut pen = 0.0_f64;
    let mut prev: Option<GlyphId> = None;
    let mut right = 0.0_f64;
    for ch in text.chars().filter(|c| !c.is_control()) {
        let Some(id) = face.glyph_index(ch) else {
            if !missing.contains(&ch) {
                missing.push(ch);
            }
            continue;
        };
        if let Some(p) = prev {
            pen += f64::from(kerning(face, p, id));
        }
        if let Some(bb) = face.glyph_bounding_box(id) {
            right = right.max(pen + f64::from(bb.x_max));
        }
        glyphs.push(Glyph { id, x: pen });
        pen += f64::from(face.glyph_hor_advance(id).unwrap_or(0));
        prev = Some(id);
    }
    if !missing.is_empty() {
        let list: String = missing.iter().collect();
        return Err(SwatchError::Text(format!(
            "The swatch font has no glyph for \u{201c}{list}\u{201d}."
        )));
    }
    Ok((glyphs, right))
}

/// Legacy `kern` pair adjustment (what OCCT/FreeType apply), font units.
fn kerning(face: &Face, left: GlyphId, right: GlyphId) -> i16 {
    let Some(kern) = face.tables().kern else {
        return 0;
    };
    kern.subtables
        .into_iter()
        .filter(|s| s.horizontal && !s.variable)
        .find_map(|s| s.glyphs_kerning(left, right))
        .unwrap_or(0)
}

/// Size and extent of one line, shrinking it to fit [`MAX_WIDTH`].
pub fn fit_line(text: &str) -> Result<LineFit, SwatchError> {
    let text = text.trim().to_owned();
    if text.chars().count() > MAX_CHARS {
        return Err(SwatchError::Text(format!(
            "A swatch line can have at most {MAX_CHARS} characters."
        )));
    }
    let face = face()?;
    let em = f64::from(face.units_per_em());
    let (_, right) = shape(&face, &text)?;
    let full_width = right * FONT_SIZE / em;
    let (font_size, shrunk) = if full_width > MAX_WIDTH {
        (FONT_SIZE * MAX_WIDTH / full_width, true)
    } else {
        (FONT_SIZE, false)
    };
    if font_size < MIN_FONT_SIZE {
        return Err(SwatchError::Text(format!(
            "\u{201c}{text}\u{201d} is too long for a swatch line: it would need {font_size:.1} mm text (minimum {MIN_FONT_SIZE} mm). Shorten it."
        )));
    }
    Ok(LineFit {
        width: right * font_size / em,
        text,
        font_size,
        shrunk,
        full_width,
    })
}

/// Flattens glyph outlines into closed contours (mm, plate coordinates).
struct Flattener {
    scale: f64,
    dx: f64,
    dy: f64,
    contours: Vec<Vec<[f64; 2]>>,
    cur: Vec<[f64; 2]>,
    last: [f64; 2],
}

impl Flattener {
    fn pt(&self, x: f32, y: f32) -> [f64; 2] {
        [
            self.dx + f64::from(x) * self.scale,
            self.dy + f64::from(y) * self.scale,
        ]
    }
    fn push(&mut self, p: [f64; 2]) {
        self.cur.push(p);
        self.last = p;
    }
    fn close_cur(&mut self) {
        let mut c = std::mem::take(&mut self.cur);
        if c.len() > 1 && c.first() == c.last() {
            c.pop();
        }
        if c.len() >= 3 {
            self.contours.push(c);
        }
    }
    /// Segments for a curve whose control polygon deviates by `dev` mm.
    fn steps(dev: f64) -> usize {
        // Flattening error of a quadratic with n segments ≈ dev / (4 n²).
        ((dev / (4.0 * FLATTEN_TOL)).sqrt().ceil() as usize).clamp(1, 64)
    }
}

fn dist_to_chord(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
    let len = (vx * vx + vy * vy).sqrt();
    if len < 1e-12 {
        return ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt();
    }
    ((p[0] - a[0]) * vy - (p[1] - a[1]) * vx).abs() / len
}

impl OutlineBuilder for Flattener {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close_cur();
        let p = self.pt(x, y);
        self.push(p);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.pt(x, y);
        self.push(p);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (a, c, b) = (self.last, self.pt(x1, y1), self.pt(x, y));
        let n = Self::steps(dist_to_chord(c, a, b));
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            self.push([
                u * u * a[0] + 2.0 * u * t * c[0] + t * t * b[0],
                u * u * a[1] + 2.0 * u * t * c[1] + t * t * b[1],
            ]);
        }
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (a, c1, c2, b) = (self.last, self.pt(x1, y1), self.pt(x2, y2), self.pt(x, y));
        let dev = dist_to_chord(c1, a, b).max(dist_to_chord(c2, a, b));
        let n = Self::steps(dev * 1.5);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.push([
                w0 * a[0] + w1 * c1[0] + w2 * c2[0] + w3 * b[0],
                w0 * a[1] + w1 * c1[1] + w2 * c2[1] + w3 * b[1],
            ]);
        }
    }
    fn close(&mut self) {
        self.close_cur();
    }
}

/// The 2D outline of one line, placed in its band (plate coordinates, mm).
/// Overlapping contours are merged with the non-zero fill rule.
pub fn line_outline(fit: &LineFit, y_center: f64) -> Result<MultiPolygon<f64>, SwatchError> {
    let face = face()?;
    let em = f64::from(face.units_per_em());
    let scale = fit.font_size / em;
    let (glyphs, _) = shape(&face, &fit.text)?;
    // OCCT Font_TextFormatter, VTA_CENTER: the line box (ascender − descender
    // + line gap) is centred on the origin; the baseline sits `ascender` below
    // its top.
    let line_height =
        f64::from(face.ascender()) - f64::from(face.descender()) + f64::from(face.line_gap());
    let baseline = y_center + (0.5 * line_height - f64::from(face.ascender())) * scale;
    let mut contours: Vec<Vec<[f64; 2]>> = Vec::new();
    for g in glyphs {
        let mut f = Flattener {
            scale,
            dx: TEXT_X + g.x * scale,
            dy: baseline,
            contours: Vec::new(),
            cur: Vec::new(),
            last: [0.0, 0.0],
        };
        face.outline_glyph(g.id, &mut f);
        f.close_cur();
        contours.extend(f.contours);
    }
    let shapes = contours.simplify_shape(FillRule::NonZero, 1e-6);
    let polys = shapes
        .into_iter()
        .filter_map(|shape| {
            let mut rings = shape
                .into_iter()
                .map(|c| LineString::from(c.into_iter().map(|p| (p[0], p[1])).collect::<Vec<_>>()));
            let outer = rings.next()?;
            Some(Polygon::new(outer, rings.collect()).orient(Direction::Default))
        })
        .collect::<Vec<_>>();
    Ok(MultiPolygon(polys))
}

// ---------------------------------------------------------------------------
// Mesh generation
// ---------------------------------------------------------------------------

fn base_mesh() -> Result<&'static Mesh<()>, SwatchError> {
    static BASE: OnceLock<Result<Mesh<()>, String>> = OnceLock::new();
    BASE.get_or_init(|| Mesh::from_stl(BASE_STL, None).map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| SwatchError::Base(e.clone()))
}

/// The two lines of a swatch.
#[derive(Debug, Clone, Default)]
pub struct SwatchText {
    /// Upper band: brand + material.
    pub line1: String,
    /// Lower band: colour name.
    pub line2: String,
}

/// A generated swatch.
#[derive(Debug, Clone)]
pub struct Swatch {
    /// Binary STL.
    pub stl: Vec<u8>,
    pub line1: Option<LineFit>,
    pub line2: Option<LineFit>,
    pub report: MeshReport,
}

/// Fits both lines without building a mesh (for the preview).
pub fn fit(text: &SwatchText) -> Result<(Option<LineFit>, Option<LineFit>), SwatchError> {
    let one = |s: &str| -> Result<Option<LineFit>, SwatchError> {
        if s.trim().is_empty() {
            Ok(None)
        } else {
            fit_line(s).map(Some)
        }
    };
    Ok((one(&text.line1)?, one(&text.line2)?))
}

/// Builds the swatch: base plate minus the debossed text.
pub fn generate(text: &SwatchText) -> Result<Swatch, SwatchError> {
    let (line1, line2) = fit(text)?;
    let mut polys = Vec::new();
    for (fit, y) in [(&line1, UPPER_Y), (&line2, LOWER_Y)] {
        if let Some(f) = fit {
            polys.extend(line_outline(f, y)?.0);
        }
    }
    let base = base_mesh()?;
    let solid = if polys.is_empty() {
        base.clone()
    } else {
        let geometry: GeometryCollection<f64> =
            GeometryCollection(polys.into_iter().map(Geometry::Polygon).collect());
        let cutter = Sketch::<()>::from_geo(geometry, None)
            .extrude(CUT_DEPTH + OVERSHOOT)
            .translate(0.0, 0.0, SURFACE_Z - CUT_DEPTH);
        base.difference(&cutter)
    };
    let indexed = IndexedMesh::from_csg(&solid);
    let report = indexed.check();
    if !report.is_watertight() {
        return Err(SwatchError::NotManifold(report.to_string()));
    }
    Ok(Swatch {
        stl: indexed.to_binary_stl(),
        line1,
        line2,
        report,
    })
}

#[cfg(test)]
mod tests;
