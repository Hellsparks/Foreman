//! App-level settings owned by the desktop shell (not the server): `desktop.json`
//! in the app data folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILE: &str = "desktop.json";

pub fn path(app_data: &Path) -> PathBuf {
    app_data.join(FILE)
}

/// Stable updates: the newest full release (GitHub's "latest", which skips
/// pre-releases). Published by the release workflow on every stable tag.
pub const STABLE_ENDPOINT: &str =
    "https://github.com/Hellsparks/Foreman/releases/latest/download/latest.json";

/// Beta updates: the newest release of any kind, rc tags included. The release
/// workflow keeps `beta.json` on the `update-channels` branch pointing at it.
pub const BETA_ENDPOINT: &str =
    "https://raw.githubusercontent.com/Hellsparks/Foreman/update-channels/beta.json";

/// Which releases the updater offers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    /// Full releases only (default).
    #[default]
    Stable,
    /// Pre-releases (`v0.4.0-rc.1`) as well as full releases.
    Beta,
}

impl UpdateChannel {
    /// The updater manifest URL for this channel.
    pub fn endpoint(self) -> &'static str {
        match self {
            UpdateChannel::Stable => STABLE_ENDPOINT,
            UpdateChannel::Beta => BETA_ENDPOINT,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopSettings {
    /// "Keep running in the background". `None` = not chosen yet: on while the
    /// built-in server is in use, off otherwise.
    #[serde(default)]
    pub keep_running: Option<bool>,
    /// Settings → About → "Beta updates".
    #[serde(default)]
    pub update_channel: UpdateChannel,
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
            ..Default::default()
        };
        assert!(!off.keep_running(true));
        let on = DesktopSettings {
            keep_running: Some(true),
            ..Default::default()
        };
        assert!(on.keep_running(false));
    }

    #[test]
    fn update_channel_defaults_to_stable_and_maps_to_endpoints() {
        // Files written before the channel existed load as stable.
        let old: DesktopSettings = serde_json::from_str(r#"{"keep_running":true}"#).expect("parse");
        assert_eq!(old.update_channel, UpdateChannel::Stable);
        let beta: DesktopSettings =
            serde_json::from_str(r#"{"update_channel":"beta"}"#).expect("parse");
        assert_eq!(beta.update_channel, UpdateChannel::Beta);
        assert!(UpdateChannel::Stable
            .endpoint()
            .ends_with("/releases/latest/download/latest.json"));
        assert!(UpdateChannel::Beta.endpoint().ends_with("/beta.json"));
    }

    #[test]
    fn stable_endpoint_matches_tauri_conf() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        assert_eq!(
            conf["plugins"]["updater"]["endpoints"][0].as_str(),
            Some(STABLE_ENDPOINT)
        );
    }

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join(format!("foreman-settings-{}", std::process::id()));
        let file = path(&dir);
        assert_eq!(load(&file), DesktopSettings::default());
        let s = DesktopSettings {
            keep_running: Some(false),
            update_channel: UpdateChannel::Beta,
        };
        save(&file, &s).expect("save");
        assert_eq!(load(&file), s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
