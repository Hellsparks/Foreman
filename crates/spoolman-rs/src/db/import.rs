//! Bring an existing Spoolman database into the built-in one (first-run setup, 6.4).
//!
//! 1. [`inspect_sqlite`] reads a file read-only: engine (SQLite magic), alembic revision,
//!    row counts. Nothing is changed.
//! 2. The caller migrates a *copy* to HEAD with [`Db::connect`] (never the original).
//! 3. [`replace_all_from`] swaps every Spoolman table of the live database for the
//!    copy's rows in one transaction (SQLite live database only).
//!
//! [`build_from_api`] writes a Spoolman database from another Spoolman's REST API
//! (`/api/v1/vendor`, `/filament`, `/spool`, `/setting/`) so the same steps apply.

use std::path::Path;

use serde::Serialize;
use serde_json::Value;
use sqlx::{sqlite::SqliteConnectOptions, ConnectOptions, Connection, Row};

use crate::{
    config::{DbType, SpoolmanConfig},
    db::{migrations, Db, Engine},
    error::{Result, SpoolmanError},
};

/// First 16 bytes of every SQLite 3 database file.
pub const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Tables copied, parents first. Deletion runs in reverse.
pub const TABLES: &[&str] = &[
    "vendor",
    "vendor_field",
    "filament",
    "filament_field",
    "spool",
    "spool_field",
    "tag",
    "setting",
];

/// Rows in a Spoolman database.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub vendors: i64,
    pub filaments: i64,
    pub spools: i64,
    pub archived_spools: i64,
    /// Custom (extra) field definitions across vendor / filament / spool.
    pub extra_fields: i64,
}

impl Counts {
    pub fn is_empty(&self) -> bool {
        self.vendors == 0 && self.filaments == 0 && self.spools == 0
    }
}

/// Where a revision sits relative to the chain this build knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionStatus {
    /// Already at HEAD: used as-is.
    Head,
    /// A known older revision: migrated to HEAD on import (the copy, not the file).
    Older,
    /// Not in the chain (a newer Spoolman, or not Spoolman): refused.
    Unknown,
    /// No `alembic_version` table: not a Spoolman database.
    Missing,
}

#[derive(Debug, Clone, Serialize)]
pub struct Inspection {
    /// `sqlite` (files are always SQLite; other engines are reached by URL).
    pub engine: String,
    pub revision: Option<String>,
    pub status: RevisionStatus,
    /// Migration steps from `revision` to HEAD (0 at HEAD).
    pub steps_to_head: usize,
    pub head: String,
    pub counts: Counts,
}

impl Inspection {
    pub fn accepted(&self) -> bool {
        matches!(self.status, RevisionStatus::Head | RevisionStatus::Older)
    }

    /// Plain-language reason a file is refused, `None` when it can be imported.
    pub fn refusal(&self) -> Option<String> {
        match self.status {
            RevisionStatus::Head | RevisionStatus::Older => None,
            RevisionStatus::Unknown => Some(format!(
                "This database is at revision {}, which this Foreman does not know (it knows up to {}). \
                 It was probably written by a newer Spoolman. Foreman will not open it, so nothing in it can be damaged. \
                 Update Foreman, or connect to that Spoolman by URL instead.",
                self.revision.as_deref().unwrap_or("?"),
                self.head
            )),
            RevisionStatus::Missing => Some(
                "This SQLite file has no alembic_version table, so it is not a Spoolman database.".into(),
            ),
        }
    }
}

/// Classify a revision string against the known chain.
pub fn classify(revision: Option<&str>) -> (RevisionStatus, usize) {
    match revision {
        None => (RevisionStatus::Missing, 0),
        Some(r) if r == migrations::HEAD => (RevisionStatus::Head, 0),
        Some(r) => match migrations::CHAIN.iter().position(|m| m.revision == r) {
            Some(i) => (RevisionStatus::Older, migrations::CHAIN.len() - 1 - i),
            None => (RevisionStatus::Unknown, 0),
        },
    }
}

/// Is this file a SQLite database (by its header)?
pub fn is_sqlite_file(path: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; 16];
    match f.read_exact(&mut buf) {
        Ok(()) => Ok(&buf == SQLITE_MAGIC),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e),
    }
}

