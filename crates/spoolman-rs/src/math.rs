//! Weight ↔ length conversion — ported from upstream `database/utils.py`.

use std::f64::consts::PI;

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
