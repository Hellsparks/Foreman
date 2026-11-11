//! Transfers against a mock Moonraker file API: progress, cancel, conflicts, the
//! printing-file guard, STL refusal, multi-select, and Library bulk trash/undo.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use foreman_core::{
    db::ForemanDb,
    drivers::moonraker_files::MoonrakerFiles,
    events::{ServerEvent, TransferInfo, TransferState},
    library::LibraryStore,
    storage::PrinterStorage,
};
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::transfers::{
    LocRef, Mode, OnConflict, StartOutcome, StorageResolver, TransferItem, TransferRequest,
    Transfers,
};

// ---------------------------------------------------------------------------
// Mock Moonraker (gcodes root only)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Fs {
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
    active: Option<String>,
    /// Delay per 64 KiB chunk, to make cancel observable.
    slow: bool,
}

type Mock = Arc<Mutex<Fs>>;

fn lock(m: &Mock) -> std::sync::MutexGuard<'_, Fs> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn rel(p: &str) -> String {
    p.trim_start_matches("gcodes").trim_matches('/').to_owned()
}

fn parent(p: &str) -> String {
    p.rsplit_once('/')
        .map(|(a, _)| a.to_owned())
        .unwrap_or_default()
}

fn leaf(p: &str) -> String {
    p.rsplit_once('/')
        .map(|(_, b)| b.to_owned())
        .unwrap_or_else(|| p.to_owned())
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": { "code": 404, "message": "Not Found" } })),
    )
        .into_response()
}

fn ok(v: Value) -> Response {
    Json(json!({ "result": v })).into_response()
}

fn add_parents(fs: &mut Fs, path: &str) {
    let mut cur = String::new();
    for seg in parent(path).split('/').filter(|s| !s.is_empty()) {
        cur = if cur.is_empty() {
            seg.to_owned()
        } else {
            format!("{cur}/{seg}")
        };
        fs.dirs.insert(cur.clone());
    }
}

async fn m_directory(State(m): State<Mock>, Query(q): Query<HashMap<String, String>>) -> Response {
    let dir = rel(q.get("path").map(String::as_str).unwrap_or("gcodes"));
    let fs = lock(&m);
    if !dir.is_empty() && !fs.dirs.contains(&dir) {
        return not_found();
    }
    let dirs: Vec<Value> = fs
        .dirs
        .iter()
        .filter(|d| parent(d) == dir)
        .map(|d| json!({ "dirname": leaf(d), "modified": 0.0, "size": 4096, "permissions": "rw" }))
        .collect();
    let files: Vec<Value> = fs
        .files
        .iter()
        .filter(|(f, _)| parent(f) == dir)
        .map(|(f, b)| {
            json!({ "filename": leaf(f), "modified": 1.0, "size": b.len(), "permissions": "rw",
            "estimated_time": 60.0, "slicer": "MockSlicer" })
        })
        .collect();
    ok(json!({ "dirs": dirs, "files": files }))
}

async fn m_mkdir(State(m): State<Mock>, Json(b): Json<Value>) -> Response {
    let p = rel(b["path"].as_str().unwrap_or(""));
    let mut fs = lock(&m);
    if fs.dirs.contains(&p) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": { "message": "exists" } })),
        )
            .into_response();
    }
    add_parents(&mut fs, &format!("{p}/x"));
    ok(json!({ "action": "create_dir" }))
}

async fn m_rmdir(State(m): State<Mock>, Query(q): Query<HashMap<String, String>>) -> Response {
    let p = rel(q.get("path").map(String::as_str).unwrap_or(""));
    let mut fs = lock(&m);
    let pre = format!("{p}/");
    fs.dirs.retain(|d| *d != p && !d.starts_with(&pre));
    fs.files.retain(|f, _| !f.starts_with(&pre));
    ok(json!({ "action": "delete_dir" }))
}