async fn scalar(conn: &mut sqlx::SqliteConnection, sql: &'static str) -> i64 {
    sqlx::query_scalar::<_, i64>(sql)
        .fetch_one(&mut *conn)
        .await
        .unwrap_or(0)
}

/// Read a Spoolman SQLite file without changing it.
pub async fn inspect_sqlite(path: &Path) -> Result<Inspection> {
    if !is_sqlite_file(path)? {
        return Err(SpoolmanError::BadRequest(
            "This is not a SQLite database. A Spoolman that runs on PostgreSQL or MySQL/MariaDB \
             is imported by URL instead."
                .into(),
        ));
    }
    let mut conn = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .connect()
        .await?;
    let revision =
        sqlx::query_scalar::<_, String>("SELECT version_num FROM alembic_version LIMIT 1")
            .fetch_optional(&mut conn)
            .await
            .ok()
            .flatten();
    let (status, steps_to_head) = classify(revision.as_deref());
    let mut counts = Counts {
        vendors: scalar(&mut conn, "SELECT COUNT(*) FROM vendor").await,
        filaments: scalar(&mut conn, "SELECT COUNT(*) FROM filament").await,
        spools: scalar(&mut conn, "SELECT COUNT(*) FROM spool").await,
        archived_spools: scalar(&mut conn, "SELECT COUNT(*) FROM spool WHERE archived = 1").await,
        extra_fields: 0,
    };
    counts.extra_fields = extra_field_count(&mut conn).await;
    conn.close().await.ok();
    Ok(Inspection {
        engine: "sqlite".into(),
        revision,
        status,
        steps_to_head,
        head: migrations::HEAD.into(),
        counts,
    })
}

/// Extra field definitions live as JSON arrays in `setting` (`extra_fields_<entity>`).
async fn extra_field_count(conn: &mut sqlx::SqliteConnection) -> i64 {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT value FROM setting WHERE key IN \
         ('extra_fields_vendor', 'extra_fields_filament', 'extra_fields_spool')",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap_or_default();
    rows.iter()
        .map(|v| {
            serde_json::from_str::<Value>(v)
                .ok()
                .and_then(|j| j.as_array().map(|a| a.len() as i64))
                .unwrap_or(0)
        })
        .sum()
}

