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
                caps: None,
                printer: p,
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
