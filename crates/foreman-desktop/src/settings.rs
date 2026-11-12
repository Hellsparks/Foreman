//! App-level settings owned by the desktop shell (not the server): `desktop.json`
//! in the app data folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILE: &str = "desktop.json";

pub fn path(app_data: &Path) -> PathBuf {
    app_data.join(FILE)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopSettings {
    /// "Keep running in the background". `None` = not chosen yet: on while the
    /// built-in server is in use, off otherwise.
    #[serde(default)]
    pub keep_running: Option<bool>,
}

impl DesktopSettings {
    /// The setting in force, given whether the built-in server is in use.
    pub fn keep_running(&self, local_in_use: bool) -> bool {
        self.keep_running.unwrap_or(local_in_use)
    }
}

pub fn load(file: &Path) -> DesktopSettings {
    match crate::store::load(file) {
        Ok(v) => serde_json::from_value(v).unwrap_or_default(),
        Err(e) => {
            tracing::warn!("desktop settings unreadable ({e}); using defaults");
            DesktopSettings::default()
        }
    }
}

pub fn save(file: &Path, s: &DesktopSettings) -> Result<(), String> {
    let v = serde_json::to_value(s).map_err(|e| e.to_string())?;
    crate::store::save(file, &v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_running_defaults_follow_the_built_in_server() {
        let unset = DesktopSettings::default();
        assert!(unset.keep_running(true));
        assert!(!unset.keep_running(false));
        let off = DesktopSettings {
            keep_running: Some(false),
        };
        assert!(!off.keep_running(true));
        let on = DesktopSettings {
            keep_running: Some(true),
        };
        assert!(on.keep_running(false));
    }

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join(format!("foreman-settings-{}", std::process::id()));
        let file = path(&dir);
        assert_eq!(load(&file), DesktopSettings::default());
        let s = DesktopSettings {
            keep_running: Some(false),
        };
        save(&file, &s).expect("save");
        assert_eq!(load(&file), s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
