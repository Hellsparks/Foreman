//! Repository functions for the `printer` table.
//!
//! Uses the sqlx runtime query API (`sqlx::query()` / `sqlx::query_as()`).
//! No `query!()` macro — no DATABASE_URL required at compile time.

use chrono::Utc;
use sqlx::{FromRow, SqlitePool};

use crate::{
    domain::{CreatePrinter, FirmwareKind, Printer, UpdatePrinter, UsageReporter},
    error::AppError,
};

// ---------------------------------------------------------------------------
// Row type (maps directly onto the SQL columns)
// ---------------------------------------------------------------------------

#[derive(FromRow)]
struct PrinterRow {
    id: i64,
    name: String,
    firmware: String,
    primary_url: Option<String>,
    api_key: Option<String>,
    serial_number: Option<String>,
    access_code: Option<String>,
    tls_fingerprint: Option<String>,
    tags: String,
    hardened_nozzle: i64,
    slicer_profiles: String,
    sort_order: i64,
    fail_after_checks: i64,
    return_after_secs: i64,
    plate_clear: i64,
    auto_start_from_queue: i64,
    usage_reporter: String,
    created_at: String,
    updated_at: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_firmware(s: &str) -> Result<FirmwareKind, AppError> {
    match s {
        "moonraker" => Ok(FirmwareKind::Moonraker),
        "octoprint" => Ok(FirmwareKind::Octoprint),
        "bambu" => Ok(FirmwareKind::Bambu),
        "duet" => Ok(FirmwareKind::Duet),
        other => Err(AppError::Database(format!(
            "unknown firmware kind: {other}"
        ))),
    }
}

fn firmware_str(k: &FirmwareKind) -> &'static str {
    match k {
        FirmwareKind::Moonraker => "moonraker",
        FirmwareKind::Octoprint => "octoprint",
        FirmwareKind::Bambu => "bambu",
        FirmwareKind::Duet => "duet",
    }
}

fn parse_tags(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

fn tags_json(tags: &[String]) -> String {
    serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_owned())
}

fn parse_dt(s: &str) -> chrono::DateTime<Utc> {
    s.parse().unwrap_or_else(|_| Utc::now())
}

