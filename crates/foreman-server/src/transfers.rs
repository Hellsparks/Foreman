//! Background copy/move between Library locations (task 3.2): the Library and each
//! Klipper printer's G-code storage. Bytes always stream through this server;
//! printer→printer goes printer → Foreman temp file → printer.
//!
//! Rules: conflicts are resolved per item (keep both = ` (copy)`, replace, skip; `ask`
//! returns them to the caller first). A file that is printing is never replaced, moved
//! or deleted. STL files are refused for printers ("slice first"). Library files that
//! get replaced or moved away go to the trash, so they can be restored.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use async_trait::async_trait;
use foreman_core::{
    domain::LibraryEntryKind,
    error::AppError,
    events::{ServerEvent, TransferInfo, TransferState},
    library::LibraryStore,
    storage::{clean_path, covers, join, name_of, parent_of, PrinterStorage},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Semaphore};
use tracing::{info, warn};

use crate::driver_manager::DriverManager;

/// Transfers that run at the same time; the rest wait as `queued`.
const PARALLEL: usize = 2;
/// Finished transfers kept for the list.
const KEEP_FINISHED: usize = 100;
const PROGRESS_EVERY: Duration = Duration::from_millis(300);

#[async_trait]
pub trait StorageResolver: Send + Sync {
    /// Printer name and its G-code storage (`None` = unknown or not a Klipper printer).
    async fn storage(&self, printer_id: i64) -> Option<(String, Arc<dyn PrinterStorage>)>;
}

#[async_trait]
impl StorageResolver for DriverManager {
    async fn storage(&self, printer_id: i64) -> Option<(String, Arc<dyn PrinterStorage>)> {
        DriverManager::storage(self, printer_id).await
    }
}

// ---------------------------------------------------------------------------
// Request / response
// ---------------------------------------------------------------------------

