//! Single-admin authentication primitives: Argon2id password hashing, random tokens
//! (stored as SHA-256 hashes), setup codes, and the DB access for admin / sessions /
//! API keys.

use argon2::{
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Argon2,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64URL, Engine as _};
use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};

use crate::{domain::ApiKey, error::AppError};

pub const MIN_PASSWORD_LEN: usize = 10;
pub const SESSION_TOKEN_PREFIX: &str = "fms_";
pub const API_KEY_PREFIX: &str = "fmk_";

fn db_err(e: sqlx::Error) -> AppError {
    AppError::Database(e.to_string())
}

// ---------------------------------------------------------------------------
// Passwords
// ---------------------------------------------------------------------------

pub fn check_password_rules(password: &str) -> Result<(), AppError> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(AppError::InvalidInput(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    Ok(())
}

/// Argon2id (default params) PHC string.
pub fn hash_password(password: &str) -> Result<String, AppError> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(format!("hash password: {e}")))
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc)
        .map(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tokens + setup codes
// ---------------------------------------------------------------------------

/// `prefix` + 32 random bytes (base64url).
pub fn random_token(prefix: &str) -> Result<String, AppError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| AppError::Internal(format!("getrandom: {e}")))?;
    Ok(format!("{prefix}{}", B64URL.encode(bytes)))
}

/// Tokens are high-entropy, so a fast hash is enough for at-rest storage.
pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// 8 characters as `XXXX-XXXX` from an unambiguous 32-letter alphabet (no 0/O/1/I).
pub fn generate_setup_code() -> Result<String, AppError> {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|e| AppError::Internal(format!("getrandom: {e}")))?;
    let chars: String = bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % 32] as char)
        .collect();
    Ok(format!("{}-{}", &chars[..4], &chars[4..]))
}

/// Constant-time equality for secrets of equal length.
pub fn secure_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

// ---------------------------------------------------------------------------
// Admin
// ---------------------------------------------------------------------------

pub async fn has_admin(pool: &SqlitePool) -> Result<bool, AppError> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_user")
        .fetch_one(pool)
        .await
        .map_err(db_err)?;
    Ok(n > 0)
}

/// `(username, password_hash)` of the admin, if one exists.
pub async fn get_admin(pool: &SqlitePool) -> Result<Option<(String, String)>, AppError> {
    sqlx::query_as("SELECT username, password_hash FROM admin_user WHERE id = 1")
        .fetch_optional(pool)
        .await
        .map_err(db_err)
}

pub async fn admin_username(pool: &SqlitePool) -> Result<Option<String>, AppError> {
    Ok(get_admin(pool).await?.map(|(u, _)| u))
}

fn check_username(username: &str) -> Result<&str, AppError> {
    let u = username.trim();
    if u.is_empty() || u.len() > 64 {
        return Err(AppError::InvalidInput(
            "username must be 1–64 characters".into(),
        ));
    }
    Ok(u)
}

/// Create the single admin. Fails if one already exists.
pub async fn create_admin(
    pool: &SqlitePool,
    username: &str,
    password: &str,
) -> Result<(), AppError> {
    let username = check_username(username)?;
    check_password_rules(password)?;
    let hash = hash_password(password)?;
    let now = Utc::now().to_rfc3339();
    let res = sqlx::query(
        "INSERT INTO admin_user (id, username, password_hash, created_at, updated_at) \
         VALUES (1, ?, ?, ?, ?) ON CONFLICT(id) DO NOTHING",
    )
    .bind(username)
    .bind(&hash)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(db_err)?;
    if res.rows_affected() == 0 {
        return Err(AppError::InvalidInput(
            "an admin account already exists".into(),
        ));
    }
    Ok(())
}

/// Create or replace the admin (CLI `reset-admin`) and sign out every session.
pub async fn reset_admin(
    pool: &SqlitePool,
    username: &str,
    password: &str,
) -> Result<(), AppError> {
    let username = check_username(username)?;
    check_password_rules(password)?;
    let hash = hash_password(password)?;
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO admin_user (id, username, password_hash, created_at, updated_at) \
         VALUES (1, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET username = excluded.username, \
           password_hash = excluded.password_hash, updated_at = excluded.updated_at",
    )
    .bind(username)
    .bind(&hash)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(db_err)?;
    sqlx::query("DELETE FROM auth_session")
        .execute(pool)
        .await
        .map_err(db_err)?;
    Ok(())
}

/// Check username + password against the stored admin.
pub async fn check_login(
    pool: &SqlitePool,
    username: &str,
    password: &str,
) -> Result<bool, AppError> {
    Ok(match get_admin(pool).await? {
        Some((u, hash)) => secure_eq(&u, username.trim()) & verify_password(password, &hash),
        None => false,
    })
}

