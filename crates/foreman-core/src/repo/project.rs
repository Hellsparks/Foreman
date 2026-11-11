//! Repository functions for templates and projects (task 5.1).

use chrono::Utc;
use sqlx::{FromRow, Row, SqlitePool};

use crate::{
    domain::QueueStatus,
    error::AppError,
    projects::{TemplateForm, TemplatePlate, TemplateSlot, ToolSlot},
};

fn db(e: sqlx::Error) -> AppError {
    AppError::Database(e.to_string())
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Body {
    #[serde(default)]
    slots: Vec<TemplateSlot>,
    #[serde(default)]
    plates: Vec<TemplatePlate>,
}

#[derive(Debug, Clone)]
pub struct TemplateRow {
    pub id: i64,
    pub form: TemplateForm,
    pub created_at: String,
    pub updated_at: String,
}

fn template_row(r: &sqlx::sqlite::SqliteRow) -> Result<TemplateRow, AppError> {
    let body: Body =
        serde_json::from_str(&r.try_get::<String, _>("body_json").map_err(db)?).unwrap_or_default();
    Ok(TemplateRow {
        id: r.try_get("id").map_err(db)?,
        form: TemplateForm {
            name: r.try_get("name").map_err(db)?,
            category: r.try_get("category").map_err(db)?,
            notes: r.try_get("notes").map_err(db)?,
            thumb_file_id: r.try_get("thumb_file_id").map_err(db)?,
            slots: body.slots,
            plates: body.plates,
        },
        created_at: r.try_get("created_at").map_err(db)?,
        updated_at: r.try_get("updated_at").map_err(db)?,
    })
}

fn body_json(f: &TemplateForm) -> String {
    serde_json::to_string(&Body {
        slots: f.slots.clone(),
        plates: f.plates.clone(),
    })
    .unwrap_or_else(|_| "{}".into())
}

pub async fn list_templates(pool: &SqlitePool) -> Result<Vec<TemplateRow>, AppError> {
    sqlx::query("SELECT * FROM template ORDER BY name COLLATE NOCASE, id")
        .fetch_all(pool)
        .await
        .map_err(db)?
        .iter()
        .map(template_row)
        .collect()
}

pub async fn get_template(pool: &SqlitePool, id: i64) -> Result<TemplateRow, AppError> {
    let r = sqlx::query("SELECT * FROM template WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db)?
        .ok_or_else(|| AppError::NotFound(format!("template {id}")))?;
    template_row(&r)
}

pub async fn create_template(pool: &SqlitePool, f: &TemplateForm) -> Result<TemplateRow, AppError> {
    let ts = now();
    let id = sqlx::query(
        "INSERT INTO template (name, category, notes, thumb_file_id, body_json, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(f.name.trim())
    .bind(&f.category)
    .bind(&f.notes)
    .bind(f.thumb_file_id)
    .bind(body_json(f))
    .bind(&ts)
    .bind(&ts)
    .execute(pool)
    .await
    .map_err(db)?
    .last_insert_rowid();
    get_template(pool, id).await
}

pub async fn update_template(
    pool: &SqlitePool,
    id: i64,
    f: &TemplateForm,
) -> Result<TemplateRow, AppError> {
    let n = sqlx::query(
        "UPDATE template SET name = ?, category = ?, notes = ?, thumb_file_id = ?, body_json = ?, \
         updated_at = ? WHERE id = ?",
    )
    .bind(f.name.trim())
    .bind(&f.category)
    .bind(&f.notes)
    .bind(f.thumb_file_id)
    .bind(body_json(f))
    .bind(now())
    .bind(id)
    .execute(pool)
    .await
    .map_err(db)?
    .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound(format!("template {id}")));
    }
    get_template(pool, id).await
}

pub async fn delete_template(pool: &SqlitePool, id: i64) -> Result<bool, AppError> {
    Ok(sqlx::query("DELETE FROM template WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(db)?
        .rows_affected()
        > 0)
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, FromRow)]
pub struct ProjectRow {
    pub id: i64,
    pub name: String,
    pub template_id: Option<i64>,
    pub template_name: Option<String>,
    pub sets: i64,
    pub due_date: Option<String>,
    /// `active` | `done` | `archived`.
    pub status: String,
    pub created_at: String,
    pub closed_at: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct SlotRow {
    pub id: i64,
    pub project_id: i64,
    pub key: String,
    pub label: Option<String>,
    pub hex: Option<String>,
    pub spool_id: Option<i64>,
    pub position: i64,
}

#[derive(Debug, Clone)]
pub struct PlateRow {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub file_id: i64,
    pub copies: i64,
    pub printed: i64,
    pub tools: Vec<ToolSlot>,
    pub position: i64,
}

fn plate_row(r: &sqlx::sqlite::SqliteRow) -> Result<PlateRow, AppError> {
    Ok(PlateRow {
        id: r.try_get("id").map_err(db)?,
        project_id: r.try_get("project_id").map_err(db)?,
        name: r.try_get("name").map_err(db)?,
        file_id: r.try_get("file_id").map_err(db)?,
        copies: r.try_get("copies").map_err(db)?,
        printed: r.try_get("printed").map_err(db)?,
        tools: serde_json::from_str(&r.try_get::<String, _>("tools_json").map_err(db)?)
            .unwrap_or_default(),
        position: r.try_get("position").map_err(db)?,
    })
}

/// A new slot: (key, label, hex, spool_id).
pub type NewSlot = (String, Option<String>, Option<String>, Option<i64>);

/// A new project: its row, slots and plates.
pub struct NewProject<'a> {
    pub name: &'a str,
    pub template_id: Option<i64>,
    pub template_name: Option<&'a str>,
    pub sets: u32,
    pub due_date: Option<&'a str>,
    /// (key, label, hex, spool_id)
    pub slots: Vec<NewSlot>,
    /// (name, file_id, copies, tools)
    pub plates: Vec<(String, i64, u32, Vec<ToolSlot>)>,
}

pub async fn create_project(pool: &SqlitePool, p: &NewProject<'_>) -> Result<i64, AppError> {
    let mut tx = pool.begin().await.map_err(db)?;
    let ts = now();
    let id = sqlx::query(
        "INSERT INTO project (name, template_id, template_name, sets, due_date, status, created_at) \
         VALUES (?, ?, ?, ?, ?, 'active', ?)",
    )
    .bind(p.name)
    .bind(p.template_id)
    .bind(p.template_name)
    .bind(p.sets as i64)
    .bind(p.due_date)
    .bind(&ts)
    .execute(&mut *tx)
    .await
    .map_err(db)?
    .last_insert_rowid();
    for (i, (key, label, hex, spool)) in p.slots.iter().enumerate() {
        sqlx::query(
            "INSERT INTO project_slot (project_id, key, label, hex, spool_id, position) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(key)
        .bind(label)
        .bind(hex)
        .bind(spool)
        .bind(i as i64)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    }
    for (i, (name, file_id, copies, tools)) in p.plates.iter().enumerate() {
        sqlx::query(
            "INSERT INTO project_plate (project_id, name, file_id, copies, printed, tools_json, position) \
             VALUES (?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(file_id)
        .bind(*copies as i64)
        .bind(serde_json::to_string(tools).unwrap_or_else(|_| "[]".into()))
        .bind(i as i64)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    }
    sqlx::query("INSERT INTO project_event (project_id, at, text) VALUES (?, ?, ?)")
        .bind(id)
        .bind(&ts)
        .bind(match p.template_name {
            Some(t) => format!("Project created from template {t}"),
            None => "Project created".to_owned(),
        })
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(id)
}

pub async fn list_projects(pool: &SqlitePool) -> Result<Vec<ProjectRow>, AppError> {
    sqlx::query_as(
        "SELECT id, name, template_id, template_name, sets, due_date, status, created_at, closed_at \
         FROM project ORDER BY (due_date IS NULL), due_date, id",
    )
    .fetch_all(pool)
    .await
    .map_err(db)
}

pub async fn get_project(pool: &SqlitePool, id: i64) -> Result<ProjectRow, AppError> {
    sqlx::query_as(
        "SELECT id, name, template_id, template_name, sets, due_date, status, created_at, closed_at \
         FROM project WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(db)?
    .ok_or_else(|| AppError::NotFound(format!("project {id}")))
}

pub async fn slots(pool: &SqlitePool, project_id: i64) -> Result<Vec<SlotRow>, AppError> {
    sqlx::query_as(
        "SELECT id, project_id, key, label, hex, spool_id, position FROM project_slot \
         WHERE project_id = ? ORDER BY position, id",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(db)
}

pub async fn plates(pool: &SqlitePool, project_id: i64) -> Result<Vec<PlateRow>, AppError> {
    sqlx::query("SELECT * FROM project_plate WHERE project_id = ? ORDER BY position, id")
        .bind(project_id)
        .fetch_all(pool)
        .await
        .map_err(db)?
        .iter()
        .map(plate_row)
        .collect()
}

pub async fn plate(pool: &SqlitePool, plate_id: i64) -> Result<PlateRow, AppError> {
    let r = sqlx::query("SELECT * FROM project_plate WHERE id = ?")
        .bind(plate_id)
        .fetch_optional(pool)
        .await
        .map_err(db)?
        .ok_or_else(|| AppError::NotFound(format!("project plate {plate_id}")))?;
    plate_row(&r)
}

/// Live queue statuses of every item for a plate (done / cancelled excluded).
pub async fn plate_items(
    pool: &SqlitePool,
    plate_id: i64,
) -> Result<Vec<(i64, QueueStatus, Option<i64>)>, AppError> {
    let rows = sqlx::query(
        "SELECT id, status, printer_id FROM queue_item WHERE project_plate_id = ? \
         AND status IN ('waiting', 'sending', 'printing', 'failed') ORDER BY id",
    )
    .bind(plate_id)
    .fetch_all(pool)
    .await
    .map_err(db)?;
    rows.iter()
        .map(|r| {
            Ok((
                r.try_get("id").map_err(db)?,
                QueueStatus::parse(&r.try_get::<String, _>("status").map_err(db)?),
                r.try_get("printer_id").map_err(db)?,
            ))
        })
        .collect()
}

pub async fn set_copies(pool: &SqlitePool, plate_id: i64, copies: u32) -> Result<(), AppError> {
    sqlx::query("UPDATE project_plate SET copies = ? WHERE id = ?")
        .bind(copies as i64)
        .bind(plate_id)
        .execute(pool)
        .await
        .map_err(db)?;
    Ok(())
}

/// One more finished print, never past `copies`. Returns whether it counted.
pub async fn add_printed(pool: &SqlitePool, plate_id: i64) -> Result<bool, AppError> {
    Ok(sqlx::query(
        "UPDATE project_plate SET printed = printed + 1 WHERE id = ? AND printed < copies",
    )
    .bind(plate_id)
    .execute(pool)
    .await
    .map_err(db)?
    .rows_affected()
        > 0)
}

pub async fn set_slot_spool(
    pool: &SqlitePool,
    project_id: i64,
    key: &str,
    spool_id: Option<i64>,
) -> Result<bool, AppError> {
    Ok(
        sqlx::query("UPDATE project_slot SET spool_id = ? WHERE project_id = ? AND key = ?")
            .bind(spool_id)
            .bind(project_id)
            .bind(key)
            .execute(pool)
            .await
            .map_err(db)?
            .rows_affected()
            > 0,
    )
}

pub async fn set_status(pool: &SqlitePool, id: i64, status: &str) -> Result<(), AppError> {
    let closed = (status != "active").then(now);
    sqlx::query("UPDATE project SET status = ?, closed_at = ? WHERE id = ?")
        .bind(status)
        .bind(closed)
        .bind(id)
        .execute(pool)
        .await
        .map_err(db)?;
    Ok(())
}

pub async fn update_project(
    pool: &SqlitePool,
    id: i64,
    name: Option<&str>,
    due_date: Option<Option<&str>>,
) -> Result<(), AppError> {
    if let Some(n) = name {
        sqlx::query("UPDATE project SET name = ? WHERE id = ?")
            .bind(n)
            .bind(id)
            .execute(pool)
            .await
            .map_err(db)?;
    }
    if let Some(d) = due_date {
        sqlx::query("UPDATE project SET due_date = ? WHERE id = ?")
            .bind(d)
            .bind(id)
            .execute(pool)
            .await
            .map_err(db)?;
    }
    Ok(())
}

pub async fn add_event(pool: &SqlitePool, project_id: i64, text: &str) -> Result<(), AppError> {
    sqlx::query("INSERT INTO project_event (project_id, at, text) VALUES (?, ?, ?)")
        .bind(project_id)
        .bind(now())
        .bind(text)
        .execute(pool)
        .await
        .map_err(db)?;
    Ok(())
}

pub async fn events(pool: &SqlitePool, project_id: i64) -> Result<Vec<(String, String)>, AppError> {
    let rows = sqlx::query(
        "SELECT at, text FROM project_event WHERE project_id = ? ORDER BY id DESC LIMIT 50",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(db)?;
    rows.iter()
        .map(|r| Ok((r.try_get("at").map_err(db)?, r.try_get("text").map_err(db)?)))
        .collect()
}

/// Mark a queue item as a print of a project plate.
pub async fn link_queue_item(
    pool: &SqlitePool,
    item_id: i64,
    plate_id: i64,
) -> Result<(), AppError> {
    sqlx::query("UPDATE queue_item SET project_plate_id = ? WHERE id = ?")
        .bind(plate_id)
        .bind(item_id)
        .execute(pool)
        .await
        .map_err(db)?;
    Ok(())
}

/// Mark a job as a print of a project plate (for time / grams per project).
pub async fn link_job(pool: &SqlitePool, job_id: i64, plate_id: i64) -> Result<(), AppError> {
    sqlx::query("UPDATE job SET project_plate_id = ? WHERE id = ?")
        .bind(plate_id)
        .bind(job_id)
        .execute(pool)
        .await
        .map_err(db)?;
    Ok(())
}

/// Per-slot use of a project's finished jobs (one `Vec<JobTool>` per job).
pub async fn job_tools(
    pool: &SqlitePool,
    project_id: i64,
) -> Result<Vec<Vec<crate::domain::JobTool>>, AppError> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT tools_json FROM job WHERE end_state != 'running' AND project_plate_id IN \
         (SELECT id FROM project_plate WHERE project_id = ?)",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(db)?;
    Ok(rows
        .iter()
        .map(|j| serde_json::from_str(j).unwrap_or_default())
        .collect())
}

/// Finished jobs of a project: (seconds, grams used).
pub async fn job_totals(pool: &SqlitePool, project_id: i64) -> Result<(i64, f64), AppError> {
    let r = sqlx::query(
        "SELECT COALESCE(SUM(duration_secs), 0) AS secs, COALESCE(SUM(grams_used), 0.0) AS grams \
         FROM job WHERE end_state != 'running' AND project_plate_id IN \
         (SELECT id FROM project_plate WHERE project_id = ?)",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await
    .map_err(db)?;
    Ok((
        r.try_get("secs").map_err(db)?,
        r.try_get("grams").map_err(db)?,
    ))
}
