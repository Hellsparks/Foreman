//! JSON response types for the spoolman-rs API.
//!
//! Shapes match the upstream Spoolman v0.27.1 OpenAPI schema so existing
//! clients (Moonraker, Mainsail, Fluidd, OctoPrint-Spoolman) are unaffected.
//!
//! Datetimes are serialised as ISO 8601 strings (`"2024-01-01T00:00:00"`),
//! matching upstream Python's `datetime.isoformat()`.

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
    #[serde(default)]
    pub extra: Option<Map<String, Value>>,
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
    #[serde(default)]
    pub extra: Option<Map<String, Value>>,
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
    pub tags: Vec<TagInfo>,
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
    #[serde(default)]
    pub extra: Option<Map<String, Value>>,
}

// ---------------------------------------------------------------------------
// Tag
// ---------------------------------------------------------------------------

/// A single NFC/RFID tag linked to a spool, embedded in spool responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagInfo {
    pub id: i64,
    pub uid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    pub added: String,
}

/// Body for POST /spool/{id}/tag — link a physical tag to a spool.
#[derive(Debug, Deserialize)]
pub struct TagLinkRequest {
    pub uid: String,
    #[serde(default)]
    pub format: Option<String>,
}

/// 409 body returned when a UID is already linked to something else.
#[derive(Debug, Serialize)]
pub struct TagConflictMessage {
    pub message: String,
    pub spool_id: Option<i64>,
    pub filament_id: Option<i64>,
}

// ---------------------------------------------------------------------------
// Scan relay
// ---------------------------------------------------------------------------

/// Body for POST /tag/scan.
#[derive(Debug, Deserialize)]
pub struct ScanTagRequest {
    pub uid: String,
    #[serde(default)]
    pub reader_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
}

/// Response from POST /tag/scan.
#[derive(Debug, Serialize)]
pub struct ScanTagResponse {
    pub uid: String,
    pub reader_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// null when unmatched — always serialised (not skipped on null).
    pub matched_spool_id: Option<i64>,
    /// null when unmatched — always serialised.
    pub matched_filament_id: Option<i64>,
    pub created: bool,
}

/// One entry in GET /tag/reader.
#[derive(Debug, Serialize)]
pub struct TagReaderInfo {
    pub reader_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub last_seen: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_uid: Option<String>,
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// Top-level result from GET /search.
#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub spools: Vec<SearchResultSpool>,
    pub filaments: Vec<SearchResultFilament>,
    pub vendors: Vec<SearchResultVendor>,
    pub is_color_query: bool,
}

#[derive(Debug, Serialize)]
pub struct SearchResultSpool {
    pub spool: SpoolResponse,
    pub match_field: String,
}

#[derive(Debug, Serialize)]
pub struct SearchResultFilament {
    pub filament: FilamentResponse,
    pub match_field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spools: Option<Vec<SearchResultFilamentSpool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spool_count: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SearchResultVendor {
    pub vendor: VendorResponse,
    pub match_field: String,
}

/// Minimal spool summary included with a filament search result.
#[derive(Debug, Serialize)]
pub struct SearchResultFilamentSpool {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_weight: Option<f64>,
    pub used_weight: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    pub archived: bool,
}

// ---------------------------------------------------------------------------
// Spool use / measure
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SpoolUseRequest {
    pub use_weight: Option<f64>,
    pub use_length: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct SpoolMeasureRequest {
    pub weight: f64,
}

// ---------------------------------------------------------------------------
// Extra fields
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExtraFieldType {
    Text,
    Integer,
    IntegerRange,
    Float,
    FloatRange,
    Datetime,
    Boolean,
    Choice,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraField {
    pub key: String,
    pub entity_type: String,
    pub name: String,
    #[serde(default)]
    pub order: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    pub field_type: ExtraFieldType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_choice: Option<bool>,
}

/// Parameters for POST /field/{entity_type}/{key} — same as ExtraField but without key/entity_type.
#[derive(Debug, Deserialize)]
pub struct ExtraFieldParams {
    pub name: String,
    #[serde(default)]
    pub order: i64,
    pub field_type: ExtraFieldType,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub default_value: Option<String>,
    #[serde(default)]
    pub choices: Option<Vec<String>>,
    #[serde(default)]
    pub multi_choice: Option<bool>,
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingResponse {
    pub value: Value,
    pub is_set: bool,
    #[serde(rename = "type")]
    pub type_: String,
}

// ---------------------------------------------------------------------------
// Info / health
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct HealthCheck {
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct InfoResponse {
    pub version: &'static str,
    pub db_type: String,
}

// ---------------------------------------------------------------------------
// Location rename
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RenameLocationBody {
    pub name: String,
}
