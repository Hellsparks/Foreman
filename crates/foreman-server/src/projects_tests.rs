//! Templates and projects from a template (task 5.1) through the REST API.

use axum::{http::StatusCode, Router};
use foreman_core::{
    domain::{JobEnd, QueueStatus},
    repo,
};
use serde_json::{json, Value};

use crate::{
    projects,
    spools_tests::{add_spool, call, setup},
    state::AppState,
};

/// A two-tool plate (T0 30 g PLA teal, T1 10 g PLA white) and a one-tool plate
/// (T0 20 g PLA white), added to the Library root.
async fn files(state: &AppState) -> (i64, i64) {
    let mut ids = vec![];
    for (name, body) in [
        (
            "pawns.gcode",
            "; filament used [g] = 30.00, 10.00\n; filament_type = PLA;PLA\n\
             ; filament_colour = #1F8A8A;#F4F4EF\n\
             ; estimated printing time (normal mode) = 1h 0m 0s\nG1 X1\n",
        ),
        (
            "frame.gcode",
            "; filament used [g] = 20.00\n; filament_type = PLA\n; filament_colour = #F4F4EF\n\
             ; estimated printing time (normal mode) = 30m 0s\nG1 X1\n",
        ),
    ] {
        let tmp = state.library.tmp_dir().join(name);
        std::fs::write(&tmp, body).expect("seed");
        ids.push(
            state
                .library
                .add_file("", name, &tmp)
                .await
                .expect("add")
                .id,
        );
    }
    (ids[0], ids[1])
}

fn template_body(pawns: i64, frame: i64) -> Value {
    json!({
        "name": "Chess set",
        "category": "Games",
        "slots": [
            { "key": "primary", "hex": "#1f8a8a", "label": "PLA Teal" },
            { "key": "ACCENT", "hex": "f4f4ef", "label": "PLA White", "allow_swap": false }
        ],
        "plates": [
            { "name": "Pawns", "file_id": pawns, "copies": 1,
              "tools": [{ "tool": 0, "slot": "PRIMARY" }, { "tool": 1, "slot": "accent" }] },
            { "name": "Frame", "file_id": frame, "copies": 1,
              "tools": [{ "tool": 0, "slot": "ACCENT" }] }
        ]
    })
}

struct World {
    _d: tempfile::TempDir,
    app: Router,
    state: AppState,
    auth: String,
    tpl: i64,
    teal: i64,
    white: i64,
}

