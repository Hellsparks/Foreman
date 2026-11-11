//! A headless server listens on all interfaces by default, and the "no admin" gate
//! still holds there: only health, info and setup answer.

use foreman_server::{config::ServerConfig, serve_with};
use serde_json::Value;
use tokio::sync::oneshot;

fn free_port() -> u16 {
    std::net::TcpListener::bind("0.0.0.0:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("free port")
}

async fn get(client: &reqwest::Client, url: &str) -> (u16, Value) {
    let res = client.get(url).send().await.expect("request");
    let status = res.status().as_u16();
    let body = res.text().await.unwrap_or_default();
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

#[test]
fn default_host_is_all_interfaces() {
    assert_eq!(ServerConfig::default().host, "0.0.0.0");
}

#[tokio::test]
async fn unclaimed_server_on_all_interfaces_only_answers_health_info_setup() {
    let dir = tempfile::TempDir::new().expect("dir");
    let port = free_port();
    let cfg = ServerConfig {
        port,
        host: "0.0.0.0".into(),
        data_dir: dir.path().to_path_buf(),
        db_url: None,
        spoolman_enabled: false,
        desktop_mode: false,
    };
    let (tx, rx) = oneshot::channel();
    let task = tokio::spawn(serve_with(cfg, Some(tx)));
    let _handle = rx.await.expect("server ready");

    let client = reqwest::Client::new();
    // Over IPv4, and over IPv6 when this machine has it.
    let mut bases = vec![format!("http://127.0.0.1:{port}")];
    if std::net::TcpStream::connect(("::1", port)).is_ok() {
        bases.push(format!("http://[::1]:{port}"));
    }
    for base in &bases {
        let (s, h) = get(&client, &format!("{base}/api/health")).await;
        assert_eq!(s, 200, "{base} health");
        assert_eq!(h["status"], "ok");

        let (s, info) = get(&client, &format!("{base}/api/info")).await;
        assert_eq!(s, 200, "{base} info");
        assert_eq!(info["needs_setup"], true);
        assert!(
            info["addresses"]
                .as_array()
                .is_some_and(|a| a.iter().all(|x| x["kind"] != "loopback")),
            "bound to all interfaces: no loopback-only address list ({info})"
        );

        for route in [
            "/api/printers",
            "/api/jobs",
            "/api/settings/library",
            "/api/auth/me",
        ] {
            let (s, b) = get(&client, &format!("{base}{route}")).await;
            assert_eq!(s, 403, "{base}{route} must be refused before setup");
            assert_eq!(b["code"], "setup_required", "{base}{route}");
        }
    }
    task.abort();
}
