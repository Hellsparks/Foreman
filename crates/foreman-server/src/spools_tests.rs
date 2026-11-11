//! Load spool flow (task 4.1): load / unload on mock drivers, guard + override,
//! Bambu read-only, job need, compat with a loaded spool.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use foreman_core::{
    compat::{self, FilamentMatch, Verdict},
    domain::{CreatePrinter, FileMeta, FirmwareKind, QueueSource, ToolUsage},
    driver::{PrinterCapabilities, PrinterCommand, PrinterDriver, PrinterFile},
    error::AppError,
    events::{AmsTray, PrinterState, PrinterStatus},
    filament::JobNeed,
    repo,
};
use serde_json::{json, Value};
use spoolman_rs::{
    config::{DbType, SpoolmanConfig},
    db::{filament, spool, Db},
    models::{CreateFilamentRequest, CreateSpoolRequest},
};
use tower::ServiceExt;

use crate::{
    auth_api_tests::app,
    spools::{self, library_meta::MetaSource},
    state::AppState,
};

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

/// A Moonraker-like driver: one active spool, a `[spoolman]` link.
pub(crate) struct MockKlipper {
    link: Option<bool>,
    pub(crate) active: Mutex<Option<i64>>,
    sets: Mutex<Vec<Option<i64>>>,
}

