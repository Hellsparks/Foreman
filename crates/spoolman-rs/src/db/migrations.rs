//! Alembic-compatible migration runner for spoolman-rs.
//!
//! Each entry in [`CHAIN`] is a `Migration`: revision ID, description, and
//! the SQL to bring the schema from the previous revision to that revision.
//! All SQL is hand-ported from the upstream Python Alembic files in
//! `Donkie/Spoolman` tag v0.27.1 (`migrations/versions/`).
//!
//! Where engines differ (auto-increment syntax, DDL for type changes) each
//! `Migration` carries per-engine override SQL that falls back to the shared
//! `sql` field when empty.

use sqlx::AnyPool;

use crate::{
    db::query::tp,
    error::{Result, SpoolmanError},
};

use super::Engine;

/// One step in the alembic revision chain.
pub struct Migration {
    pub revision: &'static str,
    pub description: &'static str,
    /// SQL that works on SQLite (and is used as fallback for all engines).
    pub sql: &'static str,
    /// SQL override for PostgreSQL and CockroachDB.  Empty → use `sql`.
    pub postgres_sql: &'static str,
    /// SQL override for MySQL / MariaDB.  Empty → use `sql`.
    pub mysql_sql: &'static str,
}

impl Migration {
    /// Return the SQL appropriate for the given engine.
    pub fn sql_for(&self, engine: &Engine) -> &'static str {
        match engine {
            Engine::Postgres if !self.postgres_sql.is_empty() => self.postgres_sql,
            Engine::Mysql if !self.mysql_sql.is_empty() => self.mysql_sql,
            _ => self.sql,
        }
    }
}

// ---------------------------------------------------------------------------
// The 15-revision chain (oldest → HEAD = fe4970567bb3)
// ---------------------------------------------------------------------------

