//! Exports + swatches (task 5.5) through the REST API, from filaments stored
//! in Spoolman the way the filament form and shrinkage calibration store them.

use std::io::Read;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use spoolman_rs::db::{extra_field, filament as sm_filament};
use tower::ServiceExt;

use crate::spools_tests::{call, setup};

/// Like `call`, but returns the raw body and content type.
async fn raw(app: &Router, uri: &str, auth: &str, body: Value) -> (StatusCode, String, Vec<u8>) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::AUTHORIZATION, auth)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let res = app.clone().oneshot(req).await.expect("response");
    let status = res.status();
    let ct = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let bytes = axum::body::to_bytes(res.into_body(), 16 << 20)
        .await
        .expect("body");
    (status, ct, bytes.to_vec())
}

async fn filament(app: &Router, t: &str, f: Value) -> i64 {
    let (s, b) = call(app, "POST", "/api/filaments", t, Some(f)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b["id"].as_i64().expect("id")
}

/// Three filaments: a calibrated ASA with TD, a two-colour PLA with TD, a PETG HF without TD.
async fn seed() -> (tempfile::TempDir, Router, String, [i64; 3]) {
    let (dir, app, state, t) = setup().await;
    let sm = state.spoolman.get().expect("spoolman");
    extra_field::ensure_foreman_defaults(&sm.db.pool, &sm.db.engine)
        .await
        .expect("fields");
    let asa = filament(
        &app,
        &t,
        json!({ "vendor_name": "Prusament", "name": "Orange", "material": "ASA", "colour_kind": "single",
                "colours": ["#ff6600"], "hue_td": 4.5, "extruder_temp": 260, "bed_temp": 110 }),
    )
    .await;
    let bicolor = filament(
        &app,
        &t,
        json!({ "vendor_name": "Poly Alkemi", "name": "Blue Red", "material": "PLA", "modifiers": ["BiColor"],
                "colour_kind": "coaxial", "colours": ["#0033ff", "#ff0000"] }),
    )
    .await;
    let petg = filament(
        &app,
        &t,
        json!({ "vendor_name": "NextShapes", "name": "Taro Purple", "material": "PETG", "modifiers": ["HF"],
                "colour_kind": "single", "colours": ["#8a6fa8"] }),
    )
    .await;
    // HueForge TD on the bicolour (the form refuses it; Marathon data has it).
    let req: spoolman_rs::models::UpdateFilamentRequest = serde_json::from_value(json!({
        "extra": { "hue_td": "3.1" }
    }))
    .expect("req");
    sm_filament::update(&sm.db.pool, bicolor, &req)
        .await
        .expect("td");
    // Shrinkage through the calibration endpoint (session 32 path).
    let body = json!({ "input": {
        "x": [{ "nominal": 100.0, "readings": [99.5] }],
        "y": [{ "nominal": 100.0, "readings": [99.7] }],
        "z": [{ "nominal": 100.0, "readings": [99.8] }] } });
    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/calibration/shrinkage/{asa}"),
        &t,
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    (dir, app, t, [asa, bicolor, petg])
}

fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).expect("zip");
    (0..z.len())
        .map(|i| {
            let mut e = z.by_index(i).expect("entry");
            let name = e.name().expect("name").to_string();
            let mut b = Vec::new();
            e.read_to_end(&mut b).expect("read");
            (name, b)
        })
        .collect()
}

