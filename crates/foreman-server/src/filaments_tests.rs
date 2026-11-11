//! Filament form, vendors, new spools (task 4.2), through the REST API and
//! read back from Spoolman's own `/api/v1` shapes.

use axum::{http::StatusCode, Router};
use serde_json::{json, Value};
use spoolman_rs::db::{extra_field, filament as sm_filament, spool as sm_spool};

use crate::{
    spools::{self, Spoolman},
    spools_tests::{call, setup},
    state::AppState,
};

async fn ready() -> (tempfile::TempDir, Router, AppState, String) {
    let (dir, app, state, token) = setup().await;
    let sm = sm(&state);
    extra_field::ensure_foreman_defaults(&sm.db.pool, &sm.db.engine)
        .await
        .expect("foreman defaults");
    (dir, app, state, token)
}

fn sm(state: &AppState) -> &Spoolman {
    state.spoolman.get().expect("spoolman")
}

/// Spoolman extra value → decoded JSON.
fn extra(v: &Value, key: &str) -> Value {
    v["extra"][key]
        .as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null)
}

fn form(name: &str, material: &str, mods: &[&str]) -> Value {
    json!({
        "vendor_name": "Polymaker",
        "name": name,
        "material": material,
        "modifiers": mods,
        "colour_kind": "single",
        "colours": ["#1d1e24"],
    })
}

#[tokio::test]
async fn modifier_ticks_abrasive_and_the_user_can_override() {
    let (_d, app, state, t) = ready().await;
    // (modifiers, explicit flag, expected stored flag)
    let cases: &[(&[&str], Option<bool>, bool)] = &[
        (&["CF"], None, true),
        (&["GF"], None, true),
        (&["AF"], None, true),
        (&["Metal"], None, true),
        (&["Glow"], None, true),
        (&["HF", "CF"], None, true),
        (&["Silk", "Matte"], None, false),
        (&["95A"], None, false),
        (&[], None, false),
        (&["CF"], Some(false), false), // user unticked it
        (&["Silk"], Some(true), true), // user ticked it
    ];
    for (i, (mods, flag, want)) in cases.iter().enumerate() {
        let mut f = form(&format!("Case {i}"), "PETG", mods);
        if let Some(b) = flag {
            f["abrasive"] = json!(b);
        }
        let (s, b) = call(&app, "POST", "/api/filaments", &t, Some(f)).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        assert_eq!(b["abrasive"], json!(want), "case {i} {mods:?} {flag:?}");
        // Stored as Spoolman extra fields.
        let raw = sm_filament::find_by_id(&sm(&state).db.pool, b["id"].as_i64().expect("id"))
            .await
            .expect("filament");
        let raw = serde_json::to_value(&raw).expect("json");
        assert_eq!(extra(&raw, "abrasive"), json!(want), "case {i}");
        let stored_mods = extra(&raw, "material_modifier");
        if mods.is_empty() {
            assert_eq!(stored_mods, Value::Null);
        } else {
            assert_eq!(stored_mods, json!(mods.join(" ")));
        }
        assert_eq!(raw["material"], "PETG");
    }
}

#[tokio::test]
async fn guard_reads_the_stored_flag_not_the_name() {
    let (_d, app, state, t) = ready().await;
    // CF modifier but the user unticked abrasive: spool facts say not abrasive.
    let mut f = form("PETG CF (unticked)", "PETG", &["CF"]);
    f["abrasive"] = json!(false);
    let (_, made) = call(
        &app,
        "POST",
        "/api/spools",
        &t,
        Some(json!({ "new_filament": f, "initial_weight": 1000 })),
    )
    .await;
    let id = made["spools"][0]["id"].as_i64().expect("spool id");
    let facts = spools::spool(sm(&state), id).await.expect("facts");
    assert!(!facts.abrasive);
    assert_eq!(facts.modifier.as_deref(), Some("CF"));
}

