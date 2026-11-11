//! Fixtures are real filaments from the owner's Spoolman (session 33).

use std::io::Read;

use super::*;

/// #11 Frem3D ASA GF Black, with the `orcaslicer_config` Marathon wrote.
fn frem3d_asa() -> ExportFilament {
    ExportFilament {
        id: 11,
        name: "Black".into(),
        vendor: Some("Frem3D".into()),
        material: Some("ASA".into()),
        modifiers: vec!["GF".into()],
        colours: vec!["000000".into()],
        density: 1.11,
        diameter: 1.75,
        extruder_temp: Some(260),
        bed_temp: Some(105),
        orca_config: Some(r#"{"is_soluble_filament":false,"support_material_interface_filament":false,"shrinkage_xy":"98.85","shrinkage_z":"100","activate_temperature_control":false,"nozzle_temp_min":"220","nozzle_temp_max":"270","bed_temp":"105","enable_pressure_advance":false,"slowdown_for_layer_cooling":false,"enable_overhang_fan":false,"activate_air_filtration":false,"retract_on_layer_change":false,"wipe_on_retract":false}"#.into()),
        ..Default::default()
    }
}

fn nextshapes_petg_hf() -> ExportFilament {
    ExportFilament {
        id: 236,
        name: "Silver".into(),
        vendor: Some("NextShapes".into()),
        material: Some("PETG HF".into()),
        colours: vec!["C0C0C0".into()],
        density: 1.27,
        diameter: 1.75,
        hue_td: Some(0.1),
        owned: true,
        ..Default::default()
    }
}

fn poly_alkemi_bicolor() -> ExportFilament {
    ExportFilament {
        id: 25,
        name: "Bicolor White Brown".into(),
        vendor: Some("Poly Alkemi".into()),
        material: Some("PLA".into()),
        modifiers: vec!["BiColor".into()],
        colours: vec!["FFFFFF".into(), "8B4513".into()],
        density: 1.24,
        diameter: 1.75,
        hue_td: Some(3.0),
        ..Default::default()
    }
}

fn s<'a>(v: &'a Value, k: &str) -> Vec<&'a str> {
    v[k].as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

#[test]
fn profile_has_the_keys_orca_needs_to_import() {
    let (v, info) = orca_profile(&frem3d_asa());
    assert_eq!(v["name"], json!(info.name));
    assert_eq!(s(&v, "filament_settings_id"), vec![info.name.as_str()]);
    assert_eq!(v["from"], "User");
    assert_eq!(v["type"], "filament");
    assert_eq!(v["version"], ORCA_VERSION);
    // inherits must name an OrcaFilamentLibrary preset or Orca skips the file.
    let inherits = v["inherits"].as_str().expect("inherits");
    assert!(PARENTS.iter().any(|p| p.0 == inherits), "{inherits}");
    assert_eq!(info.name, "Frem3D ASA GF Black (Foreman #11)");
}

#[test]
fn shrinkage_temps_and_flow_from_real_filament() {
    let (v, info) = orca_profile(&frem3d_asa());
    assert_eq!(s(&v, "filament_shrink"), vec!["98.85%"]);
    assert_eq!(s(&v, "filament_shrinkage_compensation_z"), vec!["100%"]);
    assert_eq!(info.shrinkage_xy, Some(98.85));
    assert_eq!(s(&v, "nozzle_temperature"), vec!["260"; ORCA_VARIANTS]);
    assert_eq!(s(&v, "nozzle_temperature_range_low"), vec!["220"]);
    assert_eq!(s(&v, "nozzle_temperature_range_high"), vec!["270"]);
    assert_eq!(s(&v, "hot_plate_temp"), vec!["105"]);
    assert_eq!(s(&v, "textured_plate_temp_initial_layer"), vec!["105"]);
    assert_eq!(s(&v, "enable_pressure_advance"), vec!["0"; ORCA_VARIANTS]);
    assert_eq!(s(&v, "filament_density"), vec!["1.11"]);
    assert_eq!(s(&v, "filament_diameter"), vec!["1.75"]);
    assert_eq!(s(&v, "default_filament_colour"), vec!["#000000"]);
    // No Generic ASA-GF in Orca 2.5: plain Generic ASA, Bambu GFB98.
    assert_eq!(info.inherits, "Generic ASA @System");
    assert_eq!(info.bambu_filament_id.as_deref(), Some("GFB98"));
    assert_eq!(v["filament_id"], "OFLPAxz3");
    // Flow ratio written per extruder variant when set (session 32 shape).
    let mut f = frem3d_asa();
    f.orca_config = Some(
        r#"{"filament_flow_ratio":0.95,"shrinkage_xy":"100.402","shrinkage_z":"100.2"}"#.into(),
    );
    let (v, info) = orca_profile(&f);
    assert_eq!(s(&v, "filament_flow_ratio"), vec!["0.95"; ORCA_VARIANTS]);
    assert_eq!(s(&v, "filament_shrink"), vec!["100.402%"]);
    assert_eq!(info.flow_ratio, Some(0.95));
    // Keys Orca does not know (Marathon's booleans) are not copied.
    for k in [
        "is_soluble_filament",
        "wipe_on_retract",
        "shrinkage_xy",
        "bed_temp",
    ] {
        assert!(v.get(k).is_none(), "{k}");
    }
}

