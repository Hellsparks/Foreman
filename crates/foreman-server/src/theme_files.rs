//! Theme assets served from the data directory (Phase 6.1).
//!
//! - `GET /theme/fonts/{name}` → `<data_dir>/fonts/<name>` (`.woff2` only)
//! - `GET /theme/custom.css`   → `<data_dir>/custom.css` (unsupported; 404 when absent)
//!
//! Both live outside `/api/`, so they are public like the frontend itself:
//! `@font-face` and `<link>` requests carry no bearer token. Nothing secret is
//! served — only files the owner put there for styling.

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::state::AppState;

/// Largest font file served (a .woff2 is typically 20–300 KB).
const MAX_FONT_BYTES: u64 = 8 * 1024 * 1024;
/// Largest custom.css served.
const MAX_CSS_BYTES: u64 = 1024 * 1024;

/// A font file name is a plain `.woff2` name: no directories, no dot-files.
pub fn valid_font_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".woff2") else {
        return false;
    };
    !stem.is_empty()
        && name.len() <= 128
        && !stem.starts_with('.')
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !stem.contains("..")
}

async fn read_capped(path: &std::path::Path, cap: u64) -> Option<Vec<u8>> {
    let meta = tokio::fs::metadata(path).await.ok()?;
    if !meta.is_file() || meta.len() > cap {
        return None;
    }
    tokio::fs::read(path).await.ok()
}

pub async fn font(State(state): State<AppState>, Path(name): Path<String>) -> Response {
    if !valid_font_name(&name) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = state.data_dir.join("fonts").join(&name);
    match read_capped(&path, MAX_FONT_BYTES).await {
        Some(bytes) => (
            [
                (header::CONTENT_TYPE, "font/woff2"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn custom_css(State(state): State<AppState>) -> Response {
    let path = state.data_dir.join("custom.css");
    match read_capped(&path, MAX_CSS_BYTES).await {
        Some(bytes) => (
            [
                (header::CONTENT_TYPE, "text/css; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
