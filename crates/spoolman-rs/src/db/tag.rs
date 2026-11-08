//! Tag database operations — link/unlink NFC/RFID tags to spools/filaments.
//!
//! The `tag` table schema (from migration `fe4970567bb3`):
//!   id, uid (UNIQUE), target_type, spool_id, filament_id,
//!   target_value, format, added (DATETIME)
//!
//! UIDs are normalised (separators stripped, uppercased) before storing.

use std::collections::HashMap;

use serde_json::Map;
use sqlx::AnyPool;

use crate::{
    db::{query::tp, vendor::utc_now_str, Engine},
    error::{Result, SpoolmanError},
    models::{
        CreateFilamentRequest, CreateSpoolRequest, CreateVendorRequest, SpoolResponse, TagInfo,
    },
    tags::{self, DecodedTag},
};

// ---------------------------------------------------------------------------
// UID normalisation
// ---------------------------------------------------------------------------

const UID_MAX_LENGTH: usize = 64;

/// Normalize a tag UID to uppercase hex, stripping separators.
///
/// Mirrors upstream `spoolman/tags/__init__.py::normalize_uid`.
/// Returns `Err(BadRequest)` if the UID is invalid.
pub fn normalize_uid(uid: &str) -> Result<String> {
    // Strip separators (space, colon, underscore, dot, hyphen) and optional 0x prefix.
    let stripped: String = uid
        .chars()
        .filter(|c| !matches!(c, ' ' | ':' | '_' | '.' | '-' | '\t' | '\n' | '\r'))
        .collect::<String>()
        .to_ascii_uppercase();
    let normalized = stripped.strip_prefix("0X").unwrap_or(&stripped).to_owned();

    if normalized.is_empty() {
        return Err(SpoolmanError::BadRequest(
            "A tag UID must not be empty.".into(),
        ));
    }
    if !normalized.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SpoolmanError::BadRequest(format!(
            "\"{uid}\" is not a valid tag UID. A UID must be hexadecimal."
        )));
    }
    if normalized.len() > UID_MAX_LENGTH {
        return Err(SpoolmanError::BadRequest(format!(
            "A tag UID can be at most {UID_MAX_LENGTH} hex characters."
        )));
    }
    if !normalized.len().is_multiple_of(2) {
        return Err(SpoolmanError::BadRequest(
            "A tag UID must contain a whole number of bytes (an even number of hex characters)."
                .into(),
        ));
    }
    Ok(normalized)
}

// ---------------------------------------------------------------------------
// Row helpers
// ---------------------------------------------------------------------------

