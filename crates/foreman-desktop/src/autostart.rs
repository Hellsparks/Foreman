//! Start with the computer (Tauri's autostart plugin). Off by default.
//!
//! Linux: `~/.config/autostart/<name>.desktop`. Windows: a value named `<name>` under
//! `…\CurrentVersion\Run` (HKLM when allowed, else HKCU). The entry passes [`ARG`] so
//! the app starts hidden in the tray when "keep running in the background" is on.

use tauri::{plugin::TauriPlugin, Runtime};

/// Passed by the autostart entry.
pub const ARG: &str = "--autostart";

/// Name of the autostart entry.
pub const NAME: &str = "Foreman";

pub fn plugin<R: Runtime>(name: &str) -> TauriPlugin<R> {
    tauri_plugin_autostart::Builder::new()
        .app_name(name)
        .arg(ARG)
        .build()
}

/// Whether this launch came from the autostart entry.
pub fn launched_by_autostart<I: IntoIterator<Item = String>>(args: I) -> bool {
    args.into_iter().any(|a| a == ARG)
}
