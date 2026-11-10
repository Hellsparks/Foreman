//! Library: parser fixtures (trimmed real slicer output) and the trash/restore/index store.

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
};

use foreman_core::{
    db::ForemanDb,
    domain::LibraryEntryKind,
    library::{parse_path, LibraryStore},
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/library")
        .join(name)
}

fn copy_to(dir: &Path, fixture_name: &str, as_name: &str) -> PathBuf {
    let p = dir.join(as_name);
    fs::copy(fixture(fixture_name), &p).expect("copy fixture");
    p
}

#[test]
fn orca_gcode() {
    let p = parse_path(&fixture("orca_2.3.0.gcode"));
    let m = &p.meta;
    assert_eq!(m.format, "gcode");
    assert_eq!(m.parse_status, "ok");
    assert_eq!(m.slicer.as_deref(), Some("OrcaSlicer"));
    assert_eq!(m.slicer_version.as_deref(), Some("2.3.0"));
    assert_eq!(m.printer_model.as_deref(), Some("Voron 0.1"));
    assert_eq!(
        m.printer_settings_id.as_deref(),
        Some("Voron 0.2 0.4 nozzle")
    );
    assert_eq!(m.nozzle_diameter, Some(0.4));
    assert_eq!(m.print_time_secs, Some(757));
    assert_eq!(m.layer_count, Some(66));
    assert_eq!((m.bed_width, m.bed_depth), (Some(120.0), Some(120.0)));
    assert_eq!(
        m.filament_settings_id.as_deref(),
        Some("Generic ASA Fillament PM grey")
    );
    assert_eq!(m.tools.len(), 1);
    let t = &m.tools[0];
    assert_eq!(t.material.as_deref(), Some("ASA"));
    assert_eq!(t.colour.as_deref(), Some("#26A69A"));
    assert_eq!(t.grams, Some(1.85));
    assert_eq!(t.mm, Some(737.78));
    assert_eq!(m.suggested_spool_id, None);
    let thumb = p.thumbnail.expect("embedded thumbnail");
    assert!(thumb.starts_with(b"\x89PNG"));
}

#[test]
fn prusaslicer_gcode() {
    let m = parse_path(&fixture("prusaslicer_2.9.7.gcode")).meta;
    assert_eq!(m.slicer.as_deref(), Some("PrusaSlicer"));
    assert_eq!(m.slicer_version.as_deref(), Some("2.9.7"));
    assert_eq!(m.print_time_secs, Some(13 * 60 + 54));
    assert_eq!((m.bed_width, m.bed_depth), (Some(200.0), Some(200.0)));
    assert_eq!(m.nozzle_diameter, Some(0.4));
    assert_eq!(m.printer_model, None, "empty in a CLI slice");
    let t = &m.tools[0];
    assert_eq!(t.material.as_deref(), Some("PETG"));
    assert_eq!(t.colour.as_deref(), Some("#FF8000"));
    assert_eq!(t.grams, Some(4.56));
    assert_eq!(t.mm, Some(1493.99));
}

#[test]
fn prusaslicer_bgcode() {
    let p = parse_path(&fixture("prusaslicer_2.9.7_trimmed.bgcode"));
    let m = &p.meta;
    assert_eq!(m.format, "bgcode");
    assert_eq!(m.parse_status, "ok", "{:?}", m.parse_error);
    assert_eq!(m.slicer.as_deref(), Some("PrusaSlicer"));
    assert_eq!(m.slicer_version.as_deref(), Some("2.9.7"));
    assert_eq!(m.print_time_secs, Some(13 * 60 + 54));
    assert_eq!(m.nozzle_diameter, Some(0.4));
    assert_eq!(m.tools[0].material.as_deref(), Some("PETG"));
    assert_eq!(m.tools[0].grams, Some(4.56));
}

#[test]
fn bambu_studio_gcode() {
    let m = parse_path(&fixture("bambustudio_02.08_plate_1.gcode")).meta;
    assert_eq!(m.slicer.as_deref(), Some("BambuStudio"));
    assert_eq!(m.slicer_version.as_deref(), Some("02.08.02.61"));
    assert_eq!(m.print_time_secs, Some(150));
    assert_eq!(m.layer_count, Some(34));
    assert_eq!((m.bed_width, m.bed_depth), (Some(200.0), Some(200.0)));
    let t = &m.tools[0];
    assert_eq!(t.material.as_deref(), Some("PLA"));
    assert_eq!(t.colour.as_deref(), Some("#00AE42"));
    assert_eq!(t.mm, Some(59.75));
}

