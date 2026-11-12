//! MCP endpoint tests (6.5): protocol shape, auth, key policy, plate gate, audit, rate limit.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use foreman_core::{auth as core_auth, repo};
use serde_json::{json, Value};
use tower::ServiceExt as _;

use crate::{auth_api_tests::app, mcp::catalogue, state::AppState};

struct Resp {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Value,
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Resp {
    let mut req = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .expect("request");
    let res = app.clone().oneshot(req).await.expect("response");
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    Resp {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

/// A legacy-style (2025-11-25) JSON-RPC request with an API key.
async fn rpc(app: &Router, key: &str, method: &str, params: Value) -> Resp {
    send(
        app,
        "POST",
        "/mcp",
        &[
            ("authorization", &format!("Bearer {key}")),
            ("accept", "application/json, text/event-stream"),
            ("mcp-protocol-version", "2025-11-25"),
        ],
        Some(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })),
    )
    .await
}

async fn call_tool(app: &Router, key: &str, name: &str, args: Value) -> Value {
    let r = rpc(
        app,
        key,
        "tools/call",
        json!({ "name": name, "arguments": args }),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body["result"].clone()
}

fn result_text(r: &Value) -> String {
    r["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

async fn setup() -> (tempfile::TempDir, Router, AppState, String, i64) {
    let dir = tempfile::tempdir().expect("dir");
    let (app, state) = app(&dir, true).await;
    let (key, secret) = core_auth::create_api_key(&state.foreman_db.pool, "Claude Desktop")
        .await
        .expect("key");
    (dir, app, state, secret, key.id)
}

async fn session(app: &Router) -> String {
    let r = send(
        app,
        "POST",
        "/api/auth/login",
        &[],
        Some(json!({ "username": "admin", "password": crate::auth_api_tests::PW, "return_token": true })),
    )
    .await;
    r.body["token"].as_str().expect("token").to_owned()
}

async fn set_policy(app: &Router, key_id: i64, policy: Value) -> Resp {
    let token = session(app).await;
    send(
        app,
        "PUT",
        &format!("/api/settings/mcp/keys/{key_id}"),
        &[("authorization", &format!("Bearer {token}"))],
        Some(policy),
    )
    .await
}

async fn audit_lines(state: &AppState) -> Vec<(Option<String>, String)> {
    repo::audit_log::recent(&state.foreman_db.pool, 50)
        .await
        .expect("audit")
        .into_iter()
        .map(|e| (e.user, e.command))
        .collect()
}

#[tokio::test]
async fn rejects_missing_and_bad_keys() {
    let (_d, app, _s, _key, _) = setup().await;
    let r = send(
        &app,
        "POST",
        "/mcp",
        &[],
        Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(r.headers.contains_key(header::WWW_AUTHENTICATE));
    let r = rpc(&app, "fmk_not_a_key", "tools/list", json!({})).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    // A session token is not an API key.
    let token = session(&app).await;
    let r = rpc(&app, &token, "tools/list", json!({})).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    // GET / DELETE: no stream, no sessions.
    let r = send(&app, "GET", "/mcp", &[], None).await;
    assert_eq!(r.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn legacy_handshake_and_tool_listing() {
    let (_d, app, _s, key, _) = setup().await;
    let r = rpc(
        &app,
        &key,
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } }),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let res = &r.body["result"];
    assert_eq!(res["protocolVersion"], "2025-06-18");
    assert_eq!(res["serverInfo"]["name"], "foreman");
    assert!(res["capabilities"]["tools"].is_object());
    assert!(r.headers.get("mcp-session-id").is_none(), "stateless");

    // Notifications get 202 and no body.
    let n = send(
        &app,
        "POST",
        "/mcp",
        &[("x-api-key", &key)],
        Some(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })),
    )
    .await;
    assert_eq!(n.status, StatusCode::ACCEPTED);

    let r = rpc(&app, &key, "tools/list", json!({})).await;
    let tools = r.body["result"]["tools"].as_array().expect("tools");
    assert_eq!(tools.len(), catalogue::TOOLS.len());
    for t in tools {
        let name = t["name"].as_str().expect("name");
        assert!(
            !t["description"].as_str().unwrap_or_default().is_empty(),
            "{name}"
        );
        assert_eq!(t["inputSchema"]["type"], "object", "{name}");
        assert!(t["inputSchema"]["properties"].is_object(), "{name}");
        let def = catalogue::find(name).expect("in catalogue");
        assert_eq!(
            t["annotations"]["readOnlyHint"],
            json!(def.class == catalogue::Class::Read)
        );
        assert_eq!(
            t["annotations"]["destructiveHint"],
            json!(def.class == catalogue::Class::Destructive)
        );
    }
    let print = tools
        .iter()
        .find(|t| t["name"] == "print_file")
        .expect("print_file");
    assert_eq!(
        print["inputSchema"]["properties"]["plate_confirm"],
        json!({ "type": "boolean", "const": true, "description": print["inputSchema"]["properties"]["plate_confirm"]["description"] })
    );
    assert!(print["inputSchema"]["required"]
        .as_array()
        .expect("req")
        .contains(&json!("plate_confirm")));

    // Unknown method / unknown tool are JSON-RPC errors.
    let r = rpc(&app, &key, "resources/list", json!({})).await;
    assert_eq!(r.body["error"]["code"], -32601);
    let r = rpc(&app, &key, "tools/call", json!({ "name": "nope" })).await;
    assert_eq!(r.body["error"]["code"], -32602);
}

#[tokio::test]
async fn modern_revision_headers_and_discover() {
    let (_d, app, _s, key, _) = setup().await;
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    let post = |method: &'static str, extra: Vec<(&'static str, &'static str)>, params: Value| {
        let app = app.clone();
        let key = key.clone();
        async move {
            let auth = format!("Bearer {key}");
            let mut h: Vec<(&str, &str)> = vec![
                ("authorization", auth.as_str()),
                ("accept", "application/json, text/event-stream"),
                ("mcp-protocol-version", "2026-07-28"),
            ];
            h.extend(extra);
            send(
                &app,
                "POST",
                "/mcp",
                &h,
                Some(json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params })),
            )
            .await
        }
    };
    let r = post(
        "server/discover",
        vec![("mcp-method", "server/discover")],
        json!({ "_meta": meta }),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["result"]["resultType"], "complete");
    assert!(r.body["result"]["supportedVersions"]
        .as_array()
        .expect("versions")
        .contains(&json!("2026-07-28")));
    assert_eq!(
        r.body["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "foreman"
    );

    let r = post(
        "tools/list",
        vec![("mcp-method", "tools/list")],
        json!({ "_meta": meta }),
    )
    .await;
    assert_eq!(r.body["result"]["resultType"], "complete");
    assert!(r.body["result"]["ttlMs"].is_number());
    assert_eq!(r.body["result"]["cacheScope"], "private");
    assert_eq!(
        r.body["result"]["tools"].as_array().map(Vec::len),
        Some(catalogue::TOOLS.len())
    );

    // Missing Mcp-Method → 400 HeaderMismatch.
    let r = post("tools/list", vec![], json!({ "_meta": meta })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.body["error"]["code"], -32020);
    // Mcp-Name must match the tool.
    let r = post(
        "tools/call",
        vec![("mcp-method", "tools/call"), ("mcp-name", "list_printers")],
        json!({ "_meta": meta, "name": "get_fleet_status", "arguments": {} }),
    )
    .await;
    assert_eq!(r.body["error"]["code"], -32020);
    let r = post(
        "tools/call",
        vec![
            ("mcp-method", "tools/call"),
            ("mcp-name", "get_fleet_status"),
        ],
        json!({ "_meta": meta, "name": "get_fleet_status", "arguments": {} }),
    )
    .await;
    assert_eq!(r.body["result"]["resultType"], "complete");
    assert_eq!(r.body["result"]["isError"], false);
    // Unsupported version → 400 UnsupportedProtocolVersion.
    let r = send(
        &app,
        "POST",
        "/mcp",
        &[("x-api-key", &key), ("mcp-protocol-version", "1999-01-01")],
        Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.body["error"]["code"], -32022);
    assert_eq!(r.body["error"]["data"]["requested"], "1999-01-01");
}

#[tokio::test]
async fn read_only_tools_work_by_default() {
    let (_d, app, state, key, _) = setup().await;
    let r = call_tool(&app, &key, "get_fleet_status", json!({})).await;
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["summary"]["printers"], 0);
    let r = call_tool(&app, &key, "list_library", json!({})).await;
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["structuredContent"]["entries"].is_array());
    // Bad arguments are a tool error the model can read.
    let r = call_tool(&app, &key, "get_library_file", json!({ "file_id": "x" })).await;
    assert_eq!(r["isError"], true);
    assert!(result_text(&r).contains("file_id"));
    let r = call_tool(&app, &key, "list_library", json!({ "bogus": 1 })).await;
    assert_eq!(r["isError"], true);
    // Reads are not audited.
    assert!(audit_lines(&state).await.is_empty());
}

