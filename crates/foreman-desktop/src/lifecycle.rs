//! The embedded server (Mode A): start once, keep running while the window is
//! hidden, stop cleanly on quit.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use foreman_server::{config::ServerConfig, LocalHandle};
use tauri::async_runtime::{self, JoinHandle};
use tokio::sync::{oneshot, Mutex};

struct Running {
    handle: LocalHandle,
    task: JoinHandle<anyhow::Result<()>>,
}

/// Managed state: the embedded server, if started.
#[derive(Default)]
pub struct Local {
    slot: Mutex<Option<Running>>,
    /// Mirrors "started and not stopped", for sync callers (window events).
    in_use: AtomicBool,
}

impl Local {
    /// Start the server with `cfg` (once); later calls return the running handle.
    pub async fn start(&self, mut cfg: ServerConfig) -> Result<LocalHandle, String> {
        let mut slot = self.slot.lock().await;
        if let Some(r) = slot.as_ref() {
            if !r.task.inner().is_finished() {
                return Ok(r.handle.clone());
            }
        }
        cfg.desktop_mode = true;
        let (tx, rx) = oneshot::channel();
        let mut task = async_runtime::spawn(foreman_server::serve_with(cfg, Some(tx)));
        let handle = tokio::select! {
            ready = rx => ready.map_err(|_| "the built-in server stopped while starting".to_owned())?,
            res = &mut task => {
                return Err(match res {
                    Ok(Err(e)) => format!("the built-in server could not start: {e}"),
                    _ => "the built-in server stopped while starting".to_owned(),
                });
            }
        };
        *slot = Some(Running {
            handle: handle.clone(),
            task,
        });
        self.in_use.store(true, Ordering::SeqCst);
        Ok(handle)
    }

    /// The running server's handle, if any.
    pub async fn handle(&self) -> Option<LocalHandle> {
        let slot = self.slot.lock().await;
        slot.as_ref()
            .filter(|r| !r.task.inner().is_finished())
            .map(|r| r.handle.clone())
    }

    pub async fn is_running(&self) -> bool {
        self.handle().await.is_some()
    }

    /// Whether the built-in server has been started (and not stopped) in this run.
    pub fn in_use(&self) -> bool {
        self.in_use.load(Ordering::SeqCst)
    }

    /// Stop the server and wait (up to `timeout`) for its listeners and databases to
    /// close. Returns false if it had to be abandoned after the timeout.
    pub async fn stop(&self, timeout: Duration) -> bool {
        self.in_use.store(false, Ordering::SeqCst);
        let Some(running) = self.slot.lock().await.take() else {
            return true;
        };
        running.handle.stop();
        let mut task = running.task;
        match tokio::time::timeout(timeout, &mut task).await {
            Ok(Ok(Ok(()))) => true,
            Ok(Ok(Err(e))) => {
                tracing::warn!("built-in server stopped with an error: {e}");
                true
            }
            Ok(Err(e)) => {
                tracing::warn!("built-in server task failed: {e}");
                true
            }
            Err(_) => {
                tracing::warn!("built-in server did not stop within {timeout:?}; aborting");
                task.abort();
                false
            }
        }
    }
}

/// What closing the main window does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Hide the window; the app (and the embedded server) keeps running in the tray.
    HideToTray,
    /// Close the window and quit the app (stopping the server first).
    Quit,
}

/// Decide what closing the window does. Hiding needs a tray icon to come back from.
pub fn on_close(keep_running: bool, tray_available: bool) -> CloseAction {
    if keep_running && tray_available {
        CloseAction::HideToTray
    } else {
        CloseAction::Quit
    }
}

/// Text for the tray's status line.
pub fn tray_status(running: bool, lan_addresses: &[String], port: u16) -> String {
    if !running {
        return "Built-in server: not running".to_owned();
    }
    match lan_addresses.first() {
        Some(a) => format!("Built-in server: running at {a}"),
        None => format!("Built-in server: this computer only (127.0.0.1:{port})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_hides_only_when_keep_running_and_tray() {
        assert_eq!(on_close(true, true), CloseAction::HideToTray);
        assert_eq!(on_close(true, false), CloseAction::Quit);
        assert_eq!(on_close(false, true), CloseAction::Quit);
        assert_eq!(on_close(false, false), CloseAction::Quit);
    }

    #[test]
    fn tray_status_text() {
        assert_eq!(
            tray_status(false, &[], 9090),
            "Built-in server: not running"
        );
        assert_eq!(
            tray_status(true, &[], 9090),
            "Built-in server: this computer only (127.0.0.1:9090)"
        );
        assert_eq!(
            tray_status(true, &["192.168.1.20:9090".into()], 9090),
            "Built-in server: running at 192.168.1.20:9090"
        );
    }
}
