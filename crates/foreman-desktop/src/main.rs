// Suppress the extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use foreman_desktop::{
    autostart, discover,
    lifecycle::{self, CloseAction, Local},
    probe, settings, store,
};
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager as _, RunEvent, WindowEvent, Wry,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_updater::UpdaterExt as _;
use tokio::sync::Mutex;

/// How long Quit waits for the built-in server to close its listeners and databases.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// The tray's status line (None when no tray could be created).
#[derive(Default)]
struct Tray {
    status: std::sync::Mutex<Option<MenuItem<Wry>>>,
    available: AtomicBool,
}

/// An update found by the last check, kept for "Install".
#[derive(Default)]
struct PendingUpdate(Mutex<Option<tauri_plugin_updater::Update>>);

/// Set once a quit is under way so exit requests don't start a second stop.
#[derive(Default)]
struct Quitting(AtomicBool);

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

#[derive(Serialize)]
struct DesktopPrefs {
    /// Effective "keep running in the background".
    keep_running: bool,
    /// The stored choice; null = default (on while the built-in server is in use).
    keep_running_choice: Option<bool>,
    autostart: bool,
    local_in_use: bool,
    tray_available: bool,
    update_channel: settings::UpdateChannel,
}

#[derive(Serialize)]
struct AppInfo {
    version: &'static str,
    commit: &'static str,
    app_data_dir: String,
    server_data_dir: String,
}

#[derive(Serialize)]
struct UpdateInfo {
    version: String,
    current_version: String,
    notes: Option<String>,
    date: Option<String>,
}

fn app_data(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path().app_data_dir().map_err(|e| e.to_string())
}

fn store_file(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app_data(app).map(|d| store::path(&d))
}

fn load_settings(app: &AppHandle) -> settings::DesktopSettings {
    app_data(app)
        .map(|d| settings::load(&settings::path(&d)))
        .unwrap_or_default()
}

#[tauri::command]
fn store_load(app: AppHandle) -> Result<serde_json::Value, String> {
    store::load(&store_file(&app)?)
}

#[tauri::command]
fn store_save(app: AppHandle, doc: serde_json::Value) -> Result<(), String> {
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

async fn start_local(app: &AppHandle) -> Result<foreman_server::LocalHandle, String> {
    let h = app
        .state::<Local>()
        .start(foreman_server::config::ServerConfig::load())
        .await?;
    refresh_tray(app).await;
    Ok(h)
}

/// Start the embedded server (once) and report where it listens.
#[tauri::command]
async fn start_local_server(app: AppHandle) -> Result<LocalStarted, String> {
    let h = start_local(&app).await?;
    Ok(LocalStarted {
        address: format!("127.0.0.1:{}", h.port),
        setup_code: h.setup_code(),
    })
}

/// A session token for the embedded server, minted in-process (no password needed here).
#[tauri::command]
async fn local_token(app: AppHandle) -> Result<String, String> {
    let h = start_local(&app).await?;
    h.app_token().await.map_err(|e| e.to_string())
}

#[tauri::command]
fn desktop_prefs(app: AppHandle) -> DesktopPrefs {
    let s = load_settings(&app);
    let local_in_use = app.state::<Local>().in_use();
    DesktopPrefs {
        keep_running: s.keep_running(local_in_use),
        keep_running_choice: s.keep_running,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        local_in_use,
        tray_available: app.state::<Tray>().available.load(Ordering::SeqCst),
        update_channel: s.update_channel,
    }
}

#[tauri::command]
fn set_update_channel(
    app: AppHandle,
    channel: settings::UpdateChannel,
) -> Result<DesktopPrefs, String> {
    let file = settings::path(&app_data(&app)?);
    let mut s = settings::load(&file);
    s.update_channel = channel;
    settings::save(&file, &s)?;
    Ok(desktop_prefs(app))
}

#[tauri::command]
fn set_keep_running(app: AppHandle, on: bool) -> Result<DesktopPrefs, String> {
    let file = settings::path(&app_data(&app)?);
    let mut s = settings::load(&file);
    s.keep_running = Some(on);
    settings::save(&file, &s)?;
    Ok(desktop_prefs(app))
}

#[tauri::command]
fn set_autostart(app: AppHandle, on: bool) -> Result<DesktopPrefs, String> {
    let al = app.autolaunch();
    if on { al.enable() } else { al.disable() }.map_err(|e| e.to_string())?;
    Ok(desktop_prefs(app))
}

#[tauri::command]
fn app_info(app: AppHandle) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        commit: foreman_server::GIT_COMMIT,
        app_data_dir: app_data(&app)
            .map(|d| d.display().to_string())
            .unwrap_or_default(),
        server_data_dir: foreman_server::config::ServerConfig::load()
            .data_dir
            .display()
            .to_string(),
    }
}

/// Ask the chosen update channel for a newer version; keeps it for [`update_install`].
#[tauri::command]
async fn update_check(
    app: AppHandle,
    pending: tauri::State<'_, PendingUpdate>,
) -> Result<Option<UpdateInfo>, String> {
    let endpoint = load_settings(&app).update_channel.endpoint();
    let url = endpoint.parse().map_err(|e| format!("{endpoint}: {e}"))?;
    let update = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone(),
        date: u
            .raw_json
            .get("pub_date")
            .and_then(|d| d.as_str())
            .map(str::to_owned),
    });
    *pending.0.lock().await = update;
    Ok(info)
}

