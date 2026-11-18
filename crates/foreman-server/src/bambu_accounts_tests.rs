//! Bambu Cloud accounts against a mock Bambu: the three sign-in paths, independence of
//! accounts, no retries, and the token never leaving the server in clear.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use foreman_core::bambu_cloud::BambuCloud;
use serde_json::{json, Value};

use crate::{
    api::bambu_accounts::BambuSignins,
    auth_api_tests::app,
    build_router,
    spools_tests::{call, login},
};

#[derive(Default)]
struct Hits {
    login: AtomicUsize,
    send_code: AtomicUsize,
    tfa: AtomicUsize,
}

async fn m_login(State(h): State<Arc<Hits>>, Json(b): Json<Value>) -> impl IntoResponse {
    h.login.fetch_add(1, Ordering::SeqCst);
    let account = b["account"].as_str().unwrap_or("");
    let ok = |t: &str| {
        (
            StatusCode::OK,
            Json(
                json!({"accessToken": t, "refreshToken": t, "expiresIn": 7_776_000, "loginType": ""}),
            ),
        )
    };
    let bad = |m: &str| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"code": 1, "error": m})),
        )
    };
    if let Some(code) = b["code"].as_str() {
        return if account == "email@x" && code == "123456" {
            ok("TOKEN-EMAIL")
        } else {
            bad("Incorrect code")
        };
    }
    match (account, b["password"].as_str().unwrap_or("")) {
        ("plain@x", "pw-plain") => ok("TOKEN-PLAIN"),
        ("email@x", "pw-email") => (
            StatusCode::OK,
            Json(json!({"success": false, "loginType": "verifyCode"})),
        ),
        ("totp@x", "pw-totp") => (
            StatusCode::OK,
            Json(json!({"success": false, "loginType": "tfa", "tfaKey": "K1"})),
        ),
        _ => bad("Incorrect password"),
    }
}

async fn m_send_code(State(h): State<Arc<Hits>>, Json(b): Json<Value>) -> impl IntoResponse {
    h.send_code.fetch_add(1, Ordering::SeqCst);
    assert_eq!(b["type"], "codeLogin");
    Json(json!({"success": true}))
}

async fn m_tfa(State(h): State<Arc<Hits>>, Json(b): Json<Value>) -> impl IntoResponse {
    h.tfa.fetch_add(1, Ordering::SeqCst);
    if b["tfaKey"] == "K1" && b["tfaCode"] == "654321" {
        (
            [(header::SET_COOKIE, "token=TOKEN-TOTP; Path=/; HttpOnly")],
            Json(json!({})),
        )
            .into_response()
    } else {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"code": 2, "error": "Invalid code"})),
        )
            .into_response()
    }
}

fn bearer(h: &HeaderMap) -> String {
    h.get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .to_owned()
}

const GOOD: [&str; 4] = ["TOKEN-PLAIN", "TOKEN-EMAIL", "TOKEN-TOTP", "TOKEN-PASTE"];

async fn m_profile(h: HeaderMap) -> impl IntoResponse {
    if GOOD.contains(&bearer(&h).as_str()) {
        (
            StatusCode::OK,
            Json(json!({"uid": 42, "account": "x", "name": "x"})),
        )
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"code": 1001, "error": "Invalid Token"})),
        )
    }
}

async fn m_bind(h: HeaderMap) -> impl IntoResponse {
    let t = bearer(&h);
    if GOOD.contains(&t.as_str()) {
        (
            StatusCode::OK,
            Json(
                json!({"message": "success", "code": null, "error": null, "devices": [
                    {"dev_id": format!("0309DA{t}"), "name": "Shop A1", "online": true, "print_status": "SUCCESS",
                     "dev_model_name": "N2S", "dev_product_name": "A1", "dev_access_code": "12345678"}
                ]}),
            ),
        )
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"code": 1001, "error": "Invalid Token"})),
        )
    }
}

async fn mock() -> (String, Arc<Hits>) {
    let hits = Arc::new(Hits::default());
    let r = Router::new()
        .route("/v1/user-service/user/login", post(m_login))
        .route("/v1/user-service/user/sendemail/code", post(m_send_code))
        .route("/api/sign-in/tfa", post(m_tfa))
        .route("/v1/user-service/my/profile", get(m_profile))
        .route("/v1/iot-service/api/user/bind", get(m_bind))
        .with_state(Arc::clone(&hits));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let base = format!("http://{}", l.local_addr().expect("addr"));
    tokio::spawn(async move { axum::serve(l, r).await });
    (base, hits)
}

