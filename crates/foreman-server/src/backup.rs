//! Backup and restore (task 6.2, W14).
//!
//! A backup is one zip (optionally wrapped in an `age` passphrase file):
//!
//! ```text
//! manifest.json   what is inside, versions, counts
//! foreman.db      `VACUUM INTO` snapshot (consistent while the server writes)
//! spoolman.db     same, when the built-in Spoolman uses SQLite
//! secret.key      decrypts the SSH key and passwords stored in foreman.db
//! config.toml     if present in the data dir
//! custom.css      theme override, if present
//! fonts/…         theme fonts
//! themes/…        theme files, if present
//! library/…       only when asked for (can be large)
//! ```
//!
//! Next to each backup a `<name>.manifest.json` copy of the manifest (no secrets)
//! lets the list show dates and counts without the passphrase.
//!
//! Restore is two steps so nothing is swapped under open database handles:
//! [`stage`] checks and unpacks the backup into `<data_dir>/restore-pending/`, the
//! server restarts, and [`apply_pending`] (before any database is opened) saves the
//! current state as `restore-snapshots/foreman-pre-restore-….zip`, then moves the
//! staged files into place.

use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use age::secrecy::SecretString;
use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Utc};
use foreman_core::error::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use ts_rs::TS;

/// Archive layout version. Bump when the layout changes incompatibly.
pub const FORMAT: u32 = 1;
/// File name prefix of every backup Foreman writes (and prunes).
pub const PREFIX: &str = "foreman-backup-";
const PRE_RESTORE_PREFIX: &str = "foreman-pre-restore-";
pub const PENDING_DIR: &str = "restore-pending";
pub const SNAPSHOT_DIR: &str = "restore-snapshots";
const SIDECAR: &str = ".manifest.json";

/// Plain files at the top of the data dir that go into every backup when present.
const TOP_FILES: &[&str] = &["secret.key", "config.toml", "custom.css"];
/// Theme directories that go into every backup when present.
const THEME_DIRS: &[&str] = &["fonts", "themes"];

