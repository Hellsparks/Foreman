//! OpenPrintTag NDEF/CBOR decoder for NFC-V (ISO 15693) tags.
//!
//! Mirrors `spoolman/tags/openprinttag.py`.  Only the fields Spoolman reads are
//! decoded; add a field here together with the code that uses it.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::OnceLock;

use ciborium::value::Value;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const OPENPRINTTAG_MIME: &[u8] = b"application/vnd.openprinttag";

// Material type enum — key → abbreviation.
fn material_type_map() -> &'static HashMap<i64, &'static str> {
    static MAP: OnceLock<HashMap<i64, &str>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (0, "PLA"),
            (1, "PETG"),
            (2, "TPU"),
            (3, "ABS"),
            (4, "ASA"),
            (5, "PC"),
            (6, "PCTG"),
            (7, "PP"),
            (8, "PA6"),
            (9, "PA11"),
            (10, "PA12"),
            (11, "PA66"),
            (12, "CPE"),
            (13, "TPE"),
            (14, "HIPS"),
            (15, "PHA"),
            (16, "PET"),
            (17, "PEI"),
            (18, "PBT"),
            (19, "PVB"),
            (20, "PVA"),
            (21, "PEKK"),
            (22, "PEEK"),
            (23, "BVOH"),
            (24, "TPC"),
            (25, "PPS"),
            (26, "PPSU"),
            (27, "PVC"),
            (28, "PEBA"),
            (29, "PVDF"),
            (30, "PPA"),
            (31, "PCL"),
            (32, "PES"),
            (33, "PMMA"),
            (34, "POM"),
            (35, "PPE"),
            (36, "PS"),
            (37, "PSU"),
            (38, "TPI"),
            (39, "SBS"),
            (40, "OBC"),
            (41, "EVA"),
            (42, "PA612"),
        ])
    })
}

fn material_class_map() -> &'static HashMap<i64, &'static str> {
    static MAP: OnceLock<HashMap<i64, &str>> = OnceLock::new();
    MAP.get_or_init(|| HashMap::from([(0, "FFF"), (1, "SLA")]))
}

// Main section field keys
const MF_INSTANCE_UUID: i64 = 0;
const MF_GTIN: i64 = 4;
const MF_MATERIAL_CLASS: i64 = 8;
const MF_MATERIAL_TYPE: i64 = 9;
const MF_MATERIAL_NAME: i64 = 10;
const MF_BRAND_NAME: i64 = 11;
const MF_NOMINAL_NETTO_FULL_WEIGHT: i64 = 16;
const MF_ACTUAL_NETTO_FULL_WEIGHT: i64 = 17;
const MF_EMPTY_CONTAINER_WEIGHT: i64 = 18;
const MF_PRIMARY_COLOR: i64 = 19;
const MF_DENSITY: i64 = 29;
const MF_FILAMENT_DIAMETER: i64 = 30;
const MF_MIN_PRINT_TEMPERATURE: i64 = 34;
const MF_MAX_PRINT_TEMPERATURE: i64 = 35;
const MF_MIN_BED_TEMPERATURE: i64 = 37;
const MF_MAX_BED_TEMPERATURE: i64 = 38;

// Meta section field keys
const META_AUX_REGION_OFFSET: i64 = 2;

// Aux section field keys
const AUX_CONSUMED_WEIGHT: i64 = 0;

// NFC-V Type 5 Tag TLV constants
const TLV_NDEF_MESSAGE: u8 = 0x03;
const TLV_TERMINATOR: u8 = 0xFE;
const TLV_THREE_BYTE_LENGTH: u8 = 0xFF;

