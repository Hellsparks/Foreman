use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use foreman_core::events::ServerEvent;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::{broadcast, oneshot};
use tracing::{debug, info, warn};

/// Shared state injected into the WebSocket handler.
#[derive(Clone)]
pub struct WsState {
    pub tx: Arc<broadcast::Sender<ServerEvent>>,
}

/// `GET /ws/events`
///
/// Upgrades the connection to a WebSocket. The client receives every
/// [`ServerEvent`] broadcast on the application event bus, starting with
/// any events emitted after the connection is established.
///
/// At scaffold stage the only event is [`ServerEvent::Heartbeat`] every 10 s.
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<WsState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: WsState) {
    let mut rx = state.tx.subscribe();
    let (mut sender, mut receiver) = socket.split();

    // Channel to signal that the client has closed the connection.
    let (close_tx, mut close_rx) = oneshot::channel::<()>();

    // Drain incoming messages from the client. We don't use them yet, but we
    // must read the stream to keep the connection alive and detect close frames.
    tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
        // Signal that the client side is done.
        let _ = close_tx.send(());
    });

    info!("ws/events: client connected");

    loop {
        tokio::select! {
            // Broadcast bus received an event — forward it to the client.
            event = rx.recv() => {
                match event {
                    Ok(ev) => {
                        match serde_json::to_string(&ev) {
                            Ok(json) => {
                                if sender.send(Message::Text(json.into())).await.is_err() {
                                    // Client disconnected mid-send.
                                    break;
                                }
                            }
                            Err(err) => {
                                warn!("ws event serialise error: {err}");
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("ws/events: client lagged by {n} messages");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        // Event bus shut down — server is exiting.
                        break;
                    }
                }
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
