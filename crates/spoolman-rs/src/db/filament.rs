//! Filament database queries.

use serde_json::Map;
use sqlx::AnyPool;

use crate::{
    db::{
        extra_field,
        query::{parse_sort, Pagination, WhereBuilder},
        tag,
        vendor::{normalise_dt, utc_now_str},
        Engine,
    },
    error::{Result, SpoolmanError},
    models::{CreateFilamentRequest, FilamentResponse, UpdateFilamentRequest, VendorResponse},
};

// ---------------------------------------------------------------------------
// Row type — flat JOIN result, manually mapped to nested response
// ---------------------------------------------------------------------------

struct FilamentRow {
    id: i64,
    registered: String,
    name: Option<String>,
    vendor_id: Option<i64>,
    material: Option<String>,
    price: Option<f64>,
    density: f64,
    diameter: f64,
    weight: Option<f64>,
    spool_weight: Option<f64>,
    article_number: Option<String>,
    comment: Option<String>,
    settings_extruder_temp: Option<i64>,
    settings_bed_temp: Option<i64>,
    color_hex: Option<String>,
    multi_color_hexes: Option<String>,
    multi_color_direction: Option<String>,
    external_id: Option<String>,
    // Vendor columns (from LEFT JOIN)
    v_registered: Option<String>,
    v_name: Option<String>,
    v_comment: Option<String>,
    v_empty_spool_weight: Option<f64>,
    v_external_id: Option<String>,
}

impl FilamentRow {
    fn from_row(row: &sqlx::any::AnyRow) -> Result<Self> {
        use sqlx::Row as _;
        Ok(FilamentRow {
            id: row.try_get("id")?,
            // DATETIME columns use try_get_unchecked — AnyPool rejects SQLite's
            // Datetime type info even when the actual stored value is TEXT.
            registered: row.try_get_unchecked("registered")?,
            name: row.try_get("name")?,
            vendor_id: row.try_get("vendor_id")?,
            material: row.try_get("material")?,
            price: row.try_get("price")?,
            density: row.try_get("density")?,
            diameter: row.try_get("diameter")?,
            weight: row.try_get("weight")?,
            spool_weight: row.try_get("spool_weight")?,
            article_number: row.try_get("article_number")?,
            comment: row.try_get("comment")?,
            settings_extruder_temp: row.try_get("settings_extruder_temp")?,
            settings_bed_temp: row.try_get("settings_bed_temp")?,
            color_hex: row.try_get("color_hex")?,
            multi_color_hexes: row.try_get("multi_color_hexes")?,
            multi_color_direction: row.try_get("multi_color_direction")?,
            external_id: row.try_get("external_id")?,
            v_registered: row.try_get_unchecked("v_registered")?,
            v_name: row.try_get("v_name")?,
            v_comment: row.try_get("v_comment")?,
            v_empty_spool_weight: row.try_get("v_empty_spool_weight")?,
            v_external_id: row.try_get("v_external_id")?,
        })
    }

    fn into_response(self) -> FilamentResponse {
        let vendor = match (self.vendor_id, self.v_name) {
            (Some(vid), Some(vname)) => Some(VendorResponse {
                id: vid,
                registered: normalise_dt(self.v_registered.as_deref().unwrap_or("")),
                name: vname,
                comment: self.v_comment,
                empty_spool_weight: self.v_empty_spool_weight,
                external_id: self.v_external_id,
                extra: Map::new(),
            }),
            _ => None,
        };

        FilamentResponse {
            id: self.id,
            registered: normalise_dt(&self.registered),
            name: self.name,
            vendor,
            material: self.material,
            price: self.price,
            density: self.density,
            diameter: self.diameter,
            weight: self.weight,
            spool_weight: self.spool_weight,
            article_number: self.article_number,
            comment: self.comment,
            settings_extruder_temp: self.settings_extruder_temp,
            settings_bed_temp: self.settings_bed_temp,
            color_hex: self.color_hex,
            multi_color_hexes: self.multi_color_hexes,
            multi_color_direction: self.multi_color_direction,
            external_id: self.external_id,
            extra: Map::new(),
            tags: vec![],
        }
    }
}