#[test]
fn cura_gcode() {
    let m = parse_path(&fixture("cura_5.7_synthetic.gcode")).meta;
    assert_eq!(m.slicer.as_deref(), Some("Cura"));
    assert_eq!(m.slicer_version.as_deref(), Some("5.7.1"));
    assert_eq!(m.print_time_secs, Some(3725));
    assert_eq!(m.layer_count, Some(100));
    assert_eq!(m.printer_model.as_deref(), Some("Creality Ender-3"));
    assert_eq!(m.tools[0].mm, Some(4218.71));
}

#[test]
fn spoolman_preset_becomes_spool_hint() {
    let dir = tempfile::TempDir::new().expect("dir");
    let text = fs::read_to_string(fixture("orca_2.3.0.gcode"))
        .expect("read")
        .replace(
            "\"Generic ASA Fillament PM grey\"",
            "\"Polymaker PolyLite ASA Grey (#12) - Spoolman\"",
        );
    let p = dir.path().join("spool.gcode");
    fs::write(&p, text).expect("write");
    let m = parse_path(&p).meta;
    assert_eq!(m.tools[0].spool_id, Some(12));
    assert_eq!(m.suggested_spool_id, Some(12));
}

/// A Bambu Studio `.gcode.3mf` built from real parts: slice_info.config, plate G-code, plate PNG.
fn build_3mf(path: &Path) {
    let png = parse_path(&fixture("orca_2.3.0.gcode"))
        .thumbnail
        .expect("png for the plate preview");
    let f = fs::File::create(path).expect("create");
    let mut zip = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default();
    for (name, bytes) in [
        (
            "Metadata/slice_info.config",
            fs::read(fixture("bambu_slice_info.config")).expect("read"),
        ),
        (
            "Metadata/plate_1.gcode",
            fs::read(fixture("bambustudio_02.08_plate_1.gcode")).expect("read"),
        ),
        ("Metadata/plate_1.png", png.clone()),
        ("Metadata/plate_1_small.png", png),
    ] {
        zip.start_file(name, opts).expect("start");
        zip.write_all(&bytes).expect("write");
    }
    zip.finish().expect("finish");
}

#[test]
fn bambu_3mf() {
    let dir = tempfile::TempDir::new().expect("dir");
    let path = dir.path().join("hoop.gcode.3mf");
    build_3mf(&path);
    let p = parse_path(&path);
    let m = &p.meta;
    assert_eq!(m.format, "3mf");
    assert_eq!(m.parse_status, "ok", "{:?}", m.parse_error);
    assert_eq!(m.slicer.as_deref(), Some("BambuStudio"));
    assert_eq!(m.printer_model_id.as_deref(), Some("N2S"));
    assert_eq!(m.nozzle_diameter, Some(0.4));
    assert_eq!(m.print_time_secs, Some(5691), "slice_info prediction wins");
    assert_eq!(m.plates.len(), 1);
    assert_eq!(m.plates[0].weight_g, Some(78.49));
    let t = &m.tools[0];
    assert_eq!(t.material.as_deref(), Some("PLA"));
    assert_eq!(t.colour.as_deref(), Some("#FFFFFF"));
    assert_eq!(t.grams, Some(78.49));
    assert_eq!(t.mm, Some(25_900.0));
    assert!(p.thumbnail.expect("plate png").starts_with(b"\x89PNG"));
}

#[test]
fn unknown_files_are_unsupported_not_errors() {
    let dir = tempfile::TempDir::new().expect("dir");
    let p = dir.path().join("notes.txt");
    fs::write(&p, "hello").expect("write");
    let m = parse_path(&p).meta;
    assert_eq!(
        (m.format.as_str(), m.parse_status.as_str()),
        ("other", "unsupported")
    );
    let bad = dir.path().join("broken.3mf");
    fs::write(&bad, "not a zip").expect("write");
    assert_eq!(parse_path(&bad).meta.parse_status, "error");
}

async fn store() -> (LibraryStore, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().expect("dir");
    let db = ForemanDb::connect(&dir.path().join("foreman.db"))
        .await
        .expect("db");
    let s = LibraryStore::new(dir.path(), db.pool).expect("store");
    (s, dir)
}

