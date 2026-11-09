//! Request authentication for `/api/*` and `/ws/*`.
//!
//! Credentials, checked in order: `X-Api-Key` header (API key), `Authorization: Bearer`
//! (session token, Tauri/other clients), the `foreman_session` HttpOnly cookie (browser),
//! and for WebSocket upgrades only a `?token=` query parameter (browsers can't set headers
//! on WebSockets from non-cookie clients).

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, FromRequestParts, Request, State},
    http::{header, request::Parts, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use foreman_core::auth;
use serde_json::json;
use tracing::warn;

use crate::state::AppState;

pub const SESSION_COOKIE: &str = "foreman_session";
pub const SESSION_TTL_SECS: i64 = 12 * 3600;
pub const REMEMBER_TTL_SECS: i64 = 30 * 24 * 3600;
pub const SETUP_CODE_TTL: Duration = Duration::from_secs(24 * 3600);
pub const MAX_FAILED_TRIES: u32 = 5;
pub const LOCKOUT: Duration = Duration::from_secs(60);

/// Routes reachable without credentials.
const OPEN_ROUTES: &[&str] = &[
    "/api/health",
    "/api/info",
    "/api/auth/setup",
    "/api/auth/login",
];

// ---------------------------------------------------------------------------
// Setup code
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum SetupCodeError {
    Used,
    Expired,
    Wrong,
}

/// One-time code printed at startup when no admin exists.
pub struct SetupCode {
    code: String,
    created: Instant,
    used: bool,
}

impl SetupCode {
    pub fn new(code: String, created: Instant) -> Self {
        Self {
            code,
            created,
            used: false,
        }
    }

    /// Validate without consuming (consume with [`mark_used`](Self::mark_used) once setup succeeds).
    pub fn check(&self, input: &str, now: Instant) -> Result<(), SetupCodeError> {
        if self.used {
            return Err(SetupCodeError::Used);
        }
        if now.saturating_duration_since(self.created) > SETUP_CODE_TTL {
            return Err(SetupCodeError::Expired);
        }
        let normalized: String = input
            .trim()
            .to_ascii_uppercase()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if auth::secure_eq(&normalized, &self.code) {
            Ok(())
        } else {
            Err(SetupCodeError::Wrong)
        }
    }

    pub fn mark_used(&mut self) {
        self.used = true;
    }

    /// The code while it can still be redeemed (desktop Mode A prefills it).
    pub fn current(&self, now: Instant) -> Option<&str> {
        (!self.used && now.saturating_duration_since(self.created) <= SETUP_CODE_TTL)
            .then_some(self.code.as_str())
    }
}

// ---------------------------------------------------------------------------
// Rate limiter: MAX_FAILED_TRIES failures per IP, then a LOCKOUT pause
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Attempts {
    fails: u32,
    locked_until: Option<Instant>,
}

#[derive(Default)]
pub struct RateLimiter {
    by_ip: HashMap<IpAddr, Attempts>,
}

impl RateLimiter {
    /// `Err(retry_after)` while the IP is paused.
    pub fn check(&mut self, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        if let Some(a) = self.by_ip.get_mut(&ip) {
            if let Some(until) = a.locked_until {
                if now < until {
                    return Err(until - now);
                }
                a.locked_until = None;
                a.fails = 0;
            }
        }
        Ok(())
    }

    /// Record a failure; returns tries left before the pause (0 = now paused).
    pub fn fail(&mut self, ip: IpAddr, now: Instant) -> u32 {
        let a = self.by_ip.entry(ip).or_default();
        a.fails += 1;
        if a.fails >= MAX_FAILED_TRIES {
            a.fails = 0;
            a.locked_until = Some(now + LOCKOUT);
            return 0;
        }
        MAX_FAILED_TRIES - a.fails
    }

    pub fn success(&mut self, ip: IpAddr) {
        self.by_ip.remove(&ip);
    }
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct AuthState {
    pub setup_code: Mutex<Option<SetupCode>>,
    pub limiter: Mutex<RateLimiter>,
}

/// Facts about this server (for `/api/info`, the network setting and mDNS).
pub struct ServerInfo {
    pub server_id: String,
    pub version: &'static str,
    pub desktop_mode: bool,
    pub configured_host: String,
    pub port: u16,
    pub hostname: String,
    pub magic_dns: Option<String>,
    /// Host the listener is bound to; sending a new value rebinds it.
    pub bind: tokio::sync::watch::Sender<String>,
}

impl ServerInfo {
    pub fn bound_host(&self) -> String {
        self.bind.borrow().clone()
    }
}

/// Who made the request (inserted by [`require_auth`]).
#[derive(Clone, Debug)]
pub struct AuthCtx {
    pub username: String,
    pub kind: AuthKind,
}

#[derive(Clone, Debug)]
pub enum AuthKind {
    Session { token: String },
    ApiKey { id: i64 },
}

/// Peer IP from the connection (falls back to loopback when unavailable, e.g. in tests).
pub struct ClientIp(pub IpAddr);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(ClientIp(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip())
                .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        ))
    }
}

