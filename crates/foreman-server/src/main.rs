#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "foreman_server=info,tower_http=warn".parse().unwrap()),
        )
        .init();

    let cfg = foreman_server::config::ServerConfig::load();

    if let Err(err) = foreman_server::serve(cfg).await {
        eprintln!("server error: {err}");
        std::process::exit(1);
    }
}