fn io_err(what: impl std::fmt::Display, e: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("{what}: {e}"))
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Where the live data is.
#[derive(Debug, Clone)]
pub struct Sources {
    pub data_dir: PathBuf,
    /// The built-in Spoolman's SQLite file; `None` when Spoolman is off or uses
    /// Postgres/MySQL (then it is not part of the backup).
    pub spoolman_db: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum BackupKind {
    Manual,
    Scheduled,
    PreRestore,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BackupCounts {
    #[ts(type = "number")]
    pub printers: i64,
    #[ts(type = "number")]
    pub jobs: i64,
    #[ts(type = "number")]
    pub projects: i64,
    #[ts(type = "number")]
    pub library_files: i64,
    #[ts(type = "number")]
    pub spools: i64,
    #[ts(type = "number")]
    pub filaments: i64,
    #[ts(type = "number")]
    pub vendors: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ManifestFile {
    pub path: String,
    #[ts(type = "number")]
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Manifest {
    pub format: u32,
    pub kind: BackupKind,
    /// RFC 3339, UTC.
    pub created_at: String,
    pub foreman_version: String,
    pub foreman_schema: u32,
    /// Alembic revision of `spoolman.db`; `None` when Spoolman is not included.
    pub spoolman_revision: Option<String>,
    pub host: String,
    pub encrypted: bool,
    pub library_included: bool,
    pub counts: BackupCounts,
    pub files: Vec<ManifestFile>,
    /// Sum of the file sizes before compression.
    #[ts(type = "number")]
    pub total_bytes: u64,
}

pub struct BackupOptions {
    pub dest_dir: PathBuf,
    pub include_library: bool,
    pub passphrase: Option<String>,
    pub kind: BackupKind,
}

/// One backup file in a folder.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BackupEntry {
    pub file: String,
    pub path: String,
    #[ts(type = "number")]
    pub size: u64,
    pub encrypted: bool,
    /// From the sidecar (or the zip itself when unencrypted).
    pub manifest: Option<Manifest>,
}

// ---------------------------------------------------------------------------
// Creating a backup
// ---------------------------------------------------------------------------

/// Copy a live SQLite database to `dest` as one consistent snapshot.
///
/// `VACUUM INTO` runs inside a read transaction on its own read-only connection:
/// writers keep going (WAL), and the copy is exactly the database as of the moment
/// the transaction started. Never a raw copy of the live file.
pub async fn snapshot_sqlite(src: &Path, dest: &Path) -> Result<(), AppError> {
    let opts = SqliteConnectOptions::new()
        .filename(src)
        .read_only(true)
        .create_if_missing(false);
    let pool = SqlitePool::connect_with(opts)
        .await
        .map_err(|e| io_err(format!("open {}", src.display()), e))?;
    let res = sqlx::query("VACUUM INTO ?")
        .bind(dest.to_string_lossy().into_owned())
        .execute(&pool)
        .await;
    pool.close().await;
    res.map_err(|e| io_err(format!("snapshot {}", src.display()), e))?;
    Ok(())
}

async fn open_ro(path: &Path) -> Result<SqlitePool, AppError> {
    SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .create_if_missing(false),
    )
    .await
    .map_err(|e| io_err(format!("open {}", path.display()), e))
}

async fn count(pool: &SqlitePool, table: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

/// `PRAGMA user_version` and table counts of a Foreman DB file.
pub(crate) async fn foreman_facts(path: &Path) -> Result<(u32, BackupCounts), AppError> {
    let pool = open_ro(path).await?;
    let schema = foreman_core::db::schema_version(&pool).await;
    let counts = BackupCounts {
        printers: count(&pool, "printer").await,
        jobs: count(&pool, "job").await,
        projects: count(&pool, "project").await,
        library_files: sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM library_entry WHERE kind = 'file' AND deleted_at IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap_or(0),
        ..Default::default()
    };
    let check: Result<String, _> = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(&pool)
        .await;
    pool.close().await;
    match check {
        Ok(ok) if ok == "ok" => {}
        Ok(other) => return Err(AppError::Internal(format!("foreman.db check: {other}"))),
        Err(e) => return Err(io_err("foreman.db check", e)),
    }
    Ok((schema?, counts))
}

/// Alembic revision + counts of a Spoolman DB file.
pub(crate) async fn spoolman_facts(
    path: &Path,
    counts: &mut BackupCounts,
) -> Result<String, AppError> {
    let pool = open_ro(path).await?;
    let rev: Result<String, _> = sqlx::query_scalar("SELECT version_num FROM alembic_version")
        .fetch_one(&pool)
        .await;
    counts.spools = count(&pool, "spool").await;
    counts.filaments = count(&pool, "filament").await;
    counts.vendors = count(&pool, "vendor").await;
    pool.close().await;
    rev.map_err(|e| io_err("spoolman.db has no alembic_version", e))
}

/// Files under `dir` (recursive, regular files only), as (absolute, relative with `/`).
fn walk(dir: &Path, rel: &str, out: &mut Vec<(PathBuf, String)>) -> io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let ft = e.file_type()?;
        let r = format!("{rel}/{name}");
        if ft.is_dir() {
            walk(&e.path(), &r, out)?;
        } else if ft.is_file() {
            out.push((e.path(), r));
        }
    }
    Ok(())
}

/// Files and bytes in `<data_dir>/library` (what "include Library" adds).
pub fn library_size(data_dir: &Path) -> (u64, u64) {
    let mut files = Vec::new();
    let _ = walk(&data_dir.join("library"), "library", &mut files);
    let bytes = files
        .iter()
        .filter_map(|(p, _)| fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    (files.len() as u64, bytes)
}

/// Owner-only file mode (no-op on Windows, where the data dir's ACL applies).
pub fn restrict(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Create a file that is 0600 from the start (no window where it is world-readable).
fn create_private(path: &Path) -> io::Result<fs::File> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        o.mode(0o600);
    }
    o.open(path)
}

/// Removes a temp dir when dropped.
struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn host_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .or_else(|| {
            fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .unwrap_or_default()
}

fn unique_name(dir: &Path, prefix: &str, stamp: &str, ext: &str) -> String {
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{prefix}{stamp}{ext}")
        } else {
            format!("{prefix}{stamp}-{n}{ext}")
        };
        let base = name.trim_end_matches(ext);
        if !dir.join(&name).exists() && !dir.join(format!("{base}{SIDECAR}")).exists() {
            return name;
        }
        n += 1;
    }
}

