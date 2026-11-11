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
    // 0003: Path failover timing settings.
    "ALTER TABLE printer ADD COLUMN fail_after_checks INTEGER NOT NULL DEFAULT 3",
    "ALTER TABLE printer ADD COLUMN return_after_secs  INTEGER NOT NULL DEFAULT 60",
    // 0004: SSH host key presented after a change, awaiting approval.
    "ALTER TABLE printer_ssh ADD COLUMN pending_host_key_fingerprint TEXT",
    // 0005: idle auto-close for SSH connections (seconds, 0 = never).
    "ALTER TABLE printer_ssh ADD COLUMN idle_timeout_secs INTEGER NOT NULL DEFAULT 900",
    // 0006: single admin, server-side sessions, API keys (tokens stored as SHA-256).
    "CREATE TABLE IF NOT EXISTS admin_user (\
       id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1), username TEXT NOT NULL, \
       password_hash TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS auth_session (\
       id INTEGER PRIMARY KEY NOT NULL, token_hash TEXT NOT NULL UNIQUE, \
       created_at TEXT NOT NULL, expires_at INTEGER NOT NULL, ip TEXT, user_agent TEXT)",
    "CREATE TABLE IF NOT EXISTS api_key (\
       id INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL, prefix TEXT NOT NULL, \
       key_hash TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL, last_used_at TEXT, \
       revoked_at TEXT)",
    // 0007: Library index + parsed file facts (see migrations/0007_library.sql).
    "CREATE TABLE IF NOT EXISTS library_entry (\
       id INTEGER PRIMARY KEY NOT NULL, path TEXT NOT NULL, parent TEXT NOT NULL, kind TEXT NOT NULL, \
       size INTEGER NOT NULL DEFAULT 0, hash TEXT, mtime INTEGER, added_at TEXT NOT NULL, \
       starred INTEGER NOT NULL DEFAULT 0, deleted_at TEXT, trash_id TEXT, \
       trash_root INTEGER NOT NULL DEFAULT 0)",
    "CREATE UNIQUE INDEX IF NOT EXISTS library_entry_live_path \
       ON library_entry(path) WHERE deleted_at IS NULL",
    "CREATE INDEX IF NOT EXISTS library_entry_parent ON library_entry(parent)",
    "CREATE TABLE IF NOT EXISTS file_meta (\
       entry_id INTEGER PRIMARY KEY NOT NULL REFERENCES library_entry(id) ON DELETE CASCADE, \
       format TEXT NOT NULL, parse_status TEXT NOT NULL, parse_error TEXT, slicer TEXT, \
       slicer_version TEXT, printer_model TEXT, printer_settings_id TEXT, printer_model_id TEXT, \
       nozzle_diameter REAL, print_time_secs INTEGER, \
       layer_count INTEGER, bed_width REAL, bed_depth REAL, filament_settings_id TEXT, \
       suggested_spool_id INTEGER, tools_json TEXT NOT NULL DEFAULT '[]', \
       plates_json TEXT NOT NULL DEFAULT '[]', thumb_file TEXT, parsed_at TEXT NOT NULL)",
    // 0008: slicer profiles per printer + plate-clear + auto-start (3.3 + 3.4).
    "ALTER TABLE printer ADD COLUMN slicer_profiles TEXT NOT NULL DEFAULT '[]'",
    "ALTER TABLE printer ADD COLUMN plate_clear INTEGER NOT NULL DEFAULT 1",
    "ALTER TABLE printer ADD COLUMN auto_start_from_queue INTEGER NOT NULL DEFAULT 1",
    // 0009: printer queues + fleet queue (3.5, see migrations/0009_queue.sql).
    "CREATE TABLE IF NOT EXISTS queue_item (\
       id INTEGER PRIMARY KEY NOT NULL, file_id INTEGER NOT NULL, file_name TEXT NOT NULL, \
       source TEXT NOT NULL DEFAULT 'manual', added_by TEXT NOT NULL, added_at TEXT NOT NULL, \
       status TEXT NOT NULL DEFAULT 'waiting', \
       printer_id INTEGER REFERENCES printer(id) ON DELETE SET NULL, \
       from_fleet INTEGER NOT NULL DEFAULT 0, position INTEGER NOT NULL DEFAULT 0, \
       error TEXT, updated_at TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS ix_queue_item_status ON queue_item (status)",
    // 0010: job engine (3.6, see migrations/0010_jobs.sql).
    "CREATE TABLE IF NOT EXISTS job (\
       id INTEGER PRIMARY KEY NOT NULL, \
       printer_id INTEGER REFERENCES printer(id) ON DELETE SET NULL, \
       printer_name TEXT NOT NULL, file_id INTEGER, file_name TEXT NOT NULL, \
       queue_item_id INTEGER, started_at TEXT NOT NULL, ended_at TEXT, duration_secs INTEGER, \
       end_state TEXT NOT NULL DEFAULT 'running', outcome TEXT, fail_reason TEXT, error TEXT, \
       usage_reporter TEXT NOT NULL DEFAULT 'foreman', grams_planned REAL, grams_used REAL, \
       grams_deducted REAL NOT NULL DEFAULT 0, material TEXT, \
       tools_json TEXT NOT NULL DEFAULT '[]', deduct_state TEXT NOT NULL DEFAULT 'none', \
       thumb_url TEXT)",
    "CREATE INDEX IF NOT EXISTS ix_job_printer ON job (printer_id)",
    "CREATE INDEX IF NOT EXISTS ix_job_started ON job (started_at)",
    "ALTER TABLE printer ADD COLUMN usage_reporter TEXT NOT NULL DEFAULT 'auto'",
    "ALTER TABLE attention_item ADD COLUMN job_id INTEGER",
    // 0011: tracked filaments (4.3, see migrations/0011_inventory.sql).
    "CREATE TABLE IF NOT EXISTS tracked_filament (\
       filament_id INTEGER PRIMARY KEY NOT NULL, \
       target INTEGER NOT NULL, \
       buy_below INTEGER NOT NULL, \
       updated_at TEXT NOT NULL)",
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
