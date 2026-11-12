//! Backup + restore (task 6.2): round trip incl. a stored SSH password, consistent
//! snapshot under writes, refusing newer schemas, retention, Attention on failure.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use axum::http::StatusCode;
use foreman_core::{db::ForemanDb, secrets::SecretBox};
use serde_json::{json, Value};
use spoolman_rs::{
    config::{DbType, SpoolmanConfig},
    db::Db,
};

use crate::{
    backup::{self, BackupKind, BackupOptions, Sources},
    spools::Spoolman,
    spools_tests::{add_spool, call, setup},
};

fn sm_config(dir: &Path) -> SpoolmanConfig {
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

/// A data dir with everything a backup carries: a printer with an encrypted SSH
/// password, 2 spools, theme files and one Library file.
struct Live {
    _tmp: tempfile::TempDir,
    data: PathBuf,
    sm_dir: PathBuf,
    out: PathBuf,
}

impl Live {
    fn sources(&self) -> Sources {
        Sources {
            data_dir: self.data.clone(),
            spoolman_db: Some(self.sm_dir.join("spoolman.db")),
        }
    }
}

async fn live() -> Live {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let data = tmp.path().join("data");
    let sm_dir = tmp.path().join("sm");
    let out = tmp.path().join("backups");
    fs::create_dir_all(&sm_dir).expect("sm dir");

    let db = ForemanDb::connect(&data.join("foreman.db"))
        .await
        .expect("db");
    let secrets = SecretBox::load_or_create(&data.join("secret.key")).expect("key");
    sqlx::query(
        "INSERT INTO printer (id, name, firmware, tags, created_at, updated_at) \
         VALUES (1, 'HellV0', 'moonraker', '[]', 'x', 'x')",
    )
    .execute(&db.pool)
    .await
    .expect("printer");
    sqlx::query(
        "INSERT INTO printer_ssh (printer_id, ssh_user, ssh_port, auth_type, password, \
         created_at, updated_at) VALUES (1, 'pi', 22, 'password', ?, 'x', 'x')",
    )
    .bind(secrets.seal("hunter2").expect("seal"))
    .execute(&db.pool)
    .await
    .expect("ssh");
    db.pool.close().await;

    let cfg = sm_config(&sm_dir);
    let sm = Spoolman::new(Db::connect(&cfg).await.expect("sm db"), cfg);
    add_spool(&sm, "PLA", "1d1d22", 800.0).await;
    add_spool(&sm, "PETG", "ff0000", 250.0).await;
    sm.db.pool.close().await;

    fs::create_dir_all(data.join("fonts")).expect("fonts");
    fs::write(data.join("fonts/body.woff2"), b"wOF2 font bytes").expect("font");
    fs::write(data.join("custom.css"), b":root { --x: 1px; }").expect("css");
    fs::create_dir_all(data.join("library/Parts")).expect("lib");
    fs::write(data.join("library/Parts/cube.gcode"), b"G28\nG1 X10\n").expect("gcode");
    Live {
        _tmp: tmp,
        data,
        sm_dir,
        out,
    }
}

fn opts(out: &Path, library: bool, pass: Option<&str>) -> BackupOptions {
    BackupOptions {
        dest_dir: out.to_path_buf(),
        include_library: library,
        passphrase: pass.map(str::to_owned),
        kind: BackupKind::Manual,
    }
}

async fn printer_names(data: &Path) -> Vec<String> {
    let db = ForemanDb::connect(&data.join("foreman.db"))
        .await
        .expect("db");
    let v = sqlx::query_scalar("SELECT name FROM printer ORDER BY id")
        .fetch_all(&db.pool)
        .await
        .expect("names");
    db.pool.close().await;
    v
}

async fn ssh_password(data: &Path) -> String {
    let db = ForemanDb::connect(&data.join("foreman.db"))
        .await
        .expect("db");
    let sealed = foreman_core::repo::printer_ssh::get_sealed_password(&db.pool, 1)
        .await
        .expect("query")
        .expect("password stored");
    db.pool.close().await;
    SecretBox::load_or_create(&data.join("secret.key"))
        .expect("key")
        .open(&sealed)
        .expect("decrypts with the restored secret.key")
}

async fn spool_count(sm_db: &Path) -> i64 {
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", sm_db.display()))
        .await
        .expect("sm");
    let n = sqlx::query_scalar("SELECT COUNT(*) FROM spool")
        .fetch_one(&pool)
        .await
        .expect("count");
    pool.close().await;
    n
}

#[tokio::test]
async fn round_trip_restores_everything_into_a_wiped_data_dir() {
    let l = live().await;
    let key_before = fs::read(l.data.join("secret.key")).expect("key");
    let e = backup::create(&l.sources(), &opts(&l.out, true, Some("correct horse")))
        .await
        .expect("backup");
    assert!(e.encrypted && e.file.ends_with(".zip.age"), "{}", e.file);
    let m = e.manifest.clone().expect("manifest");
    assert_eq!(m.counts.printers, 1);
    assert_eq!(m.counts.spools, 2);
    assert!(m.library_included);
    assert!(m.files.iter().any(|f| f.path == "secret.key"));
    assert!(m.files.iter().any(|f| f.path == "library/Parts/cube.gcode"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&e.path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "backup must be owner-only");
    }
    // The listing reads the sidecar, so it works without the passphrase.
    let listed = backup::list(&l.out);
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].manifest.as_ref().map(|m| m.counts.spools),
        Some(2)
    );
    // Wrong / missing passphrase.
    let err = backup::inspect(Path::new(&e.path), Some("wrong"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Wrong passphrase"), "{err}");
    assert!(backup::inspect(Path::new(&e.path), None).await.is_err());

    // Wipe everything.
    fs::remove_dir_all(&l.data).expect("wipe data");
    fs::remove_dir_all(&l.sm_dir).expect("wipe sm");
    fs::create_dir_all(&l.data).expect("empty data");

    backup::stage(&l.data, Path::new(&e.path), Some("correct horse"))
        .await
        .expect("stage");
    let applied = backup::apply_pending(&l.sources())
        .await
        .expect("apply")
        .expect("something applied");
    assert!(
        applied.snapshot.is_none(),
        "nothing to snapshot in an empty dir"
    );
    assert!(!l.data.join(backup::PENDING_DIR).exists());

    assert_eq!(printer_names(&l.data).await, vec!["HellV0".to_owned()]);
    assert_eq!(ssh_password(&l.data).await, "hunter2");
    assert_eq!(
        fs::read(l.data.join("secret.key")).expect("key"),
        key_before
    );
    assert_eq!(spool_count(&l.sm_dir.join("spoolman.db")).await, 2);
    assert_eq!(
        fs::read(l.data.join("fonts/body.woff2")).expect("font"),
        b"wOF2 font bytes"
    );
    assert_eq!(
        fs::read(l.data.join("custom.css")).expect("css"),
        b":root { --x: 1px; }"
    );
    assert_eq!(
        fs::read(l.data.join("library/Parts/cube.gcode")).expect("gcode"),
        b"G28\nG1 X10\n"
    );
    // Nothing left pending: a second start applies nothing.
    assert!(backup::apply_pending(&l.sources())
        .await
        .expect("again")
        .is_none());
}

#[tokio::test]
async fn restore_saves_the_current_state_first_and_that_snapshot_undoes_it() {
    let l = live().await;
    let e = backup::create(&l.sources(), &opts(&l.out, false, None))
        .await
        .expect("backup");
    // Change things after the backup.
    let db = ForemanDb::connect(&l.data.join("foreman.db"))
        .await
        .expect("db");
    sqlx::query(
        "INSERT INTO printer (id, name, firmware, tags, created_at, updated_at) \
         VALUES (2, 'Added later', 'moonraker', '[]', 'x', 'x')",
    )
    .execute(&db.pool)
    .await
    .expect("printer");
    db.pool.close().await;
    fs::write(l.data.join("custom.css"), b"/* newer */").expect("css");

    backup::stage(&l.data, Path::new(&e.path), None)
        .await
        .expect("stage");
    let a = backup::apply_pending(&l.sources())
        .await
        .expect("apply")
        .expect("applied");
    assert_eq!(printer_names(&l.data).await, vec!["HellV0".to_owned()]);
    // Library was not in the backup: left alone.
    assert!(l.data.join("library/Parts/cube.gcode").is_file());

    let snap = a.snapshot.expect("pre-restore snapshot");
    assert!(snap
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with("foreman-pre-restore-")));
    backup::stage(&l.data, &snap, None)
        .await
        .expect("stage undo");
    backup::apply_pending(&l.sources())
        .await
        .expect("undo")
        .expect("applied");
    assert_eq!(
        printer_names(&l.data).await,
        vec!["HellV0".to_owned(), "Added later".to_owned()]
    );
    assert_eq!(
        fs::read(l.data.join("custom.css")).expect("css"),
        b"/* newer */"
    );
    assert_eq!(ssh_password(&l.data).await, "hunter2");
}