/// The ordered chain of all 15 migrations, oldest → newest.
pub static CHAIN: &[Migration] = &[
    // -------------------------------------------------------------------------
    // 684d32cf7e4d — initial (vendor, filament, spool tables)
    // -------------------------------------------------------------------------
    Migration {
        revision: "684d32cf7e4d",
        description: "initial",

        // --- SQLite ---
        sql: "
            CREATE TABLE IF NOT EXISTS vendor (
                id         INTEGER PRIMARY KEY NOT NULL,
                registered DATETIME NOT NULL DEFAULT (CURRENT_TIMESTAMP),
                name       VARCHAR(64) NOT NULL,
                comment    VARCHAR(1024)
            );
            CREATE INDEX IF NOT EXISTS ix_vendor_id ON vendor (id);

            CREATE TABLE IF NOT EXISTS filament (
                id             INTEGER PRIMARY KEY NOT NULL,
                registered     DATETIME NOT NULL DEFAULT (CURRENT_TIMESTAMP),
                name           VARCHAR(64),
                vendor_id      INTEGER REFERENCES vendor(id),
                material       VARCHAR(64),
                price          REAL,
                density        REAL NOT NULL,
                diameter       REAL NOT NULL,
                weight         REAL,
                spool_weight   REAL,
                article_number VARCHAR(64),
                comment        VARCHAR(1024)
            );
            CREATE INDEX IF NOT EXISTS ix_filament_id ON filament (id);

            CREATE TABLE IF NOT EXISTS spool (
                id          INTEGER PRIMARY KEY NOT NULL,
                registered  DATETIME NOT NULL DEFAULT (CURRENT_TIMESTAMP),
                first_used  DATETIME,
                last_used   DATETIME,
                filament_id INTEGER NOT NULL REFERENCES filament(id),
                used_weight REAL NOT NULL,
                location    VARCHAR(64),
                lot_nr      VARCHAR(64),
                comment     VARCHAR(1024)
            );
            CREATE INDEX IF NOT EXISTS ix_spool_id ON spool (id);
        ",

        // --- PostgreSQL ---
        // sa.Integer() PK → SERIAL; sa.Float() → FLOAT; sa.DateTime() → TIMESTAMP WITHOUT TIME ZONE
        postgres_sql: "
            CREATE TABLE IF NOT EXISTS vendor (
                id         SERIAL NOT NULL,
                registered TIMESTAMP WITHOUT TIME ZONE NOT NULL DEFAULT NOW(),
                name       VARCHAR(64) NOT NULL,
                comment    VARCHAR(1024),
                CONSTRAINT vendor_pkey PRIMARY KEY (id)
            );
            CREATE INDEX IF NOT EXISTS ix_vendor_id ON vendor (id);

            CREATE TABLE IF NOT EXISTS filament (
                id             SERIAL NOT NULL,
                registered     TIMESTAMP WITHOUT TIME ZONE NOT NULL DEFAULT NOW(),
                name           VARCHAR(64),
                vendor_id      INTEGER REFERENCES vendor(id),
                material       VARCHAR(64),
                price          FLOAT,
                density        FLOAT NOT NULL,
                diameter       FLOAT NOT NULL,
                weight         FLOAT,
                spool_weight   FLOAT,
                article_number VARCHAR(64),
                comment        VARCHAR(1024),
                CONSTRAINT filament_pkey PRIMARY KEY (id)
            );
            CREATE INDEX IF NOT EXISTS ix_filament_id ON filament (id);

            CREATE TABLE IF NOT EXISTS spool (
                id          SERIAL NOT NULL,
                registered  TIMESTAMP WITHOUT TIME ZONE NOT NULL DEFAULT NOW(),
                first_used  TIMESTAMP WITHOUT TIME ZONE,
                last_used   TIMESTAMP WITHOUT TIME ZONE,
                filament_id INTEGER NOT NULL REFERENCES filament(id),
                used_weight FLOAT NOT NULL,
                location    VARCHAR(64),
                lot_nr      VARCHAR(64),
                comment     VARCHAR(1024),
                CONSTRAINT spool_pkey PRIMARY KEY (id)
            );
            CREATE INDEX IF NOT EXISTS ix_spool_id ON spool (id);
        ",

        // --- MySQL / MariaDB ---
        // sa.Integer() PK → INT AUTO_INCREMENT; sa.Float() → FLOAT; sa.Boolean() → TINYINT(1)
        mysql_sql: "
            CREATE TABLE IF NOT EXISTS vendor (
                id         INT NOT NULL AUTO_INCREMENT,
                registered DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                name       VARCHAR(64) NOT NULL,
                comment    VARCHAR(1024),
                PRIMARY KEY (id)
            ) CHARACTER SET utf8mb4;
            CREATE INDEX ix_vendor_id ON vendor (id);

            CREATE TABLE IF NOT EXISTS filament (
                id             INT NOT NULL AUTO_INCREMENT,
                registered     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                name           VARCHAR(64),
                vendor_id      INT REFERENCES vendor(id),
                material       VARCHAR(64),
                price          FLOAT,
                density        FLOAT NOT NULL,
                diameter       FLOAT NOT NULL,
                weight         FLOAT,
                spool_weight   FLOAT,
                article_number VARCHAR(64),
                comment        VARCHAR(1024),
                PRIMARY KEY (id)
            ) CHARACTER SET utf8mb4;
            CREATE INDEX ix_filament_id ON filament (id);

            CREATE TABLE IF NOT EXISTS spool (
                id          INT NOT NULL AUTO_INCREMENT,
                registered  DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                first_used  DATETIME,
                last_used   DATETIME,
                filament_id INT NOT NULL REFERENCES filament(id),
                used_weight FLOAT NOT NULL,
                location    VARCHAR(64),
                lot_nr      VARCHAR(64),
                comment     VARCHAR(1024),
                PRIMARY KEY (id)
            ) CHARACTER SET utf8mb4;
            CREATE INDEX ix_spool_id ON spool (id);
        ",
    },
    // -------------------------------------------------------------------------
    // b47376d60c6d — add extruder and bed temperature override
    // -------------------------------------------------------------------------
    Migration {
        revision: "b47376d60c6d",
        description: "add extruder and bed temperature override",
        sql: "
            ALTER TABLE filament ADD COLUMN settings_extruder_temp INTEGER;
            ALTER TABLE filament ADD COLUMN settings_bed_temp INTEGER;
        ",
        postgres_sql: "",
        mysql_sql: "",
    },
    // -------------------------------------------------------------------------
    // db385b808a20 — add filament color code
    // -------------------------------------------------------------------------
    Migration {
        revision: "db385b808a20",
        description: "add filament color code",
        sql: "
            ALTER TABLE filament ADD COLUMN color_hex VARCHAR(6);
        ",
        postgres_sql: "",
        mysql_sql: "",
    },
    // -------------------------------------------------------------------------
    // 92186a5f7b0f — add spool archived field
    // -------------------------------------------------------------------------
    Migration {
        revision: "92186a5f7b0f",
        description: "add spool archived field",
        sql: "
            ALTER TABLE spool ADD COLUMN archived BOOLEAN;
        ",
        postgres_sql: "",
        // MySQL/MariaDB: sa.Boolean() → TINYINT(1)
        mysql_sql: "
            ALTER TABLE spool ADD COLUMN archived TINYINT(1);
        ",
    },
    // -------------------------------------------------------------------------
    // 92793c8a937c — color_hex alpha (widen to 8 chars)
    // -------------------------------------------------------------------------
    Migration {
        revision: "92793c8a937c",
        description: "color_hex alpha (widen to 8 chars)",
        // SQLite does not enforce VARCHAR lengths — no-op.
        sql: "",
        // PostgreSQL: ALTER COLUMN … TYPE
        postgres_sql: "
            ALTER TABLE filament ALTER COLUMN color_hex TYPE VARCHAR(8);
        ",
        // MySQL/MariaDB: MODIFY COLUMN
        mysql_sql: "
            ALTER TABLE filament MODIFY COLUMN color_hex VARCHAR(8);
        ",
    },
    // -------------------------------------------------------------------------
    // b82cd9e2aa6f — added spool prices
    // -------------------------------------------------------------------------
    Migration {
        revision: "b82cd9e2aa6f",
        description: "added spool prices",
        sql: "
            ALTER TABLE spool ADD COLUMN price REAL;
        ",
        postgres_sql: "
            ALTER TABLE spool ADD COLUMN price FLOAT;
        ",
        mysql_sql: "
            ALTER TABLE spool ADD COLUMN price FLOAT;
        ",
    },
    // -------------------------------------------------------------------------
    // ccbb17aeda7c — added settings table
    // -------------------------------------------------------------------------
    Migration {
        revision: "ccbb17aeda7c",
        description: "added settings table",
        sql: "
            CREATE TABLE IF NOT EXISTS setting (
                key          VARCHAR(64) PRIMARY KEY NOT NULL,
                value        TEXT NOT NULL,
                last_updated DATETIME NOT NULL
            );
            CREATE INDEX IF NOT EXISTS ix_setting_key ON setting (key);
        ",
        postgres_sql: "
            CREATE TABLE IF NOT EXISTS setting (
                key          VARCHAR(64) NOT NULL,
                value        TEXT NOT NULL,
                last_updated TIMESTAMP WITHOUT TIME ZONE NOT NULL,
                CONSTRAINT setting_pkey PRIMARY KEY (key)
            );
            CREATE INDEX IF NOT EXISTS ix_setting_key ON setting (key);
        ",
        mysql_sql: "
            CREATE TABLE IF NOT EXISTS setting (
                key          VARCHAR(64) NOT NULL,
                value        TEXT NOT NULL,
                last_updated DATETIME NOT NULL,
                PRIMARY KEY (key)
            ) CHARACTER SET utf8mb4;
            CREATE INDEX ix_setting_key ON setting (key);
        ",
    },
    // -------------------------------------------------------------------------
    // b8881bdb716c — added extra fields
    // -------------------------------------------------------------------------
    Migration {
        revision: "b8881bdb716c",
        description: "added extra fields",
        sql: "
            CREATE TABLE IF NOT EXISTS vendor_field (
                vendor_id INTEGER NOT NULL REFERENCES vendor(id),
                key       VARCHAR(64) NOT NULL,
                value     TEXT NOT NULL,
                PRIMARY KEY (vendor_id, key)
            );
            CREATE INDEX IF NOT EXISTS ix_vendor_field_key       ON vendor_field (key);
            CREATE INDEX IF NOT EXISTS ix_vendor_field_vendor_id ON vendor_field (vendor_id);

            CREATE TABLE IF NOT EXISTS filament_field (
                filament_id INTEGER NOT NULL REFERENCES filament(id),
                key         VARCHAR(64) NOT NULL,
                value       TEXT NOT NULL,
                PRIMARY KEY (filament_id, key)
            );
            CREATE INDEX IF NOT EXISTS ix_filament_field_filament_id ON filament_field (filament_id);
            CREATE INDEX IF NOT EXISTS ix_filament_field_key          ON filament_field (key);

            CREATE TABLE IF NOT EXISTS spool_field (
                spool_id INTEGER NOT NULL REFERENCES spool(id),
                key      VARCHAR(64) NOT NULL,
                value    TEXT NOT NULL,
                PRIMARY KEY (spool_id, key)
            );
            CREATE INDEX IF NOT EXISTS ix_spool_field_key      ON spool_field (key);
            CREATE INDEX IF NOT EXISTS ix_spool_field_spool_id ON spool_field (spool_id);
        ",
        postgres_sql: "
            CREATE TABLE IF NOT EXISTS vendor_field (
                vendor_id INTEGER NOT NULL REFERENCES vendor(id),
                key       VARCHAR(64) NOT NULL,
                value     TEXT NOT NULL,
                CONSTRAINT vendor_field_pkey PRIMARY KEY (vendor_id, key)
            );
            CREATE INDEX IF NOT EXISTS ix_vendor_field_key       ON vendor_field (key);
            CREATE INDEX IF NOT EXISTS ix_vendor_field_vendor_id ON vendor_field (vendor_id);

            CREATE TABLE IF NOT EXISTS filament_field (
                filament_id INTEGER NOT NULL REFERENCES filament(id),
                key         VARCHAR(64) NOT NULL,
                value       TEXT NOT NULL,
                CONSTRAINT filament_field_pkey PRIMARY KEY (filament_id, key)
            );
            CREATE INDEX IF NOT EXISTS ix_filament_field_filament_id ON filament_field (filament_id);
            CREATE INDEX IF NOT EXISTS ix_filament_field_key          ON filament_field (key);

            CREATE TABLE IF NOT EXISTS spool_field (
                spool_id INTEGER NOT NULL REFERENCES spool(id),
                key      VARCHAR(64) NOT NULL,
                value    TEXT NOT NULL,
                CONSTRAINT spool_field_pkey PRIMARY KEY (spool_id, key)
            );
            CREATE INDEX IF NOT EXISTS ix_spool_field_key      ON spool_field (key);
            CREATE INDEX IF NOT EXISTS ix_spool_field_spool_id ON spool_field (spool_id);
        ",
        mysql_sql: "
            CREATE TABLE IF NOT EXISTS vendor_field (
                vendor_id INT NOT NULL REFERENCES vendor(id),
                key       VARCHAR(64) NOT NULL,
                value     TEXT NOT NULL,
                PRIMARY KEY (vendor_id, key)
            ) CHARACTER SET utf8mb4;
            CREATE INDEX ix_vendor_field_key       ON vendor_field (key);
            CREATE INDEX ix_vendor_field_vendor_id ON vendor_field (vendor_id);

            CREATE TABLE IF NOT EXISTS filament_field (
                filament_id INT NOT NULL REFERENCES filament(id),
                key         VARCHAR(64) NOT NULL,
                value       TEXT NOT NULL,
                PRIMARY KEY (filament_id, key)
            ) CHARACTER SET utf8mb4;
            CREATE INDEX ix_filament_field_filament_id ON filament_field (filament_id);
            CREATE INDEX ix_filament_field_key          ON filament_field (key);

            CREATE TABLE IF NOT EXISTS spool_field (
                spool_id INT NOT NULL REFERENCES spool(id),
                key      VARCHAR(64) NOT NULL,
                value    TEXT NOT NULL,
                PRIMARY KEY (spool_id, key)
            ) CHARACTER SET utf8mb4;
            CREATE INDEX ix_spool_field_key      ON spool_field (key);
            CREATE INDEX ix_spool_field_spool_id ON spool_field (spool_id);
        ",
    },
    // -------------------------------------------------------------------------
    // aafcd7fb0e84 — spool weights
    // -------------------------------------------------------------------------
    Migration {
        revision: "aafcd7fb0e84",
        description: "spool weights",
        sql: "
            ALTER TABLE spool ADD COLUMN initial_weight REAL;
            ALTER TABLE spool ADD COLUMN spool_weight   REAL;
        ",
        postgres_sql: "
            ALTER TABLE spool ADD COLUMN initial_weight FLOAT;
            ALTER TABLE spool ADD COLUMN spool_weight   FLOAT;
        ",
        mysql_sql: "
            ALTER TABLE spool ADD COLUMN initial_weight FLOAT;
            ALTER TABLE spool ADD COLUMN spool_weight   FLOAT;
        ",
    },
    // -------------------------------------------------------------------------
    // 304a32906234 — spool weight population (data migration, engine-independent)
    // -------------------------------------------------------------------------
    Migration {
        revision: "304a32906234",
        description: "spool weight population",
        sql: "
            UPDATE spool
               SET initial_weight = (SELECT weight       FROM filament WHERE filament.id = spool.filament_id),
                   spool_weight   = (SELECT spool_weight FROM filament WHERE filament.id = spool.filament_id);
        ",
        postgres_sql: "",
        mysql_sql: "",
    },
    // -------------------------------------------------------------------------
    // 5f069e51bd89 — vendor empty spool weight
    // -------------------------------------------------------------------------
    Migration {
        revision: "5f069e51bd89",
        description: "vendor empty spool weight",
        sql: "
            ALTER TABLE vendor ADD COLUMN empty_spool_weight REAL;
        ",
        postgres_sql: "
            ALTER TABLE vendor ADD COLUMN empty_spool_weight FLOAT;
        ",
        mysql_sql: "
            ALTER TABLE vendor ADD COLUMN empty_spool_weight FLOAT;
        ",
    },
    // -------------------------------------------------------------------------
    // 395d560284b3 — added external ID
    // -------------------------------------------------------------------------
    Migration {
        revision: "395d560284b3",
        description: "added external ID",
        sql: "
            ALTER TABLE filament ADD COLUMN external_id VARCHAR(256);
            ALTER TABLE vendor   ADD COLUMN external_id VARCHAR(256);
        ",
        postgres_sql: "",
        mysql_sql: "",
    },
    // -------------------------------------------------------------------------
    // 415a8f855e14 — multi colors
    // -------------------------------------------------------------------------
    Migration {
        revision: "415a8f855e14",
        description: "multi colors",
        sql: "
            ALTER TABLE filament ADD COLUMN multi_color_hexes     VARCHAR(128);
            ALTER TABLE filament ADD COLUMN multi_color_direction VARCHAR(16);
        ",
        postgres_sql: "",
        mysql_sql: "",
    },
    // -------------------------------------------------------------------------
    // 9c1d5f2a7b31 — strip color_hex hash (data migration, engine-independent)
    // -------------------------------------------------------------------------
    Migration {
        revision: "9c1d5f2a7b31",
        description: "strip color_hex hash",
        // REPLACE() is standard SQL supported on SQLite, Postgres, and MySQL.
        sql: "
            UPDATE filament
               SET color_hex = REPLACE(color_hex, '#', '')
             WHERE color_hex LIKE '%#%';

            UPDATE filament
               SET multi_color_hexes = REPLACE(multi_color_hexes, '#', '')
             WHERE multi_color_hexes LIKE '%#%';
        ",
        postgres_sql: "",
        mysql_sql: "",
    },
    // -------------------------------------------------------------------------
    // fe4970567bb3 — tags (HEAD)
    // -------------------------------------------------------------------------
    Migration {
        revision: "fe4970567bb3",
        description: "tags",
        sql: "
            CREATE TABLE IF NOT EXISTS tag (
                id           INTEGER PRIMARY KEY NOT NULL,
                uid          VARCHAR(64) NOT NULL,
                target_type  VARCHAR(16) NOT NULL,
                spool_id     INTEGER REFERENCES spool(id),
                filament_id  INTEGER REFERENCES filament(id) ON DELETE CASCADE,
                target_value VARCHAR(64),
                format       VARCHAR(32),
                added        DATETIME NOT NULL
            );
            CREATE INDEX IF NOT EXISTS ix_tag_id          ON tag (id);
            CREATE INDEX IF NOT EXISTS ix_tag_spool_id    ON tag (spool_id);
            CREATE INDEX IF NOT EXISTS ix_tag_filament_id ON tag (filament_id);
            CREATE UNIQUE INDEX IF NOT EXISTS ix_tag_uid  ON tag (uid);
        ",
        postgres_sql: "
            CREATE TABLE IF NOT EXISTS tag (
                id           SERIAL NOT NULL,
                uid          VARCHAR(64) NOT NULL,
                target_type  VARCHAR(16) NOT NULL,
                spool_id     INTEGER REFERENCES spool(id),
                filament_id  INTEGER REFERENCES filament(id) ON DELETE CASCADE,
                target_value VARCHAR(64),
                format       VARCHAR(32),
                added        TIMESTAMP WITHOUT TIME ZONE NOT NULL,
                CONSTRAINT tag_pkey PRIMARY KEY (id)
            );
            CREATE INDEX IF NOT EXISTS ix_tag_id          ON tag (id);
            CREATE INDEX IF NOT EXISTS ix_tag_spool_id    ON tag (spool_id);
            CREATE INDEX IF NOT EXISTS ix_tag_filament_id ON tag (filament_id);
            CREATE UNIQUE INDEX IF NOT EXISTS ix_tag_uid  ON tag (uid);
        ",
        mysql_sql: "
            CREATE TABLE IF NOT EXISTS tag (
                id           INT NOT NULL AUTO_INCREMENT,
                uid          VARCHAR(64) NOT NULL,
                target_type  VARCHAR(16) NOT NULL,
                spool_id     INT REFERENCES spool(id),
                filament_id  INT REFERENCES filament(id),
                target_value VARCHAR(64),
                format       VARCHAR(32),
                added        DATETIME NOT NULL,
                PRIMARY KEY (id),
                UNIQUE KEY ix_tag_uid (uid)
            ) CHARACTER SET utf8mb4;
            CREATE INDEX ix_tag_id          ON tag (id);
            CREATE INDEX ix_tag_spool_id    ON tag (spool_id);
            CREATE INDEX ix_tag_filament_id ON tag (filament_id);
        ",
    },
];

