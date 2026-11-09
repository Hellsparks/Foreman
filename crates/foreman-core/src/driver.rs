//! PrinterDriver trait — the interface every firmware driver implements.
//!
//! Phase 2.3: trait + Moonraker driver. OctoPrint / Duet / Bambu follow later.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{error::AppError, events::PrinterStatus};

// ---------------------------------------------------------------------------
// Capabilities (returned by probe)
// ---------------------------------------------------------------------------

/// Static capabilities discovered from a printer on first connect.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrinterCapabilities {
    /// Bed dimensions in mm (x, y).
    pub bed_size: Option<(f64, f64)>,
    /// Number of extruders (typically 1).
    pub extruder_count: u32,
    /// List of webcam URLs discovered from the firmware.
    pub webcams: Vec<WebcamInfo>,
    /// Available macro names (Klipper gcode_macro, etc.).
    pub macros: Vec<String>,
    /// Klipper version string, if available.
    pub firmware_version: Option<String>,
    /// Hostname reported by the firmware.
    pub hostname: Option<String>,
    /// Nozzle diameter in mm (Bambu).
    pub nozzle_diameter: Option<f64>,
    /// Number of AMS tray slots available (4 per AMS unit).
    pub ams_slot_count: Option<u32>,
    /// Camera type: "basic", "ai", or None.
    pub camera_type: Option<String>,
    /// Printer model string derived from serial prefix (Bambu).
    pub printer_model: Option<String>,
}

/// A webcam discovered via the firmware API.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WebcamInfo {
    pub name: String,
    pub stream_url: Option<String>,
    pub snapshot_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Command types (trait method arguments)
// ---------------------------------------------------------------------------

/// A command that can be sent to a printer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrinterCommand {
    /// Pause the current print.
    Pause,
    /// Resume a paused print.
    Resume,
    /// Cancel the current print.
    Cancel,
    /// Set extruder target temperature.
    SetNozzleTemp { target: f64 },
    /// Set bed target temperature.
    SetBedTemp { target: f64 },
    /// Home axes. Empty = home all.
    Home { axes: Vec<String> },
    /// Jog relative to current position.
    Jog {
        x: Option<f64>,
        y: Option<f64>,
        z: Option<f64>,
        speed: Option<f64>,
    },
    /// Babystep Z offset (SET_GCODE_OFFSET Z_ADJUST=N MOVE=1).
    BabyStep { offset: f64 },
    /// Motors off (M84).
    MotorsOff,
    /// Extrude or retract filament (relative E move).
    Extrude { length: f64, speed: f64 },
    /// Toggle light (output_pin or LED named "light").
    SetLight { brightness: f64 },
    /// Execute a named macro.
    Macro { name: String },
    /// Send raw G-code.
    Gcode { command: String },
    /// Emergency stop.
    EmergencyStop,
}

/// A file entry on the printer's storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterFile {
    pub path: String,
    pub size: Option<u64>,
    pub modified: Option<f64>,
}

// ---------------------------------------------------------------------------
// PrinterDriver trait
// ---------------------------------------------------------------------------

/// The core printer driver interface.
///
/// Each firmware type (Moonraker, OctoPrint, Duet, Bambu) implements this.
/// The driver manager spawns one driver per printer and calls these methods.
///
/// `status_stream` is not a method — instead, the driver is given a broadcast
/// sender at construction time and publishes `PrinterStatus` updates to it.
#[async_trait]
pub trait PrinterDriver: Send + Sync + 'static {
    /// Connect to the printer. Returns capabilities on success.
    async fn connect(&self) -> Result<PrinterCapabilities, AppError>;

    /// Disconnect cleanly.
    async fn disconnect(&self) -> Result<(), AppError>;

    /// Probe the printer for static capabilities (bed size, extruders, etc.).
    /// May be called after connect to refresh.
    async fn probe_capabilities(&self) -> Result<PrinterCapabilities, AppError>;

    /// Send a command to the printer.
    async fn send_command(&self, cmd: PrinterCommand) -> Result<(), AppError>;

    /// List files on the printer's storage.
    async fn list_files(&self, path: Option<&str>) -> Result<Vec<PrinterFile>, AppError>;

    /// Upload a file to the printer's storage.
    async fn upload_file(&self, path: &str, data: Vec<u8>) -> Result<(), AppError>;

    /// Delete a file on the printer's storage.
    async fn delete_file(&self, path: &str) -> Result<(), AppError>;

    /// Start printing a file already on the printer.
    async fn start_print(&self, path: &str) -> Result<(), AppError>;

    /// Get the current status snapshot.
    async fn current_status(&self) -> Result<PrinterStatus, AppError>;
}
