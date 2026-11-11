//! Repository functions for the `job` table (task 3.6).
//!
//! A job is opened when a print starts and closed once when it ends ([`close`] only
//! touches a `running` row). Filament deduction is claimed with [`claim_deduction`],
//! a conditional `none → pending` update, so it runs at most once per job even if
//! the server restarts mid-print or an end event repeats.

use chrono::Utc;
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};

use crate::{
    domain::{Job, JobEnd, JobOutcome, JobStats, JobTool, PrinterStats, UsageStat},
    error::AppError,
};

#[derive(FromRow)]
struct JobRow {
    id: i64,
    printer_id: Option<i64>,
    printer_name: String,
    file_id: Option<i64>,
    file_name: String,
    queue_item_id: Option<i64>,
    started_at: String,
    ended_at: Option<String>,
    duration_secs: Option<i64>,
    end_state: String,
    outcome: Option<String>,
    fail_reason: Option<String>,
    error: Option<String>,
    usage_reporter: String,
    grams_planned: Option<f64>,
    grams_used: Option<f64>,
    grams_deducted: f64,
    material: Option<String>,
    tools_json: String,
    thumb_url: Option<String>,
}

impl JobRow {
    fn into_domain(self) -> Job {
        Job {
            id: self.id,
            printer_id: self.printer_id,
            printer_name: self.printer_name,
            file_id: self.file_id,
            file_name: self.file_name,
            queue_item_id: self.queue_item_id,
            started_at: self.started_at,
            ended_at: self.ended_at,
            duration_secs: self.duration_secs,
            end: JobEnd::parse(&self.end_state),
            outcome: self.outcome.as_deref().and_then(JobOutcome::parse),
            fail_reason: self.fail_reason,
            error: self.error,
            usage_reporter: self.usage_reporter,
            grams_planned: self.grams_planned,
            grams_used: self.grams_used,
            grams_deducted: self.grams_deducted,
            material: self.material,
            tools: serde_json::from_str(&self.tools_json).unwrap_or_default(),
            thumb_url: self.thumb_url,
        }
    }
}

const COLS: &str = "id, printer_id, printer_name, file_id, file_name, queue_item_id, started_at, \
     ended_at, duration_secs, end_state, outcome, fail_reason, error, usage_reporter, \
     grams_planned, grams_used, grams_deducted, material, tools_json, thumb_url";

fn db(e: sqlx::Error) -> AppError {
    AppError::Database(e.to_string())
}

fn tools_json(tools: &[JobTool]) -> String {
    serde_json::to_string(tools).unwrap_or_else(|_| "[]".to_owned())
}

/// What is known when a print starts.
#[derive(Debug, Clone, Default)]
pub struct NewJob {
    pub printer_id: i64,
    pub printer_name: String,
    pub file_id: Option<i64>,
    pub file_name: String,
    pub queue_item_id: Option<i64>,
    pub usage_reporter: String,
    pub tools: Vec<JobTool>,
    pub thumb_url: Option<String>,
}

/// Open a running job.
pub async fn open(pool: &SqlitePool, j: &NewJob) -> Result<Job, AppError> {
    let planned: Option<f64> = {
        let g: Vec<f64> = j.tools.iter().filter_map(|t| t.grams_planned).collect();
        (!g.is_empty()).then(|| g.iter().sum())
    };
    let material = j.tools.first().and_then(|t| t.material.clone());
    let id = sqlx::query(
        "INSERT INTO job (printer_id, printer_name, file_id, file_name, queue_item_id, \
         started_at, usage_reporter, grams_planned, material, tools_json, thumb_url) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(j.printer_id)
    .bind(&j.printer_name)
    .bind(j.file_id)
    .bind(&j.file_name)
    .bind(j.queue_item_id)
    .bind(Utc::now().to_rfc3339())
    .bind(&j.usage_reporter)
    .bind(planned)
    .bind(material)
    .bind(tools_json(&j.tools))
    .bind(&j.thumb_url)
    .execute(pool)
    .await
    .map_err(db)?
    .last_insert_rowid();
    get(pool, id).await
}

pub async fn get(pool: &SqlitePool, id: i64) -> Result<Job, AppError> {
    let sql = format!("SELECT {COLS} FROM job WHERE id = ?");
    let row: Option<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db)?;
    row.map(JobRow::into_domain)
        .ok_or_else(|| AppError::NotFound(format!("job {id}")))
}

