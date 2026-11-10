//! Request authentication for `/api/*` and `/ws/*`.
//!
//! Credentials, checked in order: `X-Api-Key` header (API key), `Authorization: Bearer`
//! (session token, Tauri/other clients), the `foreman_session` HttpOnly cookie (browser),
//! and on snapshot/download/WebSocket routes a scoped `?token=` from
//! `POST /api/auth/url-token` (session tokens are never accepted in URLs).

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
use foreman_core::{auth, error::AppError};
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

/// The only API routes an unclaimed server (no admin yet) answers.
const UNCLAIMED_ROUTES: &[&str] = &["/api/health", "/api/info", "/api/auth/setup"];

/// Lifetime of a scoped URL token (`POST /api/auth/url-token`).
pub const URL_TOKEN_TTL: Duration = Duration::from_secs(60);
const URL_TOKEN_PREFIX: &str = "fmu_";

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
    pub url_grants: Mutex<UrlGrants>,
    pub deny_log: Mutex<DenyLog>,
}

// ---------------------------------------------------------------------------
// Denied-request log (401/403)
// ---------------------------------------------------------------------------

/// Why a request was refused; handlers attach it to 401/403 responses as an extension.
#[derive(Clone, Debug)]
pub struct DenyReason(pub String);

/// One log line per (IP, reason) per [`DENY_LOG_WINDOW`]; repeats are counted and
/// reported on the next line.
#[derive(Default)]
pub struct DenyLog {
    seen: HashMap<(IpAddr, String), (Instant, u32)>,
}

pub const DENY_LOG_WINDOW: Duration = Duration::from_secs(60);
const DENY_LOG_MAX_KEYS: usize = 4096;

impl DenyLog {
    /// `Some(suppressed_since_last_line)` when this denial should be logged now.
    pub fn should_log(&mut self, ip: IpAddr, reason: &str, now: Instant) -> Option<u32> {
        if self.seen.len() >= DENY_LOG_MAX_KEYS {
            self.seen
                .retain(|_, (t, _)| now.duration_since(*t) < DENY_LOG_WINDOW);
        }
        match self.seen.get_mut(&(ip, reason.to_owned())) {
            Some((t, n)) if now.duration_since(*t) < DENY_LOG_WINDOW => {
                *n += 1;
                None
            }
            Some((t, n)) => {
                let suppressed = *n;
                *t = now;
                *n = 0;
                Some(suppressed)
            }
            None => {
                self.seen.insert((ip, reason.to_owned()), (now, 0));
                Some(0)
            }
        }
    }
}

/// Warn about a refused request: IP, method, route (no query, so no URL token) and reason.
fn log_denied(state: &AppState, ip: IpAddr, method: &str, path: &str, status: u16, reason: &str) {
    let Ok(mut log) = state.auth.deny_log.lock() else {
        return;
    };
    if let Some(suppressed) = log.should_log(ip, reason, Instant::now()) {
        if suppressed > 0 {
            warn!(%ip, method, route = path, status, reason, suppressed, "auth denied (repeats since last line)");
        } else {
            warn!(%ip, method, route = path, status, reason, "auth denied");
        }
    }
}

// ---------------------------------------------------------------------------
// Scoped URL tokens (for <img src>, <a href>, WebSockets)
// ---------------------------------------------------------------------------

struct UrlGrant {
    path: String,
    ctx: AuthCtx,
    expires: Instant,
}

/// Short-lived tokens bound to one path; stored hashed, in memory only.
#[derive(Default)]
pub struct UrlGrants {
    by_hash: HashMap<String, UrlGrant>,
}

/// Paths a URL token can be issued for: camera snapshots, file downloads, WebSockets.
pub fn url_token_path_allowed(path: &str) -> bool {
    let printer_tail = |prefix: &str, tail: &str| {
        path.strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(tail))
            .is_some_and(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))
    };
    path == "/ws/events"
        || printer_tail("/api/printers/", "/snapshot")
        || printer_tail("/api/printers/", "/files/download")
        || printer_tail("/ws/printers/", "/terminal")
        || printer_tail("/api/library/files/", "/thumb")
        || printer_tail("/api/library/files/", "/download")
        || printer_tail("/api/printers/", "/storage/thumb")
        || printer_tail("/api/printers/", "/storage/download")
}

impl UrlGrants {
    pub fn issue(&mut self, path: &str, ctx: AuthCtx, now: Instant) -> Result<String, AppError> {
        self.by_hash.retain(|_, g| g.expires > now);
        let token = auth::random_token(URL_TOKEN_PREFIX)?;
        self.by_hash.insert(
            auth::token_hash(&token),
            UrlGrant {
                path: path.to_owned(),
                ctx,
                expires: now + URL_TOKEN_TTL,
            },
        );
        Ok(token)
    }

    /// The grant's identity if `token` is valid for exactly `path` at `now`.
    pub fn check(&self, token: &str, path: &str, now: Instant) -> Option<AuthCtx> {
        let g = self.by_hash.get(&auth::token_hash(token))?;
        (g.path == path && now < g.expires).then(|| g.ctx.clone())
    }
}

