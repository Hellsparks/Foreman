//! MCP endpoint (task 6.5, W14): `POST /mcp`, Streamable HTTP, JSON responses.
//!
//! Protocol: the 2026-07-28 revision (stateless: per-request `_meta`, `server/discover`,
//! `MCP-Protocol-Version` / `Mcp-Method` / `Mcp-Name` headers) and the earlier
//! `initialize`-handshake revisions (2025-11-25 back to 2025-03-26), chosen per request:
//! an `initialize` call, or a request without the 2026 `_meta`, is served the legacy way.
//! No sessions are kept in either mode (`Mcp-Session-Id` is never issued); `GET` and
//! `DELETE` answer 405.
//!
//! Auth: a Foreman API key, as `Authorization: Bearer fmk_…` or `X-Api-Key`. Each key has
//! an [`McpPolicy`]: an allow list (default: every read-only tool), a destructive switch
//! (default off) and a rate limit. Every non-read-only call (and every refusal of one) is
//! written to `audit_log` as `mcp:<key name>`.

pub mod catalogue;
pub mod tools;

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Router,
};
use foreman_core::{auth as core_auth, repo, repo::mcp_policy::McpPolicy};
use serde_json::{json, Map, Value};
use tracing::{info, warn};

use crate::{
    auth::{AuthCtx, AuthKind},
    state::AppState,
};
use catalogue::{Class, ToolDef};

pub const PATH: &str = "/mcp";
pub const MODERN: &str = "2026-07-28";
pub const LEGACY: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const RATE_WINDOW: Duration = Duration::from_secs(60);
const INSTRUCTIONS: &str = "Foreman runs a fleet of 3D printers. Start with get_fleet_status. \
    Library files have ids (list_library); check_file_compatibility says whether a file can \
    print on a printer. Tools that move a printer, start or cancel prints, delete data or \
    change spool counts are destructive and may be refused for this key. Never set \
    plate_confirm or confirm unless the user has just confirmed it.";

fn supported() -> Vec<&'static str> {
    std::iter::once(MODERN)
        .chain(LEGACY.iter().copied())
        .collect()
}

/// In-process router (all `/api` routes, no auth layer) plus per-key rate windows.
pub struct McpService {
    pub router: Router,
    windows: Mutex<HashMap<i64, VecDeque<Instant>>>,
}

impl McpService {
    pub fn new(router: Router) -> Arc<Self> {
        Arc::new(Self {
            router,
            windows: Mutex::new(HashMap::new()),
        })
    }

    /// `Err(retry_after)` when the key has used its `per_minute` requests.
    pub fn rate_check(&self, key: i64, per_minute: u32, now: Instant) -> Result<(), Duration> {
        let Ok(mut w) = self.windows.lock() else {
            return Ok(());
        };
        let q = w.entry(key).or_default();
        while q
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= RATE_WINDOW)
        {
            q.pop_front();
        }
        if q.len() >= per_minute as usize {
            let oldest = q.front().copied().unwrap_or(now);
            return Err(RATE_WINDOW.saturating_sub(now.saturating_duration_since(oldest)));
        }
        q.push_back(now);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

/// Why a key may not call a tool, or `None` when it may.
pub fn refusal(def: &ToolDef, p: &McpPolicy) -> Option<String> {
    let listed = match &p.tools {
        Some(list) => list.iter().any(|n| n == def.name),
        None => def.class == Class::Read,
    };
    if def.class == Class::Destructive && !p.allow_destructive {
        return Some(format!(
            "{} is a destructive tool and destructive tools are off for this API key. The \
             owner can enable them in Foreman → Settings → Security → MCP.",
            def.name
        ));
    }
    if !listed {
        return Some(format!(
            "{} is not on this API key's allow list. The owner can allow it in Foreman → \
             Settings → Security → MCP.",
            def.name
        ));
    }
    None
}

/// Tool names a key can call right now.
pub fn reachable(p: &McpPolicy) -> Vec<String> {
    catalogue::TOOLS
        .iter()
        .filter(|t| refusal(t, p).is_none())
        .map(|t| t.name.to_owned())
        .collect()
}

// ---------------------------------------------------------------------------
// HTTP plumbing
// ---------------------------------------------------------------------------

fn rpc_error(id: Option<&Value>, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut e = json!({ "code": code, "message": message });
    if let Some(d) = data {
        e["data"] = d;
    }
    let mut v = json!({ "jsonrpc": "2.0", "error": e });
    if let Some(id) = id {
        v["id"] = id.clone();
    }
    v
}

fn json_response(status: StatusCode, body: &Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn unauthorized(msg: &str) -> Response {
    let mut r = json_response(
        StatusCode::UNAUTHORIZED,
        &rpc_error(None, -32001, msg, None),
    );
    r.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        header::HeaderValue::from_static("Bearer realm=\"foreman\""),
    );
    r.extensions_mut()
        .insert(crate::auth::DenyReason(msg.to_owned()));
    r
}

fn header_str<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok()).map(str::trim)
}

