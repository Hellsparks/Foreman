//! Maintenance (task 5.3) through the REST API: accrual from job records,
//! thresholds, Attention raise/clear, mark done, history.

use axum::{http::StatusCode, Router};
use chrono::{Duration, Utc};
use foreman_core::domain::FirmwareKind;
use serde_json::{json, Value};

use crate::{
    maintenance,
    spools_tests::{add_printer, call, setup},
    state::AppState,
};

/// Insert a finished job directly (what the job engine writes when a print ends).
async fn job(
    state: &AppState,
    printer_id: i64,
    end_state: &str,
    outcome: Option<&str>,
    hours: f64,
    grams: f64,
) {
    let ended = Utc::now();
    let started = ended - Duration::seconds((hours * 3600.0) as i64);
    sqlx::query(
        "INSERT INTO job (printer_id, printer_name, file_name, started_at, ended_at, \
         duration_secs, end_state, outcome, grams_used) VALUES (?, 'P', 'f.gcode', ?, ?, ?, ?, ?, ?)",
    )
    .bind(printer_id)
    .bind(started.to_rfc3339())
    .bind(if end_state == "running" {
        None
    } else {
        Some(ended.to_rfc3339())
    })
    .bind((hours * 3600.0) as i64)
    .bind(end_state)
    .bind(outcome)
    .bind(grams)
    .execute(&state.foreman_db.pool)
    .await
    .expect("job");
}

/// The mixed history every test starts from: 3.5 h / 90 g / 2 completed jobs
/// count; a cancelled print, a print rated cancelled and a running one don't.
async fn history(state: &AppState, p: i64) {
    job(state, p, "complete", Some("good"), 2.0, 50.0).await;
    job(state, p, "complete", None, 1.0, 30.0).await;
    job(state, p, "error", Some("failed"), 0.5, 10.0).await;
    job(state, p, "cancelled", Some("cancelled"), 3.0, 100.0).await;
    job(state, p, "complete", Some("cancelled"), 1.0, 20.0).await;
    job(state, p, "running", None, 4.0, 0.0).await;
}

