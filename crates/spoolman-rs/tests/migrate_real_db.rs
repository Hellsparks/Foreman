//! Migration test against real Spoolman databases.
//!
//! Every `*.db` in `tests/fixtures/` (gitignored: personal data) is copied to a
//! temp dir, opened with `Db::connect` (which migrates it to HEAD), and compared
//! with an untouched copy: row counts, every spool / filament / vendor column,
//! extra field values and settings must survive unchanged. Then every row is
//! read back through spoolman-rs's own query path.
//!
//! No fixture present (CI) → the test prints a note and passes.
//!
//! Run: `cargo test -p spoolman-rs --test migrate_real_db -- --nocapture`

use std::path::{Path, PathBuf};

use spoolman_rs::{
    config::{DbType, SpoolmanConfig},
    db::{filament, migrations, spool, vendor, Db},
};
use sqlx::{AnyPool, Row};

fn fixtures() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "db"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn config(dir: &Path) -> SpoolmanConfig {
    SpoolmanConfig {
        db_type: DbType::Sqlite,
        db_host: None,
        db_port: None,
        db_name: None,
        db_username: None,
        db_password: None,
        data_dir: dir.to_path_buf(),
        logs_dir: dir.to_path_buf(),
        backups_dir: dir.join("backups"),
        host: "127.0.0.1".into(),
        port: 0,
        base_path: String::new(),
        automatic_backup: false,
    }
}

/// Every row of a query as one string per row (columns joined), sorted.
async fn dump(pool: &AnyPool, sql: &str) -> Vec<String> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .fetch_all(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    let mut out: Vec<String> = rows
        .iter()
        .map(|r| {
            (0..r.len())
                .map(|i| {
                    r.try_get::<Option<String>, _>(i)
                        .unwrap_or_else(|e| panic!("{sql} col {i}: {e}"))
                        .unwrap_or_else(|| "NULL".into())
                })
                .collect::<Vec<_>>()
                .join("|")
        })
        .collect();
    out.sort();
    out
}

/// What must not change: table → data columns (schema changes may add columns).
const SNAPSHOT: &[(&str, &[&str])] = &[
    (
        "vendor",
        &["id", "name", "comment", "empty_spool_weight", "external_id"],
    ),
    (
        "filament",
        &[
            "id",
            "name",
            "vendor_id",
            "material",
            "price",
            "density",
            "diameter",
            "weight",
            "spool_weight",
            "article_number",
            "comment",
            "settings_extruder_temp",
            "settings_bed_temp",
            "color_hex",
            "multi_color_hexes",
            "multi_color_direction",
            "external_id",
        ],
    ),
    (
        "spool",
        &[
            "id",
            "filament_id",
            "used_weight",
            "initial_weight",
            "spool_weight",
            "location",
            "lot_nr",
            "comment",
            "archived",
            "price",
            "first_used",
            "last_used",
            "registered",
        ],
    ),
    ("vendor_field", &["vendor_id", "key", "value"]),
    ("filament_field", &["filament_id", "key", "value"]),
    ("spool_field", &["spool_id", "key", "value"]),
    ("setting", &["key", "value"]),
];

/// Every column cast to TEXT: the Any driver cannot decode SQLite BOOL / DATETIME.
fn select(table: &str, cols: &[&str]) -> String {
    let c: Vec<String> = cols.iter().map(|c| format!("CAST({c} AS TEXT)")).collect();
    format!("SELECT {} FROM {table}", c.join(", "))
}

async fn snapshot(pool: &AnyPool) -> Vec<(&'static str, Vec<String>)> {
    let mut out = vec![];
    for (name, cols) in SNAPSHOT {
        out.push((*name, dump(pool, &select(name, cols)).await));
    }
    out
}

#[tokio::test]
async fn real_databases_migrate_to_head_without_losing_data() {
    let list = fixtures();
    if list.is_empty() {
        eprintln!("migrate_real_db: no tests/fixtures/*.db (gitignored); nothing to check");
        return;
    }
    sqlx::any::install_default_drivers();
    for src in list {
        let name = src.file_name().map(|n| n.to_string_lossy().into_owned());
        let name = name.unwrap_or_default();

        // Untouched copy → the "before" snapshot.
        let before_dir = tempfile::TempDir::new().expect("tmp");
        let before_path = before_dir.path().join("before.db");
        std::fs::copy(&src, &before_path).expect("copy before");
        let url = format!(
            "sqlite://{}?mode=ro",
            before_path.display().to_string().replace('\\', "/")
        );
        let pool = AnyPool::connect(&url).await.expect("open before");
        let from: String = sqlx::query_scalar("SELECT version_num FROM alembic_version")
            .fetch_one(&pool)
            .await
            .expect("revision");
        let before = snapshot(&pool).await;
        pool.close().await;

        // Migrated copy.
        let dir = tempfile::TempDir::new().expect("tmp");
        std::fs::copy(&src, dir.path().join("spoolman.db")).expect("copy");
        let db = Db::connect(&config(dir.path()))
            .await
            .expect("connect + migrate");
        let to: String = sqlx::query_scalar("SELECT version_num FROM alembic_version")
            .fetch_one(&db.pool)
            .await
            .expect("revision");
        assert_eq!(to, migrations::HEAD, "{name}");
        let after = snapshot(&db.pool).await;

        let mut summary = vec![];
        for ((t, b), (_, a)) in before.iter().zip(after.iter()) {
            assert_eq!(b.len(), a.len(), "{name}: {t} row count changed");
            for (rb, ra) in b.iter().zip(a.iter()) {
                assert_eq!(rb, ra, "{name}: {t} row changed");
            }
            summary.push(format!("{t}={}", a.len()));
        }

        // Read everything back through spoolman-rs (catches decode errors on real data).
        let (spools, n_sp) = spool::find(
            &db.pool,
            &db.engine,
            &spool::SpoolFilter {
                allow_archived: true,
                ..Default::default()
            },
        )
        .await
        .expect("spools via spoolman-rs");
        let (fils, n_fil) = filament::find(&db.pool, &db.engine, &Default::default())
            .await
            .expect("filaments via spoolman-rs");
        let (vends, n_v) = vendor::find(&db.pool, &db.engine, &Default::default())
            .await
            .expect("vendors via spoolman-rs");
        let count = |t: &str| {
            before
                .iter()
                .find(|x| x.0 == t)
                .map(|x| x.1.len())
                .unwrap_or(0)
        };
        assert_eq!(
            (spools.len(), n_sp as usize),
            (count("spool"), count("spool")),
            "{name}"
        );
        assert_eq!(
            (fils.len(), n_fil as usize),
            (count("filament"), count("filament")),
            "{name}"
        );
        assert_eq!(
            (vends.len(), n_v as usize),
            (count("vendor"), count("vendor")),
            "{name}"
        );
        // Extra values come back as raw JSON strings, as upstream stores them.
        let extras: usize = fils.iter().map(|f| f.extra.len()).sum();
        assert_eq!(extras, count("filament_field"), "{name}: filament extras");

        eprintln!(
            "migrate_real_db: {name}: {from} → {to}; {}",
            summary.join(" ")
        );
        db.pool.close().await;
    }
}