async fn m_upload(State(m): State<Mock>, mut mp: Multipart) -> Response {
    let mut folder = String::new();
    let slow = lock(&m).slow;
    while let Ok(Some(mut field)) = mp.next_field().await {
        match field.name().unwrap_or_default() {
            "path" => folder = field.text().await.unwrap_or_default(),
            "file" => {
                let name = field.file_name().unwrap_or("x").to_owned();
                let mut data = Vec::new();
                loop {
                    match field.chunk().await {
                        Ok(Some(c)) => {
                            data.extend_from_slice(&c);
                            if slow {
                                tokio::time::sleep(Duration::from_millis(40)).await;
                            }
                        }
                        Ok(None) => break,
                        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
                    }
                }
                let path = if folder.is_empty() {
                    name
                } else {
                    format!("{folder}/{name}")
                };
                let mut fs = lock(&m);
                add_parents(&mut fs, &path);
                fs.files.insert(path, data);
            }
            _ => {}
        }
    }
    (
        StatusCode::CREATED,
        Json(json!({ "result": { "action": "create_file" } })),
    )
        .into_response()
}

async fn m_get(State(m): State<Mock>, Path(p): Path<String>) -> Response {
    let (data, slow) = {
        let fs = lock(&m);
        match fs.files.get(&p) {
            Some(d) => (d.clone(), fs.slow),
            None => return not_found(),
        }
    };
    let chunks: Vec<Vec<u8>> = data.chunks(64 << 10).map(<[u8]>::to_vec).collect();
    let stream = futures_util::stream::unfold(chunks.into_iter(), move |mut it| async move {
        let c = it.next()?;
        if slow {
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        Some((Ok::<_, std::io::Error>(c), it))
    });
    Body::from_stream(stream).into_response()
}

async fn m_delete(State(m): State<Mock>, Path(p): Path<String>) -> Response {
    match lock(&m).files.remove(&p) {
        Some(_) => ok(json!({ "action": "delete_file" })),
        None => not_found(),
    }
}

async fn m_move(State(m): State<Mock>, Json(b): Json<Value>) -> Response {
    let (s, d) = (
        rel(b["source"].as_str().unwrap_or("")),
        rel(b["dest"].as_str().unwrap_or("")),
    );
    let mut fs = lock(&m);
    if fs.active.as_deref() == Some(s.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": { "message": "in use" } })),
        )
            .into_response();
    }
    let Some(data) = fs.files.remove(&s) else {
        return not_found();
    };
    add_parents(&mut fs, &d);
    fs.files.insert(d, data);
    ok(json!({ "action": "move_file" }))
}

async fn m_copy(State(m): State<Mock>, Json(b): Json<Value>) -> Response {
    let (s, d) = (
        rel(b["source"].as_str().unwrap_or("")),
        rel(b["dest"].as_str().unwrap_or("")),
    );
    let mut fs = lock(&m);
    let Some(data) = fs.files.get(&s).cloned() else {
        return not_found();
    };
    fs.files.insert(d, data);
    ok(json!({ "action": "create_file" }))
}

async fn m_query(State(m): State<Mock>) -> Response {
    let active = lock(&m).active.clone();
    ok(json!({ "status": { "print_stats": {
        "state": if active.is_some() { "printing" } else { "standby" },
        "filename": active.unwrap_or_default(),
    } } }))
}

async fn mock_moonraker() -> (String, Mock) {
    let m: Mock = Arc::default();
    let app = Router::new()
        .route(
            "/server/files/directory",
            get(m_directory).post(m_mkdir).delete(m_rmdir),
        )
        .route("/server/files/upload", post(m_upload))
        .route("/server/files/move", post(m_move))
        .route("/server/files/copy", post(m_copy))
        .route("/server/files/gcodes/{*path}", get(m_get).delete(m_delete))
        .route("/printer/objects/query", get(m_query))
        .layer(DefaultBodyLimit::disable())
        .with_state(m.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), m)
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Resolver(HashMap<i64, (String, Arc<dyn PrinterStorage>)>);

#[async_trait::async_trait]
impl StorageResolver for Resolver {
    async fn storage(&self, id: i64) -> Option<(String, Arc<dyn PrinterStorage>)> {
        self.0.get(&id).cloned()
    }
}

struct H {
    _dir: tempfile::TempDir,
    lib: Arc<LibraryStore>,
    xfer: Arc<Transfers>,
    bus: Arc<broadcast::Sender<ServerEvent>>,
    /// Printer 1 ("V0") and printer 2 ("V2").
    p1: Mock,
    p2: Mock,
}

