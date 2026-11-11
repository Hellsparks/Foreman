//! Print-file facts: `.gcode` (Orca, Prusa, Bambu Studio, Cura), `.bgcode`, `.3mf`.
//!
//! Text G-code is read as a head (header, thumbnails, Bambu config block) plus a tail
//! (footer stats, Orca/Prusa config block). The body is streamed once for filament
//! slots only (colour changes, extrusion per colour, AMS tool changes; see `slots`).

use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

use base64::{prelude::BASE64_STANDARD, Engine as _};

use super::slots::{self, BodyScan};
use crate::domain::{FileMeta, PlateSummary, ToolUsage};

const HEAD_BYTES: usize = 1 << 20;
const TAIL_BYTES: usize = 512 << 10;

/// Parse result: facts plus the best thumbnail as PNG bytes.
#[derive(Debug, Default)]
pub struct Parsed {
    pub meta: FileMeta,
    pub thumbnail: Option<Vec<u8>>,
}

/// `gcode` | `bgcode` | `3mf` | `stl` | `other` from the file name.
pub fn format_of(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    if n.ends_with(".gcode") || n.ends_with(".gco") || n.ends_with(".g") {
        "gcode"
    } else if n.ends_with(".bgcode") {
        "bgcode"
    } else if n.ends_with(".3mf") {
        "3mf"
    } else if n.ends_with(".stl") {
        "stl"
    } else {
        "other"
    }
}

/// Parse a file on disk. Never fails: problems end up in `parse_status` / `parse_error`.
pub fn parse_path(path: &Path) -> Parsed {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let format = format_of(name);
    let result = match format {
        "gcode" => parse_gcode_file(path),
        "bgcode" => parse_bgcode_file(path),
        "3mf" => parse_3mf_file(path),
        "stl" => Ok(Parsed {
            meta: FileMeta {
                parse_status: "ok".into(),
                ..Default::default()
            },
            thumbnail: super::stl::render_png(path, 256).ok(),
        }),
        _ => Ok(Parsed {
            meta: FileMeta {
                parse_status: "unsupported".into(),
                ..Default::default()
            },
            thumbnail: None,
        }),
    };
    let mut parsed = result.unwrap_or_else(|e| Parsed {
        meta: FileMeta {
            parse_status: "error".into(),
            parse_error: Some(e),
            ..Default::default()
        },
        thumbnail: None,
    });
    parsed.meta.format = format.into();
    parsed
}

// ---------------------------------------------------------------------------
// Text G-code
// ---------------------------------------------------------------------------

fn parse_gcode_file(path: &Path) -> Result<Parsed, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len() as usize;
    let mut head = vec![0u8; len.min(HEAD_BYTES)];
    f.read_exact(&mut head).map_err(|e| e.to_string())?;
    let mut scan = Scan::default();
    scan.feed(&String::from_utf8_lossy(&head), false);
    if len > HEAD_BYTES {
        let tail_len = TAIL_BYTES.min(len - HEAD_BYTES);
        f.seek(SeekFrom::Start((len - tail_len) as u64))
            .map_err(|e| e.to_string())?;
        let mut tail = vec![0u8; tail_len];
        f.read_exact(&mut tail).map_err(|e| e.to_string())?;
        scan.feed(&String::from_utf8_lossy(&tail), true);
    }
    // Body pass for slots.
    f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut body = BodyScan::default();
    let mut buf = vec![0u8; 256 << 10];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        body.feed(&buf[..n]);
    }
    body.finish();
    Ok(scan.finish(Some(&body)))
}

/// Head + tail of a non-seekable stream (a G-code inside a 3MF); every byte also
/// goes through `body`.
fn head_tail<R: Read>(mut r: R, body: &mut BodyScan) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let mut head = Vec::with_capacity(HEAD_BYTES);
    (&mut r).take(HEAD_BYTES as u64).read_to_end(&mut head)?;
    body.feed(&head);
    let mut tail: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        body.feed(&buf[..n]);
        tail.extend_from_slice(&buf[..n]);
        if tail.len() > 2 * TAIL_BYTES {
            tail.drain(..tail.len() - TAIL_BYTES);
        }
    }
    if tail.len() > TAIL_BYTES {
        tail.drain(..tail.len() - TAIL_BYTES);
    }
    Ok((head, tail))
}