impl PrinterRow {
    fn into_domain(self) -> Result<Printer, AppError> {
        Ok(Printer {
            id: self.id,
            name: self.name,
            firmware: parse_firmware(&self.firmware)?,
            primary_url: self.primary_url,
            api_key: self.api_key,
            serial_number: self.serial_number,
            access_code: self.access_code,
            tls_fingerprint: self.tls_fingerprint,
            tags: parse_tags(&self.tags),
            hardened_nozzle: self.hardened_nozzle != 0,
            slicer_profiles: parse_tags(&self.slicer_profiles),
            sort_order: self.sort_order,
            fail_after_checks: self.fail_after_checks,
            return_after_secs: self.return_after_secs,
            plate_clear: self.plate_clear != 0,
            auto_start_from_queue: self.auto_start_from_queue != 0,
            usage_reporter: UsageReporter::parse(&self.usage_reporter),
            created_at: parse_dt(&self.created_at),
            updated_at: parse_dt(&self.updated_at),
        })
    }
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

const SQL_LIST: &str =
    "SELECT id, name, firmware, primary_url, api_key, serial_number, access_code, \
     tls_fingerprint, tags, hardened_nozzle, slicer_profiles, sort_order, \
     fail_after_checks, return_after_secs, plate_clear, auto_start_from_queue, \
     usage_reporter, created_at, updated_at \
     FROM printer ORDER BY sort_order ASC, id ASC";

const SQL_GET: &str =
    "SELECT id, name, firmware, primary_url, api_key, serial_number, access_code, \
     tls_fingerprint, tags, hardened_nozzle, slicer_profiles, sort_order, \
     fail_after_checks, return_after_secs, plate_clear, auto_start_from_queue, \
     usage_reporter, created_at, updated_at \
     FROM printer WHERE id=?";

/// Return all printers ordered by `sort_order`, then `id`.
pub async fn list(pool: &SqlitePool) -> Result<Vec<Printer>, AppError> {
    let rows: Vec<PrinterRow> = sqlx::query_as(SQL_LIST)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

    rows.into_iter().map(|r| r.into_domain()).collect()
}

/// Return a single printer by id.
pub async fn get(pool: &SqlitePool, id: i64) -> Result<Printer, AppError> {
    let row: Option<PrinterRow> = sqlx::query_as(SQL_GET)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

    row.ok_or_else(|| AppError::NotFound(format!("printer {id}")))
        .and_then(|r| r.into_domain())
}

/// Insert a new printer and return the created record.
pub async fn create(pool: &SqlitePool, req: CreatePrinter) -> Result<Printer, AppError> {
    let now = Utc::now().to_rfc3339();
    let firmware = firmware_str(&req.firmware).to_owned();
    let tags = tags_json(&req.tags);
    let hardened: i64 = req.hardened_nozzle as i64;
    let slicer_profiles = tags_json(&req.slicer_profiles);
    let sort_order = req.sort_order.unwrap_or(0);
    let fail_after_checks = req.fail_after_checks.unwrap_or(3);
    let return_after_secs = req.return_after_secs.unwrap_or(60);

    let id = sqlx::query(
        "INSERT INTO printer \
         (name, firmware, primary_url, api_key, serial_number, access_code, \
          tags, hardened_nozzle, slicer_profiles, sort_order, \
          fail_after_checks, return_after_secs, \
          created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&req.name)
    .bind(&firmware)
    .bind(&req.primary_url)
    .bind(&req.api_key)
    .bind(&req.serial_number)
    .bind(&req.access_code)
    .bind(&tags)
    .bind(hardened)
    .bind(&slicer_profiles)
    .bind(sort_order)
    .bind(fail_after_checks)
    .bind(return_after_secs)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?
    .last_insert_rowid();

    get(pool, id).await
}

/// Update fields on an existing printer.  `None` fields are left unchanged.
pub async fn update(pool: &SqlitePool, id: i64, req: UpdatePrinter) -> Result<Printer, AppError> {
    let current = get(pool, id).await?;

    let name = req.name.unwrap_or(current.name);
    let firmware = firmware_str(&req.firmware.unwrap_or(current.firmware)).to_owned();
    let primary_url = req.primary_url.or(current.primary_url);
    let api_key = req.api_key.or(current.api_key);
    let serial_number = req.serial_number.or(current.serial_number);
    let access_code = req.access_code.or(current.access_code);
    let tags = tags_json(&req.tags.unwrap_or(current.tags));
    let hardened: i64 = req.hardened_nozzle.unwrap_or(current.hardened_nozzle) as i64;
    let slicer_profiles = tags_json(&req.slicer_profiles.unwrap_or(current.slicer_profiles));
    let sort_order = req.sort_order.unwrap_or(current.sort_order);
    let fail_after_checks = req.fail_after_checks.unwrap_or(current.fail_after_checks);
    let return_after_secs = req.return_after_secs.unwrap_or(current.return_after_secs);
    let plate_clear: i64 = req.plate_clear.unwrap_or(current.plate_clear) as i64;
    let auto_start: i64 = req
        .auto_start_from_queue
        .unwrap_or(current.auto_start_from_queue) as i64;
    let usage_reporter = req
        .usage_reporter
        .unwrap_or(current.usage_reporter)
        .as_str();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "UPDATE printer \
         SET name=?, firmware=?, primary_url=?, api_key=?, serial_number=?, \
             access_code=?, tags=?, hardened_nozzle=?, slicer_profiles=?, \
             sort_order=?, fail_after_checks=?, return_after_secs=?, \
             plate_clear=?, auto_start_from_queue=?, usage_reporter=?, updated_at=? \
         WHERE id=?",
    )
    .bind(&name)
    .bind(&firmware)
    .bind(&primary_url)
    .bind(&api_key)
    .bind(&serial_number)
    .bind(&access_code)
    .bind(&tags)
    .bind(hardened)
    .bind(&slicer_profiles)
    .bind(sort_order)
    .bind(fail_after_checks)
    .bind(return_after_secs)
    .bind(plate_clear)
    .bind(auto_start)
    .bind(usage_reporter)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;

    get(pool, id).await
}

/// Store the TLS certificate fingerprint for a printer (TOFU pinning).
pub async fn update_tls_fingerprint(
    pool: &SqlitePool,
    id: i64,
    fingerprint: &str,
) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE printer SET tls_fingerprint=?, updated_at=? WHERE id=?")
        .bind(fingerprint)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}

