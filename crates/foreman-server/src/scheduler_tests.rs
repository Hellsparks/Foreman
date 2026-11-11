//! Scheduler against mock printers: fleet pick order, no double dispatch, the plate
//! gate, auto-start off, start next now, reorder/move, cancel while sending.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use foreman_core::{
    compat::PrinterSnapshot,
    db::ForemanDb,
    domain::{FileMeta, QueueSource, QueueStatus, ToolUsage},
    driver::PrinterCapabilities,
    error::AppError,
    events::{AmsTray, PrinterState, PrinterStatus, ServerEvent},
    repo,
};
use tokio::sync::{broadcast, Notify};

use crate::scheduler::{SchedEnv, Scheduler, PLATE_ATTENTION};

// ---------------------------------------------------------------------------
// Mock environment
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Mock {
    status: HashMap<i64, PrinterStatus>,
    files: HashMap<i64, FileMeta>,
    uploads: Vec<(i64, i64)>,
    starts: Vec<(i64, String)>,
    fail_upload: bool,
    caps: HashMap<i64, PrinterCapabilities>,
    deductions: Vec<(i64, f64)>,
}

struct MockEnv {
    pool: sqlx::SqlitePool,
    m: Mutex<Mock>,
    /// When set, uploads wait here until notified.
    gate: Option<Arc<Notify>>,
}

impl MockEnv {
    fn lock(&self) -> std::sync::MutexGuard<'_, Mock> {
        self.m.lock().unwrap_or_else(|p| p.into_inner())
    }
    fn set_state(&self, pid: i64, state: PrinterState) {
        self.lock().status.entry(pid).or_default().state = state;
    }
    fn starts(&self) -> Vec<(i64, String)> {
        self.lock().starts.clone()
    }
}

#[async_trait]
impl SchedEnv for MockEnv {
    async fn snapshots(&self) -> Vec<PrinterSnapshot> {
        let printers = repo::printer::list(&self.pool).await.unwrap_or_default();
        let m = self.lock();
        printers
            .into_iter()
            .map(|p| PrinterSnapshot {
                status: m.status.get(&p.id).cloned(),
                caps: m.caps.get(&p.id).cloned(),
                printer: p,
                loaded: vec![],
            })
            .collect()
    }
    async fn file_meta(&self, file_id: i64) -> Result<FileMeta, AppError> {
        self.lock()
            .files
            .get(&file_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("file {file_id}")))
    }
    async fn upload(&self, printer_id: i64, file_id: i64) -> Result<String, AppError> {
        if let Some(g) = &self.gate {
            g.notified().await;
        }
        let mut m = self.lock();
        if m.fail_upload {
            return Err(AppError::Driver("upload failed: mock".into()));
        }
        m.uploads.push((printer_id, file_id));
        Ok(format!("file{file_id}.gcode"))
    }
    async fn start(&self, printer_id: i64, name: &str) -> Result<(), AppError> {
        self.lock().starts.push((printer_id, name.to_owned()));
        Ok(())
    }
    async fn deduct(&self, spool_id: i64, grams: f64) -> Result<(), AppError> {
        self.lock().deductions.push((spool_id, grams));
        Ok(())
    }
}

fn status(state: PrinterState, trays: Vec<AmsTray>) -> PrinterStatus {
    PrinterStatus {
        state,
        ams_trays: trays,
        ..Default::default()
    }
}

fn tray(material: &str, color: &str) -> AmsTray {
    AmsTray {
        slot: 0,
        color: Some(color.into()),
        material: Some(material.into()),
        remaining: None,
    }
}