/// Take a backup of `src` into `opts.dest_dir`. Returns the new entry.
///
/// For [`BackupKind::PreRestore`] the databases may be missing (restore into an
/// empty data dir); every other kind needs `foreman.db`.
pub async fn create(src: &Sources, opts: &BackupOptions) -> Result<BackupEntry, AppError> {
    let dest = &opts.dest_dir;
    fs::create_dir_all(dest).map_err(|e| io_err(format!("create {}", dest.display()), e))?;
    let work = TempDir(dest.join(format!(".foreman-backup-tmp-{}", uuid::Uuid::new_v4())));
    fs::create_dir_all(&work.0).map_err(|e| io_err("create temp folder", e))?;
    restrict_dir(&work.0);

    // 1. Database snapshots.
    let foreman_src = src.data_dir.join("foreman.db");
    let mut counts = BackupCounts::default();
    let mut schema = foreman_core::db::SCHEMA_VERSION;
    let mut entries: Vec<(PathBuf, String)> = Vec::new();
    if foreman_src.exists() {
        let snap = work.0.join("foreman.db");
        snapshot_sqlite(&foreman_src, &snap).await?;
        let (s, c) = foreman_facts(&snap).await?;
        counts = c;
        // A DB from before 6.2 has user_version 0: it is this build's schema or older.
        schema = if s == 0 {
            foreman_core::db::SCHEMA_VERSION
        } else {
            s
        };
        entries.push((snap, "foreman.db".into()));
    } else if opts.kind != BackupKind::PreRestore {
        return Err(AppError::NotFound(format!(
            "{} does not exist",
            foreman_src.display()
        )));
    }
    let mut spoolman_revision = None;
    if let Some(sm) = src.spoolman_db.as_ref().filter(|p| non_empty(p)) {
        let snap = work.0.join("spoolman.db");
        snapshot_sqlite(sm, &snap).await?;
        spoolman_revision = Some(spoolman_facts(&snap, &mut counts).await?);
        entries.push((snap, "spoolman.db".into()));
    }

    // 2. Plain files and folders (read as they are; none of them is written in place).
    for f in TOP_FILES {
        let p = src.data_dir.join(f);
        if p.is_file() {
            entries.push((p, (*f).to_owned()));
        }
    }
    let mut dirs: Vec<&str> = THEME_DIRS.to_vec();
    if opts.include_library {
        dirs.push("library");
    }
    for d in dirs {
        walk(&src.data_dir.join(d), d, &mut entries)
            .map_err(|e| io_err(format!("read {d}/"), e))?;
    }

    let created = Utc::now();
    let encrypted = opts.passphrase.as_deref().is_some_and(|p| !p.is_empty());
    let files: Vec<ManifestFile> = entries
        .iter()
        .map(|(p, r)| ManifestFile {
            path: r.clone(),
            size: fs::metadata(p).map(|m| m.len()).unwrap_or(0),
        })
        .collect();
    let manifest = Manifest {
        format: FORMAT,
        kind: opts.kind,
        created_at: created.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        foreman_version: env!("CARGO_PKG_VERSION").to_owned(),
        foreman_schema: schema,
        spoolman_revision,
        host: host_name(),
        encrypted,
        library_included: opts.include_library,
        total_bytes: files.iter().map(|f| f.size).sum(),
        counts,
        files,
    };

    // 3. Zip (blocking IO off the async runtime).
    let prefix = if opts.kind == BackupKind::PreRestore {
        PRE_RESTORE_PREFIX
    } else {
        PREFIX
    };
    let ext = if encrypted { ".zip.age" } else { ".zip" };
    let stamp = created.format("%Y%m%d-%H%M%SZ").to_string();
    let name = unique_name(dest, prefix, &stamp, ext);
    let final_path = dest.join(&name);
    let sidecar = dest.join(format!("{}{SIDECAR}", name.trim_end_matches(ext)));
    let m = manifest.clone();
    let pass = opts.passphrase.clone().filter(|p| !p.is_empty());
    let work_dir = work.0.clone();
    let fp = final_path.clone();
    tokio::task::spawn_blocking(move || write_archive(&work_dir, &entries, &m, pass, &fp))
        .await
        .map_err(|e| io_err("backup task", e))??;

    let side = serde_json::to_vec_pretty(&manifest).map_err(|e| io_err("manifest", e))?;
    let _ = fs::remove_file(&sidecar);
    fs::write(&sidecar, side).map_err(|e| io_err(format!("write {}", sidecar.display()), e))?;
    let size = fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
    Ok(BackupEntry {
        file: name,
        path: final_path.to_string_lossy().into_owned(),
        size,
        encrypted,
        manifest: Some(manifest),
    })
}

fn restrict_dir(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn non_empty(p: &Path) -> bool {
    fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false)
}

