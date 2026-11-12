//! Closing the window to the tray leaves the built-in server answering; quitting stops it.

use std::time::Duration;

use foreman_desktop::{
    lifecycle::{on_close, CloseAction, Local},
    settings::DesktopSettings,
};
use foreman_server::config::ServerConfig;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(0)
}

async fn health(port: u16) -> Result<serde_json::Value, reqwest::Error> {
    reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/api/health"))
        .timeout(Duration::from_secs(3))
        .send()
        .await?
        .json()
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn close_to_tray_keeps_server_quit_stops_it() {
    let dir =
        std::env::temp_dir().join(format!("foreman-desktop-lifecycle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let port = free_port();
    assert_ne!(port, 0);
    let cfg = ServerConfig {
        port,
        host: "127.0.0.1".into(),
        db_url: Some(format!("sqlite://{}", dir.join("foreman.db").display())),
        data_dir: dir.clone(),
        spoolman_enabled: false,
        drivers_enabled: false,
        desktop_mode: true,
    };

    let local = Local::default();
    let h = local.start(cfg).await.expect("server starts");
    assert_eq!(h.port, port);
    assert!(local.in_use());
    assert_eq!(health(port).await.expect("health")["status"], "ok");

    // Close the window with the default setting while the built-in server is in use.
    let keep = DesktopSettings::default().keep_running(local.in_use());
    assert_eq!(on_close(keep, true), CloseAction::HideToTray);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(local.is_running().await, "hiding must not stop the server");
    assert_eq!(
        health(port).await.expect("health after hide")["status"],
        "ok"
    );

    // Quit from the tray.
    assert!(
        local.stop(Duration::from_secs(10)).await,
        "stops within the timeout"
    );
    assert!(!local.in_use());
    assert!(!local.is_running().await);
    assert!(
        health(port).await.is_err(),
        "server must stop answering after quit"
    );
    // The port is free again (listener really closed).
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());

    let _ = std::fs::remove_dir_all(&dir);
}