async fn setup() -> (
    tempfile::TempDir,
    Router,
    crate::state::AppState,
    String,
    Arc<Hits>,
) {
    let dir = tempfile::TempDir::new().expect("dir");
    let (_, mut state) = app(&dir, true).await;
    let (base, hits) = mock().await;
    let cloud = BambuCloud::new()
        .expect("cloud")
        .with_bases(base.clone(), base);
    state.bambu = Arc::new(BambuSignins::with_cloud(cloud));
    let router = build_router(state.clone(), std::path::PathBuf::from("frontend/dist"));
    let tok = login(&router).await;
    (dir, router, state, tok, hits)
}

async fn add(r: &Router, t: &str, email: &str) -> i64 {
    let (st, b) = call(
        r,
        "POST",
        "/api/bambu/accounts",
        t,
        Some(json!({"label": email, "region": "global", "email": email})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    assert_eq!(b["state"], "signed_out");
    b["id"].as_i64().expect("id")
}

#[tokio::test]
async fn password_path_stores_sealed_token_and_lists_devices() {
    let (_d, r, state, t, hits) = setup().await;
    let id = add(&r, &t, "plain@x").await;
    let (st, b) = call(
        &r,
        "POST",
        &format!("/api/bambu/accounts/{id}/signin"),
        &t,
        Some(json!({"password": "pw-plain"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["state"], "connected");
    assert!(b["days_left"]
        .as_i64()
        .is_some_and(|d| (89..=90).contains(&d)));
    assert!(!b.to_string().contains("TOKEN-"), "token leaked: {b}");
    assert_eq!(hits.login.load(Ordering::SeqCst), 1);

    let raw: String = sqlx::query_scalar("SELECT token FROM bambu_account WHERE id = ?")
        .bind(id)
        .fetch_one(&state.foreman_db.pool)
        .await
        .expect("row");
    assert!(
        raw.starts_with("enc:v1:") && !raw.contains("TOKEN"),
        "{raw}"
    );
    assert_eq!(state.secrets.open(&raw).expect("open"), "TOKEN-PLAIN");
    let row: Value = {
        let all: Vec<(String, String)> =
            sqlx::query_as("SELECT label, COALESCE(token,'') FROM bambu_account")
                .fetch_all(&state.foreman_db.pool)
                .await
                .expect("all");
        json!(all)
    };
    assert!(!row.to_string().contains("pw-plain"), "password stored");

    let (st, d) = call(
        &r,
        "GET",
        &format!("/api/bambu/accounts/{id}/devices"),
        &t,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{d}");
    assert_eq!(d["devices"][0]["dev_product_name"], "A1");
    assert_eq!(d["devices"][0]["online"], true);
    assert!(
        d["devices"][0].get("dev_access_code").is_none(),
        "access code passed through"
    );
}

#[tokio::test]
async fn email_code_path_one_form_wrong_code_waits() {
    let (_d, r, _s, t, hits) = setup().await;
    let id = add(&r, &t, "email@x").await;
    let (st, b) = call(
        &r,
        "POST",
        &format!("/api/bambu/accounts/{id}/signin"),
        &t,
        Some(json!({"password": "pw-email"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["state"], "code_required");
    assert_eq!(b["code_channel"], "email");
    assert_eq!(hits.send_code.load(Ordering::SeqCst), 1);

    let (st, b) = call(
        &r,
        "POST",
        &format!("/api/bambu/accounts/{id}/code"),
        &t,
        Some(json!({"code": "000000"})),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert!(b["error"].as_str().unwrap_or("").contains("Incorrect code"));
    // No retry: one password post + one code post.
    assert_eq!(hits.login.load(Ordering::SeqCst), 2);
    let (_, list) = call(&r, "GET", "/api/bambu/accounts", &t, None).await;
    assert_eq!(list[0]["state"], "code_required");

    let (st, b) = call(
        &r,
        "POST",
        &format!("/api/bambu/accounts/{id}/code"),
        &t,
        Some(json!({"code": "123456"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["state"], "connected");
    assert_eq!(b["code_channel"], Value::Null);
}

#[tokio::test]
async fn authenticator_path_uses_tfa_cookie() {
    let (_d, r, _s, t, hits) = setup().await;
    let id = add(&r, &t, "totp@x").await;
    let (_, b) = call(
        &r,
        "POST",
        &format!("/api/bambu/accounts/{id}/signin"),
        &t,
        Some(json!({"password": "pw-totp"})),
    )
    .await;
    assert_eq!(b["code_channel"], "authenticator");
    assert_eq!(hits.send_code.load(Ordering::SeqCst), 0);
    let (st, b) = call(
        &r,
        "POST",
        &format!("/api/bambu/accounts/{id}/code"),
        &t,
        Some(json!({"code": "654321"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["state"], "connected");
    assert_eq!(hits.tfa.load(Ordering::SeqCst), 1);
    let (st, _) = call(
        &r,
        "GET",
        &format!("/api/bambu/accounts/{id}/devices"),
        &t,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
}

#[tokio::test]
async fn pasted_token_validated_before_saving() {
    let (_d, r, _s, t, hits) = setup().await;
    let id = add(&r, &t, "paste@x").await;
    let (st, b) = call(
        &r,
        "POST",
        &format!("/api/bambu/accounts/{id}/token"),
        &t,
        Some(json!({"token": "nope"})),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    let (_, list) = call(&r, "GET", "/api/bambu/accounts", &t, None).await;
    assert_eq!(list[0]["has_token"], false);
    assert_eq!(list[0]["state"], "error");

    let (st, b) = call(
        &r,
        "POST",
        &format!("/api/bambu/accounts/{id}/token"),
        &t,
        Some(json!({"token": " TOKEN-PASTE \n"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["state"], "connected");
    assert_eq!(
        hits.login.load(Ordering::SeqCst),
        0,
        "token paste must skip login"
    );
}

#[tokio::test]
async fn failing_account_does_not_disturb_another() {
    let (_d, r, state, t, hits) = setup().await;
    let good = add(&r, &t, "plain@x").await;
    let bad = add(&r, &t, "plain@x-typo").await;
    call(
        &r,
        "POST",
        &format!("/api/bambu/accounts/{good}/signin"),
        &t,
        Some(json!({"password": "pw-plain"})),
    )
    .await;
    let (st, b) = call(
        &r,
        "POST",
        &format!("/api/bambu/accounts/{bad}/signin"),
        &t,
        Some(json!({"password": "wrong"})),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        b["error"]
            .as_str()
            .unwrap_or("")
            .contains("Incorrect password"),
        "{b}"
    );
    assert_eq!(hits.login.load(Ordering::SeqCst), 2, "no automatic retry");

    let (_, list) = call(&r, "GET", "/api/bambu/accounts", &t, None).await;
    let by_id = |id: i64| {
        list.as_array()
            .expect("arr")
            .iter()
            .find(|a| a["id"] == id)
            .cloned()
            .expect("acct")
    };
    assert_eq!(by_id(good)["state"], "connected");
    assert_eq!(by_id(good)["last_error"], Value::Null);
    assert_eq!(by_id(bad)["state"], "error");
    let (st, _) = call(
        &r,
        "GET",
        &format!("/api/bambu/accounts/{good}/devices"),
        &t,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    // A mistyped re-authentication on a working account keeps its token and state.
    let (st, _) = call(
        &r,
        "POST",
        &format!("/api/bambu/accounts/{good}/signin"),
        &t,
        Some(json!({"password": "oops"})),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, list) = call(&r, "GET", "/api/bambu/accounts", &t, None).await;
    assert_eq!(list[0]["state"], "connected");

    // Removing reports and unlinks referencing printers.
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO printer (name, firmware, created_at, updated_at, bambu_account_id) VALUES ('A1','bambu',?,?,?)")
        .bind(&now)
        .bind(&now)
        .bind(good)
        .execute(&state.foreman_db.pool)
        .await
        .expect("printer");
    let (_, list) = call(&r, "GET", "/api/bambu/accounts", &t, None).await;
    assert_eq!(list[0]["printer_count"], 1);
    let (st, _) = call(
        &r,
        "DELETE",
        &format!("/api/bambu/accounts/{good}"),
        &t,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, list) = call(&r, "GET", "/api/bambu/accounts", &t, None).await;
    assert_eq!(list.as_array().map(Vec::len), Some(1));
}