fn write_archive(
    work: &Path,
    entries: &[(PathBuf, String)],
    manifest: &Manifest,
    passphrase: Option<String>,
    final_path: &Path,
) -> Result<(), AppError> {
    let zip_path = work.join("archive.zip");
    {
        let file = create_private(&zip_path).map_err(|e| io_err("create archive", e))?;
        let mut zip = zip::ZipWriter::new(io::BufWriter::new(file));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .large_file(true)
            .unix_permissions(0o600);
        let m = serde_json::to_vec_pretty(manifest).map_err(|e| io_err("manifest", e))?;
        zip.start_file("manifest.json", opts)
            .map_err(|e| io_err("zip", e))?;
        zip.write_all(&m).map_err(|e| io_err("zip", e))?;
        for (abs, rel) in entries {
            zip.start_file(rel.as_str(), opts)
                .map_err(|e| io_err("zip", e))?;
            let mut f = fs::File::open(abs).map_err(|e| io_err(format!("read {rel}"), e))?;
            io::copy(&mut f, &mut zip).map_err(|e| io_err(format!("zip {rel}"), e))?;
        }
        let w = zip.finish().map_err(|e| io_err("zip", e))?;
        w.into_inner()
            .map_err(|e| io_err("zip flush", e))?
            .sync_all()
            .map_err(|e| io_err("zip sync", e))?;
    }
    let partial = final_path.with_extension("partial");
    let _ = fs::remove_file(&partial);
    match passphrase {
        Some(p) => {
            let out = create_private(&partial).map_err(|e| io_err("create backup", e))?;
            let enc = age::Encryptor::with_user_passphrase(SecretString::from(p));
            let mut w = enc
                .wrap_output(io::BufWriter::new(out))
                .map_err(|e| io_err("encrypt", e))?;
            let mut r = fs::File::open(&zip_path).map_err(|e| io_err("read archive", e))?;
            io::copy(&mut r, &mut w).map_err(|e| io_err("encrypt", e))?;
            w.finish()
                .and_then(|b| b.into_inner().map_err(|e| e.into_error()))
                .and_then(|f| f.sync_all())
                .map_err(|e| io_err("encrypt", e))?;
        }
        None => {
            // Same folder → rename; else copy (temp is inside the destination, so rare).
            if fs::rename(&zip_path, &partial).is_err() {
                let mut out = create_private(&partial).map_err(|e| io_err("create backup", e))?;
                let mut r = fs::File::open(&zip_path).map_err(|e| io_err("read archive", e))?;
                io::copy(&mut r, &mut out).map_err(|e| io_err("copy archive", e))?;
            }
        }
    }
    restrict(&partial).map_err(|e| io_err("chmod backup", e))?;
    fs::rename(&partial, final_path)
        .map_err(|e| io_err(format!("write {}", final_path.display()), e))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Listing, retention
// ---------------------------------------------------------------------------

fn is_backup_name(name: &str) -> bool {
    (name.starts_with(PREFIX) || name.starts_with(PRE_RESTORE_PREFIX))
        && (name.ends_with(".zip") || name.ends_with(".zip.age"))
}

fn sidecar_for(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let base = name
        .strip_suffix(".zip.age")
        .or_else(|| name.strip_suffix(".zip"))
        .unwrap_or(&name);
    path.with_file_name(format!("{base}{SIDECAR}"))
}

/// Backups in `dir`, newest first.
pub fn list(dir: &Path) -> Vec<BackupEntry> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<BackupEntry> = rd
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let file = e.file_name().to_string_lossy().into_owned();
            if !is_backup_name(&file) {
                return None;
            }
            let path = e.path();
            let encrypted = file.ends_with(".age");
            let manifest = fs::read(sidecar_for(&path))
                .ok()
                .and_then(|b| serde_json::from_slice::<Manifest>(&b).ok())
                .or_else(|| {
                    (!encrypted)
                        .then(|| read_manifest_zip(&path).ok())
                        .flatten()
                });
            Some(BackupEntry {
                size: e.metadata().map(|m| m.len()).unwrap_or(0),
                path: path.to_string_lossy().into_owned(),
                file,
                encrypted,
                manifest,
            })
        })
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(age_key(e)));
    out
}

/// Newest-first key: the manifest time (survives copying between machines), then the
/// file's mtime for backups taken within the same second.
fn age_key(e: &BackupEntry) -> (String, std::time::SystemTime) {
    let created = e
        .manifest
        .as_ref()
        .map(|m| m.created_at.clone())
        .unwrap_or_default();
    let mtime = fs::metadata(&e.path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::UNIX_EPOCH);
    (created, mtime)
}

/// Keep the newest `keep` regular backups in `dir` (pre-restore snapshots are never
/// pruned here). Returns the names removed.
pub fn prune(dir: &Path, keep: usize) -> Vec<String> {
    let mut regular: Vec<BackupEntry> = list(dir)
        .into_iter()
        .filter(|e| e.file.starts_with(PREFIX))
        .collect();
    regular.sort_by_key(|e| std::cmp::Reverse(age_key(e)));
    let mut removed = Vec::new();
    for e in regular.into_iter().skip(keep.max(1)) {
        let p = PathBuf::from(&e.path);
        if fs::remove_file(&p).is_ok() {
            let _ = fs::remove_file(sidecar_for(&p));
            removed.push(e.file);
        }
    }
    removed
}

// ---------------------------------------------------------------------------
// Reading a backup, compatibility
// ---------------------------------------------------------------------------

fn read_manifest_zip(path: &Path) -> Result<Manifest, AppError> {
    let f = fs::File::open(path).map_err(|e| io_err(format!("open {}", path.display()), e))?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(f))
        .map_err(|e| AppError::InvalidInput(format!("not a Foreman backup ({e})")))?;
    let mut m = zip
        .by_name("manifest.json")
        .map_err(|_| AppError::InvalidInput("not a Foreman backup (no manifest.json)".into()))?;
    let mut s = String::new();
    m.read_to_string(&mut s)
        .map_err(|e| io_err("read manifest", e))?;
    serde_json::from_str(&s).map_err(|e| AppError::InvalidInput(format!("bad manifest.json: {e}")))
}

