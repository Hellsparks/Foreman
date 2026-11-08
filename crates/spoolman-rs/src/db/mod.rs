pub mod migrations;

use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
    SqlitePool,
};

use crate::{
    config::SpoolmanConfig,
    error::{Result, SpoolmanError},
};

/// Database handle — SQLite only in session 4.
#[derive(Debug)]
pub struct Db {
    pub pool: SqlitePool,
}

impl Db {
    /// Open (or create) the SQLite database at the path given by `config`.
    ///
    /// Creates parent directories if they do not exist.
    /// Runs the alembic-compatible startup check immediately after opening.
    pub async fn connect(config: &SpoolmanConfig) -> Result<Self> {
        let db_path = config.sqlite_path();

        // Ensure the parent directory exists.
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            // WAL is the right journal mode for a web-serving workload.
            .journal_mode(SqliteJournalMode::Wal)
            // NORMAL is safe with WAL (a crash cannot corrupt the DB).
            .synchronous(SqliteSynchronous::Normal)
            // Let SQLite enforce foreign-key constraints.
            .foreign_keys(true);

        let pool = SqlitePool::connect_with(opts).await?;

        let db = Db { pool };
        db.startup_check().await?;
        Ok(db)
    }

    /// Alembic-compatible startup revision check.
    ///
    /// | alembic_version state        | action                                    |
    /// |------------------------------|-------------------------------------------|
    /// | table missing                | fresh install — create all tables + HEAD  |
    /// | version == HEAD              | already up-to-date — no-op                |
    /// | version == known older rev   | apply incremental migrations to HEAD      |
    /// | version == unknown / newer   | refuse to start with a clear error        |
    pub async fn startup_check(&self) -> Result<()> {
        let revision = self.read_revision().await?;

        match revision.as_deref() {
            None => {
                // Fresh database: bootstrap alembic_version, then run all migrations.
                tracing::info!("fresh database — creating schema");
                self.create_alembic_version_table().await?;
                migrations::apply_from(&self.pool, None).await?;
                tracing::info!(head = migrations::HEAD, "schema created at HEAD");
            }
            Some(rev) if rev == migrations::HEAD => {
                tracing::info!(head = migrations::HEAD, "schema already at HEAD");
            }
            Some(rev) => {
                // Validate that the revision is in our known chain before attempting
                // to migrate (apply_from will also reject unknowns, but we surface
                // the error here with a more helpful message).
                if migrations::CHAIN.iter().all(|m| m.revision != rev) {
                    return Err(SpoolmanError::UnknownRevision(rev.to_owned()));
                }
                tracing::info!(from = rev, to = migrations::HEAD, "migrating database");
                migrations::apply_from(&self.pool, Some(rev)).await?;
                tracing::info!(head = migrations::HEAD, "migration complete");
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Return the current `alembic_version` value, or `None` if the table does
    /// not exist (fresh database).
    async fn read_revision(&self) -> Result<Option<String>> {
        // Check whether the alembic_version table exists first.
        let table_exists: bool = sqlx::query_scalar(
            "SELECT COUNT(*) > 0 FROM sqlite_master \
             WHERE type = 'table' AND name = 'alembic_version'",
        )
        .fetch_one(&self.pool)
        .await?;

        if !table_exists {
            return Ok(None);
        }

        let rev: Option<String> =
            sqlx::query_scalar("SELECT version_num FROM alembic_version LIMIT 1")
                .fetch_optional(&self.pool)
                .await?;

        Ok(rev)
    }

    async fn create_alembic_version_table(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS alembic_version (\
                version_num VARCHAR(32) NOT NULL, \
                CONSTRAINT alembic_version_pkc PRIMARY KEY (version_num)\
             )",
        )
        .execute(&self.pool)
        .await?;

        // Insert a placeholder so the UPDATE in apply_from has a row to update.
        // We start at an empty string; apply_from overwrites it with each revision.
        sqlx::query("INSERT OR IGNORE INTO alembic_version (version_num) VALUES ('')")
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::config::SpoolmanConfig;
    use tempfile::TempDir;

    fn config_for_dir(dir: &Path) -> SpoolmanConfig {
        SpoolmanConfig {
            db_type: crate::config::DbType::Sqlite,
            db_host: None,
            db_port: None,
            db_name: None,
            db_username: None,
            db_password: None,
            data_dir: dir.to_path_buf(),
            logs_dir: dir.to_path_buf(),
            backups_dir: dir.join("backups"),
            host: "0.0.0.0".to_owned(),
            port: 7912,
            base_path: String::new(),
            automatic_backup: false,
        }
    }

    // -----------------------------------------------------------------------
    // 1. Fresh database creation
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fresh_db_creates_schema_at_head() {
        let dir = TempDir::new().unwrap();
        let cfg = config_for_dir(dir.path());

        let db = Db::connect(&cfg).await.expect("connect failed");

        // alembic_version must equal HEAD
        let rev: String = sqlx::query_scalar("SELECT version_num FROM alembic_version LIMIT 1")
            .fetch_one(&db.pool)
            .await
            .expect("alembic_version not found");

        assert_eq!(rev, migrations::HEAD);
    }

    #[tokio::test]
    async fn fresh_db_has_all_tables() {
        let dir = TempDir::new().unwrap();
        let cfg = config_for_dir(dir.path());
        let db = Db::connect(&cfg).await.unwrap();

        for table in &[
            "vendor",
            "filament",
            "spool",
            "setting",
            "vendor_field",
            "filament_field",
            "spool_field",
            "tag",
            "alembic_version",
        ] {
            let exists: bool = sqlx::query_scalar(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_one(&db.pool)
            .await
            .unwrap_or(false);

            assert!(exists, "table '{table}' missing from fresh schema");
        }
    }

    // -----------------------------------------------------------------------
    // 2. Already-at-HEAD is a no-op
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn already_at_head_is_noop() {
        let dir = TempDir::new().unwrap();
        let cfg = config_for_dir(dir.path());

        // First open creates the schema.
        drop(Db::connect(&cfg).await.unwrap());

        // Second open must succeed without error.
        let db = Db::connect(&cfg).await.expect("second connect failed");

        let rev: String = sqlx::query_scalar("SELECT version_num FROM alembic_version LIMIT 1")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(rev, migrations::HEAD);
    }

    // -----------------------------------------------------------------------
    // 3. Incremental migration from an older revision
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn migrates_from_initial_revision() {
        let dir = TempDir::new().unwrap();
        let cfg = config_for_dir(dir.path());

        // Bootstrap a database that is at exactly the first migration (684d32cf7e4d)
        // — only that migration's SQL is applied; the rest are intentionally absent.
        let first_rev = migrations::CHAIN[0].revision;
        {
            let opts = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(cfg.sqlite_path())
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .foreign_keys(true);
            let pool = SqlitePool::connect_with(opts).await.unwrap();

            // Create alembic_version table.
            sqlx::query(
                "CREATE TABLE alembic_version (\
                    version_num VARCHAR(32) NOT NULL, \
                    CONSTRAINT alembic_version_pkc PRIMARY KEY (version_num)\
                 )",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO alembic_version (version_num) VALUES (?)")
                .bind(first_rev)
                .execute(&pool)
                .await
                .unwrap();

            // Apply only the first migration's SQL directly (not via apply_from so
            // that migrations 1-14 are genuinely absent and not just version-labelled).
            for stmt in migrations::CHAIN[0].sql.split(';') {
                let stmt = stmt.trim();
                if !stmt.is_empty() {
                    sqlx::query(stmt).execute(&pool).await.unwrap();
                }
            }

            pool.close().await;
        }

        // Now open normally — it should migrate all the way to HEAD.
        let db = Db::connect(&cfg).await.expect("migration failed");

        let rev: String = sqlx::query_scalar("SELECT version_num FROM alembic_version LIMIT 1")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(rev, migrations::HEAD, "should have migrated to HEAD");
    }

    // -----------------------------------------------------------------------
    // 4. Unknown revision triggers a clear refusal
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn unknown_revision_refuses_to_start() {
        let dir = TempDir::new().unwrap();
        let cfg = config_for_dir(dir.path());

        // Plant an unknown revision.
        {
            let opts = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(cfg.sqlite_path())
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .foreign_keys(true);
            let pool = SqlitePool::connect_with(opts).await.unwrap();
            sqlx::query(
                "CREATE TABLE alembic_version (\
                    version_num VARCHAR(32) NOT NULL, \
                    CONSTRAINT alembic_version_pkc PRIMARY KEY (version_num)\
                 )",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO alembic_version (version_num) VALUES ('deadbeefcafe')")
                .execute(&pool)
                .await
                .unwrap();
            pool.close().await;
        }

        let err = Db::connect(&cfg)
            .await
            .expect_err("should have refused unknown revision");

        assert!(
            matches!(err, SpoolmanError::UnknownRevision(ref r) if r == "deadbeefcafe"),
            "expected UnknownRevision, got: {err}"
        );
    }
}