// CAST(... AS TEXT) on all DATETIME columns so AnyPool can decode them as String.
const FILAMENT_SELECT: &str =
    "SELECT f.id, CAST(f.registered AS TEXT) AS registered, f.name, f.vendor_id, \
     f.material, f.price, f.density, f.diameter, f.weight, f.spool_weight, \
     f.article_number, f.comment, f.settings_extruder_temp, f.settings_bed_temp, \
     f.color_hex, f.multi_color_hexes, f.multi_color_direction, f.external_id, \
     CAST(v.registered AS TEXT) AS v_registered, v.name AS v_name, v.comment AS v_comment, \
     v.empty_spool_weight AS v_empty_spool_weight, v.external_id AS v_external_id \
     FROM filament f LEFT JOIN vendor v ON v.id = f.vendor_id";

// ---------------------------------------------------------------------------
// Filter params
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct FilamentFilter {
    pub name: Option<String>,
    pub vendor_id: Option<String>,
    pub vendor_name: Option<String>,
    pub material: Option<String>,
    pub article_number: Option<String>,
    pub external_id: Option<String>,
    /// Pre-computed list of IDs from colour-similarity search; `None` means no ID filter.
    pub ids: Option<Vec<i64>>,
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
    filter: &FilamentFilter,
) -> Result<(Vec<FilamentResponse>, i64)> {
    let mut wb = WhereBuilder::new();
    wb.add_str_opt("f.name", filter.name.as_deref(), engine);
    wb.add_int_opt("f.vendor_id", filter.vendor_id.as_deref());
    wb.add_str_required_opt("v.name", filter.vendor_name.as_deref(), engine);
    wb.add_str_opt("f.material", filter.material.as_deref(), engine);
    wb.add_str_opt("f.article_number", filter.article_number.as_deref(), engine);
    wb.add_str_opt("f.external_id", filter.external_id.as_deref(), engine);

    // Colour-similarity pre-filter: restrict to matched IDs.
    if let Some(ids) = &filter.ids {
        if ids.is_empty() {
            // No matches from colour search → return empty result set.
            return Ok((Vec::new(), 0));
        }
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        for id in ids {
            wb.binds.push(id.to_string());
        }
        wb.push_raw_clause(format!("f.id IN ({placeholders})"));
    }

    let base_where = wb.sql();

    // --- Extra-field handling ---
    let raw_sort = parse_sort(filter.sort.as_deref(), "id:asc");
    let mut regular_sort_parts: Vec<String> = Vec::new();
    let mut extra_sort_items: Vec<(String, bool)> = Vec::new();
    let qualifications: &[(&str, &str)] = &[
        ("id", "f.id"),
        ("registered", "f.registered"),
        ("name", "f.name"),
        ("vendor_id", "f.vendor_id"),
        ("material", "f.material"),
        ("price", "f.price"),
        ("density", "f.density"),
        ("diameter", "f.diameter"),
        ("weight", "f.weight"),
        ("spool_weight", "f.spool_weight"),
        ("article_number", "f.article_number"),
        ("comment", "f.comment"),
        ("settings_extruder_temp", "f.settings_extruder_temp"),
        ("settings_bed_temp", "f.settings_bed_temp"),
        ("color_hex", "f.color_hex"),
        ("vendor.id", "v.id"),
        ("vendor.registered", "v.registered"),
        ("vendor.name", "v.name"),
        ("vendor.comment", "v.comment"),
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
        "filament",
        "f",
        "filament_field",
        "filament_id",
        &filter.extra_filters,
        &extra_sort_items,
    )
    .await?;

    let final_where = crate::db::vendor::build_final_where(&base_where, &extra_sql.where_clauses);

    let mut all_order_parts = extra_sql.order_frags.clone();
    all_order_parts.extend(regular_sort_parts);
    let order_sql = if all_order_parts.is_empty() {
        String::new()
    } else {
        format!("ORDER BY {}", all_order_parts.join(", "))
    };

    let pag = Pagination::new(filter.limit, filter.offset);

    let count_sql = format!(
        "SELECT COUNT(*) FROM filament f LEFT JOIN vendor v ON v.id = f.vendor_id {final_where}"
    );
    let mut count_q = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(&*count_sql));
    for b in &wb.binds {
        count_q = count_q.bind(b.clone());
    }
    for b in &extra_sql.where_binds {
        count_q = count_q.bind(b.clone());
    }
    let total: i64 = count_q.fetch_one(pool).await?;

    let data_sql = format!("{FILAMENT_SELECT} {final_where} {order_sql} LIMIT ? OFFSET ?");
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
    let rows = q.fetch_all(pool).await?;

    let mut parsed_rows = Vec::with_capacity(rows.len());
    for row in &rows {
        parsed_rows.push(FilamentRow::from_row(row)?);
    }

    // Batch extra field + tag lookup.
    let f_ids: Vec<i64> = parsed_rows.iter().map(|r| r.id).collect();
    let v_ids: Vec<i64> = parsed_rows.iter().filter_map(|r| r.vendor_id).collect();

    let mut f_extras =
        extra_field::get_values_batch(pool, "filament_field", "filament_id", &f_ids).await?;
    let mut v_extras =
        extra_field::get_values_batch(pool, "vendor_field", "vendor_id", &v_ids).await?;
    let mut f_tags = tag::get_filament_tags_batch(pool, &f_ids).await?;

    let mut results = Vec::with_capacity(parsed_rows.len());
    for r in parsed_rows {
        let vid = r.vendor_id;
        let fid = r.id;
        let mut resp = r.into_response();
        if let Some(e) = f_extras.remove(&fid) {
            resp.extra = e;
        }
        if let (Some(vid), Some(ref mut v)) = (vid, resp.vendor.as_mut()) {
            if let Some(e) = v_extras.remove(&vid) {
                v.extra = e;
            }
        }
        resp.tags = f_tags.remove(&fid).unwrap_or_default();
        results.push(resp);
    }
    Ok((results, total))
}

