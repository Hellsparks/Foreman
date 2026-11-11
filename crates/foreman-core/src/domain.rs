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
    /// Slicer printer-profile names that map to this printer (e.g. `["Voron V0 0.4mm"]`).
    pub slicer_profiles: Vec<String>,
    pub sort_order: i64,
    /// Consecutive health-check failures before switching to next path.
    pub fail_after_checks: i64,
    /// Seconds a higher-priority path must be healthy before returning to it.
    pub return_after_secs: i64,
    /// Whether the print bed is clear and ready for the next job.
    pub plate_clear: bool,
    /// Whether the scheduler should auto-start queue items on this printer.
    pub auto_start_from_queue: bool,
    /// Who reports filament use to Spoolman (3.6).
    pub usage_reporter: UsageReporter,
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
    #[serde(default)]
    pub slicer_profiles: Vec<String>,
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
    pub slicer_profiles: Option<Vec<String>>,
    pub sort_order: Option<i64>,
    pub fail_after_checks: Option<i64>,
    pub return_after_secs: Option<i64>,
    pub plate_clear: Option<bool>,
    pub auto_start_from_queue: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub usage_reporter: Option<UsageReporter>,
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
    /// The job a "print finished" item is about (rate it Good / Failed).
    #[ts(type = "number | null")]
    pub job_id: Option<i64>,
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

/// How a file's filaments reach the nozzle (W9 slot model).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum SlotKind {
    /// One slot per physical tool / extruder (single-tool files too).
    #[default]
    Tool,
    /// One extruder fed by an AMS / MMU: one slot per filament (tray).
    Ams,
    /// One extruder, several filaments in sequence (M600 / colour change):
    /// the user swaps spools by hand during the print.
    Change,
}

impl SlotKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Ams => "ams",
            Self::Change => "change",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "ams" => Self::Ams,
            "change" => Self::Change,
            _ => Self::Tool,
        }
    }
}

/// One filament a sliced file needs: a tool, an AMS tray, or one colour of a
/// colour-change print. `index` is what project plates map to colour slots.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FilamentSlot {
    /// Tool index (tool / ams) or colour order (change), from 0.
    pub index: u32,
    /// Extruder / filament id (0-based) that prints it.
    pub tool: u32,
    /// "T0", "AMS 2", "Colour 1".
    pub label: String,
    pub material: Option<String>,
    /// `#RRGGBB`.
    pub colour: Option<String>,
    pub grams: Option<f64>,
    pub mm: Option<f64>,
    /// Slicer filament preset.
    pub preset: Option<String>,
    #[ts(type = "number | null")]
    pub spool_id: Option<i64>,
    /// Bambu `tray_info_idx` (filament preset id, e.g. `GFA00`); the AMS tray
    /// itself is chosen when the print is sent.
    pub tray: Option<String>,
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
    /// tool | ams | change.
    #[serde(default)]
    pub slot_kind: SlotKind,
    /// The filaments this file needs (see [`FilamentSlot`]).
    #[serde(default)]
    pub slots: Vec<FilamentSlot>,
}

/// `GET /api/library/files/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LibraryFileDetails {
    pub entry: LibraryEntry,
    pub meta: Option<FileMeta>,
}

// ---------------------------------------------------------------------------
// Queues (task 3.5)
// ---------------------------------------------------------------------------

/// Lifecycle of a queue item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum QueueStatus {
    Waiting,
    Sending,
    Printing,
    Done,
    Failed,
    Cancelled,
}

impl QueueStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Sending => "sending",
            Self::Printing => "printing",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "sending" => Self::Sending,
            "printing" => Self::Printing,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Waiting,
        }
    }

    /// Sending or printing: the item occupies its printer.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Sending | Self::Printing)
    }
}

/// Where a queue item came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum QueueSource {
    Manual,
    Slicer,
    Project,
}

impl QueueSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Slicer => "slicer",
            Self::Project => "project",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "slicer" => Self::Slicer,
            "project" => Self::Project,
            _ => Self::Manual,
        }
    }
}

/// One job waiting in (or dispatched from) a printer queue or the fleet queue.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct QueueItem {
    #[ts(type = "number")]
    pub id: i64,
    /// Library entry to print.
    #[ts(type = "number")]
    pub file_id: i64,
    /// File name when it was queued (kept if the Library entry goes away).
    pub file_name: String,
    pub source: QueueSource,
    pub added_by: String,
    pub added_at: String,
    pub status: QueueStatus,
    /// Lane: `null` = fleet queue, else that printer's queue. Once a fleet item is
    /// dispatched this is the printer it went to and `from_fleet` is true.
    #[ts(type = "number | null")]
    pub printer_id: Option<i64>,
    pub from_fleet: bool,
    /// Order inside the lane (lower first).
    #[ts(type = "number")]
    pub position: i64,
    /// Why it failed (upload / start error, missing file).
    pub error: Option<String>,
    pub updated_at: String,
    /// The project plate this print belongs to (W9), if any.
    #[ts(type = "number | null")]
    pub project_plate_id: Option<i64>,
}

