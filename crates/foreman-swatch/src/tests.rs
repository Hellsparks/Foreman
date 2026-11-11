//! Acceptance tests against the Python (CadQuery 2.8 / OCCT) output.
//!
//! Oracle numbers were measured from `swatch_generator.py`'s own code path
//! (`swatch.step`, `text(…, 6.4, -0.6, halign="left", valign="center")`,
//! STL at tolerance 0.05 / 0.3) run with Liberation Sans, session 33. The ink
//! box is the extent of the debossed side walls in each band.

use geo::BoundingRect;

use super::*;

struct Fixture {
    line1: &'static str,
    line2: &'static str,
    /// (x min, x max, y min, y max) of the cut, upper band then lower band.
    upper: [f64; 4],
    lower: [f64; 4],
    /// Python volume, mm³; `None` where the Python output is wrong.
    volume: Option<f64>,
}

const FIXTURES: &[Fixture] = &[
    // short
    Fixture {
        line1: "PLA",
        line2: "Red",
        upper: [9.025, 20.584, 31.886, 36.289],
        lower: [9.025, 19.828, 22.823, 27.523],
        volume: Some(6824.223),
    },
    // long, fits
    Fixture {
        line1: "PRUSAMENT PETG",
        line2: "Galaxy Black",
        upper: [9.025, 67.291, 31.823, 36.355],
        lower: [8.822, 45.856, 21.558, 27.523],
        volume: Some(6764.255),
    },
    // descenders
    Fixture {
        line1: "gypsy jpg",
        line2: "quality yqpg",
        upper: [8.769, 35.125, 30.558, 36.523],
        lower: [8.769, 42.244, 21.558, 27.523],
        volume: Some(6789.272),
    },
    // Æ/Ø/Å — the Python mesh is non-manifold (1462 bad edges, volume 5287),
    // so only its layout is a reference.
    Fixture {
        line1: "ÆØÅ æøå",
        line2: "Blåbær Grønn",
        upper: [8.575, 39.084, 31.72, 37.47],
        lower: [9.025, 48.631, 22.767, 28.045],
        volume: None,
    },
    // digits + symbols
    Fixture {
        line1: "0123456789",
        line2: "PLA+ 2024 #42",
        upper: [8.75, 43.791, 31.823, 36.355],
        lower: [9.025, 52.484, 22.823, 27.355],
        volume: Some(6781.005),
    },
];

/// Base plate volume (`swatch.step`); the 27 MB `swatch_base.stl` of the old
/// app measures 6844.891 (0.009 % apart, finer tessellation of the fillets).
const BASE_VOLUME: f64 = 6844.271;

fn ink(line: &str, y: f64) -> [f64; 4] {
    let fit = fit_line(line).expect("fits");
    let rect = line_outline(&fit, y)
        .expect("outline")
        .bounding_rect()
        .expect("non-empty");
    [rect.min().x, rect.max().x, rect.min().y, rect.max().y]
}

fn close(a: [f64; 4], b: [f64; 4], tol: f64) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

#[test]
fn base_plate_matches_step() {
    let m = IndexedMesh::from_binary_stl(BASE_STL).expect("base stl");
    let r = m.check();
    assert!(r.is_watertight(), "{r}");
    assert!((r.volume - BASE_VOLUME).abs() < 0.01, "{}", r.volume);
    assert!(r.min.iter().all(|v| v.abs() < 1e-6), "{:?}", r.min);
    assert!(close(
        [r.max[0], r.max[1], r.max[2], 0.0],
        [PLATE[0], PLATE[1], PLATE[2], 0.0],
        1e-4
    ));
}

#[test]
fn layout_matches_python_bands() {
    for f in FIXTURES {
        let up = ink(f.line1, UPPER_Y);
        let lo = ink(f.line2, LOWER_Y);
        // 0.02 mm: flattening of curves (OCCT 0.05 mm vs ours 0.01 mm).
        assert!(
            close(up, f.upper, 0.02),
            "{}: {up:?} vs {:?}",
            f.line1,
            f.upper
        );
        assert!(
            close(lo, f.lower, 0.02),
            "{}: {lo:?} vs {:?}",
            f.line2,
            f.lower
        );
        // Inside the bands.
        assert!(up[2] >= 30.0 - 0.6 && up[3] <= 38.0, "{up:?}");
        assert!(lo[2] >= 21.0 - 0.6 && lo[3] <= 29.0, "{lo:?}");
    }
}

