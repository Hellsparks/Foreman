//! Tag link/unlink + scan relay endpoints — mirrors `spoolman/api/v1/tag.py`.

use axum::{
    extract::{ConnectInfo, Path, State, WebSocketUpgrade},
    http::StatusCode,
    response::Response,
    Json,
};
use std::net::SocketAddr;

use crate::{
    api::error::ApiResult,
    db::{filament as filament_db, tag as db},
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
    // Broadcast spool "updated" so open spool WebSocket subscribers see the new tag.
    if let Ok(spool) = crate::db::spool::find_by_id(&state.db.pool, spool_id).await {
        crate::ws::broadcast(&state.event_tx, "updated", "spool", spool_id, &spool);
    }
    Ok((StatusCode::CREATED, Json(tag)))
}

/// DELETE {base_path}/api/v1/spool/{id}/tag/{uid} — unlink a tag from a spool.
pub async fn unlink_tag(
    State(state): State<AppState>,
    Path((spool_id, uid)): Path<(i64, String)>,
) -> ApiResult<StatusCode> {
    db::unlink_spool(&state.db.pool, spool_id, &uid).await?;
    // Broadcast spool "updated" so open spool WebSocket subscribers see the empty tag list.
    if let Ok(spool) = crate::db::spool::find_by_id(&state.db.pool, spool_id).await {
        crate::ws::broadcast(&state.event_tx, "updated", "spool", spool_id, &spool);
    }
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    Json(req): Json<ScanTagRequest>,
) -> ApiResult<Json<ScanTagResponse>> {
    let uid_norm = db::normalize_uid(&req.uid)?;

    // When no reader_id is supplied, derive one from the client's IP address.
    let reader_id = match req.reader_id.as_deref() {
        Some(rid) => {
            // reader_id is used as a WebSocket path segment — reject chars that would break routing.
            if rid.contains(' ') || rid.contains('/') {
                return Err(crate::error::SpoolmanError::ValidationError(
                    "reader_id must not contain spaces or slashes.".into(),
                )
                .into());
            }
            rid.to_owned()
        }
        None => format!("ip-{}", peer.ip()),
    };

    let matched_spool_id = db::find_spool_id_by_uid(&state.db.pool, &uid_norm).await?;
    let matched_filament_id = db::find_filament_id_by_uid(&state.db.pool, &uid_norm).await?;

    let matched = matched_spool_id
        .map(|id| ("spool", id))
        .or_else(|| matched_filament_id.map(|id| ("filament", id)));

    // Fetch the full entity when matched so the broadcast payload is self-contained.
    let spool_obj = if let Some(sid) = matched_spool_id {
        crate::db::spool::find_by_id(&state.db.pool, sid)
            .await
            .ok()
            .map(Box::new)
    } else {
        None
    };
    let filament_obj = if let Some(fid) = matched_filament_id {
        filament_db::find_by_id(&state.db.pool, fid)
            .await
            .ok()
            .map(Box::new)
    } else {
        None
    };

    let response = ScanTagResponse {
        uid: uid_norm.clone(),
        reader_id: reader_id.clone(),
        name: req.name.clone(),
        format: req.format.clone(),
        matched_spool_id,
        matched_filament_id,
        spool: spool_obj,
        filament: filament_obj,
        created: false,
    };

    if state
        .scan_relay
        .register(&reader_id, req.name.as_deref(), &uid_norm, matched)
    {
        // Wrap in the scan event envelope: {type, resource, date, payload}.
        // serde_json::to_value respects skip_serializing_if so None fields are omitted.
        let date = chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let payload_val = serde_json::to_value(&response).unwrap_or_default();
        let envelope = serde_json::json!({
            "type": "scanned",
            "resource": "tag_scan",
            "date": date,
            "payload": payload_val,
        });
        let json = serde_json::to_string(&envelope).unwrap_or_else(|_| "{}".to_owned());
        let _ = state.scan_tx.send(ScanBroadcastEvent {
            reader_id: reader_id.clone(),
            json,
        });
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