/// A file or folder in a location (for a destination: a folder).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LocRef {
    Library { path: String },
    Printer { printer_id: i64, path: String },
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OnConflict {
    #[default]
    Ask,
    KeepBoth,
    Replace,
    Skip,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Copy,
    Move,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TransferItem {
    pub src: LocRef,
    /// Overrides the request-wide policy for this item.
    pub on_conflict: Option<OnConflict>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TransferRequest {
    pub items: Vec<TransferItem>,
    pub dest: LocRef,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub on_conflict: OnConflict,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConflictInfo {
    /// Index into `items`.
    pub index: usize,
    pub name: String,
    pub is_dir: bool,
    /// The same-named item at the destination is being printed: Replace is not allowed.
    pub dest_printing: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Blocked {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum StartOutcome {
    /// Nothing started; resend with `on_conflict` set for these items.
    Conflicts { conflicts: Vec<ConflictInfo> },
    Started {
        transfers: Vec<TransferInfo>,
        skipped: Vec<String>,
        blocked: Vec<Blocked>,
    },
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Side {
    Library,
    Printer {
        id: i64,
        name: String,
        storage: Arc<dyn PrinterStorage>,
    },
}

impl Side {
    fn label(&self) -> &str {
        match self {
            Side::Library => "Library",
            Side::Printer { name, .. } => name,
        }
    }

    fn same(&self, other: &Side) -> bool {
        match (self, other) {
            (Side::Library, Side::Library) => true,
            (Side::Printer { id: a, .. }, Side::Printer { id: b, .. }) => a == b,
            _ => false,
        }
    }

    fn is_printer(&self) -> bool {
        matches!(self, Side::Printer { .. })
    }
}

struct Plan {
    src: Side,
    src_path: String,
    src_is_dir: bool,
    dest: Side,
    dest_path: String,
    replace: bool,
    mode: Mode,
}

struct Job {
    info: TransferInfo,
    progress: Arc<AtomicU64>,
    abort: Option<tokio::task::AbortHandle>,
}

/// A child of a walked folder: path relative to the folder, kind, size.
struct Node {
    rel: String,
    is_dir: bool,
    size: u64,
}

/// Removes a temp file when dropped (also when a cancelled task is aborted).
struct TmpFile(PathBuf);

impl Drop for TmpFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub struct Transfers {
    library: Arc<LibraryStore>,
    resolver: Arc<dyn StorageResolver>,
    bus: Arc<broadcast::Sender<ServerEvent>>,
    jobs: Mutex<Vec<Job>>,
    permits: Arc<Semaphore>,
}

pub fn is_stl(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".stl")
}

/// `part.gcode` → `part (copy).gcode`, then `part (copy 2).gcode`… (first one not in `taken`).
pub fn copy_name(name: &str, is_dir: bool, taken: &HashSet<String>) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && !is_dir => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1..)
        .map(|n| {
            if n == 1 {
                format!("{stem} (copy){ext}")
            } else {
                format!("{stem} (copy {n}){ext}")
            }
        })
        .find(|c| !taken.contains(&c.to_lowercase()))
        .unwrap_or_else(|| format!("{stem} (copy){ext}"))
}

fn route(src: &Side, dest: &Side) -> String {
    if src.same(dest) {
        return match src {
            Side::Library => "Library · on the server".into(),
            Side::Printer { name, .. } => format!("{name} · on the printer"),
        };
    }
    if src.is_printer() && dest.is_printer() {
        return format!("{} → Foreman → {}", src.label(), dest.label());
    }
    format!("{} → {}", src.label(), dest.label())
}

impl Transfers {
    pub fn new(
        library: Arc<LibraryStore>,
        resolver: Arc<dyn StorageResolver>,
        bus: Arc<broadcast::Sender<ServerEvent>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            library,
            resolver,
            bus,
            jobs: Mutex::new(Vec::new()),
            permits: Arc::new(Semaphore::new(PARALLEL)),
        })
    }

    async fn side(&self, printer_id: i64) -> Result<Side, AppError> {
        let (name, storage) = self.resolver.storage(printer_id).await.ok_or_else(|| {
            AppError::NotFound(format!(
                "printer {printer_id} has no browsable storage (Klipper printers only)"
            ))
        })?;
        Ok(Side::Printer {
            id: printer_id,
            name,
            storage,
        })
    }

    async fn resolve(&self, loc: &LocRef) -> Result<(Side, String), AppError> {
        match loc {
            LocRef::Library { path } => Ok((
                Side::Library,
                foreman_core::library::store::clean_rel(path)?,
            )),
            LocRef::Printer { printer_id, path } => {
                Ok((self.side(*printer_id).await?, clean_path(path)?))
            }
        }
    }

    /// `(name, is_dir, size)` of the entries in `dir`.
    async fn children(&self, side: &Side, dir: &str) -> Result<Vec<(String, bool, u64)>, AppError> {
        Ok(match side {
            Side::Library => self
                .library
                .list(dir)
                .await?
                .into_iter()
                .map(|e| {
                    (
                        e.name,
                        e.kind == LibraryEntryKind::Folder,
                        e.size.max(0) as u64,
                    )
                })
                .collect(),
            Side::Printer { storage, .. } => storage
                .list_dir(dir)
                .await?
                .into_iter()
                .map(|e| (e.name, e.is_dir, e.size))
                .collect(),
        })
    }

    async fn active(&self, side: &Side) -> Option<String> {
        let Side::Printer { storage, id, .. } = side else {
            return None;
        };
        match storage.active_file().await {
            Ok(a) => a,
            Err(e) => {
                warn!(printer_id = id, "could not read the printing file: {e}");
                None
            }
        }
    }

    /// Check every item, resolve conflicts, and start the transfers.
    pub async fn start(self: &Arc<Self>, req: TransferRequest) -> Result<StartOutcome, AppError> {
        if req.items.is_empty() {
            return Err(AppError::InvalidInput("nothing to transfer".into()));
        }
        let (dest, dest_folder) = self.resolve(&req.dest).await?;
        if let Side::Library = dest {
            if !dest_folder.is_empty() {
                match self.library.find(&dest_folder).await? {
                    Some(e) if e.kind == LibraryEntryKind::Folder => {}
                    _ => {
                        return Err(AppError::InvalidInput(format!(
                            "{dest_folder} is not a Library folder"
                        )))
                    }
                }
            }
        }
        let existing = self.children(&dest, &dest_folder).await?;
        let mut taken: HashSet<String> =
            existing.iter().map(|(n, _, _)| n.to_lowercase()).collect();
        let dest_active = self.active(&dest).await;

        let mut conflicts = Vec::new();
        let mut skipped = Vec::new();
        let mut blocked = Vec::new();
        let mut plans = Vec::new();
        let mut src_active: Option<(i64, Option<String>)> = None;

        for (index, item) in req.items.iter().enumerate() {
            let (src, src_path) = match self.resolve(&item.src).await {
                Ok(v) => v,
                Err(e) => {
                    blocked.push(Blocked {
                        name: format!("item {}", index + 1),
                        reason: e.to_string(),
                    });
                    continue;
                }
            };
            let name = name_of(&src_path).to_owned();
            let block = |reason: &str| Blocked {
                name: name.clone(),
                reason: reason.to_owned(),
            };
            if src_path.is_empty() {
                blocked.push(block("can't copy a whole location"));
                continue;
            }
            let src_is_dir = match &src {
                Side::Library => match self.library.find(&src_path).await? {
                    Some(e) => e.kind == LibraryEntryKind::Folder,
                    None => {
                        blocked.push(block("not found"));
                        continue;
                    }
                },
                Side::Printer { .. } => {
                    let siblings = self.children(&src, parent_of(&src_path)).await?;
                    match siblings.iter().find(|(n, _, _)| *n == name) {
                        Some((_, d, _)) => *d,
                        None => {
                            blocked.push(block("not found"));
                            continue;
                        }
                    }
                }
            };
            let same = src.same(&dest);
            if same && parent_of(&src_path) == dest_folder && req.mode == Mode::Move {
                skipped.push(name);
                continue;
            }
            if same && src_is_dir && covers(&src_path, &dest_folder) {
                blocked.push(block("can't copy a folder into itself"));
                continue;
            }
            if dest.is_printer() && !src_is_dir && is_stl(&name) {
                blocked.push(block("STL files can't be printed: slice first"));
                continue;
            }
            if req.mode == Mode::Move {
                if let Side::Printer { id, .. } = &src {
                    if src_active.as_ref().map(|(i, _)| *i) != Some(*id) {
                        src_active = Some((*id, self.active(&src).await));
                    }
                    if let Some((_, Some(a))) = &src_active {
                        if covers(&src_path, a) {
                            blocked.push(block("printing now, not moved"));
                            continue;
                        }
                    }
                }
            }

            let mut dest_name = name.clone();
            let mut replace = false;
            let clash = existing
                .iter()
                .find(|(n, _, _)| n.eq_ignore_ascii_case(&name))
                .cloned();
            let clash = clash.or_else(|| {
                taken
                    .contains(&name.to_lowercase())
                    .then(|| (name.clone(), src_is_dir, 0))
            });
            if let Some((clash_name, clash_dir, _)) = clash {
                let clash_path = join(&dest_folder, &clash_name);
                let dest_printing = dest_active
                    .as_deref()
                    .is_some_and(|a| covers(&clash_path, a));
                match item.on_conflict.unwrap_or(req.on_conflict) {
                    OnConflict::Ask => {
                        conflicts.push(ConflictInfo {
                            index,
                            name: name.clone(),
                            is_dir: src_is_dir,
                            dest_printing,
                        });
                        continue;
                    }
                    OnConflict::Skip => {
                        skipped.push(name);
                        continue;
                    }
                    OnConflict::KeepBoth => dest_name = copy_name(&name, src_is_dir, &taken),
                    OnConflict::Replace => {
                        if dest_printing {
                            blocked.push(block("printing now, not replaced"));
                            continue;
                        }
                        if clash_dir != src_is_dir {
                            blocked.push(block(if clash_dir {
                                "a folder with that name is there"
                            } else {
                                "a file with that name is there"
                            }));
                            continue;
                        }
                        if same && join(&dest_folder, &clash_name) == src_path {
                            blocked.push(block("can't replace a file with itself"));
                            continue;
                        }
                        if same && src_is_dir {
                            blocked.push(block("folder already here: pick Keep both"));
                            continue;
                        }
                        dest_name = clash_name;
                        replace = true;
                    }
                }
            }
            taken.insert(dest_name.to_lowercase());
            plans.push(Plan {
                src,
                src_path,
                src_is_dir,
                dest: dest.clone(),
                dest_path: join(&dest_folder, &dest_name),
                replace,
                mode: req.mode,
            });
        }

        if !conflicts.is_empty() {
            return Ok(StartOutcome::Conflicts { conflicts });
        }
        let transfers = plans.into_iter().map(|p| self.spawn(p)).collect();
        Ok(StartOutcome::Started {
            transfers,
            skipped,
            blocked,
        })
    }

    fn spawn(self: &Arc<Self>, plan: Plan) -> TransferInfo {
        let name = name_of(&plan.src_path).to_owned();
        let info = TransferInfo {
            id: uuid::Uuid::new_v4().to_string(),
            name: if plan.src_is_dir {
                format!("{name}/")
            } else {
                name
            },
            mode: match plan.mode {
                Mode::Copy => "copy".into(),
                Mode::Move => "move".into(),
            },
            route: route(&plan.src, &plan.dest),
            dest_path: plan.dest_path.clone(),
            bytes_done: 0,
            bytes_total: 0,
            state: TransferState::Queued,
            error: None,
            note: None,
            created_at: chrono::Utc::now(),
        };
        let progress = Arc::new(AtomicU64::new(0));
        {
            let mut jobs = self.lock();
            jobs.push(Job {
                info: info.clone(),
                progress: progress.clone(),
                abort: None,
            });
            let finished = jobs.iter().filter(|j| j.info.state.is_finished()).count();
            if finished > KEEP_FINISHED {
                let mut drop_n = finished - KEEP_FINISHED;
                jobs.retain(|j| {
                    if drop_n > 0 && j.info.state.is_finished() {
                        drop_n -= 1;
                        false
                    } else {
                        true
                    }
                });
            }
        }
        let _ = self.bus.send(ServerEvent::TransferStarted {
            transfer: info.clone(),
        });
        let me = Arc::clone(self);
        let id = info.id.clone();
        let handle = tokio::spawn(async move { me.run(id, plan, progress).await });
        if let Some(j) = self.lock().iter_mut().find(|j| j.info.id == info.id) {
            j.abort = Some(handle.abort_handle());
        }
        info
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Job>> {
        self.jobs.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Update a job and return a snapshot (with live byte count).
    fn update(&self, id: &str, f: impl FnOnce(&mut TransferInfo)) -> Option<TransferInfo> {
        let mut jobs = self.lock();
        let j = jobs.iter_mut().find(|j| j.info.id == id)?;
        f(&mut j.info);
        j.info.bytes_done = j.progress.load(Ordering::Relaxed);
        Some(j.info.clone())
    }

    async fn run(self: Arc<Self>, id: String, plan: Plan, progress: Arc<AtomicU64>) {
        let Ok(_permit) = self.permits.clone().acquire_owned().await else {
            return;
        };
        if let Some(t) = self.update(&id, |i| i.state = TransferState::Running) {
            let _ = self.bus.send(ServerEvent::TransferProgress { transfer: t });
        }
        let exec = self.execute(&id, &plan, progress);
        tokio::pin!(exec);
        let mut tick = tokio::time::interval(PROGRESS_EVERY);
        tick.tick().await;
        let mut last = u64::MAX;
        let res = loop {
            tokio::select! {
                r = &mut exec => break r,
                _ = tick.tick() => {
                    if let Some(t) = self.update(&id, |_| {}) {
                        if t.bytes_done != last {
                            last = t.bytes_done;
                            let _ = self.bus.send(ServerEvent::TransferProgress { transfer: t });
                        }
                    }
                }
            }
        };
        let finished = self.update(&id, |i| match res {
            Ok(note) => {
                i.state = TransferState::Done;
                i.note = note;
            }
            Err(e) => {
                i.state = TransferState::Failed;
                i.error = Some(e.to_string());
            }
        });
        if let Some(t) = finished {
            match t.state {
                TransferState::Done => {
                    info!(name = %t.name, route = %t.route, bytes = t.bytes_done, "transfer done")
                }
                _ => {
                    warn!(name = %t.name, route = %t.route, error = t.error.as_deref().unwrap_or(""), "transfer failed")
                }
            }
            let _ = self.bus.send(ServerEvent::TransferFinished { transfer: t });
        }
    }

    pub fn list(&self) -> Vec<TransferInfo> {
        let ids: Vec<String> = self.lock().iter().map(|j| j.info.id.clone()).collect();
        ids.iter()
            .filter_map(|id| self.update(id, |_| {}))
            .collect()
    }

    pub fn cancel(&self, id: &str) -> Result<TransferInfo, AppError> {
        let t = {
            let mut jobs = self.lock();
            let j = jobs
                .iter_mut()
                .find(|j| j.info.id == id)
                .ok_or_else(|| AppError::NotFound(format!("transfer {id}")))?;
            if j.info.state.is_finished() {
                return Ok(j.info.clone());
            }
            if let Some(a) = j.abort.take() {
                a.abort();
            }
            j.info.state = TransferState::Cancelled;
            j.info.bytes_done = j.progress.load(Ordering::Relaxed);
            j.info.clone()
        };
        info!(name = %t.name, route = %t.route, "transfer cancelled");
        let _ = self.bus.send(ServerEvent::TransferFinished {
            transfer: t.clone(),
        });
        Ok(t)
    }

    pub fn clear_finished(&self) {
        self.lock().retain(|j| !j.info.state.is_finished());
    }

    fn set_total(&self, id: &str, total: u64) {
        self.update(id, |i| i.bytes_total = total);
    }

    async fn execute(
        &self,
        id: &str,
        p: &Plan,
        progress: Arc<AtomicU64>,
    ) -> Result<Option<String>, AppError> {
        let dest_folder = parent_of(&p.dest_path).to_owned();
        let dest_name = name_of(&p.dest_path).to_owned();

        // Same location: server-side move/copy where possible.
        if p.src.same(&p.dest) {
            match (&p.src, p.mode) {
                (Side::Library, Mode::Move) => {
                    let e = self.lib_entry(&p.src_path).await?;
                    if p.replace {
                        self.trash_lib(&p.dest_path).await?;
                    }
                    self.library.move_to(e.id, &dest_folder, &dest_name).await?;
                    return Ok(None);
                }
                (Side::Printer { storage, .. }, mode) => {
                    if p.replace {
                        self.ensure_not_printing(&p.dest, &p.dest_path).await?;
                    }
                    if mode == Mode::Move {
                        self.ensure_not_printing(&p.src, &p.src_path).await?;
                        storage.move_entry(&p.src_path, &p.dest_path).await?;
                    } else {
                        storage.copy_entry(&p.src_path, &p.dest_path).await?;
                    }
                    return Ok(None);
                }
                (Side::Library, Mode::Copy) => {}
            }
        }

        let legs = if p.src.is_printer() && p.dest.is_printer() {
            2
        } else {
            1
        };
        let mut notes = Vec::new();
        if p.src_is_dir {
            let nodes = self.walk(&p.src, &p.src_path).await?;
            self.set_total(
                id,
                nodes
                    .iter()
                    .filter(|n| !n.is_dir)
                    .map(|n| n.size)
                    .sum::<u64>()
                    * legs,
            );
            self.make_dir(&p.dest, &p.dest_path).await;
            let active = self.active(&p.dest).await;
            let (mut stl, mut printing) = (0, 0);
            for n in &nodes {
                let to = join(&p.dest_path, &n.rel);
                if n.is_dir {
                    self.make_dir(&p.dest, &to).await;
                    continue;
                }
                if p.dest.is_printer() && is_stl(&n.rel) {
                    stl += 1;
                    progress.fetch_add(n.size * legs, Ordering::Relaxed);
                    continue;
                }
                if active.as_deref() == Some(to.as_str()) {
                    printing += 1;
                    progress.fetch_add(n.size * legs, Ordering::Relaxed);
                    continue;
                }
                self.copy_file(
                    &p.src,
                    &join(&p.src_path, &n.rel),
                    &p.dest,
                    &to,
                    true,
                    progress.clone(),
                )
                .await?;
            }
            if stl > 0 {
                notes.push(format!("{stl} STL file(s) skipped: slice first"));
            }
            if printing > 0 {
                notes.push(format!("{printing} file(s) printing now, not replaced"));
            }
        } else {
            let size = self.file_size(&p.src, &p.src_path).await?;
            self.set_total(id, size * legs);
            self.copy_file(
                &p.src,
                &p.src_path,
                &p.dest,
                &p.dest_path,
                p.replace,
                progress,
            )
            .await?;
        }

        if p.mode == Mode::Move {
            match &p.src {
                Side::Library => self.trash_lib(&p.src_path).await?,
                Side::Printer { storage, .. } => {
                    self.ensure_not_printing(&p.src, &p.src_path).await?;
                    if p.src_is_dir {
                        storage.delete_dir(&p.src_path).await?;
                    } else {
                        storage.delete_file(&p.src_path).await?;
                    }
                }
            }
        }
        Ok((!notes.is_empty()).then(|| notes.join("; ")))
    }

    async fn lib_entry(&self, path: &str) -> Result<foreman_core::domain::LibraryEntry, AppError> {
        self.library
            .find(path)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Library: {path}")))
    }

    async fn trash_lib(&self, path: &str) -> Result<(), AppError> {
        if let Some(e) = self.library.find(path).await? {
            self.library.trash(e.id).await?;
        }
        Ok(())
    }

    async fn ensure_not_printing(&self, side: &Side, path: &str) -> Result<(), AppError> {
        match self.active(side).await {
            Some(a) if covers(path, &a) => Err(AppError::InvalidInput(format!(
                "{} is printing now",
                name_of(path)
            ))),
            _ => Ok(()),
        }
    }

    async fn make_dir(&self, side: &Side, path: &str) {
        let res = match side {
            Side::Library => self.library.ensure_folder(path).await,
            // Uploads create missing folders too; this only matters for empty ones.
            Side::Printer { storage, .. } => storage.mkdir(path).await,
        };
        if let Err(e) = res {
            tracing::debug!(%path, "mkdir: {e}");
        }
    }

    async fn file_size(&self, side: &Side, path: &str) -> Result<u64, AppError> {
        let name = name_of(path);
        self.children(side, parent_of(path))
            .await?
            .into_iter()
            .find(|(n, d, _)| n == name && !d)
            .map(|(_, _, s)| s)
            .ok_or_else(|| AppError::NotFound(format!("{} {path}", side.label())))
    }

    /// Every entry under `root` (relative paths, parents before children).
    async fn walk(&self, side: &Side, root: &str) -> Result<Vec<Node>, AppError> {
        let mut out = Vec::new();
        let mut stack = vec![String::new()];
        while let Some(rel) = stack.pop() {
            for (name, is_dir, size) in self.children(side, &join(root, &rel)).await? {
                let child = join(&rel, &name);
                if is_dir {
                    stack.push(child.clone());
                }
                out.push(Node {
                    rel: child,
                    is_dir,
                    size,
                });
            }
        }
        Ok(out)
    }

    async fn copy_file(
        &self,
        src: &Side,
        src_path: &str,
        dest: &Side,
        dest_path: &str,
        replace: bool,
        progress: Arc<AtomicU64>,
    ) -> Result<(), AppError> {
        let tmp_path = || {
            self.library
                .tmp_dir()
                .join(format!(".foreman-xfer-{}", uuid::Uuid::new_v4()))
        };
        // A local file to send: the Library file itself, or a download from the printer.
        let (local, tmp) = match src {
            Side::Library => {
                let e = self.lib_entry(src_path).await?;
                (self.library.file_path(e.id).await?, None)
            }
            Side::Printer { storage, .. } => {
                let t = TmpFile(tmp_path());
                storage.download(src_path, &t.0, progress.clone()).await?;
                (t.0.clone(), Some(t))
            }
        };
        match dest {
            Side::Printer { storage, .. } => {
                if replace {
                    self.ensure_not_printing(dest, dest_path).await?;
                }
                storage.upload(dest_path, &local, progress).await?;
            }
            Side::Library => {
                // add_file moves its input into the Library, so it needs a file of its own.
                let owned = match tmp {
                    Some(t) => t,
                    None => {
                        let t = TmpFile(tmp_path());
                        copy_counted(&local, &t.0, &progress).await?;
                        t
                    }
                };
                if replace {
                    self.trash_lib(dest_path).await?;
                }
                self.library
                    .add_file(parent_of(dest_path), name_of(dest_path), &owned.0)
                    .await?;
            }
        }
        Ok(())
    }

    /// Delete printer files/folders permanently; anything printing is left alone.
    pub async fn delete_on_printer(
        &self,
        printer_id: i64,
        paths: &[String],
    ) -> Result<(Vec<String>, Vec<Blocked>), AppError> {
        let side = self.side(printer_id).await?;
        let Side::Printer { storage, name, .. } = &side else {
            return Err(AppError::Internal("not a printer".into()));
        };
        let active = self.active(&side).await;
        let mut deleted = Vec::new();
        let mut blocked = Vec::new();
        for raw in paths {
            let path = clean_path(raw)?;
            let item = name_of(&path).to_owned();
            if path.is_empty() {
                continue;
            }
            if active.as_deref().is_some_and(|a| covers(&path, a)) {
                blocked.push(Blocked {
                    name: item,
                    reason: "printing now".into(),
                });
                continue;
            }
            let is_dir = self
                .children(&side, parent_of(&path))
                .await?
                .into_iter()
                .find(|(n, _, _)| *n == item)
                .map(|(_, d, _)| d);
            let res = match is_dir {
                None => Err(AppError::NotFound(path.clone())),
                Some(true) => storage.delete_dir(&path).await,
                Some(false) => storage.delete_file(&path).await,
            };
            match res {
                Ok(()) => {
                    info!(printer = %name, %path, "printer file deleted");
                    deleted.push(path);
                }
                Err(e) => blocked.push(Blocked {
                    name: item,
                    reason: e.to_string(),
                }),
            }
        }
        Ok((deleted, blocked))
    }
}

/// Copy a local file in chunks, counting bytes.
async fn copy_counted(
    from: &std::path::Path,
    to: &std::path::Path,
    progress: &AtomicU64,
) -> Result<(), AppError> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let ioe = |e: std::io::Error| AppError::Internal(e.to_string());
    let mut r = tokio::fs::File::open(from).await.map_err(ioe)?;
    let mut w = tokio::fs::File::create(to).await.map_err(ioe)?;
    let mut buf = vec![0u8; 256 << 10];
    loop {
        let n = r.read(&mut buf).await.map_err(ioe)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n]).await.map_err(ioe)?;
        progress.fetch_add(n as u64, Ordering::Relaxed);
    }
    w.flush().await.map_err(ioe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_names() {
        let mut taken = HashSet::new();
        assert_eq!(copy_name("a.gcode", false, &taken), "a (copy).gcode");
        taken.insert("a (copy).gcode".into());
        assert_eq!(copy_name("a.gcode", false, &taken), "a (copy 2).gcode");
        assert_eq!(copy_name("v1.2", true, &taken), "v1.2 (copy)");
        assert_eq!(copy_name(".hidden", false, &taken), ".hidden (copy)");
    }
}
