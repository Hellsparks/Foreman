// Suppress the extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::Duration;

use foreman_desktop::{discover, probe, store};
use foreman_server::LocalHandle;
use serde::Serialize;
use tauri::Manager as _;
use tokio::sync::{oneshot, Mutex};

/// The embedded server (Mode A), started on demand.
#[derive(Default)]
struct Local(Mutex<Option<LocalHandle>>);

#[derive(Serialize)]
struct LocalConfig {
    port: u16,
    spoolman_port: Option<u16>,
    data_dir: String,
}

#[derive(Serialize)]
struct LocalStarted {
    address: String,
    setup_code: Option<String>,
}

fn store_file(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|d| store::path(&d))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn store_load(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    store::load(&store_file(&app)?)
}

#[tauri::command]
fn store_save(app: tauri::AppHandle, doc: serde_json::Value) -> Result<(), String> {
    store::save(&store_file(&app)?, &doc)
}

#[tauri::command]
async fn discover_servers(timeout_ms: Option<u64>) -> Result<Vec<discover::Found>, String> {
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(2500).clamp(500, 10_000));
    tauri::async_runtime::spawn_blocking(move || discover::discover(timeout))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn probe_address(address: String) -> probe::Probe {
    probe::probe(&probe::client(), &address).await
}

/// Health check for failover: the server id answering at `address`, if any.
#[tauri::command]
async fn check_address(address: String) -> Option<String> {
    probe::server_id_at(&probe::client(), &address).await
}

#[tauri::command]
fn local_server_config() -> LocalConfig {
    let cfg = foreman_server::config::ServerConfig::load();
    let spoolman_port = cfg.spoolman_enabled.then(|| {
        std::env::var("SPOOLMAN_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(probe::SPOOLMAN_PORT)
    });
    LocalConfig {
        port: cfg.port,
        spoolman_port,
        data_dir: cfg.data_dir.display().to_string(),
    }
}

async fn local_handle(local: &Local) -> Result<LocalHandle, String> {
    let mut slot = local.0.lock().await;
    if let Some(h) = slot.as_ref() {
        return Ok(h.clone());
    }
    let mut cfg = foreman_server::config::ServerConfig::load();
    cfg.desktop_mode = true;
    let (tx, rx) = oneshot::channel();
    let mut task = tauri::async_runtime::spawn(foreman_server::serve_with(cfg, Some(tx)));
    let handle = tokio::select! {
        ready = rx => ready.map_err(|_| "the built-in server stopped while starting".to_owned())?,
        res = &mut task => {
            return Err(match res {
                Ok(Err(e)) => format!("the built-in server could not start: {e}"),
                _ => "the built-in server stopped while starting".to_owned(),
            });
        }
    };
    *slot = Some(handle.clone());
    Ok(handle)
}

/// Start the embedded server (once) and report where it listens.
#[tauri::command]
async fn start_local_server(local: tauri::State<'_, Local>) -> Result<LocalStarted, String> {
    let h = local_handle(&local).await?;
    Ok(LocalStarted {
        address: format!("127.0.0.1:{}", h.port),
        setup_code: h.setup_code(),
    })
}

/// A session token for the embedded server, minted in-process (no password needed here).
#[tauri::command]
async fn local_token(local: tauri::State<'_, Local>) -> Result<String, String> {
    let h = local_handle(&local).await?;
    h.app_token().await.map_err(|e| e.to_string())
}

fn main() {
    // Linux + NVIDIA: WebKitGTK's DMA-BUF renderer can deadlock on NVIDIA
    // drivers with explicit-sync.  Disable it at process start, before any
    // threads (including WebKit internals) are spawned.
    //
    // Fallback if this alone is not enough (older driver stacks):
    //   WEBKIT_DISABLE_COMPOSITING_MODE=1
    //   __NV_DISABLE_EXPLICIT_SYNC=1
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    // Initialise tracing for the whole process (server + desktop).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "foreman_server=info,foreman_desktop=info,tower_http=warn"
                    .parse()
                    .unwrap()
            }),
        )
        .init();

    tauri::Builder::default()
        .manage(Local::default())
        .invoke_handler(tauri::generate_handler![
            store_load,
            store_save,
            discover_servers,
            probe_address,
            check_address,
            local_server_config,
            start_local_server,
            local_token,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Foreman desktop");
}
