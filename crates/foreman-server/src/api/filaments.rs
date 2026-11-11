//! Vendors, filaments, new spools (task 4.2).
//!
//! - `GET  /api/filament/catalogue`  — base materials (+ defaults) and modifier groups
//! - `GET  /api/vendors` · `POST /api/vendors` · `PATCH /api/vendors/{id}`
//! - `GET  /api/filaments` · `POST /api/filaments` · `GET|PATCH /api/filaments/{id}`
//! - `POST /api/spools`              — N spools of a filament (or of a new one)
//! - `POST /api/spools/{id}/clone`   — `{quantity}` copies of a spool
//! - `GET  /api/spools/locations`    — locations in use

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use foreman_core::{
    filament::SpoolFacts,
    materials::{self, MaterialCatalogue},
};

use super::spools::{err, sm, ApiErr};
use crate::{
    filaments::{
        self, CloneSpool, CreateSpools, CreatedSpools, FilamentForm, FilamentRow, VendorForm,
        VendorRow,
    },
    state::AppState,
};

pub async fn catalogue() -> Json<MaterialCatalogue> {
    Json(materials::catalogue())
}

pub async fn list_vendors(State(state): State<AppState>) -> Result<Json<Vec<VendorRow>>, ApiErr> {
    filaments::vendors(sm(&state)?).await.map(Json).map_err(err)
}

pub async fn create_vendor(
    State(state): State<AppState>,
    Json(form): Json<VendorForm>,
) -> Result<(StatusCode, Json<VendorRow>), ApiErr> {
    filaments::create_vendor(sm(&state)?, &form)
        .await
        .map(|v| (StatusCode::CREATED, Json(v)))
        .map_err(err)
}

pub async fn update_vendor(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(form): Json<VendorForm>,
) -> Result<Json<VendorRow>, ApiErr> {
    filaments::update_vendor(sm(&state)?, id, &form)
        .await
        .map(Json)
        .map_err(err)
}

pub async fn list_filaments(
    State(state): State<AppState>,
) -> Result<Json<Vec<FilamentRow>>, ApiErr> {
    filaments::filaments(sm(&state)?)
        .await
        .map(Json)
        .map_err(err)
}

pub async fn get_filament(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<FilamentRow>, ApiErr> {
    filaments::filament(sm(&state)?, id)
        .await
        .map(Json)
        .map_err(err)
}

pub async fn create_filament(
    State(state): State<AppState>,
    Json(form): Json<FilamentForm>,
) -> Result<(StatusCode, Json<FilamentRow>), ApiErr> {
    filaments::create_filament(sm(&state)?, &form)
        .await
        .map(|f| (StatusCode::CREATED, Json(f)))
        .map_err(err)
}

pub async fn update_filament(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(form): Json<FilamentForm>,
) -> Result<Json<FilamentRow>, ApiErr> {
    filaments::update_filament(sm(&state)?, id, &form)
        .await
        .map(Json)
        .map_err(err)
}

pub async fn create_spools(
    State(state): State<AppState>,
    Json(req): Json<CreateSpools>,
) -> Result<(StatusCode, Json<CreatedSpools>), ApiErr> {
    filaments::create_spools(sm(&state)?, &req)
        .await
        .map(|c| (StatusCode::CREATED, Json(c)))
        .map_err(err)
}

pub async fn clone_spool(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<CloneSpool>,
) -> Result<(StatusCode, Json<Vec<SpoolFacts>>), ApiErr> {
    filaments::clone_spool(sm(&state)?, id, &req)
        .await
        .map(|c| (StatusCode::CREATED, Json(c)))
        .map_err(err)
}

pub async fn locations(State(state): State<AppState>) -> Result<Json<Vec<String>>, ApiErr> {
    filaments::locations(sm(&state)?)
        .await
        .map(Json)
        .map_err(err)
}
