//! Weight ↔ length conversion and color math — ported from upstream `spoolman/math.py`.

use std::f64::consts::PI;

// ---------------------------------------------------------------------------
// Color math (ported from upstream spoolman/math.py)
// ---------------------------------------------------------------------------

/// Parse a 6-char hex string (with or without leading `#`) into `[r, g, b]`.
pub fn hex_to_rgb(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.trim_start_matches('#');
    if hex.len() < 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some([r, g, b])
}

/// Convert `[r, g, b]` (0–255) to CIE L*a*b* `[L, a, b]`.
pub fn rgb_to_lab(rgb: [u8; 3]) -> [f64; 3] {
    let r = rgb[0] as f64 / 255.0;
    let g = rgb[1] as f64 / 255.0;
    let b = rgb[2] as f64 / 255.0;

    fn lin(v: f64) -> f64 {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    let r = lin(r);
    let g = lin(g);
    let b = lin(b);

    let x = (r * 0.4124 + g * 0.3576 + b * 0.1805) / 0.95047;
    let y = (r * 0.2126 + g * 0.7152 + b * 0.0722) / 1.00000;
    let z = (r * 0.0193 + g * 0.1192 + b * 0.9505) / 1.08883;

    fn f(v: f64) -> f64 {
        if v > 0.008856 {
            v.powf(1.0 / 3.0)
        } else {
            7.787 * v + 16.0 / 116.0
        }
    }
    let x = f(x);
    let y = f(y);
    let z = f(z);

    [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

/// CIE94 delta-E colour difference between two L*a*b* triples.
pub fn delta_e(lab_a: [f64; 3], lab_b: [f64; 3]) -> f64 {
    let delta_l = lab_a[0] - lab_b[0];
    let delta_a = lab_a[1] - lab_b[1];
    let delta_b_v = lab_a[2] - lab_b[2];
    let c1 = (lab_a[1] * lab_a[1] + lab_a[2] * lab_a[2]).sqrt();
    let c2 = (lab_b[1] * lab_b[1] + lab_b[2] * lab_b[2]).sqrt();
    let delta_c = c1 - c2;
    let delta_h_sq = delta_a * delta_a + delta_b_v * delta_b_v - delta_c * delta_c;
    let delta_h = if delta_h_sq > 0.0 {
        delta_h_sq.sqrt()
    } else {
        0.0
    };
    let sc = 1.0 + 0.045 * c1;
    let sh = 1.0 + 0.015 * c1;
    let dl = delta_l; // / 1.0
    let dc = delta_c / sc;
    let dh = delta_h / sh;
    let i = dl * dl + dc * dc + dh * dh;
    if i > 0.0 {
        i.sqrt()
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// Weight ↔ length conversion
// ---------------------------------------------------------------------------

/// Convert a filament weight (grams) to length (millimetres).
///
/// Formula (from upstream `length_from_weight`):
/// ```text
/// radius_cm  = diameter_mm / 20
/// volume_cm3 = weight_g / density_g_per_cm3
/// length_cm  = volume_cm3 / (π × radius_cm²)
/// length_mm  = length_cm × 10
/// ```
pub fn length_from_weight(weight_g: f64, density: f64, diameter_mm: f64) -> f64 {
    let radius_cm = diameter_mm / 20.0;
    let volume_cm3 = weight_g / density;
    let length_cm = volume_cm3 / (PI * radius_cm * radius_cm);
    length_cm * 10.0
}

/// Convert a filament length (millimetres) to weight (grams).
///
/// Formula (from upstream `weight_from_length`):
/// ```text
/// volume_mm3 = length_mm × π × (diameter_mm/2)²
/// volume_cm3 = volume_mm3 / 1000
/// weight_g   = density × volume_cm3
/// ```
pub fn weight_from_length(length_mm: f64, density: f64, diameter_mm: f64) -> f64 {
    let radius_mm = diameter_mm / 2.0;
    let volume_mm3 = length_mm * PI * radius_mm * radius_mm;
    let volume_cm3 = volume_mm3 / 1000.0;
    density * volume_cm3
}