// ---------------------------------------------------------------------------
// Sessions (expires_at = unix seconds)
// ---------------------------------------------------------------------------

/// Create a session; returns the plaintext token (only ever shown to the client).
pub async fn create_session(
    pool: &SqlitePool,
    ttl_secs: i64,
    ip: Option<&str>,
    user_agent: Option<&str>,
) -> Result<(String, i64), AppError> {
    let token = random_token(SESSION_TOKEN_PREFIX)?;
    let expires_at = Utc::now().timestamp() + ttl_secs;
    sqlx::query(
        "INSERT INTO auth_session (token_hash, created_at, expires_at, ip, user_agent) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(token_hash(&token))
    .bind(Utc::now().to_rfc3339())
    .bind(expires_at)
    .bind(ip)
    .bind(user_agent)
    .execute(pool)
    .await
    .map_err(db_err)?;
    Ok((token, expires_at))
}

/// True if the token belongs to a live (unexpired) session.
pub async fn session_valid(pool: &SqlitePool, token: &str, now: i64) -> Result<bool, AppError> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_session WHERE token_hash = ? AND expires_at > ?",
    )
    .bind(token_hash(token))
    .bind(now)
    .fetch_one(pool)
    .await
    .map_err(db_err)?;
    Ok(n > 0)
}

pub async fn delete_session(pool: &SqlitePool, token: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM auth_session WHERE token_hash = ?")
        .bind(token_hash(token))
        .execute(pool)
        .await
        .map_err(db_err)?;
    Ok(())
}

pub async fn purge_expired_sessions(pool: &SqlitePool, now: i64) -> Result<u64, AppError> {
    Ok(
        sqlx::query("DELETE FROM auth_session WHERE expires_at <= ?")
            .bind(now)
            .execute(pool)
            .await
            .map_err(db_err)?
            .rows_affected(),
    )
}

// ---------------------------------------------------------------------------
// API keys
// ---------------------------------------------------------------------------

#[derive(FromRow)]
struct ApiKeyRow {
    id: i64,
    name: String,
    prefix: String,
    created_at: String,
    last_used_at: Option<String>,
}

impl From<ApiKeyRow> for ApiKey {
    fn from(r: ApiKeyRow) -> Self {
        ApiKey {
            id: r.id,
            name: r.name,
            prefix: r.prefix,
            created_at: r.created_at,
            last_used_at: r.last_used_at,
        }
    }
}

/// Create an API key; returns the record and the plaintext key (shown once).
pub async fn create_api_key(pool: &SqlitePool, name: &str) -> Result<(ApiKey, String), AppError> {
    let name = name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::InvalidInput(
            "key name must be 1–64 characters".into(),
        ));
    }
    let key = random_token(API_KEY_PREFIX)?;
    let prefix: String = key.chars().take(API_KEY_PREFIX.len() + 6).collect();
    let id =
        sqlx::query("INSERT INTO api_key (name, prefix, key_hash, created_at) VALUES (?, ?, ?, ?)")
            .bind(name)
            .bind(&prefix)
            .bind(token_hash(&key))
            .bind(Utc::now().to_rfc3339())
            .execute(pool)
            .await
            .map_err(db_err)?
            .last_insert_rowid();
    let row: ApiKeyRow = sqlx::query_as(
        "SELECT id, name, prefix, created_at, last_used_at FROM api_key WHERE id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(db_err)?;
    Ok((row.into(), key))
}

pub async fn list_api_keys(pool: &SqlitePool) -> Result<Vec<ApiKey>, AppError> {
    let rows: Vec<ApiKeyRow> = sqlx::query_as(
        "SELECT id, name, prefix, created_at, last_used_at FROM api_key \
         WHERE revoked_at IS NULL ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn revoke_api_key(pool: &SqlitePool, id: i64) -> Result<(), AppError> {
    let n = sqlx::query("UPDATE api_key SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await
        .map_err(db_err)?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound(format!("api key {id}")));
    }
    Ok(())
}

/// Name of an active key (for audit lines); `None` once revoked.
pub async fn api_key_name(pool: &SqlitePool, id: i64) -> Result<Option<String>, AppError> {
    sqlx::query_scalar("SELECT name FROM api_key WHERE id = ? AND revoked_at IS NULL")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db_err)
}

