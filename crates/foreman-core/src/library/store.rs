//! Library file store: files under `<data_dir>/library`, indexed in `library_entry`,
//! parsed facts in `file_meta`, thumbnails in `<data_dir>/library-cache/thumbs`.
//!
//! Paths are relative to the library root and `/`-separated; `""` is the root itself.
//! Deleting moves an entry (and everything under it) to `<data_dir>/library-trash/<trash_id>/`.

use std::{
    collections::HashMap,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use sha2::{Digest as _, Sha256};
use sqlx::{Row as _, SqlitePool};
use tracing::{info, warn};

use super::parse::{parse_path, Parsed};
use crate::{
    domain::{FileMeta, LibraryEntry, LibraryEntryKind, LibraryFileDetails, SlotKind},
    error::AppError,
};

/// What [`LibraryStore::rebuild_index`] changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RebuildReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

pub struct LibraryStore {
    root: PathBuf,
    trash: PathBuf,
    thumbs: PathBuf,
    tmp: PathBuf,
    pool: SqlitePool,
}

fn db(e: sqlx::Error) -> AppError {
    AppError::Database(e.to_string())
}

fn io(e: std::io::Error) -> AppError {
    AppError::Internal(e.to_string())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Validate one path component (file or folder name).
pub fn clean_name(name: &str) -> Result<String, AppError> {
    let n = name.trim();
    let bad = n.is_empty()
        || n == "."
        || n == ".."
        || n.len() > 255
        || n.starts_with(".foreman-")
        || n.chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c));
    if bad {
        return Err(AppError::InvalidInput(format!("invalid name: {name:?}")));
    }
    Ok(n.to_owned())
}

/// Normalise a relative folder/file path (`""` = root); rejects `..` and absolute paths.
pub fn clean_rel(path: &str) -> Result<String, AppError> {
    let parts: Result<Vec<String>, AppError> = path
        .split('/')
        .filter(|p| !p.is_empty())
        .map(clean_name)
        .collect();
    Ok(parts?.join("/"))
}

pub fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}

fn name_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(_, n)| n).unwrap_or(path)
}

fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// `part.gcode.3mf` → (`part`, `.gcode.3mf`).
fn split_ext(name: &str) -> (&str, &str) {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".gcode.3mf") {
        let i = name.len() - ".gcode.3mf".len();
        return (&name[..i], &name[i..]);
    }
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