#[tokio::test]
async fn snapshot_is_consistent_while_writes_are_happening() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let db = ForemanDb::connect(&tmp.path().join("foreman.db"))
        .await
        .expect("db");
    sqlx::query("CREATE TABLE pair (id INTEGER PRIMARY KEY, half INTEGER NOT NULL)")
        .execute(&db.pool)
        .await
        .expect("table");
    // Each transaction writes two rows: a consistent copy always has an even count.
    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let pool = db.pool.clone();
        let stop = Arc::clone(&stop);
        tokio::spawn(async move {
            let mut n = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let mut tx = pool.begin().await.expect("begin");
                for half in 0..2 {
                    sqlx::query("INSERT INTO pair (half) VALUES (?)")
                        .bind(half)
                        .execute(&mut *tx)
                        .await
                        .expect("insert");
                }
                tx.commit().await.expect("commit");
                n += 1;
            }
            n
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let mut counts = Vec::new();
    for i in 0..6 {
        let snap = tmp.path().join(format!("snap-{i}.db"));
        backup::snapshot_sqlite(&tmp.path().join("foreman.db"), &snap)
            .await
            .expect("snapshot");
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", snap.display()))
            .await
            .expect("open snap");
        let ok: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&pool)
            .await
            .expect("check");
        assert_eq!(ok, "ok");
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pair")
            .fetch_one(&pool)
            .await
            .expect("count");
        pool.close().await;
        assert_eq!(
            n % 2,
            0,
            "snapshot {i} caught half a transaction ({n} rows)"
        );
        counts.push(n);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    stop.store(true, Ordering::Relaxed);
    let txs = writer.await.expect("writer");
    assert!(txs > 10, "writer must have been busy ({txs} transactions)");
    assert!(
        counts.first() < counts.last(),
        "writes happened between snapshots: {counts:?}"
    );
}

