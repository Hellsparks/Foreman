//! Auth endpoints: server info, first-run setup, login/logout, API keys, network toggle.

use std::time::Instant;

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use foreman_core::{auth, error::AppError, repo};
use serde::Deserialize;
use serde_json::json;
use tracing::{info, warn};

use crate::{
    auth::{
        clear_session_cookie, session_cookie, AuthCtx, AuthKind, ClientIp, SetupCodeError,
        REMEMBER_TTL_SECS, SESSION_TTL_SECS,
    },
    state::AppState,
};

pub const SETTING_ALLOW_LAN: &str = "network.allow_lan";

fn err(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({ "error": message.into(), "code": code })),
    )
        .into_response()
}

fn internal(e: impl std::fmt::Display) -> Response {
    warn!("auth: {e}");
    err(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
}

/// `GET /api/info` (open)
pub async fn info(State(state): State<AppState>) -> Response {
    match auth::has_admin(&state.foreman_db.pool).await {
        Ok(has) => Json(json!({
            "server_id": state.info.server_id,
            "name": state.info.name,
            "version": state.info.version,
            "needs_setup": !has,
        }))
        .into_response(),
        Err(e) => internal(e),
    }
}

/// Create a session and build the response (cookie for browsers, token for other clients).
async fn start_session(
    state: &AppState,
    ip: &ClientIp,
    headers: &HeaderMap,
    username: &str,
    remember: bool,
    return_token: bool,
) -> Response {
    let ttl = if remember {
        REMEMBER_TTL_SECS
    } else {
        SESSION_TTL_SECS
    };
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok());
    match auth::create_session(&state.foreman_db.pool, ttl, Some(&ip.0.to_string()), ua).await {
        Ok((token, expires_at)) => {
            if return_token {
                Json(json!({ "username": username, "expires_at": expires_at, "token": token }))
                    .into_response()
            } else {
                (
                    [(header::SET_COOKIE, session_cookie(&token, remember))],
                    Json(json!({ "username": username, "expires_at": expires_at })),
                )
                    .into_response()
            }
        }
        Err(e) => internal(e),
    }
}

fn too_many(retry: std::time::Duration) -> Response {
    let secs = retry.as_secs().max(1);
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, secs.to_string())],
        Json(json!({
            "error": format!("Too many tries. Wait {secs} s before trying again."),
            "code": "rate_limited",
            "retry_after": secs,
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct SetupBody {
    code: String,
    username: String,
    password: String,
    #[serde(default)]
    remember: bool,
    #[serde(default)]
    return_token: bool,
}

/// `POST /api/auth/setup` (open) — redeem the startup setup code and create the admin.
pub async fn setup(
    State(state): State<AppState>,
    ip: ClientIp,
    headers: HeaderMap,
    Json(body): Json<SetupBody>,
) -> Response {
    let pool = &state.foreman_db.pool;
    match auth::has_admin(pool).await {
        Ok(true) => {
            return err(
                StatusCode::CONFLICT,
                "already_setup",
                "this server already has an admin",
            )
        }
        Ok(false) => {}
        Err(e) => return internal(e),
    }
    let now = Instant::now();
    if let Ok(mut l) = state.auth.limiter.lock() {
        if let Err(wait) = l.check(ip.0, now) {
            return too_many(wait);
        }
    }

    let checked = match state.auth.setup_code.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(c) => c.check(&body.code, now),
            None => Err(SetupCodeError::Expired),
        },
        Err(_) => return internal("setup code lock poisoned"),
    };
    if let Err(e) = checked {
        if e == SetupCodeError::Wrong {
            if let Ok(mut l) = state.auth.limiter.lock() {
                l.fail(ip.0, now);
            }
        }
        let msg = match e {
            SetupCodeError::Wrong => "That setup code is not right. Check the server log.",
            SetupCodeError::Used => "That setup code was already used.",
            SetupCodeError::Expired => {
                "That setup code has expired. Restart the server to print a new one."
            }
        };
        return err(StatusCode::FORBIDDEN, "bad_setup_code", msg);
    }

    if let Err(e) = auth::create_admin(pool, &body.username, &body.password).await {
        return err(StatusCode::BAD_REQUEST, "invalid", e.to_string());
    }
    if let Ok(mut guard) = state.auth.setup_code.lock() {
        if let Some(c) = guard.as_mut() {
            c.mark_used();
        }
    }
    if let Ok(mut l) = state.auth.limiter.lock() {
        l.success(ip.0);
    }
    let username = body.username.trim().to_owned();
    info!(%username, "admin account created");
    if state.info.bound_host != state.info.configured_host && !state.info.desktop_mode {
        info!(
            "restart the server to listen on {} (bound to {} until an admin existed)",
            state.info.configured_host, state.info.bound_host
        );
    }
    start_session(
        &state,
        &ip,
        &headers,
        &username,
        body.remember,
        body.return_token,
    )
    .await
}

#[derive(Deserialize)]
pub struct LoginBody {
    username: String,
    password: String,
    #[serde(default)]
    remember: bool,
    #[serde(default)]
    return_token: bool,
}

/// `POST /api/auth/login` (open)
pub async fn login(
    State(state): State<AppState>,
    ip: ClientIp,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Response {
    let now = Instant::now();
    if let Ok(mut l) = state.auth.limiter.lock() {
        if let Err(wait) = l.check(ip.0, now) {
            return too_many(wait);
        }
    }
    let ok = match auth::check_login(&state.foreman_db.pool, &body.username, &body.password).await {
        Ok(ok) => ok,
        Err(e) => return internal(e),
    };
    if !ok {
        let left = state
            .auth
            .limiter
            .lock()
            .map(|mut l| l.fail(ip.0, now))
            .unwrap_or(0);
        warn!(ip = %ip.0, "failed login");
        let msg = if left == 0 {
            "Too many tries. Wait 60 s before trying again.".to_owned()
        } else {
            format!("Wrong username or password. {left} tries left before a 60 s pause.")
        };
        return err(StatusCode::UNAUTHORIZED, "bad_credentials", msg);
    }
    if let Ok(mut l) = state.auth.limiter.lock() {
        l.success(ip.0);
    }
    let username = body.username.trim().to_owned();
    start_session(
        &state,
        &ip,
        &headers,
        &username,
        body.remember,
        body.return_token,
    )
    .await
}

/// `POST /api/auth/logout`
pub async fn logout(State(state): State<AppState>, Extension(ctx): Extension<AuthCtx>) -> Response {
    if let AuthKind::Session { token } = &ctx.kind {
        if let Err(e) = auth::delete_session(&state.foreman_db.pool, token).await {
            return internal(e);
        }
    }
    (
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, clear_session_cookie())],
    )
        .into_response()
}

