mod api;
mod auth;
#[cfg(test)]
mod auth_api_tests;
pub mod config;
mod driver_manager;
mod net;
mod ssh_pool;
mod state;

use axum::{
    middleware,
    routing::{delete, get, get_service, post, put},
    Router,
};
use foreman_core::{db::ForemanDb, events::ServerEvent, repo};
use spoolman_rs::{
    api as spoolman_api,
    config::SpoolmanConfig,
    db::{extra_field, Db as SpoolmanDb},
    state::AppState as SpoolmanState,
};
use std::{
    future::IntoFuture as _,
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    signal,
    sync::{broadcast, oneshot, watch},
};
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
    serve_with(cfg, None).await
}

/// In-process access for the desktop app (Mode A) to the server it embeds.
#[derive(Clone)]
pub struct LocalHandle {
    pool: sqlx::SqlitePool,
    auth: Arc<auth::AuthState>,
    pub port: u16,
}

impl LocalHandle {
    /// The setup code while no admin exists (so the app can prefill it).
    pub fn setup_code(&self) -> Option<String> {
        let slot = self.auth.setup_code.lock().ok()?;
        slot.as_ref()?.current(Instant::now()).map(str::to_owned)
    }

    /// A session token for this app; works before an admin exists (audit user "local").
    pub async fn app_token(&self) -> anyhow::Result<String> {
        let (token, _) = foreman_core::auth::create_session(
            &self.pool,
            auth::REMEMBER_TTL_SECS,
            Some("127.0.0.1"),
            Some("foreman-desktop"),
        )
        .await?;
        Ok(token)
    }
}