/// Rewrite a plain backup zip with a changed manifest and/or foreman.db.
fn tamper(src: &Path, dest: &Path, edit: impl Fn(&mut Value), foreman_db: Option<&Path>) {
    let mut zin = zip::ZipArchive::new(fs::File::open(src).expect("open")).expect("zip");
    let mut zout = zip::ZipWriter::new(fs::File::create(dest).expect("create"));
    let o = zip::write::SimpleFileOptions::default();
    for i in 0..zin.len() {
        let mut f = zin.by_index(i).expect("entry");
        let name = f.name().expect("name").to_string();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf).expect("read");
        if name == "manifest.json" {
            let mut v: Value = serde_json::from_slice(&buf).expect("json");
            edit(&mut v);
            buf = serde_json::to_vec(&v).expect("json");
        } else if name == "foreman.db" {
            if let Some(p) = foreman_db {
                buf = fs::read(p).expect("db");
            }
        }
        zout.start_file(name.as_str(), o).expect("start");
        std::io::Write::write_all(&mut zout, &buf).expect("write");
    }
    zout.finish().expect("finish");
}

#[tokio::test]
async fn refuses_backups_from_a_newer_foreman_or_spoolman() {
    let l = live().await;
    let e = backup::create(&l.sources(), &opts(&l.out, false, None))
        .await
        .expect("backup");
    let src = PathBuf::from(&e.path);
    let newer = foreman_core::db::SCHEMA_VERSION + 1;

    // Manifest says newer Foreman.
    let f1 = l.out.join("t1.zip");
    tamper(&src, &f1, |v| v["foreman_schema"] = json!(newer), None);
    let err = backup::stage(&l.data, &f1, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("newer Foreman") && err.contains("Update Foreman"),
        "{err}"
    );

    // Manifest lies, but the database itself is newer: still refused.
    let newer_db = l.out.join("newer.db");
    fs::copy(l.data.join("foreman.db"), &newer_db).expect("copy");
    {
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", newer_db.display()))
            .await
            .expect("open");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "PRAGMA user_version = {newer}"
        )))
        .execute(&pool)
        .await
        .expect("bump");
        pool.close().await;
    }
    let f2 = l.out.join("t2.zip");
    tamper(&src, &f2, |_| {}, Some(&newer_db));
    let err = backup::stage(&l.data, &f2, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("newer Foreman"), "{err}");

    // Spoolman schema newer than this build knows.
    let f3 = l.out.join("t3.zip");
    tamper(
        &src,
        &f3,
        |v| v["spoolman_revision"] = json!("abcdef123456"),
        None,
    );
    let err = backup::stage(&l.data, &f3, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("Spoolman database") && err.contains("abcdef123456"),
        "{err}"
    );

    // None of that touched the live data or left a staged restore behind.
    assert!(!l.data.join(backup::PENDING_DIR).exists());
    assert!(backup::apply_pending(&l.sources())
        .await
        .expect("none")
        .is_none());
    assert_eq!(printer_names(&l.data).await, vec!["HellV0".to_owned()]);
}

