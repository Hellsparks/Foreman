//! Vendor database queries.

use serde_json::Map;
use sqlx::AnyPool;

use crate::{
    db::{
        extra_field,
        query::{parse_sort, Pagination, WhereBuilder},
    },
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
    /// Dynamic `extra.<key>=<value>` query params.
    pub extra_filters: Vec<(String, String)>,
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

    let base_where = wb.sql();

    // --- Extra-field handling ---
    // Split sort into regular and extra.* clauses.
    let raw_sort = parse_sort(filter.sort.as_deref(), "id:asc");
    let mut regular_sort_parts: Vec<String> = Vec::new();
    let mut extra_sort_items: Vec<(String, bool)> = Vec::new();
    let qualifications = &[
        ("id", "v.id"),
        ("registered", "v.registered"),
        ("name", "v.name"),
    ];
    for clause in &raw_sort {
        if let Some(key) = clause.column.strip_prefix("extra.") {
            extra_sort_items.push((key.to_owned(), clause.ascending));
        } else {
            let qualified = qualifications
                .iter()
                .find(|(bare, _)| *bare == clause.column)
                .map(|(_, q)| *q)
                .unwrap_or(&clause.column);
            let dir = if clause.ascending { "ASC" } else { "DESC" };
            regular_sort_parts.push(format!("({qualified} IS NULL) ASC, {qualified} {dir}"));
        }
    }

    let extra_sql = extra_field::build_extra_sql(
        pool,
        engine,
        "vendor",
        "v",
        "vendor_field",
        "vendor_id",
        &filter.extra_filters,
        &extra_sort_items,
    )
    .await?;

    // Combine WHERE conditions.
    let final_where = build_final_where(&base_where, &extra_sql.where_clauses);

    // Build ORDER BY: extra sort comes first (preserving original order), then regular.
    let mut all_order_parts = extra_sql.order_frags.clone();
    all_order_parts.extend(regular_sort_parts);
    let order_sql = if all_order_parts.is_empty() {
        String::new()
    } else {
        format!("ORDER BY {}", all_order_parts.join(", "))
    };

    let pag = Pagination::new(filter.limit, filter.offset);

    // Count
    let count_sql = format!("SELECT COUNT(*) FROM vendor v {final_where}");
    let mut count_q = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(&*count_sql));
    for b in &wb.binds {
        count_q = count_q.bind(b.clone());
    }
    for b in &extra_sql.where_binds {
        count_q = count_q.bind(b.clone());
    }
    let total: i64 = count_q.fetch_one(pool).await?;

    // Data — CAST(datetime AS TEXT) so AnyPool can decode as String.
    let data_sql = format!(
        "SELECT v.id, CAST(v.registered AS TEXT) AS registered, v.name, v.comment, \
         v.empty_spool_weight, v.external_id \
         FROM vendor v {final_where} {order_sql} LIMIT ? OFFSET ?"
    );
    let mut q = sqlx::query(sqlx::AssertSqlSafe(&*data_sql));
    for b in &wb.binds {
        q = q.bind(b.clone());
    }
    for b in &extra_sql.where_binds {
        q = q.bind(b.clone());
    }
    for b in &extra_sql.order_binds {
        q = q.bind(b.clone());
    }
    q = q.bind(pag.limit).bind(pag.offset);
    let raw_rows = q.fetch_all(pool).await?;
    let mut rows = Vec::with_capacity(raw_rows.len());
    for r in &raw_rows {
        rows.push(VendorRow::from_row(r)?.into_response());
    }

    // Populate extra fields in one batch query.
    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let mut extras = extra_field::get_values_batch(pool, "vendor_field", "vendor_id", &ids).await?;
    for r in &mut rows {
        if let Some(e) = extras.remove(&r.id) {
            r.extra = e;
        }
    }

    Ok((rows, total))
}

/// Combine the base WHERE SQL with extra filter clauses.
pub(crate) fn build_final_where(base_where: &str, extra_clauses: &[String]) -> String {
    if extra_clauses.is_empty() {
        return base_where.to_owned();
    }
    let extra_part = extra_clauses.join(" AND ");
    if base_where.is_empty() {
        format!("WHERE {extra_part}")
    } else {
        format!("{base_where} AND {extra_part}")
    }
}