/// Header keys written as `; key: value` (Bambu/Orca header, Cura `;KEY:value`).
const COLON_KEYS: &[&str] = &[
    "total layer number",
    "total filament length [mm]",
    "total filament weight [g]",
    "TIME",
    "Filament used",
    "LAYER_COUNT",
    "TARGET_MACHINE.NAME",
];

#[derive(Default)]
struct Scan {
    slicer: Option<(String, Option<String>)>,
    kv: HashMap<String, String>,
    colon: HashMap<String, String>,
    /// (width, height, png)
    thumbs: Vec<(u32, u32, Vec<u8>)>,
}

impl Scan {
    /// Feed text; `partial_first` skips the first (possibly cut) line of a tail chunk.
    fn feed(&mut self, text: &str, partial_first: bool) {
        let mut thumb: Option<(u32, u32, String)> = None;
        for (i, raw) in text.lines().enumerate() {
            if partial_first && i == 0 {
                continue;
            }
            let line = raw.trim_end_matches('\r');
            if let Some((w, h, b64)) = thumb.as_mut() {
                if line.starts_with("; thumbnail end") || line.starts_with("; thumbnail_PNG end") {
                    if let Ok(png) = BASE64_STANDARD.decode(b64.as_bytes()) {
                        if png.starts_with(b"\x89PNG") {
                            self.thumbs.push((*w, *h, png));
                        }
                    }
                    thumb = None;
                } else if let Some(data) = line.strip_prefix("; ") {
                    b64.push_str(data.trim());
                }
                continue;
            }
            if let Some(rest) = line
                .strip_prefix("; thumbnail begin ")
                .or_else(|| line.strip_prefix("; thumbnail_PNG begin "))
            {
                let (w, h) = rest
                    .split_whitespace()
                    .next()
                    .and_then(|wh| wh.split_once('x'))
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .unwrap_or((0, 0));
                thumb = Some((w, h, String::new()));
                continue;
            }
            self.line(line);
        }
    }

    fn line(&mut self, line: &str) {
        let Some(body) = line.strip_prefix(';') else {
            return;
        };
        let body = body.trim_start();
        if self.slicer.is_none() {
            if let Some(s) = detect_slicer(body) {
                self.slicer = Some(s);
                return;
            }
        }
        if let Some((k, v)) = body.split_once(" = ") {
            let k = k.trim();
            if !k.is_empty() && k.len() <= 64 {
                self.kv
                    .entry(k.to_owned())
                    .or_insert_with(|| v.trim().to_owned());
            }
            return;
        }
        if let Some((k, v)) = body.split_once(':') {
            let k = k.trim();
            if COLON_KEYS.contains(&k) {
                self.colon
                    .entry(k.to_owned())
                    .or_insert_with(|| v.trim().to_owned());
            }
        }
    }

    fn finish(self, body: Option<&BodyScan>) -> Parsed {
        let mut meta = meta_from_maps(&self.kv, &self.colon);
        let (kind, slots) = slots::build(&meta.tools, body, semm(&self.kv), false, &[]);
        meta.slot_kind = kind;
        meta.slots = slots;
        if let Some((name, version)) = self.slicer {
            meta.slicer = Some(name);
            meta.slicer_version = version;
        }
        let thumbnail = self
            .thumbs
            .into_iter()
            .max_by_key(|(w, h, _)| u64::from(*w) * u64::from(*h))
            .map(|(_, _, png)| png);
        Parsed { meta, thumbnail }
    }
}

/// `generated by OrcaSlicer 2.3.0 on …`, `BambuStudio 02.08.02.61`,
/// `Generated with Cura_SteamEngine 5.7.1`.
fn detect_slicer(body: &str) -> Option<(String, Option<String>)> {
    let lower = body.to_ascii_lowercase();
    let words: Vec<&str> =
        if lower.starts_with("generated by ") || lower.starts_with("generated with ") {
            body.split_whitespace().skip(2).collect()
        } else if body.starts_with("BambuStudio ") {
            body.split_whitespace().collect()
        } else {
            return None;
        };
    let name = (*words.first()?).to_owned();
    let version = words
        .get(1)
        .map(|v| v.split('+').next().unwrap_or(v).to_owned());
    let name = match name.as_str() {
        "Cura_SteamEngine" => "Cura".to_owned(),
        _ => name,
    };
    Some((name, version))
}