fn row_to_tag_info(row: &sqlx::any::AnyRow) -> Result<TagInfo> {
    use sqlx::Row;
    Ok(TagInfo {
        id: row.try_get("id")?,
        uid: row.try_get("uid")?,
        format: row.try_get("format")?,
        added: row.try_get_unchecked("added")?,
    })
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

/// Fetch all tags linked to a specific spool.
pub async fn get_spool_tags(pool: &AnyPool, spool_id: i64) -> Result<Vec<TagInfo>> {
    let sql = tp("SELECT id, uid, format, CAST(added AS TEXT) AS added \
               FROM tag WHERE spool_id = ? AND target_type = 'spool' ORDER BY id");
    let rows = sqlx::query(sqlx::AssertSqlSafe(&*sql))
        .bind(spool_id)
        .fetch_all(pool)
        .await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in &rows {
        result.push(row_to_tag_info(row)?);
    }
    Ok(result)
}

/// Fetch tags for a batch of spool IDs.  Returns `HashMap<spool_id, tags>`.
pub async fn get_spool_tags_batch(
    pool: &AnyPool,
    spool_ids: &[i64],
) -> Result<HashMap<i64, Vec<TagInfo>>> {
    if spool_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = spool_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = tp(&format!(
        "SELECT spool_id, id, uid, format, CAST(added AS TEXT) AS added \
         FROM tag WHERE spool_id IN ({placeholders}) AND target_type = 'spool' ORDER BY id"
    ));
    let mut q = sqlx::query(sqlx::AssertSqlSafe(&*sql));
    for id in spool_ids {
        q = q.bind(*id);
    }
    let rows = q.fetch_all(pool).await?;

    let mut result: HashMap<i64, Vec<TagInfo>> = HashMap::new();
    for row in &rows {
        use sqlx::Row;
        let sid: i64 = row.try_get("spool_id")?;
        let tag = row_to_tag_info(row)?;
        result.entry(sid).or_default().push(tag);
    }
    Ok(result)
}

/// Link a UID to a spool.
///
/// Returns the new/existing `TagInfo`.
/// Errors:
/// - `BadRequest`: UID invalid
/// - `NotFound`: spool not found
/// - `Conflict` (returned as `BadRequest` with 409 message): UID already linked to something else
///
/// If the UID is already linked to THIS spool, it succeeds and may update the format.
pub async fn link_spool(
    pool: &AnyPool,
    spool_id: i64,
    uid: &str,
    format: Option<&str>,
) -> Result<TagInfo> {
    let uid_norm = normalize_uid(uid)?;

    // Verify spool exists.
    let exists: Option<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(&*tp(
        "SELECT id FROM spool WHERE id = ?",
    )))
    .bind(spool_id)
    .fetch_optional(pool)
    .await?;
    if exists.is_none() {
        return Err(SpoolmanError::NotFound(format!("spool with id {spool_id}")));
    }

    // Check if the UID is already in the tag table.
    let existing = sqlx::query(sqlx::AssertSqlSafe(&*tp(
        "SELECT id, uid, target_type, spool_id, filament_id, format, \
         CAST(added AS TEXT) AS added FROM tag WHERE uid = ?",
    )))
    .bind(&uid_norm)
    .fetch_optional(pool)
    .await?;

    if let Some(row) = existing {
        use sqlx::Row;
        let existing_spool_id: Option<i64> = row.try_get("spool_id")?;
        let existing_filament_id: Option<i64> = row.try_get("filament_id")?;
        let tag_id: i64 = row.try_get("id")?;

        if existing_spool_id == Some(spool_id) {
            // Re-link to same spool — may update format.
            if let Some(fmt) = format {
                sqlx::query(sqlx::AssertSqlSafe(&*tp(
                    "UPDATE tag SET format = ? WHERE id = ?",
                )))
                .bind(fmt)
                .bind(tag_id)
                .execute(pool)
                .await?;
            }
            return get_tag_by_id(pool, tag_id).await;
        }

        // Conflict: linked to a different spool or filament.
        let msg = if let Some(sid) = existing_spool_id {
            format!("Tag {uid_norm} is already linked to spool {sid}")
        } else if let Some(fid) = existing_filament_id {
            format!("Tag {uid_norm} is already linked to filament {fid}")
        } else {
            format!("Tag {uid_norm} is already linked to something else")
        };
        return Err(SpoolmanError::TagConflict {
            message: msg,
            spool_id: existing_spool_id,
            filament_id: existing_filament_id,
        });
    }

    // Insert new tag.
    let now = utc_now_str();
    let id: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(&*tp(
        "INSERT INTO tag (uid, target_type, spool_id, format, added) \
         VALUES (?, 'spool', ?, ?, ?) RETURNING id",
    )))
    .bind(&uid_norm)
    .bind(spool_id)
    .bind(format)
    .bind(&now)
    .fetch_one(pool)
    .await?;

    get_tag_by_id(pool, id).await
}

/// Unlink a UID from a spool.
/// Returns `NotFound` if no such tag exists for that spool.
pub async fn unlink_spool(pool: &AnyPool, spool_id: i64, uid: &str) -> Result<()> {
    let uid_norm = normalize_uid(uid)?;
    let result = sqlx::query(sqlx::AssertSqlSafe(&*tp(
        "DELETE FROM tag WHERE uid = ? AND spool_id = ? AND target_type = 'spool'",
    )))
    .bind(&uid_norm)
    .bind(spool_id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(SpoolmanError::NotFound(format!(
            "tag '{uid_norm}' on spool {spool_id}"
        )));
    }
    Ok(())
}

