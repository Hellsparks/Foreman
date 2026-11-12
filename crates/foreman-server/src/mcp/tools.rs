//! Tool execution: each MCP tool is one or more calls to Foreman's own REST routes,
//! dispatched in-process to the router (no auth layer, the MCP key's identity attached),
//! so MCP gets exactly the checks the app gets: compat, plate gate, spool guard, audit.

use axum::{
    body::Body,
    http::{header, Method, Request},
    Router,
};
use serde_json::{json, Map, Value};
use tower::ServiceExt as _;

use crate::auth::AuthCtx;

/// Max internal response body (Library listings and history can be large).
const MAX_BODY: usize = 16 * 1024 * 1024;

/// Keys never shown to a model (printer access codes, API keys, …).
const SECRET_KEYS: &[&str] = &[
    "api_key",
    "access_code",
    "password",
    "secret",
    "token",
    "tls_fingerprint",
    "private_key",
    "key_hash",
    "confirm_token",
];

/// Drop secret-looking keys everywhere in `v`.
pub fn redact(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.retain(|k, _| !SECRET_KEYS.contains(&k.as_str()));
            for x in m.values_mut() {
                redact(x);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(redact),
        _ => {}
    }
}

fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `path?k=v&…` from the present, non-null args named in `keys` (`(arg, query name)`).
fn with_query(path: &str, args: &Map<String, Value>, keys: &[(&str, &str)]) -> String {
    let q: Vec<String> = keys
        .iter()
        .filter_map(|(a, name)| {
            let v = args.get(*a)?;
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => return None,
            };
            Some(format!("{name}={}", pct(&s)))
        })
        .collect();
    if q.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{}", q.join("&"))
    }
}

/// Copy the named args (that are present) into a JSON object.
fn pick(args: &Map<String, Value>, keys: &[&str]) -> Value {
    let mut m = Map::new();
    for k in keys {
        if let Some(v) = args.get(*k).filter(|v| !v.is_null()) {
            m.insert((*k).to_owned(), v.clone());
        }
    }
    Value::Object(m)
}

fn int(args: &Map<String, Value>, k: &str) -> Result<i64, String> {
    args.get(k)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("missing integer argument \"{k}\""))
}

/// Calls into the in-process router as the MCP key.
pub struct Dispatcher<'a> {
    pub router: &'a Router,
    pub ctx: &'a AuthCtx,
}