/// A plain zip path for `file`: the file itself, or `file` decrypted into `work`.
fn open_zip(file: &Path, passphrase: Option<&str>, work: &Path) -> Result<PathBuf, AppError> {
    let name = file.to_string_lossy();
    if !name.ends_with(".age") {
        return Ok(file.to_owned());
    }
    let Some(pass) = passphrase.filter(|p| !p.is_empty()) else {
        return Err(AppError::InvalidInput(
            "This backup is encrypted: enter its passphrase.".into(),
        ));
    };
    let f = fs::File::open(file).map_err(|e| io_err(format!("open {}", file.display()), e))?;
    let dec = age::Decryptor::new_buffered(io::BufReader::new(f))
        .map_err(|e| AppError::InvalidInput(format!("not an age file: {e}")))?;
    let id = age::scrypt::Identity::new(SecretString::from(pass.to_owned()));
    let mut r = dec
        .decrypt(std::iter::once(&id as &dyn age::Identity))
        .map_err(|e| match e {
            age::DecryptError::NoMatchingKeys | age::DecryptError::DecryptionFailed => {
                AppError::InvalidInput("Wrong passphrase for this backup.".into())
            }
            e => AppError::InvalidInput(format!("cannot decrypt: {e}")),
        })?;
    let out = work.join("archive.zip");
    let mut w = create_private(&out).map_err(|e| io_err("decrypt", e))?;
    io::copy(&mut r, &mut w).map_err(|e| {
        AppError::InvalidInput(format!(
            "decrypt failed (file damaged or wrong passphrase): {e}"
        ))
    })?;
    Ok(out)
}

/// What this Foreman runs, for the compatibility check and the restore screen.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Current {
    pub foreman_version: String,
    pub foreman_schema: u32,
    pub spoolman_revision: String,
}

pub fn current() -> Current {
    Current {
        foreman_version: env!("CARGO_PKG_VERSION").to_owned(),
        foreman_schema: foreman_core::db::SCHEMA_VERSION,
        spoolman_revision: spoolman_rs::db::migrations::HEAD.to_owned(),
    }
}

/// `Err(message)` when this Foreman cannot restore a backup with these versions.
pub fn check_compatible(
    format: u32,
    foreman_version: &str,
    schema: u32,
    spoolman_revision: Option<&str>,
) -> Result<(), String> {
    let cur = current();
    if format > FORMAT {
        return Err(format!(
            "This backup was made by a newer Foreman ({foreman_version}) in a format this Foreman \
             ({}) cannot read. Update Foreman, then restore.",
            cur.foreman_version
        ));
    }
    if schema > cur.foreman_schema {
        return Err(format!(
            "This backup is from a newer Foreman: {foreman_version}, database schema {schema}. This \
             Foreman is {} with schema {}. Update Foreman, then restore.",
            cur.foreman_version, cur.foreman_schema
        ));
    }
    if let Some(rev) = spoolman_revision {
        let known = spoolman_rs::db::migrations::CHAIN
            .iter()
            .any(|m| m.revision == rev);
        if !known {
            return Err(format!(
                "This backup's Spoolman database is at revision {rev}, which is newer than this \
                 Foreman knows (it goes up to {}). Update Foreman, then restore.",
                cur.spoolman_revision
            ));
        }
    }
    Ok(())
}

fn check_manifest(m: &Manifest) -> Result<(), AppError> {
    check_compatible(
        m.format,
        &m.foreman_version,
        m.foreman_schema,
        m.spoolman_revision.as_deref(),
    )
    .map_err(AppError::InvalidInput)
}

/// Accept only backup-looking paths (the admin may name any file on the server).
fn check_backup_path(file: &Path) -> Result<(), AppError> {
    let name = file.to_string_lossy();
    if !(name.ends_with(".zip") || name.ends_with(".zip.age")) {
        return Err(AppError::InvalidInput(
            "Pick a Foreman backup (.zip or .zip.age).".into(),
        ));
    }
    if !file.is_file() {
        return Err(AppError::NotFound(format!(
            "{} does not exist",
            file.display()
        )));
    }
    Ok(())
}

/// Read and verify a backup's manifest (decrypting when needed).
pub async fn inspect(file: &Path, passphrase: Option<&str>) -> Result<Manifest, AppError> {
    check_backup_path(file)?;
    let file = file.to_owned();
    let pass = passphrase.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        let tmp = std::env::temp_dir().join(format!("foreman-inspect-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&tmp).map_err(|e| io_err("temp", e))?;
        restrict_dir(&tmp);
        let _guard = TempDir(tmp.clone());
        let zip = open_zip(&file, pass.as_deref(), &tmp)?;
        read_manifest_zip(&zip)
    })
    .await
    .map_err(|e| io_err("inspect task", e))?
}

// ---------------------------------------------------------------------------
// Restore
// ---------------------------------------------------------------------------

