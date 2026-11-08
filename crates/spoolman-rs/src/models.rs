//! JSON response types for the spoolman-rs API.
//!
//! Shapes match the upstream Spoolman v0.27.1 OpenAPI schema so existing
//! clients (Moonraker, Mainsail, Fluidd, OctoPrint-Spoolman) are unaffected.
//!
//! Datetimes are serialised as ISO 8601 strings (`"2024-01-01T00:00:00"`),
//! matching upstream Python's `datetime.isoformat()`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Deserializer that rejects explicit JSON `null` for a field that is
/// otherwise optional (may be absent, but must not be `null` when present).
///
/// Used with `#[serde(default, deserialize_with = "deny_null")]` so that:
/// - absent field → `None` (via `default`)
/// - `null` in JSON → 422 deserialization error
/// - any value     → `Some(value)`
pub fn deny_null<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    struct Visitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: serde::Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
        type Value = Option<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "a non-null value")
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Err(E::custom("Value must not be None."))
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Err(E::custom("Value must not be None."))
        }
        fn visit_some<D2: serde::Deserializer<'de>>(
            self,
            de: D2,
        ) -> Result<Self::Value, D2::Error> {
            Ok(Some(T::deserialize(de)?))
        }
    }
    de.deserialize_option(Visitor::<T>(std::marker::PhantomData))
}

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
    pub tags: Vec<TagInfo>,
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
    #[serde(default, deserialize_with = "deny_null")]
    pub density: Option<f64>,
    #[serde(default, deserialize_with = "deny_null")]
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
    pub first_used: Option<String>,
    pub last_used: Option<String>,
    pub filament: FilamentResponse,
    pub used_weight: f64,
    pub remaining_weight: Option<f64>,
    pub remaining_length: Option<f64>,
    pub used_length: Option<f64>,
    pub location: Option<String>,
    pub lot_nr: Option<String>,
    pub comment: Option<String>,
    pub archived: bool,
    pub price: Option<f64>,
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
    #[serde(default)]
    pub payload_b64: Option<String>,
    #[serde(default)]
    pub create: bool,
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
    /// null when unmatched — always serialised (not skipped) so the client knows the scan
    /// was processed but found no match.
    pub matched_spool_id: Option<i64>,
    pub matched_filament_id: Option<i64>,
    /// Full spool object — present in broadcasts when the UID resolves to a spool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spool: Option<Box<SpoolResponse>>,
    /// Full filament object — present in broadcasts when the UID resolves to a filament.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filament: Option<Box<FilamentResponse>>,
    /// Decoded tag payload — present when `format` + `payload_b64` are given and parseable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded: Option<TagDecodedInfo>,
    pub created: bool,
}

/// Decoded tag payload fields — mirrors upstream `TagDecodedInfo`.
#[derive(Debug, Serialize)]
pub struct TagDecodedInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub article_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diameter: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub density: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spool_weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extruder_temp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bed_temp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_uuid: Option<String>,
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
// Spool group
// ---------------------------------------------------------------------------

/// Minimal spool info included in a filament group when `preview=true`.
#[derive(Debug, Serialize)]
pub struct SpoolPreview {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_weight: Option<f64>,
}

/// Minimal filament info included in a vendor group when `preview=true`.
#[derive(Debug, Serialize)]
pub struct FilamentPreview {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_color_hexes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_color_direction: Option<String>,
}

/// One aggregated spool group returned by `GET /spool/group`.
#[derive(Debug, Serialize)]
pub struct SpoolGroupResponse {
    /// The axis this group was formed on (e.g. `"filament"`, `"location"`).
    pub group_by: String,
    /// The group's key value as a string (null = ungrouped / no-value).
    pub key: Option<String>,
    pub spool_count: i64,
    pub in_use_count: i64,
    pub total_remaining_weight: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
    /// Hydrated when `group_by = "filament"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filament: Option<FilamentResponse>,
    /// Hydrated when `group_by = "vendor"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<VendorResponse>,
    /// Count of distinct filaments — only present when `group_by = "vendor"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filament_count: Option<i64>,
    /// Up to 5 spool previews — only present for filament groups when `preview=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spools: Option<Vec<SpoolPreview>>,
    /// Up to 4 filament previews — only present for vendor groups when `preview=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filaments: Option<Vec<FilamentPreview>>,
}

/// Body for `PATCH /spool/field/{field}`.
#[derive(Debug, Deserialize)]
pub struct RenameFieldValueRequest {
    pub value: String,
    pub new_value: String,
}

/// Response for `PATCH /spool/field/{field}`.
#[derive(Debug, Serialize)]
pub struct RenameFieldValueResult {
    pub spools_updated: i64,
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
    /// Raw JSON-encoded value string (not a parsed JSON value).
    pub value: String,
    pub is_set: bool,
    #[serde(rename = "type")]
    pub type_: String,
}

// ---------------------------------------------------------------------------
// Info / health / backup
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

#[derive(Debug, Serialize)]
pub struct BackupResponse {
    pub path: String,
    pub created: bool,
}

// ---------------------------------------------------------------------------
// Location rename
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RenameLocationBody {
    pub name: String,
}
