//! Spool database queries.

use serde_json::{Map, Value};
use sqlx::AnyPool;

use crate::{
    db::{
        query::{order_by_sql, parse_sort, qualify_sort, Pagination, WhereBuilder},
        vendor::{normalise_dt, utc_now_str},
        Engine,
    },
    error::{Result, SpoolmanError},
    math::length_from_weight,
    models::{
        CreateSpoolRequest, FilamentResponse, SpoolResponse, UpdateSpoolRequest, VendorResponse,
    },
};

// ---------------------------------------------------------------------------
// Row type — flat JOIN result over spool + filament + vendor
// ---------------------------------------------------------------------------

struct SpoolRow {
    // spool columns
    id: i64,
    registered: String,
    first_used: Option<String>,
    last_used: Option<String>,
    filament_id: i64,
    used_weight: f64,
    location: Option<String>,
    lot_nr: Option<String>,
    comment: Option<String>,
    archived: Option<i64>, // TINYINT(1) / BOOLEAN — decoded as int
    price: Option<f64>,
    initial_weight: Option<f64>,
    spool_weight: Option<f64>,
    // filament columns (f_*)
    f_registered: String,
    f_name: Option<String>,
    f_material: Option<String>,
    f_price: Option<f64>,
    f_density: f64,
    f_diameter: f64,
    f_weight: Option<f64>,
    f_spool_weight: Option<f64>,
    f_article_number: Option<String>,
    f_comment: Option<String>,
    f_settings_extruder_temp: Option<i64>,
    f_settings_bed_temp: Option<i64>,
    f_color_hex: Option<String>,
    f_multi_color_hexes: Option<String>,
    f_multi_color_direction: Option<String>,
    f_external_id: Option<String>,
    // vendor columns (v_*)
    v_id: Option<i64>,
    v_registered: Option<String>,
    v_name: Option<String>,
    v_comment: Option<String>,
    v_empty_spool_weight: Option<f64>,
    v_external_id: Option<String>,
}

impl SpoolRow {
    fn from_row(row: &sqlx::any::AnyRow) -> Result<Self> {
        use sqlx::Row as _;
        Ok(SpoolRow {
            id: row.try_get("id")?,
            // DATETIME columns use try_get_unchecked — see vendor.rs for explanation.
            registered: row.try_get_unchecked("registered")?,
            first_used: row.try_get_unchecked("first_used")?,
            last_used: row.try_get_unchecked("last_used")?,
            filament_id: row.try_get("filament_id")?,
            used_weight: row.try_get("used_weight")?,
            location: row.try_get("location")?,
            lot_nr: row.try_get("lot_nr")?,
            comment: row.try_get("comment")?,
            archived: row.try_get("archived")?,
            price: row.try_get("price")?,
            initial_weight: row.try_get("initial_weight")?,
            spool_weight: row.try_get("spool_weight")?,
            f_registered: row.try_get_unchecked("f_registered")?,
            f_name: row.try_get("f_name")?,
            f_material: row.try_get("f_material")?,
            f_price: row.try_get("f_price")?,
            f_density: row.try_get("f_density")?,
            f_diameter: row.try_get("f_diameter")?,
            f_weight: row.try_get("f_weight")?,
            f_spool_weight: row.try_get("f_spool_weight")?,
            f_article_number: row.try_get("f_article_number")?,
            f_comment: row.try_get("f_comment")?,
            f_settings_extruder_temp: row.try_get("f_settings_extruder_temp")?,
            f_settings_bed_temp: row.try_get("f_settings_bed_temp")?,
            f_color_hex: row.try_get("f_color_hex")?,
            f_multi_color_hexes: row.try_get("f_multi_color_hexes")?,
            f_multi_color_direction: row.try_get("f_multi_color_direction")?,
            f_external_id: row.try_get("f_external_id")?,
            v_id: row.try_get("v_id")?,
            v_registered: row.try_get_unchecked("v_registered")?,
            v_name: row.try_get("v_name")?,
            v_comment: row.try_get("v_comment")?,
            v_empty_spool_weight: row.try_get("v_empty_spool_weight")?,
            v_external_id: row.try_get("v_external_id")?,
        })
    }

