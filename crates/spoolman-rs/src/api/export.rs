//! Export endpoints — mirrors `spoolman/api/v1/export.py`.
//!
//! Three paths, each accepting `?fmt=json` (default) or `?fmt=csv`.
//! Responses include a `Content-Disposition: attachment` header.

use axum::{
    extract::{Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    body::Body,
};
use serde::Deserialize;

use crate::{
    db::{
        filament::{self as db_filament, FilamentFilter},
        spool::{self as db_spool, SpoolFilter},
        vendor::{self as db_vendor, VendorFilter},
    },
    error::SpoolmanError,
    models::{FilamentResponse, SpoolResponse, VendorResponse},
    state::AppState,
};

// ---------------------------------------------------------------------------
// Query params
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub struct ExportQuery {
    /// Output format: `json` (default) or `csv`.
    #[serde(default)]
    pub fmt: Option<String>,
    /// Include archived spools (spools export only).
    #[serde(default)]
    pub allow_archived: Option<bool>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fmt_or_default(fmt: &Option<String>) -> &str {
    match fmt.as_deref() {
        Some("csv") => "csv",
        _ => "json",
    }
}

fn build_response(body: Vec<u8>, content_type: &'static str, filename: &str) -> Response {
    let disposition = format!("attachment; filename=\"{filename}\"");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, HeaderValue::from_static(content_type))
        .header(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&disposition).unwrap_or_else(|_| {
                HeaderValue::from_static("attachment")
            }),
        )
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

// ---------------------------------------------------------------------------
// CSV builders
// ---------------------------------------------------------------------------

fn escape_csv(v: &str) -> String {
    if v.contains(',') || v.contains('"') || v.contains('\n') {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_owned()
    }
}

fn opt_str(v: &Option<String>) -> String {
    v.as_deref().map(escape_csv).unwrap_or_default()
}

fn opt_f64(v: Option<f64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}

fn opt_i64(v: Option<i64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}

fn vendors_to_csv(rows: &[VendorResponse]) -> Vec<u8> {
    let mut out = String::from(
        "id,registered,name,comment,empty_spool_weight,external_id\n",
    );
    for v in rows {
        out.push_str(&format!(
            "{},{},{},{},{},{}\n",
            v.id,
            escape_csv(&v.registered),
            escape_csv(&v.name),
            opt_str(&v.comment),
            opt_f64(v.empty_spool_weight),
            opt_str(&v.external_id),
        ));
    }
    out.into_bytes()
}

fn filaments_to_csv(rows: &[FilamentResponse]) -> Vec<u8> {
    let mut out = String::from(
        "id,registered,name,vendor_id,vendor_name,material,price,density,diameter,\
         weight,spool_weight,article_number,comment,settings_extruder_temp,\
         settings_bed_temp,color_hex,multi_color_hexes,multi_color_direction,external_id\n",
    );
    for f in rows {
        let vendor_id = f.vendor.as_ref().map(|v| v.id.to_string()).unwrap_or_default();
        let vendor_name = f.vendor.as_ref().map(|v| escape_csv(&v.name)).unwrap_or_default();
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            f.id,
            escape_csv(&f.registered),
            opt_str(&f.name),
            vendor_id,
            vendor_name,
            opt_str(&f.material),
            opt_f64(f.price),
            f.density,
            f.diameter,
            opt_f64(f.weight),
            opt_f64(f.spool_weight),
            opt_str(&f.article_number),
            opt_str(&f.comment),
            opt_i64(f.settings_extruder_temp),
            opt_i64(f.settings_bed_temp),
            opt_str(&f.color_hex),
            opt_str(&f.multi_color_hexes),
            opt_str(&f.multi_color_direction),
            opt_str(&f.external_id),
        ));
    }
    out.into_bytes()
}

