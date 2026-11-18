//! Bambu in cloud mode: Send, Start next, Pause, Resume and Cancel are refused
//! before they reach the driver, with the compat engine's reason.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use axum::{http::StatusCode, Router};
use foreman_core::{
    bambu_cloud::Region,
    compat::{Reach, CLOUD_MODE, CLOUD_ONLY},
    domain::{FirmwareKind, QueueSource},
    driver::{PrinterCapabilities, PrinterCommand, PrinterDriver, PrinterFile},
    error::AppError,
    events::{PrinterState, PrinterStatus},
    repo,
};
use serde_json::{json, Value};

use crate::{
    spools_tests::{add_printer, call, idle, setup, MockKlipper},
    state::AppState,
};

/// Counts every call that would touch the printer.
#[derive(Default)]
struct CountingBambu {
    calls: AtomicUsize,
}

impl CountingBambu {
    fn hit(&self) -> Result<(), AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(AppError::NotImplemented("driver reached".into()))
    }
}

#[async_trait]
impl PrinterDriver for CountingBambu {
    async fn connect(&self) -> Result<PrinterCapabilities, AppError> {
        Ok(PrinterCapabilities::default())
    }
    async fn disconnect(&self) -> Result<(), AppError> {
        Ok(())
    }
    async fn probe_capabilities(&self) -> Result<PrinterCapabilities, AppError> {
        Ok(PrinterCapabilities::default())
    }
    async fn send_command(&self, _cmd: PrinterCommand) -> Result<(), AppError> {
        self.hit()
    }
    async fn list_files(&self, _p: Option<&str>) -> Result<Vec<PrinterFile>, AppError> {
        Ok(vec![])
    }
    async fn upload_file(&self, _p: &str, _d: Vec<u8>) -> Result<(), AppError> {
        self.hit()
    }
    async fn delete_file(&self, _p: &str) -> Result<(), AppError> {
        Ok(())
    }
    async fn start_print(&self, _p: &str) -> Result<(), AppError> {
        self.hit()
    }
    async fn current_status(&self) -> Result<PrinterStatus, AppError> {
        Ok(idle())
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    app: Router,
    state: AppState,
    auth: String,
    bambu: i64,
    driver: Arc<CountingBambu>,
    file: i64,
}

async fn rig(state_now: PrinterState) -> Rig {
    let (_dir, app, state, auth) = setup().await;
    let bambu = add_printer(&state, "A1", FirmwareKind::Bambu).await;
    let driver = Arc::new(CountingBambu::default());
    let status = PrinterStatus {
        state: state_now,
        ..Default::default()
    };
    state
        .driver_manager
        .insert_test_driver(
            bambu,
            driver.clone(),
            PrinterCapabilities::default(),
            Some(status),
        )
        .await;
    let tmp = state.library.tmp_dir().join("cube.gcode");
    std::fs::write(&tmp, "; filament_type = PLA\nG1 X1 E1\n").expect("seed");
    let file = state
        .library
        .add_file("", "cube.gcode", &tmp)
        .await
        .expect("add")
        .id;
    Rig {
        _dir,
        app,
        state,
        auth,
        bambu,
        driver,
        file,
    }
}

fn assert_refused(s: StatusCode, v: &Value) {
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(
        v.to_string().contains(CLOUD_MODE),
        "reason missing from {v}"
    );
}

impl Rig {
    fn driver_calls(&self) -> usize {
        self.driver.calls.load(Ordering::SeqCst)
    }
}

#[tokio::test]
async fn compat_says_not_supported_and_ui_fields_carry_it() {
    let r = rig(PrinterState::Idle).await;
    let (s, v) = call(
        &r.app,
        "GET",
        &format!("/api/send/check/{}", r.file),
        &r.auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v[0]["verdict"], "unsupported");
    assert_eq!(v[0]["reason"], CLOUD_MODE);

    let (_, v) = call(&r.app, "GET", "/api/printers", &r.auth, None).await;
    assert_eq!(v[0]["control_unsupported"], CLOUD_MODE);

    let (_, v) = call(&r.app, "GET", "/api/queue", &r.auth, None).await;
    let lane = v["lanes"]
        .as_array()
        .and_then(|l| l.iter().find(|l| l["printer_id"] == r.bambu))
        .expect("bambu lane");
    assert_eq!(lane["control_unsupported"], CLOUD_MODE);
}

#[tokio::test]
async fn send_print_now_is_refused_before_the_driver() {
    let r = rig(PrinterState::Idle).await;
    for dry_run in [false, true] {
        let (s, v) = call(
            &r.app,
            "POST",
            "/api/send",
            &r.auth,
            Some(
                json!({ "file_id": r.file, "printer_id": r.bambu, "mode": "print_now",
                         "plate_confirm": true, "dry_run": dry_run }),
            ),
        )
        .await;
        assert_refused(s, &v);
        assert_eq!(v["verdict"], "unsupported");
    }
    assert_eq!(r.driver_calls(), 0);
}

#[tokio::test]
async fn send_to_the_printer_queue_is_refused() {
    let r = rig(PrinterState::Idle).await;
    let (s, v) = call(
        &r.app,
        "POST",
        "/api/send",
        &r.auth,
        Some(json!({ "file_id": r.file, "printer_id": r.bambu, "mode": "queue" })),
    )
    .await;
    assert_refused(s, &v);
    let waiting = repo::queue::list_waiting(&r.state.foreman_db.pool, Some(r.bambu))
        .await
        .expect("list");
    assert!(waiting.is_empty());
}

#[tokio::test]
async fn start_next_is_refused_before_the_driver() {
    let r = rig(PrinterState::Idle).await;
    repo::queue::add(
        &r.state.foreman_db.pool,
        r.file,
        "cube.gcode",
        None,
        QueueSource::Manual,
        "admin",
    )
    .await
    .expect("fleet item");
    for dry_run in [false, true] {
        let (s, v) = call(
            &r.app,
            "POST",
            &format!("/api/printers/{}/queue/start-next", r.bambu),
            &r.auth,
            Some(json!({ "plate_confirm": true, "dry_run": dry_run })),
        )
        .await;
        assert_refused(s, &v);
    }
    assert_eq!(r.driver_calls(), 0);
}

#[tokio::test]
async fn pause_resume_cancel_are_refused_before_the_driver() {
    let r = rig(PrinterState::Printing).await;
    for kind in ["pause", "resume"] {
        let (s, v) = call(
            &r.app,
            "POST",
            &format!("/api/printers/{}/commands/{kind}", r.bambu),
            &r.auth,
            Some(json!({})),
        )
        .await;
        assert_refused(s, &v);
    }
    let (s, tok) = call(
        &r.app,
        "POST",
        &format!("/api/printers/{}/confirm-cancel", r.bambu),
        &r.auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{tok}");
    let (s, v) = call(
        &r.app,
        "POST",
        &format!("/api/printers/{}/commands/cancel", r.bambu),
        &r.auth,
        Some(json!({ "confirm_token": tok["confirm_token"] })),
    )
    .await;
    assert_refused(s, &v);
    assert_eq!(r.driver_calls(), 0);

    // A Klipper printer next to it still takes the same command.
    let klipper = add_printer(&r.state, "HellV0", FirmwareKind::Moonraker).await;
    r.state
        .driver_manager
        .insert_test_driver(
            klipper,
            MockKlipper::new(None),
            PrinterCapabilities::default(),
            None,
        )
        .await;
    let (s, v) = call(
        &r.app,
        "POST",
        &format!("/api/printers/{klipper}/commands/pause"),
        &r.auth,
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
}

// ---------------------------------------------------------------------------
// Bambu Cloud path (doc 07 §B): reachable only over the cloud = watch, can't send
// ---------------------------------------------------------------------------

/// Link the rig's Bambu to a cloud account. With `lan`, it also has a LAN address
/// and access code (LAN first, cloud last).
async fn link_cloud(r: &Rig, lan: bool) {
    let pool = &r.state.foreman_db.pool;
    let acct = repo::bambu_account::create(pool, "Thomas", Region::Global, "t@example.com")
        .await
        .expect("account");
    sqlx::query("UPDATE printer SET serial_number = '03919C461200718' WHERE id = ?")
        .bind(r.bambu)
        .execute(pool)
        .await
        .expect("serial");
    if lan {
        sqlx::query(
            "UPDATE printer SET primary_url = '10.0.0.249', access_code = '12345678' WHERE id = ?",
        )
        .bind(r.bambu)
        .execute(pool)
        .await
        .expect("lan");
    }
    repo::printer::set_bambu_account(pool, r.bambu, Some(acct.id))
        .await
        .expect("link");
}

#[tokio::test]
async fn cloud_path_is_last_after_every_lan_path() {
    let r = rig(PrinterState::Idle).await;
    link_cloud(&r, true).await;
    let printer = repo::printer::get(&r.state.foreman_db.pool, r.bambu)
        .await
        .expect("printer");
    let names: Vec<String> = r
        .state
        .driver_manager
        .effective_paths(&printer)
        .await
        .expect("paths")
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["LAN", "Bambu Cloud"]);
    // On its LAN path the printer keeps the existing Bambu rule.
    assert_eq!(r.state.driver_manager.reach(r.bambu).await, Reach::Lan);

    // Stored paths stay first, in their order; cloud is still last.
    for (name, prio) in [("Tailscale", 10), ("Home", 0)] {
        repo::printer_path::create(
            &r.state.foreman_db.pool,
            foreman_core::domain::CreatePrinterPath {
                printer_id: r.bambu,
                name: name.into(),
                url: format!("{name}.invalid"),
                priority: Some(prio),
            },
        )
        .await
        .expect("path");
    }
    let names: Vec<String> = r
        .state
        .driver_manager
        .effective_paths(&printer)
        .await
        .expect("paths")
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["Home", "Tailscale", "Bambu Cloud"]);
}

#[tokio::test]
async fn unlinked_printers_keep_their_paths_unchanged() {
    let r = rig(PrinterState::Idle).await;
    let printer = repo::printer::get(&r.state.foreman_db.pool, r.bambu)
        .await
        .expect("printer");
    assert!(r
        .state
        .driver_manager
        .effective_paths(&printer)
        .await
        .expect("paths")
        .is_empty());
}

#[tokio::test]
async fn cloud_only_printer_gets_the_cloud_only_verdict_from_compat() {
    let r = rig(PrinterState::Idle).await;
    link_cloud(&r, false).await;
    assert_eq!(r.state.driver_manager.reach(r.bambu).await, Reach::Cloud);

    let (s, v) = call(
        &r.app,
        "GET",
        &format!("/api/send/check/{}", r.file),
        &r.auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v[0]["verdict"], "cloud_only");
    assert_eq!(v[0]["reason"], CLOUD_ONLY);

    let (_, v) = call(&r.app, "GET", "/api/printers", &r.auth, None).await;
    assert_eq!(v[0]["control_unsupported"], CLOUD_ONLY);
    assert_eq!(v[0]["reach"], "cloud");
    assert_eq!(v[0]["active_path_name"], "Bambu Cloud");

    // Print now is refused by compat (409 with the verdict), never by the driver.
    for dry_run in [false, true] {
        let (s, v) = call(
            &r.app,
            "POST",
            "/api/send",
            &r.auth,
            Some(
                json!({ "file_id": r.file, "printer_id": r.bambu, "mode": "print_now",
                         "plate_confirm": true, "dry_run": dry_run }),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::CONFLICT, "{v}");
        assert_eq!(v["verdict"], "cloud_only");
        assert_eq!(v["reason"], CLOUD_ONLY);
    }
    let (s, v) = call(
        &r.app,
        "POST",
        &format!("/api/printers/{}/commands/pause", r.bambu),
        &r.auth,
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v.to_string().contains(CLOUD_ONLY), "{v}");
    assert_eq!(r.driver_calls(), 0);
}