/// API key from `Authorization: Bearer` or `X-Api-Key`.
fn api_key(h: &HeaderMap) -> Option<&str> {
    header_str(h, "x-api-key")
        .or_else(|| header_str(h, "authorization").and_then(|v| v.strip_prefix("Bearer ")))
        .map(str::trim)
        .filter(|k| !k.is_empty())
}

/// Browsers send `Origin`; it must name this server (DNS-rebinding guard).
fn origin_ok(h: &HeaderMap) -> bool {
    let Some(origin) = header_str(h, "origin") else {
        return true;
    };
    let Some(host) = header_str(h, "host") else {
        return false;
    };
    let authority = origin
        .split_once("://")
        .map(|(_, rest)| rest.trim_end_matches('/'))
        .unwrap_or("");
    authority.eq_ignore_ascii_case(host)
}

/// `GET` / `DELETE /mcp`: no standalone stream, no sessions.
pub async fn method_not_allowed() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [(header::ALLOW, "POST")],
        "MCP endpoint: POST only",
    )
        .into_response()
}

/// `POST /mcp`
pub async fn handle(
    State(state): State<AppState>,
    Extension(svc): Extension<Arc<McpService>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !origin_ok(&headers) {
        return json_response(
            StatusCode::FORBIDDEN,
            &rpc_error(None, -32600, "Origin not allowed", None),
        );
    }
    let pool = &state.foreman_db.pool;
    let Some(key) = api_key(&headers) else {
        return unauthorized("API key required (Authorization: Bearer <key> or X-Api-Key)");
    };
    let key_id = match core_auth::check_api_key(pool, key).await {
        Ok(Some(id)) => id,
        Ok(None) => return unauthorized("invalid API key"),
        Err(e) => {
            warn!("mcp: key check failed: {e}");
            return unauthorized("API key check failed");
        }
    };
    let key_name = core_auth::api_key_name(pool, key_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("key {key_id}"));
    let admin = core_auth::admin_username(pool).await.ok().flatten();
    if admin.is_none() {
        return json_response(
            StatusCode::FORBIDDEN,
            &rpc_error(None, -32001, "server not set up", None),
        );
    }
    let policy = match repo::mcp_policy::get(pool, key_id).await {
        Ok(p) => p,
        Err(e) => {
            warn!("mcp: policy lookup failed: {e}");
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rpc_error(None, -32603, "policy lookup failed", None),
            );
        }
    };
    if let Err(wait) = svc.rate_check(key_id, policy.rate_per_minute, Instant::now()) {
        let secs = wait.as_secs().max(1);
        let mut r = json_response(
            StatusCode::TOO_MANY_REQUESTS,
            &rpc_error(
                None,
                -32029,
                &format!(
                    "rate limit: {} requests per minute for this key; retry in {secs} s",
                    policy.rate_per_minute
                ),
                Some(json!({ "retry_after": secs })),
            ),
        );
        if let Ok(v) = header::HeaderValue::from_str(&secs.to_string()) {
            r.headers_mut().insert(header::RETRY_AFTER, v);
        }
        return r;
    }

    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &rpc_error(None, -32700, &format!("parse error: {e}"), None),
            )
        }
    };
    if msg.is_array() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &rpc_error(None, -32600, "JSON-RPC batches are not supported", None),
        );
    }
    let Some(obj) = msg.as_object() else {
        return json_response(
            StatusCode::BAD_REQUEST,
            &rpc_error(None, -32600, "expected a JSON-RPC object", None),
        );
    };
    let method = obj.get("method").and_then(Value::as_str);
    let id = obj.get("id").filter(|v| !v.is_null());
    // Notifications and responses from the client: accepted, nothing to say.
    let (Some(method), Some(id)) = (method, id) else {
        return StatusCode::ACCEPTED.into_response();
    };
    let params = obj
        .get("params")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    // Which protocol revision is this request speaking?
    let header_version = header_str(&headers, "mcp-protocol-version");
    let meta_version = params
        .get("_meta")
        .and_then(|m| m.get(META_VERSION))
        .and_then(Value::as_str);
    let modern =
        method != "initialize" && (header_version == Some(MODERN) || meta_version.is_some());
    if let Some(v) = header_version {
        if method != "initialize" && !supported().contains(&v) {
            return json_response(
                StatusCode::BAD_REQUEST,
                &rpc_error(
                    Some(id),
                    -32022,
                    &format!("unsupported protocol version {v}"),
                    Some(json!({ "supported": supported(), "requested": v })),
                ),
            );
        }
    }
    if modern {
        if let Err(m) =
            check_modern_headers(&headers, method, &params, header_version, meta_version)
        {
            return json_response(
                StatusCode::BAD_REQUEST,
                &rpc_error(Some(id), -32020, &m, None),
            );
        }
    }

    let ctx = AuthCtx {
        username: format!("mcp:{key_name}"),
        kind: AuthKind::ApiKey { id: key_id },
    };
    let out = dispatch(
        &state, &svc, &ctx, &key_name, &policy, method, &params, modern,
    )
    .await;
    match out {
        Ok(result) => json_response(
            StatusCode::OK,
            &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        ),
        Err((code, message, data)) => {
            let status = if modern && code == -32601 {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::OK
            };
            json_response(status, &rpc_error(Some(id), code, &message, data))
        }
    }
}