/// Check `file` and unpack it into `<data_dir>/restore-pending/`. The live data is
/// not touched; [`apply_pending`] does that on the next start.
pub async fn stage(
    data_dir: &Path,
    file: &Path,
    passphrase: Option<&str>,
) -> Result<Manifest, AppError> {
    check_backup_path(file)?;
    let partial = data_dir.join(format!("{PENDING_DIR}.partial"));
    let pending = data_dir.join(PENDING_DIR);
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| io_err("create restore folder", e))?;
    restrict_dir(&partial);
    let _guard = TempDir(partial.clone());

    let (file2, pass, part) = (
        file.to_owned(),
        passphrase.map(str::to_owned),
        partial.clone(),
    );
    let manifest = tokio::task::spawn_blocking(move || -> Result<Manifest, AppError> {
        let zip_path = open_zip(&file2, pass.as_deref(), &part)?;
        let manifest = read_manifest_zip(&zip_path)?;
        check_manifest(&manifest)?;
        let files = part.join("files");
        let f = fs::File::open(&zip_path).map_err(|e| io_err("open backup", e))?;
        let mut zip = zip::ZipArchive::new(io::BufReader::new(f))
            .map_err(|e| AppError::InvalidInput(format!("damaged backup: {e}")))?;
        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|e| AppError::InvalidInput(format!("damaged backup: {e}")))?;
            // Zip-slip guard: only names that stay inside the folder.
            let Some(rel) = entry.enclosed_name() else {
                return Err(AppError::InvalidInput(format!(
                    "backup contains an unsafe path: {:?}",
                    entry.name().unwrap_or_default()
                )));
            };
            if entry.is_dir() {
                continue;
            }
            let out = files.join(&rel);
            if let Some(p) = out.parent() {
                fs::create_dir_all(p).map_err(|e| io_err("unpack", e))?;
            }
            let mut w = create_private(&out).map_err(|e| io_err("unpack", e))?;
            io::copy(&mut entry, &mut w)
                .map_err(|e| AppError::InvalidInput(format!("damaged backup ({rel:?}): {e}")))?;
        }
        if !files.join("foreman.db").is_file() {
            return Err(AppError::InvalidInput("backup has no foreman.db".into()));
        }
        let _ = fs::remove_file(part.join("archive.zip"));
        Ok(manifest)
    })
    .await
    .map_err(|e| io_err("restore task", e))??;

    // Trust the databases over the manifest.
    let (schema, _) = foreman_facts(&partial.join("files/foreman.db")).await?;
    let sm = partial.join("files/spoolman.db");
    let rev = if sm.is_file() {
        let mut c = BackupCounts::default();
        Some(spoolman_facts(&sm, &mut c).await?)
    } else {
        None
    };
    check_compatible(
        manifest.format,
        &manifest.foreman_version,
        schema,
        rev.as_deref(),
    )
    .map_err(AppError::InvalidInput)?;

    let _ = fs::remove_dir_all(&pending);
    fs::rename(&partial, &pending).map_err(|e| io_err("stage restore", e))?;
    Ok(manifest)
}

/// Result of [`apply_pending`].
#[derive(Debug, Clone)]
pub struct Applied {
    pub manifest: Manifest,
    /// The state before the restore, as a backup (restore it to undo).
    pub snapshot: Option<PathBuf>,
}

fn remove_db(path: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        let _ = fs::remove_file(PathBuf::from(p));
    }
}

