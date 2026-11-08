pub mod extra_field;
pub mod filament;
pub mod migrations;
pub mod query;
pub mod search;
pub mod setting;
pub mod spool;
pub mod tag;
pub mod vendor;

use sqlx::AnyPool;

use crate::{
    config::{DbType, SpoolmanConfig},
    error::{Result, SpoolmanError},
};

/// Which SQL dialect the pool is speaking.
///
/// CockroachDB uses the Postgres wire protocol, so it shares the Postgres path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Engine {
    Sqlite,
    Postgres, // also CockroachDB
    Mysql,    // also MariaDB
}

impl From<&DbType> for Engine {
    fn from(t: &DbType) -> Self {
        match t {
            DbType::Sqlite => Engine::Sqlite,
            DbType::Postgres | DbType::Cockroachdb => Engine::Postgres,
            DbType::Mysql => Engine::Mysql,
        }
    }
}

/// Database handle — `AnyPool` with the detected engine dialect.
#[derive(Debug)]
pub struct Db {
    pub pool: AnyPool,
    pub engine: Engine,
}

/// Register all compiled-in sqlx drivers.
///
/// Must be called once before any `AnyPool` operation.  `Db::connect` calls it
/// automatically; call this directly in tests that bypass `Db::connect`.
pub fn install_drivers() {
    sqlx::any::install_default_drivers();
}

impl Db {
    /// Open (or create) the database described by `config`.
    ///
    /// For SQLite, creates the parent directory and applies WAL mode.
    /// Runs the alembic-compatible startup check immediately after connecting.
    pub async fn connect(config: &SpoolmanConfig) -> Result<Self> {
        // Register all compiled-in sqlx drivers (sqlite, postgres, mysql).
        // This is idempotent and must be called before any AnyPool operation.
        sqlx::any::install_default_drivers();

        // For SQLite: ensure the parent directory and an initial empty file both
        // exist before connecting.  AnyPool::connect opens SQLite in read-write
        // mode (without the CREATE flag), so it will fail with SQLITE_CANTOPEN
        // if the file is absent.  An empty file is a valid uninitialised SQLite
        // database — the page header is written on the first actual DB write.
        if config.db_type == DbType::Sqlite {
            let path = config.sqlite_path();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if !path.exists() {
                std::fs::File::create(&path)?;
            }
        }

        let url = config.connection_url()?;
        let pool = AnyPool::connect(&url).await?;

        // SQLite-specific per-connection setup applied to the first connection.
        // WAL mode persists in the database file after the first time it is set.
        // FK enforcement is per-connection; session 6 will wire an after_connect
        // hook so every pooled connection gets it automatically.
        if config.db_type == DbType::Sqlite {
            sqlx::query("PRAGMA journal_mode = WAL")
                .fetch_optional(&pool)
                .await?;
            sqlx::query("PRAGMA synchronous = NORMAL")
                .execute(&pool)
                .await?;
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&pool)
                .await?;
        }

        let engine = Engine::from(&config.db_type);
        let db = Db { pool, engine };
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
                tracing::info!("fresh database — creating schema");
                self.create_alembic_version_table().await?;
                migrations::apply_from(&self.pool, None, &self.engine).await?;
                tracing::info!(head = migrations::HEAD, "schema created at HEAD");
            }
            Some(rev) if rev == migrations::HEAD => {
                tracing::info!(head = migrations::HEAD, "schema already at HEAD");
            }
            Some(rev) => {
                if migrations::CHAIN.iter().all(|m| m.revision != rev) {
                    return Err(SpoolmanError::UnknownRevision(rev.to_owned()));
                }
                tracing::info!(from = rev, to = migrations::HEAD, "migrating database");
                migrations::apply_from(&self.pool, Some(rev), &self.engine).await?;
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
    ///
    /// Uses a try-select instead of a table-existence query so the same code
    /// works across SQLite (`sqlite_master`), PostgreSQL, and MySQL without
    /// engine-specific DDL introspection.
    async fn read_revision(&self) -> Result<Option<String>> {
        match sqlx::query_scalar::<_, String>("SELECT version_num FROM alembic_version LIMIT 1")
            .fetch_optional(&self.pool)
            .await
        {
            Ok(rev) => Ok(rev),
            // Any "table not found" error means a fresh database.
            Err(sqlx::Error::Database(e)) if is_table_not_found(e.as_ref()) => Ok(None),
            Err(e) => Err(e.into()),
        }
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

        // The table was just created in the fresh-DB path, so there's exactly
        // zero rows — no INSERT OR IGNORE / ON CONFLICT needed.
        sqlx::query("INSERT INTO alembic_version (version_num) VALUES (?)")
            .bind("")
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}

/// Detect "table not found" errors across the three supported engines.
fn is_table_not_found(e: &dyn sqlx::error::DatabaseError) -> bool {
    // PostgreSQL / CockroachDB: code 42P01 (undefined_table)
    // MySQL / MariaDB: code 1146 (ER_NO_SUCH_TABLE)
    // SQLite: code 1 (SQLITE_ERROR, generic) — discriminate by message instead.
    if let Some(code) = e.code() {
        if code.as_ref() == "42P01" || code.as_ref() == "1146" {
            return true;
        }
    }
    e.message().starts_with("no such table:")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::Path;

    use sqlx::SqlitePool;

    use super::*;
    use crate::config::SpoolmanConfig;
    use tempfile::TempDir;

    /// Install all compiled-in sqlx drivers.  Must be called in any test that
    /// uses AnyPool directly (Db::connect does this internally, but setup code
    /// that builds a SqlitePool and then hands the file to Db::connect does not).
    fn setup_drivers() {
        sqlx::any::install_default_drivers();
    }

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
            // sqlite_master is SQLite-specific and that's fine: these tests only
            // run against SQLite.  Postgres / MySQL tests are gated on env vars
            // in separate integration tests (session 10).
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

    // -----------------------------------------------------------------------
    // 2. Already-at-HEAD is a no-op
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn already_at_head_is_noop() {
        let dir = TempDir::new().unwrap();
        let cfg = config_for_dir(dir.path());

        drop(Db::connect(&cfg).await.unwrap());

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
        setup_drivers();
        let dir = TempDir::new().unwrap();
        let cfg = config_for_dir(dir.path());

        // Bootstrap a database that is at exactly the first migration (684d32cf7e4d).
        // We use SqlitePool directly for setup so we can use SQLite-specific
        // connection options; Db::connect (which uses AnyPool) is what's being tested.
        let first_rev = migrations::CHAIN[0].revision;
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
            sqlx::query("INSERT INTO alembic_version (version_num) VALUES (?)")
                .bind(first_rev)
                .execute(&pool)
                .await
                .unwrap();

            // Apply only migration 0's SQL so migrations 1-14 are genuinely absent.
            for stmt in migrations::CHAIN[0].sql_for(&Engine::Sqlite).split(';') {
                let stmt = stmt.trim();
                if !stmt.is_empty() {
                    sqlx::query(stmt).execute(&pool).await.unwrap();
                }
            }

            pool.close().await;
        }

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
        setup_drivers();
        let dir = TempDir::new().unwrap();
        let cfg = config_for_dir(dir.path());

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