async fn world() -> World {
    let (d, app, state, auth) = setup().await;
    let (pawns, frame) = files(&state).await;
    let sm = state.spoolman.get().expect("sm");
    let teal = add_spool(sm, "PLA", "1F8A8A", 1000.0).await;
    let white = add_spool(sm, "PLA", "F4F4EF", 500.0).await;
    let (s, v) = call(
        &app,
        "POST",
        "/api/templates",
        &auth,
        Some(template_body(pawns, frame)),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let tpl = v["id"].as_i64().expect("id");
    World {
        _d: d,
        app,
        state,
        auth,
        tpl,
        teal,
        white,
    }
}

impl World {
    async fn create(&self, sets: u32) -> Value {
        let (s, v) = call(
            &self.app,
            "POST",
            "/api/projects",
            &self.auth,
            Some(json!({
                "template_id": self.tpl, "sets": sets, "due_date": "2026-11-20",
                "spools": [{ "key": "PRIMARY", "spool_id": self.teal }, { "key": "ACCENT", "spool_id": self.white }]
            })),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        v
    }
    async fn get(&self, id: i64) -> Value {
        let (s, v) = call(
            &self.app,
            "GET",
            &format!("/api/projects/{id}"),
            &self.auth,
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }
    async fn inventory(&self) -> Value {
        call(&self.app, "GET", "/api/inventory", &self.auth, None)
            .await
            .1
    }
    async fn fleet_items(&self) -> Vec<foreman_core::domain::QueueItem> {
        repo::queue::list_waiting(&self.state.foreman_db.pool, None)
            .await
            .expect("queue")
    }
}

fn reserved_on(inv: &Value, spool: i64) -> f64 {
    inv["reservations"]
        .as_array()
        .expect("res")
        .iter()
        .filter(|r| r["spool_id"] == spool)
        .map(|r| r["grams"].as_f64().unwrap_or(0.0))
        .sum()
}

#[tokio::test]
async fn template_copies_plates_into_the_library_and_reports_problems() {
    let w = world().await;
    let (_, v) = call(
        &w.app,
        "GET",
        &format!("/api/templates/{}", w.tpl),
        &w.auth,
        None,
    )
    .await;
    assert_eq!(v["problems"], json!([]), "{v}");
    assert_eq!(v["prints_per_set"], 2);
    assert_eq!(v["template"]["slots"][0]["key"], "PRIMARY");
    assert_eq!(v["template"]["slots"][1]["hex"], "F4F4EF");
    assert_eq!(v["template"]["plates"][0]["tools"][1]["slot"], "ACCENT");
    // Plates are copies under Templates/Chess set/.
    for i in 0..2 {
        let id = v["template"]["plates"][i]["file_id"]
            .as_i64()
            .expect("file");
        let e = w.state.library.get(id).await.expect("entry");
        assert!(e.path.starts_with("Templates/Chess set/"), "{}", e.path);
    }
    assert_eq!(v["files"][0]["tools"][0]["grams"], 30.0);

    // A slot no plate uses is reported, but the draft still saves.
    let mut body = v["template"].clone();
    body["slots"]
        .as_array_mut()
        .expect("slots")
        .push(json!({ "key": "TRIM" }));
    let (s, v) = call(
        &w.app,
        "PUT",
        &format!("/api/templates/{}", w.tpl),
        &w.auth,
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        v["problems"],
        json!(["Slot TRIM is not used by any plate."])
    );
    // Saving again does not copy the (already owned) files a second time.
    assert!(v["files"][0]["file_name"]
        .as_str()
        .is_some_and(|n| n == "pawns.gcode"));
    // An unready template can't start a project.
    let (s, _) = call(
        &w.app,
        "POST",
        "/api/projects",
        &w.auth,
        Some(json!({ "template_id": w.tpl, "sets": 1, "spools": [] })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn three_sets_are_six_prints_reserved_per_slot() {
    let w = world().await;
    // Plan: 3 sets → PRIMARY 30×3, ACCENT (10 + 20)×3; teal spool fits PRIMARY first.
    let (_, plan) = call(
        &w.app,
        "GET",
        &format!("/api/templates/{}/plan?sets=3", w.tpl),
        &w.auth,
        None,
    )
    .await;
    assert_eq!(plan["prints"], 6);
    assert_eq!(plan["slots"][0]["need"], 90.0);
    assert_eq!(plan["slots"][1]["need"], 90.0);
    assert_eq!(plan["slots"][0]["materials"], json!(["PLA"]));
    assert_eq!(plan["slots"][0]["options"][0]["spool"]["id"], w.teal);
    assert_eq!(plan["slots"][0]["options"][0]["colour_distance"], 0.0);
    assert_eq!(plan["slots"][1]["options"][0]["spool"]["id"], w.white);

    let v = w.create(3).await;
    assert_eq!(v["name"], "Chess set ×3");
    assert_eq!(v["totals"]["prints"], 6);
    assert_eq!(v["totals"]["remaining"], 6);
    assert_eq!(v["plates"][0]["counts"]["copies"], 3);
    assert_eq!(v["slots"][0]["need"], 90.0);
    assert_eq!(v["slots"][0]["reserved"], 90.0);
    assert_eq!(v["slots"][1]["reserved"], 90.0);

    let inv = w.inventory().await;
    assert_eq!(reserved_on(&inv, w.teal), 90.0);
    assert_eq!(reserved_on(&inv, w.white), 90.0);
    assert_eq!(inv["grams_reserved"], 180.0);

    // A second project sees only what is left free.
    let (_, plan) = call(
        &w.app,
        "GET",
        &format!("/api/templates/{}/plan?sets=1", w.tpl),
        &w.auth,
        None,
    )
    .await;
    let white = plan["slots"][1]["options"]
        .as_array()
        .expect("opts")
        .iter()
        .find(|o| o["spool"]["id"] == w.white)
        .cloned()
        .expect("white");
    assert_eq!(white["reserved"], 90.0);
    assert_eq!(white["free"], 410.0);
}

#[tokio::test]
async fn print_all_remaining_queues_one_item_per_print() {
    let w = world().await;
    let id = w.create(3).await["id"].as_i64().expect("id");
    let (s, v) = call(
        &w.app,
        "POST",
        &format!("/api/projects/{id}/print-remaining"),
        &w.auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["added"], 6);
    let items = w.fleet_items().await;
    assert_eq!(items.len(), 6);
    assert!(items
        .iter()
        .all(|i| i.printer_id.is_none() && i.project_plate_id.is_some()));
    let p = w.get(id).await;
    let pawns = p["plates"][0]["id"].as_i64();
    assert_eq!(
        items.iter().filter(|i| i.project_plate_id == pawns).count(),
        3
    );
    assert_eq!(p["totals"]["queued"], 6);
    assert_eq!(p["totals"]["remaining"], 0);
    assert_eq!(p["plates"][0]["live"][0]["where_"], "fleet");
    // Reservations follow the queued prints to the slot spools, not double counted.
    let inv = w.inventory().await;
    assert_eq!(reserved_on(&inv, w.teal), 90.0);
    assert_eq!(reserved_on(&inv, w.white), 90.0);
    // Marking a copy done by hand takes one queued copy off the fleet queue.
    let (s, v) = call(
        &w.app,
        "POST",
        &format!("/api/project-plates/{}/done", pawns.expect("p")),
        &w.auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        v["plates"][0]["counts"],
        json!({"copies":3,"printed":1,"printing":0,"queued":2,"remaining":0})
    );
    assert_eq!(w.fleet_items().await.len(), 5);
    // Nothing left: a second press adds nothing.
    let (_, v) = call(
        &w.app,
        "POST",
        &format!("/api/projects/{id}/print-remaining"),
        &w.auth,
        None,
    )
    .await;
    assert_eq!(v["added"], 0);
    assert_eq!(w.fleet_items().await.len(), 5);
}

#[tokio::test]
async fn finished_jobs_mark_the_right_plate_and_close_the_project() {
    let w = world().await;
    let id = w.create(1).await["id"].as_i64().expect("id");
    call(
        &w.app,
        "POST",
        &format!("/api/projects/{id}/print-remaining"),
        &w.auth,
        None,
    )
    .await;
    let pool = &w.state.foreman_db.pool;
    let items = w.fleet_items().await;
    let p = w.get(id).await;
    let (pawns, frame) = (p["plates"][0]["id"].as_i64(), p["plates"][1]["id"].as_i64());
    let frame_item = items
        .iter()
        .find(|i| i.project_plate_id == frame)
        .expect("frame item");
    let pawn_item = items
        .iter()
        .find(|i| i.project_plate_id == pawns)
        .expect("pawn item");

    // Frame print is cancelled: not counted, copy outstanding again.
    repo::queue::transition(
        pool,
        frame_item.id,
        QueueStatus::Waiting,
        QueueStatus::Cancelled,
        None,
    )
    .await
    .expect("t");
    projects::print_ended(pool, frame_item, JobEnd::Cancelled, None, "HellV0")
        .await
        .expect("hook");
    let p = w.get(id).await;
    assert_eq!(p["plates"][1]["counts"]["printed"], 0);
    assert_eq!(p["plates"][1]["counts"]["remaining"], 1);

    // Pawns completes: pawns plate (not frame) counts it.
    repo::queue::transition(
        pool,
        pawn_item.id,
        QueueStatus::Waiting,
        QueueStatus::Done,
        None,
    )
    .await
    .expect("t");
    projects::print_ended(pool, pawn_item, JobEnd::Complete, None, "HellV0")
        .await
        .expect("hook");
    let p = w.get(id).await;
    assert_eq!(p["plates"][0]["counts"]["printed"], 1);
    assert_eq!(p["plates"][1]["counts"]["printed"], 0);
    assert_eq!(p["totals"]["printed"], 1);
    assert_eq!(p["status"], "active");
    // Its reservation is gone; frame's 20 g on white stays.
    let inv = w.inventory().await;
    assert_eq!(reserved_on(&inv, w.teal), 0.0);
    assert_eq!(reserved_on(&inv, w.white), 20.0);

    // Frame printed by hand → every print done → project closes itself.
    let (s, p) = call(
        &w.app,
        "POST",
        &format!("/api/project-plates/{}/done", frame.expect("f")),
        &w.auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["status"], "done");
    assert!(p["closed_at"].is_string());
    assert!(p["events"][0]["text"]
        .as_str()
        .is_some_and(|t| t.contains("closed")));
    assert!(p["events"].as_array().expect("ev").iter().any(|e| e["text"]
        .as_str()
        .is_some_and(|t| t.contains("Pawns finished on HellV0"))));

    // One more copy reopens it.
    let (s, p) = call(
        &w.app,
        "PATCH",
        &format!("/api/project-plates/{}", pawns.expect("p")),
        &w.auth,
        Some(json!({ "copies": 2 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["status"], "active");
    assert_eq!(p["totals"]["remaining"], 1);
}

#[tokio::test]
async fn copies_never_drop_below_printed_or_printing() {
    let w = world().await;
    let id = w.create(3).await["id"].as_i64().expect("id");
    call(
        &w.app,
        "POST",
        &format!("/api/projects/{id}/print-remaining"),
        &w.auth,
        None,
    )
    .await;
    let pool = &w.state.foreman_db.pool;
    let p = w.get(id).await;
    let pawns = p["plates"][0]["id"].as_i64().expect("p");
    let mut mine: Vec<_> = w
        .fleet_items()
        .await
        .into_iter()
        .filter(|i| i.project_plate_id == Some(pawns))
        .collect();
    mine.sort_by_key(|i| i.id);
    // One printed, one printing (claimed by printer… use a DB status), one waiting.
    repo::queue::transition(
        pool,
        mine[0].id,
        QueueStatus::Waiting,
        QueueStatus::Done,
        None,
    )
    .await
    .expect("t");
    projects::print_ended(pool, &mine[0], JobEnd::Complete, None, "HellV0")
        .await
        .expect("hook");
    repo::queue::transition(
        pool,
        mine[1].id,
        QueueStatus::Waiting,
        QueueStatus::Printing,
        None,
    )
    .await
    .expect("t");
    let p = w.get(id).await;
    assert_eq!(
        p["plates"][0]["counts"],
        json!({"copies":3,"printed":1,"printing":1,"queued":1,"remaining":0})
    );
    assert_eq!(p["plates"][0]["min_copies"], 2);

    let set = |n: u32| {
        let w = &w;
        async move {
            call(
                &w.app,
                "PATCH",
                &format!("/api/project-plates/{pawns}"),
                &w.auth,
                Some(json!({ "copies": n })),
            )
            .await
        }
    };
    let (s, v) = set(1).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        v["error"]
            .as_str()
            .is_some_and(|e| e.contains("at least 2")),
        "{v}"
    );

    // Down to 2: the waiting copy comes off the fleet queue.
    let (s, v) = set(2).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        v["plates"][0]["counts"],
        json!({"copies":2,"printed":1,"printing":1,"queued":0,"remaining":0})
    );
    assert!(
        repo::queue::get(pool, mine[2].id).await.is_err(),
        "waiting copy removed"
    );
    assert_eq!(
        repo::queue::get(pool, mine[1].id)
            .await
            .expect("printing")
            .status,
        QueueStatus::Printing
    );

    // Up to 4: two not started.
    let (_, v) = set(4).await;
    assert_eq!(v["plates"][0]["counts"]["remaining"], 2);
    assert_eq!(v["totals"]["prints"], 4 + 3);
}