/// The id of the active key matching `key`, marking it used.
pub async fn check_api_key(pool: &SqlitePool, key: &str) -> Result<Option<i64>, AppError> {
    let id: Option<i64> =
        sqlx::query_scalar("SELECT id FROM api_key WHERE key_hash = ? AND revoked_at IS NULL")
            .bind(token_hash(key))
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
    if let Some(id) = id {
        sqlx::query("UPDATE api_key SET last_used_at = ? WHERE id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(id)
            .execute(pool)
            .await
            .map_err(db_err)?;
    }
    Ok(id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> (crate::db::ForemanDb, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let db = crate::db::ForemanDb::connect(&dir.path().join("f.db"))
            .await
            .expect("db");
        (db, dir)
    }

    #[test]
    fn argon2id_hash_and_verify() {
        let h = hash_password("correct horse battery").expect("hash");
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &h));
        assert!(!verify_password("wrong password!", &h));
        assert!(!verify_password("x", "not-a-phc-string"));
    }

    #[test]
    fn password_rules() {
        assert!(check_password_rules("123456789").is_err());
        assert!(check_password_rules("1234567890").is_ok());
    }

    #[test]
    fn setup_code_format() {
        let c = generate_setup_code().expect("code");
        assert_eq!(c.len(), 9);
        assert_eq!(&c[4..5], "-");
        assert!(c
            .chars()
            .filter(|ch| *ch != '-')
            .all(|ch| "ABCDEFGHJKLMNPQRSTUVWXYZ23456789".contains(ch)));
    }

    #[tokio::test]
    async fn single_admin_and_login() {
        let (db, _d) = db().await;
        assert!(!has_admin(&db.pool).await.expect("q"));
        assert!(create_admin(&db.pool, "admin", "short").await.is_err());
        create_admin(&db.pool, "admin", "long enough pw")
            .await
            .expect("create");
        assert!(has_admin(&db.pool).await.expect("q"));
        assert!(create_admin(&db.pool, "other", "long enough pw")
            .await
            .is_err());
        assert!(check_login(&db.pool, "admin", "long enough pw")
            .await
            .expect("q"));
        assert!(!check_login(&db.pool, "admin", "wrong password")
            .await
            .expect("q"));
        assert!(!check_login(&db.pool, "root", "long enough pw")
            .await
            .expect("q"));
        let stored = get_admin(&db.pool).await.expect("q").expect("admin").1;
        assert!(!stored.contains("long enough pw"));
    }

    #[tokio::test]
    async fn session_expiry_and_logout() {
        let (db, _d) = db().await;
        let now = Utc::now().timestamp();
        let (tok, exp) = create_session(&db.pool, 3600, None, None).await.expect("s");
        assert!(tok.starts_with(SESSION_TOKEN_PREFIX));
        assert!(session_valid(&db.pool, &tok, now).await.expect("q"));
        assert!(
            !session_valid(&db.pool, &tok, exp).await.expect("q"),
            "expired at expires_at"
        );
        assert!(!session_valid(&db.pool, "fms_bogus", now).await.expect("q"));
        delete_session(&db.pool, &tok).await.expect("del");
        assert!(!session_valid(&db.pool, &tok, now).await.expect("q"));

        let (old, _) = create_session(&db.pool, -1, None, None).await.expect("s");
        assert_eq!(
            purge_expired_sessions(&db.pool, now).await.expect("purge"),
            1
        );
        assert!(!session_valid(&db.pool, &old, now - 10).await.expect("q"));
    }

    #[tokio::test]
    async fn reset_admin_signs_everyone_out() {
        let (db, _d) = db().await;
        create_admin(&db.pool, "admin", "first password")
            .await
            .expect("c");
        let (tok, _) = create_session(&db.pool, 3600, None, None).await.expect("s");
        reset_admin(&db.pool, "admin", "second password")
            .await
            .expect("reset");
        assert!(!session_valid(&db.pool, &tok, Utc::now().timestamp())
            .await
            .expect("q"));
        assert!(check_login(&db.pool, "admin", "second password")
            .await
            .expect("q"));
    }

    #[tokio::test]
    async fn api_keys_hashed_and_revocable() {
        let (db, _d) = db().await;
        let (key, plain) = create_api_key(&db.pool, "OrcaSlicer").await.expect("k");
        assert!(plain.starts_with(API_KEY_PREFIX));
        assert!(plain.starts_with(&key.prefix));
        let stored: String = sqlx::query_scalar("SELECT key_hash FROM api_key")
            .fetch_one(&db.pool)
            .await
            .expect("q");
        assert_ne!(stored, plain);
        assert_eq!(
            check_api_key(&db.pool, &plain).await.expect("q"),
            Some(key.id)
        );
        assert_eq!(check_api_key(&db.pool, "fmk_nope").await.expect("q"), None);
        revoke_api_key(&db.pool, key.id).await.expect("revoke");
        assert_eq!(check_api_key(&db.pool, &plain).await.expect("q"), None);
        assert!(list_api_keys(&db.pool).await.expect("q").is_empty());
    }
}
