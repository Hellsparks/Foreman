//! Repository functions for the `printer_path` table.
//!
//! Paths are ordered by `priority` ascending (lower = tried first).

use chrono::Utc;
use sqlx::{FromRow, SqlitePool};

use crate::{
    domain::{CreatePrinterPath, PrinterPath, UpdatePrinterPath},
    error::AppError,
};

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

#[derive(FromRow)]
struct PathRow {
    id: i64,
    printer_id: i64,
    name: String,
    url: String,
    priority: i64,
    created_at: String,
    updated_at: String,
}

fn parse_dt(s: &str) -> chrono::DateTime<Utc> {
    s.parse().unwrap_or_else(|_| Utc::now())
}

impl PathRow {
    fn into_domain(self) -> PrinterPath {
        PrinterPath {
            id: self.id,
            printer_id: self.printer_id,
            name: self.name,
            url: self.url,
            priority: self.priority,
            created_at: parse_dt(&self.created_at),
            updated_at: parse_dt(&self.updated_at),
        }
    }
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

const SQL_LIST: &str = "SELECT id, printer_id, name, url, priority, created_at, updated_at \
     FROM printer_path WHERE printer_id=? ORDER BY priority ASC, id ASC";

const SQL_GET: &str = "SELECT id, printer_id, name, url, priority, created_at, updated_at \
     FROM printer_path WHERE id=?";

/// Return all paths for a printer, ordered by `priority` ascending.
pub async fn list(pool: &SqlitePool, printer_id: i64) -> Result<Vec<PrinterPath>, AppError> {
    let rows: Vec<PathRow> = sqlx::query_as(SQL_LIST)
        .bind(printer_id)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

    Ok(rows.into_iter().map(PathRow::into_domain).collect())
}

/// Return a single path by id.
pub async fn get(pool: &SqlitePool, id: i64) -> Result<PrinterPath, AppError> {
    let row: Option<PathRow> = sqlx::query_as(SQL_GET)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

    row.map(PathRow::into_domain)
        .ok_or_else(|| AppError::NotFound(format!("printer_path {id}")))
}

/// Insert a new path for a printer and return the created record.
pub async fn create(pool: &SqlitePool, req: CreatePrinterPath) -> Result<PrinterPath, AppError> {
    let now = Utc::now().to_rfc3339();
    let priority = req.priority.unwrap_or(0);

    let id = sqlx::query(
        "INSERT INTO printer_path (printer_id, name, url, priority, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(req.printer_id)
    .bind(&req.name)
    .bind(&req.url)
    .bind(priority)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?
    .last_insert_rowid();

    get(pool, id).await
}

/// Update fields on an existing path.  `None` fields are left unchanged.
pub async fn update(
    pool: &SqlitePool,
    id: i64,
    req: UpdatePrinterPath,
) -> Result<PrinterPath, AppError> {
    let current = get(pool, id).await?;

    let name = req.name.unwrap_or(current.name);
    let url = req.url.unwrap_or(current.url);
    let priority = req.priority.unwrap_or(current.priority);
    let now = Utc::now().to_rfc3339();

    sqlx::query("UPDATE printer_path SET name=?, url=?, priority=?, updated_at=? WHERE id=?")
        .bind(&name)
        .bind(&url)
        .bind(priority)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

    get(pool, id).await
}

/// Delete a path.
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<(), AppError> {
    let affected = sqlx::query("DELETE FROM printer_path WHERE id=?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?
        .rows_affected();

    if affected == 0 {
        Err(AppError::NotFound(format!("printer_path {id}")))
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
    use crate::{
        db::ForemanDb,
        domain::{CreatePrinter, FirmwareKind},
        repo::printer,
    };
    use tempfile::TempDir;

    async fn db() -> (ForemanDb, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let db = ForemanDb::connect(&dir.path().join("foreman.db"))
            .await
            .expect("connect");
        (db, dir)
    }

    async fn create_printer(pool: &SqlitePool) -> i64 {
        printer::create(
            pool,
            CreatePrinter {
                name: "TestPrinter".to_owned(),
                firmware: FirmwareKind::Moonraker,
                primary_url: None,
                api_key: None,
                serial_number: None,
                access_code: None,
                tags: vec![],
                hardened_nozzle: false,
                sort_order: None,
                fail_after_checks: None,
                return_after_secs: None,
            },
        )
        .await
        .expect("create printer")
        .id
    }

    #[tokio::test]
    async fn create_and_list_paths() {
        let (db, _dir) = db().await;
        let pid = create_printer(&db.pool).await;

        create(
            &db.pool,
            CreatePrinterPath {
                printer_id: pid,
                name: "Local".to_owned(),
                url: "http://10.0.0.177".to_owned(),
                priority: Some(0),
            },
        )
        .await
        .expect("create local");

        create(
            &db.pool,
            CreatePrinterPath {
                printer_id: pid,
                name: "Tailscale".to_owned(),
                url: "http://100.64.0.1".to_owned(),
                priority: Some(10),
            },
        )
        .await
        .expect("create tailscale");

        let paths = list(&db.pool, pid).await.expect("list");
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].name, "Local");
        assert_eq!(paths[1].name, "Tailscale");
    }

    #[tokio::test]
    async fn update_path_url() {
        let (db, _dir) = db().await;
        let pid = create_printer(&db.pool).await;

        let path = create(
            &db.pool,
            CreatePrinterPath {
                printer_id: pid,
                name: "Local".to_owned(),
                url: "http://old-url".to_owned(),
                priority: None,
            },
        )
        .await
        .expect("create");

        let updated = update(
            &db.pool,
            path.id,
            UpdatePrinterPath {
                name: None,
                url: Some("http://new-url".to_owned()),
                priority: None,
            },
        )
        .await
        .expect("update");

        assert_eq!(updated.url, "http://new-url");
        assert_eq!(updated.name, "Local");
    }

    #[tokio::test]
    async fn delete_path() {
        let (db, _dir) = db().await;
        let pid = create_printer(&db.pool).await;

        let path = create(
            &db.pool,
            CreatePrinterPath {
                printer_id: pid,
                name: "Local".to_owned(),
                url: "http://10.0.0.177".to_owned(),
                priority: None,
            },
        )
        .await
        .expect("create");

        delete(&db.pool, path.id).await.expect("delete");

        let err = get(&db.pool, path.id).await;
        assert!(matches!(err, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn cascade_delete_on_printer_delete() {
        let (db, _dir) = db().await;
        let pid = create_printer(&db.pool).await;

        let path = create(
            &db.pool,
            CreatePrinterPath {
                printer_id: pid,
                name: "Local".to_owned(),
                url: "http://10.0.0.177".to_owned(),
                priority: None,
            },
        )
        .await
        .expect("create path");

        printer::delete(&db.pool, pid)
            .await
            .expect("delete printer");

        let err = get(&db.pool, path.id).await;
        assert!(matches!(err, Err(AppError::NotFound(_))));
    }
}