// ---------------------------------------------------------------------------
// Turning key/value maps into FileMeta (shared by G-code, bgcode and 3MF)
// ---------------------------------------------------------------------------

fn unquote(v: &str) -> &str {
    v.trim().trim_matches('"')
}

fn non_empty(v: Option<&String>) -> Option<String> {
    v.map(|s| unquote(s).to_owned()).filter(|s| !s.is_empty())
}

/// `"A";"B"` / `A;B` / `1.2, 3.4` → items.
fn split_list(v: &str) -> Vec<String> {
    let sep = if v.contains(';') { ';' } else { ',' };
    v.split(sep).map(|s| unquote(s).to_owned()).collect()
}

fn first_number(v: Option<&String>) -> Option<f64> {
    v.and_then(|s| split_list(s).first().and_then(|x| x.trim().parse().ok()))
}

/// `1d 2h 3m 4s` → seconds.
pub fn parse_duration(v: &str) -> Option<i64> {
    let mut total = 0i64;
    let mut any = false;
    for tok in v.split_whitespace() {
        let (num, unit) = tok.split_at(tok.find(|c: char| !c.is_ascii_digit())?);
        let n: i64 = num.parse().ok()?;
        total += n * match unit {
            "d" => 86_400,
            "h" => 3_600,
            "m" => 60,
            "s" => 1,
            _ => return None,
        };
        any = true;
    }
    any.then_some(total)
}