/// Return the IDs of all filaments whose colour is within `similarity_threshold` of
/// `color_query_hex` (CIE94 delta-E).  Mirrors upstream `filament.find_by_color`.
pub async fn find_by_color(
    pool: &AnyPool,
    color_query_hex: &str,
    similarity_threshold: f64,
) -> Result<Vec<i64>> {
    let query_rgb = crate::math::hex_to_rgb(color_query_hex).ok_or_else(|| {
        SpoolmanError::BadRequest(format!("Invalid color hex: {color_query_hex}"))
    })?;
    let query_lab = crate::math::rgb_to_lab(query_rgb);

    let rows = sqlx::query("SELECT id, color_hex, multi_color_hexes FROM filament")
        .fetch_all(pool)
        .await?;

    let mut matched = Vec::new();
    for row in &rows {
        use sqlx::Row;
        let id: i64 = row.try_get("id")?;
        let color_hex: Option<String> = row.try_get("color_hex")?;
        let multi_color_hexes: Option<String> = row.try_get("multi_color_hexes")?;

        let raw_colors: Vec<&str> = if let Some(ref hex) = color_hex {
            vec![hex.as_str()]
        } else if let Some(ref hexes) = multi_color_hexes {
            hexes.split(',').collect()
        } else {
            continue;
        };

        for color in raw_colors {
            if let Some(rgb) = crate::math::hex_to_rgb(color) {
                let lab = crate::math::rgb_to_lab(rgb);
                if crate::math::delta_e(query_lab, lab) <= similarity_threshold {
                    matched.push(id);
                    break;
                }
            }
        }
    }
    Ok(matched)
}

pub async fn find_by_id(pool: &AnyPool, id: i64) -> Result<FilamentResponse> {
    let sql = format!("{FILAMENT_SELECT} WHERE f.id = ?");
    let row = sqlx::query(sqlx::AssertSqlSafe(&*sql))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| SpoolmanError::NotFound(format!("filament with id {id}")))?;

    let parsed = FilamentRow::from_row(&row)?;
    let vendor_id = parsed.vendor_id;
    let mut resp = parsed.into_response();
    resp.extra = extra_field::get_values(pool, "filament_field", "filament_id", id).await?;
    if let (Some(vid), Some(ref mut v)) = (vendor_id, resp.vendor.as_mut()) {
        v.extra = extra_field::get_values(pool, "vendor_field", "vendor_id", vid).await?;
    }
    resp.tags = tag::get_filament_tags(pool, id).await?;
    Ok(resp)
}

