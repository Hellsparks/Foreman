//! First-run setup (task 6.4): each step's validation, Spoolman detection + import
//! (real file when present, synthetic old / newer revisions, by URL), resume after a
//! restart, skip paths, the slicer key shown once, the backup default.
//!
//! Tests run with printer connections off, so scans and probes use the sample set.

use std::path::{Path, PathBuf};

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use spoolman_rs::db::{import as sm_import, migrations};
use sqlx::{sqlite::SqliteConnectOptions, ConnectOptions, Connection};
use tower::ServiceExt as _;

use crate::{
    auth_api_tests::app,
    spools_tests::{call, login, spoolman},
    state::AppState,
};

const REAL_FIXTURE: &str = "../spoolman-rs/tests/fixtures/real-spoolman-2026-04-06.db";

/// Server with an admin and a running (empty) built-in Spoolman.
async fn server(dir: &tempfile::TempDir, first: bool) -> (Router, AppState, String) {
    let (router, state) = app(dir, first).await;
    let sm = spoolman(dir).await;
    assert!(state.spoolman.set(sm).is_ok());
    let token = login(&router).await;
    (router, state, token)
}

async fn get_setup(app: &Router, t: &str) -> Value {
    let (st, b) = call(app, "GET", "/api/setup", t, None).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    b
}

/// A Spoolman SQLite file at the *initial* revision with 2 vendors, 3 filaments,
/// 4 spools — the import has to migrate it through the whole chain.
async fn old_revision_db(path: &Path) {
    let mut conn = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .connect()
        .await
        .expect("open");
    sqlx::raw_sql(sqlx::AssertSqlSafe(migrations::CHAIN[0].sql))
        .execute(&mut conn)
        .await
        .expect("initial schema");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE TABLE alembic_version (version_num VARCHAR(32) NOT NULL PRIMARY KEY);
         INSERT INTO alembic_version VALUES ('{}');
         INSERT INTO vendor (id, name) VALUES (1, 'Polymaker'), (2, 'Prusament');
         INSERT INTO filament (id, name, vendor_id, material, density, diameter, weight)
           VALUES (1, 'PolyLite', 1, 'PLA', 1.24, 1.75, 1000),
                  (2, 'PolyMax', 1, 'PETG', 1.27, 1.75, 1000),
                  (3, 'Galaxy Black', 2, 'PLA', 1.24, 1.75, 1000);
         INSERT INTO spool (id, filament_id, used_weight, location) VALUES
           (1, 1, 100, 'Shelf'), (2, 1, 0, NULL), (3, 2, 250.5, 'Dry box'), (7, 3, 999, NULL);",
        migrations::CHAIN[0].revision
    )))
    .execute(&mut conn)
    .await
    .expect("rows");
    conn.close().await.expect("close");
}

async fn set_revision(path: &Path, rev: &str) {
    let mut conn = SqliteConnectOptions::new()
        .filename(path)
        .connect()
        .await
        .expect("open");
    sqlx::query("UPDATE alembic_version SET version_num = ?")
        .bind(rev)
        .execute(&mut conn)
        .await
        .expect("set rev");
    conn.close().await.expect("close");
}

fn sha(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(std::fs::read(path).expect("read")))
}

