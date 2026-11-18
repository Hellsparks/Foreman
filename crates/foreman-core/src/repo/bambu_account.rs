//! Repository for the `bambu_account` table (doc 07 §B).
//!
//! Many accounts by design: every function takes an account id, and every write touches
//! only that row. The token column holds a value sealed with `SecretBox`; it is only
//! read by [`get_sealed_token`] and never part of [`BambuAccount`].

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};
use ts_rs::TS;

use crate::{bambu_cloud::Region, error::AppError};

/// An account as the API shows it. Never carries the token.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct BambuAccount {
    #[ts(type = "number")]
    pub id: i64,
    pub label: String,
    pub region: Region,
    pub email: String,
    /// A token is stored (it may still be expired or rejected).
    pub has_token: bool,
    pub token_expires_at: Option<DateTime<Utc>>,
    pub last_ok_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    /// Printers that reference this account.
    #[ts(type = "number")]
    pub printer_count: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct Row {
    id: i64,
    label: String,
    region: String,
    email: String,
    has_token: i64,
    token_expires_at: Option<String>,
    last_ok_at: Option<String>,
    last_error: Option<String>,
    printer_count: i64,
    created_at: String,
}

fn dt(s: Option<String>) -> Option<DateTime<Utc>> {
    s.and_then(|s| s.parse().ok())
}

impl Row {
    fn into_domain(self) -> Result<BambuAccount, AppError> {
        Ok(BambuAccount {
            id: self.id,
            label: self.label,
            region: Region::parse(&self.region)
                .map_err(|e| AppError::Database(format!("bambu_account.region: {e}")))?,
            email: self.email,
            has_token: self.has_token != 0,
            token_expires_at: dt(self.token_expires_at),
            last_ok_at: dt(self.last_ok_at),
            last_error: self.last_error,
            printer_count: self.printer_count,
            created_at: dt(Some(self.created_at)).unwrap_or_else(Utc::now),
        })
    }
}

const SELECT: &str =
    "SELECT a.id, a.label, a.region, a.email, (a.token IS NOT NULL) AS has_token, \
     a.token_expires_at, a.last_ok_at, a.last_error, \
     (SELECT COUNT(*) FROM printer p WHERE p.bambu_account_id = a.id) AS printer_count, \
     a.created_at FROM bambu_account a";

fn db(e: sqlx::Error) -> AppError {
    AppError::Database(e.to_string())
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<BambuAccount>, AppError> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT} ORDER BY a.id")))
        .fetch_all(pool)
        .await
        .map_err(db)?;
    rows.into_iter().map(Row::into_domain).collect()
}

pub async fn get(pool: &SqlitePool, id: i64) -> Result<BambuAccount, AppError> {
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT} WHERE a.id = ?")))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db)?;
    row.ok_or_else(|| AppError::NotFound(format!("Bambu account {id}")))?
        .into_domain()
}

pub async fn create(
    pool: &SqlitePool,
    label: &str,
    region: Region,
    email: &str,
) -> Result<BambuAccount, AppError> {
    let label = label.trim();
    let email = email.trim();
    if email.is_empty() {
        return Err(AppError::InvalidInput("email is required".into()));
    }
    let label = if label.is_empty() { email } else { label };
    let now = Utc::now().to_rfc3339();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO bambu_account (label, region, email, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(label)
    .bind(region.as_str())
    .bind(email)
    .bind(&now)
    .bind(&now)
    .fetch_one(pool)
    .await
    .map_err(db)?;
    get(pool, id).await
}

/// Delete an account; printers that referenced it fall back to `NULL` (FK `SET NULL`).
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<(), AppError> {
    let done = sqlx::query("DELETE FROM bambu_account WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(db)?;
    if done.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Bambu account {id}")));
    }
    Ok(())
}

/// The sealed token (decrypt with `SecretBox::open`), if any.
pub async fn get_sealed_token(pool: &SqlitePool, id: i64) -> Result<Option<String>, AppError> {
    Ok(
        sqlx::query_scalar::<_, Option<String>>("SELECT token FROM bambu_account WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(db)?
            .flatten(),
    )
}

/// Store a freshly issued token (already sealed); marks the account healthy.
pub async fn set_token(
    pool: &SqlitePool,
    id: i64,
    sealed_token: &str,
    expires_at: Option<DateTime<Utc>>,
) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE bambu_account SET token = ?, token_expires_at = ?, last_ok_at = ?, \
         last_error = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(sealed_token)
    .bind(expires_at.map(|d| d.to_rfc3339()))
    .bind(&now)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(())
}

/// A Bambu call on this account worked.
pub async fn mark_ok(pool: &SqlitePool, id: i64) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE bambu_account SET last_ok_at = ?, last_error = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(())
}

/// A Bambu call or sign-in on this account failed. Touches only this row.
pub async fn mark_error(pool: &SqlitePool, id: i64, error: &str) -> Result<(), AppError> {
    sqlx::query("UPDATE bambu_account SET last_error = ?, updated_at = ? WHERE id = ?")
        .bind(error)
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await
        .map_err(db)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ForemanDb;

    async fn pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let db = ForemanDb::connect(&dir.path().join("f.db"))
            .await
            .expect("db");
        (dir, db.pool)
    }

    #[tokio::test]
    async fn accounts_are_independent() {
        let (_d, pool) = pool().await;
        let a = create(&pool, "Home", Region::Global, "a@example.com")
            .await
            .expect("a");
        let b = create(&pool, "", Region::China, "b@example.com")
            .await
            .expect("b");
        assert_eq!(b.label, "b@example.com");

        set_token(&pool, a.id, "enc:v1:aaa", None)
            .await
            .expect("tok");
        mark_error(&pool, b.id, "Incorrect password")
            .await
            .expect("err");

        let a2 = get(&pool, a.id).await.expect("a2");
        let b2 = get(&pool, b.id).await.expect("b2");
        assert!(a2.has_token && a2.last_error.is_none() && a2.last_ok_at.is_some());
        assert!(!b2.has_token);
        assert_eq!(b2.last_error.as_deref(), Some("Incorrect password"));
        assert_eq!(
            get_sealed_token(&pool, a.id).await.expect("t").as_deref(),
            Some("enc:v1:aaa")
        );
        assert_eq!(get_sealed_token(&pool, b.id).await.expect("t"), None);
        assert_eq!(list(&pool).await.expect("list").len(), 2);
    }

    #[tokio::test]
    async fn delete_unlinks_printers() {
        let (_d, pool) = pool().await;
        let a = create(&pool, "Home", Region::Global, "a@example.com")
            .await
            .expect("a");
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO printer (name, firmware, created_at, updated_at, bambu_account_id) \
             VALUES ('A1', 'bambu', ?, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(a.id)
        .execute(&pool)
        .await
        .expect("printer");
        assert_eq!(get(&pool, a.id).await.expect("a").printer_count, 1);
        delete(&pool, a.id).await.expect("delete");
        let linked: Option<i64> = sqlx::query_scalar("SELECT bambu_account_id FROM printer")
            .fetch_one(&pool)
            .await
            .expect("q");
        assert_eq!(linked, None);
        assert!(matches!(get(&pool, a.id).await, Err(AppError::NotFound(_))));
    }
}