/// `0x0,120x0,120x120,0x120` → (width, depth).
fn bed_size(v: &str) -> Option<(f64, f64)> {
    let pts: Vec<(f64, f64)> = v
        .split(',')
        .filter_map(|p| {
            let (x, y) = p.trim().split_once('x')?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .collect();
    if pts.len() < 3 {
        return None;
    }
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for (x, y) in pts {
        x0 = x0.min(x);
        x1 = x1.max(x);
        y0 = y0.min(y);
        y1 = y1.max(y);
    }
    Some((x1 - x0, y1 - y0))
}

/// `#RRGGBB` (drops alpha, upper-case); `None` for empty.
fn colour(v: &str) -> Option<String> {
    let v = unquote(v);
    let hex = v.strip_prefix('#')?;
    (hex.len() >= 6 && hex[..6].chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("#{}", hex[..6].to_ascii_uppercase()))
}

/// Orca Spoolman plugin preset names: `Vendor Name (#ID) - Spoolman`.
pub fn spool_hint(preset: &str) -> Option<i64> {
    let p = preset.trim();
    if !p.to_ascii_lowercase().ends_with("spoolman") {
        return None;
    }
    let start = p.rfind("(#")? + 2;
    let end = start + p[start..].find(')')?;
    p[start..end].parse().ok()
}

fn meta_from_maps(kv: &HashMap<String, String>, colon: &HashMap<String, String>) -> FileMeta {
    let get = |k: &str| kv.get(k);
    let list = |k: &str| get(k).map(|v| split_list(v)).unwrap_or_default();
    let nums =
        |k: &str| -> Vec<Option<f64>> { list(k).iter().map(|x| x.trim().parse().ok()).collect() };

    let materials = list("filament_type");
    let colours = list("filament_colour");
    let presets = list("filament_settings_id");
    let mut grams = nums("filament used [g]");
    let mut mm = nums("filament used [mm]");
    if grams.is_empty() {
        if let Some(g) = colon
            .get("total filament weight [g]")
            .and_then(|v| v.parse().ok())
        {
            grams = vec![Some(g)];
        }
    }
    if mm.is_empty() {
        if let Some(l) = colon
            .get("total filament length [mm]")
            .and_then(|v| v.parse().ok())
        {
            mm = vec![Some(l)];
        } else if let Some(cura) = colon.get("Filament used") {
            // Cura: `4.21871m` or `1.2m, 0.5m`
            mm = cura
                .split(',')
                .map(|m| {
                    m.trim()
                        .trim_end_matches('m')
                        .parse::<f64>()
                        .ok()
                        .map(|x| x * 1000.0)
                })
                .collect();
        }
    }
    let n = [
        materials.len(),
        colours.len(),
        presets.len(),
        grams.len(),
        mm.len(),
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    let tools: Vec<ToolUsage> = (0..n)
        .map(|i| {
            let preset = presets.get(i).cloned().filter(|s| !s.is_empty());
            ToolUsage {
                index: i as u32,
                material: materials.get(i).cloned().filter(|s| !s.is_empty()),
                colour: colours.get(i).and_then(|c| colour(c)),
                grams: grams.get(i).copied().flatten(),
                mm: mm.get(i).copied().flatten(),
                spool_id: preset.as_deref().and_then(spool_hint),
                preset,
            }
        })
        .collect();

    let print_time_secs = get("estimated printing time (normal mode)")
        .and_then(|v| parse_duration(v))
        .or_else(|| {
            colon
                .get("TIME")
                .and_then(|v| v.trim().parse::<f64>().ok())
                .map(|s| s as i64)
        });
    let layer_count = get("total layers count")
        .or_else(|| colon.get("total layer number"))
        .or_else(|| colon.get("LAYER_COUNT"))
        .and_then(|v| v.trim().parse().ok());
    let bed = get("printable_area")
        .or_else(|| get("bed_shape"))
        .and_then(|v| bed_size(unquote(v)));
    let suggested_spool_id = used(&tools).find_map(|t| t.spool_id);

    FileMeta {
        parse_status: "ok".into(),
        printer_model: non_empty(get("printer_model"))
            .or_else(|| non_empty(colon.get("TARGET_MACHINE.NAME"))),
        printer_settings_id: non_empty(get("printer_settings_id")),
        nozzle_diameter: first_number(get("nozzle_diameter")),
        print_time_secs,
        layer_count,
        bed_width: bed.map(|b| b.0),
        bed_depth: bed.map(|b| b.1),
        filament_settings_id: tools.iter().find_map(|t| t.preset.clone()),
        suggested_spool_id,
        tools,
        ..Default::default()
    }
}

/// `single_extruder_multi_material = 1` (MMU / AMS) in the config block.
fn semm(kv: &HashMap<String, String>) -> bool {
    kv.get("single_extruder_multi_material")
        .is_some_and(|v| unquote(v).trim() == "1")
}

/// Tools that actually print (unknown usage counts as used).
fn used(tools: &[ToolUsage]) -> impl Iterator<Item = &ToolUsage> {
    tools
        .iter()
        .filter(|t| t.grams.map(|g| g > 0.0).unwrap_or(true))
}

// ---------------------------------------------------------------------------
// Binary G-code (pure-Rust `binarygcode` reader; only metadata + thumbnail blocks)
// ---------------------------------------------------------------------------

fn parse_bgcode_file(path: &Path) -> Result<Parsed, String> {
    use binarygcode::{BlockKind, DeserialisedResult, Deserialiser};

    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut de = Deserialiser::default();
    let mut kv: HashMap<String, String> = HashMap::new();
    let mut thumbs: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    let mut buf = vec![0u8; 64 << 10];
    'read: loop {
        loop {
            match de.deserialise().map_err(|e| format!("bgcode: {e:?}"))? {
                DeserialisedResult::FileHeader(_) => {}
                DeserialisedResult::MoreBytesRequired(_) => break,
                DeserialisedResult::Block(b) => {
                    // Metadata and thumbnails come before the first G-code block.
                    if b.kind == BlockKind::GCode {
                        break 'read;
                    }
                    let data = b.decompress().map_err(|e| format!("bgcode block: {e:?}"))?;
                    if b.kind == BlockKind::Thumbnail {
                        let p = &b.parameters;
                        let format = p.get(0..2).map(|x| u16::from_le_bytes([x[0], x[1]]));
                        let w = p.get(2..4).map(|x| u16::from_le_bytes([x[0], x[1]]));
                        let h = p.get(4..6).map(|x| u16::from_le_bytes([x[0], x[1]]));
                        if let (Some(0), Some(w), Some(h)) = (format, w, h) {
                            thumbs.push((u32::from(w), u32::from(h), data.into_vec()));
                        }
                    } else {
                        for line in String::from_utf8_lossy(&data).lines() {
                            let line = line.trim_start_matches(';').trim();
                            if let Some((k, v)) = line.split_once('=') {
                                kv.entry(k.trim().to_owned())
                                    .or_insert_with(|| v.trim().to_owned());
                            }
                        }
                    }
                }
            }
        }
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        de.digest(&buf[..n]);
    }
    let mut meta = meta_from_maps(&kv, &HashMap::new());
    // The body is compressed G-code blocks; slots come from the metadata only.
    let (kind, slots) = slots::build(&meta.tools, None, semm(&kv), false, &[]);
    meta.slot_kind = kind;
    meta.slots = slots;
    if let Some(producer) = kv.get("Producer") {
        let mut parts = producer.split_whitespace();
        meta.slicer = parts.next().map(str::to_owned);
        meta.slicer_version = parts
            .next()
            .map(|v| v.split('+').next().unwrap_or(v).to_owned());
    }
    let thumbnail = thumbs
        .into_iter()
        .max_by_key(|(w, h, _)| u64::from(*w) * u64::from(*h))
        .map(|(_, _, png)| png);
    Ok(Parsed { meta, thumbnail })
}

// ---------------------------------------------------------------------------
// 3MF (zip): slice_info.config, plate G-code, project settings, plate PNGs
// ---------------------------------------------------------------------------

fn read_entry<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Option<Vec<u8>> {
    let entry = zip.by_name(name).ok()?;
    let mut out = Vec::new();
    entry.take(limit).read_to_end(&mut out).ok()?;
    Some(out)
}

fn parse_3mf_file(path: &Path) -> Result<Parsed, String> {
    let f = File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| format!("3mf: {e}"))?;
    parse_3mf(&mut zip)
}

fn plate_number(name: &str, prefix: &str, suffix: &str) -> Option<u32> {
    name.strip_prefix(prefix)?
        .strip_suffix(suffix)?
        .parse()
        .ok()
}

pub(crate) fn parse_3mf<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> Result<Parsed, String> {
    let names: Vec<String> = zip
        .file_names()
        .filter_map(|n| n.ok().map(|s| s.into_owned()))
        .collect();

    let slice_info = read_entry(zip, "Metadata/slice_info.config", 4 << 20)
        .map(|b| parse_slice_info(&String::from_utf8_lossy(&b)))
        .unwrap_or_default();

    // File-level facts from the first sliced plate's G-code, else the project settings.
    let mut plate_gcodes: Vec<(u32, &String)> = names
        .iter()
        .filter_map(|n| plate_number(n, "Metadata/plate_", ".gcode").map(|i| (i, n)))
        .collect();
    plate_gcodes.sort();
    let mut body = BodyScan::default();
    let mut semm_3mf = false;
    let mut meta = if let Some((_, name)) = plate_gcodes.first() {
        let entry = zip.by_name(name).map_err(|e| e.to_string())?;
        let (head, tail) = head_tail(entry, &mut body).map_err(|e| e.to_string())?;
        body.finish();
        let mut scan = Scan::default();
        scan.feed(&String::from_utf8_lossy(&head), false);
        if !tail.is_empty() {
            scan.feed(&String::from_utf8_lossy(&tail), true);
        }
        semm_3mf = semm(&scan.kv);
        scan.finish(Some(&body)).meta
    } else if let Some(json) = read_entry(zip, "Metadata/project_settings.config", 8 << 20) {
        meta_from_project_settings(&json)
    } else {
        FileMeta {
            parse_status: "ok".into(),
            ..Default::default()
        }
    };

    if meta.slicer.is_none() {
        if let Some(model) = read_entry(zip, "3D/3dmodel.model", 64 << 10) {
            if let Some((name, version)) = application(&String::from_utf8_lossy(&model)) {
                meta.slicer = Some(name);
                meta.slicer_version = version;
            }
        }
    }

    // slice_info.config is authoritative for Bambu/Orca plates.
    if let Some(first) = slice_info.plates.first() {
        meta.printer_model_id = slice_info.printer_model_id.clone();
        meta.nozzle_diameter = meta.nozzle_diameter.or(slice_info.nozzle);
        meta.print_time_secs = first.print_time_secs.or(meta.print_time_secs);
        if !first.tools.is_empty() {
            // Keep preset names from the G-code, usage from slice_info.
            let presets: Vec<Option<String>> =
                meta.tools.iter().map(|t| t.preset.clone()).collect();
            meta.tools = first
                .tools
                .iter()
                .map(|t| {
                    let preset = presets.get(t.index as usize).cloned().flatten();
                    ToolUsage {
                        spool_id: preset.as_deref().and_then(spool_hint),
                        preset,
                        ..t.clone()
                    }
                })
                .collect();
            meta.suggested_spool_id = used(&meta.tools).find_map(|t| t.spool_id);
        }
        meta.plates = slice_info.plates.clone();
    }
    // Slots from the final tools (slice_info wins); body scan when a plate G-code exists.
    let (kind, slots) = slots::build(
        &meta.tools,
        (!plate_gcodes.is_empty()).then_some(&body),
        semm_3mf,
        !slice_info.plates.is_empty(),
        slice_info.trays.first().map(Vec::as_slice).unwrap_or(&[]),
    );
    meta.slot_kind = kind;
    meta.slots = slots;
    if meta.slicer.is_none() && slice_info.client_version.is_some() {
        meta.slicer = Some("BambuStudio".into());
    }
    if meta.slicer_version.is_none() {
        meta.slicer_version = slice_info.client_version.clone();
    }

    let thumbnail = pick_3mf_thumbnail(zip, &names);
    Ok(Parsed { meta, thumbnail })
}

fn pick_3mf_thumbnail<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    names: &[String],
) -> Option<Vec<u8>> {
    let mut plates: Vec<(u32, &String)> = names
        .iter()
        .filter_map(|n| plate_number(n, "Metadata/plate_", ".png").map(|i| (i, n)))
        .collect();
    plates.sort();
    let fixed = [
        "Metadata/thumbnail.png",
        "Auxiliaries/.thumbnails/thumbnail_middle.png",
        "Auxiliaries/.thumbnails/thumbnail_3mf.png",
        "Thumbnails/thumbnail.png",
    ];
    let candidates: Vec<String> = plates
        .into_iter()
        .map(|(_, n)| n.clone())
        .chain(fixed.iter().map(|s| (*s).to_owned()))
        .collect();
    candidates
        .iter()
        .find_map(|n| read_entry(zip, n, 16 << 20).filter(|b| b.starts_with(b"\x89PNG")))
}

