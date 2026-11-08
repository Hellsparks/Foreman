//! Event bus types for `/ws/events`.
//!
//! All events are broadcast on a `tokio::broadcast::Sender<ServerEvent>`.
//! Clients subscribe with a JSON `{"topics": [...], "printer_ids": [...]}` message;
//! the server uses [`SubscriptionFilter`] to decide which events to forward.
//!
//! Topic names:
//! - `printer.status`     — live printer state/temps/progress (Phase 2.3+)
//! - `printer.connection` — path switch, offline, online
//! - `attention.added`    — new attention item
//! - `attention.updated`  — attention item body/actions changed
//! - `attention.cleared`  — attention item dismissed
//! - `job`                — job start/end/cancel (stub until Phase 3.6)
//! - `transfer`           — file transfer progress (stub until Phase 3.2)
//! - `device`             — device discovery/change (stub until Phase 4)
//!
//! Heartbeat has no topic and is always forwarded regardless of subscription.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ---------------------------------------------------------------------------
// Stub payload types (filled in by later phases)
// ---------------------------------------------------------------------------

/// Printer state (stub — will grow in Phase 2.3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PrinterState {
    #[default]
    Idle,
    Printing,
    Paused,
    Error,
    Offline,
}

/// Temperature reading for one heater.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TempReading {
    pub actual: f64,
    pub target: f64,
}

/// Fan speed readings (0–100 %).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FanSpeeds {
    pub part_fan: Option<u8>,
    pub aux_fan: Option<u8>,
    pub chamber_fan: Option<u8>,
    pub heatbreak_fan: Option<u8>,
}

/// A single AMS / AMS-Lite tray.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AmsTray {
    /// Global slot index (ams_id * 4 + tray_id), or 254 for external spool.
    pub slot: u32,
    /// Hex colour string "RRGGBB" (alpha stripped).
    pub color: Option<String>,
    /// Material type, e.g. "PLA", "PETG".
    pub material: Option<String>,
    /// Remaining filament 0–100 %.
    pub remaining: Option<u8>,
}

/// A raw HMS (Health Management System) code from the printer.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HmsCode {
    pub attr: u32,
    pub code: u32,
}

/// A printer status snapshot with temps, progress, and job info.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrinterStatus {
    pub state: PrinterState,
    /// Nozzle temperature (extruder 0).
    pub nozzle: Option<TempReading>,
    /// Heated bed temperature.
    pub bed: Option<TempReading>,
    /// Print progress 0.0–1.0.
    pub progress: Option<f64>,
    /// Currently printing filename (if any).
    pub filename: Option<String>,
    /// Print duration in seconds so far.
    pub print_duration: Option<f64>,
    /// Display message from the printer (M117).
    pub display_message: Option<String>,
    /// Current layer number (Bambu).
    pub layer: Option<u32>,
    /// Total layers in print (Bambu).
    pub total_layers: Option<u32>,
    /// Estimated remaining time in seconds (Bambu).
    pub remaining_time: Option<u32>,
    /// Fan speeds (Bambu).
    pub fans: Option<FanSpeeds>,
    /// AMS / AMS-Lite trays loaded (Bambu).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ams_trays: Vec<AmsTray>,
    /// Active HMS error codes (Bambu).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hms_codes: Vec<HmsCode>,
}

/// What triggered a connection event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionEventKind {
    PathSwitched,
    Offline,
    Online,
}

/// A printer connection change.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ConnectionEvent {
    pub kind: ConnectionEventKind,
    /// Name of the now-active path (e.g. `"Tailscale"`), if applicable.
    pub active_path: Option<String>,
}

/// Lightweight attention item carried in events (full record lives in the DB).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AttentionItemEvent {
    pub id: i64,
    pub printer_id: Option<i64>,
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
}

// ---------------------------------------------------------------------------
// ServerEvent
// ---------------------------------------------------------------------------

