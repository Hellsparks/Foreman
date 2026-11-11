//! Calibration (task 5.4) through the REST API: skew preview / apply refusals,
//! and shrinkage written into the filament's `orcaslicer_config` extra field.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;
use foreman_core::{
    domain::FirmwareKind,
    driver::{PrinterCapabilities, PrinterCommand, PrinterDriver, PrinterFile},
    error::AppError,
    events::{PrinterState, PrinterStatus},
};
use serde_json::{json, Value};
use spoolman_rs::db::{extra_field, filament};

use crate::{
    spools_tests::{add_printer, add_spool, call, setup},
    state::AppState,
};

/// Records every command; never prints.
struct Recorder(Mutex<Vec<String>>);

impl Recorder {
    fn sent(&self) -> Vec<String> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

#[async_trait]
impl PrinterDriver for Recorder {
    async fn connect(&self) -> Result<PrinterCapabilities, AppError> {
        Ok(PrinterCapabilities::default())
    }
    async fn disconnect(&self) -> Result<(), AppError> {
        Ok(())
    }
    async fn probe_capabilities(&self) -> Result<PrinterCapabilities, AppError> {
        Ok(PrinterCapabilities::default())
    }
    async fn send_command(&self, cmd: PrinterCommand) -> Result<(), AppError> {
        let s = match cmd {
            PrinterCommand::Gcode { command } => command,
            c => format!("{c:?}"),
        };
        self.0.lock().unwrap_or_else(|p| p.into_inner()).push(s);
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
        Ok(PrinterStatus::default())
    }
}

async fn klipper(state: &AppState, st: Option<PrinterState>) -> (i64, Arc<Recorder>) {
    let id = add_printer(state, "V0", FirmwareKind::Moonraker).await;
    let rec = Arc::new(Recorder(Mutex::new(vec![])));
    state
        .driver_manager
        .insert_test_driver(
            id,
            rec.clone(),
            PrinterCapabilities::default(),
            st.map(|state| PrinterStatus {
                state,
                ..Default::default()
            }),
        )
        .await;
    (id, rec)
}

fn docs_xy() -> Value {
    json!({ "planes": [{ "plane": "XY", "ac": 140.4, "bd": 142.8, "ad": 99.8 }] })
}

async fn measure(app: &axum::Router, auth: &str, id: i64) -> i64 {
    let (s, v) = call(
        app,
        "POST",
        &format!("/api/calibration/skew/{id}/measurements"),
        auth,
        Some(docs_xy()),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_i64().expect("id")
}

#[tokio::test]
async fn preview_docs_example_and_exact_commands() {
    let (_d, app, _s, auth) = setup().await;
    let (s, v) = call(
        &app,
        "POST",
        "/api/calibration/skew/preview",
        &auth,
        Some(docs_xy()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let deg = v["planes"][0]["degrees"].as_f64().expect("deg");
    assert_eq!(format!("{deg:.2}"), "-0.97");
    assert_eq!(
        v["commands"],
        json!([
            "SET_SKEW XY=140.4,142.8,99.8",
            "SKEW_PROFILE SAVE=foreman",
            "SAVE_CONFIG"
        ])
    );
    assert_eq!(v["clear_command"], "SET_SKEW CLEAR=1");
    // Impossible numbers → 400.
    let (s, _) = call(
        &app,
        "POST",
        "/api/calibration/skew/preview",
        &auth,
        Some(json!({ "planes": [{ "plane": "XY", "ac": 140, "bd": 141, "ad": 200 }] })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn apply_refused_while_printing_paused_or_unknown() {
    let (_d, app, state, auth) = setup().await;
    for st in [
        Some(PrinterState::Printing),
        Some(PrinterState::Paused),
        None,
    ] {
        let (id, rec) = klipper(&state, st.clone()).await;
        let rid = measure(&app, &auth, id).await;
        let (s, v) = call(
            &app,
            "POST",
            &format!("/api/calibration/skew/{id}/apply"),
            &auth,
            Some(json!({ "record_id": rid, "confirm_restart": true })),
        )
        .await;
        match st {
            Some(_) => {
                assert_eq!(s, StatusCode::CONFLICT, "{v}");
                assert_eq!(v["code"], "printing");
            }
            None => assert_eq!(s, StatusCode::BAD_REQUEST, "{v}"),
        }
        let (s, _) = call(
            &app,
            "POST",
            &format!("/api/calibration/skew/{id}/clear"),
            &auth,
            Some(json!({})),
        )
        .await;
        assert_ne!(s, StatusCode::OK);
        assert!(
            rec.sent().is_empty(),
            "nothing may be sent: {:?}",
            rec.sent()
        );
    }
}

#[tokio::test]
async fn apply_needs_restart_confirmation_then_sends_in_order() {
    let (_d, app, state, auth) = setup().await;
    let (id, rec) = klipper(&state, Some(PrinterState::Idle)).await;
    let rid = measure(&app, &auth, id).await;
    let (s, _) = call(
        &app,
        "POST",
        &format!("/api/calibration/skew/{id}/apply"),
        &auth,
        Some(json!({ "record_id": rid })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(rec.sent().is_empty());

    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/calibration/skew/{id}/apply"),
        &auth,
        Some(json!({ "record_id": rid, "confirm_restart": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        rec.sent(),
        vec![
            "SET_SKEW XY=140.4,142.8,99.8",
            "SKEW_PROFILE SAVE=foreman",
            "SAVE_CONFIG"
        ]
    );
    assert!(v["record"]["applied_at"].is_string());

    let (_, list) = call(&app, "GET", "/api/calibration/skew", &auth, None).await;
    let p = list
        .as_array()
        .and_then(|a| a.iter().find(|p| p["id"] == id))
        .expect("printer");
    assert_eq!(p["supported"], true);
    assert_eq!(p["applied"]["id"], rid);
}

#[tokio::test]
async fn non_klipper_is_not_supported() {
    let (_d, app, state, auth) = setup().await;
    let id = add_printer(&state, "P1S", FirmwareKind::Bambu).await;
    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/calibration/skew/{id}/measurements"),
        &auth,
        Some(docs_xy()),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap_or("").contains("only Klipper"));
    let (_, list) = call(&app, "GET", "/api/calibration/skew", &auth, None).await;
    let p = list
        .as_array()
        .and_then(|a| a.iter().find(|p| p["id"] == id))
        .expect("printer");
    assert_eq!(p["supported"], false);
    assert!(p["unsupported"]
        .as_str()
        .unwrap_or("")
        .contains("carries any skew"));
}

#[tokio::test]
async fn shrinkage_round_trips_through_spoolman_extra_field() {
    let (_d, app, state, auth) = setup().await;
    let sm = state.spoolman.get().expect("sm");
    extra_field::ensure_foreman_defaults(&sm.db.pool, &sm.db.engine)
        .await
        .expect("fields");
    let spool = add_spool(sm, "ABS", "808080", 900.0).await;
    let fid = spoolman_rs::db::spool::find_by_id(&sm.db.pool, spool)
        .await
        .expect("spool")
        .filament
        .id;
    // An existing Orca config with another key must survive.
    let pre: spoolman_rs::models::UpdateFilamentRequest = serde_json::from_value(json!({
        "extra": { "orcaslicer_config": json!(r#"{"filament_flow_ratio":0.95}"#).to_string() }
    }))
    .expect("req");
    filament::update(&sm.db.pool, fid, &pre).await.expect("pre");

    let body = json!({
        "input": {
            "x": [{ "nominal": 100.0, "readings": [99.5] }],
            "y": [{ "nominal": 100.0, "readings": [99.7] }],
            "z": [{ "nominal": 100.0, "readings": [99.8] }]
        }
    });
    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/calibration/shrinkage/{fid}"),
        &auth,
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["result"]["shrinkage_xy"], 100.402);
    assert_eq!(v["result"]["shrinkage_z"], 100.2);

    // Read back through spoolman-rs: the extra value is a JSON string holding the object.
    let f = filament::find_by_id(&sm.db.pool, fid).await.expect("f");
    let raw = f
        .extra
        .get("orcaslicer_config")
        .and_then(Value::as_str)
        .expect("raw");
    let inner: String = serde_json::from_str(raw).expect("json string");
    let cfg: Value = serde_json::from_str(&inner).expect("object");
    assert_eq!(cfg["shrinkage_xy"], "100.402");
    assert_eq!(cfg["shrinkage_z"], "100.2");
    assert_eq!(cfg["filament_flow_ratio"], 0.95);

    // And through Foreman's list + history.
    let (_, list) = call(&app, "GET", "/api/calibration/shrinkage", &auth, None).await;
    let row = list
        .as_array()
        .and_then(|a| a.iter().find(|f| f["id"] == fid))
        .expect("row");
    assert_eq!(row["shrinkage_xy"], 100.402);
    assert_eq!(row["history"].as_array().map(Vec::len), Some(1));

    // Unknown filament → 404; implausible → 400.
    let (s, _) = call(
        &app,
        "POST",
        "/api/calibration/shrinkage/99999",
        &auth,
        Some(json!({ "input": { "x": [{ "nominal": 100.0, "readings": [99.5] }] } })),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(
        &app,
        "POST",
        &format!("/api/calibration/shrinkage/{fid}"),
        &auth,
        Some(json!({ "input": { "x": [{ "nominal": 100.0, "readings": [10.0] }] } })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