/// Download the checked update, stop the built-in server cleanly, install, restart.
#[tauri::command]
async fn update_install(
    app: AppHandle,
    pending: tauri::State<'_, PendingUpdate>,
) -> Result<(), String> {
    let Some(update) = pending.0.lock().await.take() else {
        return Err("no update to install; check for updates first".into());
    };
    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.state::<Quitting>().0.store(true, Ordering::SeqCst);
    app.state::<Local>().stop(STOP_TIMEOUT).await;
    if let Err(e) = update.install(bytes) {
        app.state::<Quitting>().0.store(false, Ordering::SeqCst);
        return Err(e.to_string());
    }
    app.restart();
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Stop the built-in server cleanly, then exit.
fn quit(app: &AppHandle) {
    if app.state::<Quitting>().0.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        app.state::<Local>().stop(STOP_TIMEOUT).await;
        app.exit(0);
    });
}

async fn refresh_tray(app: &AppHandle) {
    let local = app.state::<Local>();
    let text = match local.handle().await {
        Some(h) => lifecycle::tray_status(true, &h.lan_addresses(), h.port),
        None => lifecycle::tray_status(false, &[], 0),
    };
    let item = app
        .state::<Tray>()
        .status
        .lock()
        .ok()
        .and_then(|s| s.clone());
    if let Some(item) = item {
        if item.text().ok().as_deref() != Some(text.as_str()) {
            let _ = item.set_text(text);
        }
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Foreman", true, None::<&str>)?;
    let status = MenuItem::with_id(
        app,
        "status",
        lifecycle::tray_status(false, &[], 0),
        false,
        None::<&str>,
    )?;
    let quit_i = MenuItem::with_id(app, "quit", "Quit Foreman", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &status, &sep, &quit_i])?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Foreman")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    let tray = app.state::<Tray>();
    if let Ok(mut slot) = tray.status.lock() {
        *slot = Some(status);
    }
    tray.available.store(true, Ordering::SeqCst);
    Ok(())
}

/// Whether the saved connection is the built-in server ("This computer").
fn last_was_local(app: &AppHandle) -> bool {
    store_file(app)
        .and_then(|f| store::load(&f))
        .map(|doc| doc.get("last").and_then(|v| v.as_str()) == Some("local"))
        .unwrap_or(false)
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    if let Err(e) = build_tray(&handle) {
        // No tray (e.g. no StatusNotifier host): closing the window quits instead.
        tracing::warn!("no tray icon: {e}");
    }

    // Reopening "This computer": start the server now, not when the page asks, so an
    // autostarted app hidden in the tray serves the fleet straight away.
    let local_last = last_was_local(&handle);
    if local_last {
        let h = handle.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = start_local(&h).await {
                tracing::warn!("built-in server did not start: {e}");
            }
        });
    }

    if autostart::launched_by_autostart(std::env::args())
        && load_settings(&handle).keep_running(local_last)
        && handle.state::<Tray>().available.load(Ordering::SeqCst)
    {
        if let Some(w) = handle.get_webview_window("main") {
            let _ = w.hide();
        }
    }

    // Keep the tray's address current (LAN toggle, network changes).
    let h = handle.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3));
        loop {
            tick.tick().await;
            refresh_tray(&h).await;
        }
    });
    Ok(())
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
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(
            "foreman_server=info,foreman_desktop=info,tower_http=warn",
        )
    });
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let app = tauri::Builder::default()
        .plugin(autostart::plugin(autostart::NAME))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Local::default())
        .manage(Tray::default())
        .manage(PendingUpdate::default())
        .manage(Quitting::default())
        .setup(setup)
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != "main" {
                    return;
                }
                let app = window.app_handle();
                let keep = load_settings(app).keep_running(app.state::<Local>().in_use());
                let tray = app.state::<Tray>().available.load(Ordering::SeqCst);
                match lifecycle::on_close(keep, tray) {
                    CloseAction::HideToTray => {
                        api.prevent_close();
                        let _ = window.hide();
                        tracing::info!("window hidden; Foreman keeps running in the tray");
                    }
                    CloseAction::Quit => {
                        api.prevent_close();
                        quit(app);
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            store_load,
            store_save,
            discover_servers,
            probe_address,
            check_address,
            local_server_config,
            start_local_server,
            local_token,
            desktop_prefs,
            set_keep_running,
            set_autostart,
            set_update_channel,
            app_info,
            update_check,
            update_install,
        ])
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(e) => {
            tracing::error!("Foreman desktop could not start: {e}");
            std::process::exit(1);
        }
    };
    app.run(|app, event| {
        // Any other way out (OS logout, last window gone) still stops the server first.
        if let RunEvent::ExitRequested { api, .. } = event {
            if app.state::<Local>().in_use() {
                api.prevent_exit();
                quit(app);
            }
        }
    });
}
