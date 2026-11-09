//! Remote file operations for the Files tab, behind a small trait so the rules
//! (`.bak` before overwrite, read-only logs root, path normalisation) are testable
//! without a live SSH server. [`SftpFs`] is the real implementation.

use async_trait::async_trait;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _};

use crate::{
    domain::{SftpEntry, SftpEntryKind},
    error::AppError,
};

/// Max bytes returned by a text read (editor / log viewer). Larger files show the tail.
pub const MAX_TEXT_READ: u64 = 2 * 1024 * 1024;

#[async_trait]
pub trait RemoteFs: Send + Sync {
    /// Canonical absolute path of `path` (resolves symlinks; `.` = home dir).
    async fn realpath(&self, path: &str) -> Result<String, AppError>;
    async fn list(&self, path: &str) -> Result<Vec<SftpEntry>, AppError>;
    /// `None` when the path does not exist.
    async fn stat(&self, path: &str) -> Result<Option<(SftpEntryKind, u64)>, AppError>;
    async fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, AppError>;
    /// Create or truncate, then write.
    async fn write(&self, path: &str, data: &[u8]) -> Result<(), AppError>;
    async fn mkdir(&self, path: &str) -> Result<(), AppError>;
    async fn rename(&self, from: &str, to: &str) -> Result<(), AppError>;
    async fn remove_file(&self, path: &str) -> Result<(), AppError>;
    async fn remove_dir(&self, path: &str) -> Result<(), AppError>;
}

// ---------------------------------------------------------------------------
// Roots + path rules
// ---------------------------------------------------------------------------

/// Quick roots under the SSH user's home (`~/printer_data/{config,gcodes,logs}`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Roots {
    pub home: String,
    pub config: String,
    pub gcodes: String,
    pub logs: String,
}

impl Roots {
    pub fn from_home(home: &str) -> Self {
        let home = home.trim_end_matches('/').to_owned();
        let pd = format!("{home}/printer_data");
        Self {
            config: format!("{pd}/config"),
            gcodes: format!("{pd}/gcodes"),
            logs: format!("{pd}/logs"),
            home,
        }
    }

    pub async fn discover(fs: &dyn RemoteFs) -> Result<Self, AppError> {
        Ok(Self::from_home(&fs.realpath(".").await?))
    }

    pub fn is_read_only(&self, normalized: &str) -> bool {
        is_under(normalized, &self.logs)
    }
}

fn is_under(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Lexically normalise an absolute POSIX path (`.`/`..`/`//`). Rejects relative paths,
/// NUL bytes and escaping above `/`.
pub fn normalize(path: &str) -> Result<String, AppError> {
    if !path.starts_with('/') {
        return Err(AppError::InvalidInput(format!(
            "path must be absolute: {path}"
        )));
    }
    if path.contains('\0') {
        return Err(AppError::InvalidInput("path contains NUL".into()));
    }
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(AppError::InvalidInput(format!("path escapes root: {path}")));
                }
            }
            s => parts.push(s),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

fn parent_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i],
    }
}