/// Clear the TLS certificate fingerprint (re-TOFU on next connect).
pub async fn clear_tls_fingerprint(pool: &SqlitePool, id: i64) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE printer SET tls_fingerprint=NULL, updated_at=? WHERE id=?")
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}

/// Set the plate_clear flag on a printer (used by attention actions and the scheduler).
pub async fn set_plate_clear(pool: &SqlitePool, id: i64, clear: bool) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    let val: i64 = clear as i64;
    sqlx::query("UPDATE printer SET plate_clear=?, updated_at=? WHERE id=?")
        .bind(val)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}

/// Delete a printer (and its cascading children).
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<(), AppError> {
    let affected = sqlx::query("DELETE FROM printer WHERE id=?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?
        .rows_affected();

    if affected == 0 {
        Err(AppError::NotFound(format!("printer {id}")))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ForemanDb;
    use tempfile::TempDir;

    async fn db() -> (ForemanDb, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let db = ForemanDb::connect(&dir.path().join("foreman.db"))
            .await
            .expect("connect");
        (db, dir)
    }

    fn sample_create() -> CreatePrinter {
        CreatePrinter {
            name: "HellV0".to_owned(),
            firmware: FirmwareKind::Moonraker,
            primary_url: Some("http://10.0.0.177".to_owned()),
            api_key: None,
            serial_number: None,
            access_code: None,
            tags: vec!["klipper".to_owned()],
            hardened_nozzle: false,
            slicer_profiles: vec![],
            sort_order: Some(0),
            fail_after_checks: None,
            return_after_secs: None,
        }
    }

    #[tokio::test]
    async fn create_and_get() {
        let (db, _dir) = db().await;
        let p = create(&db.pool, sample_create()).await.expect("create");
        assert_eq!(p.name, "HellV0");
        assert_eq!(p.firmware, FirmwareKind::Moonraker);
        assert_eq!(p.tags, vec!["klipper"]);
        assert!(!p.hardened_nozzle);

        let fetched = get(&db.pool, p.id).await.expect("get");
        assert_eq!(fetched.id, p.id);
        assert_eq!(fetched.name, "HellV0");
    }

    #[tokio::test]
    async fn list_returns_all() {
        let (db, _dir) = db().await;
        create(&db.pool, sample_create()).await.expect("create 1");
        let mut second = sample_create();
        second.name = "BambuA1".to_owned();
        second.firmware = FirmwareKind::Bambu;
        create(&db.pool, second).await.expect("create 2");

        let printers = list(&db.pool).await.expect("list");
        assert_eq!(printers.len(), 2);
    }

    #[tokio::test]
    async fn update_name_and_tags() {
        let (db, _dir) = db().await;
        let p = create(&db.pool, sample_create()).await.expect("create");

        let updated = update(
            &db.pool,
            p.id,
            UpdatePrinter {
                name: Some("HellV0-renamed".to_owned()),
                tags: Some(vec!["klipper".to_owned(), "voron".to_owned()]),
                firmware: None,
                primary_url: None,
                api_key: None,
                serial_number: None,
                access_code: None,
                hardened_nozzle: None,
                slicer_profiles: None,
                sort_order: None,
                fail_after_checks: None,
                return_after_secs: None,
                plate_clear: None,
                auto_start_from_queue: None,
                usage_reporter: None,
            },
        )
        .await
        .expect("update");

        assert_eq!(updated.name, "HellV0-renamed");
        assert_eq!(updated.tags, vec!["klipper", "voron"]);
        assert_eq!(updated.firmware, FirmwareKind::Moonraker);
    }

    #[tokio::test]
    async fn delete_removes_printer() {
        let (db, _dir) = db().await;
        let p = create(&db.pool, sample_create()).await.expect("create");
        delete(&db.pool, p.id).await.expect("delete");

        let err = get(&db.pool, p.id).await;
        assert!(matches!(err, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn get_missing_returns_not_found() {
        let (db, _dir) = db().await;
        let err = get(&db.pool, 9999).await;
        assert!(matches!(err, Err(AppError::NotFound(_))));
    }
}
