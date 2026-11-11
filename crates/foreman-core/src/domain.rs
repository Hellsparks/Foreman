//! Domain types for the Foreman-native database (Phase 2+).
//!
//! Every DTO the frontend consumes is marked `#[ts(export)]` so `cargo gen-types`
//! writes the corresponding TypeScript file into `frontend/src/api/types/`.
//!
//! Columns are intentionally minimal — only what sections 2.1–2.8 of doc 05 require.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ---------------------------------------------------------------------------
// Firmware kind
// ---------------------------------------------------------------------------

/// The printer firmware / connection protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareKind {
    Moonraker,
    Octoprint,
    Bambu,
    Duet,
}

// ---------------------------------------------------------------------------
// Printer
// ---------------------------------------------------------------------------

/// A printer record as stored in the Foreman database.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Printer {
    pub id: i64,
    pub name: String,
    pub firmware: FirmwareKind,
    pub primary_url: Option<String>,
    /// Moonraker API key, OctoPrint API key, etc.
    pub api_key: Option<String>,
    /// Bambu serial number (e.g. `01P00C123456789`).
    pub serial_number: Option<String>,
    /// Bambu LAN access code (8-digit).
    pub access_code: Option<String>,
    /// TLS certificate SHA-256 fingerprint (TOFU pinning for Bambu/FTPS).
    pub tls_fingerprint: Option<String>,
    /// User-defined labels.
    pub tags: Vec<String>,
    pub hardened_nozzle: bool,
    pub sort_order: i64,
    /// Consecutive health-check failures before switching to next path.
    pub fail_after_checks: i64,
    /// Seconds a higher-priority path must be healthy before returning to it.
    pub return_after_secs: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A printer record enriched with active path info (from the driver manager).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrinterWithStatus {
    #[serde(flatten)]
    pub printer: Printer,
    /// Index of the currently active connection path (0 = primary).
    pub active_path_index: Option<i64>,
    /// Display name of the currently active connection path.
    pub active_path_name: Option<String>,
}

/// Fields required to create a new printer.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CreatePrinter {
    pub name: String,
    pub firmware: FirmwareKind,
    pub primary_url: Option<String>,
    pub api_key: Option<String>,
    pub serial_number: Option<String>,
    pub access_code: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub hardened_nozzle: bool,
    pub sort_order: Option<i64>,
    #[serde(default)]
    pub fail_after_checks: Option<i64>,
    #[serde(default)]
    pub return_after_secs: Option<i64>,
}

/// Fields that may be updated on an existing printer.
/// `None` = leave unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdatePrinter {
    pub name: Option<String>,
    pub firmware: Option<FirmwareKind>,
    pub primary_url: Option<String>,
    pub api_key: Option<String>,
    pub serial_number: Option<String>,
    pub access_code: Option<String>,
    pub tags: Option<Vec<String>>,
    pub hardened_nozzle: Option<bool>,
    pub sort_order: Option<i64>,
    pub fail_after_checks: Option<i64>,
    pub return_after_secs: Option<i64>,
}

// ---------------------------------------------------------------------------
// PrinterPath
// ---------------------------------------------------------------------------

/// A tiered connection path for a printer (ordered by `priority` ascending).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrinterPath {
    pub id: i64,
    pub printer_id: i64,
    pub name: String,
    pub url: String,
    pub priority: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Fields required to create a new printer path.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CreatePrinterPath {
    pub printer_id: i64,
    pub name: String,
    pub url: String,
    pub priority: Option<i64>,
}

/// Fields that may be updated on an existing printer path.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdatePrinterPath {
    pub name: Option<String>,
    pub url: Option<String>,
    pub priority: Option<i64>,
}

// ---------------------------------------------------------------------------
// PrinterSsh
// ---------------------------------------------------------------------------

/// SSH / SFTP settings for a printer.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrinterSsh {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub printer_id: i64,
    pub ssh_user: String,
    #[ts(type = "number")]
    pub ssh_port: i64,
    /// "key" | "password"
    pub auth_type: String,
    /// SHA-256 fingerprint of the pinned host key (TOFU).
    pub host_key_fingerprint: Option<String>,
    /// A different key the server presented; connections are refused until trusted.
    pub pending_host_key_fingerprint: Option<String>,
    /// A password is stored (never returned).
    pub has_password: bool,
    /// Close idle SSH/SFTP/terminal connections after this many seconds; 0 = never.
    #[ts(type = "number")]
    pub idle_timeout_secs: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Fields for creating or updating SSH settings on a printer.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpsertPrinterSsh {
    pub ssh_user: String,
    #[ts(type = "number | null")]
    pub ssh_port: Option<i64>,
    /// "key" | "password"
    pub auth_type: Option<String>,
    /// Only used when auth_type = "password"; encrypted at rest. `None` keeps the stored one.
    pub password: Option<String>,
    /// Idle auto-close in seconds (0 = never). `None` keeps the stored value (default 900).
    #[ts(type = "number | null")]
    pub idle_timeout_secs: Option<i64>,
}