/// Look up a spool_id by tag UID. Returns `None` if not linked to a spool.
pub async fn find_spool_id_by_uid(pool: &AnyPool, uid: &str) -> Result<Option<i64>> {
    let uid_norm = normalize_uid(uid)?;
    let spool_id: Option<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(&*tp(
        "SELECT spool_id FROM tag WHERE uid = ? AND target_type = 'spool' AND spool_id IS NOT NULL",
    )))
    .bind(&uid_norm)
    .fetch_optional(pool)
    .await?;
    Ok(spool_id)
}

/// Look up a filament_id by tag UID. Returns `None` if not linked to a filament.
pub async fn find_filament_id_by_uid(pool: &AnyPool, uid: &str) -> Result<Option<i64>> {
    let uid_norm = normalize_uid(uid)?;
    let filament_id: Option<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(&*tp(
        "SELECT filament_id FROM tag WHERE uid = ? AND target_type = 'filament' AND filament_id IS NOT NULL",
    )))
    .bind(&uid_norm)
    .fetch_optional(pool)
    .await?;
    Ok(filament_id)
}

/// Fetch all tags linked to a specific filament.
pub async fn get_filament_tags(pool: &AnyPool, filament_id: i64) -> Result<Vec<TagInfo>> {
    let sql = tp("SELECT id, uid, format, CAST(added AS TEXT) AS added \
               FROM tag WHERE filament_id = ? AND target_type = 'filament' ORDER BY id");
    let rows = sqlx::query(sqlx::AssertSqlSafe(&*sql))
        .bind(filament_id)
        .fetch_all(pool)
        .await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in &rows {
        result.push(row_to_tag_info(row)?);
    }
    Ok(result)
}

/// Fetch tags for a batch of filament IDs.  Returns `HashMap<filament_id, tags>`.
pub async fn get_filament_tags_batch(
    pool: &AnyPool,
    filament_ids: &[i64],
) -> Result<HashMap<i64, Vec<TagInfo>>> {
    if filament_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = filament_ids
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let sql = tp(&format!(
        "SELECT filament_id, id, uid, format, CAST(added AS TEXT) AS added \
         FROM tag WHERE filament_id IN ({placeholders}) AND target_type = 'filament' ORDER BY id"
    ));
    let mut q = sqlx::query(sqlx::AssertSqlSafe(&*sql));
    for id in filament_ids {
        q = q.bind(*id);
    }
    let rows = q.fetch_all(pool).await?;

    let mut result: HashMap<i64, Vec<TagInfo>> = HashMap::new();
    for row in &rows {
        use sqlx::Row;
        let fid: i64 = row.try_get("filament_id")?;
        let tag = row_to_tag_info(row)?;
        result.entry(fid).or_default().push(tag);
    }
    Ok(result)
}

