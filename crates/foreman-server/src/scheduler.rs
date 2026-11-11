//! Queue scheduler (task 3.5).
//!
//! One loop decides what prints next. It wakes on events (a printer went idle or came
//! online, a plate was cleared, an item was added or moved) and every 30 s.
//!
//! - A printer's own queue goes first: a free printer takes its head item when the
//!   compatibility engine says Ready.
//! - A free printer with an empty queue can take fleet items. Each fleet item, in
//!   order, goes to the Ready printer with the best loaded-filament match
//!   ([`compat::filament_score`]), then by printer sort order.
//! - Free = `auto_start_from_queue` on, no sending/printing item, and Ready (online,
//!   idle, plate clear, compatible). Ready includes the plate gate: a finished print
//!   sets `plate_clear` false and raises a `print_finished` Attention item; its
//!   "Plate cleared" action sets it true again and wakes the loop.
//! - Ticks run under one lock and claim items in the DB (waiting → sending) before
//!   dispatching, so one printer never gets two jobs.
//!
//! Every decision is logged at debug (`skipped <printer>: <verdict>`).

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::Utc;
use foreman_core::{
    compat::{self, PrinterSnapshot, Verdict},
    domain::{FileMeta, JobEnd, QueueItem, QueueStatus},
    error::AppError,
    events::{
        AttentionItemEvent, ConnectionEventKind, PrintEnd, PrinterState, PrinterStatus, ServerEvent,
    },
    library::LibraryStore,
    repo,
};
use sqlx::SqlitePool;
use tokio::{
    sync::{broadcast, Mutex, Notify},
    task::JoinHandle,
};
use tracing::{debug, info, warn};

use crate::{driver_manager::DriverManager, job_engine};

/// Attention kind raised when a print ends; its action is "Plate cleared".
pub const PLATE_ATTENTION: &str = "print_finished";

/// A printing item whose printer stays idle this long without ever being seen
/// printing is marked failed.
const NEVER_STARTED_AFTER: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------
// Environment (live printers, or mocks in tests)
// ---------------------------------------------------------------------------

/// What the scheduler needs from the outside world.
#[async_trait]
pub trait SchedEnv: Send + Sync + 'static {
    /// Every printer with its capabilities and last status (sorted by sort order).
    async fn snapshots(&self) -> Vec<PrinterSnapshot>;
    /// Parsed facts of a Library file.
    async fn file_meta(&self, file_id: i64) -> Result<FileMeta, AppError>;
    /// Copy the Library file to the printer; returns the name to start.
    async fn upload(&self, printer_id: i64, file_id: i64) -> Result<String, AppError>;
    /// Start printing a file already on the printer.
    async fn start(&self, printer_id: i64, name: &str) -> Result<(), AppError>;
    /// Take `grams` off a Spoolman spool (Foreman-reported usage, 3.6).
    async fn deduct(&self, spool_id: i64, grams: f64) -> Result<(), AppError>;
    /// Clear Moonraker's active spool (its `[spoolman]` link) on a printer, so it
    /// stops counting extrusion against one spool. A Moonraker server call, not a
    /// printer command.
    async fn release_active_spool(&self, _printer_id: i64) -> Result<(), AppError> {
        Ok(())
    }
    /// Thumbnail URL of a Library file, if it has one.
    async fn thumb_url(&self, _file_id: i64) -> Option<String> {
        None
    }
}

/// The real environment: drivers + Library.
pub struct LiveEnv {
    pub pool: SqlitePool,
    pub drivers: Arc<DriverManager>,
    pub library: Arc<LibraryStore>,
    /// The built-in Spoolman, once it has started (None while disabled).
    pub spoolman: Arc<std::sync::OnceLock<spoolman_rs::state::AppState>>,
}

#[async_trait]
impl SchedEnv for LiveEnv {
    async fn snapshots(&self) -> Vec<PrinterSnapshot> {
        let printers = repo::printer::list(&self.pool).await.unwrap_or_default();
        let mut snaps = Vec::with_capacity(printers.len());
        for p in printers {
            snaps.push(crate::spools::snapshot(&self.drivers, self.spoolman.get(), p).await);
        }
        snaps
    }

    async fn file_meta(&self, file_id: i64) -> Result<FileMeta, AppError> {
        self.library.details(file_id).await?.meta.ok_or_else(|| {
            AppError::InvalidInput("file has no parsed metadata (not a printable file?)".into())
        })
    }

