//! Alembic-compatible migration runner for spoolman-rs.
//!
//! Each entry in [`CHAIN`] is a `Migration`: a revision ID string and the SQL
//! that brings the schema from the previous revision to that revision.  The
//! SQL is hand-ported from the upstream Python Alembic files in
//! `Donkie/Spoolman` tag v0.27.1 (`migrations/versions/`).
//!
//! SQLite dialect only — session 5 adds Postgres and MySQL paths.

use sqlx::SqlitePool;

use crate::error::{Result, SpoolmanError};

/// One step in the alembic revision chain.
pub struct Migration {
    pub revision: &'static str,
    pub description: &'static str,
    pub sql: &'static str,
}

/// The ordered chain of all 15 migrations, oldest → newest.
///
/// HEAD = `fe4970567bb3`.
pub static CHAIN: &[Migration] = &[
    Migration {
        revision: "684d32cf7e4d",
        description: "initial",
        sql: "
            CREATE TABLE IF NOT EXISTS vendor (
                id        INTEGER PRIMARY KEY NOT NULL,
                registered DATETIME NOT NULL DEFAULT (CURRENT_TIMESTAMP),
                name      VARCHAR(64) NOT NULL,
                comment   VARCHAR(1024)
            );
            CREATE INDEX IF NOT EXISTS ix_vendor_id ON vendor (id);

            CREATE TABLE IF NOT EXISTS filament (
                id          INTEGER PRIMARY KEY NOT NULL,
                registered  DATETIME NOT NULL DEFAULT (CURRENT_TIMESTAMP),
                name        VARCHAR(64),
                vendor_id   INTEGER REFERENCES vendor(id),
                material    VARCHAR(64),
                price       REAL,
                density     REAL NOT NULL,
                diameter    REAL NOT NULL,
                weight      REAL,
                spool_weight REAL,
                article_number VARCHAR(64),
                comment     VARCHAR(1024)
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
    },
    Migration {
        revision: "b47376d60c6d",
        description: "add extruder and bed temperature override",
        sql: "
            ALTER TABLE filament ADD COLUMN settings_extruder_temp INTEGER;
            ALTER TABLE filament ADD COLUMN settings_bed_temp INTEGER;
        ",
    },
    Migration {
        revision: "db385b808a20",
        description: "add filament color code",
        sql: "
            ALTER TABLE filament ADD COLUMN color_hex VARCHAR(6);
        ",
    },
    Migration {
        revision: "92186a5f7b0f",
        description: "add spool archived field",
        sql: "
            ALTER TABLE spool ADD COLUMN archived BOOLEAN;
        ",
    },
    Migration {
        revision: "92793c8a937c",
        description: "color_hex alpha (widen to 8 chars)",
        // SQLite does not enforce VARCHAR lengths, so this is a no-op at the
        // data level; we record the revision to stay in sync with alembic_version.
        sql: "",
    },
    Migration {
        revision: "b82cd9e2aa6f",
        description: "added spool prices",
        sql: "
            ALTER TABLE spool ADD COLUMN price REAL;
        ",
    },
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
    },
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
    },
    Migration {
        revision: "aafcd7fb0e84",
        description: "spool weights",
        sql: "
            ALTER TABLE spool ADD COLUMN initial_weight REAL;
            ALTER TABLE spool ADD COLUMN spool_weight   REAL;
        ",
    },
    Migration {
        revision: "304a32906234",
        description: "spool weight population",
        // Back-fill spool.initial_weight / spool.spool_weight from their filament row.
        sql: "
            UPDATE spool
               SET initial_weight = (SELECT weight       FROM filament WHERE filament.id = spool.filament_id),
                   spool_weight   = (SELECT spool_weight FROM filament WHERE filament.id = spool.filament_id);
        ",
    },
    Migration {
        revision: "5f069e51bd89",
        description: "vendor empty spool weight",
        sql: "
            ALTER TABLE vendor ADD COLUMN empty_spool_weight REAL;
        ",
    },
    Migration {
        revision: "395d560284b3",
        description: "added external ID",
        sql: "
            ALTER TABLE filament ADD COLUMN external_id VARCHAR(256);
            ALTER TABLE vendor   ADD COLUMN external_id VARCHAR(256);
        ",
    },
    Migration {
        revision: "415a8f855e14",
        description: "multi colors",
        sql: "
            ALTER TABLE filament ADD COLUMN multi_color_hexes     VARCHAR(128);
            ALTER TABLE filament ADD COLUMN multi_color_direction VARCHAR(16);
        ",
    },
    Migration {
        revision: "9c1d5f2a7b31",
        description: "strip color_hex hash",
        // Strip any leading '#' characters that were stored in color_hex /
        // multi_color_hexes — a data-only migration.
        sql: "
            UPDATE filament
               SET color_hex = REPLACE(color_hex, '#', '')
             WHERE color_hex LIKE '%#%';

            UPDATE filament
               SET multi_color_hexes = REPLACE(multi_color_hexes, '#', '')
             WHERE multi_color_hexes LIKE '%#%';
        ",
    },
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
    },
];

/// The revision ID that [`CHAIN`] ends at — Alembic "head".
pub const HEAD: &str = "fe4970567bb3";

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Apply every migration from (but not including) `current_revision` through HEAD.
///
/// Each step executes the migration SQL and updates `alembic_version` within
/// the same transaction so the DB is never left in a half-migrated state.
///
/// If `current_revision` is `None` the full set of migrations is applied
/// (fresh-database path).
pub async fn apply_from(pool: &SqlitePool, current_revision: Option<&str>) -> Result<()> {
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

        // Execute the migration SQL (may be empty for no-op schema steps).
        if !migration.sql.trim().is_empty() {
            // sqlx execute() only runs one statement at a time for SQLite;
            // split on ";" and run each non-empty statement separately.
            for stmt in migration.sql.split(';') {
                let stmt = stmt.trim();
                if !stmt.is_empty() {
                    sqlx::query(stmt).execute(&mut *tx).await?;
                }
            }
        }

        // Record the new revision — same value alembic would write.
        sqlx::query("UPDATE alembic_version SET version_num = $1")
            .bind(migration.revision)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        tracing::info!(revision = migration.revision, "migration applied");
    }

    Ok(())
}
