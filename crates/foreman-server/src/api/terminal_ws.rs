//! WebSocket ⇄ SSH PTY bridge: `/ws/printers/{id}/terminal?cols=&rows=`
//!
//! Client → server (text JSON):
//! - `{"type":"input","data":"..."}` keystrokes / pasted text (raw, sent to the PTY)
//! - `{"type":"resize","cols":120,"rows":32}`
//! - `{"type":"macro","id":7}` run a shell macro (its lines are typed into the shell)
//!
//! Server → client: binary frames = PTY output; text JSON = `ready` / `error` / `exit`.
//!
//! Audit: each line the user submits (best effort, reconstructed from keystrokes) and
//! every macro run go to `audit_log`.

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Path, Query, State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use foreman_core::{repo, ssh_client};
use futures_util::{stream::SplitSink, SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tracing::{info, warn};

use crate::{api::ssh, state::AppState};

#[derive(Debug, Deserialize)]
pub struct TermQuery {
    cols: Option<u32>,
    rows: Option<u32>,
}

/// `GET /ws/printers/{id}/terminal`
pub async fn terminal_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<TermQuery>,
) -> impl IntoResponse {
    let cols = q.cols.unwrap_or(120).clamp(20, 500);
    let rows = q.rows.unwrap_or(32).clamp(5, 200);
    ws.on_upgrade(move |socket| handle_terminal(socket, state, id, cols, rows))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Input { data: String },
    Resize { cols: u32, rows: u32 },
    Macro { id: i64 },
}

// ---------------------------------------------------------------------------
// Best-effort command-line reconstruction for the audit log
// ---------------------------------------------------------------------------

/// Rebuilds submitted lines from raw terminal input. Handles backspace, Ctrl-C/Ctrl-U and
/// skips escape sequences (arrows, bracketed-paste markers). Tab completion and history
/// recall can't be seen from input alone, so the result is best effort.
#[derive(Default)]
pub(crate) struct LineAudit {
    buf: String,
    esc: EscState,
}

#[derive(Default, PartialEq)]
enum EscState {
    #[default]
    None,
    Esc,
    Csi,
}

impl LineAudit {
    pub(crate) fn feed(&mut self, input: &str) -> Vec<String> {
        let mut lines = Vec::new();
        for ch in input.chars() {
            match self.esc {
                EscState::Esc => {
                    self.esc = if ch == '[' || ch == 'O' {
                        EscState::Csi
                    } else {
                        EscState::None
                    };
                    continue;
                }
                EscState::Csi => {
                    if ('\u{40}'..='\u{7e}').contains(&ch) {
                        self.esc = EscState::None;
                    }
                    continue;
                }
                EscState::None => {}
            }
            match ch {
                '\u{1b}' => self.esc = EscState::Esc,
                '\r' | '\n' => {
                    let line = self.buf.trim().to_owned();
                    self.buf.clear();
                    if !line.is_empty() {
                        lines.push(line);
                    }
                }
                '\u{7f}' | '\u{8}' => {
                    self.buf.pop();
                }
                '\u{3}' | '\u{15}' => self.buf.clear(),
                '\t' => self.buf.push(' '),
                c if c.is_control() => {}
                c => {
                    if self.buf.len() < 4096 {
                        self.buf.push(c);
                    }
                }
            }
        }
        lines
    }
}

// ---------------------------------------------------------------------------
// Socket handler
// ---------------------------------------------------------------------------

async fn send_json(sender: &mut SplitSink<WebSocket, Message>, v: serde_json::Value) {
    let _ = sender.send(Message::Text(v.to_string().into())).await;
}

async fn audit(state: &AppState, printer_id: i64, user: &str, cmd: &str) {
    if let Err(e) =
        repo::audit_log::insert(&state.foreman_db.pool, Some(printer_id), Some(user), cmd).await
    {
        warn!(printer_id, "audit_log insert failed: {e}");
    }
}