/// A single event broadcast on `/ws/events`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// Sent every 10 s so clients can detect a dropped connection.
    Heartbeat {
        timestamp: DateTime<Utc>,
    },

    // ----- printer events -----
    PrinterStatus {
        printer_id: i64,
        status: PrinterStatus,
    },
    PrinterConnection {
        printer_id: i64,
        event: ConnectionEvent,
    },

    // ----- attention events -----
    AttentionAdded {
        item: AttentionItemEvent,
    },
    AttentionUpdated {
        item: AttentionItemEvent,
    },
    AttentionCleared {
        item_id: i64,
        printer_id: Option<i64>,
    },

    // ----- stubs (data filled in later phases) -----
    /// Job lifecycle event (Phase 3.6).
    Job {
        printer_id: Option<i64>,
        #[ts(type = "unknown")]
        payload: serde_json::Value,
    },
    /// File transfer progress (Phase 3.2).
    Transfer {
        #[ts(type = "unknown")]
        payload: serde_json::Value,
    },
    /// Device discovery / change (Phase 4).
    Device {
        printer_id: Option<i64>,
        #[ts(type = "unknown")]
        payload: serde_json::Value,
    },
}

impl ServerEvent {
    pub fn heartbeat() -> Self {
        ServerEvent::Heartbeat {
            timestamp: Utc::now(),
        }
    }