/// Validate multi-color fields in a filament create/update request.
///
/// Rules mirror upstream Spoolman's Pydantic validators:
/// - `color_hex` and `multi_color_hexes` are mutually exclusive.
/// - Each hex color must be 6 or 8 valid hex characters (optional leading '#').
/// - `multi_color_hexes` requires at least 2 colors.
/// - If `multi_color_hexes` is set, `multi_color_direction` must be set too.
/// - If `multi_color_direction` is set, `multi_color_hexes` must be set too.
fn validate_multi_color(
    color_hex: Option<&str>,
    multi_color_hexes: Option<&str>,
    multi_color_direction: Option<&str>,
) -> Result<()> {
    if color_hex.is_some() && multi_color_hexes.is_some() {
        return Err(SpoolmanError::ValidationError(
            "Cannot specify both color_hex and multi_color_hexes.".into(),
        ));
    }
    if let Some(hexes) = multi_color_hexes {
        // Validate each hex code.
        for raw in hexes.split(',') {
            let hex = raw.trim_start_matches('#').to_uppercase();
            if hex.is_empty()
                || !hex.chars().all(|c| c.is_ascii_hexdigit())
                || hex.len() != 6 && hex.len() != 8
            {
                return Err(SpoolmanError::ValidationError(
                    "Invalid character in color code or color code length.".into(),
                ));
            }
        }
        // Need at least 2 colors.
        if hexes.split(',').count() < 2 {
            return Err(SpoolmanError::ValidationError(
                "Must specify at least two colors in multi_color_hexes.".into(),
            ));
        }
        // Direction required when hexes present.
        if multi_color_direction.is_none() {
            return Err(SpoolmanError::ValidationError(
                "Multi-color filament must have multi_color_direction set.".into(),
            ));
        }
    } else if multi_color_direction.is_some() {
        // Direction without hexes.
        return Err(SpoolmanError::ValidationError(
            "Single-color filament must not have multi_color_direction set.".into(),
        ));
    }
    Ok(())
}

pub async fn create(
    pool: &AnyPool,
    engine: &Engine,
    req: &CreateFilamentRequest,
) -> Result<FilamentResponse> {
    validate_multi_color(
        req.color_hex.as_deref(),
        req.multi_color_hexes.as_deref(),
        req.multi_color_direction.as_deref(),
    )?;
    let now = utc_now_str();

    const INSERT_SQL: &str = "INSERT INTO filament \
         (registered, name, vendor_id, material, price, density, diameter, weight, \
          spool_weight, article_number, comment, settings_extruder_temp, \
          settings_bed_temp, color_hex, multi_color_hexes, multi_color_direction, external_id) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)";

    macro_rules! bind_all {
        ($q:expr) => {
            $q.bind(&now)
                .bind(&req.name)
                .bind(req.vendor_id)
                .bind(&req.material)
                .bind(req.price)
                .bind(req.density)
                .bind(req.diameter)
                .bind(req.weight)
                .bind(req.spool_weight)
                .bind(&req.article_number)
                .bind(&req.comment)
                .bind(req.settings_extruder_temp)
                .bind(req.settings_bed_temp)
                .bind(&req.color_hex)
                .bind(&req.multi_color_hexes)
                .bind(&req.multi_color_direction)
                .bind(&req.external_id)
        };
    }

    let id: i64 = if !matches!(engine, Engine::Mysql) {
        // SQLite (3.35+) and Postgres support RETURNING id.
        bind_all!(sqlx::query_scalar(sqlx::AssertSqlSafe(&*format!(
            "{INSERT_SQL} RETURNING id"
        ))))
        .fetch_one(pool)
        .await?
    } else {
        // MySQL: use transaction + LAST_INSERT_ID().
        let mut tx = pool.begin().await?;
        bind_all!(sqlx::query(INSERT_SQL)).execute(&mut *tx).await?;
        let id: i64 = sqlx::query_scalar("SELECT LAST_INSERT_ID()")
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        id
    };

    if !req.extra.is_empty() {
        extra_field::validate_keys(pool, "filament", &req.extra, true).await?;
        extra_field::set_values(pool, "filament_field", "filament_id", id, &req.extra).await?;
    }

    find_by_id(pool, id).await
}

