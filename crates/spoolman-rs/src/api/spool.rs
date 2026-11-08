//! Spool REST + WebSocket endpoints — mirrors `spoolman/api/v1/spool.py`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::api::ws::OptWs;
use serde::Deserialize;

use crate::{
    api::error::ApiResult,
    db::spool as db,
    error::SpoolmanError,
    models::{
        CreateSpoolRequest, RenameFieldValueRequest, RenameFieldValueResult, SpoolGroupResponse,
        SpoolMeasureRequest, SpoolResponse, SpoolUseRequest, UpdateSpoolRequest,
    },
    state::AppState,
};

// ---------------------------------------------------------------------------
// Query params (upstream SpoolParameters — dot-notation flattened)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub struct SpoolQuery {
    #[serde(rename = "filament.name")]
    pub filament_name: Option<String>,
    #[serde(rename = "filament.id")]
    pub filament_id: Option<String>,
    #[serde(rename = "filament.material")]
    pub filament_material: Option<String>,
    #[serde(rename = "filament.vendor.name")]
    pub filament_vendor_name: Option<String>,
    #[serde(rename = "filament.vendor.id")]
    pub filament_vendor_id: Option<String>,
    pub location: Option<String>,
    pub lot_nr: Option<String>,
    pub allow_archived: Option<bool>,
    pub first_used: Option<String>,
    pub last_used: Option<String>,
    pub registered: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET {base_path}/api/v1/spool  (also accepts WS upgrade → all spool events)
pub async fn list(
    OptWs(ws): OptWs,
    State(state): State<AppState>,
    Query(q): Query<SpoolQuery>,
) -> Response {
    if let Some(ws) = ws {
        let rx = state.event_tx.subscribe();
        return ws
            .on_upgrade(move |socket| crate::api::ws::handle_entity_ws(socket, rx, "spool", None));
    }
    list_inner(state, q).await.into_response()
}

async fn list_inner(
    state: AppState,
    q: SpoolQuery,
) -> ApiResult<(StatusCode, HeaderMap, Json<Vec<SpoolResponse>>)> {
    let filter = db::SpoolFilter {
        filament_name: q.filament_name,
        filament_id: q.filament_id,
        filament_material: q.filament_material,
        filament_vendor_name: q.filament_vendor_name,
        filament_vendor_id: q.filament_vendor_id,
        location: q.location,
        lot_nr: q.lot_nr,
        allow_archived: q.allow_archived.unwrap_or(false),
        first_used: q.first_used,
        last_used: q.last_used,
        registered: q.registered,
        sort: q.sort,
        limit: q.limit,
        offset: q.offset,
    };
    let (items, total) = db::find(&state.db.pool, &state.db.engine, &filter).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-total-count",
        HeaderValue::from_str(&total.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );
    Ok((StatusCode::OK, headers, Json(items)))
}

/// GET {base_path}/api/v1/spool/{id}  (also accepts WS upgrade → events for this spool)
pub async fn get_one(
    OptWs(ws): OptWs,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    if let Some(ws) = ws {
        let rx = state.event_tx.subscribe();
        return ws.on_upgrade(move |socket| {
            crate::api::ws::handle_entity_ws(socket, rx, "spool", Some(id))
        });
    }
    get_one_inner(state, id).await.into_response()
}

async fn get_one_inner(state: AppState, id: i64) -> ApiResult<Json<SpoolResponse>> {
    let spool = db::find_by_id(&state.db.pool, id).await?;
    Ok(Json(spool))
}

/// POST {base_path}/api/v1/spool
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateSpoolRequest>,
) -> ApiResult<(StatusCode, Json<SpoolResponse>)> {
    let spool = db::create(&state.db.pool, &state.db.engine, &req).await?;
    crate::ws::broadcast(&state.event_tx, "added", "spool", spool.id, &spool);
    Ok((StatusCode::CREATED, Json(spool)))
}

/// PATCH {base_path}/api/v1/spool/{id}
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateSpoolRequest>,
) -> ApiResult<Json<SpoolResponse>> {
    let spool = db::update(&state.db.pool, id, &req).await?;
    crate::ws::broadcast(&state.event_tx, "updated", "spool", spool.id, &spool);
    Ok(Json(spool))
}

