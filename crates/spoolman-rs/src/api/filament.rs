//! Filament REST + WebSocket endpoints — mirrors `spoolman/api/v1/filament.py`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};

use crate::api::ws::OptWs;
use serde::Deserialize;

use crate::{
    api::error::ApiResult,
    db::{filament as db, tag as db_tag},
    error::SpoolmanError,
    models::{
        CreateFilamentRequest, FilamentResponse, TagInfo, TagLinkRequest, UpdateFilamentRequest,
    },
    state::AppState,
};

// ---------------------------------------------------------------------------
// Query params (upstream FilamentParameters)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub struct FilamentQuery {
    pub name: Option<String>,
    #[serde(rename = "vendor_id", alias = "vendor.id")]
    pub vendor_id: Option<String>,
    #[serde(rename = "vendor.name", alias = "vendor_name")]
    pub vendor_name: Option<String>,
    pub material: Option<String>,
    pub article_number: Option<String>,
    pub external_id: Option<String>,
    pub color_hex: Option<String>,
    pub color_similarity_threshold: Option<f64>,
    /// NFC/RFID tag UID — returns the single filament linked to this tag (or empty).
    pub tag: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET {base_path}/api/v1/filament  (also accepts WS upgrade → all filament events)
pub async fn list(
    OptWs(ws): OptWs,
    State(state): State<AppState>,
    uri: Uri,
    Query(q): Query<FilamentQuery>,
) -> Response {
    if let Some(ws) = ws {
        let rx = state.event_tx.subscribe();
        return ws.on_upgrade(move |socket| {
            crate::api::ws::handle_entity_ws(socket, rx, "filament", None)
        });
    }
    list_inner(state, uri, q).await.into_response()
}

async fn list_inner(
    state: AppState,
    uri: Uri,
    q: FilamentQuery,
) -> ApiResult<(StatusCode, HeaderMap, Json<Vec<FilamentResponse>>)> {
    let extra_filters = crate::api::parse_extra_params(&uri);

    // Tag UID filter: find the single filament linked to this tag.
    if let Some(ref tag_uid) = q.tag {
        let fid = crate::db::tag::find_filament_id_by_uid(&state.db.pool, tag_uid).await?;
        // Restrict to exactly that filament ID (or nothing if unlinked).
        let ids = fid.map(|id| vec![id]).unwrap_or_default();
        let filter = db::FilamentFilter {
            ids: Some(ids),
            sort: q.sort,
            limit: q.limit,
            offset: q.offset,
            ..Default::default()
        };
        let (items, total) = db::find(&state.db.pool, &state.db.engine, &filter).await?;
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-total-count",
            HeaderValue::from_str(&total.to_string()).unwrap_or(HeaderValue::from_static("0")),
        );
        return Ok((StatusCode::OK, headers, Json(items)));
    }

    // Colour-similarity pre-filter: resolve matching IDs before the main query.
    let ids = if let Some(ref hex) = q.color_hex {
        let threshold = q.color_similarity_threshold.unwrap_or(20.0);
        Some(db::find_by_color(&state.db.pool, hex, threshold).await?)
    } else {
        None
    };

    let filter = db::FilamentFilter {
        name: q.name,
        vendor_id: q.vendor_id,
        vendor_name: q.vendor_name,
        material: q.material,
        article_number: q.article_number,
        external_id: q.external_id,
        ids,
        sort: q.sort,
        limit: q.limit,
        offset: q.offset,
        extra_filters,
    };
    let (items, total) = db::find(&state.db.pool, &state.db.engine, &filter).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-total-count",
        HeaderValue::from_str(&total.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );
    Ok((StatusCode::OK, headers, Json(items)))
}

/// GET {base_path}/api/v1/filament/{id}  (also accepts WS upgrade → events for this filament)
pub async fn get_one(
    OptWs(ws): OptWs,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    if let Some(ws) = ws {
        let rx = state.event_tx.subscribe();
        return ws.on_upgrade(move |socket| {
            crate::api::ws::handle_entity_ws(socket, rx, "filament", Some(id))
        });
    }
    get_one_inner(state, id).await.into_response()
}

async fn get_one_inner(state: AppState, id: i64) -> ApiResult<Json<FilamentResponse>> {
    let filament = db::find_by_id(&state.db.pool, id).await?;
    Ok(Json(filament))
}

/// POST {base_path}/api/v1/filament
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateFilamentRequest>,
) -> ApiResult<(StatusCode, Json<FilamentResponse>)> {
    let filament = db::create(&state.db.pool, &state.db.engine, &req).await?;
    crate::ws::broadcast(&state.event_tx, "added", "filament", filament.id, &filament);
    Ok((StatusCode::CREATED, Json(filament)))
}

/// PATCH {base_path}/api/v1/filament/{id}
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateFilamentRequest>,
) -> ApiResult<Json<FilamentResponse>> {
    let filament = db::update(&state.db.pool, id, &req).await?;
    crate::ws::broadcast(
        &state.event_tx,
        "updated",
        "filament",
        filament.id,
        &filament,
    );
    Ok(Json(filament))
}

/// DELETE {base_path}/api/v1/filament/{id}
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let filament = db::find_by_id(&state.db.pool, id).await?;
    db::delete(&state.db.pool, id).await?;
    crate::ws::broadcast(
        &state.event_tx,
        "deleted",
        "filament",
        filament.id,
        &filament,
    );
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Tag link / unlink
// ---------------------------------------------------------------------------

/// POST {base_path}/api/v1/filament/{id}/tag
pub async fn link_tag(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<TagLinkRequest>,
) -> ApiResult<(StatusCode, Json<TagInfo>)> {
    let tag = db_tag::link_filament(&state.db.pool, id, &req.uid, req.format.as_deref()).await?;
    Ok((StatusCode::CREATED, Json(tag)))
}

/// DELETE {base_path}/api/v1/filament/{id}/tag/{uid}
pub async fn unlink_tag(
    State(state): State<AppState>,
    Path((id, uid)): Path<(i64, String)>,
) -> ApiResult<StatusCode> {
    // Validate filament exists first (returns 404 if not).
    if db::find_by_id(&state.db.pool, id).await.is_err() {
        return Err(SpoolmanError::NotFound(format!("filament with id {id}")).into());
    }
    db_tag::unlink_filament(&state.db.pool, id, &uid).await?;
    Ok(StatusCode::NO_CONTENT)
}