#[test]
fn fixtures_are_watertight_and_match_python_volume() {
    for f in FIXTURES {
        let s = generate(&SwatchText {
            line1: f.line1.into(),
            line2: f.line2.into(),
        })
        .expect("generates");
        // Re-read the STL exactly as a slicer does (f32 vertices).
        let back = IndexedMesh::from_binary_stl(&s.stl).expect("stl");
        let r = back.check();
        assert!(r.is_watertight(), "{}: {r}", f.line1);
        assert!(r.min.iter().all(|v| v.abs() < 1e-6), "{:?}", r.min);
        assert!((r.max[0] - 75.0).abs() < 1e-4 && (r.max[1] - 40.0).abs() < 1e-4);
        assert!((r.max[2] - 3.0).abs() < 1e-4);
        assert!(r.volume < BASE_VOLUME, "text removed material");
        if let Some(v) = f.volume {
            assert!(
                (r.volume - v).abs() < 0.5,
                "{}: {} vs {v}",
                f.line1,
                r.volume
            );
        }
    }
}

#[test]
fn deboss_is_0_6_mm_deep() {
    let s = generate(&SwatchText {
        line1: "PLA".into(),
        line2: String::new(),
    })
    .expect("generates");
    let m = IndexedMesh::from_binary_stl(&s.stl).expect("stl");
    // Lowest vertex inside the upper band's text area is the deboss floor.
    let floor = m
        .vertices
        .iter()
        .filter(|p| p[0] > 9.5 && p[0] < 20.0 && p[1] > 31.0 && p[1] < 37.0)
        .map(|p| p[2])
        .fold(f64::MAX, f64::min);
    assert!((floor - (SURFACE_Z - CUT_DEPTH)).abs() < 1e-5, "{floor}");
}

#[test]
fn long_text_shrinks_to_fit() {
    let long = "Polymaker PolyTerra PLA Matte";
    let fit = fit_line(long).expect("fits");
    assert!(fit.shrunk);
    assert!(fit.full_width > MAX_WIDTH);
    assert!((fit.width - MAX_WIDTH).abs() < 1e-9, "{}", fit.width);
    assert!(fit.font_size < FONT_SIZE && fit.font_size >= MIN_FONT_SIZE);
    let [x0, x1, y0, y1] = ink(long, UPPER_Y);
    assert!(x0 >= TEXT_X && x1 <= TEXT_X + MAX_WIDTH + 1e-6, "{x0} {x1}");
    // Still centred in its band.
    assert!(y0 > 30.0 && y1 < 38.0);

    // The Python cut this text through the plate edge (X 75); ours stays inside.
    let s = generate(&SwatchText {
        line1: long.into(),
        line2: "Cotton White Sapphire Gradient".into(),
    })
    .expect("generates");
    let r = IndexedMesh::from_binary_stl(&s.stl).expect("stl").check();
    assert!(r.is_watertight(), "{r}");
    assert_eq!(s.line2.as_ref().map(|f| f.shrunk), Some(true));
}

#[test]
fn short_text_keeps_full_size() {
    let fit = fit_line("PLA").expect("fits");
    assert!(!fit.shrunk);
    assert_eq!(fit.font_size, FONT_SIZE);
}

#[test]
fn too_long_text_is_refused() {
    let e = fit_line(&"W".repeat(40)).expect_err("too long");
    assert!(e.to_string().contains("too long"), "{e}");
    let e = fit_line(&"i".repeat(MAX_CHARS + 1)).expect_err("too many chars");
    assert!(e.to_string().contains("at most"), "{e}");
}

#[test]
fn missing_glyphs_are_named() {
    let e = fit_line("PLA 🦀").expect_err("no emoji in the font");
    assert!(e.to_string().contains('🦀'), "{e}");
}

#[test]
fn empty_lines_give_the_plain_plate() {
    let s = generate(&SwatchText::default()).expect("plate");
    assert!(s.line1.is_none() && s.line2.is_none());
    let r = IndexedMesh::from_binary_stl(&s.stl).expect("stl").check();
    assert!((r.volume - BASE_VOLUME).abs() < 0.01);
}

#[test]
fn generation_is_fast() {
    let t = std::time::Instant::now();
    generate(&SwatchText {
        line1: "PRUSAMENT PETG".into(),
        line2: "Galaxy Black".into(),
    })
    .expect("generates");
    let ms = t.elapsed().as_millis();
    // < 1 s is the acceptance bar; the dev profile builds the geometry crates
    // at opt-level 3 (workspace Cargo.toml), so this holds in tests too.
    assert!(ms < 1000, "{ms} ms");
}