fn file(material: &str, colour: &str) -> FileMeta {
    FileMeta {
        format: "gcode".into(),
        parse_status: "ok".into(),
        tools: vec![ToolUsage {
            index: 0,
            material: Some(material.into()),
            colour: Some(colour.into()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

struct Rig {
    sched: Arc<Scheduler>,
    env: Arc<MockEnv>,
    pool: sqlx::SqlitePool,
    rx: broadcast::Receiver<ServerEvent>,
    _tmp: tempfile::TempDir,
}

/// Printers 1 "Alpha" (sort 0) and 2 "Bravo" (sort 1), both idle, plate clear.
async fn rig(gate: Option<Arc<Notify>>) -> Rig {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let db = ForemanDb::connect(&tmp.path().join("s.db"))
        .await
        .expect("db");
    for (id, name) in [(1, "Alpha"), (2, "Bravo")] {
        sqlx::query(
            "INSERT INTO printer (id, name, firmware, tags, sort_order, created_at, updated_at) \
             VALUES (?, ?, 'moonraker', '[]', ?, 'x', 'x')",
        )
        .bind(id)
        .bind(name)
        .bind(id - 1)
        .execute(&db.pool)
        .await
        .expect("printer");
    }
    let env = Arc::new(MockEnv {
        pool: db.pool.clone(),
        m: Mutex::new(Mock::default()),
        gate,
    });
    env.set_state(1, PrinterState::Idle);
    env.set_state(2, PrinterState::Idle);
    let (tx, rx) = broadcast::channel(256);
    let sched = Scheduler::new(db.pool.clone(), Arc::new(tx), env.clone());
    Rig {
        sched,
        env,
        pool: db.pool,
        rx,
        _tmp: tmp,
    }
}

impl Rig {
    fn file(&self, id: i64, meta: FileMeta) {
        self.env.lock().files.insert(id, meta);
    }
    async fn add(&self, file_id: i64, lane: Option<i64>) -> i64 {
        repo::queue::add(
            &self.pool,
            file_id,
            &format!("file{file_id}"),
            lane,
            QueueSource::Manual,
            "t",
        )
        .await
        .expect("add")
        .id
    }
    /// Run one tick and wait for its dispatches.
    async fn tick(&self) -> usize {
        let hs = self.sched.tick().await;
        let n = hs.len();
        for h in hs {
            h.await.expect("dispatch task");
        }
        n
    }
    async fn status_of(&self, id: i64) -> QueueStatus {
        repo::queue::get(&self.pool, id).await.expect("get").status
    }
    async fn lane_of(&self, id: i64) -> Option<i64> {
        repo::queue::get(&self.pool, id)
            .await
            .expect("get")
            .printer_id
    }
    async fn plate(&self, pid: i64) -> bool {
        repo::printer::get(&self.pool, pid)
            .await
            .expect("printer")
            .plate_clear
    }
    /// Report a state to the scheduler as the bus would.
    async fn report(&self, pid: i64, state: PrinterState) -> bool {
        self.env.set_state(pid, state.clone());
        let st = self
            .env
            .lock()
            .status
            .get(&pid)
            .cloned()
            .unwrap_or_default();
        self.sched.on_status(pid, &st).await
    }
    /// Simulate a full print on `pid`: printing, then idle (finished).
    async fn finish_print(&self, pid: i64) {
        self.report(pid, PrinterState::Printing).await;
        assert!(
            self.report(pid, PrinterState::Idle).await,
            "finish must wake the loop"
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fleet_prefers_loaded_filament_then_sort_order() {
    let r = rig(None).await;
    // Bravo has white PLA loaded; Alpha reports no trays (Klipper).
    r.env
        .lock()
        .status
        .insert(2, status(PrinterState::Idle, vec![tray("PLA", "FFFFFFFF")]));
    r.file(10, file("PLA", "#FFFFFF"));
    r.file(11, file("PLA", "#000000"));
    r.file(12, file("PLA", "#FFFFFF"));
    let f1 = r.add(10, None).await;
    let f2 = r.add(11, None).await;
    let f3 = r.add(12, None).await;

    assert_eq!(r.tick().await, 2);
    // f1 (white PLA) goes to Bravo even though Alpha sorts first; f2 takes Alpha.
    assert_eq!(r.lane_of(f1).await, Some(2));
    assert_eq!(r.lane_of(f2).await, Some(1));
    assert_eq!(r.status_of(f1).await, QueueStatus::Printing);
    assert_eq!(r.status_of(f2).await, QueueStatus::Printing);
    // f3 waits: both printers busy.
    assert_eq!(r.status_of(f3).await, QueueStatus::Waiting);
    assert_eq!(r.lane_of(f3).await, None);
    let starts = r.env.starts();
    assert_eq!(starts.len(), 2);
    assert!(starts.contains(&(2, "file10.gcode".into())));
    assert!(starts.contains(&(1, "file11.gcode".into())));
    // Fleet items record that they came from the fleet.
    assert!(repo::queue::get(&r.pool, f1).await.expect("get").from_fleet);
}

#[tokio::test]
async fn fleet_order_first_item_picks_first() {
    let r = rig(None).await;
    r.file(10, file("PLA", "#FFFFFF"));
    r.file(11, file("PLA", "#FFFFFF"));
    let a = r.add(10, None).await;
    let b = r.add(11, None).await;
    // Only Alpha is free: Bravo is printing something else.
    r.env.set_state(2, PrinterState::Printing);
    assert_eq!(r.tick().await, 1);
    assert_eq!(
        r.lane_of(a).await,
        Some(1),
        "head of the fleet queue goes first"
    );
    assert_eq!(r.status_of(b).await, QueueStatus::Waiting);
}

#[tokio::test]
async fn printer_queue_head_first_and_never_two_jobs() {
    let r = rig(None).await;
    r.file(10, file("PLA", "#FFFFFF"));
    let a1 = r.add(10, Some(1)).await;
    let a2 = r.add(10, Some(1)).await;
    let f1 = r.add(10, None).await;
    // Bravo busy, so only Alpha matters.
    r.env.set_state(2, PrinterState::Printing);

    // Two concurrent ticks + a third after: Alpha gets exactly one job (a1).
    let (h1, h2) = tokio::join!(r.sched.tick(), r.sched.tick());
    for h in h1.into_iter().chain(h2) {
        h.await.expect("task");
    }
    r.tick().await;
    assert_eq!(r.env.starts(), vec![(1, "file10.gcode".to_owned())]);
    assert_eq!(r.status_of(a1).await, QueueStatus::Printing);
    assert_eq!(r.status_of(a2).await, QueueStatus::Waiting);
    // Alpha has its own queue, so the fleet item is not given to it.
    assert_eq!(r.status_of(f1).await, QueueStatus::Waiting);
}

#[tokio::test]
async fn plate_gate_holds_until_plate_cleared() {
    let mut r = rig(None).await;
    r.file(10, file("PLA", "#FFFFFF"));
    r.env.set_state(2, PrinterState::Offline);
    let a = r.add(10, Some(1)).await;
    let b = r.add(10, Some(1)).await;

    // a starts; the dispatcher closes the plate gate.
    assert_eq!(r.tick().await, 1);
    assert_eq!(r.status_of(a).await, QueueStatus::Printing);
    assert!(!r.plate(1).await);

    // The print finishes: a is done, plate stays dirty, Attention is raised.
    r.finish_print(1).await;
    assert_eq!(r.status_of(a).await, QueueStatus::Done);
    assert!(!r.plate(1).await);
    let att = repo::attention::get_active_by_printer_kind(&r.pool, 1, PLATE_ATTENTION)
        .await
        .expect("q")
        .expect("plate attention raised");
    assert!(att.title.contains("clear the plate"), "{}", att.title);
    let mut saw_added = false;
    while let Ok(ev) = r.rx.try_recv() {
        if matches!(ev, ServerEvent::AttentionAdded { ref item } if item.kind == PLATE_ATTENTION) {
            saw_added = true;
        }
    }
    assert!(saw_added, "attention.added must be broadcast");

    // Printer idle, b waiting: the gate holds it.
    assert_eq!(r.tick().await, 0);
    assert_eq!(r.tick().await, 0);
    assert_eq!(r.status_of(b).await, QueueStatus::Waiting);
    assert_eq!(r.env.starts().len(), 1);

    // "Plate cleared" releases it.
    r.sched.plate_cleared(1).await.expect("plate cleared");
    assert!(r.plate(1).await);
    assert!(
        repo::attention::get_active_by_printer_kind(&r.pool, 1, PLATE_ATTENTION)
            .await
            .expect("q")
            .is_none()
    );
    assert_eq!(r.tick().await, 1);
    assert_eq!(r.status_of(b).await, QueueStatus::Printing);
    assert_eq!(r.env.starts().len(), 2);
}

#[tokio::test]
async fn print_finished_outside_the_queue_also_closes_the_gate() {
    let r = rig(None).await;
    r.file(10, file("PLA", "#FFFFFF"));
    r.env.set_state(2, PrinterState::Offline);
    // A print started elsewhere (Mainsail) finishes on Alpha.
    r.finish_print(1).await;
    assert!(!r.plate(1).await);
    let a = r.add(10, None).await;
    assert_eq!(r.tick().await, 0);
    assert_eq!(r.status_of(a).await, QueueStatus::Waiting);
    r.sched.plate_cleared(1).await.expect("cleared");
    assert_eq!(r.tick().await, 1);
    assert_eq!(r.lane_of(a).await, Some(1));
}

#[tokio::test]
async fn auto_start_off_is_respected_and_start_next_dry_run_keeps_item() {
    let r = rig(None).await;
    r.file(10, file("PLA", "#FFFFFF"));
    sqlx::query("UPDATE printer SET auto_start_from_queue = 0")
        .execute(&r.pool)
        .await
        .expect("off");
    let a = r.add(10, Some(1)).await;
    let f = r.add(10, None).await;
    assert_eq!(r.tick().await, 0);
    assert_eq!(r.status_of(a).await, QueueStatus::Waiting);
    assert_eq!(r.status_of(f).await, QueueStatus::Waiting);

    // Start next now, dry run: uploads, starts nothing, item stays waiting.
    let s = r.sched.start_next(1, false, true).await.expect("dry run");
    assert!(s.dry_run);
    assert_eq!(s.item_id, a);
    assert_eq!(r.env.lock().uploads, vec![(1, 10)]);
    assert!(r.env.starts().is_empty());
    assert_eq!(r.status_of(a).await, QueueStatus::Waiting);

    // Real start next: a prints on Alpha despite auto-start off.
    r.sched.start_next(1, false, false).await.expect("start");
    assert_eq!(r.status_of(a).await, QueueStatus::Printing);
    // Second start next on the same printer is refused.
    assert!(r.sched.start_next(1, true, false).await.is_err());

    // Bravo: empty own queue → takes the fleet head; plate dirty needs confirm.
    repo::printer::set_plate_clear(&r.pool, 2, false)
        .await
        .expect("plate");
    assert!(r.sched.start_next(2, false, false).await.is_err());
    r.sched.start_next(2, true, false).await.expect("confirmed");
    assert_eq!(r.lane_of(f).await, Some(2));
    assert_eq!(r.status_of(f).await, QueueStatus::Printing);
}

#[tokio::test]
async fn move_between_lanes_changes_who_prints_it() {
    let r = rig(None).await;
    r.file(10, file("PLA", "#FFFFFF"));
    r.file(11, file("PLA", "#FFFFFF"));
    r.env.set_state(1, PrinterState::Printing);
    r.env.set_state(2, PrinterState::Printing);
    let a = r.add(10, Some(1)).await;
    let b = r.add(11, Some(1)).await;
    // Reorder: b before a; then move a to Bravo; then b to the fleet.
    repo::queue::move_to(&r.pool, b, Some(1), 0)
        .await
        .expect("reorder");
    assert_eq!(
        repo::queue::list_waiting(&r.pool, Some(1))
            .await
            .expect("l")
            .iter()
            .map(|i| i.id)
            .collect::<Vec<_>>(),
        vec![b, a]
    );
    repo::queue::move_to(&r.pool, a, Some(2), 0)
        .await
        .expect("move");
    repo::queue::move_to(&r.pool, b, None, 0)
        .await
        .expect("to fleet");
    // Only Bravo frees up: it takes its own a; b (fleet) waits for the next printer.
    r.env.set_state(2, PrinterState::Idle);
    assert_eq!(r.tick().await, 1);
    assert_eq!(r.env.starts(), vec![(2, "file10.gcode".to_owned())]);
    r.env.set_state(1, PrinterState::Idle);
    assert_eq!(r.tick().await, 1);
    assert_eq!(r.lane_of(b).await, Some(1));
}

#[tokio::test]
async fn cancelled_while_sending_never_starts() {
    let gate = Arc::new(Notify::new());
    let r = rig(Some(gate.clone())).await;
    r.file(10, file("PLA", "#FFFFFF"));
    r.env.set_state(2, PrinterState::Offline);
    let a = r.add(10, Some(1)).await;
    let hs = r.sched.tick().await;
    assert_eq!(hs.len(), 1);
    // Upload is blocked at the gate: item is sending.
    tokio::task::yield_now().await;
    assert_eq!(r.status_of(a).await, QueueStatus::Sending);
    assert_eq!(
        repo::queue::remove(&r.pool, a).await.expect("cancel"),
        repo::queue::Removed::Cancelled
    );
    // A tick meanwhile must not dispatch anything else.
    gate.notify_one();
    for h in hs {
        h.await.expect("task");
    }
    assert_eq!(r.status_of(a).await, QueueStatus::Cancelled);
    assert!(
        r.env.starts().is_empty(),
        "start must not be sent after cancel"
    );
    assert!(r.plate(1).await, "plate untouched when nothing started");
}

#[tokio::test]
async fn upload_failure_marks_item_failed_and_frees_printer() {
    let r = rig(None).await;
    r.file(10, file("PLA", "#FFFFFF"));
    r.env.set_state(2, PrinterState::Offline);
    r.env.lock().fail_upload = true;
    let a = r.add(10, Some(1)).await;
    let b = r.add(10, Some(1)).await;
    assert_eq!(r.tick().await, 1);
    let item = repo::queue::get(&r.pool, a).await.expect("get");
    assert_eq!(item.status, QueueStatus::Failed);
    assert!(item.error.as_deref().unwrap_or("").contains("mock"));
    // The failed item doesn't block the printer: b goes next once uploads work.
    r.env.lock().fail_upload = false;
    assert_eq!(r.tick().await, 1);
    assert_eq!(r.status_of(b).await, QueueStatus::Printing);
}

#[tokio::test]
async fn incompatible_fleet_item_is_skipped_not_blocking() {
    let r = rig(None).await;
    // Bravo has only PETG loaded → Material for a PLA file; Alpha busy.
    r.env.lock().status.insert(
        2,
        status(PrinterState::Idle, vec![tray("PETG", "000000FF")]),
    );
    r.env.set_state(1, PrinterState::Printing);
    r.file(10, file("PLA", "#FFFFFF"));
    r.file(11, file("PETG", "#000000"));
    let pla = r.add(10, None).await;
    let petg = r.add(11, None).await;
    assert_eq!(r.tick().await, 1);
    assert_eq!(r.status_of(pla).await, QueueStatus::Waiting);
    assert_eq!(r.lane_of(petg).await, Some(2));
}

// ---------------------------------------------------------------------------
// Job engine (3.6)
// ---------------------------------------------------------------------------

mod jobs {
    use super::*;
    use crate::job_engine;
    use foreman_core::{
        domain::{Job, JobEnd, JobOutcome, UpdatePrinter, UsageReporter},
        events::PrintEnd,
    };

    /// A PLA file: 12.5 g / 4000 mm on tool 0, preset names spool #7.
    fn spool_file() -> FileMeta {
        FileMeta {
            format: "gcode".into(),
            parse_status: "ok".into(),
            tools: vec![ToolUsage {
                index: 0,
                material: Some("PLA".into()),
                colour: Some("#FFFFFF".into()),
                grams: Some(12.5),
                mm: Some(4000.0),
                spool_id: Some(7),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn st(state: PrinterState, end: Option<PrintEnd>) -> PrinterStatus {
        PrinterStatus {
            state,
            print_end: end,
            filename: Some("file10.gcode".into()),
            ..Default::default()
        }
    }

    impl Rig {
        async fn push(&self, pid: i64, s: PrinterStatus) -> bool {
            self.env.lock().status.insert(pid, s.clone());
            self.sched.on_status(pid, &s).await
        }
        async fn jobs(&self) -> Vec<Job> {
            repo::job::list(
                &self.pool,
                &repo::job::JobFilter {
                    limit: 100,
                    ..Default::default()
                },
            )
            .await
            .expect("jobs")
            .0
        }
        fn deductions(&self) -> Vec<(i64, f64)> {
            self.env.lock().deductions.clone()
        }
        /// Queue spool_file on Alpha and dispatch it (Bravo offline).
        async fn start_one(&self) -> i64 {
            self.file(10, spool_file());
            self.env.set_state(2, PrinterState::Offline);
            let id = self.add(10, Some(1)).await;
            assert_eq!(self.tick().await, 1);
            id
        }
    }

    #[tokio::test]
    async fn mock_print_start_to_finish_writes_job_deducts_once_raises_plate_item() {
        let r = rig(None).await;
        let item = r.start_one().await;
        // The job opens when Foreman sends the start command.
        let jobs = r.jobs().await;
        assert_eq!(jobs.len(), 1);
        let j = &jobs[0];
        assert_eq!(j.end, JobEnd::Running);
        assert_eq!(
            (j.printer_id, j.file_id, j.queue_item_id),
            (Some(1), Some(10), Some(item))
        );
        assert_eq!(j.file_name, "file10.gcode");
        assert_eq!(j.grams_planned, Some(12.5));
        assert_eq!(j.usage_reporter, "foreman");
        assert_eq!(j.tools[0].spool_id, Some(7));

        // Printer reports printing: still one job.
        r.push(1, st(PrinterState::Printing, None)).await;
        assert_eq!(r.jobs().await.len(), 1);

        // Klipper "complete".
        assert!(
            r.push(1, st(PrinterState::Idle, Some(PrintEnd::Complete)))
                .await
        );
        let j = &r.jobs().await[0];
        assert_eq!(j.end, JobEnd::Complete);
        assert_eq!(j.outcome, None, "a clean finish waits for a rating");
        assert!(j.ended_at.is_some() && j.duration_secs.is_some());
        assert_eq!(j.grams_used, Some(12.5));
        assert_eq!(j.grams_deducted, 12.5);
        assert_eq!(r.deductions(), vec![(7, 12.5)]);
        assert_eq!(r.status_of(item).await, QueueStatus::Done);

        // Plate gate + Attention linked to the job.
        assert!(!r.plate(1).await);
        let att = repo::attention::get_active_by_printer_kind(&r.pool, 1, PLATE_ATTENTION)
            .await
            .expect("q")
            .expect("plate item");
        assert_eq!(att.job_id, Some(j.id));
        assert!(att.title.contains("print finished"), "{}", att.title);

        // Repeats never deduct again: the same end event, and a direct retry.
        r.push(1, st(PrinterState::Idle, Some(PrintEnd::Complete)))
            .await;
        job_engine::deduct_once(&r.pool, r.env.as_ref(), j.id, "foreman", j.tools.clone())
            .await
            .expect("retry");
        assert_eq!(r.deductions().len(), 1);
        assert_eq!(r.jobs().await.len(), 1);

        // Rating from the Attention item.
        let rated = repo::job::set_outcome(&r.pool, j.id, JobOutcome::Good, None)
            .await
            .expect("rate");
        assert_eq!(rated.outcome, Some(JobOutcome::Good));
    }

    #[tokio::test]
    async fn job_events_are_broadcast() {
        let mut r = rig(None).await;
        r.start_one().await;
        r.push(1, st(PrinterState::Printing, None)).await;
        r.push(1, st(PrinterState::Idle, Some(PrintEnd::Complete)))
            .await;
        let mut kinds = Vec::new();
        while let Ok(ev) = r.rx.try_recv() {
            if let ServerEvent::Job { payload, .. } = ev {
                kinds.push(payload["kind"].as_str().unwrap_or("").to_owned());
            }
        }
        assert_eq!(kinds, ["started", "ended"]);
    }

    #[tokio::test]
    async fn cancel_is_not_a_clean_finish() {
        let r = rig(None).await;
        let item = r.start_one().await;
        r.push(1, st(PrinterState::Printing, None)).await;
        // Klipper "cancelled" at 40 %: 1600 of 4000 mm extruded.
        let mut s = st(PrinterState::Idle, Some(PrintEnd::Cancelled));
        s.filament_used_mm = Some(1600.0);
        s.progress = Some(0.9); // mm wins over progress when both are known
        r.push(1, s).await;
        let j = &r.jobs().await[0];
        assert_eq!(j.end, JobEnd::Cancelled);
        assert_eq!(j.outcome, Some(JobOutcome::Cancelled));
        assert_eq!(j.grams_used, Some(5.0));
        assert_eq!(r.deductions(), vec![(7, 5.0)]);
        assert_eq!(r.status_of(item).await, QueueStatus::Cancelled);
        let att = repo::attention::get_active_by_printer_kind(&r.pool, 1, PLATE_ATTENTION)
            .await
            .expect("q")
            .expect("plate item");
        assert!(att.title.contains("cancelled"), "{}", att.title);
    }

    #[tokio::test]
    async fn error_is_failed_with_the_printer_message() {
        let r = rig(None).await;
        let item = r.start_one().await;
        r.push(1, st(PrinterState::Printing, None)).await;
        let mut s = st(PrinterState::Error, Some(PrintEnd::Error));
        s.print_message = Some("Heater extruder not heating at expected rate".into());
        s.progress = Some(0.2);
        r.push(1, s).await;
        let j = &r.jobs().await[0];
        assert_eq!(j.end, JobEnd::Error);
        assert_eq!(j.outcome, Some(JobOutcome::Failed));
        assert_eq!(
            j.error.as_deref(),
            Some("Heater extruder not heating at expected rate")
        );
        assert_eq!(j.grams_used, Some(2.5));
        assert_eq!(r.deductions(), vec![(7, 2.5)]);
        assert_eq!(r.status_of(item).await, QueueStatus::Failed);
    }

    #[tokio::test]
    async fn bambu_style_failed_without_hms_is_idle_but_still_an_error_end() {
        // Bambu FAILED with no HMS codes maps to Idle; print_end says Error.
        let r = rig(None).await;
        r.start_one().await;
        r.push(1, st(PrinterState::Printing, None)).await;
        r.push(1, st(PrinterState::Idle, Some(PrintEnd::Error)))
            .await;
        assert_eq!(r.jobs().await[0].outcome, Some(JobOutcome::Failed));
    }

    #[tokio::test]
    async fn restart_mid_print_deducts_once() {
        let r = rig(None).await;
        r.start_one().await;
        r.push(1, st(PrinterState::Printing, None)).await;

        // Server restarts while printing: a fresh scheduler on the same DB.
        let (tx, _) = broadcast::channel(64);
        let s2 = Scheduler::new(r.pool.clone(), Arc::new(tx), r.env.clone());
        let printing = st(PrinterState::Printing, None);
        s2.on_status(1, &printing).await;
        assert_eq!(r.jobs().await.len(), 1, "re-attaches, no second job");
        s2.on_status(1, &st(PrinterState::Idle, Some(PrintEnd::Complete)))
            .await;
        assert_eq!(r.deductions(), vec![(7, 12.5)]);

        // Another restart after the end: the stale "complete" state is not a new end.
        let (tx, _) = broadcast::channel(64);
        let s3 = Scheduler::new(r.pool.clone(), Arc::new(tx), r.env.clone());
        s3.on_status(1, &st(PrinterState::Idle, Some(PrintEnd::Complete)))
            .await;
        assert_eq!(r.deductions().len(), 1);
        let jobs = r.jobs().await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].end, JobEnd::Complete);
    }

    #[tokio::test]
    async fn print_that_ended_while_foreman_was_down_is_closed_once() {
        let r = rig(None).await;
        r.start_one().await;
        r.push(1, st(PrinterState::Printing, None)).await;
        // Down; the print finishes; Foreman starts and first sees "complete".
        let (tx, _) = broadcast::channel(64);
        let s2 = Scheduler::new(r.pool.clone(), Arc::new(tx), r.env.clone());
        s2.on_status(1, &st(PrinterState::Idle, Some(PrintEnd::Complete)))
            .await;
        let j = &r.jobs().await[0];
        assert_eq!(j.end, JobEnd::Complete);
        assert_eq!(r.deductions(), vec![(7, 12.5)]);
        assert!(
            repo::attention::get_active_by_printer_kind(&r.pool, 1, PLATE_ATTENTION)
                .await
                .expect("q")
                .is_some()
        );
        // Back from offline later: no second end.
        s2.on_status(1, &st(PrinterState::Offline, None)).await;
        s2.on_status(1, &st(PrinterState::Idle, Some(PrintEnd::Complete)))
            .await;
        assert_eq!(r.deductions().len(), 1);
    }

    #[tokio::test]
    async fn moonraker_reporter_printers_are_not_deducted_by_foreman() {
        let r = rig(None).await;
        // Alpha's Moonraker has a working [spoolman] link, active spool #42.
        r.env.lock().caps.insert(
            1,
            PrinterCapabilities {
                spoolman_connected: Some(true),
                spoolman_spool_id: Some(42),
                ..Default::default()
            },
        );
        let mut f = spool_file();
        f.tools[0].spool_id = None;
        r.file(10, f);
        r.env.set_state(2, PrinterState::Offline);
        r.add(10, Some(1)).await;
        assert_eq!(r.tick().await, 1);
        r.push(1, st(PrinterState::Printing, None)).await;
        r.push(1, st(PrinterState::Idle, Some(PrintEnd::Complete)))
            .await;
        let j = &r.jobs().await[0];
        assert_eq!(j.usage_reporter, "moonraker");
        assert_eq!(j.grams_used, Some(12.5), "use is still recorded for stats");
        assert_eq!(j.grams_deducted, 0.0);
        assert_eq!(
            j.tools[0].spool_id,
            Some(42),
            "Moonraker's active spool is recorded"
        );
        assert!(r.deductions().is_empty());
        assert_eq!(
            repo::job::deduct_state(&r.pool, j.id).await.expect("state"),
            "skipped"
        );

        // Forcing usage_reporter = foreman on the same printer makes Foreman deduct.
        repo::printer::update(
            &r.pool,
            1,
            UpdatePrinter {
                name: None,
                firmware: None,
                primary_url: None,
                api_key: None,
                serial_number: None,
                access_code: None,
                tags: None,
                hardened_nozzle: None,
                slicer_profiles: None,
                sort_order: None,
                fail_after_checks: None,
                return_after_secs: None,
                plate_clear: Some(true),
                auto_start_from_queue: None,
                usage_reporter: Some(UsageReporter::Foreman),
            },
        )
        .await
        .expect("update");
        r.add(10, Some(1)).await;
        assert_eq!(r.tick().await, 1);
        r.push(1, st(PrinterState::Printing, None)).await;
        r.push(1, st(PrinterState::Idle, Some(PrintEnd::Complete)))
            .await;
        assert_eq!(r.deductions(), vec![(42, 12.5)]);
    }

    #[tokio::test]
    async fn moonraker_without_a_spoolman_link_is_deducted_by_foreman() {
        let r = rig(None).await;
        r.env.lock().caps.insert(
            1,
            PrinterCapabilities {
                spoolman_connected: Some(false),
                ..Default::default()
            },
        );
        r.start_one().await;
        r.push(1, st(PrinterState::Printing, None)).await;
        r.push(1, st(PrinterState::Idle, Some(PrintEnd::Complete)))
            .await;
        assert_eq!(r.jobs().await[0].usage_reporter, "foreman");
        assert_eq!(r.deductions(), vec![(7, 12.5)]);
    }

    #[tokio::test]
    async fn print_started_on_the_printer_is_recorded_without_deduction() {
        let r = rig(None).await;
        r.env.set_state(2, PrinterState::Offline);
        let mut s = st(PrinterState::Printing, None);
        s.filename = Some("from_mainsail.gcode".into());
        r.push(1, s).await;
        let j = &r.jobs().await[0];
        assert_eq!(
            (j.file_id, j.file_name.as_str()),
            (None, "from_mainsail.gcode")
        );
        r.push(1, st(PrinterState::Idle, Some(PrintEnd::Complete)))
            .await;
        assert_eq!(r.jobs().await[0].end, JobEnd::Complete);
        assert!(
            r.deductions().is_empty(),
            "no plan, no spool: nothing to deduct"
        );
        assert!(!r.plate(1).await);
    }

    /// W9 (5.1): a queued project print that finishes on a (mock) printer marks
    /// its own plate through the job engine; the job is linked to the plate.
    #[tokio::test]
    async fn finished_project_print_marks_its_plate() {
        use foreman_core::{projects::ToolSlot, repo::project::NewProject};
        let r = rig(None).await;
        r.file(10, spool_file());
        r.file(11, spool_file());
        r.env.set_state(2, PrinterState::Offline);
        let tools = vec![ToolSlot {
            tool: 0,
            slot: "PRIMARY".into(),
        }];
        let pid = repo::project::create_project(
            &r.pool,
            &NewProject {
                name: "Clips",
                template_id: None,
                template_name: None,
                sets: 1,
                due_date: None,
                slots: vec![("PRIMARY".into(), None, None, Some(7))],
                plates: vec![
                    ("A".into(), 10, 1, tools.clone()),
                    ("B".into(), 11, 2, tools),
                ],
            },
        )
        .await
        .expect("project");
        let plates = repo::project::plates(&r.pool, pid).await.expect("plates");
        let item = r.add(11, Some(1)).await;
        repo::project::link_queue_item(&r.pool, item, plates[1].id)
            .await
            .expect("link");

        assert_eq!(r.tick().await, 1);
        r.push(1, st(PrinterState::Printing, None)).await;
        r.push(1, st(PrinterState::Idle, Some(PrintEnd::Complete)))
            .await;

        let after = repo::project::plates(&r.pool, pid).await.expect("plates");
        assert_eq!(
            (after[0].printed, after[1].printed),
            (0, 1),
            "plate B, not A"
        );
        let job_plate: Option<i64> =
            sqlx::query_scalar("SELECT project_plate_id FROM job WHERE queue_item_id = ?")
                .bind(item)
                .fetch_one(&r.pool)
                .await
                .expect("job");
        assert_eq!(job_plate, Some(plates[1].id));
        assert_eq!(
            repo::project::get_project(&r.pool, pid)
                .await
                .expect("p")
                .status,
            "active",
            "B wants 2 copies, A none printed yet"
        );
    }
}

/// The live deduction path: spoolman-rs `use_weight` on a real SQLite Spoolman DB.
#[tokio::test]
async fn live_spoolman_deduction_updates_the_spool_and_broadcasts() {
    use spoolman_rs::{
        config::{DbType, SpoolmanConfig},
        db::{filament, spool, Db},
        models::{CreateFilamentRequest, CreateSpoolRequest},
        state::AppState,
    };
    let dir = tempfile::TempDir::new().expect("tmp");
    let cfg = SpoolmanConfig {
        db_type: DbType::Sqlite,
        db_host: None,
        db_port: None,
        db_name: None,
        db_username: None,
        db_password: None,
        data_dir: dir.path().to_path_buf(),
        logs_dir: dir.path().to_path_buf(),
        backups_dir: dir.path().join("backups"),
        host: "127.0.0.1".into(),
        port: 0,
        base_path: String::new(),
        automatic_backup: false,
    };
    let db = Db::connect(&cfg).await.expect("spoolman db");
    let f: CreateFilamentRequest = serde_json::from_value(
        serde_json::json!({"density": 1.24, "diameter": 1.75, "material": "PLA"}),
    )
    .expect("filament req");
    let f = filament::create(&db.pool, &db.engine, &f)
        .await
        .expect("filament");
    let s: CreateSpoolRequest =
        serde_json::from_value(serde_json::json!({"filament_id": f.id, "initial_weight": 1000.0}))
            .expect("spool req");
    let s = spool::create(&db.pool, &db.engine, &s)
        .await
        .expect("spool");
    let sm = AppState::new(db, cfg);
    let mut events = sm.event_tx.subscribe();

    crate::scheduler::spoolman_deduct(&sm, s.id, 12.5)
        .await
        .expect("deduct");
    let after = spool::find_by_id(&sm.db.pool, s.id).await.expect("spool");
    assert_eq!(after.used_weight, 12.5);
    assert!(
        events.try_recv().is_ok(),
        "spool update broadcast to Spoolman clients"
    );
    assert!(crate::scheduler::spoolman_deduct(&sm, 9999, 1.0)
        .await
        .is_err());
}