// ---------------------------------------------------------------------------
// ShellMacro
// ---------------------------------------------------------------------------

/// A named multi-line shell macro stored in Foreman.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ShellMacro {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    /// Multi-line command body, newline-separated.
    pub commands: String,
    /// "printer" | "all_ssh"
    pub scope: String,
    /// Null when scope = "all_ssh".
    #[ts(type = "number | null")]
    pub printer_id: Option<i64>,
    pub ask_before_run: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Fields required to create a new shell macro.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CreateShellMacro {
    pub name: String,
    pub commands: String,
    pub scope: Option<String>,
    pub ask_before_run: Option<bool>,
}

/// Fields that may be updated on an existing shell macro.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdateShellMacro {
    pub name: Option<String>,
    pub commands: Option<String>,
    /// "printer" | "all_ssh"
    pub scope: Option<String>,
    pub ask_before_run: Option<bool>,
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/// An API key as listed in Settings → Security (the secret itself is never returned).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApiKey {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    /// First characters of the key, for recognising it.
    pub prefix: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

// ---------------------------------------------------------------------------
// SftpEntry  (returned by SFTP directory list)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SftpEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

/// A single entry in an SFTP directory listing.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SftpEntry {
    pub name: String,
    pub path: String,
    pub kind: SftpEntryKind,
    #[ts(type = "number | null")]
    pub size: Option<u64>,
    /// Unix timestamp (seconds since epoch), if available.
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
}

// ---------------------------------------------------------------------------
// AttentionItem
// ---------------------------------------------------------------------------

/// A live attention item (cleared_at IS NULL in the DB).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AttentionItem {
    pub id: i64,
    pub printer_id: Option<i64>,
    /// Kind tag: "offline" | "error" (auto-raised) + future manual kinds.
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Library (task 3.1)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum LibraryEntryKind {
    File,
    Folder,
}

/// A file or folder in the Library (path is relative to the library root, `/`-separated).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LibraryEntry {
    #[ts(type = "number")]
    pub id: i64,
    pub path: String,
    pub name: String,
    pub kind: LibraryEntryKind,
    #[ts(type = "number")]
    pub size: i64,
    /// SHA-256 hex of the file contents.
    pub hash: Option<String>,
    pub added_at: String,
    pub starred: bool,
    /// Set while the entry is in the trash.
    pub deleted_at: Option<String>,
    /// `gcode` | `bgcode` | `3mf` | `stl` | `other` (files only).
    pub format: Option<String>,
    /// `/api/library/files/{id}/thumb` when a thumbnail exists.
    pub thumb_url: Option<String>,
    /// Parsed print time (seconds), for grid cards.
    #[ts(type = "number | null")]
    pub print_time_secs: Option<i64>,
    /// Parsed per-tool filament use, for grid cards.
    pub tools: Vec<ToolUsage>,
}

/// Filament use for one tool / extruder / AMS slot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolUsage {
    pub index: u32,
    pub material: Option<String>,
    /// `#RRGGBB`.
    pub colour: Option<String>,
    pub grams: Option<f64>,
    pub mm: Option<f64>,
    /// Slicer filament preset (`filament_settings_id`).
    pub preset: Option<String>,
    /// From a preset named like `Vendor Name (#ID) - Spoolman`.
    #[ts(type = "number | null")]
    pub spool_id: Option<i64>,
}

/// One sliced plate of a 3MF project.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlateSummary {
    pub index: u32,
    #[ts(type = "number | null")]
    pub print_time_secs: Option<i64>,
    pub weight_g: Option<f64>,
    pub tools: Vec<ToolUsage>,
}

/// Facts extracted from a print file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileMeta {
    pub format: String,
    /// `ok` | `unsupported` | `error`.
    pub parse_status: String,
    pub parse_error: Option<String>,
    pub slicer: Option<String>,
    pub slicer_version: Option<String>,
    pub printer_model: Option<String>,
    /// Slicer printer profile name (`printer_settings_id`); matched against a printer's slicer profiles.
    pub printer_settings_id: Option<String>,
    /// Bambu printer model code from `slice_info.config` (e.g. `N2S` = A1 mini).
    pub printer_model_id: Option<String>,
    pub nozzle_diameter: Option<f64>,
    #[ts(type = "number | null")]
    pub print_time_secs: Option<i64>,
    pub layer_count: Option<u32>,
    pub bed_width: Option<f64>,
    pub bed_depth: Option<f64>,
    pub filament_settings_id: Option<String>,
    #[ts(type = "number | null")]
    pub suggested_spool_id: Option<i64>,
    pub tools: Vec<ToolUsage>,
    pub plates: Vec<PlateSummary>,
}

/// `GET /api/library/files/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LibraryFileDetails {
    pub entry: LibraryEntry,
    pub meta: Option<FileMeta>,
}