/// Rows in the live database.
pub async fn counts(db: &Db) -> Result<Counts> {
    async fn n(db: &Db, sql: &'static str) -> i64 {
        sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(&db.pool)
            .await
            .unwrap_or(0)
    }
    let defs: Vec<String> = sqlx::query_scalar(
        "SELECT value FROM setting WHERE key IN \
         ('extra_fields_vendor', 'extra_fields_filament', 'extra_fields_spool')",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap_or_default();
    Ok(Counts {
        vendors: n(db, "SELECT COUNT(*) FROM vendor").await,
        filaments: n(db, "SELECT COUNT(*) FROM filament").await,
        spools: n(db, "SELECT COUNT(*) FROM spool").await,
        archived_spools: n(db, "SELECT COUNT(*) FROM spool WHERE archived = TRUE").await,
        extra_fields: defs
            .iter()
            .map(|v| {
                serde_json::from_str::<Value>(v)
                    .ok()
                    .and_then(|j| j.as_array().map(|a| a.len() as i64))
                    .unwrap_or(0)
            })
            .sum(),
    })
}

/// A SQLite config whose `spoolman.db` lives in `dir` (for migrating a staged copy).
pub fn sqlite_config(dir: &Path) -> SpoolmanConfig {
    SpoolmanConfig {
        db_type: DbType::Sqlite,
        db_host: None,
        db_port: None,
        db_name: None,
        db_username: None,
        db_password: None,
        data_dir: dir.to_path_buf(),
        logs_dir: dir.to_path_buf(),
        backups_dir: dir.join("backups"),
        host: "127.0.0.1".into(),
        port: 0,
        base_path: String::new(),
        automatic_backup: false,
    }
}

/// Migrate `<dir>/spoolman.db` to HEAD in place and close it again.
pub async fn migrate_to_head(dir: &Path) -> Result<()> {
    let db = Db::connect(&sqlite_config(dir)).await?;
    // Fold the WAL back into the file so ATTACH sees every row.
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_optional(&db.pool)
        .await?;
    db.pool.close().await;
    Ok(())
}

async fn columns(conn: &mut sqlx::AnyConnection, schema: &str, table: &str) -> Result<Vec<String>> {
    let sql = format!("SELECT name FROM pragma_table_info('{table}', '{schema}')");
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .fetch_all(&mut *conn)
        .await?;
    rows.iter()
        .map(|r| r.try_get::<String, _>(0).map_err(Into::into))
        .collect()
}

/// Replace every Spoolman row in `db` with the rows of the HEAD-revision SQLite file at
/// `src` (one transaction: all or nothing). The live database must be SQLite.
pub async fn replace_all_from(db: &Db, src: &Path) -> Result<Counts> {
    if db.engine != Engine::Sqlite {
        return Err(SpoolmanError::BadRequest(
            "The built-in Spoolman runs on PostgreSQL or MySQL here. Importing a file needs it on SQLite \
             (the default). Import into that database with Spoolman's own tools instead."
                .into(),
        ));
    }
    let (status, _) = {
        let insp = inspect_sqlite(src).await?;
        classify(insp.revision.as_deref())
    };
    if status != RevisionStatus::Head {
        return Err(SpoolmanError::BadRequest(
            "The staged copy is not at the current revision; detect the file again.".into(),
        ));
    }
    let mut conn = db.pool.acquire().await?;
    let path = src.to_string_lossy().into_owned();
    sqlx::query("ATTACH DATABASE ? AS src")
        .bind(path)
        .execute(&mut *conn)
        .await?;
    let res = copy_tables(&mut conn).await;
    let detach = sqlx::query("DETACH DATABASE src").execute(&mut *conn).await;
    res?;
    detach?;
    drop(conn);
    counts(db).await
}

async fn copy_tables(conn: &mut sqlx::AnyConnection) -> Result<()> {
    let mut plan = Vec::new();
    for t in TABLES {
        let live = columns(conn, "main", t).await?;
        let src = columns(conn, "src", t).await?;
        let shared: Vec<String> = live.into_iter().filter(|c| src.contains(c)).collect();
        plan.push((*t, shared));
    }
    let mut tx = conn.begin().await?;
    for (t, _) in plan.iter().rev() {
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM main.{t}")))
            .execute(&mut *tx)
            .await?;
    }
    for (t, cols) in &plan {
        if cols.is_empty() {
            continue;
        }
        let list = cols
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO main.{t} ({list}) SELECT {list} FROM src.{t}"
        )))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// From another Spoolman's REST API
// ---------------------------------------------------------------------------

/// Everything read from `GET /api/v1/{vendor,filament,spool,setting/}`.
#[derive(Debug, Default)]
pub struct ApiDump {
    pub vendors: Vec<Value>,
    pub filaments: Vec<Value>,
    pub spools: Vec<Value>,
    /// `{key: {value, is_set, type}}` as `/api/v1/setting/` returns it.
    pub settings: serde_json::Map<String, Value>,
}

/// API datetimes (`2026-03-24T17:52:29Z`) → the stored form (`2026-03-24 17:52:29.000000`).
pub fn api_dt(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    match chrono::DateTime::parse_from_rfc3339(s) {
        Ok(dt) => Some(
            dt.with_timezone(&chrono::Utc)
                .format("%Y-%m-%d %H:%M:%S%.6f")
                .to_string(),
        ),
        Err(_) => Some(s.replacen('T', " ", 1).trim_end_matches('Z').to_owned()),
    }
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}
fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}
fn int(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(Value::as_i64)
}