async fn handle_terminal(
    socket: WebSocket,
    state: AppState,
    printer_id: i64,
    cols: u32,
    rows: u32,
) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    let session = match ssh::open_session(&state, printer_id).await {
        Ok(s) => s,
        Err(e) => {
            send_json(
                &mut ws_tx,
                json!({ "type": "error", "message": e.message, "code": e.code }),
            )
            .await;
            return;
        }
    };
    let user = session.cfg.ssh_user.clone();
    let host = session.host.clone();

    let channel = match ssh_client::open_pty_shell(&session.conn, cols, rows).await {
        Ok(c) => c,
        Err(e) => {
            send_json(
                &mut ws_tx,
                json!({ "type": "error", "message": e.to_string() }),
            )
            .await;
            session.conn.disconnect().await;
            return;
        }
    };
    let (mut ch_rx, ch_tx) = channel.split();

    info!(printer_id, %user, %host, "terminal session opened");
    send_json(
        &mut ws_tx,
        json!({ "type": "ready", "user": user, "host": host, "port": session.cfg.ssh_port }),
    )
    .await;

    let mut lines = LineAudit::default();

    loop {
        tokio::select! {
            msg = ch_rx.wait() => match msg {
                Some(russh::ChannelMsg::Data { data })
                | Some(russh::ChannelMsg::ExtendedData { data, .. }) => {
                    if ws_tx.send(Message::Binary(data.to_vec().into())).await.is_err() {
                        break;
                    }
                }
                Some(russh::ChannelMsg::ExitStatus { exit_status }) => {
                    send_json(&mut ws_tx, json!({ "type": "exit", "code": exit_status })).await;
                }
                Some(russh::ChannelMsg::Close) | None => {
                    send_json(&mut ws_tx, json!({ "type": "exit" })).await;
                    break;
                }
                Some(_) => {}
            },
            msg = ws_rx.next() => match msg {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientMsg>(&text) {
                    Ok(ClientMsg::Input { data }) => {
                        for line in lines.feed(&data) {
                            audit(&state, printer_id, &user, &format!("terminal: {line}")).await;
                        }
                        if ch_tx.data(data.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    Ok(ClientMsg::Resize { cols, rows }) => {
                        let _ = ch_tx
                            .window_change(cols.clamp(20, 500), rows.clamp(5, 200), 0, 0)
                            .await;
                    }
                    Ok(ClientMsg::Macro { id }) => match ssh::macro_for_run(&state, printer_id, id).await {
                        Ok(m) => {
                            audit(&state, printer_id, &user, &format!("macro run: {}", m.name)).await;
                            let mut body = String::new();
                            for line in m.commands.lines().filter(|l| !l.trim().is_empty()) {
                                audit(&state, printer_id, &user, &format!("macro '{}': {}", m.name, line.trim())).await;
                                body.push_str(line.trim_end());
                                body.push('\r');
                            }
                            if ch_tx.data(body.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            send_json(&mut ws_tx, json!({ "type": "error", "message": e.message })).await;
                        }
                    },
                    Err(e) => warn!(printer_id, "terminal: bad client message: {e}"),
                },
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
        }
    }

    info!(printer_id, "terminal session closed");
    let _ = ch_tx.eof().await;
    session.conn.disconnect().await;
}

#[cfg(test)]
mod tests {
    use super::LineAudit;

    #[test]
    fn simple_lines() {
        let mut a = LineAudit::default();
        assert!(a.feed("upt").is_empty());
        assert_eq!(a.feed("ime\r"), vec!["uptime"]);
        assert_eq!(a.feed("ls\rpwd\r"), vec!["ls", "pwd"]);
    }

    #[test]
    fn backspace_ctrl_and_escapes() {
        let mut a = LineAudit::default();
        assert_eq!(a.feed("lss\u{7f}\r"), vec!["ls"]);
        assert!(a.feed("rm -rf x\u{3}").is_empty());
        assert!(a.feed("\r").is_empty());
        // Arrow keys and bracketed-paste markers are dropped.
        assert_eq!(
            a.feed("\u{1b}[Aup\u{1b}[200~time\u{1b}[201~\r"),
            vec!["uptime"]
        );
        assert_eq!(a.feed("\u{1b}OAls\r"), vec!["ls"]);
    }
}
