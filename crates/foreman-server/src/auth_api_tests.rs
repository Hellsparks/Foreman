//! Router-level auth tests: 401s, setup, login rate limit, API keys, logout.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use foreman_core::{auth as core_auth, db::ForemanDb, events::ServerEvent, secrets::SecretBox};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tower::ServiceExt as _;

use crate::{
    auth::{AuthState, ServerInfo, SetupCode},
    build_router,
    driver_manager::DriverManager,
    ssh_pool::SshPool,
    state::AppState,
};

const CODE: &str = "ABCD-EFGH";
const PW: &str = "a long password";

async fn app(dir: &tempfile::TempDir, with_admin: bool) -> (Router, AppState) {
    let db = ForemanDb::connect(&dir.path().join("foreman.db"))
        .await
        .expect("db");
    if with_admin {
        core_auth::create_admin(&db.pool, "admin", PW)
            .await
            .expect("admin");
    }
    let (tx, _) = broadcast::channel::<ServerEvent>(16);
    let secrets = SecretBox::from_key([3u8; 32]);
    let (key, public) = foreman_core::ssh_client::ensure_identity(&db.pool, &secrets)
        .await
        .expect("identity");
    let auth = AuthState::default();
    if let Ok(mut slot) = auth.setup_code.lock() {
        *slot = Some(SetupCode::new(CODE.into(), std::time::Instant::now()));
    }
    let state = AppState {
        driver_manager: DriverManager::new(Arc::new(tx.clone()), db.pool.clone()),
        bus_tx: Arc::new(tx),
        foreman_db: db,
        http_client: reqwest::Client::new(),
        ssh_identity: Arc::new((key, public)),
        secrets: Arc::new(secrets),
        ssh_pool: Arc::new(SshPool::default()),
        auth: Arc::new(auth),
        info: Arc::new(ServerInfo {
            server_id: "test-id".into(),
            version: "0.0.0",
            desktop_mode: false,
            configured_host: "0.0.0.0".into(),
            port: 9090,
            hostname: "test".into(),
            magic_dns: None,
            bind: tokio::sync::watch::channel("127.0.0.1".to_owned()).0,
        }),
    };
    (build_router(state.clone()), state)
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .expect("request");
    let res = app.clone().oneshot(req).await.expect("response");
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, json)
}

fn cookie_of(headers: &axum::http::HeaderMap) -> String {
    let set = headers
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .expect("set-cookie");
    assert!(set.contains("HttpOnly") && set.contains("SameSite=Strict"));
    set.split(';').next().expect("pair").to_owned()
}

#[tokio::test]
async fn protected_routes_need_credentials() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, _) = app(&dir, true).await;
    for uri in [
        "/api/printers",
        "/api/attention",
        "/api/auth/me",
        "/api/printers/1/files?path=/",
        "/ws/events",
        "/ws/printers/1/terminal",
    ] {
        let (s, _, b) = call(&app, "GET", uri, &[], None).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{uri}");
        assert_eq!(b["code"], "unauthorized");
    }
    let (s, _, _) = call(&app, "GET", "/api/health", &[], None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, b) = call(&app, "GET", "/api/info", &[], None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["needs_setup"], false);
    assert_eq!(b["server_id"], "test-id");
    let (s, _, _) = call(
        &app,
        "GET",
        "/api/printers",
        &[("authorization", "Bearer fms_forged")],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn setup_code_creates_admin_once() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, state) = app(&dir, false).await;

    let (_, _, b) = call(&app, "GET", "/api/info", &[], None).await;
    assert_eq!(b["needs_setup"], true);
    let (s, _, b) = call(&app, "GET", "/api/printers", &[], None).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("setup_required"))
    );

    let setup = |code: &str, pw: &str| json!({ "code": code, "username": "admin", "password": pw });
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/auth/setup",
        &[],
        Some(setup("WRNG-CODE", PW)),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/auth/setup",
        &[],
        Some(setup(CODE, "short")),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "min length 10");
    let (s, h, b) = call(&app, "POST", "/api/auth/setup", &[], Some(setup(CODE, PW))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["username"], "admin");
    let cookie = cookie_of(&h);

    // Single use: the code is spent and a second setup is refused.
    let used = state
        .auth
        .setup_code
        .lock()
        .map(|c| c.as_ref().map(|c| c.check(CODE, std::time::Instant::now())))
        .expect("lock");
    assert_eq!(used, Some(Err(crate::auth::SetupCodeError::Used)));
    let (s, _, _) = call(&app, "POST", "/api/auth/setup", &[], Some(setup(CODE, PW))).await;
    assert_eq!(s, StatusCode::CONFLICT);

    let (s, _, b) = call(&app, "GET", "/api/auth/me", &[("cookie", &cookie)], None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["username"], "admin");
}

