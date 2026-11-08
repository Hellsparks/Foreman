//! Vendor database queries.

use serde_json::Map;
use sqlx::AnyPool;

use crate::{
    db::query::{order_by_sql, parse_sort, qualify_sort, Pagination, WhereBuilder},
    error::{Result, SpoolmanError},
    models::{CreateVendorRequest, UpdateVendorRequest, VendorResponse},
};

use super::Engine;

// ---------------------------------------------------------------------------
// Row type — manually decoded because AnyPool cannot decode DATETIME columns
// via the standard Decode<Any> path.  We use try_get_unchecked for datetime
// fields to bypass the type-info check; the actual stored value is always TEXT.
// ---------------------------------------------------------------------------

struct VendorRow {
    id: i64,
    registered: String,
    name: String,
    comment: Option<String>,
    empty_spool_weight: Option<f64>,
    external_id: Option<String>,
}

impl VendorRow {
    fn from_row(row: &sqlx::any::AnyRow) -> Result<Self> {
        use sqlx::Row as _;
        Ok(VendorRow {
            id: row.try_get("id")?,
            // DATETIME columns must be fetched unchecked — AnyPool rejects the
            // SQLite Datetime type info even when the stored value is TEXT.
            registered: row.try_get_unchecked("registered")?,
            name: row.try_get("name")?,
            comment: row.try_get("comment")?,
            empty_spool_weight: row.try_get("empty_spool_weight")?,
            external_id: row.try_get("external_id")?,
        })
    }

    fn into_response(self) -> VendorResponse {
        VendorResponse {
            id: self.id,
            registered: normalise_dt(&self.registered),
            name: self.name,
            comment: self.comment,
            empty_spool_weight: self.empty_spool_weight,
            external_id: self.external_id,
            extra: Map::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Filter params (mirrors upstream VendorParameters)
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct VendorFilter {
    pub name: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

pub async fn find(
    pool: &AnyPool,
    engine: &Engine,
    filter: &VendorFilter,
) -> Result<(Vec<VendorResponse>, i64)> {
    let mut wb = WhereBuilder::new();
    wb.add_str_opt("v.name", filter.name.as_deref(), engine);

    let where_sql = wb.sql();
    let raw_sort = parse_sort(filter.sort.as_deref(), "id:asc");
    let sort_clauses = qualify_sort(
        &raw_sort,
        &[
            ("id", "v.id"),
            ("registered", "v.registered"),
            ("name", "v.name"),
        ],
    );
    let order_sql = order_by_sql(&sort_clauses);
    let pag = Pagination::new(filter.limit, filter.offset);

    // Count
    let count_sql = format!("SELECT COUNT(*) FROM vendor v {where_sql}");
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);
    for b in &wb.binds {
        count_q = count_q.bind(b.clone());
    }
    let total: i64 = count_q.fetch_one(pool).await?;

    // Data — CAST(datetime AS TEXT) so AnyPool can decode as String.
    let data_sql = format!(
        "SELECT v.id, CAST(v.registered AS TEXT) AS registered, v.name, v.comment, \
         v.empty_spool_weight, v.external_id \
         FROM vendor v {where_sql} {order_sql} LIMIT ? OFFSET ?"
    );
    let mut q = sqlx::query(&data_sql);
    for b in &wb.binds {
        q = q.bind(b.clone());
    }
    q = q.bind(pag.limit).bind(pag.offset);
    let raw_rows = q.fetch_all(pool).await?;
    let mut rows = Vec::with_capacity(raw_rows.len());
    for r in raw_rows {
        rows.push(VendorRow::from_row(&r)?.into_response());
    }

    Ok((rows, total))
}

pub async fn find_by_id(pool: &AnyPool, id: i64) -> Result<VendorResponse> {
    let raw = sqlx::query(
        "SELECT id, CAST(registered AS TEXT) AS registered, name, comment, \
         empty_spool_weight, external_id FROM vendor WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| SpoolmanError::NotFound(format!("vendor {id}")))?;

    VendorRow::from_row(&raw).map(VendorRow::into_response)
}

pub async fn create(
    pool: &AnyPool,
    engine: &Engine,
    req: &CreateVendorRequest,
) -> Result<VendorResponse> {
    let now = utc_now_str();

    if !matches!(engine, Engine::Mysql) {
        // SQLite (3.35+) and Postgres both support RETURNING id.
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO vendor (registered, name, comment, empty_spool_weight, external_id) \
             VALUES (?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(&now)
        .bind(&req.name)
        .bind(&req.comment)
        .bind(req.empty_spool_weight)
        .bind(&req.external_id)
        .fetch_one(pool)
        .await?;
        return find_by_id(pool, id).await;
    }

    // MySQL: RETURNING is not supported; use LAST_INSERT_ID() inside a transaction.
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO vendor (registered, name, comment, empty_spool_weight, external_id) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&now)
    .bind(&req.name)
    .bind(&req.comment)
    .bind(req.empty_spool_weight)
    .bind(&req.external_id)
    .execute(&mut *tx)
    .await?;
    let id: i64 = sqlx::query_scalar("SELECT LAST_INSERT_ID()")
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    find_by_id(pool, id).await
}

pub async fn update(pool: &AnyPool, id: i64, req: &UpdateVendorRequest) -> Result<VendorResponse> {
    // Build a dynamic SET clause — only update fields that are Some.
    // Note: cannot distinguish "not sent" from "sent as null" with this model;
    // clearing nullable fields is deferred to session 7.
    enum Bind {
        Str(Option<String>),
        F64(Option<f64>),
    }

    let mut sets: Vec<&str> = Vec::new();
    let mut binds: Vec<Bind> = Vec::new();

    if let Some(name) = &req.name {
        sets.push("name = ?");
        binds.push(Bind::Str(Some(name.clone())));
    }
    if req.comment.is_some() {
        sets.push("comment = ?");
        binds.push(Bind::Str(req.comment.clone()));
    }
    if req.empty_spool_weight.is_some() {
        sets.push("empty_spool_weight = ?");
        binds.push(Bind::F64(req.empty_spool_weight));
    }
    if req.external_id.is_some() {
        sets.push("external_id = ?");
        binds.push(Bind::Str(req.external_id.clone()));
    }

    if sets.is_empty() {
        // Nothing to update — return current state.
        return find_by_id(pool, id).await;
    }

    let set_sql = sets.join(", ");
    let sql = format!("UPDATE vendor SET {set_sql} WHERE id = ?");
    let mut q = sqlx::query(&sql);
    for b in binds {
        q = match b {
            Bind::Str(s) => q.bind(s),
            Bind::F64(f) => q.bind(f),
        };
    }
    q = q.bind(id);

    let result = q.execute(pool).await?;
    if result.rows_affected() == 0 {
        return Err(SpoolmanError::NotFound(format!("vendor {id}")));
    }
    find_by_id(pool, id).await
}

pub async fn delete(pool: &AnyPool, id: i64) -> Result<()> {
    let result = sqlx::query("DELETE FROM vendor WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(SpoolmanError::NotFound(format!("vendor {id}")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers (also used by filament.rs / spool.rs)
// ---------------------------------------------------------------------------

/// Normalise a SQLite "YYYY-MM-DD HH:MM:SS" datetime to ISO 8601 with T separator.
pub fn normalise_dt(s: &str) -> String {
    if s.contains('T') {
        s.to_owned()
    } else {
        s.replacen(' ', "T", 1)
    }
}

/// Current UTC time as ISO 8601 string.
pub fn utc_now_str() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.6f")
        .to_string()
}
