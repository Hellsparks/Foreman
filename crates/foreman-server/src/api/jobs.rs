//! Job history + stats REST (task 3.6).
//!
//! - `GET    /api/jobs`        — history; `printer_id, from, to, material, spool_id,
//!   outcome (good|failed|cancelled|unrated|running), q, limit, offset`
//! - `GET    /api/jobs/stats`  — `from, to` (default: last 7 days)
//! - `GET    /api/jobs/{id}`
//! - `PATCH  /api/jobs/{id}`   — `{outcome, fail_reason?}` rate a job
//! - `DELETE /api/jobs/{id}`
//!
//! Changes broadcast a `job` event (`{kind, job_id}`); clients refetch.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{Duration, NaiveDate, Utc};
use foreman_core::{
    domain::{Job, JobList, JobOutcomeUpdate, JobStats},
    error::AppError,
    events::ServerEvent,
    repo::{self, job::JobFilter},
};
use serde::Deserialize;
use serde_json::json;

use crate::state::AppState;

use super::queue::err;

type ApiErr = (StatusCode, Json<serde_json::Value>);

fn bad(msg: impl Into<String>) -> ApiErr {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": msg.into() })),
    )
}

/// Accept `YYYY-MM-DD` or RFC 3339; returns RFC 3339 (UTC). `end_of_day` turns a
/// bare date into the last instant of that day.
fn parse_when(s: &str, end_of_day: bool) -> Result<String, ApiErr> {
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(t.with_timezone(&Utc).to_rfc3339());
    }
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| bad(format!("bad date {s:?} (use YYYY-MM-DD or RFC 3339)")))?;
    let t = if end_of_day {
        d.and_hms_milli_opt(23, 59, 59, 999)
    } else {
        d.and_hms_opt(0, 0, 0)
    }
    .ok_or_else(|| bad(format!("bad date {s:?}")))?;
    Ok(t.and_utc().to_rfc3339())
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    printer_id: Option<i64>,
    from: Option<String>,
    to: Option<String>,
    material: Option<String>,
    spool_id: Option<i64>,
    outcome: Option<String>,
    q: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

/// `GET /api/jobs`
pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<JobList>, ApiErr> {
    if let Some(o) = q.outcome.as_deref() {
        if !matches!(
            o,
            "good" | "failed" | "cancelled" | "unrated" | "running" | "all"
        ) {
            return Err(bad(format!("unknown outcome {o:?}")));
        }
    }
    let f = JobFilter {
        printer_id: q.printer_id,
        from: q
            .from
            .as_deref()
            .map(|s| parse_when(s, false))
            .transpose()?,
        to: q.to.as_deref().map(|s| parse_when(s, true)).transpose()?,
        material: q.material.filter(|m| !m.trim().is_empty()),
        spool_id: q.spool_id,
        outcome: q.outcome.filter(|o| o != "all"),
        q: q.q,
        limit: q.limit.unwrap_or(200),
        offset: q.offset.unwrap_or(0),
    };
    let (jobs, total) = repo::job::list(&state.foreman_db.pool, &f)
        .await
        .map_err(err)?;
    Ok(Json(JobList { jobs, total }))
}

/// `GET /api/jobs/{id}`
pub async fn get(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<Job>, ApiErr> {
    repo::job::get(&state.foreman_db.pool, id)
        .await
        .map(Json)
        .map_err(err)
}

/// `PATCH /api/jobs/{id}` `{outcome, fail_reason?}`
pub async fn rate(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<JobOutcomeUpdate>,
) -> Result<Json<Job>, ApiErr> {
    let job = repo::job::set_outcome(
        &state.foreman_db.pool,
        id,
        body.outcome,
        body.fail_reason.as_deref(),
    )
    .await
    .map_err(err)?;
    tracing::info!(job = id, outcome = body.outcome.as_str(), reason = ?job.fail_reason, "job rated");
    let _ = state.bus_tx.send(ServerEvent::Job {
        printer_id: job.printer_id,
        payload: json!({ "kind": "updated", "job_id": id }),
    });
    Ok(Json(job))
}

/// `DELETE /api/jobs/{id}`
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiErr> {
    let job = repo::job::get(&state.foreman_db.pool, id)
        .await
        .map_err(err)?;
    if job.end == foreman_core::domain::JobEnd::Running {
        return Err(err(AppError::InvalidInput(
            "job is still running; delete it after it ends".into(),
        )));
    }
    repo::job::delete(&state.foreman_db.pool, id)
        .await
        .map_err(err)?;
    let _ = state.bus_tx.send(ServerEvent::Job {
        printer_id: job.printer_id,
        payload: json!({ "kind": "deleted", "job_id": id }),
    });
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct StatsQuery {
    from: Option<String>,
    to: Option<String>,
}

/// `GET /api/jobs/stats`
pub async fn stats(
    State(state): State<AppState>,
    Query(q): Query<StatsQuery>,
) -> Result<Json<JobStats>, ApiErr> {
    let to = match q.to.as_deref() {
        Some(s) => parse_when(s, true)?,
        None => Utc::now().to_rfc3339(),
    };
    let from = match q.from.as_deref() {
        Some(s) => parse_when(s, false)?,
        None => (Utc::now() - Duration::days(7)).to_rfc3339(),
    };
    if from > to {
        return Err(bad("from is after to"));
    }
    repo::job::stats(&state.foreman_db.pool, &from, &to)
        .await
        .map(Json)
        .map_err(err)
}