async fn harness() -> H {
    let dir = tempfile::TempDir::new().expect("dir");
    let db = ForemanDb::connect(&dir.path().join("foreman.db"))
        .await
        .expect("db");
    let lib = Arc::new(LibraryStore::new(dir.path(), db.pool.clone()).expect("lib"));
    let (u1, p1) = mock_moonraker().await;
    let (u2, p2) = mock_moonraker().await;
    let mut map: HashMap<i64, (String, Arc<dyn PrinterStorage>)> = HashMap::new();
    map.insert(
        1,
        (
            "V0".into(),
            Arc::new(MoonrakerFiles::new(&u1, Some("k".into()))),
        ),
    );
    map.insert(2, ("V2".into(), Arc::new(MoonrakerFiles::new(&u2, None))));
    let (tx, _) = broadcast::channel(1024);
    let bus = Arc::new(tx);
    let xfer = Transfers::new(lib.clone(), Arc::new(Resolver(map)), bus.clone());
    H {
        _dir: dir,
        lib,
        xfer,
        bus,
        p1,
        p2,
    }
}

impl H {
    async fn lib_file(&self, folder: &str, name: &str, data: &[u8]) {
        let tmp = self.lib.tmp_dir().join(format!("seed-{name}"));
        std::fs::write(&tmp, data).expect("seed");
        self.lib.add_file(folder, name, &tmp).await.expect("add");
    }

    async fn lib_bytes(&self, path: &str) -> Vec<u8> {
        let e = self.lib.find(path).await.expect("find").expect("exists");
        std::fs::read(self.lib.file_path(e.id).await.expect("path")).expect("read")
    }
}

fn lib(path: &str) -> LocRef {
    LocRef::Library { path: path.into() }
}

fn prn(id: i64, path: &str) -> LocRef {
    LocRef::Printer {
        printer_id: id,
        path: path.into(),
    }
}

fn req(items: Vec<LocRef>, dest: LocRef, mode: Mode, on: OnConflict) -> TransferRequest {
    TransferRequest {
        items: items
            .into_iter()
            .map(|src| TransferItem {
                src,
                on_conflict: None,
            })
            .collect(),
        dest,
        mode,
        on_conflict: on,
    }
}

fn started(
    o: StartOutcome,
) -> (
    Vec<TransferInfo>,
    Vec<String>,
    Vec<crate::transfers::Blocked>,
) {
    match o {
        StartOutcome::Started {
            transfers,
            skipped,
            blocked,
        } => (transfers, skipped, blocked),
        StartOutcome::Conflicts { conflicts } => panic!("unexpected conflicts: {conflicts:?}"),
    }
}

