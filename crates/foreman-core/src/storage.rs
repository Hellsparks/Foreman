//! A printer's G-code storage as a Library location (task 3.2).
//!
//! Paths are relative to the storage root (`/`-separated, `""` = root). Klipper printers
//! implement this over Moonraker's file API; printer config stays on the SSH Files tab.

use std::{
    path::Path,
    sync::{atomic::AtomicU64, Arc},
};

use async_trait::async_trait;
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::AppError;

/// Facts the printer host already parsed (Moonraker metadata).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RemoteFileMeta {
    pub slicer: Option<String>,
    pub slicer_version: Option<String>,
    /// Seconds.
    pub estimated_time: Option<f64>,
    pub filament_type: Option<String>,
    pub filament_name: Option<String>,
    pub filament_weight_total: Option<f64>,
    pub nozzle_diameter: Option<f64>,
    pub layer_height: Option<f64>,
    /// `#RRGGBB` per tool.
    pub filament_colors: Vec<String>,
    /// Largest thumbnail, relative to the storage root.
    pub thumb_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StorageEntry {
    pub name: String,
    /// Relative to the storage root.
    pub path: String,
    pub is_dir: bool,
    #[ts(type = "number")]
    pub size: u64,
    /// Unix seconds.
    pub modified: f64,
    pub meta: Option<RemoteFileMeta>,
}

pub type ByteStream = BoxStream<'static, Result<Vec<u8>, AppError>>;

#[async_trait]
pub trait PrinterStorage: Send + Sync {
    /// Entries directly in `path` (hidden entries such as `.thumbs` left out).
    async fn list_dir(&self, path: &str) -> Result<Vec<StorageEntry>, AppError>;
    /// Stream `src` to `dest` (parent folders created); `progress` counts bytes sent.
    async fn upload(
        &self,
        dest: &str,
        src: &Path,
        progress: Arc<AtomicU64>,
    ) -> Result<(), AppError>;
    /// Stream `path` into the local file `dest`; `progress` counts bytes received.
    async fn download(
        &self,
        path: &str,
        dest: &Path,
        progress: Arc<AtomicU64>,
    ) -> Result<u64, AppError>;
    /// Size (if known) and body of `path`, for proxying to a browser.
    async fn read_stream(&self, path: &str) -> Result<(Option<u64>, ByteStream), AppError>;
    async fn delete_file(&self, path: &str) -> Result<(), AppError>;
    /// Delete a folder and everything in it.
    async fn delete_dir(&self, path: &str) -> Result<(), AppError>;
    async fn mkdir(&self, path: &str) -> Result<(), AppError>;
    /// Server-side move/rename. Overwrites an existing file at `to`.
    async fn move_entry(&self, from: &str, to: &str) -> Result<(), AppError>;
    /// Server-side copy. Overwrites an existing file at `to`.
    async fn copy_entry(&self, from: &str, to: &str) -> Result<(), AppError>;
    /// The file loaded for a print that is running or paused, if any.
    async fn active_file(&self) -> Result<Option<String>, AppError>;
}

/// Normalise a storage-relative path: no `.`/`..`, no empty or control-character segments.
pub fn clean_path(path: &str) -> Result<String, AppError> {
    let mut parts = Vec::new();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        if seg == "." || seg == ".." || seg.chars().any(|c| c.is_control() || c == '\\') {
            return Err(AppError::InvalidInput(format!("invalid path: {path:?}")));
        }
        parts.push(seg);
    }
    Ok(parts.join("/"))
}

pub fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

pub fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}

pub fn name_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(_, n)| n).unwrap_or(path)
}

/// True if `path` is `active` or a folder containing it.
pub fn covers(path: &str, active: &str) -> bool {
    path.is_empty()
        || active == path
        || active
            .strip_prefix(path)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_and_cover() {
        assert_eq!(clean_path("/a//b/").expect("ok"), "a/b");
        assert!(clean_path("a/../b").is_err());
        assert!(covers("jobs", "jobs/a.gcode"));
        assert!(covers("jobs/a.gcode", "jobs/a.gcode"));
        assert!(!covers("job", "jobs/a.gcode"));
        assert!(covers("", "a.gcode"));
    }
}