#[tokio::test]
async fn destructive_refused_until_enabled_and_audited() {
    let (_d, app, state, key, key_id) = setup().await;
    // Default key: destructive and write tools refused, and the refusal is audited.
    let r = call_tool(&app, &key, "pause_print", json!({ "printer_id": 1 })).await;
    assert_eq!(r["isError"], true);
    assert!(result_text(&r).contains("destructive"), "{r}");
    let r = call_tool(&app, &key, "create_vendor", json!({ "name": "X" })).await;
    assert_eq!(r["isError"], true);
    assert!(result_text(&r).contains("allow list"), "{r}");
    let lines = audit_lines(&state).await;
    assert!(lines
        .iter()
        .any(|(u, c)| u.as_deref() == Some("mcp:Claude Desktop")
            && c.starts_with("mcp pause_print")
            && c.contains("refused")));

    // An API key cannot change MCP access.
    let r = send(
        &app,
        "PUT",
        &format!("/api/settings/mcp/keys/{key_id}"),
        &[("x-api-key", &key)],
        Some(json!({ "tools": null, "allow_destructive": true, "rate_per_minute": 60 })),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // Unknown tool names are rejected.
    let r = set_policy(
        &app,
        key_id,
        json!({ "tools": ["nope"], "allow_destructive": true, "rate_per_minute": 60 }),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    // Listed but destructive switch off: still refused.
    let r = set_policy(
        &app,
        key_id,
        json!({ "tools": ["get_fleet_status", "pause_print", "print_file"], "allow_destructive": false, "rate_per_minute": 60 }),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let r = call_tool(&app, &key, "pause_print", json!({ "printer_id": 1 })).await;
    assert!(result_text(&r).contains("destructive"));

    // Enabled: the call goes through to the printer layer (printer 1 doesn't exist here).
    let r = set_policy(
        &app,
        key_id,
        json!({ "tools": ["get_fleet_status", "pause_print", "print_file"], "allow_destructive": true, "rate_per_minute": 60 }),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(
        r.body["reachable"],
        json!(["get_fleet_status", "print_file", "pause_print"])
    );
    let r = call_tool(&app, &key, "pause_print", json!({ "printer_id": 999 })).await;
    assert_eq!(r["isError"], true);
    assert!(!result_text(&r).contains("destructive"), "{r}");
    let lines = audit_lines(&state).await;
    let (user, cmd) = &lines[0];
    assert_eq!(user.as_deref(), Some("mcp:Claude Desktop"));
    assert!(
        cmd.starts_with("mcp pause_print {\"printer_id\":999} → error"),
        "{cmd}"
    );

    // Settings show the key's reachable tools.
    let token = session(&app).await;
    let s = send(
        &app,
        "GET",
        "/api/settings/mcp",
        &[("authorization", &format!("Bearer {token}"))],
        None,
    )
    .await;
    assert_eq!(s.body["path"], "/mcp");
    assert_eq!(s.body["keys"][0]["policy"]["allow_destructive"], true);
    assert_eq!(
        s.body["tools"].as_array().map(Vec::len),
        Some(catalogue::TOOLS.len())
    );
}

#[tokio::test]
async fn print_start_requires_plate_confirm() {
    let (_d, app, state, key, key_id) = setup().await;
    let r = set_policy(
        &app,
        key_id,
        json!({ "tools": ["print_file", "start_next_in_queue"], "allow_destructive": true, "rate_per_minute": 60 }),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    for (tool, args) in [
        ("print_file", json!({ "file_id": 1, "printer_id": 1 })),
        (
            "print_file",
            json!({ "file_id": 1, "printer_id": 1, "plate_confirm": false }),
        ),
        ("start_next_in_queue", json!({ "printer_id": 1 })),
    ] {
        let r = call_tool(&app, &key, tool, args).await;
        assert_eq!(r["isError"], true);
        assert!(result_text(&r).contains("plate_confirm"), "{r}");
    }
    let lines = audit_lines(&state).await;
    assert_eq!(lines.len(), 3);
    assert!(lines
        .iter()
        .all(|(_, c)| c.contains("refused: plate_confirm")));
    // With the confirmation the request reaches Send (and fails there: no such file).
    let r = call_tool(
        &app,
        &key,
        "print_file",
        json!({ "file_id": 424242, "printer_id": 1, "plate_confirm": true, "dry_run": true }),
    )
    .await;
    assert_eq!(r["isError"], true);
    assert!(!result_text(&r).contains("plate_confirm"), "{r}");
}

#[tokio::test]
async fn rate_limit_per_key() {
    let (_d, app, state, key, key_id) = setup().await;
    let r = set_policy(
        &app,
        key_id,
        json!({ "tools": null, "allow_destructive": false, "rate_per_minute": 3 }),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    for _ in 0..3 {
        assert_eq!(
            rpc(&app, &key, "tools/list", json!({})).await.status,
            StatusCode::OK
        );
    }
    let r = rpc(&app, &key, "tools/list", json!({})).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(r.headers.contains_key(header::RETRY_AFTER));
    // Another key has its own budget.
    let (_, other) = core_auth::create_api_key(&state.foreman_db.pool, "Other")
        .await
        .expect("key");
    assert_eq!(
        rpc(&app, &other, "tools/list", json!({})).await.status,
        StatusCode::OK
    );
}
