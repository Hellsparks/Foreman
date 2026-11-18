//! Filament tracking at runtime (doc 07 §A).
//!
//! [`Tracking`] owns the built-in Spoolman handle and the current
//! [`FilamentMode`]. Code reaches Spoolman through it in one of two ways:
//! - [`Tracking::spools`] for anything that *uses* spools (compat snapshots,
//!   reservations, deduction, project spool slots, loading). `None` while
//!   tracking is Off, so every such path goes quiet from this one switch;
//! - [`Tracking::store`] for the records themselves (backup, setup import,
//!   exports), which are kept in every mode.

use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, OnceLock,
    },
};

use foreman_core::{
    error::AppError,
    repo,
    tracking::{FilamentMode, FilamentPolicy},
};
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;
use tracing::info;

pub type Spoolman = spoolman_rs::state::AppState;

/// `setting` key holding the mode (`full` | `external` | `off`).
pub const SETTING_MODE: &str = "filament.mode";
/// `setting` key for serving the Spoolman API on its port (`true` | `false`);
/// unset = the mode's default.
pub const SETTING_SERVE_API: &str = "filament.serve_api";

pub struct Tracking {
    mode: AtomicU8,
    store: Arc<OnceLock<Spoolman>>,
    /// The Spoolman API listener while it runs (cancel to stop it).
    api: tokio::sync::Mutex<Option<CancellationToken>>,
    /// Server shutdown; the listener stops with it.
    shutdown: OnceLock<CancellationToken>,
}

fn encode(m: FilamentMode) -> u8 {
    match m {
        FilamentMode::Full => 0,
        FilamentMode::External => 1,
        FilamentMode::Off => 2,
    }
}

fn decode(v: u8) -> FilamentMode {
    match v {
        1 => FilamentMode::External,
        2 => FilamentMode::Off,
        _ => FilamentMode::Full,
    }
}

impl Tracking {
    pub fn new(mode: FilamentMode, store: Arc<OnceLock<Spoolman>>) -> Self {
        Self {
            mode: AtomicU8::new(encode(mode)),
            store,
            api: tokio::sync::Mutex::new(None),
            shutdown: OnceLock::new(),
        }
    }

    /// Tie the Spoolman API listener to server shutdown.
    pub fn set_shutdown(&self, stop: CancellationToken) {
        let _ = self.shutdown.set(stop);
    }

    /// Whether the Spoolman API is listening now.
    pub async fn api_running(&self) -> bool {
        self.api.lock().await.is_some()
    }

    /// Start or stop the Spoolman API listener (port from Spoolman's config).
    pub async fn serve_api(&self, on: bool) -> Result<(), AppError> {
        let mut api = self.api.lock().await;
        if !on {
            if let Some(t) = api.take() {
                t.cancel();
                info!("spoolman-rs API stopped");
            }
            return Ok(());
        }
        if api.is_some() {
            return Ok(());
        }
        let Some(sm) = self.store.get() else {
            return Err(AppError::Internal("Spoolman is not running".into()));
        };
        let addr: SocketAddr = format!("{}:{}", sm.config.host, sm.config.port)
            .parse()
            .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 7912)));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| AppError::Internal(format!("could not bind {addr}: {e}")))?;
        let router = spoolman_rs::api::router(&sm.config.base_path).with_state(sm.clone());
        let token = match self.shutdown.get() {
            Some(s) => s.child_token(),
            None => CancellationToken::new(),
        };
        let done = token.clone();
        info!(%addr, "spoolman-rs API listening");
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, router)
                .with_graceful_shutdown(done.cancelled_owned())
                .await
            {
                tracing::error!("spoolman-rs server error: {e}");
            }
        });
        *api = Some(token);
        Ok(())
    }

    pub fn mode(&self) -> FilamentMode {
        decode(self.mode.load(Ordering::Relaxed))
    }

    pub fn policy(&self) -> FilamentPolicy {
        self.mode().policy()
    }

    pub fn set_mode(&self, m: FilamentMode) {
        self.mode.store(encode(m), Ordering::Relaxed);
    }

    /// Spoolman for spool behaviour; `None` when tracking is Off or Spoolman is
    /// not running.
    pub fn spools(&self) -> Option<&Spoolman> {
        self.mode()
            .tracks_spools()
            .then(|| self.store.get())
            .flatten()
    }

    /// Spoolman records in any mode (`None` only when it is not running).
    pub fn store(&self) -> Option<&Spoolman> {
        self.store.get()
    }

    /// A spool id that may be stored now (on a project slot): `None` while
    /// tracking is Off.
    pub fn keep_spool(&self, id: Option<i64>) -> Option<i64> {
        id.filter(|_| self.mode().tracks_spools())
    }

    /// The cell the Spoolman state is put into once it starts.
    pub fn cell(&self) -> &Arc<OnceLock<Spoolman>> {
        &self.store
    }
}

/// The saved mode; Full when never set (existing installs keep today's behaviour).
pub async fn load_mode(pool: &SqlitePool) -> Result<FilamentMode, AppError> {
    Ok(repo::setting::get(pool, SETTING_MODE)
        .await?
        .as_deref()
        .and_then(FilamentMode::parse)
        .unwrap_or_default())
}

/// The explicit serve-API switch, if it was ever set.
pub async fn serve_api_setting(pool: &SqlitePool) -> Result<Option<bool>, AppError> {
    Ok(repo::setting::get(pool, SETTING_SERVE_API)
        .await?
        .map(|v| v == "true"))
}

/// Whether the Spoolman API should listen on its port for `mode`.
pub async fn serve_api(pool: &SqlitePool, mode: FilamentMode) -> Result<bool, AppError> {
    Ok(serve_api_setting(pool)
        .await?
        .unwrap_or(mode.policy().serve_api_default))
}