#[tokio::test]
async fn orca_export_carries_shrinkage_from_calibration() {
    let (_d, app, t, [asa, _, petg]) = seed().await;
    let (s, ct, bytes) = raw(
        &app,
        "/api/exports/orca",
        &t,
        json!({ "filament_ids": [asa] }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ct, "application/zip");
    let files = unzip(&bytes);
    assert_eq!(files.len(), 1);
    let (name, body) = &files[0];
    assert_eq!(name, &format!("Prusament ASA Orange (Foreman #{asa}).json"));
    let v: Value = serde_json::from_slice(body).expect("json");
    assert_eq!(v["inherits"], "Generic ASA @System");
    assert_eq!(v["filament_shrink"], json!(["100.402%"]));
    assert_eq!(v["filament_shrinkage_compensation_z"], json!(["100.2%"]));
    assert_eq!(v["nozzle_temperature"], json!(vec!["260"; 6]));
    assert_eq!(v["hot_plate_temp"], json!(["110"]));
    assert_eq!(v["default_filament_colour"], json!(["#FF6600"]));

    // All: one profile per filament.
    let (s, _, bytes) = raw(&app, "/api/exports/orca", &t, json!({})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(unzip(&bytes).len(), 3);
    // A selection keeps the order asked for.
    let (_, _, bytes) = raw(
        &app,
        "/api/exports/orca",
        &t,
        json!({ "filament_ids": [petg, asa] }),
    )
    .await;
    let names: Vec<String> = unzip(&bytes).into_iter().map(|f| f.0).collect();
    assert!(
        names[0].contains("Taro Purple") && names[1].contains("Orange"),
        "{names:?}"
    );
    // Unknown id → 404.
    let (s, _, _) = raw(
        &app,
        "/api/exports/orca",
        &t,
        json!({ "filament_ids": [999_999] }),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn hueforge_export_skips_multi_colour_and_lists_missing_td() {
    let (_d, app, t, [asa, bicolor, petg]) = seed().await;
    let (s, ct, bytes) = raw(&app, "/api/exports/hueforge", &t, json!({})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ct, "application/json");
    let v: Value = serde_json::from_slice(&bytes).expect("json");
    let list = v["Filaments"].as_array().expect("list");
    assert_eq!(list.len(), 1, "{v}");
    assert_eq!(list[0]["Brand"], "Prusament");
    assert_eq!(list[0]["Color"], "#ff6600");
    assert_eq!(list[0]["Transmissivity"], 4.5);
    assert_eq!(list[0]["Type"], "ASA");

    let (s, rows) = call(&app, "GET", "/api/exports", &t, None).await;
    assert_eq!(s, StatusCode::OK, "{rows}");
    let row = |id: i64| {
        rows.as_array()
            .expect("rows")
            .iter()
            .find(|r| r["id"] == id)
            .cloned()
            .expect("row")
    };
    assert_eq!(row(asa)["hueforge_skip"], Value::Null);
    assert_eq!(row(bicolor)["hueforge_skip"], "multi_colour");
    assert_eq!(row(petg)["hueforge_skip"], "missing_td");
    assert_eq!(row(asa)["orca"]["shrinkage_xy"], 100.402);
    assert_eq!(row(petg)["orca"]["inherits"], "Generic PETG HF @System");
    assert_eq!(row(petg)["orca"]["bambu_filament_id"], "GFG96");
    assert_eq!(row(asa)["swatch_line1"], "Prusament ASA");
    assert_eq!(row(asa)["swatch_line2"], "Orange");
}

#[tokio::test]
async fn swatch_stl_fit_and_batch() {
    let (_d, app, t, [asa, bicolor, _]) = seed().await;
    let (s, fit) = call(
        &app,
        "POST",
        "/api/swatch/fit",
        &t,
        Some(json!({ "line1": "Polymaker PolyTerra PLA Matte", "line2": "Teal" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(fit["line1"]["shrunk"], true);
    assert_eq!(fit["line2"]["shrunk"], false);
    assert_eq!(fit["error"], Value::Null);

    let (s, fit) = call(
        &app,
        "POST",
        "/api/swatch/fit",
        &t,
        Some(json!({ "line1": "W".repeat(40) })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        fit["error"].as_str().unwrap_or("").contains("too long"),
        "{fit}"
    );

    let (s, ct, stl) = raw(
        &app,
        "/api/swatch/stl",
        &t,
        json!({ "line1": "Prusament ASA", "line2": "Orange" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ct, "model/stl");
    let m = foreman_swatch::IndexedMesh::from_binary_stl(&stl).expect("stl");
    assert!(m.check().is_watertight(), "{}", m.check());

    let (s, _, _) = raw(
        &app,
        "/api/swatch/stl",
        &t,
        json!({ "line1": "W".repeat(40) }),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, ct, bytes) = raw(
        &app,
        "/api/swatch/batch",
        &t,
        json!({ "filament_ids": [asa, bicolor] }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ct, "application/zip");
    let files = unzip(&bytes);
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].0, format!("swatch-{asa}-prusament-asa-orange.stl"));
    for (_, b) in files {
        let m = foreman_swatch::IndexedMesh::from_binary_stl(&b).expect("stl");
        assert!(m.check().is_watertight());
    }
}

#[tokio::test]
async fn exports_need_login() {
    let (_d, app, _, _) = seed().await;
    let (s, _, _) = raw(&app, "/api/exports/orca", "", json!({})).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}
