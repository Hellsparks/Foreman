//! Per-printer pooled SSH + SFTP sessions.
//!
//! Files-tab requests reuse one authenticated session per printer instead of a fresh
//! handshake per call. Sessions close on request (`DELETE /ssh/session`), when SSH settings
//! change, or after the printer's `idle_timeout_secs` (reaped by [`SshPool::reap`]).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use foreman_core::{
    remote_fs::{Roots, SftpFs},
    repo,
    ssh_client::SshConnection,
};
use sqlx::SqlitePool;
use tracing::info;

pub struct PooledSftp {
    conn: SshConnection,
    pub fs: SftpFs,
    pub roots: Roots,
    pub user: String,
    pub host: String,
    pub port: i64,
    pub fingerprint: Option<String>,
    opened_at: Instant,
    last_used: Mutex<Instant>,
}

impl PooledSftp {
    pub fn new(
        conn: SshConnection,
        fs: SftpFs,
        roots: Roots,
        user: String,
        host: String,
        port: i64,
    ) -> Self {
        let now = Instant::now();
        Self {
            fingerprint: conn.received_fingerprint(),
            conn,
            fs,
            roots,
            user,
            host,
            port,
            opened_at: now,
            last_used: Mutex::new(now),
        }
    }

    fn alive(&self) -> bool {
        !self.conn.handle.is_closed()
    }

    fn touch(&self) {
        if let Ok(mut t) = self.last_used.lock() {
            *t = Instant::now();
        }
    }

    fn idle(&self) -> Duration {
        self.last_used
            .lock()
            .map(|t| t.elapsed())
            .unwrap_or_default()
    }

    async fn close(&self) {
        self.conn.disconnect().await;
    }
}

/// Snapshot of a pooled session for the UI.
pub struct SessionInfo {
    pub open_secs: u64,
    pub idle_secs: u64,
}

#[derive(Default)]
pub struct SshPool {
    sessions: tokio::sync::Mutex<HashMap<i64, Arc<PooledSftp>>>,
    /// Serialises opening so concurrent first requests share one session.
    pub open_lock: tokio::sync::Mutex<()>,
}

impl SshPool {
    /// A live session for the printer (marks it used), dropping dead ones.
    pub async fn get(&self, printer_id: i64) -> Option<Arc<PooledSftp>> {
        let mut map = self.sessions.lock().await;
        match map.get(&printer_id) {
            Some(s) if s.alive() => {
                s.touch();
                Some(Arc::clone(s))
            }
            Some(_) => {
                map.remove(&printer_id);
                None
            }
            None => None,
        }
    }

    pub async fn insert(&self, printer_id: i64, session: PooledSftp) -> Arc<PooledSftp> {
        let session = Arc::new(session);
        let old = self
            .sessions
            .lock()
            .await
            .insert(printer_id, Arc::clone(&session));
        if let Some(old) = old {
            old.close().await;
        }
        session
    }

    /// Close and forget the printer's session. Returns whether one was open.
    pub async fn close(&self, printer_id: i64) -> bool {
        let removed = self.sessions.lock().await.remove(&printer_id);
        match removed {
            Some(s) => {
                s.close().await;
                info!(printer_id, "SSH session closed");
                true
            }
            None => false,
        }
    }

    pub async fn info(&self, printer_id: i64) -> Option<SessionInfo> {
        let map = self.sessions.lock().await;
        map.get(&printer_id)
            .filter(|s| s.alive())
            .map(|s| SessionInfo {
                open_secs: s.opened_at.elapsed().as_secs(),
                idle_secs: s.idle().as_secs(),
            })
    }

    /// Close sessions idle longer than their printer's `idle_timeout_secs` (0 = never).
    pub async fn reap(&self, db: &SqlitePool) {
        let snapshot: Vec<(i64, Arc<PooledSftp>)> = self
            .sessions
            .lock()
            .await
            .iter()
            .map(|(k, v)| (*k, Arc::clone(v)))
            .collect();
        for (printer_id, s) in snapshot {
            let timeout = repo::printer_ssh::get(db, printer_id)
                .await
                .map(|c| c.idle_timeout_secs)
                .unwrap_or(0);
            let expired = timeout > 0 && s.idle() >= Duration::from_secs(timeout as u64);
            if expired || !s.alive() {
                if expired {
                    info!(printer_id, timeout, "closing idle SSH session");
                }
                self.close(printer_id).await;
            }
        }
    }
}