#[tokio::test]
async fn login_rate_limited_after_five_failures() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, _) = app(&dir, true).await;
    let body = |pw: &str| json!({ "username": "admin", "password": pw });
    for i in 1..=5 {
        let (s, _, b) = call(
            &app,
            "POST",
            "/api/auth/login",
            &[],
            Some(body("wrong one!")),
        )
        .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "try {i}");
        if i < 5 {
            assert!(b["error"].as_str().unwrap_or("").contains("tries left"));
        }
    }
    let (s, h, b) = call(&app, "POST", "/api/auth/login", &[], Some(body(PW))).await;
    assert_eq!(
        s,
        StatusCode::TOO_MANY_REQUESTS,
        "even the right password waits"
    );
    assert_eq!(b["code"], "rate_limited");
    assert!(h.get(header::RETRY_AFTER).is_some());
}

#[tokio::test]
async fn login_logout_and_api_keys() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, _) = app(&dir, true).await;

    // Browser login → cookie only (no token in the body).
    let (s, h, b) = call(
        &app,
        "POST",
        "/api/auth/login",
        &[],
        Some(json!({ "username": "admin", "password": PW, "remember": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(b.get("token").is_none());
    let cookie = cookie_of(&h);
    assert!(h
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("Max-Age=2592000")));

    // Token client login → bearer.
    let (_, _, b) = call(
        &app,
        "POST",
        "/api/auth/login",
        &[],
        Some(json!({ "username": "admin", "password": PW, "return_token": true })),
    )
    .await;
    let bearer = format!("Bearer {}", b["token"].as_str().expect("token"));
    let (s, _, _) = call(
        &app,
        "GET",
        "/api/printers",
        &[("authorization", &bearer)],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // API key: created once, works via X-Api-Key, can't manage keys, revocable.
    let (s, _, b) = call(
        &app,
        "POST",
        "/api/auth/api-keys",
        &[("cookie", &cookie)],
        Some(json!({ "name": "OrcaSlicer" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let secret = b["secret"].as_str().expect("secret").to_owned();
    let id = b["key"]["id"].as_i64().expect("id");
    let (s, _, _) = call(
        &app,
        "GET",
        "/api/printers",
        &[("x-api-key", &secret)],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, list) = call(
        &app,
        "GET",
        "/api/auth/api-keys",
        &[("cookie", &cookie)],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(!list.to_string().contains(&secret), "secret never listed");
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/auth/api-keys",
        &[("x-api-key", &secret)],
        Some(json!({ "name": "nope" })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, _) = call(
        &app,
        "DELETE",
        &format!("/api/auth/api-keys/{id}"),
        &[("cookie", &cookie)],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = call(
        &app,
        "GET",
        "/api/printers",
        &[("x-api-key", &secret)],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // Logout ends the cookie session; the bearer session is separate.
    let (s, h, _) = call(
        &app,
        "POST",
        "/api/auth/logout",
        &[("cookie", &cookie)],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(h
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("Max-Age=0")));
    let (s, _, _) = call(&app, "GET", "/api/printers", &[("cookie", &cookie)], None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, _) = call(
        &app,
        "GET",
        "/api/printers",
        &[("authorization", &bearer)],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
}

async fn bearer_login(app: &Router) -> String {
    let (_, _, b) = call(
        app,
        "POST",
        "/api/auth/login",
        &[],
        Some(json!({ "username": "admin", "password": PW, "return_token": true })),
    )
    .await;
    b["token"].as_str().expect("token").to_owned()
}

#[tokio::test]
async fn info_name_addresses_and_rename() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, _) = app(&dir, true).await;
    let (_, _, b) = call(&app, "GET", "/api/info", &[], None).await;
    assert_eq!(b["name"], "test", "falls back to the hostname");
    assert_eq!(
        b["addresses"],
        json!([{ "address": "127.0.0.1:9090", "kind": "loopback" }]),
        "loopback-bound server only reports loopback"
    );

    let bearer = format!("Bearer {}", bearer_login(&app).await);
    let (s, _, _) = call(
        &app,
        "PUT",
        "/api/settings/server",
        &[("authorization", &bearer)],
        Some(json!({ "name": "  " })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = call(
        &app,
        "PUT",
        "/api/settings/server",
        &[("authorization", &bearer)],
        Some(json!({ "name": "Workshop Pi" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, _, b) = call(&app, "GET", "/api/info", &[], None).await;
    assert_eq!(b["name"], "Workshop Pi");
    assert_eq!(b["server_id"], "test-id", "id never changes");
}

#[tokio::test]
async fn unclaimed_server_only_answers_health_info_setup() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, _) = app(&dir, false).await;
    for uri in ["/api/health", "/api/info"] {
        let (s, _, _) = call(&app, "GET", uri, &[], None).await;
        assert_eq!(s, StatusCode::OK, "{uri}");
    }
    for (method, uri) in [
        ("GET", "/api/printers"),
        ("GET", "/api/auth/me"),
        ("POST", "/api/auth/login"),
        ("POST", "/api/auth/url-token"),
        ("GET", "/ws/events"),
    ] {
        let (s, _, b) = call(&app, method, uri, &[], Some(json!({}))).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri}");
        assert_eq!(b["error"], "server not set up");
    }
}

#[tokio::test]
async fn setup_code_rate_limited_after_five_wrong_codes() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, _) = app(&dir, false).await;
    let body = |code: &str| json!({ "code": code, "username": "admin", "password": PW });
    for i in 1..=5 {
        let (s, _, _) = call(
            &app,
            "POST",
            "/api/auth/setup",
            &[],
            Some(body("WRNG-CODE")),
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN, "try {i}");
    }
    let (s, h, _) = call(&app, "POST", "/api/auth/setup", &[], Some(body(CODE))).await;
    assert_eq!(
        s,
        StatusCode::TOO_MANY_REQUESTS,
        "even the right code waits"
    );
    assert!(h.get(header::RETRY_AFTER).is_some());
}

#[tokio::test]
async fn setup_keeps_configured_host() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, state) = app(&dir, false).await;
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/auth/setup",
        &[],
        Some(json!({ "code": CODE, "username": "admin", "password": PW })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(state.info.bound_host(), "0.0.0.0", "configured host");
}

#[tokio::test]
async fn desktop_app_session_works_before_setup() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, state) = app(&dir, false).await;
    let (token, _) = core_auth::create_session(&state.foreman_db.pool, 3600, None, None)
        .await
        .expect("session");
    let bearer = format!("Bearer {token}");
    let (s, _, b) = call(
        &app,
        "GET",
        "/api/auth/me",
        &[("authorization", &bearer)],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["username"], crate::auth::LOCAL_APP_USER);
    let (s, _, b) = call(&app, "GET", "/api/printers", &[], None).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("setup_required"))
    );
}

#[tokio::test]
async fn url_tokens_scoped_session_tokens_refused() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, _) = app(&dir, true).await;
    let session = bearer_login(&app).await;
    let bearer = format!("Bearer {session}");

    // Session tokens never work in a URL.
    for uri in [
        format!("/api/printers?token={session}"),
        format!("/api/printers/999/snapshot?token={session}"),
        format!("/ws/events?token={session}"),
    ] {
        let (s, _, _) = call(&app, "GET", &uri, &[], None).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{uri}");
    }

    // Only snapshot / download / WebSocket paths can get a URL token.
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/auth/url-token",
        &[("authorization", &bearer)],
        Some(json!({ "path": "/api/printers" })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, b) = call(
        &app,
        "POST",
        "/api/auth/url-token",
        &[("authorization", &bearer)],
        Some(json!({ "path": "/api/printers/999/snapshot?t=1" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["expires_in"], 60);
    let url_token = b["token"].as_str().expect("token").to_owned();

    let (s, _, _) = call(
        &app,
        "GET",
        &format!("/api/printers/999/snapshot?t=2&token={url_token}"),
        &[],
        None,
    )
    .await;
    assert_ne!(s, StatusCode::UNAUTHORIZED, "valid for its own path");
    for uri in [
        format!("/api/printers/998/snapshot?token={url_token}"),
        format!("/api/printers?token={url_token}"),
        format!("/ws/events?token={url_token}"),
    ] {
        let (s, _, _) = call(&app, "GET", &uri, &[], None).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{uri}");
    }
}
