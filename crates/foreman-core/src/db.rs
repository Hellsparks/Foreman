//! Foreman-native SQLite database handle.
//!
//! Separate from the Spoolman DB (managed by `spoolman-rs`).
//! Path comes from server config.
//!
//! Uses `SqlitePool` directly — no `AnyPool`, typed binds throughout.
//! Migration SQL is embedded as a string constant; tables are created with
//! `IF NOT EXISTS` so the runner is idempotent on every startup.

use std::path::Path;

use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
    SqlitePool,
};

use crate::error::AppError;

/// Embedded initial schema (all tables, idempotent via IF NOT EXISTS).
const SCHEMA_SQL: &str = include_str!("../migrations/0001_initial.sql");

/// Incremental migrations applied after the initial schema.
/// Each is a single SQL statement. We catch "duplicate column" errors to make
/// them idempotent (the column already exists in fresh databases).
const MIGRATIONS: &[&str] = &[
    // 0002: TLS fingerprint pinning for Bambu printers.
    "ALTER TABLE printer ADD COLUMN tls_fingerprint TEXT",
];

/// Foreman-native database handle.
#[derive(Debug, Clone)]
pub struct ForemanDb {
    pub pool: SqlitePool,
}

impl ForemanDb {
    /// Open (or create) the Foreman SQLite database at `path` and apply the
    /// embedded schema (idempotent — `CREATE TABLE IF NOT EXISTS`).
    pub async fn connect(path: &Path) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError::Database(format!("create data dir: {e}")))?;
        }

        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);

        let pool = SqlitePool::connect_with(opts)
            .await
            .map_err(|e| AppError::Database(format!("open db: {e}")))?;

        Self::run_schema(&pool).await?;
        Self::run_migrations(&pool).await?;

        Ok(ForemanDb { pool })
    }

    /// Apply incremental migrations, ignoring "duplicate column" errors
    /// so they are idempotent across fresh and existing databases.
    async fn run_migrations(pool: &SqlitePool) -> Result<(), AppError> {
        for stmt in MIGRATIONS {
            match sqlx::query(*stmt).execute(pool).await {
                Ok(_) => {}
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("duplicate column") {
                        continue;
                    }
                    return Err(AppError::Database(format!("migration: {e}\nSQL: {stmt}")));
                }
            }
        }
        Ok(())
    }

    /// Execute the embedded schema SQL, skipping blank lines and comments.
    async fn run_schema(pool: &SqlitePool) -> Result<(), AppError> {
        for stmt in SCHEMA_SQL.split(';') {
            let stmt = stmt.trim();
            // Skip empty chunks and comment-only blocks.
            let non_comment: String = stmt
                .lines()
                .filter(|l| !l.trim_start().starts_with("--") && !l.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if non_comment.is_empty() {
                continue;
            }
            sqlx::query(stmt)
                .execute(pool)
                .await
                .map_err(|e| AppError::Database(format!("schema: {e}\nSQL: {stmt}")))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn open_fresh() -> (ForemanDb, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let db = ForemanDb::connect(&dir.path().join("foreman.db"))
            .await
            .expect("connect");
        (db, dir)
    }

    #[tokio::test]
    async fn fresh_db_creates_all_tables() {
        let (db, _dir) = open_fresh().await;

        for table in &[
            "printer",
            "printer_path",
            "printer_ssh",
            "webcam",
            "macro_favourite",
            "temp_preset",
            "shell_macro",
            "audit_log",
            "attention_item",
            "setting",
        ] {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(*table)
            .fetch_one(&db.pool)
            .await
            .unwrap_or(0);

            assert!(count > 0, "table '{table}' missing from fresh schema");
        }
    }

    #[tokio::test]
    async fn schema_is_idempotent() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("foreman.db");

        let _db1 = ForemanDb::connect(&path).await.expect("first open");
        let _db2 = ForemanDb::connect(&path).await.expect("second open");
    }
}
