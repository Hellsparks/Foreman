use axum::Json;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Response body for `GET /api/health`.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

/// `GET /api/health`
///
/// Returns `{"status":"ok","version":"<crate version>"}`.
/// Used by the frontend WS hook and by healthcheck probes.
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
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
    }
}
