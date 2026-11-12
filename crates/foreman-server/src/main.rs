#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "foreman_server=info,tower_http=warn".parse().unwrap()),
        )
        .init();

    let cfg = foreman_server::config::ServerConfig::load();

    if std::env::args().nth(1).as_deref() == Some("reset-admin") {
        if let Err(err) = foreman_server::reset_admin_cli(cfg).await {
            eprintln!("reset-admin failed: {err}");
            std::process::exit(1);
        }
        return;
    }

    if std::env::args().nth(1).as_deref() == Some("dev-token") {
        if let Err(err) = foreman_server::dev_token_cli(cfg).await {
            eprintln!("dev-token failed: {err}");
            std::process::exit(1);
        }
        return;
    }

    if std::env::args().nth(1).as_deref() == Some("backup") {
        let dir = std::env::args().nth(2).map(std::path::PathBuf::from);
        if let Err(err) = foreman_server::backup_cli(cfg, dir).await {
            eprintln!("backup failed: {err}");
            std::process::exit(1);
        }
        return;
    }

    if std::env::args().nth(1).as_deref() == Some("restore") {
        let Some(file) = std::env::args().nth(2) else {
            eprintln!("usage: foreman-server restore <backup.zip | backup.zip.age>");
            std::process::exit(2);
        };
        if let Err(err) = foreman_server::restore_cli(cfg, file.into()).await {
            eprintln!("restore failed: {err}");
            std::process::exit(1);
        }
        return;
    }

    if let Err(err) = foreman_server::serve(cfg).await {
        eprintln!("server error: {err}");
        std::process::exit(1);
    }
}
