use axum::Json;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Git commit this binary was built from (set by `build.rs`; `"unknown"` outside git).
pub const GIT_COMMIT: &str = env!("FOREMAN_GIT_COMMIT");

/// Build time as Unix seconds (set by `build.rs`).
const BUILD_EPOCH: &str = env!("FOREMAN_BUILD_EPOCH");

/// Build time as RFC 3339 UTC, e.g. `2026-11-11T02:30:00Z`.
pub fn built_at() -> String {
    BUILD_EPOCH
        .parse::<i64>()
        .ok()
        .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| "unknown".into())
}

/// Response body for `GET /api/health`.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    /// Git commit the running binary was built from.
    pub commit: String,
    /// When the binary was built (RFC 3339 UTC).
    pub built_at: String,
}

/// `GET /api/health`
///
/// Returns `{"status":"ok","version":"<crate version>","commit":"<git sha>","built_at":"<rfc3339>"}`.
/// Used by the frontend WS hook, healthcheck probes and `scripts/dev-check.sh`.
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        commit: GIT_COMMIT.into(),
        built_at: built_at(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_returns_ok() {
        let Json(resp) = health().await;
        assert_eq!(resp.status, "ok");
        assert!(!resp.version.is_empty());
        assert!(!resp.commit.is_empty());
        assert!(resp.built_at.ends_with('Z'), "{}", resp.built_at);
    }
}