// ---------------------------------------------------------------------------
// Bind rule
// ---------------------------------------------------------------------------

pub fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

/// Address to listen on, plus why it differs from the configured one (if it does).
///
/// - No admin yet → loopback only, so nobody on the LAN can claim the server first.
/// - Desktop Mode A → loopback unless "Let other devices connect" is on.
/// - Otherwise the configured address.
pub fn effective_bind_host(
    configured: &str,
    has_admin: bool,
    desktop_mode: bool,
    allow_lan: bool,
) -> (String, Option<&'static str>) {
    if !has_admin {
        let reason = (!is_loopback(configured))
            .then_some("no admin account yet — listening on 127.0.0.1 only until setup is done");
        return ("127.0.0.1".into(), reason);
    }
    if desktop_mode {
        if !allow_lan {
            return ("127.0.0.1".into(), None);
        }
        if is_loopback(configured) {
            return ("0.0.0.0".into(), None);
        }
    }
    (configured.to_owned(), None)
}

// ---------------------------------------------------------------------------
// Cookies
// ---------------------------------------------------------------------------

pub fn session_cookie(token: &str, remember: bool) -> String {
    let max_age = if remember {
        format!("; Max-Age={REMEMBER_TTL_SECS}")
    } else {
        String::new()
    };
    format!("{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/{max_age}")
}

pub fn clear_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0")
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| {
            let (k, v) = kv.trim().split_once('=')?;
            (k == SESSION_COOKIE && !v.is_empty()).then(|| v.to_owned())
        })
}

/// The session token from `Authorization: Bearer` or the session cookie.
pub fn session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_owned())
        .or_else(|| cookie_token(headers))
}

fn query_token(req: &Request) -> Option<String> {
    req.uri().query()?.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == "token" && !v.is_empty()).then(|| v.to_owned())
    })
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

fn unauthorized(code: &'static str, message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": message, "code": code })),
    )
        .into_response()
}

pub fn needs_auth(path: &str) -> bool {
    (path.starts_with("/api/") || path.starts_with("/ws/")) && !OPEN_ROUTES.contains(&path)
}

/// Routes where `?token=` is accepted: WebSockets and files/images opened via
/// `<img src>` / `<a href>`, none of which can carry an `Authorization` header.
fn query_token_allowed(method: &axum::http::Method, path: &str) -> bool {
    path.starts_with("/ws/")
        || (method == axum::http::Method::GET
            && path.starts_with("/api/printers/")
            && (path.ends_with("/snapshot") || path.ends_with("/files/download")))
}

/// Audit name for sessions minted in-process by the desktop app before an admin exists.
pub const LOCAL_APP_USER: &str = "local";