    /// The topic string for this event, or `None` for heartbeat (always forwarded).
    pub fn topic(&self) -> Option<&'static str> {
        match self {
            ServerEvent::Heartbeat { .. } => None,
            ServerEvent::PrinterStatus { .. } => Some("printer.status"),
            ServerEvent::PrinterConnection { .. } => Some("printer.connection"),
            ServerEvent::AttentionAdded { .. } => Some("attention.added"),
            ServerEvent::AttentionUpdated { .. } => Some("attention.updated"),
            ServerEvent::AttentionCleared { .. } => Some("attention.cleared"),
            ServerEvent::Job { .. } => Some("job"),
            ServerEvent::Transfer { .. } => Some("transfer"),
            ServerEvent::Device { .. } => Some("device"),
        }
    }

    /// The printer this event concerns, if any.
    pub fn printer_id(&self) -> Option<i64> {
        match self {
            ServerEvent::PrinterStatus { printer_id, .. } => Some(*printer_id),
            ServerEvent::PrinterConnection { printer_id, .. } => Some(*printer_id),
            ServerEvent::AttentionAdded { item } => item.printer_id,
            ServerEvent::AttentionUpdated { item } => item.printer_id,
            ServerEvent::AttentionCleared { printer_id, .. } => *printer_id,
            ServerEvent::Job { printer_id, .. } => *printer_id,
            ServerEvent::Device { printer_id, .. } => *printer_id,
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// SubscriptionFilter
// ---------------------------------------------------------------------------

/// Client-side subscription state used to filter events before forwarding.
///
/// An empty `topics` list means **all topics** are accepted.
/// An empty `printer_ids` list means **all printers** are accepted.
#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct SubscriptionFilter {
    /// Topic allowlist.  Empty = accept everything.
    #[serde(default)]
    pub topics: Vec<String>,
    /// Printer-id allowlist.  Empty = accept all printers.
    #[serde(default)]
    pub printer_ids: Vec<i64>,
}

impl SubscriptionFilter {
    /// Return `true` if `event` should be forwarded to this subscriber.
    pub fn matches(&self, event: &ServerEvent) -> bool {
        // Heartbeat always passes through regardless of subscription.
        let topic = match event.topic() {
            None => return true,
            Some(t) => t,
        };

        // Topic check: empty list = wildcard.
        let topic_ok = self.topics.is_empty() || self.topics.iter().any(|t| t.as_str() == topic);

        // Printer-id check: empty list = wildcard; events with no printer always pass.
        let printer_ok = self.printer_ids.is_empty()
            || match event.printer_id() {
                None => true,
                Some(id) => self.printer_ids.contains(&id),
            };

        topic_ok && printer_ok
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    fn status_event(printer_id: i64) -> ServerEvent {
        ServerEvent::PrinterStatus {
            printer_id,
            status: PrinterStatus {
                state: PrinterState::Idle,
                nozzle: None,
                bed: None,
                progress: None,
                filename: None,
                print_duration: None,
                display_message: None,
                layer: None,
                total_layers: None,
                remaining_time: None,
                fans: None,
                ams_trays: Vec::new(),
                hms_codes: Vec::new(),
            },
        }
    }

    fn attention_event() -> ServerEvent {
        ServerEvent::AttentionAdded {
            item: AttentionItemEvent {
                id: 1,
                printer_id: Some(2),
                kind: "print_finished".to_owned(),
                title: "Plate ready".to_owned(),
                body: None,
            },
        }
    }

    // ----- SubscriptionFilter unit tests -----

    #[test]
    fn heartbeat_always_passes() {
        let filter = SubscriptionFilter {
            topics: vec!["printer.status".to_owned()],
            printer_ids: vec![99],
        };
        assert!(filter.matches(&ServerEvent::heartbeat()));
    }

    #[test]
    fn empty_filter_passes_everything() {
        let filter = SubscriptionFilter::default();
        assert!(filter.matches(&status_event(1)));
        assert!(filter.matches(&ServerEvent::heartbeat()));
        assert!(filter.matches(&attention_event()));
    }

    #[test]
    fn topic_filter_blocks_other_topics() {
        let filter = SubscriptionFilter {
            topics: vec!["printer.status".to_owned()],
            printer_ids: vec![],
        };
        assert!(filter.matches(&status_event(1)));
        assert!(!filter.matches(&attention_event()));
    }

    #[test]
    fn printer_id_filter_blocks_other_printers() {
        let filter = SubscriptionFilter {
            topics: vec![],
            printer_ids: vec![1],
        };
        assert!(filter.matches(&status_event(1)));
        assert!(!filter.matches(&status_event(2)));
    }

    #[test]
    fn events_without_printer_pass_printer_filter() {
        // A Transfer event has no printer_id — should pass any printer_id filter.
        let filter = SubscriptionFilter {
            topics: vec![],
            printer_ids: vec![42],
        };
        let ev = ServerEvent::Transfer {
            payload: serde_json::json!({}),
        };
        assert!(filter.matches(&ev));
    }

    // ----- Broadcast channel integration test -----
    // "publish → subscribed client receives, unsubscribed client doesn't"

    #[tokio::test]
    async fn subscribed_receives_unsubscribed_does_not() {
        let (tx, _) = broadcast::channel::<ServerEvent>(16);

        // subscribed: wants "printer.status" for printer 1
        let subscribed_filter = SubscriptionFilter {
            topics: vec!["printer.status".to_owned()],
            printer_ids: vec![1],
        };
        let mut subscribed_rx = tx.subscribe();

        // unsubscribed: different topic (attention only)
        let unsubscribed_filter = SubscriptionFilter {
            topics: vec!["attention.added".to_owned()],
            printer_ids: vec![],
        };
        let mut unsubscribed_rx = tx.subscribe();

        // Publish a printer.status event for printer 1.
        let ev = status_event(1);
        tx.send(ev.clone()).expect("send");

        // Both receivers get the raw event from the channel…
        let raw1 = subscribed_rx.recv().await.expect("recv subscribed");
        let raw2 = unsubscribed_rx.recv().await.expect("recv unsubscribed");

        // …but only the subscribed filter passes it through.
        assert!(
            subscribed_filter.matches(&raw1),
            "subscribed filter should accept printer.status for printer 1"
        );
        assert!(
            !unsubscribed_filter.matches(&raw2),
            "unsubscribed filter should reject printer.status (wants attention.added)"
        );
    }
}