fn spools_to_csv(rows: &[SpoolResponse]) -> Vec<u8> {
    let mut out = String::from(
        "id,registered,first_used,last_used,filament_id,filament_name,filament_material,\
         used_weight,remaining_weight,location,lot_nr,comment,archived,price,\
         initial_weight,spool_weight\n",
    );
    for s in rows {
        let fil_name = s.filament.name.as_deref().map(escape_csv).unwrap_or_default();
        let fil_material = s.filament.material.as_deref().map(escape_csv).unwrap_or_default();
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            s.id,
            escape_csv(&s.registered),
            opt_str(&s.first_used),
            opt_str(&s.last_used),
            s.filament.id,
            fil_name,
            fil_material,
            s.used_weight,
            opt_f64(s.remaining_weight),
            opt_str(&s.location),
            opt_str(&s.lot_nr),
            opt_str(&s.comment),
            s.archived,
            opt_f64(s.price),
            opt_f64(s.initial_weight),
            opt_f64(s.spool_weight),
        ));
    }
    out.into_bytes()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET {base_path}/api/v1/export/spools?fmt=json|csv&allow_archived=bool
pub async fn export_spools(
    State(state): State<AppState>,
    Query(q): Query<ExportQuery>,
) -> Result<Response, (StatusCode, axum::Json<serde_json::Value>)> {
    let allow_archived = q.allow_archived.unwrap_or(false);
    let (rows, _) = db_spool::find(
        &state.db.pool,
        &state.db.engine,
        &SpoolFilter {
            allow_archived,
            limit: Some(1_000_000),
            ..Default::default()
        },
    )
    .await
    .map_err(map_err)?;

    let fmt = fmt_or_default(&q.fmt);
    if fmt == "csv" {
        Ok(build_response(spools_to_csv(&rows), "text/csv; charset=utf-8", "spools.csv"))
    } else {
        let body = serde_json::to_vec(&rows).map_err(|e| map_err(SpoolmanError::Json(e)))?;
        Ok(build_response(body, "application/json", "spools.json"))
    }
}

/// GET {base_path}/api/v1/export/filaments?fmt=json|csv
pub async fn export_filaments(
    State(state): State<AppState>,
    Query(q): Query<ExportQuery>,
) -> Result<Response, (StatusCode, axum::Json<serde_json::Value>)> {
    let (rows, _) = db_filament::find(
        &state.db.pool,
        &state.db.engine,
        &FilamentFilter {
            limit: Some(1_000_000),
            ..Default::default()
        },
    )
    .await
    .map_err(map_err)?;

    let fmt = fmt_or_default(&q.fmt);
    if fmt == "csv" {
        Ok(build_response(filaments_to_csv(&rows), "text/csv; charset=utf-8", "filaments.csv"))
    } else {
        let body = serde_json::to_vec(&rows).map_err(|e| map_err(SpoolmanError::Json(e)))?;
        Ok(build_response(body, "application/json", "filaments.json"))
    }
}

/// GET {base_path}/api/v1/export/vendors?fmt=json|csv
pub async fn export_vendors(
    State(state): State<AppState>,
    Query(q): Query<ExportQuery>,
) -> Result<Response, (StatusCode, axum::Json<serde_json::Value>)> {
    let (rows, _) = db_vendor::find(
        &state.db.pool,
        &state.db.engine,
        &VendorFilter {
            limit: Some(1_000_000),
            ..Default::default()
        },
    )
    .await
    .map_err(map_err)?;

    let fmt = fmt_or_default(&q.fmt);
    if fmt == "csv" {
        Ok(build_response(vendors_to_csv(&rows), "text/csv; charset=utf-8", "vendors.csv"))
    } else {
        let body = serde_json::to_vec(&rows).map_err(|e| map_err(SpoolmanError::Json(e)))?;
        Ok(build_response(body, "application/json", "vendors.json"))
    }
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

fn map_err(e: SpoolmanError) -> (StatusCode, axum::Json<serde_json::Value>) {
    let msg = e.to_string();
    tracing::error!("export error: {msg}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(serde_json::json!({ "message": msg })),
    )
}