    fn into_response(self) -> SpoolResponse {
        // Computed weight / length fields.
        let remaining_weight = self.initial_weight.map(|iw| iw - self.used_weight);
        let remaining_length =
            remaining_weight.map(|rw| length_from_weight(rw, self.f_density, self.f_diameter));
        let used_length = Some(length_from_weight(
            self.used_weight,
            self.f_density,
            self.f_diameter,
        ));

        let vendor = match (self.v_id, self.v_name.as_ref()) {
            (Some(vid), Some(vname)) => Some(VendorResponse {
                id: vid,
                registered: normalise_dt(self.v_registered.as_deref().unwrap_or("")),
                name: vname.clone(),
                comment: self.v_comment,
                empty_spool_weight: self.v_empty_spool_weight,
                external_id: self.v_external_id,
                extra: Map::new(),
            }),
            _ => None,
        };

        let filament = FilamentResponse {
            id: self.filament_id,
            registered: normalise_dt(&self.f_registered),
            name: self.f_name,
            vendor,
            material: self.f_material,
            price: self.f_price,
            density: self.f_density,
            diameter: self.f_diameter,
            weight: self.f_weight,
            spool_weight: self.f_spool_weight,
            article_number: self.f_article_number,
            comment: self.f_comment,
            settings_extruder_temp: self.f_settings_extruder_temp,
            settings_bed_temp: self.f_settings_bed_temp,
            color_hex: self.f_color_hex,
            multi_color_hexes: self.f_multi_color_hexes,
            multi_color_direction: self.f_multi_color_direction,
            external_id: self.f_external_id,
            extra: Map::new(),
        };

        SpoolResponse {
            id: self.id,
            registered: normalise_dt(&self.registered),
            first_used: self.first_used.as_deref().map(normalise_dt),
            last_used: self.last_used.as_deref().map(normalise_dt),
            filament,
            used_weight: self.used_weight,
            remaining_weight,
            remaining_length,
            used_length,
            location: self.location,
            lot_nr: self.lot_nr,
            comment: self.comment,
            archived: self.archived.map(|v| v != 0).unwrap_or(false),
            price: self.price,
            initial_weight: self.initial_weight,
            spool_weight: self.spool_weight,
            tags: Vec::<Value>::new(),
            extra: Map::new(),
        }
    }
}

// CAST(... AS TEXT) on all DATETIME columns so AnyPool can decode them as String.
const SPOOL_SELECT: &str = "SELECT s.id, CAST(s.registered AS TEXT) AS registered, \
     CAST(s.first_used AS TEXT) AS first_used, CAST(s.last_used AS TEXT) AS last_used, \
     s.filament_id, s.used_weight, s.location, s.lot_nr, s.comment, \
     CAST(s.archived AS INTEGER) AS archived, s.price, \
     s.initial_weight, s.spool_weight, \
     CAST(f.registered AS TEXT) AS f_registered, f.name AS f_name, \
     f.material AS f_material, f.price AS f_price, f.density AS f_density, \
     f.diameter AS f_diameter, f.weight AS f_weight, f.spool_weight AS f_spool_weight, \
     f.article_number AS f_article_number, f.comment AS f_comment, \
     f.settings_extruder_temp AS f_settings_extruder_temp, \
     f.settings_bed_temp AS f_settings_bed_temp, f.color_hex AS f_color_hex, \
     f.multi_color_hexes AS f_multi_color_hexes, \
     f.multi_color_direction AS f_multi_color_direction, \
     f.external_id AS f_external_id, \
     v.id AS v_id, CAST(v.registered AS TEXT) AS v_registered, v.name AS v_name, \
     v.comment AS v_comment, v.empty_spool_weight AS v_empty_spool_weight, \
     v.external_id AS v_external_id \
     FROM spool s \
     JOIN filament f ON f.id = s.filament_id \
     LEFT JOIN vendor v ON v.id = f.vendor_id";

// ---------------------------------------------------------------------------
// Filter params
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct SpoolFilter {
    pub filament_name: Option<String>,
    pub filament_id: Option<String>,
    pub filament_material: Option<String>,
    pub filament_vendor_name: Option<String>,
    pub filament_vendor_id: Option<String>,
    pub location: Option<String>,
    pub lot_nr: Option<String>,
    pub allow_archived: bool,
    pub first_used: Option<String>,
    pub last_used: Option<String>,
    pub registered: Option<String>,
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
    filter: &SpoolFilter,
) -> Result<(Vec<SpoolResponse>, i64)> {
    let mut wb = WhereBuilder::new();
    wb.add_str_opt("f.name", filter.filament_name.as_deref(), engine);
    wb.add_int_opt("f.id", filter.filament_id.as_deref());
    wb.add_str_opt("f.material", filter.filament_material.as_deref(), engine);
    wb.add_str_opt("v.name", filter.filament_vendor_name.as_deref(), engine);
    wb.add_int_opt("v.id", filter.filament_vendor_id.as_deref());
    wb.add_str_opt("s.location", filter.location.as_deref(), engine);
    wb.add_str_opt("s.lot_nr", filter.lot_nr.as_deref(), engine);
    wb.add_datetime_range_opt("s.first_used", filter.first_used.as_deref());
    wb.add_datetime_range_opt("s.last_used", filter.last_used.as_deref());
    wb.add_datetime_range_opt("s.registered", filter.registered.as_deref());
    if !filter.allow_archived {
        wb.add_bool("s.archived", false);
    }

    let where_sql = wb.sql();
    let raw_sort = parse_sort(filter.sort.as_deref(), "id:asc");
    let sort_clauses = qualify_sort(
        &raw_sort,
        &[
            ("id", "s.id"),
            ("registered", "s.registered"),
            ("first_used", "s.first_used"),
            ("last_used", "s.last_used"),
            ("location", "s.location"),
            ("filament.name", "f.name"),
            ("filament.material", "f.material"),
        ],
    );
    let order_sql = order_by_sql(&sort_clauses);
    let pag = Pagination::new(filter.limit, filter.offset);

    let joins = "FROM spool s JOIN filament f ON f.id = s.filament_id LEFT JOIN vendor v ON v.id = f.vendor_id";

    let count_sql = format!("SELECT COUNT(*) {joins} {where_sql}");
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);
    for b in &wb.binds {
        count_q = count_q.bind(b.clone());
    }
    let total: i64 = count_q.fetch_one(pool).await?;

    let data_sql = format!("{SPOOL_SELECT} {where_sql} {order_sql} LIMIT ? OFFSET ?");
    let mut q = sqlx::query(&data_sql);
    for b in &wb.binds {
        q = q.bind(b.clone());
    }
    q = q.bind(pag.limit).bind(pag.offset);
    let rows = q.fetch_all(pool).await?;

    let mut results = Vec::with_capacity(rows.len());
    for row in rows {
        results.push(SpoolRow::from_row(&row)?.into_response());
    }
    Ok((results, total))
}

