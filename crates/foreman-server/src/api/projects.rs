//! Templates and projects (task 5.1, W9).
//!
//! - `GET/POST /api/templates` · `GET/PUT/DELETE /api/templates/{id}`
//! - `GET /api/templates/{id}/plan?sets=N` — grams per slot + spool options
//! - `GET/POST /api/projects` · `GET/PATCH /api/projects/{id}`
//! - `POST /api/projects/{id}/print-remaining` — outstanding prints → fleet queue
//! - `PUT /api/projects/{id}/slots/{key}` — `{spool_id}`
//! - `PATCH /api/project-plates/{id}` — `{copies}` · `POST /api/project-plates/{id}/done`

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use foreman_core::{projects::TemplateForm, repo};
use serde::Deserialize;
use serde_json::{json, Value};

use super::spools::{err, ApiErr};
use crate::{
    auth::AuthCtx,
    projects::{
        self, CreateProject, ProjectPlan, ProjectSummary, ProjectView, TemplateView, UpdateProject,
    },
    state::AppState,
};

pub async fn list_templates(State(s): State<AppState>) -> Result<Json<Vec<TemplateView>>, ApiErr> {
    projects::templates(&s).await.map(Json).map_err(err)
}

pub async fn get_template(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<TemplateView>, ApiErr> {
    projects::template(&s, id).await.map(Json).map_err(err)
}

pub async fn create_template(
    State(s): State<AppState>,
    Json(f): Json<TemplateForm>,
) -> Result<(StatusCode, Json<TemplateView>), ApiErr> {
    projects::save_template(&s, None, &f)
        .await
        .map(|v| (StatusCode::CREATED, Json(v)))
        .map_err(err)
}

pub async fn update_template(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(f): Json<TemplateForm>,
) -> Result<Json<TemplateView>, ApiErr> {
    projects::save_template(&s, Some(id), &f)
        .await
        .map(Json)
        .map_err(err)
}

pub async fn delete_template(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiErr> {
    match repo::project::delete_template(&s.foreman_db.pool, id).await {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err(err(foreman_core::error::AppError::NotFound(format!(
            "template {id}"
        )))),
        Err(e) => Err(err(e)),
    }
}

#[derive(Deserialize)]
pub struct PlanQuery {
    #[serde(default)]
    sets: Option<u32>,
}

pub async fn plan(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<PlanQuery>,
) -> Result<Json<ProjectPlan>, ApiErr> {
    projects::plan(&s, id, q.sets.unwrap_or(1))
        .await
        .map(Json)
        .map_err(err)
}

pub async fn list(State(s): State<AppState>) -> Result<Json<Vec<ProjectSummary>>, ApiErr> {
    projects::list(&s).await.map(Json).map_err(err)
}

pub async fn get(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<ProjectView>, ApiErr> {
    projects::project(&s, id).await.map(Json).map_err(err)
}

pub async fn create(
    State(s): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Json(req): Json<CreateProject>,
) -> Result<(StatusCode, Json<ProjectView>), ApiErr> {
    projects::create_project(&s, &req, &ctx.username)
        .await
        .map(|v| (StatusCode::CREATED, Json(v)))
        .map_err(err)
}

pub async fn update(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateProject>,
) -> Result<Json<ProjectView>, ApiErr> {
    projects::update(&s, id, &req).await.map_err(err)?;
    get(State(s), Path(id)).await
}

pub async fn print_remaining(
    State(s): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, ApiErr> {
    let added = projects::print_all_remaining(&s, id, &ctx.username)
        .await
        .map_err(err)?;
    Ok(Json(json!({ "added": added })))
}

#[derive(Deserialize)]
pub struct SlotBody {
    spool_id: Option<i64>,
}

pub async fn set_slot(
    State(s): State<AppState>,
    Path((id, key)): Path<(i64, String)>,
    Json(b): Json<SlotBody>,
) -> Result<Json<ProjectView>, ApiErr> {
    projects::set_slot_spool(&s, id, &key, b.spool_id)
        .await
        .map_err(err)?;
    get(State(s), Path(id)).await
}

#[derive(Deserialize)]
pub struct CopiesBody {
    copies: u32,
}

pub async fn set_copies(
    State(s): State<AppState>,
    Path(plate_id): Path<i64>,
    Json(b): Json<CopiesBody>,
) -> Result<Json<ProjectView>, ApiErr> {
    projects::set_copies(&s, plate_id, b.copies)
        .await
        .map_err(err)?;
    let p = repo::project::plate(&s.foreman_db.pool, plate_id)
        .await
        .map_err(err)?;
    get(State(s), Path(p.project_id)).await
}

pub async fn mark_done(
    State(s): State<AppState>,
    Path(plate_id): Path<i64>,
) -> Result<Json<ProjectView>, ApiErr> {
    projects::mark_done(&s, plate_id).await.map_err(err)?;
    let p = repo::project::plate(&s.foreman_db.pool, plate_id)
        .await
        .map_err(err)?;
    get(State(s), Path(p.project_id)).await
}