/// [`serve`], optionally handing a [`LocalHandle`] to the caller once the server is up.
pub async fn serve_with(
    cfg: config::ServerConfig,
    ready: Option<oneshot::Sender<LocalHandle>>,
) -> anyhow::Result<()> {
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

    // Auth bootstrap: setup code when there is no admin yet, then the bind rule.
    let has_admin = foreman_core::auth::has_admin(&foreman_db.pool).await?;
    let auth_state = Arc::new(auth::AuthState::default());
    if !has_admin {
        let code = foreman_core::auth::generate_setup_code()?;
        info!(
            "No admin account yet. Setup code: {code} (single use, valid 24 h). \
             Open the app or http://<this machine>:{} and enter it to create the admin. \
             Until then only /api/health, /api/info and /api/auth/setup answer.",
            cfg.port
        );
        if let Ok(mut slot) = auth_state.setup_code.lock() {
            *slot = Some(auth::SetupCode::new(code, std::time::Instant::now()));
        }
    }
    let allow_lan = repo::setting::get(&foreman_db.pool, api::auth::SETTING_ALLOW_LAN)
        .await?
        .is_some_and(|v| v == "true");
    let bind_host = auth::effective_bind_host(&cfg.host, has_admin, cfg.desktop_mode, allow_lan);
    let hostname = net::hostname();
    let has_tailscale = net::interface_ips()
        .iter()
        .any(|(_, ip)| net::is_tailscale(*ip));
    let magic_dns = if has_tailscale {
        net::magic_dns_name().await
    } else {
        None
    };
    let (bind_tx, mut bind_rx) = watch::channel(bind_host);
    let server_info = Arc::new(auth::ServerInfo {
        server_id: server_id(&foreman_db).await?,
        version: env!("CARGO_PKG_VERSION"),
        desktop_mode: cfg.desktop_mode,
        configured_host: cfg.host.clone(),
        port: cfg.port,
        hostname,
        magic_dns,
        bind: bind_tx,
    });
    {
        let db = foreman_db.pool.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            loop {
                interval.tick().await;
                let now = chrono::Utc::now().timestamp();
                if let Err(e) = foreman_core::auth::purge_expired_sessions(&db, now).await {
                    warn!("purge expired sessions: {e}");
                }
            }
        });
    }

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
        auth: auth_state,
        info: server_info,
    };

    let app = build_router(app_state.clone());

    spawn_spoolman(&cfg).await;

    let mut ready = ready;
    let handle = LocalHandle {
        pool: app_state.foreman_db.pool.clone(),
        auth: Arc::clone(&app_state.auth),
        port: cfg.port,
    };

    // Serve; when the bind host changes (admin created, LAN toggle, rename) move the
    // listener without touching drivers or open connections.
    let mut shutdown = std::pin::pin!(shutdown_signal());
    loop {
        let host = bind_rx.borrow_and_update().clone();
        let addr: SocketAddr = format!("{host}:{}", cfg.port)
            .parse()
            .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], cfg.port)));
        let listener = bind_with_retry(addr).await?;
        info!("listening on http://{addr}");
        info!("health: http://{addr}/api/health");
        info!("events: ws://{addr}/ws/events");
        if let Some(tx) = ready.take() {
            let _ = tx.send(handle.clone());
        }

        let _announcer = if auth::is_loopback(&host) {
            None
        } else {
            let ips: Vec<_> = match addr.ip() {
                std::net::IpAddr::V4(v4) if !v4.is_unspecified() => vec![v4],
                _ => net::interface_ips().into_iter().map(|(_, ip)| ip).collect(),
            };
            let pool = &app_state.foreman_db.pool;
            let name = api::auth::server_name(pool, &app_state.info.hostname)
                .await
                .unwrap_or_else(|_| "Foreman".to_owned());
            let needs_setup = !foreman_core::auth::has_admin(pool).await.unwrap_or(true);
            net::announce(
                &app_state.info.server_id,
                &name,
                app_state.info.version,
                needs_setup,
                &app_state.info.hostname,
                cfg.port,
                &ips,
            )
        };

        let server = axum::serve(
            listener,
            app.clone()
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .into_future();
        tokio::select! {
            res = server => {
                res.map_err(|e| anyhow::anyhow!("server error: {e}"))?;
                break;
            }
            _ = &mut shutdown => break,
            changed = bind_rx.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Bind, retrying briefly (the previous listener on the same port may still be closing).
async fn bind_with_retry(addr: SocketAddr) -> anyhow::Result<tokio::net::TcpListener> {
    let mut last = None;
    for _ in 0..10 {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => return Ok(l),
            Err(e) => {
                last = Some(e);
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        }
    }
    Err(anyhow::anyhow!(
        "failed to bind {addr}: {}",
        last.map(|e| e.to_string()).unwrap_or_default()
    ))
}

/// Persistent server id (UUID v4, created once).
async fn server_id(db: &ForemanDb) -> anyhow::Result<String> {
    if let Some(id) = repo::setting::get(&db.pool, "server.id").await? {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    repo::setting::set(&db.pool, "server.id", &id).await?;
    Ok(id)
}

/// `foreman-server reset-admin`: set a new admin password from the terminal.
pub async fn reset_admin_cli(cfg: config::ServerConfig) -> anyhow::Result<()> {
    use std::io::Write as _;

    let db = ForemanDb::connect(&cfg.data_dir.join("foreman.db")).await?;
    let current = foreman_core::auth::admin_username(&db.pool).await?;
    let default_user = current.clone().unwrap_or_else(|| "admin".to_owned());
    print!("Username [{default_user}]: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let username = match line.trim() {
        "" => default_user,
        u => u.to_owned(),
    };
    let pw = rpassword::prompt_password(format!(
        "New password (min {} characters): ",
        foreman_core::auth::MIN_PASSWORD_LEN
    ))?;
    foreman_core::auth::check_password_rules(&pw)?;
    let pw2 = rpassword::prompt_password("Repeat password: ")?;
    if pw != pw2 {
        anyhow::bail!("passwords do not match");
    }
    foreman_core::auth::reset_admin(&db.pool, &username, &pw).await?;
    println!("Admin '{username}' saved. All sessions were signed out.");
    Ok(())
}

/// The Spoolman-compatible API on its own port (default 7912).
async fn spawn_spoolman(cfg: &config::ServerConfig) {
    if !cfg.spoolman_enabled {
        return;
    }
    info!(
        "Spoolman-compatible API is unauthenticated by design (Moonraker and slicers expect that)"
    );
    match SpoolmanConfig::from_env() {
        Ok(spoolman_cfg) => {
            let spoolman_addr: SocketAddr = format!("{}:{}", spoolman_cfg.host, spoolman_cfg.port)
                .parse()
                .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 7912)));
            match SpoolmanDb::connect(&spoolman_cfg).await {
                Ok(db) => {
                    if let Err(e) = extra_field::ensure_foreman_defaults(&db.pool, &db.engine).await
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

/// Build the HTTP router (all routes behind [`auth::require_auth`]).
fn build_router(app_state: AppState) -> Router {
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

    Router::new()
        .route("/api/health", get(api::health::health))
        // Auth
        .route("/api/info", get(api::auth::info))
        .route("/api/auth/setup", post(api::auth::setup))
        .route("/api/auth/login", post(api::auth::login))
        .route("/api/auth/logout", post(api::auth::logout))
        .route("/api/auth/me", get(api::auth::me))
        .route("/api/auth/url-token", post(api::auth::url_token))
        .route(
            "/api/auth/api-keys",
            get(api::auth::list_keys).post(api::auth::create_key),
        )
        .route("/api/auth/api-keys/{id}", delete(api::auth::revoke_key))
        .route(
            "/api/settings/network",
            get(api::auth::get_network).put(api::auth::put_network),
        )
        .route(
            "/api/settings/server",
            get(api::auth::get_server).put(api::auth::put_server),
        )
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
        .layer(middleware::from_fn_with_state(
            app_state.clone(),
            auth::require_auth,
        ))
        .with_state(app_state)
        .layer(
            // URIs may carry scoped `?token=`s; never log them.
            TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<_>| {
                tracing::debug_span!(
                    "request",
                    method = %req.method(),
                    uri = %auth::redact_token(&req.uri().to_string()),
                )
            }),
        )
        .layer(CorsLayer::permissive())
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