fn move_path(from: &Path, to: &Path) -> Result<(), AppError> {
    if let Some(p) = to.parent() {
        fs::create_dir_all(p).map_err(io)?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    // Cross-device: copy then remove (files only; folders never leave the data dir).
    fs::copy(from, to).map_err(io)?;
    fs::remove_file(from).map_err(io)
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 << 10];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

fn mtime_secs(md: &fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `SELECT <entry columns> FROM library_entry e LEFT JOIN file_meta m … <rest>` as a literal.
macro_rules! select_entries {
    ($rest:literal) => {
        concat!(
            "SELECT e.id, e.path, e.kind, e.size, e.hash, e.added_at, e.starred, e.deleted_at, \
             m.format, m.thumb_file, m.print_time_secs, m.tools_json, m.slot_kind, m.slots_json \
             FROM library_entry e LEFT JOIN file_meta m ON m.entry_id = e.id ",
            $rest
        )
    };
}

fn entry_from_row(r: &sqlx::sqlite::SqliteRow) -> Result<LibraryEntry, AppError> {
    let id: i64 = r.try_get("id").map_err(db)?;
    let path: String = r.try_get("path").map_err(db)?;
    let kind: String = r.try_get("kind").map_err(db)?;
    let thumb: Option<String> = r.try_get("thumb_file").map_err(db)?;
    let hash: Option<String> = r.try_get("hash").map_err(db)?;
    let tools: Option<String> = r.try_get("tools_json").map_err(db)?;
    Ok(LibraryEntry {
        id,
        print_time_secs: r.try_get("print_time_secs").map_err(db)?,
        tools: tools
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default(),
        slot_kind: r
            .try_get::<Option<String>, _>("slot_kind")
            .map_err(db)?
            .map(|k| SlotKind::parse(&k)),
        slots: r
            .try_get::<Option<String>, _>("slots_json")
            .map_err(db)?
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default(),
        name: name_of(&path).to_owned(),
        kind: if kind == "folder" {
            LibraryEntryKind::Folder
        } else {
            LibraryEntryKind::File
        },
        size: r.try_get("size").map_err(db)?,
        added_at: r.try_get("added_at").map_err(db)?,
        starred: r.try_get::<i64, _>("starred").map_err(db)? != 0,
        deleted_at: r.try_get("deleted_at").map_err(db)?,
        format: r.try_get("format").map_err(db)?,
        // The hash prefix busts browser caches when a file is replaced.
        thumb_url: thumb.map(|_| {
            let v = hash
                .as_deref()
                .map(|h| &h[..h.len().min(12)])
                .unwrap_or("0");
            format!("/api/library/files/{id}/thumb?v={v}")
        }),
        hash,
        path,
    })
}

impl LibraryStore {
    pub fn new(data_dir: &Path, pool: SqlitePool) -> Result<Self, AppError> {
        let s = Self {
            root: data_dir.join("library"),
            trash: data_dir.join("library-trash"),
            thumbs: data_dir.join("library-cache").join("thumbs"),
            tmp: data_dir.join("library-cache").join("tmp"),
            pool,
        };
        for d in [&s.root, &s.trash, &s.thumbs, &s.tmp] {
            fs::create_dir_all(d).map_err(io)?;
        }
        Ok(s)
    }

    /// Where uploads are spooled before [`add_file`](Self::add_file) moves them in.
    pub fn tmp_dir(&self) -> &Path {
        &self.tmp
    }

    pub fn thumb_path(&self, id: i64) -> PathBuf {
        self.thumbs.join(format!("{id}.png"))
    }

    fn abs(&self, rel: &str) -> PathBuf {
        rel.split('/')
            .filter(|p| !p.is_empty())
            .fold(self.root.clone(), |acc, p| acc.join(p))
    }

    async fn live_id(&self, path: &str) -> Result<Option<i64>, AppError> {
        sqlx::query_scalar("SELECT id FROM library_entry WHERE path = ? AND deleted_at IS NULL")
            .bind(path)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)
    }

    async fn taken(&self, path: &str) -> Result<bool, AppError> {
        Ok(self.live_id(path).await?.is_some() || self.abs(path).exists())
    }

    /// `name` in `folder`, or `name (1)`, `name (2)`… if that is taken.
    async fn unique_path(&self, folder: &str, name: &str) -> Result<String, AppError> {
        let first = join(folder, name);
        if !self.taken(&first).await? {
            return Ok(first);
        }
        let (stem, ext) = split_ext(name);
        for i in 1..10_000 {
            let p = join(folder, &format!("{stem} ({i}){ext}"));
            if !self.taken(&p).await? {
                return Ok(p);
            }
        }
        Err(AppError::InvalidInput(format!("too many copies of {name}")))
    }

    pub async fn get(&self, id: i64) -> Result<LibraryEntry, AppError> {
        let row = sqlx::query(select_entries!("WHERE e.id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?
            .ok_or_else(|| AppError::NotFound(format!("library entry {id}")))?;
        entry_from_row(&row)
    }

    /// Live entries directly in `folder` (folders first, then by name).
    pub async fn list(&self, folder: &str) -> Result<Vec<LibraryEntry>, AppError> {
        let folder = clean_rel(folder)?;
        let rows = sqlx::query(select_entries!(
            "WHERE e.parent = ? AND e.deleted_at IS NULL ORDER BY e.kind = 'file', e.path COLLATE NOCASE"
        ))
        .bind(&folder)
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.iter().map(entry_from_row).collect()
    }

    pub async fn meta(&self, id: i64) -> Result<Option<FileMeta>, AppError> {
        let row = sqlx::query("SELECT * FROM file_meta WHERE entry_id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        let Some(r) = row else { return Ok(None) };
        let tools: String = r.try_get("tools_json").map_err(db)?;
        let plates: String = r.try_get("plates_json").map_err(db)?;
        Ok(Some(FileMeta {
            format: r.try_get("format").map_err(db)?,
            parse_status: r.try_get("parse_status").map_err(db)?,
            parse_error: r.try_get("parse_error").map_err(db)?,
            slicer: r.try_get("slicer").map_err(db)?,
            slicer_version: r.try_get("slicer_version").map_err(db)?,
            printer_model: r.try_get("printer_model").map_err(db)?,
            printer_settings_id: r.try_get("printer_settings_id").map_err(db)?,
            printer_model_id: r.try_get("printer_model_id").map_err(db)?,
            nozzle_diameter: r.try_get("nozzle_diameter").map_err(db)?,
            print_time_secs: r.try_get("print_time_secs").map_err(db)?,
            layer_count: r
                .try_get::<Option<i64>, _>("layer_count")
                .map_err(db)?
                .map(|v| v as u32),
            bed_width: r.try_get("bed_width").map_err(db)?,
            bed_depth: r.try_get("bed_depth").map_err(db)?,
            filament_settings_id: r.try_get("filament_settings_id").map_err(db)?,
            suggested_spool_id: r.try_get("suggested_spool_id").map_err(db)?,
            tools: serde_json::from_str(&tools).unwrap_or_default(),
            plates: serde_json::from_str(&plates).unwrap_or_default(),
            slot_kind: SlotKind::parse(
                r.try_get::<Option<String>, _>("slot_kind")
                    .map_err(db)?
                    .as_deref()
                    .unwrap_or("tool"),
            ),
            slots: r
                .try_get::<Option<String>, _>("slots_json")
                .map_err(db)?
                .and_then(|j| serde_json::from_str(&j).ok())
                .unwrap_or_default(),
        }))
    }

    pub async fn details(&self, id: i64) -> Result<LibraryFileDetails, AppError> {
        Ok(LibraryFileDetails {
            entry: self.get(id).await?,
            meta: self.meta(id).await?,
        })
    }

    async fn insert_row(
        &self,
        path: &str,
        kind: &str,
        size: i64,
        hash: Option<&str>,
        mtime: i64,
    ) -> Result<i64, AppError> {
        sqlx::query_scalar(
            "INSERT INTO library_entry (path, parent, kind, size, hash, mtime, added_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(path)
        .bind(parent_of(path))
        .bind(kind)
        .bind(size)
        .bind(hash)
        .bind(mtime)
        .bind(now())
        .fetch_one(&self.pool)
        .await
        .map_err(db)
    }

    /// Create `rel` and any missing parents (on disk and in the index).
    pub async fn ensure_folder(&self, rel: &str) -> Result<(), AppError> {
        let rel = clean_rel(rel)?;
        let mut cur = String::new();
        for part in rel.split('/').filter(|p| !p.is_empty()) {
            cur = join(&cur, part);
            fs::create_dir_all(self.abs(&cur)).map_err(io)?;
            if self.live_id(&cur).await?.is_none() {
                self.insert_row(&cur, "folder", 0, None, 0).await?;
            }
        }
        Ok(())
    }

    pub async fn mkdir(&self, folder: &str, name: &str) -> Result<LibraryEntry, AppError> {
        let path = join(&clean_rel(folder)?, &clean_name(name)?);
        if self.taken(&path).await? {
            return Err(AppError::InvalidInput(format!("{path} already exists")));
        }
        self.ensure_folder(&path).await?;
        let id = self
            .live_id(&path)
            .await?
            .ok_or_else(|| AppError::Internal("folder vanished".into()))?;
        self.get(id).await
    }

    /// Move `src` (a finished upload) into `folder` as `name` (suffixed if taken), then index + parse it.
    pub async fn add_file(
        &self,
        folder: &str,
        name: &str,
        src: &Path,
    ) -> Result<LibraryEntry, AppError> {
        let folder = clean_rel(folder)?;
        let name = clean_name(name)?;
        self.ensure_folder(&folder).await?;
        let path = self.unique_path(&folder, &name).await?;
        let dest = self.abs(&path);
        move_path(src, &dest)?;
        let (size, hash, mtime) = file_facts(&dest).await?;
        let id = self
            .insert_row(&path, "file", size, Some(&hash), mtime)
            .await?;
        self.reparse(id, &dest).await?;
        info!(%path, size, "library: file added");
        self.get(id).await
    }

    async fn reparse(&self, id: i64, abs: &Path) -> Result<(), AppError> {
        let p = abs.to_path_buf();
        let parsed = tokio::task::spawn_blocking(move || parse_path(&p))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        self.save_meta(id, parsed).await
    }

    async fn save_meta(&self, id: i64, parsed: Parsed) -> Result<(), AppError> {
        let thumb_path = self.thumb_path(id);
        let thumb_file = match parsed.thumbnail {
            Some(png) => {
                fs::write(&thumb_path, png).map_err(io)?;
                Some(format!("{id}.png"))
            }
            None => {
                let _ = fs::remove_file(&thumb_path);
                None
            }
        };
        let m = parsed.meta;
        if m.parse_status == "error" {
            warn!(
                id,
                error = m.parse_error.as_deref().unwrap_or(""),
                "library: could not parse file"
            );
        }
        sqlx::query(
            "INSERT OR REPLACE INTO file_meta (entry_id, format, parse_status, parse_error, slicer, \
             slicer_version, printer_model, printer_settings_id, printer_model_id, nozzle_diameter, \
             print_time_secs, layer_count, bed_width, bed_depth, filament_settings_id, \
             suggested_spool_id, tools_json, plates_json, thumb_file, parsed_at, slot_kind, \
             slots_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(&m.format)
        .bind(&m.parse_status)
        .bind(&m.parse_error)
        .bind(&m.slicer)
        .bind(&m.slicer_version)
        .bind(&m.printer_model)
        .bind(&m.printer_settings_id)
        .bind(&m.printer_model_id)
        .bind(m.nozzle_diameter)
        .bind(m.print_time_secs)
        .bind(m.layer_count.map(i64::from))
        .bind(m.bed_width)
        .bind(m.bed_depth)
        .bind(&m.filament_settings_id)
        .bind(m.suggested_spool_id)
        .bind(serde_json::to_string(&m.tools).unwrap_or_else(|_| "[]".into()))
        .bind(serde_json::to_string(&m.plates).unwrap_or_else(|_| "[]".into()))
        .bind(thumb_file)
        .bind(now())
        .bind(m.slot_kind.as_str())
        .bind(serde_json::to_string(&m.slots).unwrap_or_else(|_| "[]".into()))
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn live(&self, id: i64) -> Result<LibraryEntry, AppError> {
        let e = self.get(id).await?;
        if e.deleted_at.is_some() {
            return Err(AppError::NotFound(format!(
                "library entry {id} is in the trash"
            )));
        }
        Ok(e)
    }

    /// Live descendants of a folder: (id, path).
    async fn descendants(&self, path: &str) -> Result<Vec<(i64, String)>, AppError> {
        let rows = sqlx::query("SELECT id, path FROM library_entry WHERE deleted_at IS NULL AND substr(path, 1, ?) = ?")
            .bind((path.len() + 1) as i64)
            .bind(format!("{path}/"))
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.iter()
            .map(|r| Ok((r.try_get("id").map_err(db)?, r.try_get("path").map_err(db)?)))
            .collect()
    }

    async fn set_path(&self, id: i64, path: &str) -> Result<(), AppError> {
        sqlx::query("UPDATE library_entry SET path = ?, parent = ? WHERE id = ?")
            .bind(path)
            .bind(parent_of(path))
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    pub async fn rename(&self, id: i64, new_name: &str) -> Result<LibraryEntry, AppError> {
        let e = self.live(id).await?;
        let new_name = clean_name(new_name)?;
        if new_name == e.name {
            return Ok(e);
        }
        let new_path = join(parent_of(&e.path), &new_name);
        // Case-only renames hit the same file on case-insensitive disks.
        if !new_path.eq_ignore_ascii_case(&e.path) && self.taken(&new_path).await? {
            return Err(AppError::InvalidInput(format!("{new_path} already exists")));
        }
        fs::rename(self.abs(&e.path), self.abs(&new_path)).map_err(io)?;
        for (child, p) in self.descendants(&e.path).await? {
            self.set_path(child, &format!("{new_path}{}", &p[e.path.len()..]))
                .await?;
        }
        self.set_path(id, &new_path).await?;
        self.get(id).await
    }

    /// Live entry at `path`, if any.
    pub async fn find(&self, path: &str) -> Result<Option<LibraryEntry>, AppError> {
        match self.live_id(&clean_rel(path)?).await? {
            Some(id) => Ok(Some(self.get(id).await?)),
            None => Ok(None),
        }
    }

    /// Move an entry (and a folder's contents) into `folder` as `name`, which must be free.
    pub async fn move_to(
        &self,
        id: i64,
        folder: &str,
        name: &str,
    ) -> Result<LibraryEntry, AppError> {
        let e = self.live(id).await?;
        let folder = clean_rel(folder)?;
        let new_path = join(&folder, &clean_name(name)?);
        if new_path == e.path {
            return Ok(e);
        }
        if folder == e.path || folder.starts_with(&format!("{}/", e.path)) {
            return Err(AppError::InvalidInput(format!(
                "can't move {} into itself",
                e.name
            )));
        }
        if !new_path.eq_ignore_ascii_case(&e.path) && self.taken(&new_path).await? {
            return Err(AppError::InvalidInput(format!("{new_path} already exists")));
        }
        self.ensure_folder(&folder).await?;
        fs::rename(self.abs(&e.path), self.abs(&new_path)).map_err(io)?;
        for (child, p) in self.descendants(&e.path).await? {
            self.set_path(child, &format!("{new_path}{}", &p[e.path.len()..]))
                .await?;
        }
        self.set_path(id, &new_path).await?;
        info!(from = %e.path, to = %new_path, "library: moved");
        self.get(id).await
    }

    pub async fn set_starred(&self, id: i64, starred: bool) -> Result<LibraryEntry, AppError> {
        self.live(id).await?;
        sqlx::query("UPDATE library_entry SET starred = ? WHERE id = ?")
            .bind(starred)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        self.get(id).await
    }

    /// Move an entry (and a folder's contents) to the trash.
    pub async fn trash(&self, id: i64) -> Result<(), AppError> {
        let e = self.live(id).await?;
        let trash_id = uuid::Uuid::new_v4().to_string();
        move_dir_or_file(
            &self.abs(&e.path),
            &self.trash.join(&trash_id).join(&e.name),
        )?;
        let when = now();
        let mut ids: Vec<i64> = self
            .descendants(&e.path)
            .await?
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        ids.push(id);
        for i in ids {
            sqlx::query("UPDATE library_entry SET deleted_at = ?, trash_id = ?, trash_root = ? WHERE id = ?")
                .bind(&when)
                .bind(&trash_id)
                .bind(i == id)
                .bind(i)
                .execute(&self.pool)
                .await
                .map_err(db)?;
        }
        info!(path = %e.path, "library: moved to trash");
        Ok(())
    }

    /// Top-level trashed entries, newest first.
    pub async fn list_trash(&self) -> Result<Vec<LibraryEntry>, AppError> {
        let rows = sqlx::query(select_entries!(
            "WHERE e.trash_root = 1 ORDER BY e.deleted_at DESC"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.iter().map(entry_from_row).collect()
    }

    /// Put a trashed entry back where it was (suffixed if the name was reused meanwhile).
    pub async fn restore(&self, id: i64) -> Result<LibraryEntry, AppError> {
        let row = sqlx::query("SELECT path, trash_id, trash_root FROM library_entry WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?
            .ok_or_else(|| AppError::NotFound(format!("library entry {id}")))?;
        let old: String = row.try_get("path").map_err(db)?;
        let trash_id: Option<String> = row.try_get("trash_id").map_err(db)?;
        let is_root: bool = row.try_get::<i64, _>("trash_root").map_err(db)? != 0;
        let (Some(trash_id), true) = (trash_id, is_root) else {
            return Err(AppError::InvalidInput(
                "only a trashed item itself can be restored".into(),
            ));
        };
        let folder = parent_of(&old).to_owned();
        self.ensure_folder(&folder).await?;
        let target = self.unique_path(&folder, name_of(&old)).await?;
        move_dir_or_file(
            &self.trash.join(&trash_id).join(name_of(&old)),
            &self.abs(&target),
        )?;
        let _ = fs::remove_dir_all(self.trash.join(&trash_id));

        let group = sqlx::query("SELECT id, path FROM library_entry WHERE trash_id = ?")
            .bind(&trash_id)
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        for r in &group {
            let gid: i64 = r.try_get("id").map_err(db)?;
            let p: String = r.try_get("path").map_err(db)?;
            let new_path = format!("{target}{}", &p[old.len()..]);
            sqlx::query(
                "UPDATE library_entry SET path = ?, parent = ?, deleted_at = NULL, trash_id = NULL, \
                 trash_root = 0 WHERE id = ?",
            )
            .bind(&new_path)
            .bind(parent_of(&new_path))
            .bind(gid)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        }
        info!(path = %target, "library: restored from trash");
        self.get(id).await
    }

    /// Permanently delete trash older than `days`. Returns how many trashed items went.
    pub async fn purge_trash(&self, days: i64) -> Result<usize, AppError> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        let rows = sqlx::query(
            "SELECT DISTINCT trash_id FROM library_entry WHERE trash_root = 1 AND deleted_at < ?",
        )
        .bind(&cutoff)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        let mut n = 0;
        for r in rows {
            let trash_id: String = r.try_get("trash_id").map_err(db)?;
            self.drop_group(&trash_id).await?;
            n += 1;
        }
        if n > 0 {
            info!(count = n, days, "library: purged old trash");
        }
        Ok(n)
    }

    async fn drop_group(&self, trash_id: &str) -> Result<(), AppError> {
        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM library_entry WHERE trash_id = ?")
            .bind(trash_id)
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        for id in &ids {
            let _ = fs::remove_file(self.thumb_path(*id));
        }
        sqlx::query("DELETE FROM library_entry WHERE trash_id = ?")
            .bind(trash_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        let dir = self.trash.join(trash_id);
        if dir.exists() {
            fs::remove_dir_all(dir).map_err(io)?;
        }
        Ok(())
    }

    /// Make the index match the disk: add new files/folders, re-parse changed files,
    /// drop rows whose file is gone.
    pub async fn rebuild_index(&self) -> Result<RebuildReport, AppError> {
        let root = self.root.clone();
        let on_disk = tokio::task::spawn_blocking(move || walk(&root))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))??;
        let rows = sqlx::query(
            "SELECT id, path, kind, size, mtime FROM library_entry WHERE deleted_at IS NULL",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        let mut indexed: HashMap<String, (i64, String, i64, i64)> = HashMap::new();
        for r in &rows {
            let path: String = r.try_get("path").map_err(db)?;
            indexed.insert(
                path,
                (
                    r.try_get("id").map_err(db)?,
                    r.try_get("kind").map_err(db)?,
                    r.try_get("size").map_err(db)?,
                    r.try_get::<Option<i64>, _>("mtime")
                        .map_err(db)?
                        .unwrap_or(0),
                ),
            );
        }

        let mut report = RebuildReport::default();
        for (path, (id, kind, _, _)) in &indexed {
            let gone = match on_disk.get(path) {
                None => true,
                Some(d) => (kind == "folder") != d.is_dir,
            };
            if gone {
                let _ = fs::remove_file(self.thumb_path(*id));
                sqlx::query("DELETE FROM library_entry WHERE id = ?")
                    .bind(id)
                    .execute(&self.pool)
                    .await
                    .map_err(db)?;
                report.removed += 1;
            }
        }
        let mut paths: Vec<&String> = on_disk.keys().collect();
        paths.sort();
        for path in paths {
            let d = &on_disk[path];
            match indexed.get(path) {
                Some((_, kind, _, _)) if (kind == "folder") != d.is_dir => {
                    self.index_new(path, d).await?;
                    report.added += 1;
                }
                Some((id, _, size, mtime))
                    if !d.is_dir && (*size != d.size || *mtime != d.mtime) =>
                {
                    let abs = self.abs(path);
                    let (size, hash, mtime) = file_facts(&abs).await?;
                    sqlx::query(
                        "UPDATE library_entry SET size = ?, hash = ?, mtime = ? WHERE id = ?",
                    )
                    .bind(size)
                    .bind(&hash)
                    .bind(mtime)
                    .bind(id)
                    .execute(&self.pool)
                    .await
                    .map_err(db)?;
                    self.reparse(*id, &abs).await?;
                    report.updated += 1;
                }
                Some(_) => {}
                None => {
                    self.index_new(path, d).await?;
                    report.added += 1;
                }
            }
        }
        if report != RebuildReport::default() {
            info!(?report, "library: index rebuilt from disk");
        }
        Ok(report)
    }

    async fn index_new(&self, path: &str, d: &DiskEntry) -> Result<(), AppError> {
        if d.is_dir {
            self.insert_row(path, "folder", 0, None, 0).await?;
        } else {
            let abs = self.abs(path);
            let (size, hash, mtime) = file_facts(&abs).await?;
            let id = self
                .insert_row(path, "file", size, Some(&hash), mtime)
                .await?;
            self.reparse(id, &abs).await?;
        }
        Ok(())
    }

    /// Re-parse live files whose facts predate filament slots (5.2). Returns how many.
    pub async fn reparse_missing_slots(&self) -> Result<usize, AppError> {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT m.entry_id FROM file_meta m JOIN library_entry e ON e.id = m.entry_id \
             WHERE m.slots_json IS NULL AND e.deleted_at IS NULL",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        let mut n = 0;
        for id in ids {
            match self.file_path(id).await {
                Ok(abs) => {
                    self.reparse(id, &abs).await?;
                    n += 1;
                }
                Err(e) => warn!(id, "library: re-parse for slots: {e}"),
            }
        }
        if n > 0 {
            info!(files = n, "library: re-parsed files for filament slots");
        }
        Ok(n)
    }

    /// Absolute path of a live file (for download / thumbnails / parsing).
    pub async fn file_path(&self, id: i64) -> Result<PathBuf, AppError> {
        let e = self.live(id).await?;
        if e.kind != LibraryEntryKind::File {
            return Err(AppError::InvalidInput("not a file".into()));
        }
        Ok(self.abs(&e.path))
    }
}

fn move_dir_or_file(from: &Path, to: &Path) -> Result<(), AppError> {
    if from.is_dir() {
        if let Some(p) = to.parent() {
            fs::create_dir_all(p).map_err(io)?;
        }
        fs::rename(from, to).map_err(io)
    } else {
        move_path(from, to)
    }
}

async fn file_facts(abs: &Path) -> Result<(i64, String, i64), AppError> {
    let p = abs.to_path_buf();
    tokio::task::spawn_blocking(move || -> std::io::Result<(i64, String, i64)> {
        let md = fs::metadata(&p)?;
        Ok((md.len() as i64, sha256_file(&p)?, mtime_secs(&md)))
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map_err(io)
}

struct DiskEntry {
    is_dir: bool,
    size: i64,
    mtime: i64,
}

fn walk(root: &Path) -> Result<HashMap<String, DiskEntry>, AppError> {
    let mut out = HashMap::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        for item in fs::read_dir(&dir).map_err(io)? {
            let item = item.map_err(io)?;
            let Some(name) = item.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if clean_name(&name).is_err() {
                continue;
            }
            let md = item.metadata().map_err(io)?;
            let path = join(&rel, &name);
            if md.is_dir() {
                stack.push((item.path(), path.clone()));
            }
            out.insert(
                path,
                DiskEntry {
                    is_dir: md.is_dir(),
                    size: if md.is_dir() { 0 } else { md.len() as i64 },
                    mtime: if md.is_dir() { 0 } else { mtime_secs(&md) },
                },
            );
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths() {
        assert_eq!(
            clean_rel("/Slicer inbox//a/").expect("ok"),
            "Slicer inbox/a"
        );
        assert!(clean_rel("a/../b").is_err());
        assert!(clean_name("..").is_err());
        assert!(clean_name("a/b").is_err());
        assert!(clean_name("con:x").is_err());
        assert_eq!(split_ext("Cube.gcode.3mf"), ("Cube", ".gcode.3mf"));
        assert_eq!(split_ext("a.gcode"), ("a", ".gcode"));
        assert_eq!(split_ext(".hidden"), (".hidden", ""));
        assert_eq!(parent_of("a/b/c.gcode"), "a/b");
        assert_eq!(parent_of("c.gcode"), "");
    }
}