pub async fn find_by_id(pool: &AnyPool, id: i64) -> Result<SpoolResponse> {
    let sql = format!("{SPOOL_SELECT} WHERE s.id = ?");
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| SpoolmanError::NotFound(format!("spool {id}")))?;
    SpoolRow::from_row(&row).map(SpoolRow::into_response)
}

pub async fn create(
    pool: &AnyPool,
    engine: &Engine,
    req: &CreateSpoolRequest,
) -> Result<SpoolResponse> {
    let now = utc_now_str();

    // Derive initial_weight and used_weight from request following upstream logic:
    // - If remaining_weight is given: used_weight = initial_weight - remaining_weight
    // - If used_weight is given: used_weight = used_weight
    // - Otherwise: used_weight = 0.0
    // initial_weight is taken from the request or filament.weight (fetched separately).
    // For session 6 we just store what's given; session 7 will add the filament.weight lookup.
    let used_weight = if let (Some(iw), Some(rw)) = (req.initial_weight, req.remaining_weight) {
        iw - rw
    } else {
        req.used_weight.unwrap_or(0.0)
    };
    let initial_weight = req.initial_weight;
    let spool_weight = req.spool_weight;
    // Default archived to false (0) — upstream Spoolman also defaults to false.
    let archived_val: i64 = if req.archived.unwrap_or(false) { 1 } else { 0 };

    const INSERT_SQL: &str = "INSERT INTO spool \
         (registered, first_used, last_used, filament_id, used_weight, location, lot_nr, \
          comment, archived, price, initial_weight, spool_weight) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?)";

    macro_rules! bind_all {
        ($q:expr) => {
            $q.bind(&now)
                .bind(&req.first_used)
                .bind(&req.last_used)
                .bind(req.filament_id)
                .bind(used_weight)
                .bind(&req.location)
                .bind(&req.lot_nr)
                .bind(&req.comment)
                .bind(archived_val)
                .bind(req.price)
                .bind(initial_weight)
                .bind(spool_weight)
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

pub async fn update(pool: &AnyPool, id: i64, req: &UpdateSpoolRequest) -> Result<SpoolResponse> {
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

    // Handle remaining_weight → used_weight derivation if initial_weight or used_weight given.
    if req.remaining_weight.is_some() || req.used_weight.is_some() {
        // Need to compute used_weight.  If remaining_weight is given we need initial_weight
        // from the DB; for simplicity if both remaining_weight AND initial_weight are in
        // the request we compute inline.  Otherwise fall through to store used_weight directly.
        if let (Some(iw), Some(rw)) = (req.initial_weight, req.remaining_weight) {
            sets.push("used_weight = ?");
            binds.push(Bind::F64(Some(iw - rw)));
        } else if let Some(uw) = req.used_weight {
            sets.push("used_weight = ?");
            binds.push(Bind::F64(Some(uw)));
        }
    }

    maybe_i64!(req.filament_id, "filament_id");
    maybe_f64!(req.initial_weight, "initial_weight");
    maybe_f64!(req.spool_weight, "spool_weight");
    maybe_str!(req.first_used, "first_used");
    maybe_str!(req.last_used, "last_used");
    maybe_str!(req.location, "location");
    maybe_str!(req.lot_nr, "lot_nr");
    maybe_str!(req.comment, "comment");
    if let Some(archived) = req.archived {
        sets.push("archived = ?");
        binds.push(Bind::I64(Some(if archived { 1 } else { 0 })));
    }
    maybe_f64!(req.price, "price");

    if sets.is_empty() {
        return find_by_id(pool, id).await;
    }

    let sql = format!("UPDATE spool SET {} WHERE id = ?", sets.join(", "));
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
        return Err(SpoolmanError::NotFound(format!("spool {id}")));
    }
    find_by_id(pool, id).await
}

pub async fn delete(pool: &AnyPool, id: i64) -> Result<()> {
    let result = sqlx::query("DELETE FROM spool WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(SpoolmanError::NotFound(format!("spool {id}")));
    }
    Ok(())
}
