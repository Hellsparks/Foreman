//! Inventory, shopping list, reserved weight, locations (task 4.3).

use axum::http::StatusCode;
use foreman_core::{
    domain::{FileMeta, FirmwareKind, QueueSource, ToolUsage},
    driver::PrinterCapabilities,
    repo,
};
use serde_json::json;
use spoolman_rs::db::{setting, spool as sm_spool};

use crate::{
    inventory,
    spools::Spoolman,
    spools_tests::{add_printer, add_spool, call, idle, setup, MockKlipper, NoMeta},
    state::AppState,
};

fn sm(state: &AppState) -> &Spoolman {
    state.spoolman.get().expect("spoolman")
}

async fn filament_of(sm: &Spoolman, spool: i64) -> i64 {
    sm_spool::find_by_id(&sm.db.pool, spool)
        .await
        .expect("spool")
        .filament
        .id
}

async fn set_price(sm: &Spoolman, filament: i64, price: f64) {
    sqlx::query("UPDATE filament SET price = ? WHERE id = ?")
        .bind(price)
        .bind(filament)
        .execute(&sm.db.pool)
        .await
        .expect("price");
}

async fn set_location(sm: &Spoolman, spool: i64, loc: &str) {
    sqlx::query("UPDATE spool SET location = ? WHERE id = ?")
        .bind(loc)
        .bind(spool)
        .execute(&sm.db.pool)
        .await
        .expect("location");
}