/// Reject unauthenticated requests to protected routes; attach [`AuthCtx`] otherwise.
pub async fn require_auth(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    if !needs_auth(&path) {
        return next.run(req).await;
    }
    let pool = &state.foreman_db.pool;

    // No admin yet: only sessions minted in-process by the desktop app (Mode A) exist.
    let admin = match auth::admin_username(pool).await {
        Ok(a) => a,
        Err(e) => {
            warn!("auth: admin lookup failed: {e}");
            return unauthorized("unauthorized", "sign in required");
        }
    };

    let headers = req.headers();
    let kind = if let Some(key) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        match auth::check_api_key(pool, key.trim()).await {
            Ok(Some(id)) => Some(AuthKind::ApiKey { id }),
            _ => None,
        }
    } else {
        let token = session_token(headers).or_else(|| {
            query_token_allowed(req.method(), &path)
                .then(|| query_token(&req))
                .flatten()
        });
        match token {
            Some(t) => match auth::session_valid(pool, &t, chrono::Utc::now().timestamp()).await {
                Ok(true) => Some(AuthKind::Session { token: t }),
                _ => None,
            },
            None => None,
        }
    };

    match (kind, admin) {
        (Some(kind), admin) => {
            let username = admin.unwrap_or_else(|| LOCAL_APP_USER.to_owned());
            req.extensions_mut().insert(AuthCtx { username, kind });
            next.run(req).await
        }
        (None, None) => unauthorized("setup_required", "this server has no admin account yet"),
        (None, Some(_)) => unauthorized("unauthorized", "sign in required"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_code_single_use_and_expiry() {
        let t0 = Instant::now();
        let mut c = SetupCode::new("K7QD-2MXP".into(), t0);
        assert_eq!(c.check("WRNG-CODE", t0), Err(SetupCodeError::Wrong));
        assert_eq!(c.check(" k7qd-2mxp ", t0), Ok(()));
        c.mark_used();
        assert_eq!(c.check("K7QD-2MXP", t0), Err(SetupCodeError::Used));

        let fresh = SetupCode::new("K7QD-2MXP".into(), t0);
        let later = t0 + SETUP_CODE_TTL + Duration::from_secs(1);
        assert_eq!(
            fresh.check("K7QD-2MXP", later),
            Err(SetupCodeError::Expired)
        );
        assert_eq!(
            fresh.check("K7QD-2MXP", t0 + SETUP_CODE_TTL - Duration::from_secs(1)),
            Ok(())
        );
    }

    #[test]
    fn rate_limit_five_fails_then_pause() {
        let ip: IpAddr = "10.0.0.5".parse().expect("ip");
        let other: IpAddr = "10.0.0.6".parse().expect("ip");
        let t0 = Instant::now();
        let mut rl = RateLimiter::default();
        for left in (1..MAX_FAILED_TRIES).rev() {
            assert!(rl.check(ip, t0).is_ok());
            assert_eq!(rl.fail(ip, t0), left);
        }
        assert_eq!(rl.fail(ip, t0), 0);
        let wait = rl
            .check(ip, t0 + Duration::from_secs(1))
            .expect_err("paused");
        assert_eq!(wait, Duration::from_secs(59));
        assert!(rl.check(other, t0).is_ok(), "other IPs unaffected");
        assert!(rl.check(ip, t0 + LOCKOUT).is_ok(), "pause over");
        assert_eq!(
            rl.fail(ip, t0 + LOCKOUT),
            MAX_FAILED_TRIES - 1,
            "counter reset"
        );
        rl.success(ip);
        assert_eq!(rl.fail(ip, t0 + LOCKOUT), MAX_FAILED_TRIES - 1);
    }

    #[test]
    fn bind_rule() {
        // No admin → loopback, with a reason only when that changes something.
        assert_eq!(
            effective_bind_host("0.0.0.0", false, false, true).0,
            "127.0.0.1"
        );
        assert!(effective_bind_host("0.0.0.0", false, false, true)
            .1
            .is_some());
        assert!(effective_bind_host("127.0.0.1", false, false, false)
            .1
            .is_none());
        // Admin, server mode → configured address.
        assert_eq!(
            effective_bind_host("0.0.0.0", true, false, false).0,
            "0.0.0.0"
        );
        assert_eq!(
            effective_bind_host("10.0.0.10", true, false, false).0,
            "10.0.0.10"
        );
        // Desktop: loopback unless the LAN toggle is on.
        assert_eq!(
            effective_bind_host("127.0.0.1", true, true, false).0,
            "127.0.0.1"
        );
        assert_eq!(
            effective_bind_host("127.0.0.1", true, true, true).0,
            "0.0.0.0"
        );
        assert_eq!(
            effective_bind_host("0.0.0.0", true, true, false).0,
            "127.0.0.1"
        );
    }

    #[test]
    fn open_routes() {
        assert!(!needs_auth("/api/health"));
        assert!(!needs_auth("/api/info"));
        assert!(!needs_auth("/api/auth/login"));
        assert!(!needs_auth("/api/auth/setup"));
        assert!(!needs_auth("/assets/index.js"));
        assert!(!needs_auth("/"));
        assert!(needs_auth("/api/printers"));
        assert!(needs_auth("/api/auth/logout"));
        assert!(needs_auth("/ws/events"));
        assert!(needs_auth("/ws/printers/1/terminal"));
    }

    #[test]
    fn credential_parsing() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            "a=b; foreman_session=fms_abc; c=d".parse().expect("hv"),
        );
        assert_eq!(session_token(&h).as_deref(), Some("fms_abc"));
        h.insert(header::AUTHORIZATION, "Bearer fms_xyz".parse().expect("hv"));
        assert_eq!(session_token(&h).as_deref(), Some("fms_xyz"));
        assert!(session_cookie("t", false).contains("HttpOnly; SameSite=Strict"));
        assert!(!session_cookie("t", false).contains("Max-Age"));
        assert!(session_cookie("t", true).contains("Max-Age=2592000"));
    }
}