// NDEF TNF: Media-type
const NDEF_TNF_MEDIA_TYPE: u8 = 0x02;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct OpenPrintTagData {
    pub instance_uuid: Option<String>,
    pub gtin: Option<i64>,
    pub material_class: Option<String>,
    pub material_type: Option<String>,
    pub material_name: Option<String>,
    pub brand_name: Option<String>,
    pub density: Option<f64>,
    pub filament_diameter: Option<f64>,
    pub nominal_netto_full_weight: Option<f64>,
    pub actual_netto_full_weight: Option<f64>,
    pub empty_container_weight: Option<f64>,
    pub consumed_weight: Option<f64>,
    pub primary_color_hex: Option<String>,
    pub min_print_temperature: Option<i64>,
    pub max_print_temperature: Option<i64>,
    pub min_bed_temperature: Option<i64>,
    pub max_bed_temperature: Option<i64>,
    pub nfc_tag_uid: Option<Vec<u8>>,
}

impl OpenPrintTagData {
    /// Filament diameter, defaulting to 1.75mm per spec.
    pub fn effective_diameter(&self) -> f64 {
        self.filament_diameter.unwrap_or(1.75)
    }

    /// Instance UUID, deriving from tag UID if not explicit.
    pub fn effective_instance_uuid(&self) -> Option<String> {
        if let Some(ref uuid) = self.instance_uuid {
            return Some(uuid.clone());
        }
        if let Some(ref uid) = self.nfc_tag_uid {
            return Some(uuid5_from_bytes(uid));
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Decode OpenPrintTag data from raw NFC-V tag memory.
pub fn decode_nfcv_memory(
    raw_bytes: &[u8],
    nfc_tag_uid: Option<&[u8]>,
) -> Result<OpenPrintTagData, String> {
    let payload = find_ndef_payload(raw_bytes)
        .ok_or_else(|| "Could not find OpenPrintTag NDEF record in tag memory".to_owned())?;

    let uid = nfc_tag_uid.map(|u| spec_order_uid(u).to_vec());
    let mut data = OpenPrintTagData {
        nfc_tag_uid: uid,
        ..Default::default()
    };

    // Parse meta section (first CBOR object)
    let (meta, meta_size) = decode_cbor_map(&payload)?;

    let main_offset = get_int(&meta, 0) // META_MAIN_REGION_OFFSET = 0
        .unwrap_or(meta_size as i64) as usize;
    let aux_offset = get_int(&meta, META_AUX_REGION_OFFSET);

    // Parse main section
    if main_offset < payload.len() {
        let (main_data, _) = decode_cbor_map(&payload[main_offset..])?;
        populate_main_fields(&mut data, &main_data)?;
    }

    // Parse aux section if present
    if let Some(aux_off) = aux_offset {
        let aux_off = aux_off as usize;
        if aux_off < payload.len() {
            if let Ok((aux_data, _)) = decode_cbor_map(&payload[aux_off..]) {
                if let Some(v) = find_value(&aux_data, AUX_CONSUMED_WEIGHT) {
                    if let Ok(w) = strict_number(v) {
                        data.consumed_weight = Some(w);
                    }
                }
            }
        }
    }

    Ok(data)
}

// ---------------------------------------------------------------------------
// CBOR helpers
// ---------------------------------------------------------------------------

/// Decode a CBOR map from bytes, return (map entries as Vec, bytes consumed).
fn decode_cbor_map(raw: &[u8]) -> Result<(Vec<(Value, Value)>, usize), String> {
    let mut cursor = Cursor::new(raw);
    let value: Value =
        ciborium::from_reader(&mut cursor).map_err(|e| format!("Invalid CBOR: {e}"))?;
    let consumed = cursor.position() as usize;

    match value {
        Value::Map(entries) => Ok((entries, consumed)),
        _ => Err(format!(
            "Expected a CBOR map, got {:?}",
            cbor_type_name(&value)
        )),
    }
}

fn cbor_type_name(v: &Value) -> &'static str {
    match v {
        Value::Integer(_) => "integer",
        Value::Float(_) => "float",
        Value::Bytes(_) => "bytes",
        Value::Text(_) => "text",
        Value::Array(_) => "array",
        Value::Map(_) => "map",
        Value::Bool(_) => "bool",
        Value::Null => "null",
        Value::Tag(_, _) => "tag",
        _ => "unknown",
    }
}

/// Get an integer value from a CBOR map by integer key (used for meta section).
fn get_int(map: &[(Value, Value)], key: i64) -> Option<i64> {
    cbor_int(find_value(map, key)?)
}

/// Look up a key in a CBOR map by integer key.
fn find_value(map: &[(Value, Value)], key: i64) -> Option<&Value> {
    for (k, v) in map {
        if cbor_int(k) == Some(key) {
            return Some(v);
        }
    }
    None
}

fn cbor_int(v: &Value) -> Option<i64> {
    match v {
        Value::Integer(i) => {
            let n: i128 = (*i).into();
            i64::try_from(n).ok()
        }
        _ => None,
    }
}

/// Strict number parser — mirrors upstream `_number()`.
///
/// Rejects bools, non-numeric types, bignums (> 2^53), NaN, and infinity.
/// A bad value makes the whole tag undecodable, matching upstream behavior.
fn strict_number(v: &Value) -> Result<f64, String> {
    match v {
        Value::Bool(_) => Err("Expected a number, got bool".into()),
        Value::Float(f) => {
            if f.is_finite() {
                Ok(*f)
            } else {
                Err("Expected a finite number".into())
            }
        }
        Value::Integer(i) => {
            let n: i128 = (*i).into();
            if n.unsigned_abs() > (1u128 << 53) {
                Err("Integer too large to be a physical measurement".into())
            } else {
                Ok(n as f64)
            }
        }
        Value::Tag(_, _) => Err("CBOR semantic tags are not supported".into()),
        _ => Err(format!("Expected a number, got {}", cbor_type_name(v))),
    }
}

/// Strict text parser — mirrors upstream `_text()`.
fn strict_text(v: &Value) -> Result<String, String> {
    match v {
        Value::Text(s) if s.len() <= 1024 => Ok(s.clone()),
        Value::Text(_) => Err("Text string too long".into()),
        _ => Err(format!("Expected a text string, got {}", cbor_type_name(v))),
    }
}

/// Strict integer parser — mirrors upstream `_int32()` / `_gtin()`.
fn strict_int(v: &Value) -> Result<i64, String> {
    match v {
        Value::Bool(_) => Err("Expected an integer, got bool".into()),
        Value::Integer(i) => {
            let n: i128 = (*i).into();
            i64::try_from(n).map_err(|_| "Integer out of range".to_owned())
        }
        _ => Err(format!("Expected an integer, got {}", cbor_type_name(v))),
    }
}

/// Get a bytes value from a CBOR map by integer key (no strict validation needed).
fn get_bytes(map: &[(Value, Value)], key: i64) -> Option<Vec<u8>> {
    match find_value(map, key)? {
        Value::Bytes(b) => Some(b.clone()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Main section field population
// ---------------------------------------------------------------------------

/// Populate OpenPrintTagData from the decoded main CBOR map.
///
/// Mirrors upstream `_populate_main_fields`. Strict about types: a field that
/// is present but has the wrong type (e.g. density is a list) makes the entire
/// tag undecodable, matching the upstream Python behavior where `_number()`,
/// `_text()`, etc. raise and propagate up.
fn populate_main_fields(
    data: &mut OpenPrintTagData,
    main: &[(Value, Value)],
) -> Result<(), String> {
    // UUID (bytes, lenient — bad UUIDs are just skipped)
    if let Some(bytes) = get_bytes(main, MF_INSTANCE_UUID) {
        if bytes.len() == 16 {
            data.instance_uuid = Some(uuid_from_bytes(&bytes));
        }
    }

    // GTIN (strict int, must be non-negative)
    if let Some(v) = find_value(main, MF_GTIN) {
        let n = strict_int(v)?;
        if n >= 0 {
            data.gtin = Some(n);
        }
    }

    // Material class (enum — strict int lookup)
    if let Some(v) = find_value(main, MF_MATERIAL_CLASS) {
        let idx = strict_int(v)?;
        data.material_class = material_class_map().get(&idx).map(|s| s.to_string());
    }

    // Material type (enum — strict int lookup)
    if let Some(v) = find_value(main, MF_MATERIAL_TYPE) {
        let idx = strict_int(v)?;
        data.material_type = material_type_map().get(&idx).map(|s| s.to_string());
    }

    // Text fields (strict)
    if let Some(v) = find_value(main, MF_MATERIAL_NAME) {
        data.material_name = Some(strict_text(v)?);
    }
    if let Some(v) = find_value(main, MF_BRAND_NAME) {
        data.brand_name = Some(strict_text(v)?);
    }

    // Numeric fields (strict)
    if let Some(v) = find_value(main, MF_DENSITY) {
        data.density = Some(strict_number(v)?);
    }
    if let Some(v) = find_value(main, MF_FILAMENT_DIAMETER) {
        data.filament_diameter = Some(strict_number(v)?);
    }
    if let Some(v) = find_value(main, MF_NOMINAL_NETTO_FULL_WEIGHT) {
        data.nominal_netto_full_weight = Some(strict_number(v)?);
    }
    if let Some(v) = find_value(main, MF_ACTUAL_NETTO_FULL_WEIGHT) {
        data.actual_netto_full_weight = Some(strict_number(v)?);
    }
    if let Some(v) = find_value(main, MF_EMPTY_CONTAINER_WEIGHT) {
        data.empty_container_weight = Some(strict_number(v)?);
    }

    // Color (RGBA bytes → hex RGB, lenient — bad color is just skipped)
    if let Some(rgba) = get_bytes(main, MF_PRIMARY_COLOR) {
        if rgba.len() >= 3 {
            data.primary_color_hex = Some(format!("{:02x}{:02x}{:02x}", rgba[0], rgba[1], rgba[2]));
        }
    }

    // Temperature integers (strict)
    if let Some(v) = find_value(main, MF_MIN_PRINT_TEMPERATURE) {
        data.min_print_temperature = Some(strict_int(v)?);
    }
    if let Some(v) = find_value(main, MF_MAX_PRINT_TEMPERATURE) {
        data.max_print_temperature = Some(strict_int(v)?);
    }
    if let Some(v) = find_value(main, MF_MIN_BED_TEMPERATURE) {
        data.min_bed_temperature = Some(strict_int(v)?);
    }
    if let Some(v) = find_value(main, MF_MAX_BED_TEMPERATURE) {
        data.max_bed_temperature = Some(strict_int(v)?);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// NFC-V / NDEF parsing
// ---------------------------------------------------------------------------

/// Extract OpenPrintTag NDEF record payload from NFC-V tag memory.
fn find_ndef_payload(raw: &[u8]) -> Option<Vec<u8>> {
    if raw.len() < 4 || raw[0] != 0xE1 {
        return None;
    }

    let mut pos = 4; // skip 4-byte capability container

    // Walk TLV blocks
    while pos < raw.len() {
        let tag = raw[pos];
        pos += 1;

        if tag == TLV_TERMINATOR {
            break;
        }
        if tag == 0x00 {
            continue; // null TLV
        }

        if pos >= raw.len() {
            break;
        }
        let mut tlv_len = raw[pos] as usize;
        pos += 1;

        if tlv_len as u8 == TLV_THREE_BYTE_LENGTH {
            if pos + 2 > raw.len() {
                break;
            }
            tlv_len = ((raw[pos] as usize) << 8) | (raw[pos + 1] as usize);
            pos += 2;
        }

        if tag == TLV_NDEF_MESSAGE {
            let end = (pos + tlv_len).min(raw.len());
            return parse_ndef_records(&raw[pos..end]);
        }

        pos += tlv_len;
    }

    None
}

/// Minimal NDEF parser — returns the payload of the application/vnd.openprinttag record.
fn parse_ndef_records(ndef_data: &[u8]) -> Option<Vec<u8>> {
    let mut pos = 0;
    while pos < ndef_data.len() {
        let header = ndef_data[pos];
        pos += 1;

        let message_end = header & 0x40 != 0;
        let short_record = header & 0x10 != 0;
        let id_present = header & 0x08 != 0;
        let tnf = header & 0x07;

        if pos >= ndef_data.len() {
            break;
        }
        let type_length = ndef_data[pos] as usize;
        pos += 1;

        // Payload length
        let payload_length;
        if short_record {
            if pos >= ndef_data.len() {
                break;
            }
            payload_length = ndef_data[pos] as usize;
            pos += 1;
        } else {
            if pos + 4 > ndef_data.len() {
                break;
            }
            payload_length = u32::from_be_bytes([
                ndef_data[pos],
                ndef_data[pos + 1],
                ndef_data[pos + 2],
                ndef_data[pos + 3],
            ]) as usize;
            pos += 4;
        }

        let mut id_length = 0usize;
        if id_present {
            if pos >= ndef_data.len() {
                break;
            }
            id_length = ndef_data[pos] as usize;
            pos += 1;
        }

        if pos + type_length > ndef_data.len() {
            break;
        }
        let record_type = &ndef_data[pos..pos + type_length];
        pos += type_length;

        pos += id_length; // skip ID

        if pos + payload_length > ndef_data.len() {
            break;
        }
        let payload = &ndef_data[pos..pos + payload_length];
        pos += payload_length;

        if tnf == NDEF_TNF_MEDIA_TYPE && record_type == OPENPRINTTAG_MIME {
            return Some(payload.to_vec());
        }

        if message_end {
            break;
        }
    }

    None
}

// ---------------------------------------------------------------------------
// UID helpers
// ---------------------------------------------------------------------------

const NFC_V_UID_LENGTH: usize = 8;
const NFC_V_UID_LEAD_BYTE: u8 = 0xE0;

/// Return an 8-byte NFC-V UID in specification order (starts with E0).
fn spec_order_uid(uid: &[u8]) -> Vec<u8> {
    if uid.len() != NFC_V_UID_LENGTH {
        return uid.to_vec();
    }
    if uid.last() == Some(&NFC_V_UID_LEAD_BYTE) && uid.first() != Some(&NFC_V_UID_LEAD_BYTE) {
        let mut reversed = uid.to_vec();
        reversed.reverse();
        return reversed;
    }
    uid.to_vec()
}

/// Derive a UUID v5 from a namespace + raw bytes.
fn uuid5_from_bytes(name: &[u8]) -> String {
    use std::fmt::Write;

    // UUID_NS_INSTANCE = "31062f81-b5bd-4f86-a5f8-46367e841508"
    let ns_bytes: [u8; 16] = [
        0x31, 0x06, 0x2f, 0x81, 0xb5, 0xbd, 0x4f, 0x86, 0xa5, 0xf8, 0x46, 0x36, 0x7e, 0x84, 0x15,
        0x08,
    ];

    // SHA-1 hash of namespace + name
    // We inline a minimal SHA-1 here to avoid adding a dependency.
    // The sha1 crate would be cleaner, but this is a single-use function.
    let digest = sha1_digest(&ns_bytes, name);

    // Take first 16 bytes, set version=5 and variant bits
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0F) | 0x50; // version 5
    bytes[8] = (bytes[8] & 0x3F) | 0x80; // variant 10

    let mut s = String::with_capacity(36);
    for (i, b) in bytes.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            s.push('-');
        }
        write!(s, "{b:02x}").ok();
    }
    s
}

fn uuid_from_bytes(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(36);
    for (i, b) in bytes.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            s.push('-');
        }
        write!(s, "{b:02x}").ok();
    }
    s
}

/// Minimal SHA-1 for UUID v5 derivation (namespace + name).
#[allow(clippy::chunks_exact_to_as_chunks, clippy::needless_range_loop)]
fn sha1_digest(ns: &[u8], name: &[u8]) -> [u8; 20] {
    // SHA-1 constants
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];

    let mut msg = Vec::with_capacity(ns.len() + name.len() + 64);
    msg.extend_from_slice(ns);
    msg.extend_from_slice(name);
    let bit_len = (msg.len() as u64) * 8;

    // Padding
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    // Process blocks
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = h;
        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w[i]);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    let mut digest = [0u8; 20];
    for (i, val) in h.iter().enumerate() {
        digest[i * 4..i * 4 + 4].copy_from_slice(&val.to_be_bytes());
    }
    digest
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic OpenPrintTag NFC-V memory dump, matching the Python
    /// `_openprinttag_fixtures.build_openprinttag`.
    fn build_openprinttag(main: &[(Value, Value)]) -> Vec<u8> {
        let meta = Value::Map(vec![]);
        let main_map = Value::Map(main.to_vec());

        let mut meta_bytes = Vec::new();
        ciborium::into_writer(&meta, &mut meta_bytes).unwrap();
        let mut main_bytes = Vec::new();
        ciborium::into_writer(&main_map, &mut main_bytes).unwrap();

        let payload = [meta_bytes, main_bytes].concat();
        let mime = b"application/vnd.openprinttag";
        // Short NDEF record: MB=1 ME=1 CF=0 SR=1 IL=0 TNF=0x02
        let header = 0xD2u8;
        let ndef = [
            &[header, mime.len() as u8, payload.len() as u8],
            &mime[..],
            &payload[..],
        ]
        .concat();
        // NFC-V memory: CC + NDEF TLV + terminator
        let cc = [0xE1u8, 0x40, 0x00, 0x01];
        let tlv = [&[0x03u8, ndef.len() as u8], &ndef[..], &[0xFEu8]].concat();
        [&cc[..], &tlv[..]].concat()
    }

