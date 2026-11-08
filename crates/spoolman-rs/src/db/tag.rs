//! Tag database operations — link/unlink NFC/RFID tags to spools/filaments.
//!
//! The `tag` table schema (from migration `fe4970567bb3`):
//!   id, uid (UNIQUE), target_type, spool_id, filament_id,
//!   target_value, format, added (DATETIME)
//!
//! UIDs are normalised (separators stripped, uppercased) before storing.

use std::collections::HashMap;

use sqlx::AnyPool;

use crate::{
    db::vendor::utc_now_str,
    error::{Result, SpoolmanError},
    models::TagInfo,
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
    let sql = "SELECT id, uid, format, CAST(added AS TEXT) AS added \
               FROM tag WHERE spool_id = ? AND target_type = 'spool' ORDER BY id";
    let rows = sqlx::query(sql).bind(spool_id).fetch_all(pool).await?;
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
    let sql = format!(
        "SELECT spool_id, id, uid, format, CAST(added AS TEXT) AS added \
         FROM tag WHERE spool_id IN ({placeholders}) AND target_type = 'spool' ORDER BY id"
    );
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
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM spool WHERE id = ?")
        .bind(spool_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Err(SpoolmanError::NotFound(format!("spool with id {spool_id}")));
    }

    // Check if the UID is already in the tag table.
    let existing = sqlx::query(
        "SELECT id, uid, target_type, spool_id, filament_id, format, \
         CAST(added AS TEXT) AS added FROM tag WHERE uid = ?",
    )
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
                sqlx::query("UPDATE tag SET format = ? WHERE id = ?")
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
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO tag (uid, target_type, spool_id, format, added) \
         VALUES (?, 'spool', ?, ?, ?) RETURNING id",
    )
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
    let result =
        sqlx::query("DELETE FROM tag WHERE uid = ? AND spool_id = ? AND target_type = 'spool'")
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
    let spool_id: Option<i64> = sqlx::query_scalar(
        "SELECT spool_id FROM tag WHERE uid = ? AND target_type = 'spool' AND spool_id IS NOT NULL",
    )
    .bind(&uid_norm)
    .fetch_optional(pool)
    .await?;
    Ok(spool_id)
}

/// Look up a filament_id by tag UID. Returns `None` if not linked to a filament.
pub async fn find_filament_id_by_uid(pool: &AnyPool, uid: &str) -> Result<Option<i64>> {
    let uid_norm = normalize_uid(uid)?;
    let filament_id: Option<i64> = sqlx::query_scalar(
        "SELECT filament_id FROM tag WHERE uid = ? AND target_type = 'filament' AND filament_id IS NOT NULL",
    )
    .bind(&uid_norm)
    .fetch_optional(pool)
    .await?;
    Ok(filament_id)
}

/// Fetch all tags linked to a specific filament.
pub async fn get_filament_tags(pool: &AnyPool, filament_id: i64) -> Result<Vec<TagInfo>> {
    let sql = "SELECT id, uid, format, CAST(added AS TEXT) AS added \
               FROM tag WHERE filament_id = ? AND target_type = 'filament' ORDER BY id";
    let rows = sqlx::query(sql).bind(filament_id).fetch_all(pool).await?;
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
    let sql = format!(
        "SELECT filament_id, id, uid, format, CAST(added AS TEXT) AS added \
         FROM tag WHERE filament_id IN ({placeholders}) AND target_type = 'filament' ORDER BY id"
    );
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
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM filament WHERE id = ?")
        .bind(filament_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Err(SpoolmanError::NotFound(format!(
            "filament with id {filament_id}"
        )));
    }

    // Check if the UID is already in the tag table.
    let existing = sqlx::query(
        "SELECT id, uid, target_type, spool_id, filament_id, format, \
         CAST(added AS TEXT) AS added FROM tag WHERE uid = ?",
    )
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
                sqlx::query("UPDATE tag SET format = ? WHERE id = ?")
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
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO tag (uid, target_type, filament_id, format, added) \
         VALUES (?, 'filament', ?, ?, ?) RETURNING id",
    )
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
    let result = sqlx::query(
        "DELETE FROM tag WHERE uid = ? AND filament_id = ? AND target_type = 'filament'",
    )
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
// Private helpers
// ---------------------------------------------------------------------------

async fn get_tag_by_id(pool: &AnyPool, id: i64) -> Result<TagInfo> {
    let row =
        sqlx::query("SELECT id, uid, format, CAST(added AS TEXT) AS added FROM tag WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await?;
    row_to_tag_info(&row)
}