/// Link a UID to a filament.
///
/// Returns the new/existing `TagInfo`.  Mirrors `link_spool` semantics exactly.
pub async fn link_filament(
    pool: &AnyPool,
    filament_id: i64,
    uid: &str,
    format: Option<&str>,
) -> Result<TagInfo> {
    let uid_norm = normalize_uid(uid)?;

    // Verify filament exists.
    let exists: Option<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(&*tp(
        "SELECT id FROM filament WHERE id = ?",
    )))
    .bind(filament_id)
    .fetch_optional(pool)
    .await?;
    if exists.is_none() {
        return Err(SpoolmanError::NotFound(format!(
            "filament with id {filament_id}"
        )));
    }

    // Check if the UID is already in the tag table.
    let existing = sqlx::query(sqlx::AssertSqlSafe(&*tp(
        "SELECT id, uid, target_type, spool_id, filament_id, format, \
         CAST(added AS TEXT) AS added FROM tag WHERE uid = ?",
    )))
    .bind(&uid_norm)
    .fetch_optional(pool)
    .await?;

    if let Some(row) = existing {
        use sqlx::Row;
        let existing_spool_id: Option<i64> = row.try_get("spool_id")?;
        let existing_filament_id: Option<i64> = row.try_get("filament_id")?;
        let tag_id: i64 = row.try_get("id")?;

        if existing_filament_id == Some(filament_id) {
            // Re-link to same filament — may update format.
            if let Some(fmt) = format {
                sqlx::query(sqlx::AssertSqlSafe(&*tp(
                    "UPDATE tag SET format = ? WHERE id = ?",
                )))
                .bind(fmt)
                .bind(tag_id)
                .execute(pool)
                .await?;
            }
            return get_tag_by_id(pool, tag_id).await;
        }

        let msg = if let Some(sid) = existing_spool_id {
            format!("Tag {uid_norm} is already linked to spool {sid}")
        } else if let Some(fid) = existing_filament_id {
            format!("Tag {uid_norm} is already linked to filament {fid}")
        } else {
            format!("Tag {uid_norm} is already linked to something else")
        };
        return Err(SpoolmanError::TagConflict {
            message: msg,
            spool_id: existing_spool_id,
            filament_id: existing_filament_id,
        });
    }

    let now = utc_now_str();
    let id: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(&*tp(
        "INSERT INTO tag (uid, target_type, filament_id, format, added) \
         VALUES (?, 'filament', ?, ?, ?) RETURNING id",
    )))
    .bind(&uid_norm)
    .bind(filament_id)
    .bind(format)
    .bind(&now)
    .fetch_one(pool)
    .await?;

    get_tag_by_id(pool, id).await
}