#[test]
fn parent_by_material_and_modifier() {
    let pick = |material: &str, mods: &[&str], vendor: &str, name: &str| {
        let f = ExportFilament {
            id: 1,
            name: name.into(),
            vendor: Some(vendor.into()),
            material: Some(material.into()),
            modifiers: mods.iter().map(|m| (*m).into()).collect(),
            ..Default::default()
        };
        orca_profile(&f).1
    };
    assert_eq!(
        pick("PETG HF", &[], "NextShapes", "Silver").inherits,
        "Generic PETG HF @System"
    );
    assert_eq!(
        pick("PETG", &["CF"], "Sunlu", "PETG-CF Black").inherits,
        "Generic PETG-CF @System"
    );
    assert_eq!(
        pick("PLA", &["Silk"], "Sunlu", "Sunset").inherits,
        "Generic PLA Silk @System"
    );
    assert_eq!(
        pick("TPU95A", &[], "add:north", "Black").inherits,
        "Generic TPU @System"
    );
    assert_eq!(
        pick("PA12", &["CF"], "X", "Black").inherits,
        "Generic PA-CF @System"
    );
    assert_eq!(
        pick("PLA AF", &[], "Clas Ohlson", "Aluminium").inherits,
        "Generic PLA @System"
    );
    let b = pick("PLA", &["Matte"], "Bambu Lab", "Charcoal");
    assert_eq!(b.inherits, "Bambu PLA Matte @System");
    assert_eq!(b.bambu_filament_id.as_deref(), Some("GFA01"));
    assert_eq!(
        pick("PETG", &["HF"], "Bambu Lab", "Black")
            .bambu_filament_id
            .as_deref(),
        Some("GFG02")
    );
    let odd = pick("Nylon-X", &[], "X", "Y");
    assert_eq!(odd.inherits, "Generic PLA @System");
    assert!(!odd.notes.is_empty());
    // An explicit Bambu id in orcaslicer_config wins.
    let mut f = frem3d_asa();
    f.orca_config = Some(r#"{"bambu_filament_id":"GFB01"}"#.into());
    assert_eq!(orca_profile(&f).1.inherits, "Bambu ASA @System");
}

#[test]
fn multi_colour_lists_all_colours() {
    let (v, info) = orca_profile(&poly_alkemi_bicolor());
    assert!(info.multi_colour);
    assert_eq!(s(&v, "default_filament_colour"), vec!["#FFFFFF"]);
    let notes = s(&v, "filament_notes").join("");
    assert!(
        notes.contains("#FFFFFF") && notes.contains("#8B4513"),
        "{notes}"
    );
}

#[test]
fn names_are_unique_and_path_safe() {
    let mut a = nextshapes_petg_hf();
    a.name = "Silver / Grey: v2".into();
    let n = orca_preset_name(&a);
    assert!(!n.contains('/') && !n.contains(':'), "{n}");
    let mut b = a.clone();
    b.id = 20;
    assert_ne!(orca_preset_name(&a), orca_preset_name(&b));
    // Material already in the name is not repeated.
    let f = ExportFilament {
        id: 289,
        name: "PETG-CF Black".into(),
        vendor: Some("Sunlu".into()),
        material: Some("PETG".into()),
        modifiers: vec!["CF".into()],
        ..Default::default()
    };
    assert_eq!(
        orca_preset_name(&f),
        "Sunlu PETG CF PETG-CF Black (Foreman #289)"
    );
}

#[test]
fn zip_holds_one_json_profile_per_filament() {
    let fs = [frem3d_asa(), nextshapes_petg_hf(), poly_alkemi_bicolor()];
    let (bytes, infos) = orca_zip(&fs).expect("zip");
    assert_eq!(infos.len(), 3);
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    assert_eq!(z.len(), 3);
    for i in 0..z.len() {
        let mut e = z.by_index(i).expect("entry");
        assert!(e.name().expect("name").ends_with(".json"));
        let mut text = String::new();
        e.read_to_string(&mut text).expect("read");
        let v: Value = serde_json::from_str(&text).expect("json");
        assert!(v["inherits"].is_string() && v["name"].is_string());
    }
}

#[test]
fn hueforge_excludes_multi_colour_and_lists_missing_td() {
    let mut no_td = frem3d_asa();
    no_td.hue_td = None;
    let fs = [nextshapes_petg_hf(), poly_alkemi_bicolor(), no_td];
    let (v, skipped) = hueforge_catalogue(&fs);
    let list = v["Filaments"].as_array().expect("array");
    assert_eq!(list.len(), 1);
    let e = &list[0];
    assert_eq!(e["Brand"], "NextShapes");
    assert_eq!(e["Name"], "Silver");
    assert_eq!(e["Color"], "#c0c0c0");
    assert_eq!(e["Type"], "PETG");
    assert_eq!(e["Transmissivity"], 0.1);
    assert_eq!(e["Owned"], true);
    assert_eq!(e["Tags"], json!([]));
    let uuid = e["uuid"].as_str().expect("uuid");
    assert!(
        uuid.starts_with('{') && uuid.ends_with('}') && uuid.len() == 38,
        "{uuid}"
    );
    // Same filament → same uuid on every export.
    assert_eq!(hueforge_catalogue(&fs).0["Filaments"][0]["uuid"], e["uuid"]);
    // Key set is exactly HueForge's.
    let keys: Vec<&String> = e.as_object().expect("obj").keys().collect();
    assert_eq!(
        keys,
        [
            "Brand",
            "Color",
            "Name",
            "Owned",
            "Tags",
            "Transmissivity",
            "Type",
            "uuid"
        ]
    );
    assert_eq!(
        skipped
            .iter()
            .map(|s| (s.filament_id, s.reason))
            .collect::<Vec<_>>(),
        vec![
            (25, HueForgeSkip::MultiColour),
            (11, HueForgeSkip::MissingTd)
        ]
    );
}
