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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