/// 2026-07-28: `MCP-Protocol-Version`, `Mcp-Method` and (for tools/call) `Mcp-Name` must be
/// present and match the body.
fn check_modern_headers(
    h: &HeaderMap,
    method: &str,
    params: &Map<String, Value>,
    header_version: Option<&str>,
    meta_version: Option<&str>,
) -> Result<(), String> {
    let Some(hv) = header_version else {
        return Err("Header mismatch: MCP-Protocol-Version header is required".into());
    };
    if let Some(mv) = meta_version {
        if mv != hv {
            return Err(format!(
                "Header mismatch: MCP-Protocol-Version header value '{hv}' does not match body value '{mv}'"
            ));
        }
    }
    match header_str(h, "mcp-method") {
        None => return Err("Header mismatch: Mcp-Method header is required".into()),
        Some(m) if m != method => {
            return Err(format!(
            "Header mismatch: Mcp-Method header value '{m}' does not match body value '{method}'"
        ))
        }
        Some(_) => {}
    }
    if method == "tools/call" {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        match header_str(h, "mcp-name") {
            None => return Err("Header mismatch: Mcp-Name header is required".into()),
            Some(n) if n != name => {
                return Err(format!(
                "Header mismatch: Mcp-Name header value '{n}' does not match body value '{name}'"
            ))
            }
            Some(_) => {}
        }
    }
    Ok(())
}

type RpcErr = (i64, String, Option<Value>);

fn server_info(state: &AppState) -> Value {
    json!({ "name": "foreman", "title": "Foreman", "version": state.info.version })
}