// ---------------------------------------------------------------------------
// Jobs (task 3.6)
// ---------------------------------------------------------------------------

/// Who reports a printer's filament use to Spoolman.
/// `auto` = Moonraker when its `[spoolman]` link is connected, else Foreman.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum UsageReporter {
    #[default]
    Auto,
    Moonraker,
    Foreman,
}

impl UsageReporter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Moonraker => "moonraker",
            Self::Foreman => "foreman",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "moonraker" => Self::Moonraker,
            "foreman" => Self::Foreman,
            _ => Self::Auto,
        }
    }
}

/// How a job ended, as the printer reported it. `running` while it prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum JobEnd {
    Running,
    Complete,
    Cancelled,
    Error,
}

impl JobEnd {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Complete => "complete",
            Self::Cancelled => "cancelled",
            Self::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "complete" => Self::Complete,
            "cancelled" => Self::Cancelled,
            "error" => Self::Error,
            _ => Self::Running,
        }
    }
}

/// The user's verdict on a job. A clean finish starts unrated (`None` on the job)
/// until someone rates it from Attention or History; a cancel starts `cancelled`
/// and an error starts `failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum JobOutcome {
    Good,
    Failed,
    Cancelled,
}

impl JobOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "good" => Some(Self::Good),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// Filament for one tool of a job: planned (from the file) and deducted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobTool {
    pub index: u32,
    pub material: Option<String>,
    pub colour: Option<String>,
    /// Grams the file needs for a full print.
    pub grams_planned: Option<f64>,
    /// Grams this print used: planned for a full print, planned × progress for a
    /// cancelled / failed one. Set when the job ends.
    pub grams_used: Option<f64>,
    /// Grams Foreman took off the spool (0 when Moonraker reports, or no spool).
    pub grams_deducted: f64,
    #[ts(type = "number | null")]
    pub spool_id: Option<i64>,
}

/// One print: started, ended, what it used and how it went.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Job {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number | null")]
    pub printer_id: Option<i64>,
    /// Printer name when the job ran (kept if the printer is deleted).
    pub printer_name: String,
    /// Library file, when Foreman sent it.
    #[ts(type = "number | null")]
    pub file_id: Option<i64>,
    pub file_name: String,
    #[ts(type = "number | null")]
    pub queue_item_id: Option<i64>,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// Seconds from start to end (printer-reported print time when known).
    #[ts(type = "number | null")]
    pub duration_secs: Option<i64>,
    pub end: JobEnd,
    pub outcome: Option<JobOutcome>,
    /// adhesion | spaghetti | layer shift | clog | warping | other.
    pub fail_reason: Option<String>,
    pub error: Option<String>,
    /// Who reported filament use for this job: `moonraker` or `foreman`.
    pub usage_reporter: String,
    pub grams_planned: Option<f64>,
    /// Sum of the tools' `grams_used`.
    pub grams_used: Option<f64>,
    /// Grams Foreman took off spools (0 when Moonraker reports usage itself).
    pub grams_deducted: f64,
    /// Main material (first tool).
    pub material: Option<String>,
    pub tools: Vec<JobTool>,
    /// `/api/library/files/{id}/thumb` when the file has one.
    pub thumb_url: Option<String>,
}

/// `PATCH /api/jobs/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobOutcomeUpdate {
    pub outcome: JobOutcome,
    #[serde(default)]
    pub fail_reason: Option<String>,
}

/// `GET /api/jobs` result.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobList {
    pub jobs: Vec<Job>,
    #[ts(type = "number")]
    pub total: i64,
}

/// Per printer, over the stats period.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrinterStats {
    #[ts(type = "number | null")]
    pub printer_id: Option<i64>,
    pub name: String,
    pub hours: f64,
    #[ts(type = "number")]
    pub jobs: i64,
    #[ts(type = "number")]
    pub good: i64,
    #[ts(type = "number")]
    pub failed: i64,
    #[ts(type = "number")]
    pub cancelled: i64,
    /// failed / (jobs − cancelled − still running); 0 when none.
    pub failure_rate: f64,
    /// Share of the period spent printing, 0–100.
    pub utilization_pct: f64,
}

/// Grams by material or by spool.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UsageStat {
    pub key: String,
    #[ts(type = "number | null")]
    pub spool_id: Option<i64>,
    pub grams: f64,
    #[ts(type = "number")]
    pub jobs: i64,
}

/// `GET /api/jobs/stats`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobStats {
    pub from: String,
    pub to: String,
    pub hours: f64,
    #[ts(type = "number")]
    pub jobs: i64,
    #[ts(type = "number")]
    pub good: i64,
    #[ts(type = "number")]
    pub failed: i64,
    #[ts(type = "number")]
    pub cancelled: i64,
    #[ts(type = "number")]
    pub unrated: i64,
    /// 1 − failed / (jobs − cancelled − running); null when there are none.
    pub success_rate: Option<f64>,
    pub grams: f64,
    pub printers: Vec<PrinterStats>,
    pub by_material: Vec<UsageStat>,
    pub by_spool: Vec<UsageStat>,
}