async fn wait(x: &Transfers, id: &str) -> TransferInfo {
    for _ in 0..200 {
        if let Some(t) = x.list().into_iter().find(|t| t.id == id) {
            if t.state.is_finished() {
                return t;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("transfer {id} did not finish");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn library_to_printer_and_back_with_progress_events() {
    let h = harness().await;
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    h.lib_file("Parts", "bracket.gcode", &data).await;
    lock(&h.p1).dirs.insert("foreman-test".into());
    let mut rx = h.bus.subscribe();

    let (ts, _, blocked) = started(
        h.xfer
            .start(req(
                vec![lib("Parts/bracket.gcode")],
                prn(1, "foreman-test"),
                Mode::Copy,
                OnConflict::Ask,
            ))
            .await
            .expect("start"),
    );
    assert!(blocked.is_empty());
    let t = wait(&h.xfer, &ts[0].id).await;
    assert_eq!(t.state, TransferState::Done, "{:?}", t.error);
    assert_eq!(t.route, "Library → V0");
    assert_eq!(t.bytes_total, data.len() as u64);
    assert_eq!(t.bytes_done, data.len() as u64);
    assert_eq!(
        lock(&h.p1).files.get("foreman-test/bracket.gcode"),
        Some(&data)
    );

    let mut kinds = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        kinds.push(ev.topic().unwrap_or("").to_owned());
    }
    assert_eq!(kinds.first().map(String::as_str), Some("transfer.started"));
    assert_eq!(kinds.last().map(String::as_str), Some("transfer.finished"));
    assert!(kinds.iter().any(|k| k == "transfer.progress"));

    // Back into the Library: same name exists → keep both gives " (copy)".
    let (ts, _, _) = started(
        h.xfer
            .start(req(
                vec![prn(1, "foreman-test/bracket.gcode")],
                lib("Parts"),
                Mode::Copy,
                OnConflict::KeepBoth,
            ))
            .await
            .expect("start"),
    );
    let t = wait(&h.xfer, &ts[0].id).await;
    assert_eq!(t.state, TransferState::Done, "{:?}", t.error);
    assert_eq!(t.dest_path, "Parts/bracket (copy).gcode");
    assert_eq!(h.lib_bytes("Parts/bracket (copy).gcode").await, data);
}

#[tokio::test]
async fn conflicts_ask_keep_both_replace_skip() {
    let h = harness().await;
    h.lib_file("", "a.gcode", b"new").await;
    h.lib_file("", "b.gcode", b"bbb").await;
    {
        let mut fs = lock(&h.p1);
        fs.files.insert("a.gcode".into(), b"old".to_vec());
        fs.files.insert("b.gcode".into(), b"old-b".to_vec());
    }
    let items = vec![lib("a.gcode"), lib("b.gcode")];

    match h
        .xfer
        .start(req(items.clone(), prn(1, ""), Mode::Copy, OnConflict::Ask))
        .await
        .expect("ask")
    {
        StartOutcome::Conflicts { conflicts } => {
            assert_eq!(conflicts.len(), 2);
            assert_eq!(conflicts[0].name, "a.gcode");
            assert!(!conflicts[0].dest_printing);
        }
        other => panic!("expected conflicts, got {other:?}"),
    }
    assert_eq!(lock(&h.p1).files.len(), 2, "ask starts nothing");

    // Per-item answers: replace a, skip b.
    let mut r = req(items.clone(), prn(1, ""), Mode::Copy, OnConflict::Ask);
    r.items[0].on_conflict = Some(OnConflict::Replace);
    r.items[1].on_conflict = Some(OnConflict::Skip);
    let (ts, skipped, _) = started(h.xfer.start(r).await.expect("start"));
    assert_eq!(skipped, vec!["b.gcode".to_owned()]);
    assert_eq!(wait(&h.xfer, &ts[0].id).await.state, TransferState::Done);
    assert_eq!(lock(&h.p1).files["a.gcode"], b"new");
    assert_eq!(lock(&h.p1).files["b.gcode"], b"old-b");

    // Apply to all: keep both.
    let (ts, _, _) = started(
        h.xfer
            .start(req(items, prn(1, ""), Mode::Copy, OnConflict::KeepBoth))
            .await
            .expect("start"),
    );
    for t in &ts {
        assert_eq!(wait(&h.xfer, &t.id).await.state, TransferState::Done);
    }
    let names: Vec<String> = lock(&h.p1).files.keys().cloned().collect();
    assert_eq!(
        names,
        ["a (copy).gcode", "a.gcode", "b (copy).gcode", "b.gcode"]
    );

    // Replacing into the Library trashes the old file (restorable).
    lock(&h.p1)
        .files
        .insert("a.gcode".into(), b"from-printer".to_vec());
    let (ts, _, _) = started(
        h.xfer
            .start(req(
                vec![prn(1, "a.gcode")],
                lib(""),
                Mode::Copy,
                OnConflict::Replace,
            ))
            .await
            .expect("start"),
    );
    assert_eq!(wait(&h.xfer, &ts[0].id).await.state, TransferState::Done);
    assert_eq!(h.lib_bytes("a.gcode").await, b"from-printer");
    let trash = h.lib.list_trash().await.expect("trash");
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].name, "a.gcode");
}

#[tokio::test]
async fn printing_file_is_never_replaced_moved_or_deleted() {
    let h = harness().await;
    h.lib_file("", "job.gcode", b"new").await;
    {
        let mut fs = lock(&h.p1);
        fs.files.insert("job.gcode".into(), b"printing".to_vec());
        fs.files.insert("other.gcode".into(), b"o".to_vec());
        fs.active = Some("job.gcode".into());
    }
    match h
        .xfer
        .start(req(
            vec![lib("job.gcode")],
            prn(1, ""),
            Mode::Copy,
            OnConflict::Ask,
        ))
        .await
        .expect("ask")
    {
        StartOutcome::Conflicts { conflicts } => assert!(conflicts[0].dest_printing),
        other => panic!("expected conflicts, got {other:?}"),
    }
    let (ts, _, blocked) = started(
        h.xfer
            .start(req(
                vec![lib("job.gcode")],
                prn(1, ""),
                Mode::Copy,
                OnConflict::Replace,
            ))
            .await
            .expect("start"),
    );
    assert!(ts.is_empty());
    assert_eq!(blocked[0].reason, "printing now, not replaced");

    let (ts, _, blocked) = started(
        h.xfer
            .start(req(
                vec![prn(1, "job.gcode")],
                lib(""),
                Mode::Move,
                OnConflict::KeepBoth,
            ))
            .await
            .expect("start"),
    );
    assert!(ts.is_empty());
    assert_eq!(blocked[0].reason, "printing now, not moved");

    let (deleted, blocked) = h
        .xfer
        .delete_on_printer(1, &["job.gcode".into(), "other.gcode".into()])
        .await
        .expect("delete");
    assert_eq!(deleted, vec!["other.gcode".to_owned()]);
    assert_eq!(blocked[0].name, "job.gcode");
    assert_eq!(lock(&h.p1).files["job.gcode"], b"printing");

    // Copying the printing file away is fine.
    let (ts, _, _) = started(
        h.xfer
            .start(req(
                vec![prn(1, "job.gcode")],
                lib(""),
                Mode::Copy,
                OnConflict::KeepBoth,
            ))
            .await
            .expect("start"),
    );
    assert_eq!(wait(&h.xfer, &ts[0].id).await.state, TransferState::Done);
}

#[tokio::test]
async fn stl_refused_for_printers_and_skipped_inside_folders() {
    let h = harness().await;
    h.lib_file("Kit", "part.stl", b"solid x").await;
    h.lib_file("Kit", "part.gcode", b"G1").await;
    let (ts, _, blocked) = started(
        h.xfer
            .start(req(
                vec![lib("Kit/part.stl")],
                prn(1, ""),
                Mode::Copy,
                OnConflict::Ask,
            ))
            .await
            .expect("start"),
    );
    assert!(ts.is_empty());
    assert!(blocked[0].reason.contains("slice first"));

    let (ts, _, _) = started(
        h.xfer
            .start(req(
                vec![lib("Kit")],
                prn(1, ""),
                Mode::Copy,
                OnConflict::Ask,
            ))
            .await
            .expect("start"),
    );
    let t = wait(&h.xfer, &ts[0].id).await;
    assert_eq!(t.state, TransferState::Done, "{:?}", t.error);
    assert_eq!(t.name, "Kit/");
    assert!(t.note.as_deref().unwrap_or("").contains("1 STL"));
    let fs = lock(&h.p1);
    assert!(fs.files.contains_key("Kit/part.gcode"));
    assert!(!fs.files.contains_key("Kit/part.stl"));
}

#[tokio::test]
async fn cancel_stops_a_running_transfer() {
    let h = harness().await;
    // Bigger than loopback socket buffers, so the upload is still in flight at cancel time.
    let data = vec![7u8; 24 << 20];
    h.lib_file("", "big.gcode", &data).await;
    lock(&h.p1).slow = true;
    let (ts, _, _) = started(
        h.xfer
            .start(req(
                vec![lib("big.gcode")],
                prn(1, ""),
                Mode::Copy,
                OnConflict::Ask,
            ))
            .await
            .expect("start"),
    );
    let id = ts[0].id.clone();
    let mut rx = h.bus.subscribe();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let t = h.xfer.cancel(&id).expect("cancel");
    assert_eq!(t.state, TransferState::Cancelled);
    assert!(t.bytes_done < t.bytes_total);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !lock(&h.p1).files.contains_key("big.gcode"),
        "no partial file"
    );
    assert_eq!(wait(&h.xfer, &id).await.state, TransferState::Cancelled);
    let mut saw = false;
    while let Ok(ev) = rx.try_recv() {
        if let ServerEvent::TransferFinished { transfer } = ev {
            saw |= transfer.state == TransferState::Cancelled;
        }
    }
    assert!(saw, "transfer.finished with cancelled");
}

