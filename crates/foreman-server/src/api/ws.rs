//! `/ws/events` WebSocket handler.
//!
//! The client may send a JSON subscribe message at any time:
//! ```json
//! { "topics": ["printer.status", "attention.added"], "printer_ids": [1, 2] }
//! ```
//! An empty `topics` array (or absent field) means **all topics**.
//! An empty `printer_ids` array (or absent field) means **all printers**.
//!
//! The server filters every broadcast event through the client's current
//! [`SubscriptionFilter`] before forwarding. Heartbeats are always forwarded.

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use foreman_core::events::{ServerEvent, SubscriptionFilter};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, info, warn};

use crate::state::AppState;

/// `GET /ws/events`
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state.bus_tx))
}

async fn handle_socket(socket: WebSocket, bus_tx: Arc<broadcast::Sender<ServerEvent>>) {
    let mut rx = bus_tx.subscribe();
    let (mut sender, mut receiver) = socket.split();

    // Channel to pass subscribe messages from the reader task to the main loop.
    let (filter_tx, mut filter_rx) = mpsc::channel::<SubscriptionFilter>(8);
    // Signal that the client has closed the connection.
    let (close_tx, mut close_rx) = oneshot::channel::<()>();

    // Reader task: drain incoming client messages.
    // Parse any JSON subscribe messages and forward the new filter.
    tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    match serde_json::from_str::<SubscriptionFilter>(&text) {
                        Ok(filter) => {
                            if filter_tx.send(filter).await.is_err() {
                                break; // main loop exited
                            }
                        }
                        Err(e) => {
                            debug!("ws/events: ignoring unparseable message: {e}");
                        }
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
        let _ = close_tx.send(());
    });

    info!("ws/events: client connected");

    // Current subscription state — default is accept everything.
    let mut filter = SubscriptionFilter::default();

    loop {
        tokio::select! {
            // Broadcast bus received an event — filter and forward.
            event = rx.recv() => {
                match event {
                    Ok(ev) => {
                        if filter.matches(&ev) {
                            match serde_json::to_string(&ev) {
                                Ok(json) => {
                                    if sender.send(Message::Text(json.into())).await.is_err() {
                                        break;
                                    }
                                }
                                Err(err) => warn!("ws event serialise error: {err}"),
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("ws/events: client lagged by {n} messages");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            // Client sent a new subscribe message.
            Some(new_filter) = filter_rx.recv() => {
                debug!(?new_filter, "ws/events: subscription updated");
                filter = new_filter;
            }

            // Client closed the connection.
            _ = &mut close_rx => {
                debug!("ws/events: client closed connection");
                break;
            }
        }
    }

    info!("ws/events: client disconnected");
}