#[tokio::test]
async fn tracked_filament_goes_on_the_shopping_list_in_spoolmans_currency() {
    let (_d, app, state, auth) = setup().await;
    let sm = sm(&state);
    setting::set(&sm.db.pool, "currency", "\"NOK\"")
        .await
        .expect("currency");
    let a = add_spool(sm, "PETG", "1d1d22", 800.0).await;
    let fid = filament_of(sm, a).await;
    set_price(sm, fid, 299.0).await;

    let (s, v) = call(&app, "GET", "/api/inventory", &auth, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["currency"], "NOK");
    assert_eq!(v["tracked"].as_array().map(Vec::len), Some(0));
    assert_eq!(v["grams_on_hand"], 800.0);

    // target 3, buy below 2, have 1 → BUY 2 at 299 each
    let (s, v) = call(
        &app,
        "PUT",
        &format!("/api/inventory/{fid}"),
        &auth,
        Some(json!({ "target": 3, "buy_below": 2 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = &v["tracked"][0];
    assert_eq!(r["filament"]["id"], fid);
    assert_eq!(r["stock"], 1);
    assert_eq!(r["status"], "buy");
    assert_eq!(r["buy"], 2);
    assert_eq!(r["line_cost"], 598.0);
    assert_eq!(v["need_count"], 1);
    assert_eq!(v["est_cost"], 598.0);
    assert_eq!(v["unpriced"], 0);

    // buy below 1 → OK, off the list
    let (_, v) = call(
        &app,
        "PUT",
        &format!("/api/inventory/{fid}"),
        &auth,
        Some(json!({ "target": 3, "buy_below": 1 })),
    )
    .await;
    assert_eq!(v["tracked"][0]["status"], "ok");
    assert_eq!(v["tracked"][0]["buy"], 0);
    assert_eq!(v["need_count"], 0);

    // a nearly empty spool does not count as stock
    sm_spool::measure(&sm.db.pool, a, 0.0).await.ok();
    sqlx::query("UPDATE spool SET used_weight = initial_weight - 40 WHERE id = ?")
        .bind(a)
        .execute(&sm.db.pool)
        .await
        .expect("use");
    let (_, v) = call(&app, "GET", "/api/inventory", &auth, None).await;
    assert_eq!(v["tracked"][0]["stock"], 0);
    assert_eq!(v["tracked"][0]["status"], "out");
    assert_eq!(v["tracked"][0]["buy"], 3);

    // bad numbers
    let (s, _) = call(
        &app,
        "PUT",
        &format!("/api/inventory/{fid}"),
        &auth,
        Some(json!({ "target": 1, "buy_below": 2 })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(
        &app,
        "PUT",
        "/api/inventory/9999",
        &auth,
        Some(json!({ "target": 1, "buy_below": 1 })),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // untrack
    let (s, v) = call(
        &app,
        "DELETE",
        &format!("/api/inventory/{fid}"),
        &auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["tracked"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn queued_jobs_reserve_weight_on_the_loaded_spool() {
    let (_d, _app, state, _auth) = setup().await;
    let sm = sm(&state);
    let loaded = add_spool(sm, "PLA", "ff0000", 300.0).await;
    let spare = add_spool(sm, "PLA", "00ff00", 900.0).await;
    let fid = filament_of(sm, loaded).await;
    let pid = add_printer(&state, "HellV0", FirmwareKind::Moonraker).await;
    let mock = MockKlipper::new(Some(true));
    *mock.active.lock().unwrap_or_else(|p| p.into_inner()) = Some(loaded);
    state
        .driver_manager
        .insert_test_driver(
            pid,
            mock,
            PrinterCapabilities {
                spoolman_connected: Some(true),
                spoolman_spool_id: Some(loaded),
                ..Default::default()
            },
            Some(idle()),
        )
        .await;
    let pool = &state.foreman_db.pool;
    repo::tracked_filament::upsert(
        pool,
        repo::tracked_filament::Tracked {
            filament_id: fid,
            target: 2,
            buy_below: 1,
        },
    )
    .await
    .expect("track");
    let meta = |g: f64| FileMeta {
        tools: vec![ToolUsage {
            index: 0,
            material: Some("PLA".into()),
            grams: Some(g),
            ..Default::default()
        }],
        ..Default::default()
    };

    let inv = inventory::inventory(pool, &state.driver_manager, sm, &NoMeta(Some(meta(280.0))))
        .await
        .expect("inventory");
    assert_eq!(inv.grams_reserved, 0.0, "nothing queued");
    assert_eq!(inv.tracked[0].stock, 1);

    // Two waiting jobs on the printer, 280 g each: the first takes 280 g, the
    // second only the 20 g that is left. The loaded spool drops out of stock.
    for name in ["a.gcode", "b.gcode"] {
        repo::queue::add(pool, 7, name, Some(pid), QueueSource::Manual, "admin")
            .await
            .expect("queue");
    }
    // A fleet job (no printer yet) holds nothing.
    repo::queue::add(pool, 8, "fleet.gcode", None, QueueSource::Manual, "admin")
        .await
        .expect("queue");
    let inv = inventory::inventory(pool, &state.driver_manager, sm, &NoMeta(Some(meta(280.0))))
        .await
        .expect("inventory");
    assert_eq!(inv.grams_on_hand, 1200.0);
    assert_eq!(inv.grams_reserved, 300.0);
    assert_eq!(inv.reservations.len(), 2);
    assert!(inv.reservations.iter().all(|r| r.spool_id == loaded));
    assert_eq!(inv.reservations[0].grams, 280.0);
    assert_eq!(inv.reservations[0].file_name, "a.gcode");
    assert_eq!(inv.reservations[1].grams, 20.0);
    let row = &inv.tracked[0];
    assert_eq!(row.grams_reserved, 300.0);
    assert_eq!(row.stock, 0, "the loaded spool is fully reserved");
    // `spare` is a different filament (add_spool makes one per spool).
    assert_ne!(filament_of(sm, spare).await, fid);

    // A file sliced for a named spool reserves that spool, even from the fleet queue.
    let named = FileMeta {
        tools: vec![ToolUsage {
            index: 0,
            grams: Some(100.0),
            spool_id: Some(spare),
            ..Default::default()
        }],
        ..Default::default()
    };
    let inv = inventory::inventory(pool, &state.driver_manager, sm, &NoMeta(Some(named)))
        .await
        .expect("inventory");
    assert_eq!(inv.reservations.len(), 3);
    assert!(inv.reservations.iter().all(|r| r.spool_id == spare));
    assert_eq!(inv.grams_reserved, 300.0);
}

#[tokio::test]
async fn renaming_a_location_moves_every_spool_on_it() {
    let (_d, app, state, auth) = setup().await;
    let sm = sm(&state);
    let a = add_spool(sm, "PLA", "ff0000", 500.0).await;
    let b = add_spool(sm, "PLA", "00ff00", 400.0).await;
    let c = add_spool(sm, "PETG", "0000ff", 300.0).await;
    let _loose = add_spool(sm, "PETG", "000000", 200.0).await;
    set_location(sm, a, "Shelf A").await;
    set_location(sm, b, "Shelf A").await;
    set_location(sm, c, "Dry box").await;
    setting::set(
        &sm.db.pool,
        "locations",
        r#"["Shelf A","Empty bin","Dry box"]"#,
    )
    .await
    .expect("setting");
    setting::set(
        &sm.db.pool,
        "locations_spoolorders",
        &format!(r#"{{"Shelf A":[{b},{a}]}}"#),
    )
    .await
    .expect("setting");

    let (s, v) = call(&app, "GET", "/api/spool-locations", &auth, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let names: Vec<&str> = v["locations"]
        .as_array()
        .expect("list")
        .iter()
        .filter_map(|l| l["name"].as_str())
        .collect();
    assert_eq!(names, ["Dry box", "Empty bin", "Shelf A"]);
    assert_eq!(v["locations"][2]["spool_count"], 2);
    assert_eq!(v["locations"][2]["grams"], 900.0);
    assert_eq!(v["locations"][1]["spool_count"], 0);
    assert_eq!(v["unplaced"], 1);

    let (s, v) = call(
        &app,
        "PATCH",
        "/api/spool-locations/Shelf%20A",
        &auth,
        Some(json!({ "name": " Rack 1 " })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["spools_moved"], 2);
    assert_eq!(v["name"], "Rack 1");
    for id in [a, b] {
        let sp = sm_spool::find_by_id(&sm.db.pool, id).await.expect("spool");
        assert_eq!(sp.location.as_deref(), Some("Rack 1"));
    }
    let sp = sm_spool::find_by_id(&sm.db.pool, c).await.expect("spool");
    assert_eq!(sp.location.as_deref(), Some("Dry box"));
    let list = setting::get(&sm.db.pool, "locations").await.expect("get");
    assert_eq!(list.as_deref(), Some(r#"["Rack 1","Empty bin","Dry box"]"#));
    let orders = setting::get(&sm.db.pool, "locations_spoolorders")
        .await
        .expect("get")
        .unwrap_or_default();
    assert_eq!(orders, format!(r#"{{"Rack 1":[{b},{a}]}}"#));

    // Onto an existing location: merges, no duplicate in the list.
    let (s, v) = call(
        &app,
        "PATCH",
        "/api/spool-locations/Rack%201",
        &auth,
        Some(json!({ "name": "Dry box" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, v) = call(&app, "GET", "/api/spool-locations", &auth, None).await;
    assert_eq!(v["locations"][0]["name"], "Dry box");
    assert_eq!(v["locations"][0]["spool_count"], 3);
    let list = setting::get(&sm.db.pool, "locations").await.expect("get");
    assert_eq!(list.as_deref(), Some(r#"["Empty bin","Dry box"]"#));

    // Errors
    let (s, _) = call(
        &app,
        "PATCH",
        "/api/spool-locations/Nowhere",
        &auth,
        Some(json!({ "name": "X" })),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(
        &app,
        "PATCH",
        "/api/spool-locations/Dry%20box",
        &auth,
        Some(json!({ "name": "  " })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
