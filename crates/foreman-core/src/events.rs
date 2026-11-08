use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A single event broadcast on `/ws/events`.
///
/// At scaffold stage this only carries a heartbeat. Future event kinds
/// (printer state, job update, attention item, etc.) will be added as
/// the Phase 1–3 work lands.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// Sent every 10 s so clients can detect a dropped connection.
    Heartbeat { timestamp: DateTime<Utc> },
}

impl ServerEvent {
    pub fn heartbeat() -> Self {
        ServerEvent::Heartbeat {
            timestamp: Utc::now(),
        }
    }
}
