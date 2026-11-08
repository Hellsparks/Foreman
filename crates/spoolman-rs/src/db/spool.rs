//! Spool database queries.

use serde_json::Map;
use sqlx::AnyPool;

use crate::{
    db::{
        extra_field,
        query::{order_by_sql, parse_sort, qualify_sort, Pagination, WhereBuilder},
        tag,
        vendor::{normalise_dt, utc_now_str},
        Engine,
    },
    error::{Result, SpoolmanError},
    math::{length_from_weight, weight_from_length},
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
            tags: vec![],
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
            tags: vec![],
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

    let mut parsed = Vec::with_capacity(rows.len());
    for row in &rows {
        parsed.push(SpoolRow::from_row(row)?);
    }

    // Batch extra field lookup for spools, filaments, and vendors.
    let spool_ids: Vec<i64> = parsed.iter().map(|r| r.id).collect();
    let filament_ids: Vec<i64> = parsed.iter().map(|r| r.filament_id).collect();
    let vendor_ids: Vec<i64> = parsed.iter().filter_map(|r| r.v_id).collect();

    let mut s_extras =
        extra_field::get_values_batch(pool, "spool_field", "spool_id", &spool_ids).await?;
    let mut f_extras =
        extra_field::get_values_batch(pool, "filament_field", "filament_id", &filament_ids).await?;
    let mut v_extras =
        extra_field::get_values_batch(pool, "vendor_field", "vendor_id", &vendor_ids).await?;
    let mut tags_map = tag::get_spool_tags_batch(pool, &spool_ids).await?;
    let mut filament_tags_map = tag::get_filament_tags_batch(pool, &filament_ids).await?;

    let mut results = Vec::with_capacity(parsed.len());
    for r in parsed {
        let sid = r.id;
        let fid = r.filament_id;
        let vid = r.v_id;
        let mut resp = r.into_response();
        if let Some(e) = s_extras.remove(&sid) {
            resp.extra = e;
        }
        if let Some(e) = f_extras.remove(&fid) {
            resp.filament.extra = e;
        }
        if let (Some(vid), Some(ref mut v)) = (vid, resp.filament.vendor.as_mut()) {
            if let Some(e) = v_extras.remove(&vid) {
                v.extra = e;
            }
        }
        resp.tags = tags_map.remove(&sid).unwrap_or_default();
        resp.filament.tags = filament_tags_map.remove(&fid).unwrap_or_default();
        results.push(resp);
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
    let parsed = SpoolRow::from_row(&row)?;
    let filament_id = parsed.filament_id;
    let vendor_id = parsed.v_id;
    let mut resp = parsed.into_response();
    resp.extra = extra_field::get_values(pool, "spool_field", "spool_id", id).await?;
    resp.filament.extra =
        extra_field::get_values(pool, "filament_field", "filament_id", filament_id).await?;
    if let (Some(vid), Some(ref mut v)) = (vendor_id, resp.filament.vendor.as_mut()) {
        v.extra = extra_field::get_values(pool, "vendor_field", "vendor_id", vid).await?;
    }
    resp.tags = tag::get_spool_tags(pool, id).await?;
    resp.filament.tags = tag::get_filament_tags(pool, filament_id).await?;
    Ok(resp)
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

    let id: i64 = if !matches!(engine, Engine::Mysql) {
        // SQLite (3.35+) and Postgres support RETURNING id.
        bind_all!(sqlx::query_scalar(&format!("{INSERT_SQL} RETURNING id")))
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
        extra_field::set_values(pool, "spool_field", "spool_id", id, &req.extra).await?;
    }

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

    if let Some(extra) = &req.extra {
        extra_field::set_values(pool, "spool_field", "spool_id", id, extra).await?;
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

// ---------------------------------------------------------------------------
// Use / Measure
// ---------------------------------------------------------------------------

/// Consume filament by weight (grams) — mirrors upstream `use_weight`.
///
/// Increments `used_weight` atomically (clamped to 0), sets `first_used` if
/// not already set, and updates `last_used` to now.
pub async fn use_weight(pool: &AnyPool, id: i64, weight: f64) -> Result<SpoolResponse> {
    let now = utc_now_str();

    // Atomic update: clamp used_weight to >= 0; set timestamps in same statement.
    // COALESCE(first_used, ?) preserves existing first_used (NULL → now).
    let n = sqlx::query(
        "UPDATE spool SET \
         used_weight = CASE WHEN used_weight + ? >= 0.0 THEN used_weight + ? ELSE 0.0 END, \
         first_used = COALESCE(first_used, ?), \
         last_used = ? \
         WHERE id = ?",
    )
    .bind(weight)
    .bind(weight)
    .bind(&now)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;

    if n.rows_affected() == 0 {
        return Err(SpoolmanError::NotFound(format!("spool {id}")));
    }

    find_by_id(pool, id).await
}

/// Consume filament by length (millimetres) — mirrors upstream `use_length`.
///
/// Converts length → weight via the spool's filament density and diameter,
/// then delegates to [`use_weight`].
pub async fn use_length(pool: &AnyPool, id: i64, length_mm: f64) -> Result<SpoolResponse> {
    // Fetch filament density and diameter for the weight calculation.
    let row = sqlx::query(
        "SELECT f.density, f.diameter FROM spool s \
         JOIN filament f ON f.id = s.filament_id \
         WHERE s.id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| SpoolmanError::NotFound(format!("spool {id}")))?;

    let density: f64 = {
        use sqlx::Row;
        row.try_get("density")?
    };
    let diameter: f64 = {
        use sqlx::Row;
        row.try_get("diameter")?
    };

    let weight = weight_from_length(length_mm, density, diameter);
    use_weight(pool, id, weight).await
}

/// Measure the current gross weight of the spool and derive how much filament
/// was used — mirrors upstream `measure`.
///
/// `weight_g` is the current gross weight of the spool (filament + empty spool).
pub async fn measure(pool: &AnyPool, id: i64, weight_g: f64) -> Result<SpoolResponse> {
    // --- Step 1: get spool's own initial_weight, used_weight, spool_weight ---
    let spool_row =
        sqlx::query("SELECT initial_weight, used_weight, spool_weight FROM spool WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| SpoolmanError::NotFound(format!("spool {id}")))?;

    let (spool_initial_weight, spool_used_weight, spool_spool_weight): (
        Option<f64>,
        f64,
        Option<f64>,
    ) = {
        use sqlx::Row;
        (
            spool_row.try_get("initial_weight")?,
            spool_row.try_get("used_weight")?,
            spool_row.try_get("spool_weight")?,
        )
    };

    let mut initial_weight = spool_initial_weight;
    let mut spool_weight = spool_spool_weight;

    // --- Step 2: fill in from filament/vendor if needed ---
    let needs_filament = initial_weight.map(|w| w == 0.0).unwrap_or(true) || spool_weight.is_none();
    if needs_filament {
        let fil_row = sqlx::query(
            "SELECT f.weight, f.spool_weight, v.empty_spool_weight \
             FROM spool s \
             JOIN filament f ON f.id = s.filament_id \
             LEFT JOIN vendor v ON v.id = f.vendor_id \
             WHERE s.id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| SpoolmanError::NotFound(format!("filament for spool {id}")))?;

        let (fil_weight, fil_spool_weight, vendor_spool_weight): (
            Option<f64>,
            Option<f64>,
            Option<f64>,
        ) = {
            use sqlx::Row;
            (
                fil_row.try_get("weight")?,
                fil_row.try_get("spool_weight")?,
                fil_row.try_get("empty_spool_weight")?,
            )
        };

        if spool_weight.is_none() {
            spool_weight = fil_spool_weight.or(vendor_spool_weight);
        }
        if initial_weight.map(|w| w == 0.0).unwrap_or(true) {
            initial_weight = fil_weight.or(Some(0.0));
        }
    }

    let initial_weight = initial_weight.unwrap_or(0.0);
    if initial_weight == 0.0 {
        return Err(SpoolmanError::MeasureError(
            "initial weight is not set for this spool".into(),
        ));
    }
    let spool_weight = spool_weight.unwrap_or(0.0);

    let initial_gross = initial_weight + spool_weight;

    // --- Step 3: apply measurement ---
    if weight_g > initial_gross {
        // Measured more than expected — reset initial_weight.
        sqlx::query("UPDATE spool SET initial_weight = ?, used_weight = 0.0 WHERE id = ?")
            .bind(weight_g - spool_weight)
            .bind(id)
            .execute(pool)
            .await?;
        return find_by_id(pool, id).await;
    }

    // current_use = how much we expect to still be on the spool (gross)
    let current_use = initial_gross - spool_used_weight;
    // weight_to_consume = difference between expected and measured remaining
    let mut weight_to_consume = current_use - weight_g;
    // If the measurement is below the empty spool weight, cap at "use all filament".
    if initial_gross - weight_to_consume < spool_weight {
        weight_to_consume = current_use - spool_weight;
    }

    use_weight(pool, id, weight_to_consume).await
}

// ---------------------------------------------------------------------------
// Find helpers
// ---------------------------------------------------------------------------

pub async fn find_locations(pool: &AnyPool) -> Result<Vec<String>> {
    let rows = sqlx::query(
        "SELECT DISTINCT location FROM spool WHERE location IS NOT NULL ORDER BY location",
    )
    .fetch_all(pool)
    .await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        use sqlx::Row;
        result.push(row.try_get::<String, _>("location")?);
    }
    Ok(result)
}

pub async fn find_lot_numbers(pool: &AnyPool) -> Result<Vec<String>> {
    let rows =
        sqlx::query("SELECT DISTINCT lot_nr FROM spool WHERE lot_nr IS NOT NULL ORDER BY lot_nr")
            .fetch_all(pool)
            .await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        use sqlx::Row;
        result.push(row.try_get::<String, _>("lot_nr")?);
    }
    Ok(result)
}

pub async fn rename_location(pool: &AnyPool, old_name: &str, new_name: &str) -> Result<()> {
    sqlx::query("UPDATE spool SET location = ? WHERE location = ?")
        .bind(new_name)
        .bind(old_name)
        .execute(pool)
        .await?;
    Ok(())
}

/// Rename a spool field value across all spools that hold it.
///
/// `field` is either `"location"` or `"extra.<key>"`.  Any other value returns a `BadRequest`.
/// Returns the number of spools changed.
///
/// Mirrors `spoolman/database/spool.py::rename_field_value`.
pub async fn rename_field_value(
    pool: &AnyPool,
    field: &str,
    value: &str,
    new_value: &str,
) -> Result<i64> {
    if field == "location" {
        let result = sqlx::query("UPDATE spool SET location = ? WHERE location = ?")
            .bind(new_value)
            .bind(value)
            .execute(pool)
            .await?;
        return Ok(result.rows_affected() as i64);
    }

    if let Some(key) = field.strip_prefix("extra.") {
        // Encode the value as a JSON string (the way it's stored in spool_field.value).
        let encoded_old = serde_json::to_string(value)?;
        let encoded_new = serde_json::to_string(new_value)?;
        let result = sqlx::query("UPDATE spool_field SET value = ? WHERE key = ? AND value = ?")
            .bind(encoded_new)
            .bind(key)
            .bind(encoded_old)
            .execute(pool)
            .await?;
        return Ok(result.rows_affected() as i64);
    }

    Err(SpoolmanError::BadRequest(format!(
        "Cannot rename values of '{field}'. Only fields the spool itself owns can be renamed: \
         location, or 'extra.<spool extra field key>'."
    )))
}

// ---------------------------------------------------------------------------
// Spool grouping
// ---------------------------------------------------------------------------

/// Parameters for `GET /spool/group`.
#[derive(Debug, Default)]
pub struct SpoolGroupFilter {
    /// Required: `filament`, `vendor`, `material`, `location`, or `extra.<key>`.
    pub group_by: String,
    // Reuse spool filter fields:
    pub filament_name: Option<String>,
    pub filament_id: Option<String>,
    pub filament_material: Option<String>,
    pub filament_vendor_name: Option<String>,
    pub filament_vendor_id: Option<String>,
    pub location: Option<String>,
    pub allow_archived: bool,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// One row of the aggregation query.
struct GroupRow {
    group_key: Option<String>,
    spool_count: i64,
    in_use_count: i64,
    total_remaining_weight: f64,
    last_used: Option<String>,
    /// Only for group_by = vendor
    filament_count: Option<i64>,
}

/// Find grouped spool aggregates.
///
/// Supports `group_by` values: `filament`, `vendor`, `material`, `location`, `extra.<key>`.
/// Does not support `include_empty` (deferred to session 10).
/// Does not support the `preview` flag (deferred to session 10).
pub async fn find_groups(
    pool: &AnyPool,
    engine: &Engine,
    filter: &SpoolGroupFilter,
) -> Result<(Vec<crate::models::SpoolGroupResponse>, i64, i64, f64)> {
    use crate::db::{filament as filament_db, vendor as vendor_db};

    let group_by = filter.group_by.trim();

    // Resolve the SQL GROUP BY column.
    let group_col_sql = if group_by == "filament" {
        "s.filament_id"
    } else if group_by == "vendor" {
        "f.vendor_id"
    } else if group_by == "material" {
        "f.material"
    } else if group_by == "location" {
        "s.location"
    } else if let Some(key) = group_by.strip_prefix("extra.") {
        // For extra fields we join spool_field table once.
        // We'll handle this via a subquery approach below.
        // For now, validate and fall through to extra-field path.
        let _ = key; // used below
        "sf_group.value"
    } else {
        return Err(SpoolmanError::BadRequest(format!(
            "Invalid group_by field '{group_by}'. \
             Must be one of: filament, vendor, material, location, or extra.<key>."
        )));
    };

    // Build filter WHERE conditions (same as spool find).
    let mut wb = WhereBuilder::new();
    wb.add_str_opt("f.name", filter.filament_name.as_deref(), engine);
    wb.add_int_opt("s.filament_id", filter.filament_id.as_deref());
    wb.add_str_opt("f.material", filter.filament_material.as_deref(), engine);
    wb.add_str_opt("v.name", filter.filament_vendor_name.as_deref(), engine);
    wb.add_int_opt("f.vendor_id", filter.filament_vendor_id.as_deref());
    wb.add_str_opt("s.location", filter.location.as_deref(), engine);
    if !filter.allow_archived {
        wb.add_bool("s.archived", false);
    }

    let where_sql = wb.sql();
    let pag = Pagination::new(filter.limit, filter.offset);

    // Build the extra-field join clause if needed.
    let extra_join = if let Some(key) = group_by.strip_prefix("extra.") {
        format!(
            "LEFT JOIN spool_field sf_group \
             ON sf_group.spool_id = s.id AND sf_group.key = '{}'",
            key.replace('\'', "''")
        )
    } else {
        String::new()
    };

    // The GROUP BY column in the SELECT.
    // For vendor, also count distinct filaments.
    let extra_agg = if group_by == "vendor" {
        ", COUNT(DISTINCT f.id) AS filament_count"
    } else {
        ""
    };

    // Remaining weight expression — mirrors upstream's formula:
    //   max(0, coalesce(initial_weight, filament.weight) - used_weight)
    let remaining_expr = "CASE WHEN COALESCE(s.initial_weight, f.weight, 0.0) - s.used_weight < 0 \
         THEN 0.0 \
         ELSE COALESCE(s.initial_weight, f.weight, 0.0) - s.used_weight END";

    let agg_sql = format!(
        "SELECT \
         CAST({group_col_sql} AS TEXT) AS group_key, \
         COUNT(s.id) AS spool_count, \
         COUNT(CASE WHEN s.used_weight > 0 THEN s.id END) AS in_use_count, \
         COALESCE(SUM({remaining_expr}), 0.0) AS total_remaining_weight, \
         MAX(CAST(s.last_used AS TEXT)) AS last_used \
         {extra_agg} \
         FROM spool s \
         JOIN filament f ON f.id = s.filament_id \
         LEFT JOIN vendor v ON v.id = f.vendor_id \
         {extra_join} \
         {where_sql} \
         GROUP BY {group_col_sql} \
         ORDER BY {group_col_sql} ASC \
         LIMIT ? OFFSET ?"
    );

    let count_sql = format!(
        "SELECT COUNT(*) FROM ( \
         SELECT {group_col_sql} \
         FROM spool s \
         JOIN filament f ON f.id = s.filament_id \
         LEFT JOIN vendor v ON v.id = f.vendor_id \
         {extra_join} \
         {where_sql} \
         GROUP BY {group_col_sql} \
         ) AS _cnt"
    );

    // Count query.
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);
    for b in &wb.binds {
        count_q = count_q.bind(b.clone());
    }
    let total_count: i64 = count_q.fetch_one(pool).await?;

    // Data query.
    let mut q = sqlx::query(&agg_sql);
    for b in &wb.binds {
        q = q.bind(b.clone());
    }
    q = q.bind(pag.limit).bind(pag.offset);
    let rows = q.fetch_all(pool).await?;

    // Parse aggregate rows.
    let mut group_rows: Vec<GroupRow> = Vec::with_capacity(rows.len());
    for row in &rows {
        use sqlx::Row as _;
        let group_key: Option<String> = row.try_get("group_key").ok().flatten();
        let spool_count: i64 = row.try_get("spool_count").unwrap_or(0);
        let in_use_count: i64 = row.try_get("in_use_count").unwrap_or(0);
        let total_remaining_weight: f64 = row.try_get("total_remaining_weight").unwrap_or(0.0);
        let last_used: Option<String> = row.try_get_unchecked("last_used").ok().flatten();
        let filament_count: Option<i64> = if group_by == "vendor" {
            row.try_get("filament_count").ok()
        } else {
            None
        };
        group_rows.push(GroupRow {
            group_key,
            spool_count,
            in_use_count,
            total_remaining_weight,
            last_used,
            filament_count,
        });
    }

    // Totals across all groups (before pagination).
    let totals_sql = format!(
        "SELECT \
         COALESCE(SUM(spool_count), 0) AS total_spools, \
         COALESCE(SUM(total_remaining_weight), 0.0) AS total_remaining_weight \
         FROM ( \
           SELECT COUNT(s.id) AS spool_count, \
           COALESCE(SUM({remaining_expr}), 0.0) AS total_remaining_weight \
           FROM spool s \
           JOIN filament f ON f.id = s.filament_id \
           LEFT JOIN vendor v ON v.id = f.vendor_id \
           {extra_join} \
           {where_sql} \
           GROUP BY {group_col_sql} \
         ) AS _t"
    );
    let mut totals_q = sqlx::query(&totals_sql);
    for b in &wb.binds {
        totals_q = totals_q.bind(b.clone());
    }
    let totals_row = totals_q.fetch_one(pool).await?;
    let total_spools: i64 = {
        use sqlx::Row as _;
        totals_row.try_get("total_spools").unwrap_or(0)
    };
    let total_remaining_weight: f64 = {
        use sqlx::Row as _;
        totals_row.try_get("total_remaining_weight").unwrap_or(0.0)
    };

    // Hydrate grouped entities (filament or vendor).
    let mut results: Vec<crate::models::SpoolGroupResponse> = Vec::with_capacity(group_rows.len());

    for gr in group_rows {
        let filament_obj = if group_by == "filament" {
            if let Some(ref key) = gr.group_key {
                if let Ok(fid) = key.parse::<i64>() {
                    filament_db::find_by_id(pool, fid).await.ok()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let vendor_obj = if group_by == "vendor" {
            if let Some(ref key) = gr.group_key {
                if let Ok(vid) = key.parse::<i64>() {
                    vendor_db::find_by_id(pool, vid).await.ok()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        results.push(crate::models::SpoolGroupResponse {
            group_by: group_by.to_owned(),
            key: gr.group_key,
            spool_count: gr.spool_count,
            in_use_count: gr.in_use_count,
            total_remaining_weight: gr.total_remaining_weight,
            last_used: gr.last_used.as_deref().map(normalise_dt),
            filament: filament_obj,
            vendor: vendor_obj,
            filament_count: gr.filament_count,
        });
    }

    Ok((results, total_count, total_spools, total_remaining_weight))
}