async fn live_counts(state: &AppState) -> sm_import::Counts {
    sm_import::counts(&state.spoolman.get().expect("sm").db)
        .await
        .expect("counts")
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fresh_server_starts_the_wizard_and_steps_are_validated() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, _s, t) = server(&dir, true).await;
    let b = get_setup(&app, &t).await;
    assert_eq!(b["state"]["step"], "data");
    assert_eq!(b["needs_wizard"], true);
    assert_eq!(b["checklist"]["done"], 1, "only 'server running' at first");
    assert_eq!(b["checklist"]["total"], 5);

    // Printers / finish before a data source: refused.
    for step in ["printers", "finish"] {
        let (st, e) = call(
            &app,
            "POST",
            "/api/setup/step",
            &t,
            Some(json!({ "step": step })),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{step}: {e}");
    }
    let (st, _) = call(
        &app,
        "POST",
        "/api/setup/step",
        &t,
        Some(json!({ "step": "done" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "done only via finish");
    let (st, _) = call(&app, "POST", "/api/setup/finish", &t, None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "finish needs a data source");

    // Fresh start → printers.
    let (st, b) = call(&app, "POST", "/api/setup/data/fresh", &t, None).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["state"]["step"], "printers");
    assert_eq!(b["state"]["data_source"]["kind"], "fresh");
    let filament = &b["checklist"]["items"][1];
    assert_eq!(filament["done"], true);

    // Back to data and forward again are allowed.
    let (st, b) = call(
        &app,
        "POST",
        "/api/setup/step",
        &t,
        Some(json!({ "step": "data" })),
    )
    .await;
    assert_eq!(
        (st, b["state"]["step"].clone()),
        (StatusCode::OK, json!("data"))
    );
    let (st, _) = call(
        &app,
        "POST",
        "/api/setup/step",
        &t,
        Some(json!({ "step": "printers" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    // Skip the printer step: finish with none added.
    let (st, b) = call(
        &app,
        "POST",
        "/api/setup/step",
        &t,
        Some(json!({ "step": "finish" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(b["state"]["printers_skipped"], true);
    let (st, b) = call(&app, "POST", "/api/setup/finish", &t, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(b["state"]["step"], "done");
    assert_eq!(b["needs_wizard"], false);
    // The checklist still shows what is left (for the empty Fleet page).
    let printers = &b["checklist"]["items"][2];
    assert_eq!(
        (printers["id"].clone(), printers["done"].clone()),
        (json!("printers"), json!(false))
    );
}

#[tokio::test]
async fn skip_and_rerun_from_settings() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, _s, t) = server(&dir, true).await;
    let (st, b) = call(&app, "POST", "/api/setup/skip", &t, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        (b["state"]["step"].clone(), b["state"]["skipped"].clone()),
        (json!("done"), json!(true))
    );
    assert_eq!(b["needs_wizard"], false);
    assert_eq!(
        b["checklist"]["items"][1]["done"], false,
        "no data source chosen"
    );

    let (st, b) = call(&app, "POST", "/api/setup/restart", &t, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(b["state"]["step"], "data");
    assert_eq!(b["needs_wizard"], true);
}

#[tokio::test]
async fn existing_install_is_not_sent_to_the_wizard() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, s, t) = server(&dir, true).await;
    foreman_core::auth::create_api_key(&s.foreman_db.pool, "OrcaSlicer")
        .await
        .expect("key");
    let b = get_setup(&app, &t).await;
    assert_eq!(b["state"]["step"], "done");
    assert_eq!(b["state"]["existing_install"], true);
    assert_eq!(b["needs_wizard"], false);
}

#[tokio::test]
async fn resumes_where_it_stopped_after_a_restart() {
    let dir = tempfile::TempDir::new().expect("dir");
    let file = dir.path().join("old.db");
    old_revision_db(&file).await;
    {
        let (app, _s, t) = server(&dir, true).await;
        let (st, b) = call(
            &app,
            "POST",
            "/api/setup/data/detect-path",
            &t,
            Some(json!({ "path": file.to_string_lossy() })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{b}");
    }
    // "Restart": a new server process over the same data dir (admin exists already).
    let (app, s, t) = server(&dir, false).await;
    let b = get_setup(&app, &t).await;
    assert_eq!(b["state"]["step"], "data");
    assert_eq!(
        b["state"]["pending"]["counts"]["spools"], 4,
        "detection kept"
    );
    assert!(crate::setup::staged_db(&s).is_file(), "staged copy kept");
    // …and the import still works from there.
    let (st, b) = call(&app, "POST", "/api/setup/data/import", &t, Some(json!({}))).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["state"]["step"], "printers");

    // Leave on the printer step, restart again: still on printers.
    let (app, _s, t) = server(&dir, false).await;
    let b = get_setup(&app, &t).await;
    assert_eq!(b["state"]["step"], "printers");
    assert_eq!(b["state"]["data_source"]["kind"], "spoolman_file");
    assert_eq!(b["needs_wizard"], true);
}

// ---------------------------------------------------------------------------
// Spoolman detection + import
// ---------------------------------------------------------------------------

#[tokio::test]
async fn old_revision_file_is_detected_migrated_and_imported() {
    let dir = tempfile::TempDir::new().expect("dir");
    let file = dir.path().join("spoolman.db");
    old_revision_db(&file).await;
    let before = sha(&file);
    let (app, s, t) = server(&dir, true).await;

    let (st, p) = call(
        &app,
        "POST",
        "/api/setup/data/detect-path",
        &t,
        Some(json!({ "path": file.to_string_lossy() })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{p}");
    assert_eq!(p["engine"], "sqlite");
    assert_eq!(p["revision"], migrations::CHAIN[0].revision);
    assert_eq!(p["status"], "older");
    assert_eq!(p["steps_to_head"], migrations::CHAIN.len() as u64 - 1);
    assert_eq!(p["accepted"], true);
    assert_eq!(
        (
            p["counts"]["vendors"].clone(),
            p["counts"]["filaments"].clone(),
            p["counts"]["spools"].clone()
        ),
        (json!(2), json!(3), json!(4))
    );
    // Counts are shown before anything is committed.
    assert!(live_counts(&s).await.is_empty());

    let (st, b) = call(&app, "POST", "/api/setup/data/import", &t, Some(json!({}))).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    let c = live_counts(&s).await;
    assert_eq!((c.vendors, c.filaments, c.spools), (2, 3, 4));
    assert_eq!(b["state"]["data_source"]["counts"]["spools"], 4);
    assert_eq!(sha(&file), before, "the user's file is never changed");

    // Rows come back through the Spoolman API path with ids kept.
    let sm = s.spoolman.get().expect("sm");
    let spool = spoolman_rs::db::spool::find_by_id(&sm.db.pool, 7)
        .await
        .expect("spool 7");
    assert_eq!(spool.filament.name.as_deref(), Some("Galaxy Black"));
}

#[tokio::test]
async fn newer_unknown_revision_is_refused() {
    let dir = tempfile::TempDir::new().expect("dir");
    let file = dir.path().join("newer.db");
    old_revision_db(&file).await;
    set_revision(&file, "ffffffffffff").await;
    let (app, s, t) = server(&dir, true).await;
    let (st, p) = call(
        &app,
        "POST",
        "/api/setup/data/detect-path",
        &t,
        Some(json!({ "path": file.to_string_lossy() })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "detection itself succeeds: {p}");
    assert_eq!(p["status"], "unknown");
    assert_eq!(p["accepted"], false);
    assert!(p["refusal"]
        .as_str()
        .expect("refusal")
        .contains("ffffffffffff"));
    assert_eq!(p["counts"]["spools"], 4, "what was found is still shown");

    let (st, e) = call(
        &app,
        "POST",
        "/api/setup/data/import",
        &t,
        Some(json!({ "replace_existing": true })),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert!(live_counts(&s).await.is_empty(), "nothing imported");
    assert_eq!(
        get_setup(&app, &t).await["state"]["data_source"],
        Value::Null
    );
}

#[tokio::test]
async fn not_a_spoolman_file_is_refused() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, _s, t) = server(&dir, true).await;
    let text = dir.path().join("notes.txt");
    std::fs::write(&text, "hello, not a database").expect("write");
    let (st, e) = call(
        &app,
        "POST",
        "/api/setup/data/detect-path",
        &t,
        Some(json!({ "path": text.to_string_lossy() })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(
        e["error"].as_str().unwrap_or("").contains("not a SQLite"),
        "{e}"
    );

    // A SQLite file without alembic_version.
    let other = dir.path().join("other.db");
    let mut c = SqliteConnectOptions::new()
        .filename(&other)
        .create_if_missing(true)
        .connect()
        .await
        .expect("db");
    sqlx::raw_sql("CREATE TABLE x (id INTEGER)")
        .execute(&mut c)
        .await
        .expect("t");
    c.close().await.expect("close");
    let (st, p) = call(
        &app,
        "POST",
        "/api/setup/data/detect-path",
        &t,
        Some(json!({ "path": other.to_string_lossy() })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        (p["status"].clone(), p["accepted"].clone()),
        (json!("missing"), json!(false))
    );

    for bad in ["relative/path.db", "/nope/does-not-exist.db"] {
        let (st, _) = call(
            &app,
            "POST",
            "/api/setup/data/detect-path",
            &t,
            Some(json!({ "path": bad })),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{bad}");
    }
    // The refused detection stays on screen; importing it is refused.
    let (st, _) = call(&app, "POST", "/api/setup/data/import", &t, Some(json!({}))).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    let (st, _) = call(&app, "POST", "/api/setup/data/discard", &t, None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(&app, "POST", "/api/setup/data/import", &t, Some(json!({}))).await;
    assert_eq!(
        st,
        StatusCode::BAD_REQUEST,
        "nothing detected → nothing to import"
    );
}

#[tokio::test]
async fn import_over_existing_data_needs_confirmation() {
    let dir = tempfile::TempDir::new().expect("dir");
    let file = dir.path().join("old.db");
    old_revision_db(&file).await;
    let (app, s, t) = server(&dir, true).await;
    crate::spools_tests::add_spool(s.spoolman.get().expect("sm"), "ABS", "000000", 1000.0).await;
    let (st, _) = call(
        &app,
        "POST",
        "/api/setup/data/detect-path",
        &t,
        Some(json!({ "path": file.to_string_lossy() })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, e) = call(&app, "POST", "/api/setup/data/import", &t, Some(json!({}))).await;
    assert_eq!(st, StatusCode::CONFLICT, "{e}");
    assert!(e["error"].as_str().unwrap_or("").contains("1 spool"), "{e}");
    assert_eq!(live_counts(&s).await.spools, 1, "untouched");
    let (st, _) = call(
        &app,
        "POST",
        "/api/setup/data/import",
        &t,
        Some(json!({ "replace_existing": true })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(live_counts(&s).await.spools, 4, "replaced");
}

#[tokio::test]
async fn upload_detects_the_same_way() {
    let dir = tempfile::TempDir::new().expect("dir");
    let file = dir.path().join("upload.db");
    old_revision_db(&file).await;
    let (app, _s, t) = server(&dir, true).await;
    let boundary = "XfwBoundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"spoolman.db\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend(std::fs::read(&file).expect("read"));
    body.extend(format!("\r\n--{boundary}--\r\n").into_bytes());
    let req = Request::builder()
        .method("POST")
        .uri("/api/setup/data/detect-upload")
        .header(header::AUTHORIZATION, &t)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .expect("req");
    let res = app.clone().oneshot(req).await.expect("res");
    let st = res.status();
    let b: Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .expect("body"),
    )
    .unwrap_or(Value::Null);
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["label"], "spoolman.db");
    assert_eq!(b["counts"]["spools"], 4);
}

/// The owner's real Spoolman backup (gitignored; skipped in CI).
#[tokio::test]
async fn real_backup_file_imports_and_a_newer_copy_is_refused() {
    let real = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(REAL_FIXTURE);
    if !real.is_file() {
        eprintln!("no real Spoolman fixture at {}; skipped", real.display());
        return;
    }
    let dir = tempfile::TempDir::new().expect("dir");
    let file = dir.path().join("real.db");
    std::fs::copy(&real, &file).expect("copy");
    let insp = sm_import::inspect_sqlite(&file).await.expect("inspect");
    let (app, s, t) = server(&dir, true).await;

    let (st, p) = call(
        &app,
        "POST",
        "/api/setup/data/detect-path",
        &t,
        Some(json!({ "path": file.to_string_lossy() })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{p}");
    eprintln!("real file: {p}");
    assert_eq!(p["engine"], "sqlite");
    assert_eq!(p["accepted"], true);
    assert_eq!(p["revision"].as_str(), insp.revision.as_deref());
    let (st, b) = call(&app, "POST", "/api/setup/data/import", &t, Some(json!({}))).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    let c = live_counts(&s).await;
    assert_eq!(
        (c.vendors, c.filaments, c.spools),
        (
            insp.counts.vendors,
            insp.counts.filaments,
            insp.counts.spools
        )
    );
    // The file's extra field definitions are kept; Foreman adds its own on top.
    assert!(
        c.extra_fields >= insp.counts.extra_fields,
        "{} < {}",
        c.extra_fields,
        insp.counts.extra_fields
    );

    // The same file stamped with a revision from a newer Spoolman: refused.
    let newer = dir.path().join("real-newer.db");
    std::fs::copy(&real, &newer).expect("copy");
    set_revision(&newer, "aaaaaaaaaaaa").await;
    let (st, p) = call(
        &app,
        "POST",
        "/api/setup/data/detect-path",
        &t,
        Some(json!({ "path": newer.to_string_lossy() })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        (p["status"].clone(), p["accepted"].clone()),
        (json!("unknown"), json!(false))
    );
    let (st, _) = call(
        &app,
        "POST",
        "/api/setup/data/import",
        &t,
        Some(json!({ "replace_existing": true })),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
}

/// An external Spoolman (spoolman-rs on a local port) copied in over its REST API.
#[tokio::test]
async fn external_spoolman_by_url_is_copied_once() {
    // Source Spoolman with data, served on a random port.
    let src_dir = tempfile::TempDir::new().expect("dir");
    let file = src_dir.path().join("spoolman.db");
    old_revision_db(&file).await;
    let src_db = spoolman_rs::db::Db::connect(&sm_import::sqlite_config(src_dir.path()))
        .await
        .expect("src db");
    sqlx::query("UPDATE spool SET archived = TRUE WHERE id = 2")
        .execute(&src_db.pool)
        .await
        .expect("archive");
    let src_state =
        spoolman_rs::state::AppState::new(src_db, sm_import::sqlite_config(src_dir.path()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        let r = spoolman_rs::api::router("").with_state(src_state);
        let _ = axum::serve(listener, r).await;
    });

    let dir = tempfile::TempDir::new().expect("dir");
    let (app, s, t) = server(&dir, true).await;
    let (st, e) = call(
        &app,
        "POST",
        "/api/setup/data/detect-url",
        &t,
        Some(json!({ "url": "http://127.0.0.1:1" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "nothing there: {e}");
    let (st, p) = call(
        &app,
        "POST",
        "/api/setup/data/detect-url",
        &t,
        Some(json!({ "url": format!("127.0.0.1:{port}/api/v1/") })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{p}");
    assert_eq!(p["kind"], "spoolman_url");
    assert_eq!(p["label"], format!("http://127.0.0.1:{port}"));
    assert_eq!(p["accepted"], true);
    assert_eq!(p["counts"]["spools"], 4, "archived spool included");
    assert_eq!(p["counts"]["archived_spools"], 1);

    let (st, b) = call(&app, "POST", "/api/setup/data/import", &t, Some(json!({}))).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    let c = live_counts(&s).await;
    assert_eq!(
        (c.vendors, c.filaments, c.spools, c.archived_spools),
        (2, 3, 4, 1)
    );
    let spool = spoolman_rs::db::spool::find_by_id(&s.spoolman.get().expect("sm").db.pool, 3)
        .await
        .expect("spool 3");
    assert_eq!(spool.location.as_deref(), Some("Dry box"));
    assert!((spool.used_weight - 250.5).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// Printers (sample set: printer connections are off in tests)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discovery_and_adding_printers_are_validated() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, _s, t) = server(&dir, true).await;
    let (st, scan) = call(&app, "POST", "/api/setup/discover", &t, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(scan["sample"], true);
    assert_eq!(scan["printers"].as_array().map(Vec::len), Some(3));

    let add = |body: Value| {
        let app = app.clone();
        let t = t.clone();
        async move { call(&app, "POST", "/api/setup/printers", &t, Some(body)).await }
    };
    // Field rules.
    let (st, _) = add(json!({ "name": " ", "firmware": "moonraker", "host": "192.0.2.10" })).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "name required");
    let (st, _) = add(json!({ "name": "x", "firmware": "moonraker", "host": "" })).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "host required");
    let (st, _) = add(json!({ "name": "x", "firmware": "bambu", "host": "192.0.2.20", "serial": "123", "access_code": "12345678" })).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "serial format");
    let (st, _) = add(json!({ "name": "x", "firmware": "bambu", "host": "192.0.2.20", "serial": "03919C000000001", "access_code": "123" })).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "access code format");
    let (st, _) = add(json!({ "name": "x", "firmware": "octoprint", "host": "192.0.2.30" })).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "setup adds Moonraker + Bambu");

    // Credentials: the API-key sample refuses without / with a wrong key.
    let (st, e) =
        add(json!({ "name": "trident", "firmware": "moonraker", "host": "192.0.2.11" })).await;
    assert_eq!(
        (st, e["code"].clone()),
        (StatusCode::UNPROCESSABLE_ENTITY, json!("api_key"))
    );
    let (st, _) = add(json!({ "name": "trident", "firmware": "moonraker", "host": "192.0.2.11", "api_key": "wrong", "force": true })).await;
    assert_eq!(
        st,
        StatusCode::UNPROCESSABLE_ENTITY,
        "force never stores refused credentials"
    );
    let (st, b) = add(json!({ "name": "trident", "firmware": "moonraker", "host": "192.0.2.11", "api_key": "sample-key" })).await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    assert_eq!(
        b["printer"]["primary_url"], "http://192.0.2.11:7125",
        "probed URL stored"
    );

    let (st, b) =
        add(json!({ "name": "voron24", "firmware": "moonraker", "host": "192.0.2.10" })).await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    let (st, e) =
        add(json!({ "name": "Voron24", "firmware": "moonraker", "host": "192.0.2.99" })).await;
    assert_eq!(
        (st, e["code"].clone()),
        (StatusCode::CONFLICT, json!("duplicate_name"))
    );
    let (st, e) =
        add(json!({ "name": "again", "firmware": "moonraker", "host": "http://192.0.2.10/" }))
            .await;
    assert_eq!(
        (st, e["code"].clone()),
        (StatusCode::CONFLICT, json!("already_added"))
    );

    let (st, b) = add(json!({ "name": "A1", "firmware": "bambu", "host": "192.0.2.20", "serial": "03919C000000001", "access_code": "12345678" })).await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    assert_eq!(b["printer"]["serial_number"], "03919C000000001");

    // A rescan marks them as added.
    let (_, scan) = call(&app, "POST", "/api/setup/discover", &t, None).await;
    let added = scan["printers"]
        .as_array()
        .expect("list")
        .iter()
        .filter(|p| p["added_as"].is_number())
        .count();
    assert_eq!(added, 3);
    let b = get_setup(&app, &t).await;
    assert_eq!(b["checklist"]["items"][2]["title"], "3 printers added");
}

// ---------------------------------------------------------------------------
// Slicer key + backups
// ---------------------------------------------------------------------------

#[tokio::test]
async fn slicer_key_is_generated_once_and_shown_once() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, _s, t) = server(&dir, true).await;
    let (st, b) = call(&app, "POST", "/api/setup/slicer-key", &t, None).await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    let secret = b["secret"].as_str().expect("secret").to_owned();
    assert!(secret.len() > 20);
    assert!(b["urls"].as_array().is_some_and(|u| !u.is_empty()));

    let (st, e) = call(&app, "POST", "/api/setup/slicer-key", &t, None).await;
    assert_eq!(
        (st, e["code"].clone()),
        (StatusCode::CONFLICT, json!("exists"))
    );
    // Never shown again: not in the key list, not in setup.
    let (_, keys) = call(&app, "GET", "/api/auth/api-keys", &t, None).await;
    assert!(!keys.to_string().contains(&secret));
    let b = get_setup(&app, &t).await;
    assert!(!b.to_string().contains(&secret));
    assert_eq!(b["checklist"]["slicer"]["setup_key_exists"], true);
    assert_eq!(b["checklist"]["items"][3]["done"], true);

    // The key works as an OctoPrint-style X-Api-Key.
    let req = Request::builder()
        .uri("/api/version")
        .header("X-Api-Key", &secret)
        .body(Body::empty())
        .expect("req");
    assert_eq!(
        app.clone().oneshot(req).await.expect("res").status(),
        StatusCode::OK
    );
    // …but cannot make keys itself.
    let req = Request::builder()
        .method("POST")
        .uri("/api/setup/slicer-key")
        .header("X-Api-Key", &secret)
        .body(Body::empty())
        .expect("req");
    assert_eq!(
        app.clone().oneshot(req).await.expect("res").status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn backup_default_schedule_is_offered() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, _s, t) = server(&dir, true).await;
    let target = dir.path().join("backups");
    let (st, b) = call(
        &app,
        "POST",
        "/api/setup/backup",
        &t,
        Some(json!({ "enable": true, "dir": target.to_string_lossy() })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    let item = &b["checklist"]["items"][4];
    assert_eq!(item["done"], true);
    assert!(
        item["detail"]
            .as_str()
            .unwrap_or("")
            .starts_with("Daily at 03:00 · keep 7"),
        "{item}"
    );
    let (_, ov) = call(&app, "GET", "/api/backup", &t, None).await;
    assert_eq!(ov["settings"]["schedule"], "daily");
    assert_eq!(ov["settings"]["dir"], target.to_string_lossy().as_ref());

    let (st, _) = call(
        &app,
        "POST",
        "/api/setup/backup",
        &t,
        Some(json!({ "enable": true, "dir": "relative/dir" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "folder must be absolute");
    let (st, b) = call(
        &app,
        "POST",
        "/api/setup/backup",
        &t,
        Some(json!({ "enable": false })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(b["checklist"]["items"][4]["done"], false);
}