/// The printer's running job, if any (newest first).
pub async fn running_for_printer(
    pool: &SqlitePool,
    printer_id: i64,
) -> Result<Option<Job>, AppError> {
    let sql = format!(
        "SELECT {COLS} FROM job WHERE printer_id = ? AND end_state = 'running' \
         ORDER BY id DESC LIMIT 1"
    );
    let row: Option<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(printer_id)
        .fetch_optional(pool)
        .await
        .map_err(db)?;
    Ok(row.map(JobRow::into_domain))
}

/// How a job ended.
#[derive(Debug, Clone)]
pub struct JobClose {
    pub end: JobEnd,
    pub duration_secs: Option<i64>,
    pub error: Option<String>,
    /// Starting outcome: cancelled → Cancelled, error → Failed, complete → None.
    pub outcome: Option<JobOutcome>,
    /// Per-tool `grams_used` filled in.
    pub tools: Vec<JobTool>,
}

/// Close a running job. Returns false when it was already closed.
pub async fn close(pool: &SqlitePool, id: i64, c: &JobClose) -> Result<bool, AppError> {
    let used: Option<f64> = {
        let g: Vec<f64> = c.tools.iter().filter_map(|t| t.grams_used).collect();
        (!g.is_empty()).then(|| g.iter().sum())
    };
    let n = sqlx::query(
        "UPDATE job SET end_state = ?, ended_at = ?, duration_secs = ?, error = ?, \
         outcome = COALESCE(outcome, ?), grams_used = ?, tools_json = ? \
         WHERE id = ? AND end_state = 'running'",
    )
    .bind(c.end.as_str())
    .bind(Utc::now().to_rfc3339())
    .bind(c.duration_secs)
    .bind(&c.error)
    .bind(c.outcome.map(JobOutcome::as_str))
    .bind(used)
    .bind(tools_json(&c.tools))
    .bind(id)
    .execute(pool)
    .await
    .map_err(db)?
    .rows_affected();
    Ok(n == 1)
}