/// `GET /api/auth/me`
pub async fn me(Extension(ctx): Extension<AuthCtx>) -> Json<serde_json::Value> {
    match ctx.kind {
        AuthKind::Session { .. } => Json(json!({ "username": ctx.username, "kind": "session" })),
        AuthKind::ApiKey { id } => {
            Json(json!({ "username": ctx.username, "kind": "api_key", "api_key_id": id }))
        }
    }
}

/// API keys may not manage keys or network exposure: `Some(403)` for them.
fn forbid_api_key(ctx: &AuthCtx) -> Option<Response> {
    match ctx.kind {
        AuthKind::Session { .. } => None,
        AuthKind::ApiKey { .. } => Some(err(
            StatusCode::FORBIDDEN,
            "session_required",
            "sign in to do this (API keys can't)",
        )),
    }
}

/// `GET /api/auth/api-keys`
pub async fn list_keys(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
) -> Response {
    if let Some(r) = forbid_api_key(&ctx) {
        return r;
    }
    match auth::list_api_keys(&state.foreman_db.pool).await {
        Ok(keys) => Json(keys).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
pub struct CreateKeyBody {
    name: String,
}

/// `POST /api/auth/api-keys` — the plaintext key is only in this response.
pub async fn create_key(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Json(body): Json<CreateKeyBody>,
) -> Response {
    if let Some(r) = forbid_api_key(&ctx) {
        return r;
    }
    match auth::create_api_key(&state.foreman_db.pool, &body.name).await {
        Ok((key, plain)) => {
            info!(name = %key.name, "API key created");
            (
                StatusCode::CREATED,
                Json(json!({ "key": key, "secret": plain })),
            )
                .into_response()
        }
        Err(e) => err(StatusCode::BAD_REQUEST, "invalid", e.to_string()),
    }
}

/// `DELETE /api/auth/api-keys/{id}`
pub async fn revoke_key(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<i64>,
) -> Response {
    if let Some(r) = forbid_api_key(&ctx) {
        return r;
    }
    match auth::revoke_api_key(&state.foreman_db.pool, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(StatusCode::NOT_FOUND, "not_found", e.to_string()),
    }
}

async fn network_body(state: &AppState) -> Result<serde_json::Value, AppError> {
    let allow_lan = repo::setting::get(&state.foreman_db.pool, SETTING_ALLOW_LAN)
        .await?
        .is_some_and(|v| v == "true");
    let (wanted, _) = crate::auth::effective_bind_host(
        &state.info.configured_host,
        true,
        state.info.desktop_mode,
        allow_lan,
    );
    Ok(json!({
        "desktop_mode": state.info.desktop_mode,
        "allow_lan": allow_lan,
        "configured_host": state.info.configured_host,
        "bound_host": state.info.bound_host,
        "restart_required": wanted != state.info.bound_host,
    }))
}

/// `GET /api/settings/network`
pub async fn get_network(State(state): State<AppState>) -> Response {
    match network_body(&state).await {
        Ok(b) => Json(b).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
pub struct NetworkBody {
    allow_lan: bool,
}

/// `PUT /api/settings/network` — "Let other devices connect" (applies on next start).
pub async fn put_network(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Json(body): Json<NetworkBody>,
) -> Response {
    if let Some(r) = forbid_api_key(&ctx) {
        return r;
    }
    let value = if body.allow_lan { "true" } else { "false" };
    if let Err(e) = repo::setting::set(&state.foreman_db.pool, SETTING_ALLOW_LAN, value).await {
        return internal(e);
    }
    info!(
        allow_lan = body.allow_lan,
        "network exposure setting changed"
    );
    match network_body(&state).await {
        Ok(b) => Json(b).into_response(),
        Err(e) => internal(e),
    }
}
