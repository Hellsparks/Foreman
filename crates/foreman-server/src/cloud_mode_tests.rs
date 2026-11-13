//! Bambu in cloud mode: Send, Start next, Pause, Resume and Cancel are refused
//! before they reach the driver, with the compat engine's reason.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use axum::{http::StatusCode, Router};
use foreman_core::{
    compat::CLOUD_MODE,
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
