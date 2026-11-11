//! Inventory, shopping list, locations (task 4.3).
//!
//! - `GET    /api/inventory`                — tiles, tracked filaments, reservations
//! - `PUT    /api/inventory/{filament_id}`  — track / set `{target, buy_below}`
//! - `DELETE /api/inventory/{filament_id}`  — stop tracking
//! - `GET    /api/spool-locations`                — locations with spool counts
//! - `PATCH  /api/spool-locations/{name}`         — `{name}`: rename (moves every spool)

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use foreman_core::repo;
use serde_json::{json, Value};

use super::spools::{err, sm, ApiErr};
use crate::{
    inventory::{self, Inventory, Locations, RenameLocation, TrackFilament},
    state::AppState,
};

pub async fn get(State(state): State<AppState>) -> Result<Json<Inventory>, ApiErr> {
    inventory::inventory(
        &state.foreman_db.pool,
        &state.driver_manager,
        sm(&state)?,
        state.library.as_ref(),
    )
    .await
    .map(Json)
    .map_err(err)
}

pub async fn track(
    State(state): State<AppState>,
    Path(filament_id): Path<i64>,
    Json(req): Json<TrackFilament>,
) -> Result<Json<Inventory>, ApiErr> {
    inventory::track(&state.foreman_db.pool, sm(&state)?, filament_id, &req)
        .await
        .map_err(err)?;
    get(State(state)).await
}

pub async fn untrack(
    State(state): State<AppState>,
    Path(filament_id): Path<i64>,
) -> Result<Json<Inventory>, ApiErr> {
    repo::tracked_filament::remove(&state.foreman_db.pool, filament_id)
        .await
        .map_err(err)?;
    get(State(state)).await
}

pub async fn locations(State(state): State<AppState>) -> Result<Json<Locations>, ApiErr> {
    inventory::locations(sm(&state)?)
        .await
        .map(Json)
        .map_err(err)
}

pub async fn rename_location(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<RenameLocation>,
) -> Result<(StatusCode, Json<Value>), ApiErr> {
    let moved = inventory::rename_location(sm(&state)?, &name, &req.name)
        .await
        .map_err(err)?;
    Ok((
        StatusCode::OK,
        Json(json!({ "name": req.name.trim(), "spools_moved": moved })),
    ))
}