/// Unlink a UID from a filament.
/// Returns `NotFound` if no such tag exists for that filament.
pub async fn unlink_filament(pool: &AnyPool, filament_id: i64, uid: &str) -> Result<()> {
    let uid_norm = normalize_uid(uid)?;
    let result = sqlx::query(sqlx::AssertSqlSafe(&*tp(
        "DELETE FROM tag WHERE uid = ? AND filament_id = ? AND target_type = 'filament'",
    )))
    .bind(&uid_norm)
    .bind(filament_id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(SpoolmanError::NotFound(format!(
            "tag '{uid_norm}' on filament {filament_id}"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Auto-create from decoded tag
// ---------------------------------------------------------------------------

/// Validation ranges matching upstream `spoolman/database/tag.py`.
const MIN_DIAMETER_MM: f64 = 0.5;
const MAX_DIAMETER_MM: f64 = 5.0;
const MIN_DENSITY: f64 = 0.3;
const MAX_DENSITY: f64 = 10.0;
const MIN_WEIGHT_G: f64 = 1.0;
const MAX_WEIGHT_G: f64 = 50_000.0;

/// Filament matching tolerances.
const DIAMETER_TOLERANCE: f64 = 0.01;
const WEIGHT_TOLERANCE: f64 = 1.0;

fn check_in_range(name: &str, value: Option<f64>, min: f64, max: f64) -> Result<()> {
    if let Some(v) = value {
        if !v.is_finite() || v < min || v > max {
            return Err(SpoolmanError::BadRequest(format!(
                "Decoded tag {name} ({v}) is outside the allowed range [{min}, {max}]"
            )));
        }
    }
    Ok(())
}

/// Create a spool from a decoded tag and link the tag to it.
///
/// Mirrors upstream `database/tag.py::create_spool_from_decoded_tag`:
/// find-or-create vendor → find-or-create filament → create spool → link tag.
pub async fn create_spool_from_decoded_tag(
    pool: &AnyPool,
    engine: &Engine,
    uid: &str,
    tag_format: Option<&str>,
    decoded: &DecodedTag,
) -> Result<SpoolResponse> {
    let uid_norm = normalize_uid(uid)?;

    // Density/diameter fallbacks
    let density = decoded
        .density
        .unwrap_or_else(|| tags::density_or_fallback(decoded.material.as_deref()));
    let diameter = decoded.diameter.unwrap_or(1.75);

    // Validate ranges
    if !density.is_finite() || !diameter.is_finite() {
        return Err(SpoolmanError::BadRequest(
            "Decoded tag density or diameter is not finite".into(),
        ));
    }
    check_in_range("diameter", Some(diameter), MIN_DIAMETER_MM, MAX_DIAMETER_MM)?;
    check_in_range("density", Some(density), MIN_DENSITY, MAX_DENSITY)?;
    check_in_range("weight", decoded.weight, MIN_WEIGHT_G, MAX_WEIGHT_G)?;
    check_in_range(
        "initial_weight",
        decoded.initial_weight,
        MIN_WEIGHT_G,
        MAX_WEIGHT_G,
    )?;
    check_in_range(
        "spool_weight",
        decoded.spool_weight,
        MIN_WEIGHT_G,
        MAX_WEIGHT_G,
    )?;

    // String length validation matching upstream Pydantic models (max_length=64).
    const MAX_NAME_LEN: usize = 64;
    if decoded
        .vendor_name
        .as_ref()
        .is_some_and(|n| n.len() > MAX_NAME_LEN)
    {
        return Err(SpoolmanError::BadRequest(
            "Decoded tag vendor_name exceeds 64 characters".into(),
        ));
    }
    if decoded
        .name
        .as_ref()
        .is_some_and(|n| n.len() > MAX_NAME_LEN)
    {
        return Err(SpoolmanError::BadRequest(
            "Decoded tag name exceeds 64 characters".into(),
        ));
    }
    if decoded
        .material
        .as_ref()
        .is_some_and(|n| n.len() > MAX_NAME_LEN)
    {
        return Err(SpoolmanError::BadRequest(
            "Decoded tag material exceeds 64 characters".into(),
        ));
    }

    // --- Find or create vendor ---
    let vendor_id = if let Some(ref vendor_name) = decoded.vendor_name {
        let existing: Option<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(&*tp(
            "SELECT id FROM vendor WHERE name = ? LIMIT 1",
        )))
        .bind(vendor_name)
        .fetch_optional(pool)
        .await?;

        match existing {
            Some(id) => Some(id),
            None => {
                let vendor = crate::db::vendor::create(
                    pool,
                    engine,
                    &CreateVendorRequest {
                        name: vendor_name.clone(),
                        comment: None,
                        empty_spool_weight: None,
                        external_id: None,
                        extra: Map::new(),
                    },
                )
                .await?;
                Some(vendor.id)
            }
        }
    } else {
        None
    };

    // --- Find or create filament ---
    let filament_id = match find_filament_for_product(pool, decoded, vendor_id, diameter).await? {
        Some(id) => id,
        None => {
            let filament = crate::db::filament::create(
                pool,
                engine,
                &CreateFilamentRequest {
                    density,
                    diameter,
                    name: decoded.name.clone(),
                    vendor_id,
                    material: decoded.material.clone(),
                    price: None,
                    weight: decoded.weight,
                    spool_weight: decoded.spool_weight,
                    article_number: decoded.article_number.clone(),
                    comment: None,
                    settings_extruder_temp: decoded.extruder_temp,
                    settings_bed_temp: decoded.bed_temp,
                    color_hex: decoded.color_hex.clone(),
                    multi_color_hexes: None,
                    multi_color_direction: None,
                    external_id: None,
                    extra: Map::new(),
                },
            )
            .await?;
            filament.id
        }
    };

    // --- Create spool ---
    let spool = crate::db::spool::create(
        pool,
        engine,
        &CreateSpoolRequest {
            filament_id,
            used_weight: None,
            remaining_weight: None,
            initial_weight: decoded.initial_weight,
            spool_weight: decoded.spool_weight,
            first_used: None,
            last_used: None,
            location: None,
            lot_nr: None,
            comment: None,
            archived: None,
            price: None,
            extra: Map::new(),
        },
    )
    .await?;

    // --- Link tag to spool ---
    link_spool(pool, spool.id, &uid_norm, tag_format).await?;

    // Re-fetch to get the full spool (with filament + vendor populated).
    crate::db::spool::find_by_id(pool, spool.id).await
}

/// Find an existing filament that matches this decoded product.
async fn find_filament_for_product(
    pool: &AnyPool,
    decoded: &DecodedTag,
    vendor_id: Option<i64>,
    diameter: f64,
) -> Result<Option<i64>> {
    // Without a name or material, no product match is possible.
    let name = match decoded.name.as_deref() {
        Some(n) => n,
        None => return Ok(None),
    };
    let material = match decoded.material.as_deref() {
        Some(m) => m,
        None => return Ok(None),
    };

    // Query candidates with exact match on name, material, vendor_id, color_hex.
    let sql = if vendor_id.is_some() {
        tp("SELECT id, diameter, weight FROM filament \
         WHERE name = ? AND material = ? AND vendor_id = ? AND \
         (color_hex = ? OR (color_hex IS NULL AND ? IS NULL))")
    } else {
        tp("SELECT id, diameter, weight FROM filament \
         WHERE name = ? AND material = ? AND vendor_id IS NULL AND \
         (color_hex = ? OR (color_hex IS NULL AND ? IS NULL))")
    };

    let mut q = sqlx::query(sqlx::AssertSqlSafe(&*sql))
        .bind(name)
        .bind(material);
    if let Some(vid) = vendor_id {
        q = q.bind(vid);
    }
    q = q.bind(&decoded.color_hex).bind(&decoded.color_hex);

    let rows = q.fetch_all(pool).await?;

    for row in &rows {
        use sqlx::Row;
        let fid: i64 = row.try_get("id")?;
        let f_diameter: Option<f64> = row.try_get("diameter").ok();
        let f_weight: Option<f64> = row.try_get("weight").ok();

        let diameter_match = match f_diameter {
            Some(fd) => (fd - diameter).abs() <= DIAMETER_TOLERANCE,
            None => false,
        };
        let weight_match = match (f_weight, decoded.weight) {
            (Some(fw), Some(dw)) => (fw - dw).abs() <= WEIGHT_TOLERANCE,
            (None, None) => true,
            _ => false,
        };

        if diameter_match && weight_match {
            return Ok(Some(fid));
        }
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

async fn get_tag_by_id(pool: &AnyPool, id: i64) -> Result<TagInfo> {
    let row = sqlx::query(sqlx::AssertSqlSafe(&*tp(
        "SELECT id, uid, format, CAST(added AS TEXT) AS added FROM tag WHERE id = ?",
    )))
    .bind(id)
    .fetch_one(pool)
    .await?;
    row_to_tag_info(&row)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{DbType, SpoolmanConfig},
        db::Db,
        tags::DecodedTag,
    };
    use tempfile::TempDir;

    fn config_for_dir(dir: &std::path::Path) -> SpoolmanConfig {
        SpoolmanConfig {
            db_type: DbType::Sqlite,
            db_host: None,
            db_port: None,
            db_name: None,
            db_username: None,
            db_password: None,
            data_dir: dir.to_path_buf(),
            logs_dir: dir.to_path_buf(),
            backups_dir: dir.join("backups"),
            host: "0.0.0.0".to_owned(),
            port: 7912,
            base_path: String::new(),
            automatic_backup: false,
        }
    }

    /// Regression test for the create=true tag scan path.
    ///
    /// Mirrors Python `test_create_true_on_an_unmatched_decodable_tag_creates_and_links_a_spool`.
    /// Creates vendor + filament + spool from a decoded FFF/PLA tag, then
    /// verifies the whole chain exists and the tag is linked.
    #[tokio::test]
    async fn create_spool_from_decoded_tag_creates_full_chain() {
        let dir = TempDir::new().unwrap();
        let db = Db::connect(&config_for_dir(dir.path())).await.unwrap();

        let decoded = DecodedTag {
            material_class: Some("FFF".to_owned()),
            material: Some("PLA".to_owned()),
            name: Some("PLA Galaxy Black".to_owned()),
            vendor_name: Some("Prusament".to_owned()),
            article_number: None,
            color_hex: Some("112233".to_owned()),
            diameter: Some(1.75),
            density: Some(1.24),
            weight: Some(1000.0),
            initial_weight: Some(980.0),
            spool_weight: Some(200.0),
            used_weight: None,
            extruder_temp: Some(215),
            bed_temp: Some(60),
            instance_uuid: None,
        };

        let uid = "E0040150006F4567";

        // Create spool from decoded tag.
        let spool = create_spool_from_decoded_tag(
            &db.pool,
            &db.engine,
            uid,
            Some("openprinttag"),
            &decoded,
        )
        .await
        .expect("create_spool_from_decoded_tag failed");

        // Verify spool was created.
        assert!(spool.id > 0);
        assert_eq!(spool.initial_weight, Some(980.0));
        assert_eq!(spool.spool_weight, Some(200.0));

        // Verify filament was created with correct fields.
        let filament = &spool.filament;
        assert_eq!(filament.name.as_deref(), Some("PLA Galaxy Black"));
        assert_eq!(filament.material.as_deref(), Some("PLA"));
        assert_eq!(filament.color_hex.as_deref(), Some("112233"));
        assert!((filament.diameter - 1.75).abs() < f64::EPSILON);
        assert!((filament.density - 1.24).abs() < f64::EPSILON);
        assert_eq!(filament.weight, Some(1000.0));
        assert_eq!(filament.spool_weight, Some(200.0));
        assert_eq!(filament.settings_extruder_temp, Some(215));
        assert_eq!(filament.settings_bed_temp, Some(60));

        // Verify vendor was created.
        let vendor = filament
            .vendor
            .as_ref()
            .expect("filament should have vendor");
        assert_eq!(vendor.name, "Prusament");

        // Verify tag is linked.
        let tags = get_spool_tags(&db.pool, spool.id).await.unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].uid, uid);

        // Second call with the same UID should fail with TagConflict.
        let err = create_spool_from_decoded_tag(
            &db.pool,
            &db.engine,
            uid,
            Some("openprinttag"),
            &decoded,
        )
        .await
        .expect_err("should conflict on duplicate UID");
        assert!(
            matches!(err, SpoolmanError::TagConflict { .. }),
            "expected TagConflict, got: {err}"
        );
    }

    /// When a matching filament already exists, the create path reuses it
    /// instead of creating a duplicate.
    #[tokio::test]
    async fn create_spool_reuses_existing_filament() {
        let dir = TempDir::new().unwrap();
        let db = Db::connect(&config_for_dir(dir.path())).await.unwrap();

        let decoded = DecodedTag {
            material_class: Some("FFF".to_owned()),
            material: Some("PETG".to_owned()),
            name: Some("PETG Orange".to_owned()),
            vendor_name: Some("Prusa".to_owned()),
            article_number: None,
            color_hex: Some("FF6600".to_owned()),
            diameter: Some(1.75),
            density: Some(1.27),
            weight: Some(1000.0),
            initial_weight: None,
            spool_weight: None,
            used_weight: None,
            extruder_temp: None,
            bed_temp: None,
            instance_uuid: None,
        };

        // First spool.
        let spool1 = create_spool_from_decoded_tag(
            &db.pool,
            &db.engine,
            "AA11BB22CC33DD01",
            Some("openprinttag"),
            &decoded,
        )
        .await
        .unwrap();

        // Second spool with a different UID but same product.
        let spool2 = create_spool_from_decoded_tag(
            &db.pool,
            &db.engine,
            "AA11BB22CC33DD02",
            Some("openprinttag"),
            &decoded,
        )
        .await
        .unwrap();

        // Both spools should share the same filament.
        assert_eq!(
            spool1.filament.id, spool2.filament.id,
            "second spool should reuse existing filament"
        );
    }
}
