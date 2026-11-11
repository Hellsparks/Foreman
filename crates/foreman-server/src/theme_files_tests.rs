//! Theme asset routes: public, data-dir only, no traversal.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use tower::ServiceExt;

use crate::auth_api_tests::app;
use crate::theme_files::valid_font_name;

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Option<String>, Vec<u8>) {
    let req = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request");
    let res = app.clone().oneshot(req).await.expect("response");
    let status = res.status();
    let ct = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 24)
        .await
        .expect("body");
    (status, ct, bytes.to_vec())
}

#[test]
fn font_names() {
    assert!(valid_font_name("Inter-Var.woff2"));
    assert!(valid_font_name("jet_brains.mono.woff2"));
    assert!(!valid_font_name(".woff2"));
    assert!(!valid_font_name(".hidden.woff2"));
    assert!(!valid_font_name("x.ttf"));
    assert!(!valid_font_name("../secret.key.woff2"));
    assert!(!valid_font_name("a..b.woff2"));
    assert!(!valid_font_name("dir/x.woff2"));
    assert!(!valid_font_name("a\\b.woff2"));
}

#[tokio::test]
async fn serves_fonts_and_custom_css_without_auth() {
    let dir = tempfile::tempdir().expect("tempdir");
    // with_admin = true: the server is claimed, so /api/* would need credentials.
    let (app, _) = app(&dir, true).await;

    // Absent files are 404, not 401.
    assert_eq!(
        get(&app, "/theme/custom.css").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/theme/fonts/a.woff2").await.0,
        StatusCode::NOT_FOUND
    );

    std::fs::create_dir_all(dir.path().join("fonts")).expect("mkdir");
    std::fs::write(dir.path().join("fonts/a.woff2"), b"wOF2fake").expect("write");
    std::fs::write(dir.path().join("custom.css"), b".x{color:red}").expect("write");

    let (st, ct, body) = get(&app, "/theme/fonts/a.woff2").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(ct.as_deref(), Some("font/woff2"));
    assert_eq!(body, b"wOF2fake");

    let (st, ct, body) = get(&app, "/theme/custom.css").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(ct.as_deref(), Some("text/css; charset=utf-8"));
    assert_eq!(body, b".x{color:red}");

    // Only .woff2 inside fonts/; nothing else in the data dir is reachable.
    assert_eq!(
        get(&app, "/theme/fonts/..%2Fsecret.key").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/theme/fonts/foreman.db").await.0,
        StatusCode::NOT_FOUND
    );
}