    async fn upload(&self, printer_id: i64, file_id: i64) -> Result<String, AppError> {
        let entry = self.library.get(file_id).await?;
        let abs = self.library.file_path(file_id).await?;
        let (_name, storage) = self
            .drivers
            .storage(printer_id)
            .await
            .ok_or_else(|| AppError::Driver("printer has no accessible storage".into()))?;
        let progress = Arc::new(std::sync::atomic::AtomicU64::new(0));
        storage
            .upload(&entry.name, &abs, progress)
            .await
            .map_err(|e| AppError::Driver(format!("upload failed: {e}")))?;
        Ok(entry.name)
    }

    async fn start(&self, printer_id: i64, name: &str) -> Result<(), AppError> {
        let driver = self
            .drivers
            .get_driver(printer_id)
            .await
            .ok_or_else(|| AppError::Driver("no driver for printer".into()))?;
        driver
            .start_print(name)
            .await
            .map_err(|e| AppError::Driver(format!("start_print failed: {e}")))
    }

    async fn deduct(&self, spool_id: i64, grams: f64) -> Result<(), AppError> {
        let sm = self
            .spoolman
            .get()
            .ok_or_else(|| AppError::Internal("Spoolman is not running".into()))?;
        spoolman_deduct(sm, spool_id, grams).await
    }

    async fn release_active_spool(&self, printer_id: i64) -> Result<(), AppError> {
        crate::spools::set_slot(&self.drivers, printer_id, 0, None).await
    }

    async fn thumb_url(&self, file_id: i64) -> Option<String> {
        self.library
            .get(file_id)
            .await
            .ok()
            .and_then(|e| e.thumb_url)
    }
}

