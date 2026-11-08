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

/// Strip separators (`:`, `-`, `_`, space) and uppercase — matches upstream
/// `spoolman/tags.py::normalize_uid`.
pub fn normalize_uid(uid: &str) -> String {
    uid.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
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
    let mut q = sqlx::query(&sql);
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
    let uid_norm = normalize_uid(uid);
    if uid_norm.is_empty() {
        return Err(SpoolmanError::BadRequest("Invalid tag UID".into()));
    }

    // Verify spool exists.
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM spool WHERE id = ?")
        .bind(spool_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Err(SpoolmanError::NotFound(format!("spool {spool_id}")));
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
    let uid_norm = normalize_uid(uid);
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
    let uid_norm = normalize_uid(uid);
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
    let uid_norm = normalize_uid(uid);
    let filament_id: Option<i64> = sqlx::query_scalar(
        "SELECT filament_id FROM tag WHERE uid = ? AND target_type = 'filament' AND filament_id IS NOT NULL",
    )
    .bind(&uid_norm)
    .fetch_optional(pool)
    .await?;
    Ok(filament_id)
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