#[tokio::test]
async fn retention_keeps_the_newest_and_leaves_other_files_alone() {
    let l = live().await;
    let mut made = Vec::new();
    for _ in 0..5 {
        made.push(
            backup::create(&l.sources(), &opts(&l.out, false, None))
                .await
                .expect("backup")
                .file,
        );
    }
    fs::write(l.out.join("notes.txt"), b"mine").expect("other file");
    fs::write(l.out.join("foreman-pre-restore-20200101-000000Z.zip"), b"x").expect("snap");
    let removed = backup::prune(&l.out, 3);
    assert_eq!(removed.len(), 2, "{removed:?}");
    // The two oldest went, with their sidecars.
    for f in &made[..2] {
        assert!(removed.contains(f), "{f} should be pruned: {removed:?}");
        assert!(!l.out.join(f).exists());
        let side = f.trim_end_matches(".zip").to_owned() + ".manifest.json";
        assert!(!l.out.join(side).exists());
    }
    for f in &made[2..] {
        assert!(l.out.join(f).exists(), "{f} should be kept");
    }
    assert!(l.out.join("notes.txt").exists());
    assert!(l
        .out
        .join("foreman-pre-restore-20200101-000000Z.zip")
        .exists());
    assert_eq!(backup::list(&l.out).len(), 4); // 3 kept + the pre-restore file
}

