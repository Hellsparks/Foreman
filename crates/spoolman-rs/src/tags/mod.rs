//! Tag payload decoding — mirrors `spoolman/tags/decode.py`.
//!
//! `POST /tag/scan` accepts a `format` and a raw `payload_b64`; format-specific
//! decoding lives here.  Decoding is best-effort: an unrecognised format or an
//! unparseable payload returns `None` rather than an error.

pub mod openprinttag;

use std::collections::HashMap;

/// Decoded tag contents, named after the fields the rest of the API uses.
///
/// Every format's decoder funnels into this shape so the handler and DB layer
/// never need to know which codec produced it.
#[derive(Debug, Clone)]
pub struct DecodedTag {
    pub material_class: Option<String>, // "FFF" or "SLA"
    pub material: Option<String>,       // e.g. "PLA"
    pub name: Option<String>,           // e.g. "PLA Galaxy Black"
    pub vendor_name: Option<String>,    // e.g. "Prusament"
    pub article_number: Option<String>, // vendor GTIN, as text
    pub color_hex: Option<String>,
    pub diameter: Option<f64>,
    pub density: Option<f64>,
    pub weight: Option<f64>,         // nominal full net weight, grams
    pub initial_weight: Option<f64>, // measured net weight this roll, grams
    pub spool_weight: Option<f64>,   // tare weight of empty spool, grams
    pub used_weight: Option<f64>,    // already used — shown, never applied
    pub extruder_temp: Option<i64>,
    pub bed_temp: Option<i64>,
    pub instance_uuid: Option<String>,
}

/// Approximate densities (g/cm³) for common materials, used as a fallback when
/// the tag doesn't carry its own density and the caller needs one to create a
/// filament.
fn approximate_density_table() -> &'static HashMap<&'static str, f64> {
    use std::sync::OnceLock;
    static TABLE: OnceLock<HashMap<&str, f64>> = OnceLock::new();
    TABLE.get_or_init(|| {
        HashMap::from([
            ("PLA", 1.24),
            ("PETG", 1.27),
            ("ABS", 1.04),
            ("ASA", 1.07),
            ("TPU", 1.21),
            ("PA6", 1.14),
            ("PA11", 1.03),
            ("PA12", 1.01),
            ("PA66", 1.15),
            ("PC", 1.20),
            ("PCTG", 1.23),
            ("HIPS", 1.04),
            ("PVA", 1.23),
        ])
    })
}

const FALLBACK_DENSITY: f64 = 1.24;

/// Approximate density for a material, or the PLA-equivalent fallback.
pub fn density_or_fallback(material: Option<&str>) -> f64 {
    material
        .and_then(|m| {
            approximate_density_table()
                .get(m.to_ascii_uppercase().as_str())
                .copied()
        })
        .unwrap_or(FALLBACK_DENSITY)
}

/// Decode a tag's raw payload for a known format.
///
/// Returns `None` if the format is unknown or the payload could not be decoded.
/// Never panics: whatever a hostile or damaged payload does to a codec, the scan
/// carries on as if the payload were not decodable.
pub fn decode(
    tag_format: Option<&str>,
    payload: &[u8],
    uid_bytes: Option<&[u8]>,
) -> Option<DecodedTag> {
    let fmt = tag_format?.trim().to_ascii_lowercase();
    match fmt.as_str() {
        "openprinttag" => match openprinttag::decode_nfcv_memory(payload, uid_bytes) {
            Ok(data) => Some(data.into_decoded_tag()),
            Err(e) => {
                tracing::debug!("could not decode OpenPrintTag payload: {e}");
                None
            }
        },
        _ => None,
    }
}

fn finite(v: Option<f64>) -> Option<f64> {
    v.filter(|x| x.is_finite())
}

impl openprinttag::OpenPrintTagData {
    fn into_decoded_tag(self) -> DecodedTag {
        let extruder_temp = self.max_print_temperature.or(self.min_print_temperature);
        let bed_temp = self.max_bed_temperature.or(self.min_bed_temperature);
        let diameter = finite(Some(self.effective_diameter()));
        let density = finite(self.density);
        let weight = finite(self.nominal_netto_full_weight);
        let initial_weight = finite(self.actual_netto_full_weight);
        let spool_weight = finite(self.empty_container_weight);
        let used_weight = finite(self.consumed_weight);
        let article_number = self.gtin.map(|g| g.to_string());
        let instance_uuid = self.effective_instance_uuid();
        DecodedTag {
            material_class: self.material_class,
            material: self.material_type,
            name: self.material_name,
            vendor_name: self.brand_name,
            article_number,
            color_hex: self.primary_color_hex,
            diameter,
            density,
            weight,
            initial_weight,
            spool_weight,
            used_weight,
            extruder_temp,
            bed_temp,
            instance_uuid,
        }
    }
}