/// Normalise and refuse anything under the read-only logs root, including via a
/// symlinked parent directory.
async fn guard_mutation(fs: &dyn RemoteFs, roots: &Roots, path: &str) -> Result<String, AppError> {
    let p = normalize(path)?;
    if p == "/" {
        return Err(AppError::InvalidInput("refusing to modify /".into()));
    }
    let real_parent = fs
        .realpath(parent_of(&p))
        .await
        .unwrap_or_else(|_| parent_of(&p).to_owned());
    if roots.is_read_only(&p) || roots.is_read_only(&real_parent) {
        return Err(AppError::InvalidInput(
            "logs are read-only — download instead".into(),
        ));
    }
    Ok(p)
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

pub async fn list(fs: &dyn RemoteFs, path: &str) -> Result<Vec<SftpEntry>, AppError> {
    let p = normalize(path)?;
    let mut entries: Vec<SftpEntry> = fs
        .list(&p)
        .await?
        .into_iter()
        .filter(|e| e.name != "." && e.name != "..")
        .collect();
    entries.sort_by(|a, b| {
        let ad = matches!(a.kind, SftpEntryKind::Directory);
        let bd = matches!(b.kind, SftpEntryKind::Directory);
        bd.cmp(&ad)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

/// Text read result for the editor / log viewer.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TextRead {
    pub path: String,
    pub content: String,
    pub size: u64,
    /// Only the tail was returned (file larger than [`MAX_TEXT_READ`]).
    pub truncated: bool,
    pub read_only: bool,
}

/// Read a file as UTF-8 text. Large files return their last [`MAX_TEXT_READ`] bytes
/// (and are then read-only so a partial file can't be saved back).
pub async fn read_text(fs: &dyn RemoteFs, roots: &Roots, path: &str) -> Result<TextRead, AppError> {
    let p = normalize(path)?;
    let (kind, size) = fs
        .stat(&p)
        .await?
        .ok_or_else(|| AppError::NotFound(p.clone()))?;
    if matches!(kind, SftpEntryKind::Directory) {
        return Err(AppError::InvalidInput(format!("{p} is a directory")));
    }
    let truncated = size > MAX_TEXT_READ;
    let offset = size.saturating_sub(MAX_TEXT_READ);
    let bytes = fs.read_range(&p, offset, MAX_TEXT_READ).await?;
    let content = if truncated {
        String::from_utf8_lossy(&bytes).into_owned()
    } else {
        String::from_utf8(bytes).map_err(|_| AppError::InvalidInput("binary file".into()))?
    };
    let read_only = truncated || roots.is_read_only(&p);
    Ok(TextRead {
        path: p,
        content,
        size,
        truncated,
        read_only,
    })
}

/// Copy an existing file to `<path>.bak`. Returns whether a backup was made.
async fn backup_existing(fs: &dyn RemoteFs, path: &str) -> Result<bool, AppError> {
    match fs.stat(path).await? {
        Some((SftpEntryKind::Directory, _)) => {
            Err(AppError::InvalidInput(format!("{path} is a directory")))
        }
        Some((_, size)) => {
            let original = fs.read_range(path, 0, size).await?;
            fs.write(&format!("{path}.bak"), &original).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Write a whole file, keeping the previous contents in `<path>.bak`.
pub async fn write_with_backup(
    fs: &dyn RemoteFs,
    roots: &Roots,
    path: &str,
    data: &[u8],
) -> Result<bool, AppError> {
    let p = guard_mutation(fs, roots, path).await?;
    let backed_up = backup_existing(fs, &p).await?;
    fs.write(&p, data).await?;
    Ok(backed_up)
}

/// Checks + `.bak` for an upload; the caller then streams to the returned path.
pub async fn prepare_upload(
    fs: &dyn RemoteFs,
    roots: &Roots,
    path: &str,
) -> Result<String, AppError> {
    let p = guard_mutation(fs, roots, path).await?;
    backup_existing(fs, &p).await?;
    Ok(p)
}

pub async fn mkdir(fs: &dyn RemoteFs, roots: &Roots, path: &str) -> Result<String, AppError> {
    let p = guard_mutation(fs, roots, path).await?;
    fs.mkdir(&p).await?;
    Ok(p)
}

pub async fn rename(
    fs: &dyn RemoteFs,
    roots: &Roots,
    from: &str,
    to: &str,
) -> Result<(String, String), AppError> {
    let f = guard_mutation(fs, roots, from).await?;
    let t = guard_mutation(fs, roots, to).await?;
    if fs.stat(&t).await?.is_some() {
        return Err(AppError::InvalidInput(format!("{t} already exists")));
    }
    fs.rename(&f, &t).await?;
    Ok((f, t))
}

/// Delete a file or an empty directory.
pub async fn delete(fs: &dyn RemoteFs, roots: &Roots, path: &str) -> Result<String, AppError> {
    let p = guard_mutation(fs, roots, path).await?;
    match fs.stat(&p).await? {
        None => Err(AppError::NotFound(p)),
        Some((SftpEntryKind::Directory, _)) => {
            fs.remove_dir(&p).await?;
            Ok(p)
        }
        Some(_) => {
            fs.remove_file(&p).await?;
            Ok(p)
        }
    }
}

/// True for Klipper config files that "Save + FIRMWARE_RESTART" applies to.
pub fn is_klipper_cfg(roots: &Roots, path: &str) -> bool {
    normalize(path).is_ok_and(|p| is_under(&p, &roots.config) && p.ends_with(".cfg"))
}

// ---------------------------------------------------------------------------
// SFTP implementation
// ---------------------------------------------------------------------------

/// [`RemoteFs`] over a live SFTP session.
pub struct SftpFs {
    sftp: russh_sftp::client::SftpSession,
}

fn sftp_err(op: &str, path: &str, e: russh_sftp::client::error::Error) -> AppError {
    AppError::Driver(format!("sftp {op} '{path}': {e}"))
}

fn is_not_found(e: &russh_sftp::client::error::Error) -> bool {
    matches!(
        e,
        russh_sftp::client::error::Error::Status(s)
            if s.status_code == russh_sftp::protocol::StatusCode::NoSuchFile
    )
}

fn kind_of(m: &russh_sftp::client::fs::Metadata) -> SftpEntryKind {
    if m.is_dir() {
        SftpEntryKind::Directory
    } else if m.is_symlink() {
        SftpEntryKind::Symlink
    } else if m.is_regular() {
        SftpEntryKind::File
    } else {
        SftpEntryKind::Other
    }
}

impl SftpFs {
    pub fn new(sftp: russh_sftp::client::SftpSession) -> Self {
        Self { sftp }
    }

    pub fn session(&self) -> &russh_sftp::client::SftpSession {
        &self.sftp
    }
}

#[async_trait]
impl RemoteFs for SftpFs {
    async fn realpath(&self, path: &str) -> Result<String, AppError> {
        self.sftp
            .canonicalize(path)
            .await
            .map_err(|e| sftp_err("realpath", path, e))
    }

    async fn list(&self, path: &str) -> Result<Vec<SftpEntry>, AppError> {
        let dir = self
            .sftp
            .read_dir(path)
            .await
            .map_err(|e| sftp_err("readdir", path, e))?;
        let base = path.trim_end_matches('/');
        let mut out = Vec::new();
        for item in dir {
            let name = item.file_name();
            let full = format!("{base}/{name}");
            let mut meta = item.metadata();
            // Follow symlinks so linked dirs (e.g. gcodes on some installs) browse like dirs.
            if meta.is_symlink() {
                if let Ok(target) = self.sftp.metadata(full.clone()).await {
                    meta = target;
                }
            }
            out.push(SftpEntry {
                kind: kind_of(&meta),
                size: (!meta.is_dir()).then(|| meta.len()),
                modified: meta.mtime.map(i64::from),
                path: full,
                name,
            });
        }
        Ok(out)
    }

    async fn stat(&self, path: &str) -> Result<Option<(SftpEntryKind, u64)>, AppError> {
        match self.sftp.metadata(path).await {
            Ok(m) => Ok(Some((kind_of(&m), m.len()))),
            Err(e) if is_not_found(&e) => Ok(None),
            Err(e) => Err(sftp_err("stat", path, e)),
        }
    }

    async fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, AppError> {
        let mut f = self
            .sftp
            .open(path)
            .await
            .map_err(|e| sftp_err("open", path, e))?;
        if offset > 0 {
            f.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(|e| AppError::Driver(format!("sftp seek '{path}': {e}")))?;
        }
        let mut buf = Vec::new();
        (&mut f)
            .take(len)
            .read_to_end(&mut buf)
            .await
            .map_err(|e| AppError::Driver(format!("sftp read '{path}': {e}")))?;
        Ok(buf)
    }

    async fn write(&self, path: &str, data: &[u8]) -> Result<(), AppError> {
        let mut f = self
            .sftp
            .create(path)
            .await
            .map_err(|e| sftp_err("create", path, e))?;
        f.write_all(data)
            .await
            .map_err(|e| AppError::Driver(format!("sftp write '{path}': {e}")))?;
        f.shutdown()
            .await
            .map_err(|e| AppError::Driver(format!("sftp close '{path}': {e}")))?;
        Ok(())
    }

    async fn mkdir(&self, path: &str) -> Result<(), AppError> {
        self.sftp
            .create_dir(path)
            .await
            .map_err(|e| sftp_err("mkdir", path, e))
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), AppError> {
        self.sftp
            .rename(from, to)
            .await
            .map_err(|e| sftp_err("rename", from, e))
    }

    async fn remove_file(&self, path: &str) -> Result<(), AppError> {
        self.sftp
            .remove_file(path)
            .await
            .map_err(|e| sftp_err("remove", path, e))
    }

    async fn remove_dir(&self, path: &str) -> Result<(), AppError> {
        self.sftp
            .remove_dir(path)
            .await
            .map_err(|e| sftp_err("rmdir", path, e))
    }
}

// ---------------------------------------------------------------------------
// In-memory implementation (tests)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod mem {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// Flat in-memory FS: `None` value = directory.
    pub struct MemFs {
        pub home: String,
        pub nodes: Mutex<BTreeMap<String, Option<Vec<u8>>>>,
        pub symlinks: BTreeMap<String, String>,
    }

    impl MemFs {
        pub fn new(home: &str) -> Self {
            let mut nodes = BTreeMap::new();
            nodes.insert("/".to_owned(), None);
            Self {
                home: home.to_owned(),
                nodes: Mutex::new(nodes),
                symlinks: BTreeMap::new(),
            }
        }
        pub fn dir(&self, p: &str) {
            self.nodes.lock().expect("lock").insert(p.to_owned(), None);
        }
        pub fn file(&self, p: &str, data: &str) {
            self.nodes
                .lock()
                .expect("lock")
                .insert(p.to_owned(), Some(data.as_bytes().to_vec()));
        }
        pub fn get(&self, p: &str) -> Option<String> {
            self.nodes
                .lock()
                .expect("lock")
                .get(p)
                .cloned()
                .flatten()
                .map(|b| String::from_utf8_lossy(&b).into_owned())
        }
        pub fn exists(&self, p: &str) -> bool {
            self.nodes.lock().expect("lock").contains_key(p)
        }
    }

    fn nf(p: &str) -> AppError {
        AppError::Driver(format!("no such file: {p}"))
    }

    #[async_trait]
    impl RemoteFs for MemFs {
        async fn realpath(&self, path: &str) -> Result<String, AppError> {
            let p = if path == "." {
                self.home.clone()
            } else {
                path.to_owned()
            };
            Ok(self.symlinks.get(&p).cloned().unwrap_or(p))
        }
        async fn list(&self, path: &str) -> Result<Vec<SftpEntry>, AppError> {
            let nodes = self.nodes.lock().expect("lock");
            let prefix = format!("{}/", path.trim_end_matches('/'));
            Ok(nodes
                .iter()
                .filter(|(k, _)| {
                    k.strip_prefix(&prefix)
                        .is_some_and(|r| !r.is_empty() && !r.contains('/'))
                })
                .map(|(k, v)| SftpEntry {
                    name: k[prefix.len()..].to_owned(),
                    path: k.clone(),
                    kind: if v.is_some() {
                        SftpEntryKind::File
                    } else {
                        SftpEntryKind::Directory
                    },
                    size: v.as_ref().map(|b| b.len() as u64),
                    modified: None,
                })
                .collect())
        }
        async fn stat(&self, path: &str) -> Result<Option<(SftpEntryKind, u64)>, AppError> {
            Ok(self.nodes.lock().expect("lock").get(path).map(|v| match v {
                Some(b) => (SftpEntryKind::File, b.len() as u64),
                None => (SftpEntryKind::Directory, 0),
            }))
        }
        async fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, AppError> {
            let nodes = self.nodes.lock().expect("lock");
            let b = nodes.get(path).cloned().flatten().ok_or_else(|| nf(path))?;
            let start = (offset as usize).min(b.len());
            let end = (start + len as usize).min(b.len());
            Ok(b[start..end].to_vec())
        }
        async fn write(&self, path: &str, data: &[u8]) -> Result<(), AppError> {
            self.nodes
                .lock()
                .expect("lock")
                .insert(path.to_owned(), Some(data.to_vec()));
            Ok(())
        }
        async fn mkdir(&self, path: &str) -> Result<(), AppError> {
            self.dir(path);
            Ok(())
        }
        async fn rename(&self, from: &str, to: &str) -> Result<(), AppError> {
            let mut nodes = self.nodes.lock().expect("lock");
            let v = nodes.remove(from).ok_or_else(|| nf(from))?;
            nodes.insert(to.to_owned(), v);
            Ok(())
        }
        async fn remove_file(&self, path: &str) -> Result<(), AppError> {
            self.nodes
                .lock()
                .expect("lock")
                .remove(path)
                .map(|_| ())
                .ok_or_else(|| nf(path))
        }
        async fn remove_dir(&self, path: &str) -> Result<(), AppError> {
            self.remove_file(path).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mem::MemFs;
    use super::*;

    const HOME: &str = "/home/pi";

    fn setup() -> (MemFs, Roots) {
        let fs = MemFs::new(HOME);
        for d in [
            "/home",
            HOME,
            "/home/pi/printer_data",
            "/home/pi/printer_data/config",
            "/home/pi/printer_data/gcodes",
            "/home/pi/printer_data/logs",
        ] {
            fs.dir(d);
        }
        fs.file(
            "/home/pi/printer_data/config/printer.cfg",
            "[printer]\nold\n",
        );
        fs.file("/home/pi/printer_data/logs/klippy.log", "log line\n");
        (fs, Roots::from_home(HOME))
    }

    #[test]
    fn normalize_rules() {
        assert_eq!(normalize("/a/./b//c/../d").expect("ok"), "/a/b/d");
        assert_eq!(normalize("/").expect("ok"), "/");
        assert!(normalize("relative/path").is_err());
        assert!(normalize("/../etc").is_err());
        assert!(normalize("/a\0b").is_err());
    }

    #[test]
    fn logs_root_detection() {
        let r = Roots::from_home(HOME);
        assert!(r.is_read_only("/home/pi/printer_data/logs"));
        assert!(r.is_read_only("/home/pi/printer_data/logs/klippy.log"));
        assert!(!r.is_read_only("/home/pi/printer_data/logs_old/x"));
        assert!(!r.is_read_only("/home/pi/printer_data/config/printer.cfg"));
    }

    #[tokio::test]
    async fn write_creates_bak_with_original_contents() {
        let (fs, roots) = setup();
        let p = "/home/pi/printer_data/config/printer.cfg";
        let backed = write_with_backup(&fs, &roots, p, b"[printer]\nnew\n")
            .await
            .expect("write");
        assert!(backed);
        assert_eq!(fs.get(p).as_deref(), Some("[printer]\nnew\n"));
        assert_eq!(
            fs.get(&format!("{p}.bak")).as_deref(),
            Some("[printer]\nold\n")
        );
    }

    #[tokio::test]
    async fn write_new_file_has_no_bak() {
        let (fs, roots) = setup();
        let p = "/home/pi/printer_data/config/new.cfg";
        let backed = write_with_backup(&fs, &roots, p, b"x")
            .await
            .expect("write");
        assert!(!backed);
        assert!(!fs.exists(&format!("{p}.bak")));
    }

    #[tokio::test]
    async fn logs_are_read_only() {
        let (fs, roots) = setup();
        let log = "/home/pi/printer_data/logs/klippy.log";
        assert!(write_with_backup(&fs, &roots, log, b"x").await.is_err());
        assert!(delete(&fs, &roots, log).await.is_err());
        assert!(mkdir(&fs, &roots, "/home/pi/printer_data/logs/d")
            .await
            .is_err());
        assert!(
            prepare_upload(&fs, &roots, "/home/pi/printer_data/logs/u.txt")
                .await
                .is_err()
        );
        // Moving a log out, or something in, is also refused.
        assert!(
            rename(&fs, &roots, log, "/home/pi/printer_data/config/k.log")
                .await
                .is_err()
        );
        // Traversal into logs is normalised and refused.
        assert!(delete(
            &fs,
            &roots,
            "/home/pi/printer_data/config/../logs/klippy.log"
        )
        .await
        .is_err());
        assert_eq!(fs.get(log).as_deref(), Some("log line\n"));

        let t = read_text(&fs, &roots, log).await.expect("read");
        assert!(t.read_only);
    }

    #[tokio::test]
    async fn symlinked_parent_into_logs_is_refused() {
        let (mut fs, roots) = setup();
        fs.symlinks.insert(
            "/home/pi/printer_data/config/logs_link".into(),
            "/home/pi/printer_data/logs".into(),
        );
        assert!(write_with_backup(
            &fs,
            &roots,
            "/home/pi/printer_data/config/logs_link/x.log",
            b"x"
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn delete_file_and_dir() {
        let (fs, roots) = setup();
        fs.file("/home/pi/printer_data/gcodes/a.gcode", "G28");
        fs.dir("/home/pi/printer_data/gcodes/empty");
        delete(&fs, &roots, "/home/pi/printer_data/gcodes/a.gcode")
            .await
            .expect("del file");
        delete(&fs, &roots, "/home/pi/printer_data/gcodes/empty")
            .await
            .expect("del dir");
        assert!(!fs.exists("/home/pi/printer_data/gcodes/a.gcode"));
        assert!(!fs.exists("/home/pi/printer_data/gcodes/empty"));
        assert!(delete(&fs, &roots, "/home/pi/printer_data/gcodes/missing")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn upload_over_existing_keeps_bak() {
        let (fs, roots) = setup();
        let p = "/home/pi/printer_data/config/printer.cfg";
        let got = prepare_upload(&fs, &roots, p).await.expect("prepare");
        assert_eq!(got, p);
        assert_eq!(
            fs.get(&format!("{p}.bak")).as_deref(),
            Some("[printer]\nold\n")
        );
    }

    #[tokio::test]
    async fn rename_refuses_overwrite() {
        let (fs, roots) = setup();
        fs.file("/home/pi/printer_data/config/a.cfg", "a");
        assert!(rename(
            &fs,
            &roots,
            "/home/pi/printer_data/config/a.cfg",
            "/home/pi/printer_data/config/printer.cfg"
        )
        .await
        .is_err());
        rename(
            &fs,
            &roots,
            "/home/pi/printer_data/config/a.cfg",
            "/home/pi/printer_data/config/b.cfg",
        )
        .await
        .expect("rename");
        assert_eq!(
            fs.get("/home/pi/printer_data/config/b.cfg").as_deref(),
            Some("a")
        );
    }

    #[tokio::test]
    async fn list_sorts_dirs_first() {
        let (fs, _) = setup();
        fs.file("/home/pi/printer_data/a.txt", "");
        let names: Vec<String> = list(&fs, "/home/pi/printer_data")
            .await
            .expect("list")
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["config", "gcodes", "logs", "a.txt"]);
    }

    #[test]
    fn klipper_cfg_detection() {
        let r = Roots::from_home(HOME);
        assert!(is_klipper_cfg(
            &r,
            "/home/pi/printer_data/config/printer.cfg"
        ));
        assert!(is_klipper_cfg(
            &r,
            "/home/pi/printer_data/config/macros/m.cfg"
        ));
        assert!(!is_klipper_cfg(
            &r,
            "/home/pi/printer_data/config/moonraker.conf"
        ));
        assert!(!is_klipper_cfg(&r, "/home/pi/printer_data/gcodes/x.cfg"));
    }
}
