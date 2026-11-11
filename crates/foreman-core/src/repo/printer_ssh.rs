//! Repository for the `printer_ssh` table.
//!
//! Each printer has at most one SSH config row (enforced by the UNIQUE constraint on
//! `printer_id`).  `upsert` creates the row on first call and updates it on subsequent
//! ones, preserving `id` and `created_at`.

use chrono::Utc;
use sqlx::{FromRow, SqlitePool};

use crate::{
    domain::{PrinterSsh, UpsertPrinterSsh},
    error::AppError,
};

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

#[derive(FromRow)]
struct SshRow {
    id: i64,
    printer_id: i64,
    ssh_user: String,
    ssh_port: i64,
    auth_type: String,
    // password intentionally not selected — never returned over the API
    host_key_fingerprint: Option<String>,
    pending_host_key_fingerprint: Option<String>,
    has_password: i64,
    idle_timeout_secs: i64,
    created_at: String,
    updated_at: String,
}

fn parse_dt(s: &str) -> chrono::DateTime<Utc> {
    s.parse().unwrap_or_else(|_| Utc::now())
}

impl SshRow {
    fn into_domain(self) -> PrinterSsh {
        PrinterSsh {
            id: self.id,
            printer_id: self.printer_id,
            ssh_user: self.ssh_user,
            ssh_port: self.ssh_port,
            auth_type: self.auth_type,
            host_key_fingerprint: self.host_key_fingerprint,
            pending_host_key_fingerprint: self.pending_host_key_fingerprint,
            has_password: self.has_password != 0,
            idle_timeout_secs: self.idle_timeout_secs,
            created_at: parse_dt(&self.created_at),
            updated_at: parse_dt(&self.updated_at),
        }
    }
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

const SQL_GET: &str =
    "SELECT id, printer_id, ssh_user, ssh_port, auth_type, host_key_fingerprint, \
     pending_host_key_fingerprint, (password IS NOT NULL) AS has_password, \
     idle_timeout_secs, created_at, updated_at \
     FROM printer_ssh WHERE printer_id = ?";

/// Fetch SSH settings for a printer.  Returns `NotFound` if none are configured.
pub async fn get(pool: &SqlitePool, printer_id: i64) -> Result<PrinterSsh, AppError> {
    let row: Option<SshRow> = sqlx::query_as(SQL_GET)
        .bind(printer_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

    row.map(SshRow::into_domain)
        .ok_or_else(|| AppError::NotFound(format!("printer_ssh for printer {printer_id}")))
}

/// Fetch the sealed (encrypted) password for a printer; decrypt with `SecretBox::open`.
pub async fn get_sealed_password(
    pool: &SqlitePool,
    printer_id: i64,
) -> Result<Option<String>, AppError> {
    let pw: Option<String> =
        sqlx::query_scalar("SELECT password FROM printer_ssh WHERE printer_id = ?")
            .bind(printer_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
            .flatten();
    Ok(pw)
}

/// Create or update SSH settings for a printer.
///
/// `sealed_password` must already be encrypted; `None` keeps the stored password.
/// `req.password` is ignored here (the API layer seals it).
pub async fn upsert(
    pool: &SqlitePool,
    printer_id: i64,
    req: UpsertPrinterSsh,
    sealed_password: Option<String>,
) -> Result<PrinterSsh, AppError> {
    let port = req.ssh_port.unwrap_or(22);
    let auth_type = req.auth_type.as_deref().unwrap_or("key").to_owned();
    if auth_type != "key" && auth_type != "password" {
        return Err(AppError::InvalidInput(format!(
            "auth_type must be 'key' or 'password', got '{auth_type}'"
        )));
    }
    if !(1..=65535).contains(&port) {
        return Err(AppError::InvalidInput(format!("invalid SSH port {port}")));
    }
    if req.ssh_user.trim().is_empty() {
        return Err(AppError::InvalidInput("SSH user is required".into()));
    }
    if req.idle_timeout_secs.is_some_and(|t| t < 0) {
        return Err(AppError::InvalidInput("idle timeout must be >= 0".into()));
    }
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO printer_ssh \
           (printer_id, ssh_user, ssh_port, auth_type, password, idle_timeout_secs, \
            created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, COALESCE(?, 900), ?, ?) \
         ON CONFLICT(printer_id) DO UPDATE SET \
           ssh_user   = excluded.ssh_user, \
           ssh_port   = excluded.ssh_port, \
           auth_type  = excluded.auth_type, \
           password   = COALESCE(excluded.password, printer_ssh.password), \
           idle_timeout_secs = COALESCE(?, printer_ssh.idle_timeout_secs), \
           updated_at = excluded.updated_at",
    )
    .bind(printer_id)
    .bind(req.ssh_user.trim())
    .bind(port)
    .bind(&auth_type)
    .bind(sealed_password.as_deref())
    .bind(req.idle_timeout_secs)
    .bind(&now)
    .bind(&now)
    .bind(req.idle_timeout_secs)
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;

    get(pool, printer_id).await
}

/// Remove SSH settings for a printer.
pub async fn delete(pool: &SqlitePool, printer_id: i64) -> Result<(), AppError> {
    let n = sqlx::query("DELETE FROM printer_ssh WHERE printer_id = ?")
        .bind(printer_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?
        .rows_affected();
    if n == 0 {
        Err(AppError::NotFound(format!(
            "printer_ssh for printer {printer_id}"
        )))
    } else {
        Ok(())
    }
}

/// Store the host key fingerprint captured during the first (TOFU) connection.
pub async fn store_host_key(
    pool: &SqlitePool,
    printer_id: i64,
    fingerprint: &str,
) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE printer_ssh SET host_key_fingerprint = ?, pending_host_key_fingerprint = NULL, \
         updated_at = ? WHERE printer_id = ?",
    )
    .bind(fingerprint)
    .bind(&now)
    .bind(printer_id)
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}

/// Record a changed host key presented by the server (awaiting user approval).
pub async fn set_pending_host_key(
    pool: &SqlitePool,
    printer_id: i64,
    fingerprint: &str,
) -> Result<(), AppError> {
    sqlx::query("UPDATE printer_ssh SET pending_host_key_fingerprint = ? WHERE printer_id = ?")
        .bind(fingerprint)
        .bind(printer_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}

/// Promote the pending host key to pinned. Returns the newly pinned fingerprint.
pub async fn trust_pending_host_key(
    pool: &SqlitePool,
    printer_id: i64,
) -> Result<String, AppError> {
    let cfg = get(pool, printer_id).await?;
    let fp = cfg.pending_host_key_fingerprint.ok_or_else(|| {
        AppError::InvalidInput("no changed host key is waiting for approval".into())
    })?;
    store_host_key(pool, printer_id, &fp).await?;
    Ok(fp)
}

/// Clear the pinned host key (lets the next connect re-pin via TOFU).
pub async fn clear_host_key(pool: &SqlitePool, printer_id: i64) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE printer_ssh SET host_key_fingerprint = NULL, pending_host_key_fingerprint = NULL, \
         updated_at = ? WHERE printer_id = ?",
    )
    .bind(&now)
    .bind(printer_id)
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
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
                primary_url: Some("http://10.0.0.177:7125".to_owned()),
                api_key: None,
                serial_number: None,
                access_code: None,
                tags: vec![],
                hardened_nozzle: false,
                slicer_profiles: vec![],
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
    async fn upsert_creates_and_returns() {
        let (db, _dir) = db().await;
        let pid = create_printer(&db.pool).await;

        let cfg = upsert(
            &db.pool,
            pid,
            UpsertPrinterSsh {
                ssh_user: "pi".to_owned(),
                ssh_port: Some(22),
                auth_type: Some("key".to_owned()),
                password: None,
                idle_timeout_secs: None,
            },
            None,
        )
        .await
        .expect("upsert");

        assert_eq!(cfg.ssh_user, "pi");
        assert_eq!(cfg.ssh_port, 22);
        assert_eq!(cfg.auth_type, "key");
        assert_eq!(cfg.idle_timeout_secs, 900);
        assert!(cfg.host_key_fingerprint.is_none());
    }

