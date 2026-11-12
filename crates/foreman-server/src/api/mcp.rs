//! Settings → Security → MCP (task 6.5): the tool catalogue and each API key's MCP access.
//!
//! - `GET /api/settings/mcp` — endpoint path, protocol versions, tools, keys + policies
//! - `PUT /api/settings/mcp/keys/{id}` — set a key's [`McpPolicy`]
//!
//! Session only: an API key cannot widen its own access.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use foreman_core::{auth, domain::ApiKey, error::AppError, repo, repo::mcp_policy::McpPolicy};
use serde::Serialize;
use serde_json::json;
use tracing::info;
use ts_rs::TS;

use crate::{
    api::auth::forbid_api_key,
    auth::AuthCtx,
    mcp::{self, catalogue},
    state::AppState,
};

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct McpTool {
    pub name: String,
    pub title: String,
    pub description: String,
    pub class: catalogue::Class,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct McpKey {
    pub key: ApiKey,
    pub policy: McpPolicy,
    /// Tools this key can call right now.
    pub reachable: Vec<String>,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct McpSettings {
    /// Path of the MCP endpoint on this server (`/mcp`).
    pub path: String,
    pub protocol_versions: Vec<String>,
    pub tools: Vec<McpTool>,
    pub keys: Vec<McpKey>,
}

fn fail(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

async fn key_entry(state: &AppState, key: ApiKey) -> Result<McpKey, AppError> {
    let policy = repo::mcp_policy::get(&state.foreman_db.pool, key.id).await?;
    let reachable = mcp::reachable(&policy);
    Ok(McpKey {
        key,
        policy,
        reachable,
    })
}

/// `GET /api/settings/mcp`
pub async fn get(State(state): State<AppState>, Extension(ctx): Extension<AuthCtx>) -> Response {
    if let Some(r) = forbid_api_key(&ctx) {
        return r;
    }
    let keys = match auth::list_api_keys(&state.foreman_db.pool).await {
        Ok(k) => k,
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let mut entries = Vec::with_capacity(keys.len());
    for k in keys {
        match key_entry(&state, k).await {
            Ok(e) => entries.push(e),
            Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        }
    }
    Json(McpSettings {
        path: mcp::PATH.to_owned(),
        protocol_versions: std::iter::once(mcp::MODERN)
            .chain(mcp::LEGACY.iter().copied())
            .map(str::to_owned)
            .collect(),
        tools: catalogue::TOOLS
            .iter()
            .map(|t| McpTool {
                name: t.name.to_owned(),
                title: t.title.to_owned(),
                description: t.description.to_owned(),
                class: t.class,
            })
            .collect(),
        keys: entries,
    })
    .into_response()
}

/// `PUT /api/settings/mcp/keys/{id}`
pub async fn put_key(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<i64>,
    Json(mut policy): Json<McpPolicy>,
) -> Response {
    if let Some(r) = forbid_api_key(&ctx) {
        return r;
    }
    let pool = &state.foreman_db.pool;
    let key = match auth::list_api_keys(pool).await {
        Ok(keys) => keys.into_iter().find(|k| k.id == id),
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let Some(key) = key else {
        return fail(StatusCode::NOT_FOUND, format!("api key {id} not found"));
    };
    if let Some(tools) = policy.tools.as_mut() {
        if let Some(bad) = tools.iter().find(|n| catalogue::find(n).is_none()) {
            return fail(StatusCode::BAD_REQUEST, format!("unknown tool: {bad}"));
        }
        tools.sort();
        tools.dedup();
    }
    if let Err(e) = repo::mcp_policy::set(pool, id, &policy).await {
        return fail(StatusCode::BAD_REQUEST, e.to_string());
    }
    info!(
        key = %key.name,
        allow_destructive = policy.allow_destructive,
        rate = policy.rate_per_minute,
        tools = policy.tools.as_ref().map_or(-1, |t| t.len() as i64),
        "MCP access changed"
    );
    match key_entry(&state, key).await {
        Ok(e) => Json(e).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}