/// Claim the job's deduction (`none → pending`). Only the caller that gets `true`
/// may deduct.
pub async fn claim_deduction(pool: &SqlitePool, id: i64) -> Result<bool, AppError> {
    let n = sqlx::query(
        "UPDATE job SET deduct_state = 'pending' WHERE id = ? AND deduct_state = 'none'",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(db)?
    .rows_affected();
    Ok(n == 1)
}

/// Record what was deducted (`state` = done | skipped).
pub async fn finish_deduction(
    pool: &SqlitePool,
    id: i64,
    tools: &[JobTool],
    state: &str,
) -> Result<(), AppError> {
    let total: f64 = tools.iter().map(|t| t.grams_deducted).sum();
    sqlx::query("UPDATE job SET deduct_state = ?, grams_deducted = ?, tools_json = ? WHERE id = ?")
        .bind(state)
        .bind(total)
        .bind(tools_json(tools))
        .bind(id)
        .execute(pool)
        .await
        .map_err(db)?;
    Ok(())
}

pub async fn deduct_state(pool: &SqlitePool, id: i64) -> Result<String, AppError> {
    sqlx::query_scalar("SELECT deduct_state FROM job WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db)?
        .ok_or_else(|| AppError::NotFound(format!("job {id}")))
}

pub async fn set_outcome(
    pool: &SqlitePool,
    id: i64,
    outcome: JobOutcome,
    fail_reason: Option<&str>,
) -> Result<Job, AppError> {
    let reason = if outcome == JobOutcome::Failed {
        fail_reason.map(str::trim).filter(|r| !r.is_empty())
    } else {
        None
    };
    let n = sqlx::query("UPDATE job SET outcome = ?, fail_reason = ? WHERE id = ?")
        .bind(outcome.as_str())
        .bind(reason)
        .bind(id)
        .execute(pool)
        .await
        .map_err(db)?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound(format!("job {id}")));
    }
    get(pool, id).await
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<(), AppError> {
    let n = sqlx::query("DELETE FROM job WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(db)?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound(format!("job {id}")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

/// History filters. Dates are RFC 3339 or `YYYY-MM-DD`, compared to `started_at`.
#[derive(Debug, Clone, Default)]
pub struct JobFilter {
    pub printer_id: Option<i64>,
    pub from: Option<String>,
    pub to: Option<String>,
    /// Matches any tool's material, case-insensitive.
    pub material: Option<String>,
    pub spool_id: Option<i64>,
    /// good | failed | cancelled | unrated | running.
    pub outcome: Option<String>,
    /// Substring of the file name.
    pub q: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

fn push_filters(b: &mut QueryBuilder<Sqlite>, f: &JobFilter) {
    b.push(" WHERE 1=1");
    if let Some(p) = f.printer_id {
        b.push(" AND printer_id = ").push_bind(p);
    }
    if let Some(from) = &f.from {
        b.push(" AND started_at >= ").push_bind(from.clone());
    }
    if let Some(to) = &f.to {
        // A bare date means "through the end of that day".
        let to = if to.len() == 10 {
            format!("{to}T23:59:59.999999999+00:00")
        } else {
            to.clone()
        };
        b.push(" AND started_at <= ").push_bind(to);
    }
    if let Some(m) = &f.material {
        b.push(
            " AND EXISTS (SELECT 1 FROM json_each(job.tools_json) t \
             WHERE lower(json_extract(t.value, '$.material')) = lower(",
        )
        .push_bind(m.clone())
        .push("))");
    }
    if let Some(s) = f.spool_id {
        b.push(
            " AND EXISTS (SELECT 1 FROM json_each(job.tools_json) t \
             WHERE json_extract(t.value, '$.spool_id') = ",
        )
        .push_bind(s)
        .push(")");
    }
    match f.outcome.as_deref() {
        Some("unrated") => {
            b.push(" AND outcome IS NULL AND end_state != 'running'");
        }
        Some("running") => {
            b.push(" AND end_state = 'running'");
        }
        Some(o @ ("good" | "failed" | "cancelled")) => {
            b.push(" AND outcome = ").push_bind(o.to_owned());
        }
        _ => {}
    }
    if let Some(q) = f.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        let esc = q
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        b.push(" AND file_name LIKE ")
            .push_bind(format!("%{esc}%"))
            .push(" ESCAPE '\\'");
    }
}

/// Matching jobs, newest first, and the total count.
pub async fn list(pool: &SqlitePool, f: &JobFilter) -> Result<(Vec<Job>, i64), AppError> {
    let mut b = QueryBuilder::<Sqlite>::new(format!("SELECT {COLS} FROM job"));
    push_filters(&mut b, f);
    b.push(" ORDER BY started_at DESC, id DESC LIMIT ")
        .push_bind(f.limit.clamp(1, 1000))
        .push(" OFFSET ")
        .push_bind(f.offset.max(0));
    let rows: Vec<JobRow> = b.build_query_as().fetch_all(pool).await.map_err(db)?;

    let mut c = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM job");
    push_filters(&mut c, f);
    let total: i64 = c.build_query_scalar().fetch_one(pool).await.map_err(db)?;
    Ok((rows.into_iter().map(JobRow::into_domain).collect(), total))
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

#[derive(FromRow)]
struct PrinterAgg {
    printer_id: Option<i64>,
    name: String,
    secs: i64,
    jobs: i64,
    good: i64,
    failed: i64,
    cancelled: i64,
    running: i64,
}

#[derive(FromRow)]
struct UsageAgg {
    key: Option<String>,
    spool_id: Option<i64>,
    grams: f64,
    jobs: i64,
}

fn rate(failed: i64, jobs: i64, cancelled: i64, running: i64) -> Option<f64> {
    let base = jobs - cancelled - running;
    (base > 0).then(|| failed as f64 / base as f64)
}

/// Stats for jobs started in `[from, to]` (RFC 3339).
pub async fn stats(pool: &SqlitePool, from: &str, to: &str) -> Result<JobStats, AppError> {
    let period_secs = chrono::DateTime::parse_from_rfc3339(to)
        .ok()
        .zip(chrono::DateTime::parse_from_rfc3339(from).ok())
        .map(|(t, f)| (t - f).num_seconds().max(1))
        .unwrap_or(1);

    let per: Vec<PrinterAgg> = sqlx::query_as(
        "SELECT printer_id, MAX(printer_name) AS name, \
           COALESCE(SUM(duration_secs), 0) AS secs, COUNT(*) AS jobs, \
           SUM(CASE WHEN outcome = 'good' THEN 1 ELSE 0 END) AS good, \
           SUM(CASE WHEN outcome = 'failed' THEN 1 ELSE 0 END) AS failed, \
           SUM(CASE WHEN outcome = 'cancelled' THEN 1 ELSE 0 END) AS cancelled, \
           SUM(CASE WHEN end_state = 'running' THEN 1 ELSE 0 END) AS running \
         FROM job WHERE started_at >= ? AND started_at <= ? \
         GROUP BY printer_id ORDER BY secs DESC, name",
    )
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await
    .map_err(db)?;

    let usage_sql = |group: &str| {
        format!(
            "SELECT {group}, ROUND(SUM(COALESCE(json_extract(t.value, '$.grams_used'), 0)), 2) AS grams, \
               COUNT(DISTINCT CASE WHEN COALESCE(json_extract(t.value, '$.grams_used'), 0) > 0 THEN job.id END) AS jobs \
             FROM job, json_each(job.tools_json) t \
             WHERE job.started_at >= ? AND job.started_at <= ? AND job.end_state != 'running' \
             GROUP BY 1, 2 HAVING grams > 0 ORDER BY grams DESC"
        )
    };
    let by_material: Vec<UsageAgg> = sqlx::query_as(sqlx::AssertSqlSafe(usage_sql(
        "COALESCE(upper(json_extract(t.value, '$.material')), 'Unknown') AS key, NULL AS spool_id",
    )))
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await
    .map_err(db)?;
    let by_spool: Vec<UsageAgg> = sqlx::query_as(sqlx::AssertSqlSafe(usage_sql(
        "COALESCE(upper(json_extract(t.value, '$.material')), 'Unknown') AS key, \
         json_extract(t.value, '$.spool_id') AS spool_id",
    )))
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await
    .map_err(db)?;

    let mut out = JobStats {
        from: from.to_owned(),
        to: to.to_owned(),
        hours: 0.0,
        jobs: 0,
        good: 0,
        failed: 0,
        cancelled: 0,
        unrated: 0,
        success_rate: None,
        grams: 0.0,
        printers: Vec::with_capacity(per.len()),
        by_material: Vec::new(),
        by_spool: Vec::new(),
    };
    let mut running = 0;
    for p in per {
        let hours = p.secs as f64 / 3600.0;
        out.hours += hours;
        out.jobs += p.jobs;
        out.good += p.good;
        out.failed += p.failed;
        out.cancelled += p.cancelled;
        running += p.running;
        out.unrated += p.jobs - p.good - p.failed - p.cancelled - p.running;
        out.printers.push(PrinterStats {
            printer_id: p.printer_id,
            name: p.name,
            hours: round2(hours),
            jobs: p.jobs,
            good: p.good,
            failed: p.failed,
            cancelled: p.cancelled,
            failure_rate: round4(rate(p.failed, p.jobs, p.cancelled, p.running).unwrap_or(0.0)),
            utilization_pct: round2((p.secs as f64 / period_secs as f64 * 100.0).min(100.0)),
        });
    }
    out.hours = round2(out.hours);
    out.success_rate = rate(out.failed, out.jobs, out.cancelled, running).map(|r| round4(1.0 - r));
    out.by_material = by_material
        .into_iter()
        .map(|u| UsageStat {
            key: u.key.unwrap_or_else(|| "Unknown".into()),
            spool_id: None,
            grams: u.grams,
            jobs: u.jobs,
        })
        .collect();
    // `+ 0.0`: an empty f64 sum is -0.0.
    out.grams = round2(out.by_material.iter().map(|u| u.grams).sum::<f64>() + 0.0);
    out.by_spool = by_spool
        .into_iter()
        .map(|u| UsageStat {
            key: u.key.unwrap_or_else(|| "Unknown".into()),
            spool_id: u.spool_id,
            grams: u.grams,
            jobs: u.jobs,
        })
        .collect();
    Ok(out)
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ForemanDb;

    async fn setup() -> (ForemanDb, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().expect("tmp");
        let db = ForemanDb::connect(&tmp.path().join("j.db"))
            .await
            .expect("db");
        for (id, name) in [(1, "Alpha"), (2, "Bravo")] {
            sqlx::query(
                "INSERT INTO printer (id, name, firmware, tags, created_at, updated_at) \
                 VALUES (?, ?, 'moonraker', '[]', 'x', 'x')",
            )
            .bind(id)
            .bind(name)
            .execute(&db.pool)
            .await
            .expect("printer");
        }
        (db, tmp)
    }

    fn tool(material: &str, grams: f64, spool: Option<i64>) -> JobTool {
        JobTool {
            index: 0,
            material: Some(material.into()),
            grams_planned: Some(grams),
            spool_id: spool,
            ..Default::default()
        }
    }

    /// Open + close a job that started at `started` and ran `secs`, used `frac`.
    #[allow(clippy::too_many_arguments)]
    async fn job(
        pool: &SqlitePool,
        printer: i64,
        file: &str,
        started: &str,
        secs: i64,
        tools: Vec<JobTool>,
        end: JobEnd,
        outcome: Option<JobOutcome>,
    ) -> i64 {
        let j = open(
            pool,
            &NewJob {
                printer_id: printer,
                printer_name: if printer == 1 { "Alpha" } else { "Bravo" }.into(),
                file_name: file.into(),
                usage_reporter: "foreman".into(),
                tools: tools.clone(),
                ..Default::default()
            },
        )
        .await
        .expect("open");
        let frac = if end == JobEnd::Complete { 1.0 } else { 0.5 };
        let used: Vec<JobTool> = tools
            .into_iter()
            .map(|mut t| {
                t.grams_used = t.grams_planned.map(|g| g * frac);
                t
            })
            .collect();
        assert!(close(
            pool,
            j.id,
            &JobClose {
                end,
                duration_secs: Some(secs),
                error: None,
                outcome,
                tools: used,
            },
        )
        .await
        .expect("close"));
        sqlx::query("UPDATE job SET started_at = ? WHERE id = ?")
            .bind(started)
            .bind(j.id)
            .execute(pool)
            .await
            .expect("backdate");
        j.id
    }

    async fn seed(pool: &SqlitePool) -> [i64; 5] {
        [
            job(
                pool,
                1,
                "cube.gcode",
                "2026-11-01T10:00:00+00:00",
                3600,
                vec![tool("PLA", 20.0, Some(7))],
                JobEnd::Complete,
                Some(JobOutcome::Good),
            )
            .await,
            job(
                pool,
                1,
                "hinge.gcode",
                "2026-11-02T10:00:00+00:00",
                1800,
                vec![tool("PETG", 10.0, Some(8))],
                JobEnd::Error,
                Some(JobOutcome::Failed),
            )
            .await,
            job(
                pool,
                2,
                "benchy.gcode",
                "2026-11-03T10:00:00+00:00",
                7200,
                vec![tool("pla", 30.0, Some(7))],
                JobEnd::Complete,
                None,
            )
            .await,
            job(
                pool,
                2,
                "bracket.gcode",
                "2026-11-04T10:00:00+00:00",
                600,
                vec![tool("PLA", 8.0, None)],
                JobEnd::Cancelled,
                Some(JobOutcome::Cancelled),
            )
            .await,
            // Outside the stats window.
            job(
                pool,
                1,
                "old.gcode",
                "2026-10-01T10:00:00+00:00",
                3600,
                vec![tool("ABS", 50.0, Some(9))],
                JobEnd::Complete,
                Some(JobOutcome::Good),
            )
            .await,
        ]
    }

    fn f() -> JobFilter {
        JobFilter {
            limit: 100,
            ..Default::default()
        }
    }

    async fn files(pool: &SqlitePool, f: JobFilter) -> Vec<String> {
        let (jobs, total) = list(pool, &f).await.expect("list");
        assert_eq!(total as usize, jobs.len());
        jobs.into_iter().map(|j| j.file_name).collect()
    }

    #[tokio::test]
    async fn history_filters() {
        let (db, _t) = setup().await;
        let p = &db.pool;
        seed(p).await;
        // Newest first.
        assert_eq!(
            files(p, f()).await,
            [
                "bracket.gcode",
                "benchy.gcode",
                "hinge.gcode",
                "cube.gcode",
                "old.gcode"
            ]
        );
        assert_eq!(
            files(
                p,
                JobFilter {
                    printer_id: Some(2),
                    ..f()
                }
            )
            .await,
            ["bracket.gcode", "benchy.gcode"]
        );
        // Material is case-insensitive.
        assert_eq!(
            files(
                p,
                JobFilter {
                    material: Some("PLA".into()),
                    ..f()
                }
            )
            .await,
            ["bracket.gcode", "benchy.gcode", "cube.gcode"]
        );
        assert_eq!(
            files(
                p,
                JobFilter {
                    spool_id: Some(7),
                    ..f()
                }
            )
            .await,
            ["benchy.gcode", "cube.gcode"]
        );
        assert_eq!(
            files(
                p,
                JobFilter {
                    outcome: Some("failed".into()),
                    ..f()
                }
            )
            .await,
            ["hinge.gcode"]
        );
        assert_eq!(
            files(
                p,
                JobFilter {
                    outcome: Some("unrated".into()),
                    ..f()
                }
            )
            .await,
            ["benchy.gcode"]
        );
        assert_eq!(
            files(
                p,
                JobFilter {
                    outcome: Some("cancelled".into()),
                    ..f()
                }
            )
            .await,
            ["bracket.gcode"]
        );
        // Date range: bare `to` date includes that whole day.
        assert_eq!(
            files(
                p,
                JobFilter {
                    from: Some("2026-11-02T00:00:00+00:00".into()),
                    to: Some("2026-11-03".into()),
                    ..f()
                }
            )
            .await,
            ["benchy.gcode", "hinge.gcode"]
        );
        assert_eq!(
            files(
                p,
                JobFilter {
                    q: Some("ben".into()),
                    ..f()
                }
            )
            .await,
            ["benchy.gcode"]
        );
        // LIKE wildcards in the search are literal.
        assert!(files(
            p,
            JobFilter {
                q: Some("%".into()),
                ..f()
            }
        )
        .await
        .is_empty());
        // Combined + paging.
        let (page, total) = list(
            p,
            &JobFilter {
                limit: 2,
                offset: 1,
                ..f()
            },
        )
        .await
        .expect("page");
        assert_eq!(total, 5);
        assert_eq!(
            page.iter()
                .map(|j| j.file_name.as_str())
                .collect::<Vec<_>>(),
            ["benchy.gcode", "hinge.gcode"]
        );
    }

    #[tokio::test]
    async fn rate_and_delete() {
        let (db, _t) = setup().await;
        let p = &db.pool;
        let [_, hinge, benchy, ..] = seed(p).await;
        let j = set_outcome(p, benchy, JobOutcome::Failed, Some(" spaghetti "))
            .await
            .expect("rate");
        assert_eq!(j.outcome, Some(JobOutcome::Failed));
        assert_eq!(j.fail_reason.as_deref(), Some("spaghetti"));
        // Good clears the reason.
        let j = set_outcome(p, benchy, JobOutcome::Good, Some("x"))
            .await
            .expect("rate");
        assert_eq!(j.fail_reason, None);
        delete(p, hinge).await.expect("delete");
        assert!(matches!(get(p, hinge).await, Err(AppError::NotFound(_))));
        assert!(matches!(delete(p, hinge).await, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn stats_arithmetic() {
        let (db, _t) = setup().await;
        let p = &db.pool;
        seed(p).await;
        // 2026-11-01 .. 2026-11-05 = 4 days = 96 h.
        let s = stats(p, "2026-11-01T00:00:00+00:00", "2026-11-05T00:00:00+00:00")
            .await
            .expect("stats");
        assert_eq!(s.jobs, 4);
        assert_eq!((s.good, s.failed, s.cancelled, s.unrated), (1, 1, 1, 1));
        // 3600 + 1800 + 7200 + 600 s = 13200 s = 3.67 h.
        assert_eq!(s.hours, 3.67);
        // Failure rate base excludes cancelled: 1 failed of 3 → success 0.6667.
        assert_eq!(s.success_rate, Some(0.6667));
        // Used grams: 20 + 10×0.5 + 30 + 8×0.5 = 59.
        assert_eq!(s.grams, 59.0);

        let alpha = s
            .printers
            .iter()
            .find(|x| x.printer_id == Some(1))
            .expect("alpha");
        assert_eq!((alpha.jobs, alpha.good, alpha.failed), (2, 1, 1));
        assert_eq!(alpha.hours, 1.5);
        assert_eq!(alpha.failure_rate, 0.5);
        // 5400 s of 345600 s = 1.5625 %.
        assert_eq!(alpha.utilization_pct, 1.56);
        let bravo = s
            .printers
            .iter()
            .find(|x| x.printer_id == Some(2))
            .expect("bravo");
        assert_eq!((bravo.jobs, bravo.cancelled), (2, 1));
        assert_eq!(bravo.hours, 2.17);
        // 0 failed of 1 (benchy; bracket cancelled).
        assert_eq!(bravo.failure_rate, 0.0);
        // Sorted by hours: Bravo first.
        assert_eq!(s.printers[0].name, "Bravo");

        let mat: Vec<(&str, f64)> = s
            .by_material
            .iter()
            .map(|u| (u.key.as_str(), u.grams))
            .collect();
        assert_eq!(mat, [("PLA", 54.0), ("PETG", 5.0)]);
        let spool: Vec<(Option<i64>, f64, i64)> = s
            .by_spool
            .iter()
            .map(|u| (u.spool_id, u.grams, u.jobs))
            .collect();
        assert_eq!(
            spool,
            [(Some(7), 50.0, 2), (Some(8), 5.0, 1), (None, 4.0, 1)]
        );

        // A job that used 0 g of a spool does not count as a job for it.
        job(
            p,
            1,
            "aborted.gcode",
            "2026-11-04T12:00:00+00:00",
            60,
            vec![JobTool {
                grams_used: Some(0.0),
                ..tool("PLA", 0.0, Some(7))
            }],
            JobEnd::Cancelled,
            Some(JobOutcome::Cancelled),
        )
        .await;
        let s2 = stats(p, "2026-11-01T00:00:00+00:00", "2026-11-05T00:00:00+00:00")
            .await
            .expect("stats");
        let s7 = s2
            .by_spool
            .iter()
            .find(|u| u.spool_id == Some(7))
            .expect("#7");
        assert_eq!((s7.grams, s7.jobs), (50.0, 2));

        // Empty period.
        let e = stats(p, "2025-01-01T00:00:00+00:00", "2025-01-02T00:00:00+00:00")
            .await
            .expect("empty");
        assert_eq!((e.jobs, e.hours, e.grams), (0, 0.0, 0.0));
        assert!(e.grams.is_sign_positive(), "no -0.0 in the JSON");
        assert_eq!(e.success_rate, None);
        assert!(e.printers.is_empty());
    }

    #[tokio::test]
    async fn close_and_claim_happen_once() {
        let (db, _t) = setup().await;
        let p = &db.pool;
        let j = open(
            p,
            &NewJob {
                printer_id: 1,
                printer_name: "Alpha".into(),
                file_name: "x.gcode".into(),
                usage_reporter: "foreman".into(),
                ..Default::default()
            },
        )
        .await
        .expect("open");
        assert_eq!(
            running_for_printer(p, 1).await.expect("q").map(|x| x.id),
            Some(j.id)
        );
        let c = JobClose {
            end: JobEnd::Complete,
            duration_secs: Some(1),
            error: None,
            outcome: None,
            tools: vec![],
        };
        assert!(close(p, j.id, &c).await.expect("close"));
        assert!(!close(p, j.id, &c).await.expect("close again"));
        assert!(running_for_printer(p, 1).await.expect("q").is_none());
        assert!(claim_deduction(p, j.id).await.expect("claim"));
        assert!(!claim_deduction(p, j.id).await.expect("claim again"));
    }
}