    #[tokio::test]
    async fn upsert_updates_user() {
        let (db, _dir) = db().await;
        let pid = create_printer(&db.pool).await;

        upsert(
            &db.pool,
            pid,
            UpsertPrinterSsh {
                ssh_user: "pi".to_owned(),
                ssh_port: None,
                auth_type: None,
                password: None,
                idle_timeout_secs: Some(0),
            },
            None,
        )
        .await
        .expect("first upsert");

        let updated = upsert(
            &db.pool,
            pid,
            UpsertPrinterSsh {
                ssh_user: "klipper".to_owned(),
                ssh_port: Some(2222),
                auth_type: None,
                password: None,
                idle_timeout_secs: None,
            },
            None,
        )
        .await
        .expect("second upsert");

        assert_eq!(updated.ssh_user, "klipper");
        assert_eq!(updated.ssh_port, 2222);
        assert_eq!(
            updated.idle_timeout_secs, 0,
            "omitted timeout keeps stored value"
        );
    }

    #[tokio::test]
    async fn store_and_clear_host_key() {
        let (db, _dir) = db().await;
        let pid = create_printer(&db.pool).await;

        upsert(
            &db.pool,
            pid,
            UpsertPrinterSsh {
                ssh_user: "pi".to_owned(),
                ssh_port: None,
                auth_type: None,
                password: None,
                idle_timeout_secs: None,
            },
            None,
        )
        .await
        .expect("upsert");

        let fp = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        store_host_key(&db.pool, pid, fp).await.expect("store");
        let cfg = get(&db.pool, pid).await.expect("get");
        assert_eq!(cfg.host_key_fingerprint.as_deref(), Some(fp));

        // Changed key is held as pending until trusted.
        assert!(trust_pending_host_key(&db.pool, pid).await.is_err());
        set_pending_host_key(&db.pool, pid, "SHA256:new")
            .await
            .expect("pending");
        let cfg = get(&db.pool, pid).await.expect("get");
        assert_eq!(cfg.host_key_fingerprint.as_deref(), Some(fp));
        assert_eq!(
            cfg.pending_host_key_fingerprint.as_deref(),
            Some("SHA256:new")
        );
        let pinned = trust_pending_host_key(&db.pool, pid).await.expect("trust");
        assert_eq!(pinned, "SHA256:new");
        let cfg = get(&db.pool, pid).await.expect("get");
        assert_eq!(cfg.host_key_fingerprint.as_deref(), Some("SHA256:new"));
        assert!(cfg.pending_host_key_fingerprint.is_none());

        clear_host_key(&db.pool, pid).await.expect("clear");
        let cfg = get(&db.pool, pid).await.expect("get after clear");
        assert!(cfg.host_key_fingerprint.is_none());
    }

    #[tokio::test]
    async fn get_not_found_when_unconfigured() {
        let (db, _dir) = db().await;
        let pid = create_printer(&db.pool).await;
        let err = get(&db.pool, pid).await;
        assert!(matches!(err, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn password_kept_when_omitted_and_flagged() {
        let (db, _dir) = db().await;
        let pid = create_printer(&db.pool).await;
        let req = || UpsertPrinterSsh {
            ssh_user: "pi".to_owned(),
            ssh_port: None,
            auth_type: Some("password".to_owned()),
            password: None,
            idle_timeout_secs: None,
        };
        let cfg = upsert(&db.pool, pid, req(), Some("enc:v1:abc".into()))
            .await
            .expect("upsert");
        assert!(cfg.has_password);
        upsert(&db.pool, pid, req(), None).await.expect("upsert 2");
        assert_eq!(
            get_sealed_password(&db.pool, pid)
                .await
                .expect("pw")
                .as_deref(),
            Some("enc:v1:abc")
        );
        let bad = UpsertPrinterSsh {
            auth_type: Some("magic".into()),
            ..req()
        };
        assert!(upsert(&db.pool, pid, bad, None).await.is_err());
    }
}