async fn task(app: &Router, auth: &str, p: i64, name: &str, basis: &str, interval: f64) -> Value {
    let (s, v) = call(
        app,
        "POST",
        "/api/maintenance/tasks",
        auth,
        Some(json!({ "printer_id": p, "name": name, "basis": basis, "interval": interval })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

async fn attention(app: &Router, auth: &str) -> Vec<Value> {
    let (s, v) = call(app, "GET", "/api/attention", auth, None).await;
    assert_eq!(s, StatusCode::OK);
    v.as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| {
            i["kind"]
                .as_str()
                .is_some_and(|k| k.starts_with("maintenance:"))
        })
        .collect()
}

#[tokio::test]
async fn accrues_hours_grams_and_jobs_from_job_records() {
    let (_d, app, state, auth) = setup().await;
    let p = add_printer(&state, "HellV0", FirmwareKind::Moonraker).await;
    let other = add_printer(&state, "Other", FirmwareKind::Moonraker).await;
    history(&state, p).await;
    job(&state, other, "complete", None, 9.0, 900.0).await;

    let h = task(&app, &auth, p, "Belt tension", "hours", 200.0).await;
    let g = task(&app, &auth, p, "Nozzle change", "grams", 5000.0).await;
    let j = task(&app, &auth, p, "Clean bed", "jobs", 20.0).await;
    for v in [&h, &g, &j] {
        assert_eq!(v["counters"]["hours"], json!(3.5), "{v}");
        assert_eq!(v["counters"]["grams"], json!(90.0), "{v}");
        assert_eq!(v["counters"]["jobs"], json!(2), "{v}");
    }
    assert_eq!(h["progress"]["value"], json!(3.5));
    assert_eq!(g["progress"]["value"], json!(90.0));
    assert_eq!(j["progress"]["value"], json!(2.0));

    // start_now: a counter that starts at zero ignores the history.
    let (_, fresh) = call(
        &app,
        "POST",
        "/api/maintenance/tasks",
        &auth,
        Some(json!({ "printer_id": p, "name": "Lube", "basis": "hours", "interval": 10, "start_now": true })),
    )
    .await;
    assert_eq!(fresh["progress"]["value"], json!(0.0));
}

#[tokio::test]
async fn cancelled_jobs_never_accrue() {
    let (_d, app, state, auth) = setup().await;
    let p = add_printer(&state, "HellV0", FirmwareKind::Moonraker).await;
    job(&state, p, "cancelled", Some("cancelled"), 5.0, 200.0).await;
    job(&state, p, "complete", Some("cancelled"), 5.0, 200.0).await;
    let v = task(&app, &auth, p, "Belt tension", "hours", 1.0).await;
    assert_eq!(
        v["counters"],
        json!({ "hours": 0.0, "grams": 0.0, "jobs": 0 })
    );
    assert_eq!(v["progress"]["state"], "ok");
    assert!(attention(&app, &auth).await.is_empty());

    // Rating a completed job "cancelled" afterwards takes it back out.
    job(&state, p, "complete", None, 2.0, 10.0).await;
    maintenance::evaluate_printer(&state.foreman_db.pool, &state.bus_tx, p)
        .await
        .expect("eval");
    assert_eq!(attention(&app, &auth).await.len(), 1);
    let (_, jobs) = call(&app, "GET", "/api/jobs?outcome=unrated", &auth, None).await;
    let id = jobs["jobs"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|j| j["id"].as_i64())
        .expect("unrated job");
    let (s, _) = call(
        &app,
        "PATCH",
        &format!("/api/jobs/{id}"),
        &auth,
        Some(json!({ "outcome": "cancelled" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(attention(&app, &auth).await.is_empty());
}

#[tokio::test]
async fn due_soon_and_overdue_thresholds_and_attention() {
    let (_d, app, state, auth) = setup().await;
    let p = add_printer(&state, "HellV0", FirmwareKind::Moonraker).await;
    history(&state, p).await; // 3.5 h

    // 3.5 / 4 = 87.5 %: below the default 90 % → ok.
    let t = task(&app, &auth, p, "Belt tension", "hours", 4.0).await;
    let id = t["task"]["id"].as_i64().expect("id");
    assert_eq!(t["progress"]["state"], "ok");

    // Lower the threshold to 85 % → due soon, shown in Service, not in Attention.
    let (s, _) = call(
        &app,
        "PUT",
        "/api/maintenance/settings",
        &auth,
        Some(json!({ "due_soon_pct": 85 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, ov) = call(&app, "GET", "/api/maintenance", &auth, None).await;
    assert_eq!(ov["due_soon_pct"], json!(85.0));
    assert_eq!(ov["tasks"][0]["progress"]["state"], "soon");
    assert!(attention(&app, &auth).await.is_empty());

    // Another hour prints (the job engine re-checks the printer when a job ends):
    // 4.5 / 4 → overdue, Attention raised once with the counter in its title.
    job(&state, p, "complete", None, 1.0, 5.0).await;
    maintenance::evaluate_printer(&state.foreman_db.pool, &state.bus_tx, p)
        .await
        .expect("eval");
    maintenance::evaluate_printer(&state.foreman_db.pool, &state.bus_tx, p)
        .await
        .expect("eval twice");
    let a = attention(&app, &auth).await;
    assert_eq!(a.len(), 1, "{a:?}");
    assert_eq!(
        a[0]["title"],
        "Belt tension overdue on HellV0 · 4.5 / 4 print hours"
    );
    assert_eq!(a[0]["kind"], format!("maintenance:{id}"));
    assert_eq!(a[0]["printer_id"], json!(p));

    // More hours retitle the same item instead of raising another.
    job(&state, p, "complete", None, 1.0, 5.0).await;
    maintenance::evaluate_printer(&state.foreman_db.pool, &state.bus_tx, p)
        .await
        .expect("eval");
    let a = attention(&app, &auth).await;
    assert_eq!(a.len(), 1);
    assert_eq!(
        a[0]["title"],
        "Belt tension overdue on HellV0 · 5.5 / 4 print hours"
    );

    // Overdue sorts first.
    task(&app, &auth, p, "Check fans", "hours", 500.0).await;
    let (_, ov) = call(&app, "GET", "/api/maintenance", &auth, None).await;
    assert_eq!(ov["tasks"][0]["task"]["id"], json!(id));
    assert_eq!(ov["tasks"][0]["progress"]["state"], "overdue");

    // Raising the interval clears it.
    let (s, v) = call(
        &app,
        "PUT",
        &format!("/api/maintenance/tasks/{id}"),
        &auth,
        Some(json!({ "name": "Belt tension", "basis": "hours", "interval": 100 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(attention(&app, &auth).await.is_empty());
}

#[tokio::test]
async fn mark_done_resets_logs_and_clears_attention() {
    let (_d, app, state, auth) = setup().await;
    let p = add_printer(&state, "HellV0", FirmwareKind::Moonraker).await;
    history(&state, p).await;
    let t = task(&app, &auth, p, "Clean bed", "jobs", 2.0).await;
    let id = t["task"]["id"].as_i64().expect("id");
    assert_eq!(t["progress"]["state"], "overdue");
    assert_eq!(attention(&app, &auth).await.len(), 1);

    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/maintenance/tasks/{id}/done"),
        &auth,
        Some(json!({ "notes": "  IPA wipe  " })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["log"]["counter"], json!(2.0));
    assert_eq!(v["log"]["done_by"], "admin");
    assert_eq!(v["log"]["notes"], "IPA wipe");
    assert_eq!(v["log"]["first"], json!(true));
    assert_eq!(v["log"]["interval"], json!(2.0));
    assert_eq!(v["task"]["progress"]["value"], json!(0.0));
    assert_eq!(v["task"]["progress"]["state"], "ok");
    assert!(v["task"]["last_done"].is_string());
    assert!(attention(&app, &auth).await.is_empty());

    // Jobs before the service no longer count; new ones do.
    job(&state, p, "complete", None, 0.5, 5.0).await;
    let (_, ov) = call(&app, "GET", "/api/maintenance", &auth, None).await;
    assert_eq!(ov["tasks"][0]["counters"]["jobs"], json!(1));

    // Undo restores the counter and the Attention item.
    let log_id = v["log"]["id"].as_i64().expect("log id");
    let (s, _) = call(
        &app,
        "DELETE",
        &format!("/api/maintenance/history/{log_id}"),
        &auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, ov) = call(&app, "GET", "/api/maintenance", &auth, None).await;
    assert_eq!(ov["tasks"][0]["counters"]["jobs"], json!(3));
    assert_eq!(attention(&app, &auth).await.len(), 1);
}

#[tokio::test]
async fn history_newest_first_with_gaps() {
    let (_d, app, state, auth) = setup().await;
    let p = add_printer(&state, "HellV0", FirmwareKind::Moonraker).await;
    let q = add_printer(&state, "Other", FirmwareKind::Moonraker).await;
    history(&state, p).await;
    let belt = task(&app, &auth, p, "Belt tension", "hours", 200.0).await["task"]["id"]
        .as_i64()
        .expect("id");
    let bed = task(&app, &auth, p, "Clean bed", "jobs", 20.0).await["task"]["id"]
        .as_i64()
        .expect("id");
    let fans = task(&app, &auth, q, "Check fans", "hours", 500.0).await["task"]["id"]
        .as_i64()
        .expect("id");
    let done = |id: i64, note: &'static str| {
        let app = app.clone();
        let auth = auth.clone();
        async move {
            let (s, v) = call(
                &app,
                "POST",
                &format!("/api/maintenance/tasks/{id}/done"),
                &auth,
                Some(json!({ "notes": note })),
            )
            .await;
            assert_eq!(s, StatusCode::OK, "{v}");
            v["log"]["id"].as_i64().expect("log")
        }
    };
    let b1 = done(belt, "first").await;
    job(&state, p, "complete", None, 1.5, 12.0).await;
    let c1 = done(bed, "bed").await;
    let b2 = done(belt, "second").await;
    done(fans, "fans").await;

    // Per task: newest first, second entry carries the gap since the first.
    let (_, h) = call(
        &app,
        "GET",
        &format!("/api/maintenance/history?task_id={belt}"),
        &auth,
        None,
    )
    .await;
    let ids: Vec<i64> = h
        .as_array()
        .expect("arr")
        .iter()
        .filter_map(|r| r["id"].as_i64())
        .collect();
    assert_eq!(ids, vec![b2, b1]);
    assert_eq!(h[0]["counter"], json!(1.5));
    assert_eq!(h[0]["first"], json!(false));
    assert_eq!(h[1]["counter"], json!(3.5));
    assert_eq!(h[1]["first"], json!(true));

    // Per printer: only that printer's services, newest first.
    let (_, h) = call(
        &app,
        "GET",
        &format!("/api/maintenance/history?printer_id={p}"),
        &auth,
        None,
    )
    .await;
    let ids: Vec<i64> = h
        .as_array()
        .expect("arr")
        .iter()
        .filter_map(|r| r["id"].as_i64())
        .collect();
    assert_eq!(ids, vec![b2, c1, b1]);

    // Only the latest service of a task can be undone.
    let (s, _) = call(
        &app,
        "DELETE",
        &format!("/api/maintenance/history/{b1}"),
        &auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn templates_offered_and_snooze() {
    let (_d, app, state, auth) = setup().await;
    let p = add_printer(&state, "HellV0", FirmwareKind::Moonraker).await;
    let (_, t) = call(&app, "GET", "/api/maintenance/templates", &auth, None).await;
    let keys: Vec<&str> = t
        .as_array()
        .expect("arr")
        .iter()
        .filter_map(|x| x["key"].as_str())
        .collect();
    assert_eq!(
        keys,
        vec![
            "belt_tension",
            "nozzle_change",
            "lube_rails",
            "clean_bed",
            "check_fans"
        ]
    );
    let (s, v) = call(
        &app,
        "POST",
        "/api/maintenance/from-templates",
        &auth,
        Some(json!({ "printer_id": p, "keys": ["belt_tension", "clean_bed"], "start_now": true })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v.as_array().map(Vec::len), Some(2));
    assert_eq!(v[1]["task"]["basis"], "jobs");
    assert_eq!(v[1]["task"]["interval"], json!(20.0));

    // Snooze: 2 / 2 jobs overdue → one snooze adds a whole job (2 / 3 → ok).
    history(&state, p).await;
    let id = task(&app, &auth, p, "Wipe", "jobs", 2.0).await["task"]["id"]
        .as_i64()
        .expect("id");
    let (_, v) = call(
        &app,
        "POST",
        &format!("/api/maintenance/tasks/{id}/snooze"),
        &auth,
        Some(json!({})),
    )
    .await;
    assert_eq!(v["progress"]["limit"], json!(3.0), "{v}");
    assert_eq!(v["snooze_amount"], json!(1.0));
    assert_eq!(v["progress"]["state"], "ok");
    assert!(attention(&app, &auth).await.is_empty());
    let (_, v) = call(
        &app,
        "POST",
        &format!("/api/maintenance/tasks/{id}/snooze"),
        &auth,
        Some(json!({ "snooze": 0 })),
    )
    .await;
    assert_eq!(v["progress"]["state"], "overdue");
    assert_eq!(attention(&app, &auth).await.len(), 1);

    // Deleting a task clears its item.
    let (s, _) = call(
        &app,
        "DELETE",
        &format!("/api/maintenance/tasks/{id}"),
        &auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(attention(&app, &auth).await.is_empty());
}