/// `token=…` query values replaced, so URLs can be logged safely.
pub fn redact_token(uri: &str) -> String {
    let Some((base, query)) = uri.split_once('?') else {
        return uri.to_owned();
    };
    let query: Vec<String> = query
        .split('&')
        .map(|kv| match kv.split_once('=') {
            Some(("token", _)) => "token=REDACTED".to_owned(),
            _ => kv.to_owned(),
        })
        .collect();
    format!("{base}?{}", query.join("&"))
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
        Ok(ClientIp(peer_ip(&parts.extensions)))
    }
}

/// Peer IP with IPv4-mapped IPv6 (`::ffff:a.b.c.d` from the dual-stack listener) unwrapped.
fn peer_ip(ext: &axum::http::Extensions) -> IpAddr {
    ext.get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_canonical())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
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

/// Address to listen on.
///
/// - Server: the configured address, even before setup (an unclaimed server only
///   answers health, info and setup until the admin exists).
/// - Desktop Mode A: loopback unless an admin exists and "Let other devices connect" is on.
pub fn effective_bind_host(
    configured: &str,
    has_admin: bool,
    desktop_mode: bool,
    allow_lan: bool,
) -> String {
    if desktop_mode {
        if !(has_admin && allow_lan) {
            return "127.0.0.1".into();
        }
        if is_loopback(configured) {
            return "0.0.0.0".into();
        }
    }
    configured.to_owned()
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

fn unauthorized(code: &'static str, message: &str, reason: &str) -> Response {
    let mut r = (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": message, "code": code })),
    )
        .into_response();
    r.extensions_mut().insert(DenyReason(reason.to_owned()));
    r
}

fn not_set_up() -> Response {
    let mut r = (
        StatusCode::FORBIDDEN,
        Json(json!({ "error": "server not set up", "code": "setup_required" })),
    )
        .into_response();
    r.extensions_mut()
        .insert(DenyReason("server not set up".into()));
    r
}

pub fn needs_auth(path: &str) -> bool {
    (path.starts_with("/api/") || path.starts_with("/ws/")) && !OPEN_ROUTES.contains(&path)
}

/// Audit name for sessions minted in-process by the desktop app before an admin exists.
pub const LOCAL_APP_USER: &str = "local";

/// Credentials on the request: API key, bearer/cookie session, or a scoped URL token.
/// Session tokens are never taken from the URL. `Err` carries the reason for the log.
async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<AuthKind, &'static str> {
    let pool = &state.foreman_db.pool;
    if let Some(key) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        return match auth::check_api_key(pool, key.trim()).await {
            Ok(Some(id)) => Ok(AuthKind::ApiKey { id }),
            _ => Err("invalid API key"),
        };
    }
    let t = session_token(headers).ok_or("no credentials")?;
    match auth::session_valid(pool, &t, chrono::Utc::now().timestamp()).await {
        Ok(true) => Ok(AuthKind::Session { token: t }),
        _ => Err("invalid or expired session"),
    }
}

fn url_grant(state: &AppState, req: &Request, path: &str) -> Option<AuthCtx> {
    if req.method() != axum::http::Method::GET || !url_token_path_allowed(path) {
        return None;
    }
    let token = query_token(req)?;
    let grants = state.auth.url_grants.lock().ok()?;
    grants.check(&token, path, Instant::now())
}

/// Reject unauthenticated requests to protected routes; attach [`AuthCtx`] otherwise.
/// Every 401/403 (from here or a handler) is logged via [`log_denied`].
///
/// An unclaimed server (no admin) answers only health, info and setup; everything else
/// is 403 — except in-process desktop sessions (Mode A, loopback-only until setup).
pub async fn require_auth(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    if !(path.starts_with("/api/") || path.starts_with("/ws/")) {
        return next.run(req).await;
    }
    let ip = peer_ip(req.extensions());
    let method = req.method().to_string();
    let resp = gate(&state, req, next, &path).await;
    let status = resp.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        let reason = resp
            .extensions()
            .get::<DenyReason>()
            .map(|r| r.0.as_str())
            .unwrap_or("refused by handler");
        log_denied(&state, ip, &method, &path, status.as_u16(), reason);
    }
    resp
}

