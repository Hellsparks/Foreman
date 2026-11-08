//! WebSocket event bus for entity changes (vendor/filament/spool) and tag scan relay.
//!
//! Entity events use a `tokio::sync::broadcast` channel.  Every CRUD handler
//! and spool-use/measure handler publishes a [`BroadcastEvent`] after a
//! successful DB write; WebSocket handler tasks subscribe and filter by
//! resource + optional resource-id before forwarding to the client.
//!
//! Tag scan events use a parallel broadcast channel ([`ScanBroadcastEvent`])
//! keyed by reader-id; the WebSocket handlers for `/tag/scan` and
//! `/tag/scan/{reader_id}` subscribe and filter accordingly.
//!
//! The [`ScanRelay`] struct is an in-memory reader registry with a 3-second
//! deduplication window matching upstream Spoolman's `scanrelay.py`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;

// ---------------------------------------------------------------------------
// Entity events
// ---------------------------------------------------------------------------

/// A single entity-change event broadcast to all WebSocket subscribers.
#[derive(Clone, Debug)]
pub struct BroadcastEvent {
    /// One of `"vendor"`, `"filament"`, `"spool"`.
    pub resource: &'static str,
    /// Database ID of the changed entity.
    pub resource_id: i64,
    /// Pre-serialised JSON for the entire WsEvent envelope
    /// (`{type, resource, date, payload}`).  Pre-serialising once avoids
    /// re-encoding for each subscriber.
    pub json: String,
}

impl BroadcastEvent {
    pub fn new<T: Serialize>(
        event_type: &'static str,
        resource: &'static str,
        resource_id: i64,
        payload: &T,
    ) -> Self {
        // Build the event envelope.  The inner `payload` field is serialised via
        // the struct's own `Serialize` impl, which respects `skip_serializing_if`
        // on each field — matching upstream's `exclude_none=True` behaviour.
        let date = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        let event = WsEventEnvelope {
            event_type,
            resource,
            date,
            payload,
        };
        let json = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_owned());
        BroadcastEvent {
            resource,
            resource_id,
            json,
        }
    }
}

#[derive(Serialize)]
struct WsEventEnvelope<'a, T: Serialize> {
    #[serde(rename = "type")]
    event_type: &'static str,
    resource: &'static str,
    date: String,
    payload: &'a T,
}

/// Convenience helper: build and send a [`BroadcastEvent`].
/// A send error (no active subscribers) is intentionally ignored.
pub fn broadcast<T: Serialize>(
    tx: &tokio::sync::broadcast::Sender<BroadcastEvent>,
    event_type: &'static str,
    resource: &'static str,
    resource_id: i64,
    payload: &T,
) {
    let evt = BroadcastEvent::new(event_type, resource, resource_id, payload);
    let _ = tx.send(evt);
}

// ---------------------------------------------------------------------------
// Scan events
// ---------------------------------------------------------------------------

/// A tag scan event broadcast on the scan relay WebSocket channel.
#[derive(Clone, Debug)]
pub struct ScanBroadcastEvent {
    /// The reader that reported the scan.
    pub reader_id: String,
    /// Pre-serialised JSON for the entire scan event envelope.
    pub json: String,
}

// ---------------------------------------------------------------------------
// Scan relay — in-memory reader registry + 3-second dedup window
// ---------------------------------------------------------------------------

/// In-memory registry of recently seen tag readers with deduplication.
///
/// Cloning is cheap — the registry is behind a shared `Arc`.
#[derive(Clone, Default)]
pub struct ScanRelay {
    inner: Arc<ScanRelayInner>,
}

impl std::fmt::Debug for ScanRelay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScanRelay").finish_non_exhaustive()
    }
}

#[derive(Default)]
struct ScanRelayInner {
    readers: Mutex<HashMap<String, ReaderEntry>>,
    dedup: Mutex<HashMap<String, DedupEntry>>,
}

#[derive(Clone)]
struct ReaderEntry {
    name: Option<String>,
    last_seen: String,
    last_uid: Option<String>,
}

struct DedupEntry {
    /// The match result at the time of the last broadcast (target_type + id).
    matched: Option<(String, i64)>,
    at: Instant,
}

/// Info about one recently-seen reader, returned by [`ScanRelay::readers`].
pub struct ReaderInfo {
    pub reader_id: String,
    pub name: Option<String>,
    pub last_seen: String,
    pub last_uid: Option<String>,
}

impl ScanRelay {
    /// Register a scan and decide whether to broadcast it.
    ///
    /// Returns `true` when the caller should broadcast (new UID, different
    /// match result, or more than 3 seconds since the last broadcast of the
    /// same UID from the same reader with the same match).
    pub fn register(
        &self,
        reader_id: &str,
        name: Option<&str>,
        uid: &str,
        matched: Option<(&str, i64)>,
    ) -> bool {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();

        // Update reader registry.
        {
            let mut readers = self.inner.readers.lock().unwrap_or_else(|e| e.into_inner());
            readers.insert(
                reader_id.to_owned(),
                ReaderEntry {
                    name: name.map(str::to_owned),
                    last_seen: now,
                    last_uid: Some(uid.to_owned()),
                },
            );
        }

        // Dedup: same reader + UID + match within 3 seconds → skip broadcast.
        let key = format!("{reader_id}\x00{uid}");
        let matched_pair = matched.map(|(t, id)| (t.to_owned(), id));

        let mut dedup = self.inner.dedup.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = dedup.get(&key) {
            if entry.at.elapsed().as_secs() < 3 && entry.matched == matched_pair {
                return false;
            }
        }
        dedup.insert(
            key,
            DedupEntry {
                matched: matched_pair,
                at: Instant::now(),
            },
        );
        true
    }

    /// Return all recently seen readers.
    pub fn readers(&self) -> Vec<ReaderInfo> {
        let readers = self.inner.readers.lock().unwrap_or_else(|e| e.into_inner());
        readers
            .iter()
            .map(|(id, e)| ReaderInfo {
                reader_id: id.clone(),
                name: e.name.clone(),
                last_seen: e.last_seen.clone(),
                last_uid: e.last_uid.clone(),
            })
            .collect()
    }
}
