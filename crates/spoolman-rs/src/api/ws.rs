//! WebSocket handler helpers shared by entity and scan relay endpoints.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use futures_util::{SinkExt, StreamExt};
use std::convert::Infallible;
use tokio::sync::broadcast;

use crate::ws::{BroadcastEvent, ScanBroadcastEvent};

// ---------------------------------------------------------------------------
// Optional WebSocket upgrade extractor
// ---------------------------------------------------------------------------

/// Extracts a WebSocket upgrade from a request if present.
///
/// Returns `Some(ws)` for WS upgrade requests (`Upgrade: websocket` header)
/// and `None` for ordinary GET requests.  The `Infallible` rejection means
/// this extractor never fails, allowing REST handlers to share the same path
/// as their corresponding WebSocket subscription.
pub struct OptWs(pub Option<WebSocketUpgrade>);

impl<S: Send + Sync> FromRequestParts<S> for OptWs {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Infallible> {
        Ok(OptWs(
            WebSocketUpgrade::from_request_parts(parts, state)
                .await
                .ok(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Entity events
// ---------------------------------------------------------------------------

/// Forward entity events to a connected WebSocket client.
///
/// - `resource = ""` → all events (root subscription).
/// - `resource = "vendor"|"filament"|"spool"` → one collection.
/// - `id_filter = Some(id)` → only events for that specific entity.
///
/// Loop terminates on client close, disconnect, or broadcast channel close.
pub async fn handle_entity_ws(
    socket: WebSocket,
    mut rx: broadcast::Receiver<BroadcastEvent>,
    resource: &'static str,
    id_filter: Option<i64>,
) {
    let (mut sender, mut receiver) = socket.split();

    loop {
        tokio::select! {
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        if sender.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            evt = rx.recv() => {
                match evt {
                    Ok(evt) => {
                        let matches = if resource.is_empty() {
                            true
                        } else {
                            evt.resource == resource
                                && id_filter.is_none_or(|id| evt.resource_id == id)
                        };
                        if matches && sender.send(Message::Text(evt.json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Scan relay events
// ---------------------------------------------------------------------------

/// Forward tag scan events to a connected WebSocket client.
///
/// - `reader_id_filter = None` → all scan events.
/// - `reader_id_filter = Some(id)` → only events from that reader.
pub async fn handle_scan_ws(
    socket: WebSocket,
    mut rx: broadcast::Receiver<ScanBroadcastEvent>,
    reader_id_filter: Option<String>,
) {
    let (mut sender, mut receiver) = socket.split();

    loop {
        tokio::select! {
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        if sender.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            evt = rx.recv() => {
                match evt {
                    Ok(evt) => {
                        let matches = reader_id_filter
                            .as_deref()
                            .is_none_or(|rid| evt.reader_id == rid);
                        if matches && sender.send(Message::Text(evt.json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}