async fn gate(state: &AppState, mut req: Request, next: Next, path: &str) -> Response {
    let admin = match auth::admin_username(&state.foreman_db.pool).await {
        Ok(a) => a,
        Err(e) => {
            warn!("auth: admin lookup failed: {e}");
            return unauthorized("unauthorized", "sign in required", "admin lookup failed");
        }
    };

    let Some(admin) = admin else {
        if UNCLAIMED_ROUTES.contains(&path) {
            return next.run(req).await;
        }
        return match authenticate(state, req.headers()).await {
            Ok(kind @ AuthKind::Session { .. }) => {
                req.extensions_mut().insert(AuthCtx {
                    username: LOCAL_APP_USER.to_owned(),
                    kind,
                });
                next.run(req).await
            }
            _ => not_set_up(),
        };
    };

    if !needs_auth(path) {
        return next.run(req).await;
    }
    let ctx = match authenticate(state, req.headers()).await {
        Ok(kind) => Ok(AuthCtx {
            username: admin,
            kind,
        }),
        Err(reason) => match url_grant(state, &req, path) {
            Some(ctx) => Ok(ctx),
            None if query_token(&req).is_some() => Err("invalid or expired URL token"),
            None => Err(reason),
        },
    };
    match ctx {
        Ok(ctx) => {
            req.extensions_mut().insert(ctx);
            next.run(req).await
        }
        Err(reason) => unauthorized("unauthorized", "sign in required", reason),
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
    fn deny_log_once_per_window_then_reports_repeats() {
        let ip: IpAddr = "10.0.0.5".parse().expect("ip");
        let t0 = Instant::now();
        let mut log = DenyLog::default();
        assert_eq!(log.should_log(ip, "no credentials", t0), Some(0));
        for _ in 0..3 {
            assert_eq!(log.should_log(ip, "no credentials", t0), None);
        }
        assert_eq!(
            log.should_log(ip, "invalid API key", t0),
            Some(0),
            "other reason logs"
        );
        assert_eq!(
            log.should_log(ip, "no credentials", t0 + DENY_LOG_WINDOW),
            Some(3)
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
        // Server mode: the configured address, admin or not (unclaimed servers are gated).
        assert_eq!(
            effective_bind_host("0.0.0.0", false, false, false),
            "0.0.0.0"
        );
        assert_eq!(
            effective_bind_host("0.0.0.0", true, false, false),
            "0.0.0.0"
        );
        assert_eq!(
            effective_bind_host("10.0.0.10", true, false, false),
            "10.0.0.10"
        );
        assert_eq!(
            effective_bind_host("127.0.0.1", false, false, true),
            "127.0.0.1"
        );
        // Desktop: loopback unless an admin exists and the LAN toggle is on.
        assert_eq!(
            effective_bind_host("127.0.0.1", true, true, false),
            "127.0.0.1"
        );
        assert_eq!(
            effective_bind_host("127.0.0.1", false, true, true),
            "127.0.0.1"
        );
        assert_eq!(
            effective_bind_host("127.0.0.1", true, true, true),
            "0.0.0.0"
        );
        assert_eq!(
            effective_bind_host("0.0.0.0", true, true, false),
            "127.0.0.1"
        );
    }

    fn ctx() -> AuthCtx {
        AuthCtx {
            username: "admin".into(),
            kind: AuthKind::ApiKey { id: 1 },
        }
    }

    #[test]
    fn url_tokens_are_scoped_and_short_lived() {
        let t0 = Instant::now();
        let mut g = UrlGrants::default();
        let tok = g
            .issue("/api/printers/1/snapshot", ctx(), t0)
            .expect("issue");
        assert!(tok.starts_with(URL_TOKEN_PREFIX));
        assert!(g.check(&tok, "/api/printers/1/snapshot", t0).is_some());
        assert!(
            g.check(&tok, "/api/printers/2/snapshot", t0).is_none(),
            "other path"
        );
        assert!(g.check(&tok, "/ws/events", t0).is_none(), "other path");
        assert!(
            g.check(&tok, "/api/printers/1/snapshot", t0 + URL_TOKEN_TTL)
                .is_none(),
            "expired"
        );
        assert!(g
            .check("fmu_forged", "/api/printers/1/snapshot", t0)
            .is_none());
        // Expired grants are dropped on the next issue.
        g.issue("/ws/events", ctx(), t0 + URL_TOKEN_TTL)
            .expect("issue");
        assert_eq!(g.by_hash.len(), 1);
    }

    #[test]
    fn url_token_paths() {
        assert!(url_token_path_allowed("/ws/events"));
        assert!(url_token_path_allowed("/ws/printers/3/terminal"));
        assert!(url_token_path_allowed("/api/printers/12/snapshot"));
        assert!(url_token_path_allowed("/api/printers/12/files/download"));
        assert!(!url_token_path_allowed("/api/printers"));
        assert!(!url_token_path_allowed("/api/printers/x/snapshot"));
        assert!(!url_token_path_allowed("/api/printers//snapshot"));
        assert!(!url_token_path_allowed("/api/auth/api-keys"));
    }

    #[test]
    fn token_redaction() {
        assert_eq!(
            redact_token("/ws/events?token=fmu_secret"),
            "/ws/events?token=REDACTED"
        );
        assert_eq!(
            redact_token("/api/printers/1/files/download?path=%2Fa&token=fmu_x&inline=true"),
            "/api/printers/1/files/download?path=%2Fa&token=REDACTED&inline=true"
        );
        assert_eq!(redact_token("/api/printers"), "/api/printers");
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