/// `<metadata name="Application">BambuStudio-02.08.02.61</metadata>`
fn application(model_xml: &str) -> Option<(String, Option<String>)> {
    let start = model_xml.find("name=\"Application\"")?;
    let rest = &model_xml[start..];
    let open = rest.find('>')? + 1;
    let close = rest[open..].find('<')? + open;
    let app = rest[open..close].trim();
    match app.split_once('-') {
        Some((name, version)) => Some((name.to_owned(), Some(version.to_owned()))),
        None => Some((app.to_owned(), None)),
    }
}

/// Bambu/Orca `Metadata/project_settings.config` (JSON) for unsliced projects.
fn meta_from_project_settings(json: &[u8]) -> FileMeta {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(json) else {
        return FileMeta {
            parse_status: "ok".into(),
            ..Default::default()
        };
    };
    let mut kv = HashMap::new();
    for key in [
        "printer_model",
        "printer_settings_id",
        "nozzle_diameter",
        "printable_area",
        "filament_type",
        "filament_colour",
        "filament_settings_id",
    ] {
        let joined = match v.get(key) {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Array(a)) => {
                let items: Vec<&str> = a.iter().filter_map(|x| x.as_str()).collect();
                // printable_area points are comma-joined; the other lists are per filament.
                items.join(if key == "printable_area" { "," } else { ";" })
            }
            _ => continue,
        };
        kv.insert(key.to_owned(), joined);
    }
    meta_from_maps(&kv, &HashMap::new())
}

