//! Filament database queries.

use serde_json::Map;
use sqlx::AnyPool;

use crate::{
    db::{
        query::{order_by_sql, parse_sort, qualify_sort, Pagination, WhereBuilder},
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
    pub color_hex: Option<String>,
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
    filter: &FilamentFilter,
) -> Result<(Vec<FilamentResponse>, i64)> {
    let mut wb = WhereBuilder::new();
    wb.add_str_opt("f.name", filter.name.as_deref(), engine);
    wb.add_int_opt("f.vendor_id", filter.vendor_id.as_deref());
    wb.add_str_opt("v.name", filter.vendor_name.as_deref(), engine);
    wb.add_str_opt("f.material", filter.material.as_deref(), engine);
    wb.add_str_opt("f.article_number", filter.article_number.as_deref(), engine);
    wb.add_str_opt("f.external_id", filter.external_id.as_deref(), engine);
    wb.add_str_opt("f.color_hex", filter.color_hex.as_deref(), engine);

    let where_sql = wb.sql();
    let raw_sort = parse_sort(filter.sort.as_deref(), "id:asc");
    let sort_clauses = qualify_sort(
        &raw_sort,
        &[
            ("id", "f.id"),
            ("registered", "f.registered"),
            ("name", "f.name"),
            ("material", "f.material"),
            ("density", "f.density"),
            ("diameter", "f.diameter"),
            ("weight", "f.weight"),
            ("vendor.name", "v.name"),
        ],
    );
    let order_sql = order_by_sql(&sort_clauses);
    let pag = Pagination::new(filter.limit, filter.offset);

    let count_sql = format!(
        "SELECT COUNT(*) FROM filament f LEFT JOIN vendor v ON v.id = f.vendor_id {where_sql}"
    );
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);
    for b in &wb.binds {
        count_q = count_q.bind(b.clone());
    }
    let total: i64 = count_q.fetch_one(pool).await?;

    let data_sql = format!("{FILAMENT_SELECT} {where_sql} {order_sql} LIMIT ? OFFSET ?");
    let mut q = sqlx::query(&data_sql);
    for b in &wb.binds {
        q = q.bind(b.clone());
    }
    q = q.bind(pag.limit).bind(pag.offset);
    let rows = q.fetch_all(pool).await?;

    let mut results = Vec::with_capacity(rows.len());
    for row in rows {
        results.push(FilamentRow::from_row(&row)?.into_response());
    }
    Ok((results, total))
}

pub async fn find_by_id(pool: &AnyPool, id: i64) -> Result<FilamentResponse> {
    let sql = format!("{FILAMENT_SELECT} WHERE f.id = ?");
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| SpoolmanError::NotFound(format!("filament {id}")))?;

    FilamentRow::from_row(&row).map(FilamentRow::into_response)
}

pub async fn create(
    pool: &AnyPool,
    engine: &Engine,
    req: &CreateFilamentRequest,
) -> Result<FilamentResponse> {
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

    if !matches!(engine, Engine::Mysql) {
        // SQLite (3.35+) and Postgres support RETURNING id.
        let id: i64 = bind_all!(sqlx::query_scalar(&format!("{INSERT_SQL} RETURNING id")))
            .fetch_one(pool)
            .await?;
        return find_by_id(pool, id).await;
    }

    // MySQL: use transaction + LAST_INSERT_ID().
    let mut tx = pool.begin().await?;
    bind_all!(sqlx::query(INSERT_SQL)).execute(&mut *tx).await?;
    let id: i64 = sqlx::query_scalar("SELECT LAST_INSERT_ID()")
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
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

    if sets.is_empty() {
        return find_by_id(pool, id).await;
    }

    let sql = format!("UPDATE filament SET {} WHERE id = ?", sets.join(", "));
    let mut q = sqlx::query(&sql);
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
        return Err(SpoolmanError::NotFound(format!("filament {id}")));
    }
    find_by_id(pool, id).await
}

pub async fn delete(pool: &AnyPool, id: i64) -> Result<()> {
    let result = sqlx::query("DELETE FROM filament WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(SpoolmanError::NotFound(format!("filament {id}")));
    }
    Ok(())
}
