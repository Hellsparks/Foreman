mod api;
pub mod config;
mod driver_manager;
mod ssh_pool;
mod state;

use axum::{
    routing::{delete, get, get_service, post, put},
    Router,
};
use foreman_core::{db::ForemanDb, events::ServerEvent};
use spoolman_rs::{
    api as spoolman_api,
    config::SpoolmanConfig,
    db::{extra_field, Db as SpoolmanDb},
    state::AppState as SpoolmanState,
};
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::{signal, sync::broadcast};
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing::{info, warn};

use driver_manager::DriverManager;
use state::AppState;

/// Start the Foreman HTTP server and serve until a shutdown signal is received.
///
/// The caller is responsible for initialising a tracing subscriber before
/// calling this function.
pub async fn serve(cfg: config::ServerConfig) -> anyhow::Result<()> {
    info!(
        port = cfg.port,
        host = %cfg.host,
        data_dir = %cfg.data_dir.display(),
        db_url = cfg.db_url.as_deref().unwrap_or("(none)"),
        "foreman-server starting"
    );

    // Ensure data dir exists.
    if let Err(err) = std::fs::create_dir_all(&cfg.data_dir) {
        warn!(
            "could not create data dir {}: {err}",
            cfg.data_dir.display()
        );
    }

    // Open (or create) the Foreman-native SQLite database.
    let foreman_db_path = cfg.data_dir.join("foreman.db");
    let foreman_db = ForemanDb::connect(&foreman_db_path).await?;
    info!(path = %foreman_db_path.display(), "foreman db ready");

    // Broadcast bus — 256-slot ring buffer.
    let (tx, _) = broadcast::channel::<ServerEvent>(256);
    let tx = Arc::new(tx);

    // Heartbeat task: publish a heartbeat every 10 s.
    {
        let tx = Arc::clone(&tx);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                let _ = tx.send(ServerEvent::heartbeat());
            }
        });
    }

    // Driver manager: load printers from DB and spawn drivers.
    let driver_manager = DriverManager::new(Arc::clone(&tx), foreman_db.pool.clone());
    driver_manager.start_all(&foreman_db.pool).await;

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    // Secrets master key lives next to the DB, outside it.
    let secrets = Arc::new(foreman_core::secrets::SecretBox::load_or_create(
        &cfg.data_dir.join("secret.key"),
    )?);
    // SSH identity: generated once, stored encrypted in `setting`.
    let (ssh_key, ssh_pub) =
        foreman_core::ssh_client::ensure_identity(&foreman_db.pool, &secrets).await?;
    info!(pub_key = %ssh_pub, "SSH identity ready");
    let ssh_identity = Arc::new((ssh_key, ssh_pub));

    let ssh_pool = Arc::new(ssh_pool::SshPool::default());
    {
        let ssh_pool = Arc::clone(&ssh_pool);
        let db = foreman_db.pool.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                ssh_pool.reap(&db).await;
            }
        });
    }

    let app_state = AppState {
        foreman_db,
        bus_tx: Arc::clone(&tx),
        driver_manager,
        http_client,
        ssh_identity,
        secrets,
        ssh_pool,
    };

    // Static file serving for the frontend.
    let frontend_dist = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|r| r.join("frontend/dist"))
        .unwrap_or_else(|| PathBuf::from("frontend/dist"));

    let spa_fallback = frontend_dist.join("index.html");

    let static_service = if frontend_dist.exists() {
        get_service(ServeDir::new(&frontend_dist).fallback(ServeFile::new(&spa_fallback)))
    } else {
        get_service(ServeDir::new(&frontend_dist))
    };

    let app = Router::new()
        .route("/api/health", get(api::health::health))
        .route("/api/printers", get(api::printers::list_printers))
        .route("/api/printers", post(api::printers::create_printer))
        .route("/api/printers/{id}", delete(api::printers::delete_printer))
        .route(
            "/api/printers/{id}",
            put(api::printer_paths::update_printer),
        )
        .route("/api/printers/{id}/snapshot", get(api::printers::snapshot))
        .route(
            "/api/printers/{id}/trust-new-cert",
            post(api::printers::trust_new_cert),
        )
        // Printer paths
        .route(
            "/api/printers/{id}/paths",
            get(api::printer_paths::list_paths).post(api::printer_paths::create_path),
        )
        .route(
            "/api/printers/{id}/paths/{pid}",
            put(api::printer_paths::update_path).delete(api::printer_paths::delete_path),
        )
        .route(
            "/api/printers/{id}/paths/reorder",
            post(api::printer_paths::reorder_paths),
        )
        .route(
            "/api/printers/{id}/paths/test-all",
            post(api::printer_paths::test_all_paths),
        )
        .route(
            "/api/printers/{id}/active-path",
            get(api::printer_paths::get_active_path),
        )
        .route(
            "/api/printers/{id}/paths/switch-back",
            post(api::printer_paths::switch_back),
        )
        // Printer commands + status + capabilities
        .route(
            "/api/printers/{id}/commands/{kind}",
            post(api::printer_commands::send_command),
        )
        .route(
            "/api/printers/{id}/confirm-cancel",
            post(api::printer_commands::confirm_cancel),
        )
        .route(
            "/api/printers/{id}/capabilities",
            get(api::printer_commands::get_capabilities),
        )
        .route(
            "/api/printers/{id}/status",
            get(api::printer_commands::get_status),
        )
        // Temp presets
        .route(
            "/api/printers/{id}/presets",
            get(api::printer_commands::list_presets).post(api::printer_commands::create_preset),
        )
        .route(
            "/api/printers/{id}/presets/{pid}",
            delete(api::printer_commands::delete_preset),
        )
        // Macro favourites
        .route(
            "/api/printers/{id}/macro-favourites",
            get(api::printer_commands::list_favourites).post(api::printer_commands::add_favourite),
        )
        .route(
            "/api/printers/{id}/macro-favourites/{fid}",
            delete(api::printer_commands::remove_favourite),
        )
        .route("/api/attention", get(api::attention::list_attention))
        .route("/api/attention/count", get(api::attention::attention_count))
        .route(
            "/api/attention/{id}/dismiss",
            post(api::attention::dismiss_attention),
        )
        // SSH / SFTP / shell-macros
        .route("/api/ssh/public-key", get(api::ssh::get_public_key))
        .route(
            "/api/printers/{id}/ssh",
            get(api::ssh::get_ssh)
                .put(api::ssh::upsert_ssh)
                .delete(api::ssh::delete_ssh),
        )
        .route(
            "/api/printers/{id}/ssh/trust",
            post(api::ssh::trust_host_key),
        )
        .route("/api/printers/{id}/ssh/test", post(api::ssh::test_ssh))
        .route(
            "/api/printers/{id}/ssh/session",
            get(api::ssh::session_status).delete(api::ssh::close_session),
        )
        .route("/api/printers/{id}/files/roots", get(api::ssh::roots))
        .route(
            "/api/printers/{id}/files",
            get(api::ssh::list_files).delete(api::ssh::delete_file),
        )
        .route("/api/printers/{id}/files/read", get(api::ssh::read_file))
        .route(
            "/api/printers/{id}/files/download",
            get(api::ssh::download_file),
        )
        .route("/api/printers/{id}/files/write", put(api::ssh::write_file))
        .route(
            "/api/printers/{id}/files/save-restart",
            post(api::ssh::save_and_restart),
        )
        .route(
            "/api/printers/{id}/files/upload",
            post(api::ssh::upload_file),
        )
        .route("/api/printers/{id}/files/mkdir", post(api::ssh::mkdir))
        .route(
            "/api/printers/{id}/files/rename",
            post(api::ssh::rename_file),
        )
        .route(
            "/api/printers/{id}/shell-macros",
            get(api::ssh::list_macros).post(api::ssh::create_macro),
        )
        .route(
            "/api/printers/{id}/shell-macros/{mid}",
            put(api::ssh::update_macro).delete(api::ssh::delete_macro),
        )
        // Terminal WebSocket
        .route(
            "/ws/printers/{id}/terminal",
            get(api::terminal_ws::terminal_ws),
        )
        .route("/ws/events", get(api::ws::ws_handler))
        .fallback_service(static_service)
        .with_state(app_state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());

    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port)
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 9090)));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|err| anyhow::anyhow!("failed to bind {addr}: {err}"))?;

    info!("listening on http://{addr}");
    info!("health: http://{addr}/api/health");
    info!("events: ws://{addr}/ws/events");

    // Spawn the Spoolman-compatible API on its own port (default 7912).
    if cfg.spoolman_enabled {
        match SpoolmanConfig::from_env() {
            Ok(spoolman_cfg) => {
                let spoolman_addr: SocketAddr =
                    format!("{}:{}", spoolman_cfg.host, spoolman_cfg.port)
                        .parse()
                        .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 7912)));
                match SpoolmanDb::connect(&spoolman_cfg).await {
                    Ok(db) => {
                        if let Err(e) =
                            extra_field::ensure_foreman_defaults(&db.pool, &db.engine).await
                        {
                            warn!("could not create Foreman default extra fields: {e}");
                        }
                        let state = SpoolmanState::new(db, spoolman_cfg.clone());
                        let spoolman_router =
                            spoolman_api::router(&spoolman_cfg.base_path).with_state(state);
                        match tokio::net::TcpListener::bind(spoolman_addr).await {
                            Ok(spoolman_listener) => {
                                info!(%spoolman_addr, "spoolman-rs API listening");
                                tokio::spawn(async move {
                                    if let Err(e) =
                                        axum::serve(spoolman_listener, spoolman_router).await
                                    {
                                        tracing::error!("spoolman-rs server error: {e}");
                                    }
                                });
                            }
                            Err(e) => {
                                warn!("could not bind spoolman-rs to {spoolman_addr}: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        warn!("spoolman-rs database connect failed: {e}");
                    }
                }
            }
            Err(e) => {
                warn!("spoolman-rs config error (spoolman-rs disabled): {e}");
            }
        }
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| anyhow::anyhow!("server error: {e}"))?;

    info!("foreman-server stopped");
    Ok(())
}

/// Waits for Ctrl+C (all platforms) or SIGTERM (Unix).
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { info!("received Ctrl+C, shutting down"); },
        _ = terminate => { info!("received SIGTERM, shutting down"); },
    }
}