    #[test]
    fn decode_pla_prusament() {
        let main = vec![
            (Value::Integer(8.into()), Value::Integer(0.into())), // material_class FFF
            (Value::Integer(9.into()), Value::Integer(0.into())), // material_type PLA
            (Value::Integer(11.into()), Value::Text("Prusament".into())), // brand_name
            (Value::Integer(16.into()), Value::Float(1000.0)),    // nominal weight
            (
                Value::Integer(19.into()),
                Value::Bytes(vec![0x11, 0x22, 0x33, 0xFF]),
            ), // primary color
        ];

        let mem = build_openprinttag(&main);
        let data = decode_nfcv_memory(&mem, None).expect("decode failed");

        assert_eq!(data.material_class.as_deref(), Some("FFF"));
        assert_eq!(data.material_type.as_deref(), Some("PLA"));
        assert_eq!(data.brand_name.as_deref(), Some("Prusament"));
        assert_eq!(data.nominal_netto_full_weight, Some(1000.0));
        assert_eq!(data.primary_color_hex.as_deref(), Some("112233"));
        assert_eq!(data.effective_diameter(), 1.75); // default
    }

    #[test]
    fn decode_garbage_returns_error() {
        let result = decode_nfcv_memory(b"not a tag", None);
        assert!(result.is_err());
    }

    #[test]
    fn full_decode_pipeline() {
        let main = vec![
            (Value::Integer(8.into()), Value::Integer(0.into())),
            (Value::Integer(9.into()), Value::Integer(0.into())),
            (Value::Integer(11.into()), Value::Text("Prusament".into())),
            (Value::Integer(16.into()), Value::Float(1000.0)),
            (
                Value::Integer(19.into()),
                Value::Bytes(vec![0x11, 0x22, 0x33, 0xFF]),
            ),
        ];

        let mem = build_openprinttag(&main);
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &mem);

        // Simulate what the API handler does
        let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64).unwrap();
        let decoded = crate::tags::decode(Some("openprinttag"), &raw, None).unwrap();

        assert_eq!(decoded.material_class.as_deref(), Some("FFF"));
        assert_eq!(decoded.material.as_deref(), Some("PLA"));
        assert_eq!(decoded.vendor_name.as_deref(), Some("Prusament"));
        assert_eq!(decoded.weight, Some(1000.0));
        assert_eq!(decoded.color_hex.as_deref(), Some("112233"));
    }
}