pub async fn find_by_id(pool: &AnyPool, id: i64) -> Result<VendorResponse> {
    let raw = sqlx::query(
        "SELECT id, CAST(registered AS TEXT) AS registered, name, comment, \
         empty_spool_weight, external_id FROM vendor WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| SpoolmanError::NotFound(format!("vendor with id {id}")))?;

    let mut resp = VendorRow::from_row(&raw).map(VendorRow::into_response)?;
    resp.extra = extra_field::get_values(pool, "vendor_field", "vendor_id", id).await?;
    Ok(resp)
}

pub async fn create(
    pool: &AnyPool,
    engine: &Engine,
    req: &CreateVendorRequest,
) -> Result<VendorResponse> {
    let now = utc_now_str();

    let id: i64 = if !matches!(engine, Engine::Mysql) {
        // SQLite (3.35+) and Postgres both support RETURNING id.
        sqlx::query_scalar(
            "INSERT INTO vendor (registered, name, comment, empty_spool_weight, external_id) \
             VALUES (?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(&now)
        .bind(&req.name)
        .bind(&req.comment)
        .bind(req.empty_spool_weight)
        .bind(&req.external_id)
        .fetch_one(pool)
        .await?
    } else {
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
        id
    };

    // Validate and store extra fields if any.
    if !req.extra.is_empty() {
        extra_field::validate_keys(pool, "vendor", &req.extra, true).await?;
        extra_field::set_values(pool, "vendor_field", "vendor_id", id, &req.extra).await?;
    }

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

    if sets.is_empty() && req.extra.is_none() {
        // Nothing to update — return current state.
        return find_by_id(pool, id).await;
    }

    if !sets.is_empty() {
        let set_sql = sets.join(", ");
        let sql = format!("UPDATE vendor SET {set_sql} WHERE id = ?");
        let mut q = sqlx::query(sqlx::AssertSqlSafe(&*sql));
        for b in binds {
            q = match b {
                Bind::Str(s) => q.bind(s),
                Bind::F64(f) => q.bind(f),
            };
        }
        q = q.bind(id);

        let result = q.execute(pool).await?;
        if result.rows_affected() == 0 {
            return Err(SpoolmanError::NotFound(format!("vendor with id {id}")));
        }
    }

    // Replace extra fields if provided.
    if let Some(extra) = &req.extra {
        extra_field::validate_keys(pool, "vendor", extra, false).await?;
        extra_field::set_values(pool, "vendor_field", "vendor_id", id, extra).await?;
    }

    find_by_id(pool, id).await
}

pub async fn delete(pool: &AnyPool, id: i64) -> Result<()> {
    // Remove extra field values first; vendor_field.vendor_id has no ON DELETE CASCADE.
    sqlx::query("DELETE FROM vendor_field WHERE vendor_id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    let result = sqlx::query("DELETE FROM vendor WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err.kind() == sqlx::error::ErrorKind::ForeignKeyViolation {
                    return SpoolmanError::DeleteConflict(
                        "Failed to delete vendor, see server logs for more information."
                            .to_owned(),
                    );
                }
            }
            SpoolmanError::Database(e)
        })?;
    if result.rows_affected() == 0 {
        return Err(SpoolmanError::NotFound(format!("vendor with id {id}")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers (also used by filament.rs / spool.rs)
// ---------------------------------------------------------------------------

/// Normalise a datetime string to ISO 8601 UTC.
///
/// Handles three input forms:
/// 1. RFC 3339 with timezone offset (e.g. `"2023-01-02T12:00:00+01:00"`) → converted to UTC
/// 2. RFC 3339 with Z (already UTC) → reformatted uniformly
/// 3. SQLite space-separated naive datetime (e.g. `"2023-01-02 11:00:00"`) → T separator added
pub fn normalise_dt(s: &str) -> String {
    // Try RFC 3339 parse — handles "+HH:MM" offsets, "+00:00", and "Z".
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        let utc = dt.with_timezone(&chrono::Utc);
        // Preserve sub-second precision if present.
        if utc.timestamp_subsec_nanos() != 0 {
            return utc.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
        }
        return utc.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    }
    // Space-separated naive datetime → ISO 8601 with T separator.
    if !s.contains('T') {
        return s.replacen(' ', "T", 1);
    }
    // Already has T but not RFC 3339 parseable — return as-is.
    s.to_owned()
}

/// Current UTC time as ISO 8601 string with explicit UTC offset.
pub fn utc_now_str() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.6f+00:00")
        .to_string()
}

/// Normalise a datetime bound string to UTC for use in SQL comparisons.
///
/// Converts timezone offsets to UTC so that filter bounds like
/// `"2023-06-01T02:00:00+02:00"` compare correctly against UTC-stored values.
/// Returns the input unchanged if it cannot be parsed as RFC 3339.
pub fn normalise_dt_bound(s: &str) -> String {
    normalise_dt(s)
}