/// DELETE {base_path}/api/v1/spool/{id}
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let spool = db::find_by_id(&state.db.pool, id).await?;
    db::delete(&state.db.pool, id).await?;
    crate::ws::broadcast(&state.event_tx, "deleted", "spool", spool.id, &spool);
    Ok(StatusCode::NO_CONTENT)
}

/// PUT {base_path}/api/v1/spool/{id}/use
pub async fn use_spool(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<SpoolUseRequest>,
) -> ApiResult<Json<SpoolResponse>> {
    if req.use_weight.is_some() && req.use_length.is_some() {
        return Err(SpoolmanError::BadRequest(
            "Only specify either use_weight or use_length.".into(),
        )
        .into());
    }
    let spool = if let Some(w) = req.use_weight {
        db::use_weight(&state.db.pool, id, w).await?
    } else if let Some(l) = req.use_length {
        db::use_length(&state.db.pool, id, l).await?
    } else {
        return Err(SpoolmanError::BadRequest(
            "Either use_weight or use_length must be specified.".into(),
        )
        .into());
    };
    crate::ws::broadcast(&state.event_tx, "updated", "spool", spool.id, &spool);
    Ok(Json(spool))
}

/// PUT {base_path}/api/v1/spool/{id}/measure
pub async fn measure_spool(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<SpoolMeasureRequest>,
) -> ApiResult<Json<SpoolResponse>> {
    let spool = db::measure(&state.db.pool, id, req.weight).await?;
    crate::ws::broadcast(&state.event_tx, "updated", "spool", spool.id, &spool);
    Ok(Json(spool))
}

// ---------------------------------------------------------------------------
// Group aggregation
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub struct SpoolGroupQuery {
    pub group_by: Option<String>,
    #[serde(rename = "filament.name")]
    pub filament_name: Option<String>,
    #[serde(rename = "filament.id")]
    pub filament_id: Option<String>,
    #[serde(rename = "filament.material")]
    pub filament_material: Option<String>,
    #[serde(rename = "filament.vendor.name")]
    pub filament_vendor_name: Option<String>,
    #[serde(rename = "filament.vendor.id")]
    pub filament_vendor_id: Option<String>,
    pub location: Option<String>,
    pub allow_archived: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// GET {base_path}/api/v1/spool/group
pub async fn find_groups(
    State(state): State<AppState>,
    Query(q): Query<SpoolGroupQuery>,
) -> ApiResult<(StatusCode, HeaderMap, Json<Vec<SpoolGroupResponse>>)> {
    let group_by = q.group_by.unwrap_or_default();
    if group_by.is_empty() {
        return Err(
            SpoolmanError::BadRequest("group_by query parameter is required.".into()).into(),
        );
    }

    let filter = db::SpoolGroupFilter {
        group_by,
        filament_name: q.filament_name,
        filament_id: q.filament_id,
        filament_material: q.filament_material,
        filament_vendor_name: q.filament_vendor_name,
        filament_vendor_id: q.filament_vendor_id,
        location: q.location,
        allow_archived: q.allow_archived.unwrap_or(false),
        limit: q.limit,
        offset: q.offset,
    };

    let (groups, total_count, total_spools, total_remaining_weight) =
        db::find_groups(&state.db.pool, &state.db.engine, &filter).await?;

    let mut headers = HeaderMap::new();
    headers.insert(
        "x-total-count",
        HeaderValue::from_str(&total_count.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );
    headers.insert(
        "x-total-spools",
        HeaderValue::from_str(&total_spools.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );
    headers.insert(
        "x-total-remaining-weight",
        HeaderValue::from_str(&format!("{total_remaining_weight:.2}"))
            .unwrap_or(HeaderValue::from_static("0")),
    );
    Ok((StatusCode::OK, headers, Json(groups)))
}

// ---------------------------------------------------------------------------
// Rename field value
// ---------------------------------------------------------------------------

/// PATCH {base_path}/api/v1/spool/field/{field}
pub async fn rename_field_value(
    State(state): State<AppState>,
    Path(field): Path<String>,
    Json(req): Json<RenameFieldValueRequest>,
) -> ApiResult<Json<RenameFieldValueResult>> {
    let count = db::rename_field_value(&state.db.pool, &field, &req.value, &req.new_value).await?;
    Ok(Json(RenameFieldValueResult { count }))
}
