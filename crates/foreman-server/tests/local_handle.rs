//! Desktop Mode A: the in-process handle mints app sessions and exposes the setup code.

use foreman_server::{config::ServerConfig, serve_with};
use serde_json::Value;
use tokio::sync::oneshot;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("free port")
}

async fn get(client: &reqwest::Client, url: &str, token: Option<&str>) -> (u16, Value) {
    let mut req = client.get(url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let res = req.send().await.expect("request");
    let status = res.status().as_u16();
    let body = res.text().await.unwrap_or_default();
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn local_handle_mints_app_sessions_before_setup() {
    let dir = tempfile::TempDir::new().expect("dir");
    let port = free_port();
    let cfg = ServerConfig {
        port,
        host: "127.0.0.1".into(),
        data_dir: dir.path().to_path_buf(),
        db_url: None,
        spoolman_enabled: false,
        drivers_enabled: true,
        desktop_mode: true,
    };
    let (tx, rx) = oneshot::channel();
    let task = tokio::spawn(serve_with(cfg, Some(tx)));
    let handle = rx.await.expect("server ready");
    assert_eq!(handle.port, port);
    let code = handle.setup_code().expect("setup code while no admin");
    assert_eq!(code.len(), 9);

    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    let (s, info) = get(&client, &format!("{base}/api/info"), None).await;
    assert_eq!(s, 200);
    assert_eq!(info["needs_setup"], true);
    assert_eq!(
        info["addresses"][0]["kind"], "loopback",
        "Mode A without LAN only reports loopback"
    );

    let (s, _) = get(&client, &format!("{base}/api/printers"), None).await;
    assert_eq!(
        s, 403,
        "unclaimed server: everything but health/info/setup is refused"
    );
    let token = handle.app_token().await.expect("token");
    let (s, _) = get(&client, &format!("{base}/api/printers"), Some(&token)).await;
    assert_eq!(s, 200);
    let (_, me) = get(&client, &format!("{base}/api/auth/me"), Some(&token)).await;
    assert_eq!(me["username"], "local");

    task.abort();
}