fn move_into(from: &Path, to: &Path) -> Result<(), AppError> {
    if let Some(p) = to.parent() {
        fs::create_dir_all(p).map_err(|e| io_err(format!("create {}", p.display()), e))?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    // Different filesystem (e.g. Spoolman's DB outside the data dir): copy.
    if from.is_dir() {
        let mut files = Vec::new();
        walk(from, "", &mut files).map_err(|e| io_err("read restore", e))?;
        for (abs, rel) in files {
            move_into(&abs, &to.join(rel.trim_start_matches('/')))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map_err(|e| io_err(format!("write {}", to.display()), e))?;
        Ok(())
    }
}

/// On start, before any database is opened: if a restore is staged, save the current
/// state as a pre-restore backup, then move the staged files into place.
///
/// Returns `Ok(None)` when nothing is staged. On an error nothing has been replaced
/// unless the error says so; the staged folder is removed either way so a bad backup
/// cannot loop.
pub async fn apply_pending(src: &Sources) -> Result<Option<Applied>, AppError> {
    let pending = src.data_dir.join(PENDING_DIR);
    let staged = pending.join("files");
    let mf = staged.join("manifest.json");
    if !mf.is_file() {
        let _ = fs::remove_dir_all(src.data_dir.join(format!("{PENDING_DIR}.partial")));
        return Ok(None);
    }
    let _cleanup = TempDir(pending.clone());
    let manifest: Manifest = fs::read(&mf)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| AppError::InvalidInput("staged restore has a bad manifest".into()))?;
    check_manifest(&manifest)?;

    // 1. Current state → pre-restore backup (nothing is open yet, so it is exact).
    let has_current = src.data_dir.join("foreman.db").exists()
        || src.spoolman_db.as_deref().is_some_and(non_empty);
    let snapshot = if has_current {
        let e = create(
            src,
            &BackupOptions {
                dest_dir: src.data_dir.join(SNAPSHOT_DIR),
                include_library: manifest.library_included,
                passphrase: None,
                kind: BackupKind::PreRestore,
            },
        )
        .await
        .map_err(|e| {
            AppError::Internal(format!(
                "restore cancelled: could not save the current state first: {e}"
            ))
        })?;
        Some(PathBuf::from(e.path))
    } else {
        None
    };

    // 2. Swap. From here on the data dir holds the backup's state.
    let dd = &src.data_dir;
    remove_db(&dd.join("foreman.db"));
    move_into(&staged.join("foreman.db"), &dd.join("foreman.db"))?;
    if let Some(sm) = &src.spoolman_db {
        if staged.join("spoolman.db").is_file() {
            remove_db(sm);
            move_into(&staged.join("spoolman.db"), sm)?;
        }
    } else if staged.join("spoolman.db").is_file() {
        tracing::warn!(
            "backup has spoolman.db but this server's Spoolman is off or not SQLite; not restored"
        );
    }
    for f in TOP_FILES {
        let target = dd.join(f);
        let _ = fs::remove_file(&target);
        if staged.join(f).is_file() {
            move_into(&staged.join(f), &target)?;
        }
    }
    let _ = restrict(&dd.join("secret.key"));
    let mut dirs: Vec<&str> = THEME_DIRS.to_vec();
    if manifest.library_included {
        dirs.push("library");
    }
    for d in dirs {
        let target = dd.join(d);
        let _ = fs::remove_dir_all(&target);
        if staged.join(d).is_dir() {
            move_into(&staged.join(d), &target)?;
        }
    }
    if let Err(e) = reanchor_schedule(&dd.join("foreman.db")).await {
        tracing::warn!("re-anchor backup schedule after restore: {e}");
    }
    Ok(Some(Applied { manifest, snapshot }))
}

/// A restored database carries the schedule and last run from backup time, so the
/// latest slot looks missed. Count the schedule from now: no catch-up backup (and
/// prune) the moment a restored server starts, whichever way it was restored.
async fn reanchor_schedule(db: &Path) -> Result<(), AppError> {
    let pool = SqlitePool::connect_with(SqliteConnectOptions::new().filename(db))
        .await
        .map_err(|e| io_err("open restored db", e))?;
    let res = async {
        let raw: Option<String> = sqlx::query_scalar("SELECT value FROM setting WHERE key = ?")
            .bind(SETTING_KEY)
            .fetch_optional(&pool)
            .await
            .map_err(|e| io_err("read backup settings", e))?;
        let Some(mut settings) = raw.and_then(|v| serde_json::from_str::<BackupSettings>(&v).ok())
        else {
            return Ok(());
        };
        settings.schedule_since = Some(Utc::now().to_rfc3339());
        let json = serde_json::to_string(&settings).map_err(|e| io_err("settings", e))?;
        sqlx::query("UPDATE setting SET value = ? WHERE key = ?")
            .bind(json)
            .bind(SETTING_KEY)
            .execute(&pool)
            .await
            .map_err(|e| io_err("write backup settings", e))?;
        Ok(())
    }
    .await;
    pool.close().await;
    res
}

/// Replace this process with a fresh copy of itself (same arguments and environment).
/// Unix: `exec` (same PID; listening sockets are close-on-exec). Windows: spawn, exit.
pub fn restart_process() -> ! {
    let exe = std::env::current_exe();
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match exe {
        Ok(exe) => {
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt as _;
                let err = std::process::Command::new(&exe).args(&args).exec();
                tracing::error!("restart failed: {err}");
            }
            #[cfg(not(unix))]
            {
                if let Err(err) = std::process::Command::new(&exe).args(&args).spawn() {
                    tracing::error!("restart failed: {err}");
                }
            }
        }
        Err(e) => tracing::error!("restart failed: {e}"),
    }
    std::process::exit(0)
}

// ---------------------------------------------------------------------------
// Settings + schedule
// ---------------------------------------------------------------------------

pub const SETTING_KEY: &str = "backup.settings";
pub const SETTING_PASSPHRASE: &str = "backup.passphrase";
pub const SETTING_LAST: &str = "backup.last";
pub const ATTENTION_KIND: &str = "backup_failed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Schedule {
    Off,
    Daily,
    Weekly,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, rename = "BackupSettingsDto")]
pub struct BackupSettings {
    /// Absolute folder on this machine (a share you mounted yourself works too).
    pub dir: String,
    pub schedule: Schedule,
    /// Local time `HH:MM`.
    pub time: String,
    /// 1 = Monday … 7 = Sunday (weekly only).
    pub weekday: u8,
    /// How many regular backups to keep in the folder (at least 1).
    pub retention: u32,
    pub include_library: bool,
    pub encrypt: bool,
    /// When the schedule was last turned on/changed (RFC 3339); slots before it don't count.
    #[serde(default)]
    pub schedule_since: Option<String>,
}

pub fn default_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("foreman-backups")
}

impl Default for BackupSettings {
    fn default() -> Self {
        Self {
            dir: default_dir().to_string_lossy().into_owned(),
            schedule: Schedule::Off,
            time: "03:00".into(),
            weekday: 7,
            retention: 7,
            include_library: false,
            encrypt: false,
            schedule_since: None,
        }
    }
}