#[tokio::test]
async fn multi_colour_round_trips_through_spoolman() {
    let (_d, app, state, t) = ready().await;
    let mut f = form("Rainbow", "PLA", &["Silk Rainbow"]);
    f["colour_kind"] = json!("longitudinal");
    f["colours"] = json!(["#ff0000", "00ff00", "#0000FF"]);
    f["hue_td"] = json!(3.2); // dropped: TD is single-colour only
    let (s, b) = call(&app, "POST", "/api/filaments", &t, Some(f.clone())).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id = b["id"].as_i64().expect("id");
    assert_eq!(b["colour_kind"], "longitudinal");
    assert_eq!(b["colours"], json!(["FF0000", "00FF00", "0000FF"]));
    assert_eq!(b["hue_td"], Value::Null);
    assert_eq!(b["modifiers"], json!(["Silk Rainbow"]));

    // Spoolman's own columns.
    let raw = sm_filament::find_by_id(&sm(&state).db.pool, id)
        .await
        .expect("filament");
    assert_eq!(raw.color_hex, None);
    assert_eq!(
        raw.multi_color_hexes.as_deref(),
        Some("FF0000,00FF00,0000FF")
    );
    assert_eq!(raw.multi_color_direction.as_deref(), Some("longitudinal"));

    // Edit → coextruded, 2 colours.
    f["colour_kind"] = json!("coaxial");
    f["colours"] = json!(["#d4af37", "#c0c0c0"]);
    let (s, b) = call(
        &app,
        "PATCH",
        &format!("/api/filaments/{id}"),
        &t,
        Some(f.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["colour_kind"], "coaxial");
    let raw = sm_filament::find_by_id(&sm(&state).db.pool, id)
        .await
        .expect("filament");
    assert_eq!(raw.multi_color_hexes.as_deref(), Some("D4AF37,C0C0C0"));
    assert_eq!(raw.multi_color_direction.as_deref(), Some("coaxial"));

    // Edit → single colour with TD: multi columns cleared, color_hex set.
    f["colour_kind"] = json!("single");
    f["colours"] = json!(["#1f8a8a"]);
    f["hue_td"] = json!(4.1);
    let (_, b) = call(
        &app,
        "PATCH",
        &format!("/api/filaments/{id}"),
        &t,
        Some(f.clone()),
    )
    .await;
    assert_eq!(b["colour_kind"], "single");
    assert_eq!(b["hue_td"], json!(4.1));
    let raw = sm_filament::find_by_id(&sm(&state).db.pool, id)
        .await
        .expect("filament");
    assert_eq!(raw.color_hex.as_deref(), Some("1F8A8A"));
    assert_eq!(raw.multi_color_hexes, None);
    assert_eq!(raw.multi_color_direction, None);

    // Spool facts see the colours too (swatch rendering).
    let (_, made) = call(
        &app,
        "POST",
        "/api/spools",
        &t,
        Some(json!({ "filament_id": id, "initial_weight": 1000 })),
    )
    .await;
    assert_eq!(made["spools"][0]["color_hex"], "1F8A8A");

    // Colour counts per kind are enforced.
    for (kind, n) in [
        ("single", 2),
        ("coaxial", 1),
        ("coaxial", 4),
        ("longitudinal", 7),
    ] {
        f["colour_kind"] = json!(kind);
        f["colours"] = json!(vec!["#ffffff"; n]);
        let (s, b) = call(&app, "POST", "/api/filaments", &t, Some(f.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{kind} × {n}: {b}");
    }
    f["colour_kind"] = json!("single");
    f["colours"] = json!(["#ggg000"]);
    let (s, _) = call(&app, "POST", "/api/filaments", &t, Some(f)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn vendor_crud_and_empty_weight_prefills_new_spools() {
    let (_d, app, state, t) = ready().await;
    let (s, v) = call(
        &app,
        "POST",
        "/api/vendors",
        &t,
        Some(json!({ "name": "Sunlu", "empty_spool_weight": 165, "website": "https://sunlu.com", "comment": "cardboard" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let vid = v["id"].as_i64().expect("vendor id");
    assert_eq!(v["website"], "https://sunlu.com");

    // Duplicate names are refused (case-insensitive).
    let (s, _) = call(
        &app,
        "POST",
        "/api/vendors",
        &t,
        Some(json!({ "name": "sunlu" })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Edit: new weight, clear comment.
    let (s, v) = call(
        &app,
        "PATCH",
        &format!("/api/vendors/{vid}"),
        &t,
        Some(json!({ "name": "Sunlu", "empty_spool_weight": 170, "website": "https://sunlu.com" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["empty_spool_weight"], json!(170.0));
    assert_eq!(v["comment"], Value::Null);

    // A filament of that vendor, then a spool without an empty weight.
    let mut f = form("PLA Grey", "PLA", &[]);
    f["vendor_id"] = json!(vid);
    f["vendor_name"] = Value::Null;
    let (_, fil) = call(&app, "POST", "/api/filaments", &t, Some(f)).await;
    assert_eq!(fil["spool_weight"], json!(170.0));
    let (s, made) = call(
        &app,
        "POST",
        "/api/spools",
        &t,
        Some(json!({ "filament_id": fil["id"], "initial_weight": 1000 })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    assert_eq!(made["spools"][0]["spool_weight"], json!(170.0));
    // An explicit empty weight wins.
    let (_, made) = call(
        &app,
        "POST",
        "/api/spools",
        &t,
        Some(json!({ "filament_id": fil["id"], "initial_weight": 1000, "spool_weight": 120 })),
    )
    .await;
    assert_eq!(made["spools"][0]["spool_weight"], json!(120.0));

    // Stored in Spoolman: vendor url extra field.
    let raw = spoolman_rs::db::vendor::find_by_id(&sm(&state).db.pool, vid)
        .await
        .expect("vendor");
    let raw = serde_json::to_value(&raw).expect("json");
    assert_eq!(extra(&raw, "url"), "https://sunlu.com");
    let (_, list) = call(&app, "GET", "/api/vendors", &t, None).await;
    assert_eq!(list[0]["filament_count"], 1);
}

#[tokio::test]
async fn create_filament_then_n_spools_in_one_request() {
    let (_d, app, state, t) = ready().await;
    let mut f = form("PETG Gradient", "PETG", &[]);
    f["vendor_name"] = json!("Elegoo"); // new vendor, created inline
    f["colour_kind"] = json!("longitudinal");
    f["colours"] = json!(["#e2621f", "#c9c2b0", "#1f8a8a"]);
    let (s, b) = call(
        &app,
        "POST",
        "/api/spools",
        &t,
        Some(json!({
            "new_filament": f,
            "initial_weight": 1000,
            "location": "Shelf A1",
            "lot_nr": "L42",
            "price": 19.9,
            "quantity": 2,
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["filament_created"], true);
    assert_eq!(b["filament"]["vendor"], "Elegoo");
    assert_eq!(b["filament"]["spool_count"], 2);
    assert_eq!(b["filament"]["grams_left"], json!(2000.0));
    let spools = b["spools"].as_array().expect("spools");
    assert_eq!(spools.len(), 2);
    for sp in spools {
        assert_eq!(sp["remaining_weight"], json!(1000.0));
        assert_eq!(sp["location"], "Shelf A1");
        assert_eq!(sp["lot_nr"], "L42");
        assert_eq!(sp["multi_color_direction"], "longitudinal");
        assert_eq!(sp["multi_color_hexes"].as_array().map(Vec::len), Some(3));
    }
    // Defaults from the material (PETG: 1.27 g/cm³).
    assert_eq!(b["filament"]["density"], json!(1.27));

    // Spoolman sees both spools of that filament.
    let filter = sm_spool::SpoolFilter {
        filament_id: Some(b["filament"]["id"].to_string()),
        ..Default::default()
    };
    let (rows, n) = sm_spool::find(&sm(&state).db.pool, &sm(&state).db.engine, &filter)
        .await
        .expect("find");
    assert_eq!(n, 2);
    assert!(rows
        .iter()
        .all(|r| r.filament.vendor.as_ref().map(|v| v.name.as_str()) == Some("Elegoo")));

    // The inline vendor is reused, not duplicated, next time.
    let mut f2 = form("PETG Plain", "PETG", &[]);
    f2["vendor_name"] = json!("elegoo");
    let (_, b2) = call(&app, "POST", "/api/filaments", &t, Some(f2)).await;
    assert_eq!(b2["vendor_id"], b["filament"]["vendor_id"]);
    let (_, vendors) = call(&app, "GET", "/api/vendors", &t, None).await;
    let names: Vec<&str> = vendors
        .as_array()
        .expect("vendors")
        .iter()
        .filter_map(|v| v["name"].as_str())
        .collect();
    assert_eq!(names, ["Elegoo"]);
}

#[tokio::test]
async fn clone_and_quantity_limits() {
    let (_d, app, _state, t) = ready().await;
    let (_, made) = call(
        &app,
        "POST",
        "/api/spools",
        &t,
        Some(json!({ "new_filament": form("PLA Teal", "PLA", &[]), "initial_weight": 750, "location": "Dry box 1", "spool_weight": 200 })),
    )
    .await;
    let id = made["spools"][0]["id"].as_i64().expect("id");
    let (s, copies) = call(
        &app,
        "POST",
        &format!("/api/spools/{id}/clone"),
        &t,
        Some(json!({ "quantity": 3 })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{copies}");
    let copies = copies.as_array().expect("copies");
    assert_eq!(copies.len(), 3);
    for c in copies {
        assert_ne!(c["id"], json!(id));
        assert_eq!(c["filament_id"], made["spools"][0]["filament_id"]);
        assert_eq!(c["initial_weight"], json!(750.0));
        assert_eq!(c["remaining_weight"], json!(750.0));
        assert_eq!(c["spool_weight"], json!(200.0));
        assert_eq!(c["location"], "Dry box 1");
    }
    for q in [0, 51] {
        let (s, _) = call(
            &app,
            "POST",
            &format!("/api/spools/{id}/clone"),
            &t,
            Some(json!({ "quantity": q })),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "quantity {q}");
    }
    let (s, _) = call(
        &app,
        "POST",
        "/api/spools",
        &t,
        Some(json!({ "initial_weight": 1000 })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, locs) = call(&app, "GET", "/api/spools/locations", &t, None).await;
    assert_eq!(locs, json!(["Dry box 1"]));
}

#[tokio::test]
async fn catalogue_lists_materials_and_abrasive_modifiers() {
    let (_d, app, _state, t) = ready().await;
    let (s, c) = call(&app, "GET", "/api/filament/catalogue", &t, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(c["materials"].as_array().map(Vec::len), Some(10));
    let filled = &c["modifier_groups"][1];
    assert_eq!(filled["label"], "Filled");
    assert_eq!(
        filled["items"][0],
        json!({ "name": "CF", "abrasive": true, "why": "carbon fibre" })
    );
}