#[derive(Debug, Default)]
struct SliceInfo {
    /// Per plate: (filament id 0-based, `tray_info_idx`).
    trays: Vec<Vec<(u32, String)>>,
    client_version: Option<String>,
    printer_model_id: Option<String>,
    nozzle: Option<f64>,
    plates: Vec<PlateSummary>,
}

fn attrs(e: &quick_xml::events::BytesStart<'_>) -> HashMap<String, String> {
    e.attributes()
        .flatten()
        .map(|a| (a.key.as_ref().to_owned(), a.value.as_ref().to_owned()))
        .collect()
}

fn parse_slice_info(xml: &str) -> SliceInfo {
    use quick_xml::events::Event;
    let mut info = SliceInfo::default();
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut plate: Option<PlateSummary> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == "plate" => {
                plate = Some(PlateSummary::default());
                info.trays.push(vec![]);
            }
            Ok(Event::End(e)) if e.name().as_ref() == "plate" => {
                if let Some(p) = plate.take() {
                    info.plates.push(p);
                }
            }
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                let a = attrs(&e);
                match (e.name().as_ref(), plate.as_mut()) {
                    ("header_item", _)
                        if a.get("key").map(String::as_str) == Some("X-BBL-Client-Version") =>
                    {
                        info.client_version = a.get("value").cloned();
                    }
                    ("metadata", Some(p)) => {
                        let value = a.get("value").cloned().unwrap_or_default();
                        match a.get("key").map(String::as_str) {
                            Some("index") => p.index = value.parse().unwrap_or(0),
                            Some("prediction") => p.print_time_secs = value.parse().ok(),
                            Some("weight") => p.weight_g = value.parse().ok(),
                            Some("printer_model_id") if info.printer_model_id.is_none() => {
                                info.printer_model_id = Some(value).filter(|v| !v.is_empty());
                            }
                            Some("nozzle_diameters") if info.nozzle.is_none() => {
                                info.nozzle =
                                    value.split(',').next().and_then(|v| v.trim().parse().ok());
                            }
                            _ => {}
                        }
                    }
                    ("filament", Some(p)) => {
                        let id: u32 = a.get("id").and_then(|v| v.parse().ok()).unwrap_or(1);
                        if let (Some(t), Some(trays)) = (
                            a.get("tray_info_idx").filter(|v| !v.is_empty()),
                            info.trays.last_mut(),
                        ) {
                            trays.push((id.saturating_sub(1), t.clone()));
                        }
                        p.tools.push(ToolUsage {
                            index: id.saturating_sub(1),
                            material: a.get("type").cloned().filter(|v| !v.is_empty()),
                            colour: a.get("color").and_then(|c| colour(c)),
                            grams: a.get("used_g").and_then(|v| v.parse().ok()),
                            mm: a
                                .get("used_m")
                                .and_then(|v| v.parse::<f64>().ok())
                                .map(|m| m * 1000.0),
                            ..Default::default()
                        });
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("12m 37s"), Some(757));
        assert_eq!(parse_duration("1d 2h 3m 4s"), Some(93_784));
        assert_eq!(parse_duration("2h"), Some(7200));
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("soon"), None);
    }

    #[test]
    fn spool_hints() {
        assert_eq!(
            spool_hint("Polymaker PolyLite PLA Grey (#12) - Spoolman"),
            Some(12)
        );
        assert_eq!(
            spool_hint("\"eSun PETG (#7) - Spoolman\""),
            None,
            "caller unquotes"
        );
        assert_eq!(
            spool_hint("Generic PLA (#12)"),
            None,
            "needs the Spoolman suffix"
        );
        assert_eq!(spool_hint("Generic ASA"), None);
    }

    #[test]
    fn formats() {
        assert_eq!(format_of("a.GCODE"), "gcode");
        assert_eq!(format_of("Cube.gcode.3mf"), "3mf");
        assert_eq!(format_of("x.bgcode"), "bgcode");
        assert_eq!(format_of("x.stl"), "stl");
        assert_eq!(format_of("readme.txt"), "other");
    }

    #[test]
    fn helpers() {
        assert_eq!(bed_size("0x0,120x0,120x120,0x120"), Some((120.0, 120.0)));
        assert_eq!(colour("#26a69aff").as_deref(), Some("#26A69A"));
        assert_eq!(colour("\"\""), None);
        assert_eq!(split_list("\"A\";\"B\""), vec!["A", "B"]);
        assert_eq!(split_list("1.2, 3.4"), vec!["1.2", "3.4"]);
    }
}