#[tokio::test]
async fn upload_parse_trash_restore() {
    let (s, dir) = store().await;
    let src = copy_to(dir.path(), "orca_2.3.0.gcode", "upload.tmp");
    let e = s
        .add_file("Slicer inbox", "carrier.gcode", &src)
        .await
        .expect("add");
    assert_eq!(e.path, "Slicer inbox/carrier.gcode");
    assert_eq!(e.format.as_deref(), Some("gcode"));
    assert!(e.thumb_url.is_some());
    assert_eq!(e.hash.as_ref().map(String::len), Some(64));
    let d = s.details(e.id).await.expect("details");
    assert_eq!(d.meta.expect("meta").slicer.as_deref(), Some("OrcaSlicer"));

    // Same name again → kept as a copy.
    let src = copy_to(dir.path(), "orca_2.3.0.gcode", "upload2.tmp");
    let e2 = s
        .add_file("Slicer inbox", "carrier.gcode", &src)
        .await
        .expect("add");
    assert_eq!(e2.path, "Slicer inbox/carrier (1).gcode");

    let folders = s.list("").await.expect("root");
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].kind, LibraryEntryKind::Folder);
    assert_eq!(s.list("Slicer inbox").await.expect("list").len(), 2);

    // Star, rename.
    assert!(s.set_starred(e.id, true).await.expect("star").starred);
    let renamed = s.rename(e.id, "LED carrier.gcode").await.expect("rename");
    assert_eq!(renamed.path, "Slicer inbox/LED carrier.gcode");

    // Trash the whole folder, then restore it.
    let inbox = folders[0].id;
    s.trash(inbox).await.expect("trash");
    assert!(s.list("").await.expect("root").is_empty());
    assert!(!dir.path().join("library/Slicer inbox").exists());
    let trash = s.list_trash().await.expect("trash list");
    assert_eq!(trash.len(), 1, "only the folder is a trash root");
    assert!(
        s.restore(e.id).await.is_err(),
        "children restore with their folder"
    );
    s.restore(inbox).await.expect("restore");
    assert_eq!(s.list("Slicer inbox").await.expect("list").len(), 2);
    assert!(dir
        .path()
        .join("library/Slicer inbox/LED carrier.gcode")
        .exists());
    assert!(
        s.get(e.id).await.expect("entry").starred,
        "star survives the trash"
    );

    // Purge: nothing younger than 30 days goes; everything older does.
    s.trash(e2.id).await.expect("trash file");
    assert_eq!(s.purge_trash(30).await.expect("purge"), 0);
    assert_eq!(s.purge_trash(-1).await.expect("purge"), 1);
    assert!(s.list_trash().await.expect("trash").is_empty());
    assert!(s.get(e2.id).await.is_err());
}

#[tokio::test]
async fn restore_into_a_reused_name_gets_a_suffix() {
    let (s, dir) = store().await;
    let a = s
        .add_file(
            "",
            "part.gcode",
            &copy_to(dir.path(), "cura_5.7_synthetic.gcode", "a.tmp"),
        )
        .await
        .expect("add");
    s.trash(a.id).await.expect("trash");
    s.add_file(
        "",
        "part.gcode",
        &copy_to(dir.path(), "cura_5.7_synthetic.gcode", "b.tmp"),
    )
    .await
    .expect("add again");
    let restored = s.restore(a.id).await.expect("restore");
    assert_eq!(restored.path, "part (1).gcode");
}

#[tokio::test]
async fn index_rebuild_matches_disk() {
    let (s, dir) = store().await;
    let lib = dir.path().join("library");
    fs::create_dir_all(lib.join("Calibration")).expect("mkdir");
    fs::copy(
        fixture("prusaslicer_2.9.7.gcode"),
        lib.join("Calibration/cube.gcode"),
    )
    .expect("copy");
    let r = s.rebuild_index().await.expect("rebuild");
    assert_eq!((r.added, r.updated, r.removed), (2, 0, 0));
    let files = s.list("Calibration").await.expect("list");
    assert_eq!(files[0].format.as_deref(), Some("gcode"));

    // Changed on disk → re-parsed; removed on disk → dropped; unchanged → untouched.
    fs::copy(
        fixture("orca_2.3.0.gcode"),
        lib.join("Calibration/cube.gcode"),
    )
    .expect("copy");
    let r = s.rebuild_index().await.expect("rebuild");
    assert_eq!((r.added, r.updated, r.removed), (0, 1, 0));
    let meta = s.meta(files[0].id).await.expect("meta").expect("some");
    assert_eq!(meta.slicer.as_deref(), Some("OrcaSlicer"));
    fs::remove_dir_all(lib.join("Calibration")).expect("rm");
    let r = s.rebuild_index().await.expect("rebuild");
    assert_eq!((r.added, r.updated, r.removed), (0, 0, 2));
    assert_eq!(
        s.rebuild_index().await.expect("rebuild"),
        Default::default()
    );
}