impl BackupSettings {
    pub fn validate(&self) -> Result<(), AppError> {
        if !Path::new(&self.dir).is_absolute() {
            return Err(AppError::InvalidInput(
                "Backup folder must be a full path (e.g. /home/you/foreman-backups or D:\\Backups).".into(),
            ));
        }
        if parse_time(&self.time).is_none() {
            return Err(AppError::InvalidInput("Time must be HH:MM.".into()));
        }
        if !(1..=7).contains(&self.weekday) {
            return Err(AppError::InvalidInput("Weekday must be 1–7.".into()));
        }
        if !(1..=365).contains(&self.retention) {
            return Err(AppError::InvalidInput(
                "Keep between 1 and 365 backups.".into(),
            ));
        }
        Ok(())
    }
}

fn parse_time(t: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(t, "%H:%M").ok()
}

/// The latest scheduled moment at or before `now` (local time).
pub fn last_slot<Tz: TimeZone>(s: &BackupSettings, now: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let t = parse_time(&s.time)?;
    let tz = now.timezone();
    let at = |d: chrono::NaiveDate| tz.from_local_datetime(&d.and_time(t)).earliest();
    let today = now.date_naive();
    match s.schedule {
        Schedule::Off => None,
        Schedule::Daily => {
            let a = at(today)?;
            if a <= *now {
                Some(a)
            } else {
                at(today - Duration::days(1))
            }
        }
        Schedule::Weekly => {
            let want = u32::from(s.weekday);
            for back in 0..8 {
                let d = today - Duration::days(back);
                if d.weekday().number_from_monday() == want {
                    let a = at(d)?;
                    if a <= *now {
                        return Some(a);
                    }
                }
            }
            None
        }
    }
}

/// The next scheduled moment after `now`.
pub fn next_slot<Tz: TimeZone>(s: &BackupSettings, now: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let step = match s.schedule {
        Schedule::Off => return None,
        Schedule::Daily => 1,
        Schedule::Weekly => 7,
    };
    let last = last_slot(s, now)?;
    let t = parse_time(&s.time)?;
    let d = last.date_naive() + Duration::days(step);
    now.timezone()
        .from_local_datetime(&d.and_time(t))
        .earliest()
}

/// True when a scheduled backup is due: the latest slot is after both the last
/// attempt and the moment the schedule was set.
pub fn is_due(
    s: &BackupSettings,
    last_attempt: Option<DateTime<Utc>>,
    now: DateTime<Local>,
) -> bool {
    let Some(slot) = last_slot(s, &now) else {
        return false;
    };
    let slot = slot.with_timezone(&Utc);
    let since = s
        .schedule_since
        .as_deref()
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|d| d.with_timezone(&Utc));
    since.is_none_or(|t| slot > t) && last_attempt.is_none_or(|t| slot > t)
}

/// Outcome of the last backup run.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LastRun {
    pub at: String,
    pub kind: BackupKind,
    pub ok: bool,
    pub file: Option<String>,
    #[ts(type = "number | null")]
    pub size: Option<u64>,
    pub error: Option<String>,
    pub pruned: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(schedule: Schedule, time: &str, weekday: u8) -> BackupSettings {
        BackupSettings {
            schedule,
            time: time.into(),
            weekday,
            ..Default::default()
        }
    }

    #[test]
    fn daily_and_weekly_slots() {
        let now = Utc.with_ymd_and_hms(2026, 11, 12, 10, 0, 0).unwrap(); // Thursday
        let d = s(Schedule::Daily, "03:00", 1);
        assert_eq!(
            last_slot(&d, &now).unwrap(),
            Utc.with_ymd_and_hms(2026, 11, 12, 3, 0, 0).unwrap()
        );
        assert_eq!(
            next_slot(&d, &now).unwrap(),
            Utc.with_ymd_and_hms(2026, 11, 13, 3, 0, 0).unwrap()
        );
        let late = s(Schedule::Daily, "23:30", 1);
        assert_eq!(
            last_slot(&late, &now).unwrap(),
            Utc.with_ymd_and_hms(2026, 11, 11, 23, 30, 0).unwrap()
        );
        let w = s(Schedule::Weekly, "03:00", 7); // Sunday
        assert_eq!(
            last_slot(&w, &now).unwrap(),
            Utc.with_ymd_and_hms(2026, 11, 8, 3, 0, 0).unwrap()
        );
        assert_eq!(
            next_slot(&w, &now).unwrap(),
            Utc.with_ymd_and_hms(2026, 11, 15, 3, 0, 0).unwrap()
        );
        assert!(last_slot(&s(Schedule::Off, "03:00", 1), &now).is_none());
    }

    #[test]
    fn compat_messages() {
        assert!(check_compatible(1, "0.1.0", 1, Some("fe4970567bb3")).is_ok());
        let e =
            check_compatible(1, "9.0.0", foreman_core::db::SCHEMA_VERSION + 1, None).unwrap_err();
        assert!(e.contains("newer Foreman"), "{e}");
        let e = check_compatible(1, "0.1.0", 1, Some("ffffffffffff")).unwrap_err();
        assert!(e.contains("Spoolman database"), "{e}");
        assert!(check_compatible(FORMAT + 1, "9", 1, None).is_err());
    }
}