pub async fn update(
    pool: &AnyPool,
    id: i64,
    req: &UpdateFilamentRequest,
) -> Result<FilamentResponse> {
    enum Bind {
        Str(Option<String>),
        I64(Option<i64>),
        F64(Option<f64>),
    }

    let mut sets: Vec<&str> = Vec::new();
    let mut binds: Vec<Bind> = Vec::new();

    macro_rules! maybe_str {
        ($field:expr, $col:literal) => {
            if $field.is_some() {
                sets.push(concat!($col, " = ?"));
                binds.push(Bind::Str($field.clone()));
            }
        };
    }
    macro_rules! maybe_i64 {
        ($field:expr, $col:literal) => {
            if $field.is_some() {
                sets.push(concat!($col, " = ?"));
                binds.push(Bind::I64($field));
            }
        };
    }
    macro_rules! maybe_f64 {
        ($field:expr, $col:literal) => {
            if $field.is_some() {
                sets.push(concat!($col, " = ?"));
                binds.push(Bind::F64($field));
            }
        };
    }

    maybe_str!(req.name, "name");
    maybe_i64!(req.vendor_id, "vendor_id");
    maybe_str!(req.material, "material");
    maybe_f64!(req.price, "price");
    maybe_f64!(req.density, "density");
    maybe_f64!(req.diameter, "diameter");
    maybe_f64!(req.weight, "weight");
    maybe_f64!(req.spool_weight, "spool_weight");
    maybe_str!(req.article_number, "article_number");
    maybe_str!(req.comment, "comment");
    maybe_i64!(req.settings_extruder_temp, "settings_extruder_temp");
    maybe_i64!(req.settings_bed_temp, "settings_bed_temp");
    maybe_str!(req.color_hex, "color_hex");
    maybe_str!(req.multi_color_hexes, "multi_color_hexes");
    maybe_str!(req.multi_color_direction, "multi_color_direction");
    maybe_str!(req.external_id, "external_id");

    if sets.is_empty() && req.extra.is_none() {
        return find_by_id(pool, id).await;
    }

    if !sets.is_empty() {
        let sql = format!("UPDATE filament SET {} WHERE id = ?", sets.join(", "));
        let mut q = sqlx::query(sqlx::AssertSqlSafe(&*sql));
        for b in binds {
            q = match b {
                Bind::Str(s) => q.bind(s),
                Bind::I64(i) => q.bind(i),
                Bind::F64(f) => q.bind(f),
            };
        }
        q = q.bind(id);

        let result = q.execute(pool).await?;
        if result.rows_affected() == 0 {
            return Err(SpoolmanError::NotFound(format!("filament with id {id}")));
        }
    }

    if let Some(extra) = &req.extra {
        extra_field::validate_keys(pool, "filament", extra, false).await?;
        extra_field::set_values(pool, "filament_field", "filament_id", id, extra).await?;
    }

    find_by_id(pool, id).await
}

pub async fn delete(pool: &AnyPool, id: i64) -> Result<()> {
    // Remove extra field values first; filament_field.filament_id has no ON DELETE CASCADE.
    sqlx::query("DELETE FROM filament_field WHERE filament_id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    let result = sqlx::query("DELETE FROM filament WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err.kind() == sqlx::error::ErrorKind::ForeignKeyViolation {
                    return SpoolmanError::DeleteConflict(
                        "Failed to delete filament, see server logs for more information."
                            .to_owned(),
                    );
                }
            }
            SpoolmanError::Database(e)
        })?;
    if result.rows_affected() == 0 {
        return Err(SpoolmanError::NotFound(format!("filament with id {id}")));
    }
    Ok(())
}

pub async fn find_materials(pool: &AnyPool) -> Result<Vec<String>> {
    let rows = sqlx::query(
        "SELECT DISTINCT material FROM filament WHERE material IS NOT NULL ORDER BY material",
    )
    .fetch_all(pool)
    .await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        use sqlx::Row;
        result.push(row.try_get::<String, _>("material")?);
    }
    Ok(result)
}

pub async fn find_article_numbers(pool: &AnyPool) -> Result<Vec<String>> {
    let rows = sqlx::query(
        "SELECT DISTINCT article_number FROM filament WHERE article_number IS NOT NULL ORDER BY article_number",
    )
    .fetch_all(pool)
    .await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        use sqlx::Row;
        result.push(row.try_get::<String, _>("article_number")?);
    }
    Ok(result)
}