#[allow(clippy::too_many_arguments)]
async fn dispatch(
    state: &AppState,
    svc: &McpService,
    ctx: &AuthCtx,
    key_name: &str,
    policy: &McpPolicy,
    method: &str,
    params: &Map<String, Value>,
    modern: bool,
) -> Result<Value, RpcErr> {
    let caps = json!({ "tools": { "listChanged": false } });
    match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("");
            let version = if LEGACY.contains(&requested) {
                requested
            } else {
                LEGACY[0]
            };
            Ok(json!({
                "protocolVersion": version,
                "capabilities": caps,
                "serverInfo": server_info(state),
                "instructions": INSTRUCTIONS,
            }))
        }
        "server/discover" => Ok(json!({
            "resultType": "complete",
            "supportedVersions": supported(),
            "capabilities": caps,
            "instructions": INSTRUCTIONS,
            "ttlMs": 300_000,
            "cacheScope": "private",
            "_meta": { "io.modelcontextprotocol/serverInfo": server_info(state) },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => {
            let tools: Vec<Value> = catalogue::TOOLS.iter().map(ToolDef::to_mcp).collect();
            let mut r = json!({ "tools": tools });
            if modern {
                r["resultType"] = json!("complete");
                r["ttlMs"] = json!(300_000);
                r["cacheScope"] = json!("private");
            }
            Ok(r)
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let Some(def) = catalogue::find(name) else {
                return Err((-32602, format!("Unknown tool: {name}"), None));
            };
            let args = match params.get("arguments") {
                None | Some(Value::Null) => Map::new(),
                Some(Value::Object(m)) => m.clone(),
                Some(_) => return Err((-32602, "arguments must be an object".into(), None)),
            };
            let mut r = call_tool(state, svc, ctx, key_name, policy, def, &args).await;
            if modern {
                r["resultType"] = json!("complete");
            }
            Ok(r)
        }
        other => Err((-32601, format!("Method not found: {other}"), None)),
    }
}

fn text_result(v: &Value, is_error: bool) -> Value {
    let text = if let Value::String(s) = v {
        s.clone()
    } else {
        serde_json::to_string_pretty(v).unwrap_or_default()
    };
    let mut r = json!({ "content": [{ "type": "text", "text": text }], "isError": is_error });
    if !is_error {
        r["structuredContent"] = match v {
            Value::Object(_) => v.clone(),
            other => json!({ "result": other }),
        };
    }
    r
}

async fn audit(state: &AppState, key_name: &str, args: &Map<String, Value>, line: String) {
    let printer = args.get("printer_id").and_then(Value::as_i64);
    let user = format!("mcp:{key_name}");
    let pool = &state.foreman_db.pool;
    // An unknown printer id fails the foreign key; keep the line (the id is in the args).
    let mut r = repo::audit_log::insert(pool, printer, Some(&user), &line).await;
    if r.is_err() && printer.is_some() {
        r = repo::audit_log::insert(pool, None, Some(&user), &line).await;
    }
    if let Err(e) = r {
        warn!("mcp audit write failed: {e}");
    }
}

fn args_line(args: &Map<String, Value>) -> String {
    let s = Value::Object(args.clone()).to_string();
    if s.chars().count() > 400 {
        format!("{}…", s.chars().take(400).collect::<String>())
    } else {
        s
    }
}

async fn call_tool(
    state: &AppState,
    svc: &McpService,
    ctx: &AuthCtx,
    key_name: &str,
    policy: &McpPolicy,
    def: &ToolDef,
    args: &Map<String, Value>,
) -> Value {
    let audited = def.class != Class::Read;
    if let Some(why) = refusal(def, policy) {
        if audited {
            audit(
                state,
                key_name,
                args,
                format!(
                    "mcp {} {} → refused: not enabled for key",
                    def.name,
                    args_line(args)
                ),
            )
            .await;
        }
        info!(
            tool = def.name,
            key = key_name,
            "mcp call refused by key policy"
        );
        return text_result(&Value::String(why), true);
    }
    if let Err(why) = def.validate(args) {
        if audited {
            audit(
                state,
                key_name,
                args,
                format!("mcp {} {} → refused: {why}", def.name, args_line(args)),
            )
            .await;
        }
        return text_result(&Value::String(why), true);
    }
    let d = tools::Dispatcher {
        router: &svc.router,
        ctx,
    };
    let out = d.run(def.name, args).await;
    if audited {
        let outcome = match &out {
            Ok(_) => "ok".to_owned(),
            Err(e) => format!("error: {e}"),
        };
        audit(
            state,
            key_name,
            args,
            format!("mcp {} {} → {outcome}", def.name, args_line(args)),
        )
        .await;
        info!(tool = def.name, key = key_name, %outcome, "mcp call");
    }
    match out {
        Ok(v) => text_result(&v, false),
        Err(e) => text_result(&Value::String(e), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_window_slides() {
        let svc = McpService::new(Router::new());
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(svc.rate_check(1, 3, t0).is_ok());
        }
        let wait = svc
            .rate_check(1, 3, t0 + Duration::from_secs(10))
            .expect_err("limited");
        assert_eq!(wait, Duration::from_secs(50));
        assert!(svc.rate_check(2, 3, t0).is_ok(), "other keys unaffected");
        assert!(
            svc.rate_check(1, 3, t0 + RATE_WINDOW).is_ok(),
            "window moved"
        );
    }

    #[test]
    fn default_policy_is_read_only() {
        let p = McpPolicy::default();
        let r = reachable(&p);
        assert!(r.contains(&"get_fleet_status".to_owned()));
        assert!(!r.contains(&"pause_print".to_owned()));
        assert!(!r.contains(&"create_vendor".to_owned()));
        let pause = catalogue::find("pause_print").expect("tool");
        // On the allow list but destructive switch off: still refused.
        let listed = McpPolicy {
            tools: Some(vec!["pause_print".into()]),
            ..McpPolicy::default()
        };
        assert!(refusal(pause, &listed).is_some());
        let on = McpPolicy {
            allow_destructive: true,
            ..listed
        };
        assert!(refusal(pause, &on).is_none());
    }

    #[test]
    fn origin_guard() {
        let mut h = HeaderMap::new();
        assert!(origin_ok(&h));
        h.insert("host", "10.0.0.10:9090".parse().expect("hv"));
        h.insert("origin", "http://10.0.0.10:9090".parse().expect("hv"));
        assert!(origin_ok(&h));
        h.insert("origin", "http://evil.example".parse().expect("hv"));
        assert!(!origin_ok(&h));
    }
}