async fn attention_kinds(app: &axum::Router, auth: &str) -> Vec<String> {
    let (_, v) = call(app, "GET", "/api/attention", auth, None).await;
    v.as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|i| i["kind"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn failed_backup_raises_attention_and_a_good_one_clears_it() {
    let (dir, app, _state, auth) = setup().await;
    // A folder that cannot be created: its parent is a regular file.
    fs::write(dir.path().join("blocker"), b"file").expect("blocker");
    let bad = dir.path().join("blocker/backups");
    let settings = |d: &Path| {
        json!({ "settings": {
            "dir": d.to_string_lossy(), "schedule": "daily", "time": "03:00", "weekday": 7,
            "retention": 2, "include_library": false, "encrypt": false
        }})
    };
    let (s, v) = call(
        &app,
        "PUT",
        "/api/backup/settings",
        &auth,
        Some(settings(&bad)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["next_run"].is_string(), "{v}");

    let (s, v) = call(
        &app,
        "POST",
        "/api/backup/check-dir",
        &auth,
        Some(json!({"dir": bad})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["ok"], json!(false), "{v}");

    let (s, v) = call(&app, "POST", "/api/backup/run", &auth, Some(json!({}))).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert!(attention_kinds(&app, &auth)
        .await
        .contains(&"backup_failed".to_owned()));
    let (_, v) = call(&app, "GET", "/api/backup", &auth, None).await;
    assert_eq!(v["last"]["ok"], json!(false), "{v}");

    let good = dir.path().join("good");
    let (s, _) = call(
        &app,
        "PUT",
        "/api/backup/settings",
        &auth,
        Some(settings(&good)),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    for _ in 0..3 {
        let (s, v) = call(&app, "POST", "/api/backup/run", &auth, Some(json!({}))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    assert!(!attention_kinds(&app, &auth)
        .await
        .contains(&"backup_failed".to_owned()));
    let (_, v) = call(&app, "GET", "/api/backup", &auth, None).await;
    assert_eq!(
        v["backups"].as_array().map(Vec::len),
        Some(2),
        "retention 2: {v}"
    );
    assert_eq!(v["last"]["ok"], json!(true));
    assert_eq!(v["last"]["pruned"].as_array().map(Vec::len), Some(1));

    // Inspect + restore through the API (stages; the test build does not restart).
    let path = v["backups"][0]["path"].as_str().expect("path").to_owned();
    let (s, v) = call(
        &app,
        "POST",
        "/api/backup/inspect",
        &auth,
        Some(json!({"path": path})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["problem"].is_null(), "{v}");
    assert_eq!(
        v["manifest"]["counts"]["printers"],
        v["current_counts"]["printers"]
    );
    let (s, v) = call(
        &app,
        "POST",
        "/api/backup/restore",
        &auth,
        Some(json!({"path": path})),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    assert!(dir
        .path()
        .join(backup::PENDING_DIR)
        .join("files/foreman.db")
        .is_file());

    // Encryption needs a passphrase.
    let mut enc = settings(&good);
    enc["settings"]["encrypt"] = json!(true);
    let (s, _) = call(
        &app,
        "PUT",
        "/api/backup/settings",
        &auth,
        Some(enc.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    enc["passphrase"] = json!("a long passphrase");
    let (s, v) = call(&app, "PUT", "/api/backup/settings", &auth, Some(enc)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["has_passphrase"], json!(true));
    let (s, v) = call(&app, "POST", "/api/backup/run", &auth, Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(
        v["file"].as_str().is_some_and(|f| f.ends_with(".zip.age")),
        "{v}"
    );
}

#[tokio::test]
async fn spool_csv_and_spoolman_json_exports() {
    let (_d, app, state, auth) = setup().await;
    let sm = state.spoolman.get().expect("sm");
    add_spool(sm, "PLA", "1d1d22", 800.0).await;
    add_spool(sm, "PETG", "ff0000", 250.5).await;
    let (csv, n) = crate::exports::spools_csv(sm).await.expect("csv");
    assert_eq!(n, 2);
    let text = String::from_utf8(csv).expect("utf8");
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("spool_id,vendor,filament,material,colour,remaining_g"));
    assert_eq!(lines.len(), 3);
    assert!(
        lines.iter().any(|l| l.contains(",PETG,ff0000,250.5,")),
        "{text}"
    );
    let v = crate::exports::spoolman_json(sm).await.expect("json");
    assert_eq!(v["spools"].as_array().map(Vec::len), Some(2));
    assert_eq!(v["filaments"].as_array().map(Vec::len), Some(2));
    assert!(v["spools"][0]["filament"]["material"].is_string());
    let (s, _) = call(&app, "GET", "/api/exports/spools.csv", &auth, None).await;
    assert_eq!(s, StatusCode::OK);
}
