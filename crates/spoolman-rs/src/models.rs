//! JSON response types for the spoolman-rs API.
//!
//! Shapes match the upstream Spoolman v0.27.1 OpenAPI schema so existing
//! clients (Moonraker, Mainsail, Fluidd, OctoPrint-Spoolman) are unaffected.
//!
//! Datetimes are serialised as ISO 8601 strings (`"2024-01-01T00:00:00"`),
//! matching upstream Python's `datetime.isoformat()`.  Extra fields are
//! serialised as a JSON object; they return `{}` until session 7 wires the
//! DB queries.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// ---------------------------------------------------------------------------
// Vendor
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorResponse {
    pub id: i64,
    pub registered: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty_spool_weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct CreateVendorRequest {
    pub name: String,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub empty_spool_weight: Option<f64>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateVendorRequest {
    pub name: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub empty_spool_weight: Option<f64>,
    #[serde(default)]
    pub external_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Filament
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilamentResponse {
    pub id: i64,
    pub registered: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<VendorResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
    pub density: f64,
    pub diameter: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spool_weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub article_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings_extruder_temp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings_bed_temp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_color_hexes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_color_direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFilamentRequest {
    pub density: f64,
    pub diameter: f64,
    pub name: Option<String>,
    #[serde(default)]
    pub vendor_id: Option<i64>,
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub price: Option<f64>,
    #[serde(default)]
    pub weight: Option<f64>,
    #[serde(default)]
    pub spool_weight: Option<f64>,
    #[serde(default)]
    pub article_number: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub settings_extruder_temp: Option<i64>,
    #[serde(default)]
    pub settings_bed_temp: Option<i64>,
    #[serde(default)]
    pub color_hex: Option<String>,
    #[serde(default)]
    pub multi_color_hexes: Option<String>,
    #[serde(default)]
    pub multi_color_direction: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFilamentRequest {
    pub name: Option<String>,
    #[serde(default)]
    pub vendor_id: Option<i64>,
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub price: Option<f64>,
    #[serde(default)]
    pub density: Option<f64>,
    #[serde(default)]
    pub diameter: Option<f64>,
    #[serde(default)]
    pub weight: Option<f64>,
    #[serde(default)]
    pub spool_weight: Option<f64>,
    #[serde(default)]
    pub article_number: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub settings_extruder_temp: Option<i64>,
    #[serde(default)]
    pub settings_bed_temp: Option<i64>,
    #[serde(default)]
    pub color_hex: Option<String>,
    #[serde(default)]
    pub multi_color_hexes: Option<String>,
    #[serde(default)]
    pub multi_color_direction: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Spool
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpoolResponse {
    pub id: i64,
    pub registered: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_used: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
    pub filament: FilamentResponse,
    pub used_weight: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_length: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_length: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lot_nr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spool_weight: Option<f64>,
    pub tags: Vec<Value>,
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSpoolRequest {
    pub filament_id: i64,
    #[serde(default)]
    pub used_weight: Option<f64>,
    #[serde(default)]
    pub remaining_weight: Option<f64>,
    #[serde(default)]
    pub initial_weight: Option<f64>,
    #[serde(default)]
    pub spool_weight: Option<f64>,
    #[serde(default)]
    pub first_used: Option<String>,
    #[serde(default)]
    pub last_used: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub lot_nr: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
    #[serde(default)]
    pub price: Option<f64>,
    #[serde(default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSpoolRequest {
    #[serde(default)]
    pub filament_id: Option<i64>,
    #[serde(default)]
    pub used_weight: Option<f64>,
    #[serde(default)]
    pub remaining_weight: Option<f64>,
    #[serde(default)]
    pub initial_weight: Option<f64>,
    #[serde(default)]
    pub spool_weight: Option<f64>,
    #[serde(default)]
    pub first_used: Option<String>,
    #[serde(default)]
    pub last_used: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub lot_nr: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
    #[serde(default)]
    pub price: Option<f64>,
}