#[tokio::test]
async fn multi_select_move_printer_to_printer_and_within_library() {
    let h = harness().await;
    {
        let mut fs = lock(&h.p1);
        fs.files.insert("x.gcode".into(), b"xx".to_vec());
        fs.files.insert("y.gcode".into(), b"yyy".to_vec());
    }
    let (ts, _, _) = started(
        h.xfer
            .start(req(
                vec![prn(1, "x.gcode"), prn(1, "y.gcode")],
                prn(2, ""),
                Mode::Move,
                OnConflict::Ask,
            ))
            .await
            .expect("start"),
    );
    assert_eq!(ts.len(), 2);
    for t in &ts {
        let t = wait(&h.xfer, &t.id).await;
        assert_eq!(t.state, TransferState::Done, "{:?}", t.error);
        assert_eq!(t.route, "V0 → Foreman → V2");
        assert_eq!(t.bytes_done, t.bytes_total);
    }
    assert!(lock(&h.p1).files.is_empty(), "moved away");
    assert_eq!(lock(&h.p2).files["y.gcode"], b"yyy");

    // Library → Library move of two files into a folder; moving into the same folder is a no-op.
    h.lib_file("", "m1.gcode", b"1").await;
    h.lib_file("", "m2.gcode", b"2").await;
    h.lib.mkdir("", "Dest").await.expect("mkdir");
    let (ts, skipped, _) = started(
        h.xfer
            .start(req(
                vec![lib("m1.gcode"), lib("m2.gcode"), lib("Dest")],
                lib("Dest"),
                Mode::Move,
                OnConflict::Ask,
            ))
            .await
            .expect("start"),
    );
    assert!(skipped.is_empty());
    assert_eq!(ts.len(), 2, "Dest into itself is blocked");
    for t in &ts {
        assert_eq!(wait(&h.xfer, &t.id).await.state, TransferState::Done);
    }
    let names: Vec<String> = h
        .lib
        .list("Dest")
        .await
        .expect("list")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["m1.gcode", "m2.gcode"]);
}