impl MockKlipper {
    pub(crate) fn new(link: Option<bool>) -> Arc<Self> {
        Arc::new(Self {
            link,
            active: Mutex::new(None),
            sets: Mutex::new(vec![]),
        })
    }
    fn active(&self) -> Option<i64> {
        *self.active.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[async_trait]
impl PrinterDriver for MockKlipper {
    async fn connect(&self) -> Result<PrinterCapabilities, AppError> {
        Ok(PrinterCapabilities::default())
    }
    async fn disconnect(&self) -> Result<(), AppError> {
        Ok(())
    }
    async fn probe_capabilities(&self) -> Result<PrinterCapabilities, AppError> {
        Ok(PrinterCapabilities {
            spoolman_connected: self.link,
            spoolman_spool_id: self.active(),
            ..Default::default()
        })
    }
    async fn send_command(&self, _cmd: PrinterCommand) -> Result<(), AppError> {
        Ok(())
    }
    async fn list_files(&self, _p: Option<&str>) -> Result<Vec<PrinterFile>, AppError> {
        Ok(vec![])
    }
    async fn upload_file(&self, _p: &str, _d: Vec<u8>) -> Result<(), AppError> {
        Ok(())
    }
    async fn delete_file(&self, _p: &str) -> Result<(), AppError> {
        Ok(())
    }
    async fn start_print(&self, _p: &str) -> Result<(), AppError> {
        panic!("tests never start a print")
    }
    async fn current_status(&self) -> Result<PrinterStatus, AppError> {
        Ok(idle())
    }
    async fn set_active_spool(&self, tool: u32, id: Option<i64>) -> Result<(), AppError> {
        assert_eq!(tool, 0);
        *self.active.lock().unwrap_or_else(|p| p.into_inner()) = id;
        self.sets.lock().unwrap_or_else(|p| p.into_inner()).push(id);
        Ok(())
    }
}

/// A Bambu-like driver: AMS trays, no way to set them (default trait method).
struct MockBambu;

#[async_trait]
impl PrinterDriver for MockBambu {
    async fn connect(&self) -> Result<PrinterCapabilities, AppError> {
        Ok(PrinterCapabilities::default())
    }
    async fn disconnect(&self) -> Result<(), AppError> {
        Ok(())
    }
    async fn probe_capabilities(&self) -> Result<PrinterCapabilities, AppError> {
        Ok(bambu_caps())
    }
    async fn send_command(&self, _cmd: PrinterCommand) -> Result<(), AppError> {
        Ok(())
    }
    async fn list_files(&self, _p: Option<&str>) -> Result<Vec<PrinterFile>, AppError> {
        Ok(vec![])
    }
    async fn upload_file(&self, _p: &str, _d: Vec<u8>) -> Result<(), AppError> {
        Ok(())
    }
    async fn delete_file(&self, _p: &str) -> Result<(), AppError> {
        Ok(())
    }
    async fn start_print(&self, _p: &str) -> Result<(), AppError> {
        panic!("tests never start a print")
    }
    async fn current_status(&self) -> Result<PrinterStatus, AppError> {
        Ok(idle())
    }
}

fn bambu_caps() -> PrinterCapabilities {
    PrinterCapabilities {
        ams_slot_count: Some(4),
        enclosed: Some(false),
        max_nozzle_temp: Some(300.0),
        ..Default::default()
    }
}

pub(crate) fn idle() -> PrinterStatus {
    PrinterStatus {
        state: PrinterState::Idle,
        ..Default::default()
    }
}

pub(crate) struct NoMeta(pub(crate) Option<FileMeta>);

#[async_trait]
impl MetaSource for NoMeta {
    async fn meta(&self, _file_id: i64) -> Result<Option<FileMeta>, AppError> {
        Ok(self.0.clone())
    }
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

async fn spoolman(dir: &tempfile::TempDir) -> spools::Spoolman {
    let cfg = SpoolmanConfig {
        db_type: DbType::Sqlite,
        db_host: None,
        db_port: None,
        db_name: None,
        db_username: None,
        db_password: None,
        data_dir: dir.path().join("sm"),
        logs_dir: dir.path().join("sm"),
        backups_dir: dir.path().join("sm/backups"),
        host: "127.0.0.1".into(),
        port: 0,
        base_path: String::new(),
        automatic_backup: false,
    };
    std::fs::create_dir_all(dir.path().join("sm")).expect("sm dir");
    let db = Db::connect(&cfg).await.expect("spoolman db");
    spools::Spoolman::new(db, cfg)
}

/// Add a filament + spool; returns the spool id.
pub(crate) async fn add_spool(
    sm: &spools::Spoolman,
    material: &str,
    color: &str,
    remaining: f64,
) -> i64 {
    let f: CreateFilamentRequest = serde_json::from_value(json!({
        "density": 1.24, "diameter": 1.75, "material": material,
        "name": format!("{material} Test"), "color_hex": color,
    }))
    .expect("filament req");
    let f = filament::create(&sm.db.pool, &sm.db.engine, &f)
        .await
        .expect("filament");
    let s: CreateSpoolRequest = serde_json::from_value(
        json!({"filament_id": f.id, "initial_weight": 1000.0, "remaining_weight": remaining}),
    )
    .expect("spool req");
    spool::create(&sm.db.pool, &sm.db.engine, &s)
        .await
        .expect("spool")
        .id
}

pub(crate) async fn add_printer(state: &AppState, name: &str, firmware: FirmwareKind) -> i64 {
    repo::printer::create(
        &state.foreman_db.pool,
        CreatePrinter {
            name: name.into(),
            firmware,
            primary_url: None,
            api_key: None,
            serial_number: None,
            access_code: None,
            tags: vec![],
            hardened_nozzle: false,
            slicer_profiles: vec![],
            sort_order: None,
            fail_after_checks: None,
            return_after_secs: None,
        },
    )
    .await
    .expect("printer")
    .id
}

pub(crate) async fn setup() -> (tempfile::TempDir, Router, AppState, String) {
    let dir = tempfile::TempDir::new().expect("dir");
    let (router, state) = app(&dir, true).await;
    let sm = spoolman(&dir).await;
    assert!(state.spoolman.set(sm).is_ok());
    let token = login(&router).await;
    (dir, router, state, token)
}

async fn login(app: &Router) -> String {
    let (_, b) = call(
        app,
        "POST",
        "/api/auth/login",
        "",
        Some(json!({ "username": "admin", "password": crate::auth_api_tests::PW, "return_token": true })),
    )
    .await;
    format!("Bearer {}", b["token"].as_str().expect("token"))
}

pub(crate) async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    auth: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    if !auth.is_empty() {
        req = req.header(header::AUTHORIZATION, auth);
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
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn audit_lines(state: &AppState) -> Vec<String> {
    repo::audit_log::recent(&state.foreman_db.pool, 50)
        .await
        .expect("audit")
        .into_iter()
        .map(|a| a.command)
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn load_and_unload_on_a_klipper_printer() {
    let (_d, app, state, auth) = setup().await;
    let sm = state.spoolman.get().expect("sm");
    let sid = add_spool(sm, "PLA", "1d1d22", 800.0).await;
    let pid = add_printer(&state, "HellV0", FirmwareKind::Moonraker).await;
    let mock = MockKlipper::new(Some(true));
    state
        .driver_manager
        .insert_test_driver(
            pid,
            mock.clone(),
            PrinterCapabilities::default(),
            Some(idle()),
        )
        .await;

    let (s, v) = call(
        &app,
        "GET",
        &format!("/api/printers/{pid}/spools"),
        &auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v[0]["label"], "T0");
    assert_eq!(v[0]["writable"], true);
    assert!(v[0]["spool_id"].is_null());

    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/printers/{pid}/spools/load"),
        &auth,
        Some(json!({ "tool": 0, "spool_id": sid })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["slot"]["spool_id"], sid);
    assert_eq!(v["slot"]["spool"]["material"], "PLA");
    assert_eq!(v["overridden"], false);
    assert_eq!(mock.active(), Some(sid), "driver was told");

    // The Spools list shows where it is.
    let (_, v) = call(&app, "GET", "/api/spools", &auth, None).await;
    assert_eq!(v[0]["spool"]["id"], sid);
    assert_eq!(v[0]["loaded"][0]["printer_name"], "HellV0");
    assert_eq!(v[0]["loaded"][0]["label"], "T0");
    assert_eq!(v[0]["spool"]["color_hex"], "1D1D22");

    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/printers/{pid}/spools/unload"),
        &auth,
        Some(json!({ "tool": 0 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["spool_id"].is_null());
    assert_eq!(mock.active(), None);
    let lines = audit_lines(&state).await;
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with(&format!("spool.load #{sid} PLA Test → T0"))),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with(&format!("spool.unload #{sid}"))),
        "{lines:?}"
    );

    // Unauthenticated is refused.
    let (s, _) = call(&app, "GET", "/api/spools", "", None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn blocking_guard_needs_an_override_reason_and_is_audited() {
    let (_d, app, state, auth) = setup().await;
    let sm = state.spoolman.get().expect("sm");
    let sid = add_spool(sm, "PA12-CF", "2b2b2b", 480.0).await;
    let pid = add_printer(&state, "HellV0", FirmwareKind::Moonraker).await;
    let mock = MockKlipper::new(Some(true));
    state
        .driver_manager
        .insert_test_driver(
            pid,
            mock.clone(),
            PrinterCapabilities::default(),
            Some(idle()),
        )
        .await;

    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/printers/{pid}/spools/check"),
        &auth,
        Some(json!({ "tool": 0, "spool_id": sid })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["blocked"], true);
    assert_eq!(v["warnings"][0]["code"], "abrasive");
    assert_eq!(v["warnings"][0]["severity"], "block");

    // No reason (or blank): refused, nothing set.
    for body in [
        json!({ "tool": 0, "spool_id": sid }),
        json!({ "tool": 0, "spool_id": sid, "override_reason": "  " }),
    ] {
        let (s, v) = call(
            &app,
            "POST",
            &format!("/api/printers/{pid}/spools/load"),
            &auth,
            Some(body),
        )
        .await;
        assert_eq!(s, StatusCode::CONFLICT, "{v}");
        assert_eq!(v["blocked"], true);
        assert_eq!(v["warnings"][0]["code"], "abrasive");
    }
    assert_eq!(mock.active(), None);

    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/printers/{pid}/spools/load"),
        &auth,
        Some(json!({ "tool": 0, "spool_id": sid, "override_reason": "hardened nozzle fitted today" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["overridden"], true);
    assert_eq!(mock.active(), Some(sid));
    let lines = audit_lines(&state).await;
    assert!(
        lines.iter().any(|l| l.contains("OVERRIDE: hardened nozzle fitted today") && l.contains("abrasive")),
        "{lines:?}"
    );
}

#[tokio::test]
async fn low_spool_warns_but_loads_and_candidates_are_guarded() {
    let (_d, app, state, auth) = setup().await;
    let sm = state.spoolman.get().expect("sm");
    let low = add_spool(sm, "PLA", "ffffff", 40.0).await;
    let ok = add_spool(sm, "PLA", "ffffff", 900.0).await;
    let cf = add_spool(sm, "PLA-CF", "000000", 900.0).await;
    let pid = add_printer(&state, "HellV0", FirmwareKind::Moonraker).await;
    let mock = MockKlipper::new(Some(true));
    state
        .driver_manager
        .insert_test_driver(
            pid,
            mock.clone(),
            PrinterCapabilities::default(),
            Some(idle()),
        )
        .await;

    let (s, v) = call(
        &app,
        "GET",
        &format!("/api/printers/{pid}/spools/candidates?tool=0"),
        &auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let find = |id: i64| {
        v.as_array()
            .and_then(|a| a.iter().find(|c| c["spool"]["id"] == id))
            .cloned()
            .expect("cand")
    };
    assert_eq!(find(low)["warnings"][0]["code"], "low_remaining");
    assert_eq!(find(low)["blocked"], false);
    assert_eq!(find(ok)["warnings"].as_array().map(Vec::len), Some(0));
    assert_eq!(find(cf)["blocked"], true);

    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/printers/{pid}/spools/load"),
        &auth,
        Some(json!({ "tool": 0, "spool_id": low })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["warnings"][0]["code"], "low_remaining");
    assert_eq!(v["warnings"][0]["severity"], "warn");
    assert_eq!(mock.active(), Some(low));
}

#[tokio::test]
async fn job_need_comes_from_the_next_queued_job() {
    let (_d, _app, state, _auth) = setup().await;
    let pid = add_printer(&state, "HellV0", FirmwareKind::Moonraker).await;
    state
        .driver_manager
        .insert_test_driver(
            pid,
            MockKlipper::new(Some(true)),
            PrinterCapabilities::default(),
            Some(idle()),
        )
        .await;
    let pool = &state.foreman_db.pool;
    let meta = FileMeta {
        tools: vec![ToolUsage {
            index: 0,
            material: Some("PLA".into()),
            grams: Some(58.0),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(
        spools::job_need(
            pool,
            &state.driver_manager,
            &NoMeta(Some(meta.clone())),
            pid,
            0
        )
        .await,
        None,
        "nothing queued"
    );
    repo::queue::add(
        pool,
        7,
        "chess_4.gcode",
        Some(pid),
        QueueSource::Manual,
        "admin",
    )
    .await
    .expect("queue");
    assert_eq!(
        spools::job_need(pool, &state.driver_manager, &NoMeta(Some(meta)), pid, 0).await,
        Some(JobNeed {
            grams: 58.0,
            what: "the next queued job (chess_4.gcode)".into()
        })
    );
}

#[tokio::test]
async fn loading_moves_a_spool_off_another_printer() {
    let (_d, app, state, auth) = setup().await;
    let sm = state.spoolman.get().expect("sm");
    let sid = add_spool(sm, "PETG", "ff0000", 700.0).await;
    let a = add_printer(&state, "Alpha", FirmwareKind::Moonraker).await;
    let b = add_printer(&state, "Beta", FirmwareKind::Moonraker).await;
    let ma = MockKlipper::new(Some(true));
    let mb = MockKlipper::new(Some(true));
    *ma.active.lock().unwrap_or_else(|p| p.into_inner()) = Some(sid);
    for (id, m) in [(a, ma.clone()), (b, mb.clone())] {
        state
            .driver_manager
            .insert_test_driver(id, m, PrinterCapabilities::default(), Some(idle()))
            .await;
    }
    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/printers/{b}/spools/check"),
        &auth,
        Some(json!({ "tool": 0, "spool_id": sid })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["warnings"][0]["code"], "loaded_elsewhere");
    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/printers/{b}/spools/load"),
        &auth,
        Some(json!({ "tool": 0, "spool_id": sid })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["unloaded_from"][0]["printer_name"], "Alpha");
    assert_eq!(ma.active(), None);
    assert_eq!(mb.active(), Some(sid));
}

#[tokio::test]
async fn no_spoolman_link_or_offline_is_read_only() {
    let (_d, app, state, auth) = setup().await;
    let sm = state.spoolman.get().expect("sm");
    let sid = add_spool(sm, "PLA", "ffffff", 900.0).await;
    let pid = add_printer(&state, "NoLink", FirmwareKind::Moonraker).await;
    let mock = MockKlipper::new(None);
    state
        .driver_manager
        .insert_test_driver(
            pid,
            mock.clone(),
            PrinterCapabilities::default(),
            Some(idle()),
        )
        .await;
    let (_, v) = call(
        &app,
        "GET",
        &format!("/api/printers/{pid}/spools"),
        &auth,
        None,
    )
    .await;
    assert_eq!(v[0]["writable"], false);
    assert!(
        v[0]["read_only_reason"]
            .as_str()
            .is_some_and(|r| r.contains("[spoolman]")),
        "{v}"
    );
    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/printers/{pid}/spools/load"),
        &auth,
        Some(json!({ "tool": 0, "spool_id": sid })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["read_only"], true);
    assert!(mock
        .sets
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .is_empty());
}

#[tokio::test]
async fn bambu_trays_are_read_only_in_cloud_mode() {
    let (_d, app, state, auth) = setup().await;
    let sm = state.spoolman.get().expect("sm");
    let sid = add_spool(sm, "PLA", "ffffff", 900.0).await;
    let pid = add_printer(&state, "A1", FirmwareKind::Bambu).await;
    let status = PrinterStatus {
        state: PrinterState::Idle,
        ams_trays: vec![AmsTray {
            slot: 1,
            color: Some("FF0000".into()),
            material: Some("PETG".into()),
            remaining: Some(60),
        }],
        ..Default::default()
    };
    state
        .driver_manager
        .insert_test_driver(pid, Arc::new(MockBambu), bambu_caps(), Some(status))
        .await;

    let (s, v) = call(
        &app,
        "GET",
        &format!("/api/printers/{pid}/spools"),
        &auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let slots = v.as_array().expect("slots");
    assert_eq!(slots.len(), 4);
    assert!(slots.iter().all(|s| s["writable"] == false));
    assert!(slots[0]["read_only_reason"]
        .as_str()
        .is_some_and(|r| r.contains("cloud mode")));
    assert!(slots[0]["tray"].is_null(), "slot 1 is empty");
    assert_eq!(slots[1]["label"], "AMS 1 · slot 2");
    assert_eq!(slots[1]["tray"]["material"], "PETG");

    // The guard still reads the tray: a PLA spool vs a PETG tray mismatches.
    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/printers/{pid}/spools/check"),
        &auth,
        Some(json!({ "tool": 1, "spool_id": sid })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["warnings"][0]["code"], "tray_mismatch");
    assert!(v["warnings"].as_array().is_some_and(|w| !w.is_empty()));

    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/printers/{pid}/spools/load"),
        &auth,
        Some(json!({ "tool": 1, "spool_id": sid })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["read_only"], true);
    assert!(v["error"]
        .as_str()
        .is_some_and(|e| e.contains("cloud mode")));
    let (s, _) = call(
        &app,
        "POST",
        &format!("/api/printers/{pid}/spools/unload"),
        &auth,
        Some(json!({ "tool": 1 })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn compat_improves_with_a_loaded_spool() {
    let (_d, _app, state, _auth) = setup().await;
    let sm = state.spoolman.get().expect("sm");
    let sid = add_spool(sm, "PLA", "fafafa", 800.0).await;
    let pid = add_printer(&state, "HellV0", FirmwareKind::Moonraker).await;
    let mock = MockKlipper::new(Some(true));
    state
        .driver_manager
        .insert_test_driver(
            pid,
            mock.clone(),
            PrinterCapabilities::default(),
            Some(idle()),
        )
        .await;
    let meta = FileMeta {
        tools: vec![ToolUsage {
            index: 0,
            material: Some("PLA".into()),
            colour: Some("#FFFFFF".into()),
            grams: Some(20.0),
            ..Default::default()
        }],
        ..Default::default()
    };
    let printer = repo::printer::get(&state.foreman_db.pool, pid)
        .await
        .expect("p");

    let snap = spools::snapshot(&state.driver_manager, Some(sm), printer.clone()).await;
    let before = compat::check(&meta, &snap);
    assert_eq!(before.verdict, Verdict::NoSpool, "{}", before.reason);

    spools::set_slot(&state.driver_manager, pid, 0, Some(sid))
        .await
        .expect("load");
    let snap = spools::snapshot(&state.driver_manager, Some(sm), printer).await;
    let after = compat::check(&meta, &snap);
    assert_eq!(after.verdict, Verdict::Ready, "{}", after.reason);
    assert_eq!(after.filament, FilamentMatch::Colour);
    assert!(
        after.reason.contains(&format!("#{sid} PLA Test")),
        "{}",
        after.reason
    );
}

#[tokio::test]
async fn edit_spool_fields() {
    let (_d, app, state, auth) = setup().await;
    let sm = state.spoolman.get().expect("sm");
    let sid = add_spool(sm, "PLA", "ffffff", 900.0).await;
    let (s, v) = call(
        &app,
        "PATCH",
        &format!("/api/spools/{sid}"),
        &auth,
        Some(json!({ "location": "Shelf B1", "lot_nr": "L-42", "comment": "opened" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["location"], "Shelf B1");
    assert_eq!(v["lot_nr"], "L-42");
    assert_eq!(v["comment"], "opened");
    // Archive with the other fields null: they stay as they were.
    let (s, v) = call(
        &app,
        "PATCH",
        &format!("/api/spools/{sid}"),
        &auth,
        Some(json!({ "archived": true, "location": null, "lot_nr": null, "comment": null, "price": null })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["archived"], true);
    assert_eq!(v["location"], "Shelf B1");
    let (_, v) = call(&app, "GET", "/api/spools", &auth, None).await;
    assert_eq!(
        v.as_array().map(Vec::len),
        Some(0),
        "archived spools hidden by default"
    );
    let (_, v) = call(&app, "GET", "/api/spools?archived=true", &auth, None).await;
    assert_eq!(v[0]["spool"]["archived"], true);
    let (s, _) = call(
        &app,
        "PATCH",
        "/api/spools/999",
        &auth,
        Some(json!({ "comment": "x" })),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(
        &app,
        "PATCH",
        &format!("/api/spools/{sid}"),
        &auth,
        Some(json!({ "price": -1.0 })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