impl Dispatcher<'_> {
    /// One internal request; `Ok(json)` on 2xx, `Err(message)` otherwise.
    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> Result<Value, String> {
        let mut req = Request::builder().method(method).uri(uri);
        let req = match body {
            Some(b) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                req.body(Body::from(b.to_string()))
            }
            None => req.body(Body::empty()),
        };
        let mut req = req.map_err(|e| format!("bad internal request: {e}"))?;
        req.extensions_mut().insert(self.ctx.clone());
        let res = self
            .router
            .clone()
            .oneshot(req)
            .await
            .map_err(|e| format!("internal call failed: {e}"))?;
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), MAX_BODY)
            .await
            .map_err(|e| format!("reading response: {e}"))?;
        let v: Value = if bytes.is_empty() {
            json!({ "ok": true })
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        if status.is_success() {
            return Ok(v);
        }
        let msg = v
            .get("error")
            .or_else(|| v.get("message"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| match &v {
                Value::String(s) if !s.is_empty() => s.clone(),
                _ => status.canonical_reason().unwrap_or("error").to_owned(),
            });
        Err(format!("{} ({})", msg, status.as_u16()))
    }

    async fn get(&self, uri: &str) -> Result<Value, String> {
        self.call(Method::GET, uri, None).await
    }

    async fn post(&self, uri: &str, body: Value) -> Result<Value, String> {
        self.call(Method::POST, uri, Some(body)).await
    }

    async fn command(&self, printer: i64, kind: &str, body: Value) -> Result<Value, String> {
        self.post(&format!("/api/printers/{printer}/commands/{kind}"), body)
            .await?;
        Ok(json!({ "ok": true, "printer_id": printer, "command": kind }))
    }

    /// Run a validated tool call.
    pub async fn run(&self, name: &str, a: &Map<String, Value>) -> Result<Value, String> {
        let mut out = match name {
            "get_fleet_status" => self.fleet_status().await?,
            "list_printers" => {
                let mut v = self.get("/api/printers").await?;
                if let Some(arr) = v.as_array_mut() {
                    for p in arr.iter_mut() {
                        let state = p
                            .pointer("/status/state")
                            .cloned()
                            .unwrap_or(json!("offline"));
                        if let Some(o) = p.as_object_mut() {
                            o.remove("status");
                            o.insert("state".into(), state);
                        }
                    }
                }
                json!({ "printers": v })
            }
            "get_printer_status" => {
                let id = int(a, "printer_id")?;
                self.get(&format!("/api/printers/{id}/status")).await?
            }
            "get_printer_capabilities" => {
                let id = int(a, "printer_id")?;
                self.get(&format!("/api/printers/{id}/capabilities"))
                    .await?
            }
            "list_attention" => json!({ "items": self.get("/api/attention").await? }),
            "list_library" => {
                let mut b = Map::new();
                if let Some(f) = a.get("folder") {
                    b.insert("folder".into(), f.clone());
                }
                self.get(&with_query("/api/library", &b, &[("folder", "path")]))
                    .await?
            }
            "get_library_file" => {
                let id = int(a, "file_id")?;
                self.get(&format!("/api/library/files/{id}")).await?
            }
            "check_file_compatibility" => {
                let id = int(a, "file_id")?;
                let mut v = self.get(&format!("/api/send/check/{id}")).await?;
                if let (Some(pid), Some(arr)) = (
                    a.get("printer_id").and_then(Value::as_i64),
                    v.as_array_mut(),
                ) {
                    arr.retain(|r| r.get("printer_id").and_then(Value::as_i64) == Some(pid));
                }
                json!({ "file_id": id, "printers": v })
            }
            "get_queue" => self.get("/api/queue").await?,
            "print_file" => {
                let mut b = pick(
                    a,
                    &[
                        "file_id",
                        "printer_id",
                        "plate_confirm",
                        "dry_run",
                        "accept_manual_swaps",
                    ],
                );
                b["mode"] = json!("print_now");
                self.post("/api/send", b).await?
            }
            "add_to_queue" => {
                let mut b = pick(a, &["file_id", "printer_id", "accept_manual_swaps"]);
                b["mode"] = json!(if a.get("printer_id").is_some_and(|v| !v.is_null()) {
                    "queue"
                } else {
                    "fleet_queue"
                });
                self.post("/api/send", b).await?
            }
            "remove_from_queue" => {
                let id = int(a, "queue_item_id")?;
                self.call(Method::DELETE, &format!("/api/queue/{id}"), None)
                    .await?
            }
            "start_next_in_queue" => {
                let id = int(a, "printer_id")?;
                self.post(
                    &format!("/api/printers/{id}/queue/start-next"),
                    pick(a, &["plate_confirm", "dry_run"]),
                )
                .await?
            }
            "pause_print" => {
                self.command(int(a, "printer_id")?, "pause", json!({}))
                    .await?
            }
            "resume_print" => {
                self.command(int(a, "printer_id")?, "resume", json!({}))
                    .await?
            }
            "cancel_print" => {
                let id = int(a, "printer_id")?;
                let t = self
                    .post(&format!("/api/printers/{id}/confirm-cancel"), json!({}))
                    .await?;
                let token = t.get("confirm_token").cloned().unwrap_or(Value::Null);
                self.command(id, "cancel", json!({ "confirm_token": token }))
                    .await?
            }
            "set_temperature" => {
                let id = int(a, "printer_id")?;
                let kind = match a.get("heater").and_then(Value::as_str) {
                    Some("bed") => "set_bed_temp",
                    _ => "set_nozzle_temp",
                };
                self.command(id, kind, json!({ "target": a.get("target") }))
                    .await?
            }
            "home_axes" => {
                let id = int(a, "printer_id")?;
                self.command(
                    id,
                    "home",
                    json!({ "axes": a.get("axes").cloned().unwrap_or(json!([])) }),
                )
                .await?
            }
            "run_macro" => {
                let id = int(a, "printer_id")?;
                self.command(id, "macro", pick(a, &["name"])).await?
            }
            "send_gcode" => {
                let id = int(a, "printer_id")?;
                self.command(id, "gcode", pick(a, &["command"])).await?
            }
            "list_jobs" => {
                let mut q = a.clone();
                if !q.contains_key("limit") {
                    q.insert("limit".into(), json!(50));
                }
                if let Some(l) = q.get("limit").and_then(Value::as_i64) {
                    q.insert("limit".into(), json!(l.clamp(1, 500)));
                }
                self.get(&with_query(
                    "/api/jobs",
                    &q,
                    &[
                        ("printer_id", "printer_id"),
                        ("from", "from"),
                        ("to", "to"),
                        ("material", "material"),
                        ("spool_id", "spool_id"),
                        ("outcome", "outcome"),
                        ("search", "q"),
                        ("limit", "limit"),
                        ("offset", "offset"),
                    ],
                ))
                .await?
            }
            "get_job" => {
                self.get(&format!("/api/jobs/{}", int(a, "job_id")?))
                    .await?
            }
            "get_job_stats" => {
                self.get(&with_query(
                    "/api/jobs/stats",
                    a,
                    &[("from", "from"), ("to", "to")],
                ))
                .await?
            }
            "list_spools" => {
                let archived = a
                    .get("include_archived")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                json!({ "spools": self.get(&format!("/api/spools?archived={archived}")).await? })
            }
            "get_spool" => {
                let id = int(a, "spool_id")?;
                let all = self.get("/api/spools?archived=true").await?;
                all.as_array()
                    .and_then(|arr| {
                        arr.iter()
                            .find(|s| {
                                s.get("id").and_then(Value::as_i64) == Some(id)
                                    || s.pointer("/spool/id").and_then(Value::as_i64) == Some(id)
                            })
                            .cloned()
                    })
                    .ok_or_else(|| format!("spool {id} not found"))?
            }
            "get_loaded_spools" => match a.get("printer_id").and_then(Value::as_i64) {
                Some(id) => {
                    json!({ "slots": self.get(&format!("/api/printers/{id}/spools")).await? })
                }
                None => json!({ "slots": self.get("/api/spools/loaded").await? }),
            },
            "load_spool" => {
                let id = int(a, "printer_id")?;
                self.post(
                    &format!("/api/printers/{id}/spools/load"),
                    pick(a, &["tool", "spool_id", "override_reason"]),
                )
                .await?
            }
            "unload_spool" => {
                let id = int(a, "printer_id")?;
                self.post(
                    &format!("/api/printers/{id}/spools/unload"),
                    pick(a, &["tool"]),
                )
                .await?
            }
            "create_spool" => {
                self.post(
                    "/api/spools",
                    pick(
                        a,
                        &[
                            "filament_id",
                            "initial_weight",
                            "spool_weight",
                            "location",
                            "lot_nr",
                            "price",
                            "comment",
                            "quantity",
                        ],
                    ),
                )
                .await?
            }
            "update_spool" => {
                let id = int(a, "spool_id")?;
                self.call(
                    Method::PATCH,
                    &format!("/api/spools/{id}"),
                    Some(pick(
                        a,
                        &["location", "lot_nr", "comment", "price", "archived"],
                    )),
                )
                .await?
            }
            "list_filaments" => json!({ "filaments": self.get("/api/filaments").await? }),
            "get_filament" => {
                self.get(&format!("/api/filaments/{}", int(a, "filament_id")?))
                    .await?
            }
            "create_filament" => self.post("/api/filaments", filament_form(a)).await?,
            "update_filament" => {
                let id = int(a, "filament_id")?;
                self.call(
                    Method::PATCH,
                    &format!("/api/filaments/{id}"),
                    Some(filament_form(a)),
                )
                .await?
            }
            "list_vendors" => json!({ "vendors": self.get("/api/vendors").await? }),
            "create_vendor" => {
                self.post(
                    "/api/vendors",
                    pick(a, &["name", "empty_spool_weight", "website", "comment"]),
                )
                .await?
            }
            "update_vendor" => {
                let id = int(a, "vendor_id")?;
                let all = self.get("/api/vendors").await?;
                let cur = all
                    .as_array()
                    .and_then(|v| {
                        v.iter()
                            .find(|x| x.get("id").and_then(Value::as_i64) == Some(id))
                    })
                    .cloned()
                    .ok_or_else(|| format!("vendor {id} not found"))?;
                let mut form = pick(
                    &cur.as_object().cloned().unwrap_or_default(),
                    &["name", "empty_spool_weight", "website", "comment"],
                );
                if let (Some(f), Value::Object(upd)) = (
                    form.as_object_mut(),
                    pick(a, &["name", "empty_spool_weight", "website", "comment"]),
                ) {
                    f.extend(upd);
                }
                self.call(Method::PATCH, &format!("/api/vendors/{id}"), Some(form))
                    .await?
            }
            "get_inventory" => self.get("/api/inventory").await?,
            "list_spool_locations" => self.get("/api/spool-locations").await?,
            "list_projects" => json!({ "projects": self.get("/api/projects").await? }),
            "get_project" => {
                self.get(&format!("/api/projects/{}", int(a, "project_id")?))
                    .await?
            }
            "list_templates" => json!({ "templates": self.get("/api/templates").await? }),
            "print_project_remaining" => {
                let id = int(a, "project_id")?;
                self.post(
                    &format!("/api/projects/{id}/print-remaining"),
                    pick(a, &["accept_manual_swaps"]),
                )
                .await?
            }
            "get_maintenance" => {
                self.get(&with_query(
                    "/api/maintenance",
                    a,
                    &[("printer_id", "printer_id")],
                ))
                .await?
            }
            "get_maintenance_history" => json!({
                "entries": self
                    .get(&with_query(
                        "/api/maintenance/history",
                        a,
                        &[("printer_id", "printer_id"), ("task_id", "task_id")],
                    ))
                    .await?
            }),
            "log_maintenance_done" => {
                let id = int(a, "task_id")?;
                self.post(
                    &format!("/api/maintenance/tasks/{id}/done"),
                    pick(a, &["notes"]),
                )
                .await?
            }
            "snooze_maintenance" => {
                let id = int(a, "task_id")?;
                self.post(&format!("/api/maintenance/tasks/{id}/snooze"), json!({}))
                    .await?
            }
            other => return Err(format!("tool {other} has no implementation")),
        };
        redact(&mut out);
        Ok(out)
    }

    /// Compact live view of every printer plus open Attention items.
    async fn fleet_status(&self) -> Result<Value, String> {
        let printers = self.get("/api/printers").await?;
        let attention = self.get("/api/attention").await.unwrap_or(json!([]));
        let rows: Vec<Value> = printers
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|p| {
                        let s = p.get("status");
                        let field = |k: &str| s.and_then(|s| s.get(k)).cloned().unwrap_or(Value::Null);
                        let mut row = json!({
                            "id": p.get("id"),
                            "name": p.get("name"),
                            "firmware": p.get("firmware"),
                            "state": s.and_then(|s| s.get("state")).cloned().unwrap_or(json!("offline")),
                            "plate_clear": p.get("plate_clear"),
                            "file": field("filename"),
                            "progress_percent": s
                                .and_then(|s| s.get("progress"))
                                .and_then(Value::as_f64)
                                .map(|f| (f * 1000.0).round() / 10.0),
                            "remaining_seconds": field("remaining_time"),
                            "elapsed_seconds": field("print_duration"),
                            "layer": field("layer"),
                            "total_layers": field("total_layers"),
                            "nozzle": field("nozzle"),
                            "bed": field("bed"),
                            "message": field("print_message"),
                            "active_connection": p.get("active_path_name"),
                        });
                        if let Some(o) = row.as_object_mut() {
                            o.retain(|_, v| !v.is_null());
                        }
                        row
                    })
                    .collect()
            })
            .unwrap_or_default();
        let count = |st: &str| {
            rows.iter()
                .filter(|r| r.get("state").and_then(Value::as_str) == Some(st))
                .count()
        };
        Ok(json!({
            "summary": {
                "printers": rows.len(),
                "printing": count("printing"),
                "paused": count("paused"),
                "idle": count("idle"),
                "error": count("error"),
                "offline": count("offline"),
            },
            "printers": rows,
            "attention": attention,
        }))
    }
}

fn filament_form(a: &Map<String, Value>) -> Value {
    pick(
        a,
        &[
            "vendor_id",
            "vendor_name",
            "name",
            "material",
            "colour_kind",
            "colours",
            "modifiers",
            "extruder_temp",
            "bed_temp",
            "density",
            "diameter",
            "price",
            "url",
            "article_number",
            "comment",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secrets_deeply() {
        let mut v = json!({"printers":[{"name":"V0","api_key":"x","access_code":"1","nested":{"token":"t","ok":1}}]});
        redact(&mut v);
        assert_eq!(v, json!({"printers":[{"name":"V0","nested":{"ok":1}}]}));
    }

    #[test]
    fn query_encoding() {
        let mut a = Map::new();
        a.insert("folder".into(), json!("Voron parts/a b&c"));
        assert_eq!(
            with_query("/api/library", &a, &[("folder", "path")]),
            "/api/library?path=Voron%20parts%2Fa%20b%26c"
        );
        assert_eq!(with_query("/x", &Map::new(), &[("folder", "path")]), "/x");
    }
}