/// Take `grams` off a spool in the built-in Spoolman and tell its websocket
/// clients (Spoolman UI, Moonraker) the spool changed.
pub async fn spoolman_deduct(
    sm: &spoolman_rs::state::AppState,
    spool_id: i64,
    grams: f64,
) -> Result<(), AppError> {
    let spool = spoolman_rs::db::spool::use_weight(&sm.db.pool, spool_id, grams)
        .await
        .map_err(|e| AppError::Internal(format!("spoolman use_weight: {e}")))?;
    spoolman_rs::ws::broadcast(&sm.event_tx, "updated", "spool", spool.id, &spool);
    Ok(())
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// Result of "start next now" on one printer.
#[derive(Debug, Clone)]
pub struct StartNext {
    pub item_id: i64,
    pub file: String,
    pub dry_run: bool,
    pub message: String,
}

pub struct Scheduler {
    pool: SqlitePool,
    bus_tx: Arc<broadcast::Sender<ServerEvent>>,
    env: Arc<dyn SchedEnv>,
    kick: Notify,
    /// Serialises ticks and manual starts.
    tick_lock: Mutex<()>,
    /// Last printer state seen on the bus (for finished/started transitions).
    last_state: Mutex<HashMap<i64, PrinterState>>,
    /// Printers seen printing since their current queue item was dispatched.
    seen_running: Mutex<HashMap<i64, bool>>,
}

impl Scheduler {
    pub fn new(
        pool: SqlitePool,
        bus_tx: Arc<broadcast::Sender<ServerEvent>>,
        env: Arc<dyn SchedEnv>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            bus_tx,
            env,
            kick: Notify::new(),
            tick_lock: Mutex::new(()),
            last_state: Mutex::new(HashMap::new()),
            seen_running: Mutex::new(HashMap::new()),
        })
    }

    /// Reset interrupted uploads, then run the loop and the event listener.
    pub fn spawn(self: &Arc<Self>) {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            match repo::queue::reset_sending(&me.pool).await {
                Ok(0) => {}
                Ok(n) => info!(n, "queue: interrupted uploads back to waiting"),
                Err(e) => warn!("queue: reset sending items: {e}"),
            }
            loop {
                let _ = me.tick().await;
                tokio::select! {
                    _ = me.kick.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                }
            }
        });

        let me = Arc::clone(self);
        let mut rx = self.bus_tx.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ServerEvent::PrinterStatus { printer_id, status }) => {
                        if me.on_status(printer_id, &status).await {
                            me.kick();
                        }
                    }
                    Ok(ServerEvent::PrinterConnection { event, .. })
                        if event.kind == ConnectionEventKind::Online =>
                    {
                        me.kick()
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("scheduler: event listener lagged by {n}");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// Wake the loop (item added/moved, plate cleared, printer idle).
    pub fn kick(&self) {
        self.kick.notify_one();
    }

    fn changed(&self, printer_id: Option<i64>) {
        let _ = self.bus_tx.send(ServerEvent::QueueChanged { printer_id });
    }

    /// Tell clients a queue changed and wake the loop.
    pub fn queue_changed(&self, printer_id: Option<i64>) {
        self.changed(printer_id);
        self.kick();
    }

    /// One scheduling pass. Returns the dispatch tasks it started (tests await them).
    pub async fn tick(self: &Arc<Self>) -> Vec<JoinHandle<()>> {
        let _guard = self.tick_lock.lock().await;
        let mut started = Vec::new();

        let board = match repo::queue::list_board(&self.pool).await {
            Ok(b) => b,
            Err(e) => {
                warn!("scheduler: list queue: {e}");
                return started;
            }
        };
        let snaps = self.env.snapshots().await;
        self.fail_never_started(&board, &snaps).await;

        let waiting = |lane: Option<i64>| -> Vec<&QueueItem> {
            board
                .iter()
                .filter(|i| i.status == QueueStatus::Waiting && i.printer_id == lane)
                .collect()
        };
        let fleet = waiting(None);
        if board.iter().all(|i| i.status != QueueStatus::Waiting) {
            return started;
        }

        let mut meta_cache: HashMap<i64, Option<FileMeta>> = HashMap::new();
        let mut free: Vec<&PrinterSnapshot> = Vec::new();

        // 1. Printer queues: each free printer takes its own head item.
        for snap in &snaps {
            let p = &snap.printer;
            if !p.auto_start_from_queue {
                debug!(printer = %p.name, "skipped {}: auto_start_from_queue is off", p.name);
                continue;
            }
            if let Some(a) = board
                .iter()
                .find(|i| i.status.is_active() && i.printer_id == Some(p.id))
            {
                debug!(
                    printer = %p.name,
                    "skipped {}: already has a job ({} is {})",
                    p.name,
                    a.file_name,
                    a.status.as_str()
                );
                continue;
            }
            let own = waiting(Some(p.id));
            let Some(head) = own.first() else {
                free.push(snap);
                continue;
            };
            let Some(meta) = self.meta(&mut meta_cache, head).await else {
                continue;
            };
            let v = compat::check(&meta, snap);
            if v.verdict == Verdict::Ready {
                debug!(item = head.id, printer = %p.name, "dispatch {} → {} (printer queue head)", head.file_name, p.name);
                if let Some(h) = self.claim_and_spawn(head, p.id).await {
                    started.push(h);
                }
            } else {
                debug!(
                    item = head.id,
                    "skipped {}: {:?} ({}) — waits for its own queue head {}",
                    p.name,
                    v.verdict,
                    v.reason,
                    head.file_name
                );
            }
        }

        // 2. Fleet queue: next item → best Ready free printer.
        for item in fleet {
            if free.is_empty() {
                debug!(item = item.id, "fleet {}: no free printer", item.file_name);
                break;
            }
            let Some(meta) = self.meta(&mut meta_cache, item).await else {
                continue;
            };
            let mut best: Option<(usize, u32)> = None;
            for (idx, snap) in free.iter().enumerate() {
                let v = compat::check(&meta, snap);
                if v.verdict != Verdict::Ready {
                    debug!(
                        item = item.id,
                        "skipped {}: {:?} ({}) for fleet item {}",
                        snap.printer.name,
                        v.verdict,
                        v.reason,
                        item.file_name
                    );
                    continue;
                }
                let score = compat::filament_score(&meta, snap);
                debug!(
                    item = item.id,
                    "candidate {}: ready, filament score {score}", snap.printer.name
                );
                // `free` is in sort order, so the first best score wins ties.
                if best.is_none_or(|(_, s)| score > s) {
                    best = Some((idx, score));
                }
            }
            let Some((idx, score)) = best else {
                debug!(item = item.id, "fleet {}: no ready printer", item.file_name);
                continue;
            };
            let snap = free.remove(idx);
            debug!(
                item = item.id,
                "dispatch {} → {} (fleet, filament score {score})",
                item.file_name,
                snap.printer.name
            );
            if let Some(h) = self.claim_and_spawn(item, snap.printer.id).await {
                started.push(h);
            }
        }
        started
    }

    async fn meta(
        &self,
        cache: &mut HashMap<i64, Option<FileMeta>>,
        item: &QueueItem,
    ) -> Option<FileMeta> {
        if let Some(m) = cache.get(&item.file_id) {
            return m.clone();
        }
        let m = match self.env.file_meta(item.file_id).await {
            Ok(m) => Some(m),
            Err(e) => {
                warn!(item = item.id, "queue item {}: {e}", item.file_name);
                let msg = format!("file unavailable: {e}");
                if repo::queue::transition(
                    &self.pool,
                    item.id,
                    QueueStatus::Waiting,
                    QueueStatus::Failed,
                    Some(&msg),
                )
                .await
                .unwrap_or(false)
                {
                    self.changed(item.printer_id);
                }
                None
            }
        };
        cache.insert(item.file_id, m.clone());
        m
    }

    async fn claim_and_spawn(
        self: &Arc<Self>,
        item: &QueueItem,
        printer_id: i64,
    ) -> Option<JoinHandle<()>> {
        match repo::queue::claim(&self.pool, item.id, printer_id).await {
            Ok(true) => {}
            Ok(false) => {
                debug!(item = item.id, "claim lost: item changed meanwhile");
                return None;
            }
            Err(e) => {
                warn!(item = item.id, "claim: {e}");
                return None;
            }
        }
        if item.printer_id.is_none() {
            self.changed(None);
        }
        let me = Arc::clone(self);
        let (id, file_id) = (item.id, item.file_id);
        Some(tokio::spawn(async move {
            if let Err(e) = me.dispatch(id, file_id, printer_id).await {
                warn!(item = id, printer_id, "dispatch failed: {e}");
            }
        }))
    }

    /// Upload + start a claimed (sending) item. Stops before starting when the item
    /// was cancelled during the upload.
    async fn dispatch(&self, id: i64, file_id: i64, printer_id: i64) -> Result<(), AppError> {
        self.seen_running.lock().await.insert(printer_id, false);
        self.changed(Some(printer_id));
        let name = match self.env.upload(printer_id, file_id).await {
            Ok(n) => n,
            Err(e) => {
                let msg = e.to_string();
                repo::queue::transition(
                    &self.pool,
                    id,
                    QueueStatus::Sending,
                    QueueStatus::Failed,
                    Some(&msg),
                )
                .await?;
                self.changed(Some(printer_id));
                return Err(e);
            }
        };
        if !repo::queue::transition(
            &self.pool,
            id,
            QueueStatus::Sending,
            QueueStatus::Printing,
            None,
        )
        .await?
        {
            info!(
                item = id,
                printer_id, "queue item cancelled during upload; print not started"
            );
            self.changed(Some(printer_id));
            return Ok(());
        }
        if let Err(e) = self.env.start(printer_id, &name).await {
            let msg = e.to_string();
            repo::queue::transition(
                &self.pool,
                id,
                QueueStatus::Printing,
                QueueStatus::Failed,
                Some(&msg),
            )
            .await?;
            self.changed(Some(printer_id));
            return Err(e);
        }
        repo::printer::set_plate_clear(&self.pool, printer_id, false).await?;
        self.job_sent(printer_id, file_id, &name).await;
        info!(item = id, printer_id, file = %name, "queue: print started");
        self.changed(Some(printer_id));
        Ok(())
    }

    /// Mark printing items failed when their printer stayed idle and never printed.
    async fn fail_never_started(&self, board: &[QueueItem], snaps: &[PrinterSnapshot]) {
        let seen = self.seen_running.lock().await.clone();
        for item in board.iter().filter(|i| i.status == QueueStatus::Printing) {
            let Some(pid) = item.printer_id else { continue };
            let idle = snaps.iter().any(|s| {
                s.printer.id == pid
                    && s.status
                        .as_ref()
                        .is_some_and(|st| st.state == PrinterState::Idle)
            });
            let age = chrono::DateTime::parse_from_rfc3339(&item.updated_at)
                .map(|t| Utc::now().signed_duration_since(t))
                .ok()
                .and_then(|d| d.to_std().ok())
                .unwrap_or_default();
            if idle && !seen.get(&pid).copied().unwrap_or(false) && age > NEVER_STARTED_AFTER {
                warn!(
                    item = item.id,
                    printer_id = pid,
                    "printer idle and never seen printing"
                );
                let _ = repo::queue::transition(
                    &self.pool,
                    item.id,
                    QueueStatus::Printing,
                    QueueStatus::Failed,
                    Some("printer stayed idle; the print did not start (or ended unseen)"),
                )
                .await;
                self.changed(Some(pid));
            }
        }
        // Jobs Foreman started that never ran (Print now or queue).
        for snap in snaps {
            let pid = snap.printer.id;
            let idle = snap
                .status
                .as_ref()
                .is_some_and(|st| st.state == PrinterState::Idle);
            if !idle || seen.get(&pid).copied().unwrap_or(true) {
                continue;
            }
            let Ok(Some(job)) = repo::job::running_for_printer(&self.pool, pid).await else {
                continue;
            };
            let age = chrono::DateTime::parse_from_rfc3339(&job.started_at)
                .map(|t| Utc::now().signed_duration_since(t))
                .ok()
                .and_then(|d| d.to_std().ok())
                .unwrap_or_default();
            if age > NEVER_STARTED_AFTER {
                let st = PrinterStatus {
                    state: PrinterState::Idle,
                    print_end: Some(PrintEnd::Error),
                    print_message: Some("printer stayed idle; the print did not start".into()),
                    progress: Some(0.0),
                    ..Default::default()
                };
                let _ =
                    job_engine::job_ended(&self.pool, self.env.as_ref(), &self.bus_tx, pid, &st)
                        .await;
            }
        }
    }

    /// Track printer state. A print that ends (printing/paused → idle or error)
    /// finishes its queue item, closes the plate gate and raises Attention.
    /// Returns true when the loop should run (printer became idle / online).
    pub async fn on_status(&self, printer_id: i64, status: &PrinterStatus) -> bool {
        let prev = self
            .last_state
            .lock()
            .await
            .insert(printer_id, status.state.clone());
        let was_running = matches!(prev, Some(PrinterState::Printing | PrinterState::Paused));
        // First status after a restart, or back from offline: a job may have ended
        // while Foreman wasn't watching.
        let unseen = matches!(prev, None | Some(PrinterState::Offline));
        match status.state {
            PrinterState::Printing | PrinterState::Paused => {
                self.seen_running.lock().await.insert(printer_id, true);
                if !was_running {
                    self.print_started(printer_id, status).await;
                }
                false
            }
            PrinterState::Idle | PrinterState::Error
                if was_running || (unseen && self.has_running_job(printer_id).await) =>
            {
                self.print_ended(printer_id, status).await;
                status.state == PrinterState::Idle
            }
            PrinterState::Idle => prev != Some(PrinterState::Idle),
            _ => false,
        }
    }

    async fn has_running_job(&self, printer_id: i64) -> bool {
        repo::job::running_for_printer(&self.pool, printer_id)
            .await
            .ok()
            .flatten()
            .is_some()
    }

    /// A print began (sent by Foreman or started on the printer): open its job
    /// unless one is already running (restart / back online mid-print).
    async fn print_started(&self, printer_id: i64, status: &PrinterStatus) {
        let item = repo::queue::active_for_printer(&self.pool, printer_id)
            .await
            .ok()
            .flatten()
            .filter(|i| i.status == QueueStatus::Printing);
        if let Err(e) = job_engine::job_started(
            &self.pool,
            self.env.as_ref(),
            &self.bus_tx,
            printer_id,
            None,
            status.filename.as_deref(),
            item.as_ref(),
        )
        .await
        {
            warn!(printer_id, "job start: {e}");
        }
    }

    /// Print now is about to send a start command: watch whether the printer
    /// actually starts (a job that never runs is closed as an error).
    pub async fn expect_start(&self, printer_id: i64) {
        self.seen_running.lock().await.insert(printer_id, false);
    }

    /// Foreman just sent the start command: open the job with the Library file.
    pub async fn job_sent(&self, printer_id: i64, file_id: i64, name: &str) {
        let item = repo::queue::active_for_printer(&self.pool, printer_id)
            .await
            .ok()
            .flatten()
            .filter(|i| i.status == QueueStatus::Printing);
        if let Err(e) = job_engine::job_started(
            &self.pool,
            self.env.as_ref(),
            &self.bus_tx,
            printer_id,
            Some(file_id),
            Some(name),
            item.as_ref(),
        )
        .await
        {
            warn!(printer_id, "job start: {e}");
        }
    }

    async fn print_ended(&self, printer_id: i64, status: &PrinterStatus) {
        let end = job_engine::end_of(status);
        let item = repo::queue::active_for_printer(&self.pool, printer_id)
            .await
            .ok()
            .flatten()
            .filter(|i| i.status == QueueStatus::Printing);
        if let Some(item) = &item {
            let (to, err) = match end {
                JobEnd::Cancelled => (QueueStatus::Cancelled, None),
                JobEnd::Error => (QueueStatus::Failed, Some("printer reported an error")),
                _ => (QueueStatus::Done, None),
            };
            let _ =
                repo::queue::transition(&self.pool, item.id, QueueStatus::Printing, to, err).await;
            info!(
                item = item.id,
                printer_id,
                end = end.as_str(),
                "queue item finished"
            );
        }
        let job = match job_engine::job_ended(
            &self.pool,
            self.env.as_ref(),
            &self.bus_tx,
            printer_id,
            status,
        )
        .await
        {
            Ok(j) => j,
            Err(e) => {
                warn!(printer_id, "job end: {e}");
                None
            }
        };
        // W9: a finished print counts for its project plate (and may close the project).
        if let Some(item) = &item {
            let printer_name = repo::printer::get(&self.pool, printer_id)
                .await
                .map(|p| p.name)
                .unwrap_or_else(|_| format!("printer {printer_id}"));
            if let Err(e) = crate::projects::print_ended(
                &self.pool,
                item,
                end,
                job.as_ref().map(|j| j.id),
                &printer_name,
            )
            .await
            {
                warn!(item = item.id, "project plate update: {e}");
            }
        }
        if let Err(e) = repo::printer::set_plate_clear(&self.pool, printer_id, false).await {
            warn!(printer_id, "set plate_clear false: {e}");
        }
        self.raise_plate_attention(
            printer_id,
            status,
            item.as_ref(),
            end,
            job.as_ref().map(|j| j.id),
        )
        .await;
        self.changed(Some(printer_id));
    }

    async fn raise_plate_attention(
        &self,
        printer_id: i64,
        status: &PrinterStatus,
        item: Option<&QueueItem>,
        end: JobEnd,
        job_id: Option<i64>,
    ) {
        match repo::attention::get_active_by_printer_kind(&self.pool, printer_id, PLATE_ATTENTION)
            .await
        {
            Ok(Some(a)) => {
                // Still uncleared from an earlier print: point it at the newest job.
                if let Some(j) = job_id {
                    let _ = repo::attention::set_job(&self.pool, a.id, j).await;
                }
                return;
            }
            Ok(None) => {}
            Err(e) => {
                warn!(printer_id, "plate attention lookup: {e}");
                return;
            }
        }
        let name = repo::printer::get(&self.pool, printer_id)
            .await
            .map(|p| p.name)
            .unwrap_or_else(|_| format!("Printer {printer_id}"));
        let title = match end {
            JobEnd::Cancelled => format!("{name}: print cancelled — clear the plate"),
            JobEnd::Error => format!("{name}: print stopped with an error — clear the plate"),
            _ => format!("{name}: print finished — clear the plate"),
        };
        let file = status
            .filename
            .clone()
            .or_else(|| item.map(|i| i.file_name.clone()));
        let body = file.map(|f| format!("{f} · the queue waits until the plate is cleared"));
        match repo::attention::raise(
            &self.pool,
            Some(printer_id),
            PLATE_ATTENTION,
            &title,
            body.as_deref(),
        )
        .await
        {
            Ok(raised) => {
                let created = raised.created;
                let a = raised.item;
                if let Some(j) = job_id {
                    let _ = repo::attention::set_job(&self.pool, a.id, j).await;
                }
                let _ = self.bus_tx.send(ServerEvent::attention_raised(
                    created,
                    AttentionItemEvent {
                        id: a.id,
                        printer_id: Some(printer_id),
                        kind: PLATE_ATTENTION.into(),
                        title: a.title,
                        body: a.body,
                    },
                ));
            }
            Err(e) => warn!(printer_id, "raise plate attention: {e}"),
        }
    }

    /// Hold the scheduling lock (Print now holds it so no tick dispatches meanwhile).
    pub async fn hold(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.tick_lock.lock().await
    }

    /// "Plate cleared": open the plate gate, close the Attention item, run the loop.
    pub async fn plate_cleared(&self, printer_id: i64) -> Result<(), AppError> {
        repo::printer::get(&self.pool, printer_id).await?;
        repo::printer::set_plate_clear(&self.pool, printer_id, true).await?;
        self.clear_plate_attention(printer_id).await?;
        info!(printer_id, "plate cleared");
        self.queue_changed(Some(printer_id));
        Ok(())
    }

    /// Close the printer's open "clear the plate" Attention items.
    pub async fn clear_plate_attention(&self, printer_id: i64) -> Result<(), AppError> {
        while let Some(a) =
            repo::attention::get_active_by_printer_kind(&self.pool, printer_id, PLATE_ATTENTION)
                .await?
        {
            repo::attention::dismiss(&self.pool, a.id).await?;
            let _ = self.bus_tx.send(ServerEvent::AttentionCleared {
                item_id: a.id,
                printer_id: Some(printer_id),
            });
        }
        Ok(())
    }

    /// "Start next now" on one printer: its queue head, else the first fleet item it
    /// can print. Ignores `auto_start_from_queue`; the plate must be clear or
    /// confirmed. `dry_run` uploads and stops before the start command; the item
    /// stays waiting.
    pub async fn start_next(
        self: &Arc<Self>,
        printer_id: i64,
        plate_confirm: bool,
        dry_run: bool,
    ) -> Result<StartNext, AppError> {
        let _guard = self.tick_lock.lock().await;
        if let Some(a) = repo::queue::active_for_printer(&self.pool, printer_id).await? {
            return Err(AppError::InvalidInput(format!(
                "printer already has a job: {} is {}",
                a.file_name,
                a.status.as_str()
            )));
        }
        let mut snap = self
            .env
            .snapshots()
            .await
            .into_iter()
            .find(|s| s.printer.id == printer_id)
            .ok_or_else(|| AppError::NotFound(format!("printer {printer_id}")))?;
        if plate_confirm {
            snap.printer.plate_clear = true;
        }
        let own = repo::queue::list_waiting(&self.pool, Some(printer_id)).await?;
        let candidates = if own.is_empty() {
            repo::queue::list_waiting(&self.pool, None).await?
        } else {
            own.into_iter().take(1).collect()
        };
        if candidates.is_empty() {
            return Err(AppError::InvalidInput(
                "nothing queued for this printer".into(),
            ));
        }
        let mut last_reason = String::new();
        let mut pick = None;
        for item in candidates {
            let meta = self.env.file_meta(item.file_id).await?;
            let v = compat::check(&meta, &snap);
            if v.verdict == Verdict::Ready {
                pick = Some(item);
                break;
            }
            debug!(
                item = item.id,
                "start next: skipped {}: {:?} ({})", snap.printer.name, v.verdict, v.reason
            );
            last_reason = format!("{}: {}", item.file_name, v.reason);
        }
        let item = pick
            .ok_or_else(|| AppError::InvalidInput(format!("printer not ready ({last_reason})")))?;

        if dry_run {
            let name = self.env.upload(printer_id, item.file_id).await?;
            info!(item = item.id, printer_id, file = %name, "start next dry run: uploaded, start skipped");
            return Ok(StartNext {
                item_id: item.id,
                file: name.clone(),
                dry_run: true,
                message: format!(
                    "Dry run: uploaded {name} to {}; would start it (start command not sent). It stays queued.",
                    snap.printer.name
                ),
            });
        }
        if plate_confirm {
            repo::printer::set_plate_clear(&self.pool, printer_id, true).await?;
        }
        if !repo::queue::claim(&self.pool, item.id, printer_id).await? {
            return Err(AppError::InvalidInput(
                "item changed meanwhile; try again".into(),
            ));
        }
        if item.printer_id.is_none() {
            self.changed(None);
        }
        self.dispatch(item.id, item.file_id, printer_id).await?;
        Ok(StartNext {
            item_id: item.id,
            file: item.file_name.clone(),
            dry_run: false,
            message: format!("Printing {} on {}", item.file_name, snap.printer.name),
        })
    }
}
