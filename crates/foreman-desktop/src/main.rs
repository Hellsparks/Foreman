// Suppress the extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
        .setup(|_app| {
            // Spawn the embedded HTTP + WebSocket server as a background task
            // on Tauri's (tokio) async runtime.  It binds to 127.0.0.1 on the
            // port configured via FOREMAN__PORT (default 9090).
            let mut cfg = foreman_server::config::ServerConfig::load();
            cfg.desktop_mode = true;
            tauri::async_runtime::spawn(async move {
                if let Err(e) = foreman_server::serve(cfg).await {
                    tracing::error!("embedded server stopped: {e}");
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Foreman desktop");
}