/// The revision ID that [`CHAIN`] ends at — Alembic "head".
pub const HEAD: &str = "fe4970567bb3";

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Apply every migration from (but not including) `current_revision` through HEAD.
///
/// Each step executes the engine-appropriate SQL and updates `alembic_version`
/// within the same transaction so the DB is never left in a half-migrated state.
///
/// If `current_revision` is `None` the full set of migrations is applied
/// (fresh-database path).
pub async fn apply_from(
    pool: &AnyPool,
    current_revision: Option<&str>,
    engine: &Engine,
) -> Result<()> {
    // Find where in the chain to start.
    let start_index = match current_revision {
        None => 0,
        Some(rev) => {
            let pos = CHAIN
                .iter()
                .position(|m| m.revision == rev)
                .ok_or_else(|| SpoolmanError::UnknownRevision(rev.to_owned()))?;
            pos + 1 // start *after* the current revision
        }
    };

    if start_index >= CHAIN.len() {
        // Already at or past HEAD — nothing to do.
        return Ok(());
    }

    for migration in &CHAIN[start_index..] {
        tracing::info!(
            revision = migration.revision,
            description = migration.description,
            "applying migration"
        );

        let mut tx = pool.begin().await?;

        let sql = migration.sql_for(engine);
        if !sql.trim().is_empty() {
            // Split on ";" and run each non-empty statement separately.
            // (sqlx execute() processes one statement at a time.)
            for stmt in sql.split(';') {
                let stmt = stmt.trim();
                if !stmt.is_empty() {
                    sqlx::query(stmt).execute(&mut *tx).await?;
                }
            }
        }

        // Record the new revision — same value alembic would write.
        sqlx::query(sqlx::AssertSqlSafe(&*tp(
            "UPDATE alembic_version SET version_num = ?",
        )))
        .bind(migration.revision)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        tracing::info!(revision = migration.revision, "migration applied");
    }

    Ok(())
}
