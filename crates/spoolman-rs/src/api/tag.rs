//! Tag link/unlink + scan relay endpoints — mirrors `spoolman/api/v1/tag.py`.

use axum::{
    extract::{Path, State, WebSocketUpgrade},
    http::StatusCode,
    response::Response,
    Json,
};

use crate::{
    api::error::ApiResult,
    db::tag as db,
    models::{ScanTagRequest, ScanTagResponse, TagInfo, TagLinkRequest, TagReaderInfo},
    state::AppState,
    ws::ScanBroadcastEvent,
};

// ---------------------------------------------------------------------------
// Tag link / unlink
// ---------------------------------------------------------------------------

/// POST {base_path}/api/v1/spool/{id}/tag — link a physical NFC/RFID tag to a spool.
pub async fn link_tag(
    State(state): State<AppState>,
    Path(spool_id): Path<i64>,
    Json(req): Json<TagLinkRequest>,
) -> ApiResult<(StatusCode, Json<TagInfo>)> {
    let tag = db::link_spool(&state.db.pool, spool_id, &req.uid, req.format.as_deref()).await?;
    Ok((StatusCode::CREATED, Json(tag)))
}

/// DELETE {base_path}/api/v1/spool/{id}/tag/{uid} — unlink a tag from a spool.
pub async fn unlink_tag(
    State(state): State<AppState>,
    Path((spool_id, uid)): Path<(i64, String)>,
) -> ApiResult<StatusCode> {
    db::unlink_spool(&state.db.pool, spool_id, &uid).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Scan relay — REST
// ---------------------------------------------------------------------------

/// POST {base_path}/api/v1/tag/scan — relay a tag scan event.
///
/// Looks up the UID, registers with the dedup relay, and broadcasts if
/// the dedup window allows.
pub async fn scan_post(
    State(state): State<AppState>,
    Json(req): Json<ScanTagRequest>,
) -> ApiResult<Json<ScanTagResponse>> {
    let uid_norm = db::normalize_uid(&req.uid);
    let reader_id = req.reader_id.as_deref().unwrap_or("unknown").to_owned();

    let matched_spool_id = db::find_spool_id_by_uid(&state.db.pool, &uid_norm).await?;
    let matched_filament_id = db::find_filament_id_by_uid(&state.db.pool, &uid_norm).await?;

    let matched = matched_spool_id
        .map(|id| ("spool", id))
        .or_else(|| matched_filament_id.map(|id| ("filament", id)));

    let response = ScanTagResponse {
        uid: uid_norm.clone(),
        reader_id: reader_id.clone(),
        name: req.name.clone(),
        format: req.format.clone(),
        matched_spool_id,
        matched_filament_id,
        created: false,
    };

    if state
        .scan_relay
        .register(&reader_id, req.name.as_deref(), &uid_norm, matched)
    {
        let json = serde_json::to_string(&response).unwrap_or_else(|_| "{}".to_owned());
        let _ = state.scan_tx.send(ScanBroadcastEvent { reader_id, json });
    }

    Ok(Json(response))
}

/// GET {base_path}/api/v1/tag/reader — list recently seen tag readers.
pub async fn list_readers(State(state): State<AppState>) -> ApiResult<Json<Vec<TagReaderInfo>>> {
    let readers = state
        .scan_relay
        .readers()
        .into_iter()
        .map(|r| TagReaderInfo {
            reader_id: r.reader_id,
            name: r.name,
            last_seen: r.last_seen,
            last_uid: r.last_uid,
        })
        .collect();
    Ok(Json(readers))
}

// ---------------------------------------------------------------------------
// Scan relay — WebSocket
// ---------------------------------------------------------------------------

/// GET/WS {base_path}/api/v1/tag/scan — subscribe to all scan events.
pub async fn scan_ws_any(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    let rx = state.scan_tx.subscribe();
    ws.on_upgrade(move |socket| crate::api::ws::handle_scan_ws(socket, rx, None))
}

/// GET/WS {base_path}/api/v1/tag/scan/{reader_id} — subscribe to scan events from one reader.
pub async fn scan_ws_reader(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(reader_id): Path<String>,
) -> Response {
    let rx = state.scan_tx.subscribe();
    ws.on_upgrade(move |socket| crate::api::ws::handle_scan_ws(socket, rx, Some(reader_id)))
}