#[tokio::test]
async fn library_bulk_trash_and_undo_over_http() {
    let dir = tempfile::TempDir::new().expect("dir");
    let (app, state) = crate::auth_api_tests::app(&dir, true).await;
    let (token, _) = foreman_core::auth::create_session(&state.foreman_db.pool, 3600, None, None)
        .await
        .expect("session");
    let bearer = format!("Bearer {token}");
    let mut ids = Vec::new();
    for n in ["a.gcode", "b.gcode"] {
        let tmp = state.library.tmp_dir().join(n);
        std::fs::write(&tmp, b"x").expect("seed");
        ids.push(state.library.add_file("", n, &tmp).await.expect("add").id);
    }
    let call = |uri: &'static str, body: Value| {
        let app = app.clone();
        let bearer = bearer.clone();
        async move {
            use tower::ServiceExt as _;
            let res = app
                .oneshot(
                    axum::http::Request::post(uri)
                        .header("authorization", bearer)
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .expect("req"),
                )
                .await
                .expect("res");
            let s = res.status();
            let b = axum::body::to_bytes(res.into_body(), 1 << 20)
                .await
                .expect("body");
            (
                s,
                serde_json::from_slice::<Value>(&b).unwrap_or(Value::Null),
            )
        }
    };
    let (s, b) = call("/api/library/trash-many", json!({ "ids": ids })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["trashed"].as_array().map(Vec::len), Some(2));
    assert!(state.library.list("").await.expect("list").is_empty());
    let (s, b) = call("/api/library/restore-many", json!({ "ids": ids })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["restored"].as_array().map(Vec::len), Some(2));
    assert_eq!(state.library.list("").await.expect("list").len(), 2);

    // Transfer request against an unknown printer is a 404, not a crash.
    let (s, _) = call(
        "/api/transfers",
        json!({ "items": [{ "src": { "kind": "library", "path": "a.gcode" } }],
                "dest": { "kind": "printer", "printer_id": 99, "path": "" } }),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