fn extras(v: &Value) -> Vec<(String, String)> {
    v.get("extra")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

/// Write `dump` into a fresh Spoolman database at `<dir>/spoolman.db` (created at HEAD).
pub async fn build_from_api(dir: &Path, dump: &ApiDump) -> Result<()> {
    let db = Db::connect(&sqlite_config(dir)).await?;
    let res = write_dump(&db, dump).await;
    if res.is_ok() {
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .fetch_optional(&db.pool)
            .await?;
    }
    db.pool.close().await;
    res
}

async fn write_dump(db: &Db, dump: &ApiDump) -> Result<()> {
    let mut tx = db.pool.begin().await?;
    let bad = |what: &str| SpoolmanError::BadRequest(format!("{what} without an id"));
    for v in &dump.vendors {
        let id = int(v, "id").ok_or_else(|| bad("vendor"))?;
        sqlx::query(
            "INSERT INTO vendor (id, registered, name, comment, empty_spool_weight, external_id) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(v.get("registered").and_then(api_dt).unwrap_or_default())
        .bind(text(v, "name").unwrap_or_default())
        .bind(text(v, "comment"))
        .bind(num(v, "empty_spool_weight"))
        .bind(text(v, "external_id"))
        .execute(&mut *tx)
        .await?;
        for (k, val) in extras(v) {
            sqlx::query("INSERT INTO vendor_field (vendor_id, key, value) VALUES (?, ?, ?)")
                .bind(id)
                .bind(k)
                .bind(val)
                .execute(&mut *tx)
                .await?;
        }
    }
    for v in &dump.filaments {
        let id = int(v, "id").ok_or_else(|| bad("filament"))?;
        let vendor_id = v.get("vendor").and_then(|x| int(x, "id"));
        sqlx::query(
            "INSERT INTO filament (id, registered, name, vendor_id, material, price, density, diameter, \
             weight, spool_weight, article_number, comment, settings_extruder_temp, settings_bed_temp, \
             color_hex, multi_color_hexes, multi_color_direction, external_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(v.get("registered").and_then(api_dt).unwrap_or_default())
        .bind(text(v, "name"))
        .bind(vendor_id)
        .bind(text(v, "material"))
        .bind(num(v, "price"))
        .bind(num(v, "density").unwrap_or(1.24))
        .bind(num(v, "diameter").unwrap_or(1.75))
        .bind(num(v, "weight"))
        .bind(num(v, "spool_weight"))
        .bind(text(v, "article_number"))
        .bind(text(v, "comment"))
        .bind(int(v, "settings_extruder_temp"))
        .bind(int(v, "settings_bed_temp"))
        .bind(text(v, "color_hex"))
        .bind(text(v, "multi_color_hexes"))
        .bind(text(v, "multi_color_direction"))
        .bind(text(v, "external_id"))
        .execute(&mut *tx)
        .await?;
        for (k, val) in extras(v) {
            sqlx::query("INSERT INTO filament_field (filament_id, key, value) VALUES (?, ?, ?)")
                .bind(id)
                .bind(k)
                .bind(val)
                .execute(&mut *tx)
                .await?;
        }
    }
    for v in &dump.spools {
        let id = int(v, "id").ok_or_else(|| bad("spool"))?;
        let filament_id = v
            .get("filament")
            .and_then(|x| int(x, "id"))
            .ok_or_else(|| bad("spool filament"))?;
        sqlx::query(
            "INSERT INTO spool (id, registered, first_used, last_used, filament_id, price, initial_weight, \
             spool_weight, used_weight, location, lot_nr, comment, archived) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(v.get("registered").and_then(api_dt).unwrap_or_default())
        .bind(v.get("first_used").and_then(api_dt))
        .bind(v.get("last_used").and_then(api_dt))
        .bind(filament_id)
        .bind(num(v, "price"))
        .bind(num(v, "initial_weight"))
        .bind(num(v, "spool_weight"))
        .bind(num(v, "used_weight").unwrap_or(0.0))
        .bind(text(v, "location"))
        .bind(text(v, "lot_nr"))
        .bind(text(v, "comment"))
        .bind(v.get("archived").and_then(Value::as_bool).unwrap_or(false))
        .execute(&mut *tx)
        .await?;
        for (k, val) in extras(v) {
            sqlx::query("INSERT INTO spool_field (spool_id, key, value) VALUES (?, ?, ?)")
                .bind(id)
                .bind(k)
                .bind(val)
                .execute(&mut *tx)
                .await?;
        }
    }
    let now = chrono::Utc::now()
        .format("%Y-%m-%d %H:%M:%S%.6f")
        .to_string();
    for (key, s) in &dump.settings {
        if !s.get("is_set").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let Some(value) = s.get("value").and_then(Value::as_str) else {
            continue;
        };
        sqlx::query("INSERT INTO setting (key, value, last_updated) VALUES (?, ?, ?)")
            .bind(key.clone())
            .bind(value.to_owned())
            .bind(now.clone())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
