//! Repository functions for the `queue_item` table (task 3.5).
//!
//! A lane is `printer_id` (NULL = fleet queue). Only `waiting` items are ordered by
//! `position`; `sending` / `printing` items occupy their printer. Status changes that
//! race (scheduler vs. user) go through [`transition`], a conditional update.

use chrono::Utc;
use sqlx::{FromRow, SqlitePool};

use crate::{
    domain::{QueueItem, QueueSource, QueueStatus},
    error::AppError,
};

#[derive(FromRow)]
struct QueueRow {
    id: i64,
    file_id: i64,
    file_name: String,
    source: String,
    added_by: String,
    added_at: String,
    status: String,
    printer_id: Option<i64>,
    from_fleet: i64,
    position: i64,
    error: Option<String>,
    updated_at: String,
}

impl QueueRow {
    fn into_domain(self) -> QueueItem {
        QueueItem {
            id: self.id,
            file_id: self.file_id,
            file_name: self.file_name,
            source: QueueSource::parse(&self.source),
            added_by: self.added_by,
            added_at: self.added_at,
            status: QueueStatus::parse(&self.status),
            printer_id: self.printer_id,
            from_fleet: self.from_fleet != 0,
            position: self.position,
            error: self.error,
            updated_at: self.updated_at,
        }
    }
}

/// `SELECT <all columns> FROM queue_item <rest>` as a static string.
macro_rules! select_items {
    ($rest:literal) => {
        concat!(
            "SELECT id, file_id, file_name, source, added_by, added_at, status, printer_id, \
             from_fleet, position, error, updated_at FROM queue_item ",
            $rest
        )
    };
}

fn db(e: sqlx::Error) -> AppError {
    AppError::Database(e.to_string())
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

/// Add an item at the end of a lane (`printer_id` None = fleet queue).
pub async fn add(
    pool: &SqlitePool,
    file_id: i64,
    file_name: &str,
    printer_id: Option<i64>,
    source: QueueSource,
    added_by: &str,
) -> Result<QueueItem, AppError> {
    let ts = now();
    let next: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM queue_item \
         WHERE status = 'waiting' AND printer_id IS ?",
    )
    .bind(printer_id)
    .fetch_one(pool)
    .await
    .map_err(db)?;
    let id = sqlx::query(
        "INSERT INTO queue_item (file_id, file_name, source, added_by, added_at, status, \
         printer_id, from_fleet, position, updated_at) VALUES (?, ?, ?, ?, ?, 'waiting', ?, 0, ?, ?)",
    )
    .bind(file_id)
    .bind(file_name)
    .bind(source.as_str())
    .bind(added_by)
    .bind(&ts)
    .bind(printer_id)
    .bind(next)
    .bind(&ts)
    .execute(pool)
    .await
    .map_err(db)?
    .last_insert_rowid();
    get(pool, id).await
}

pub async fn get(pool: &SqlitePool, id: i64) -> Result<QueueItem, AppError> {
    let row: Option<QueueRow> = sqlx::query_as(select_items!("WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db)?;
    row.map(QueueRow::into_domain)
        .ok_or_else(|| AppError::NotFound(format!("queue item {id}")))
}

/// Items shown on the board: waiting, sending, printing and failed.
/// Waiting items come in lane order; active and failed ones first per lane.
pub async fn list_board(pool: &SqlitePool) -> Result<Vec<QueueItem>, AppError> {
    let rows: Vec<QueueRow> = sqlx::query_as(select_items!(
        "\
         WHERE status IN ('waiting', 'sending', 'printing', 'failed') \
         ORDER BY CASE status WHEN 'printing' THEN 0 WHEN 'sending' THEN 1 \
                  WHEN 'failed' THEN 2 ELSE 3 END, position, id"
    ))
    .fetch_all(pool)
    .await
    .map_err(db)?;
    Ok(rows.into_iter().map(QueueRow::into_domain).collect())
}

/// Waiting items of one lane, in order.
pub async fn list_waiting(
    pool: &SqlitePool,
    printer_id: Option<i64>,
) -> Result<Vec<QueueItem>, AppError> {
    let rows: Vec<QueueRow> = sqlx::query_as(select_items!(
        "WHERE status = 'waiting' AND printer_id IS ? \
         ORDER BY position, id"
    ))
    .bind(printer_id)
    .fetch_all(pool)
    .await
    .map_err(db)?;
    Ok(rows.into_iter().map(QueueRow::into_domain).collect())
}

/// The sending/printing item on a printer, if any.
pub async fn active_for_printer(
    pool: &SqlitePool,
    printer_id: i64,
) -> Result<Option<QueueItem>, AppError> {
    let row: Option<QueueRow> = sqlx::query_as(select_items!(
        "WHERE printer_id = ? \
         AND status IN ('sending', 'printing') ORDER BY id LIMIT 1"
    ))
    .bind(printer_id)
    .fetch_optional(pool)
    .await
    .map_err(db)?;
    Ok(row.map(QueueRow::into_domain))
}

/// Move a waiting (or failed → back to waiting) item to `index` in a lane.
/// Renumbers the target lane. Returns the lane's waiting ids in order.
pub async fn move_to(
    pool: &SqlitePool,
    id: i64,
    printer_id: Option<i64>,
    index: usize,
) -> Result<Vec<i64>, AppError> {
    let item = get(pool, id).await?;
    if !matches!(item.status, QueueStatus::Waiting | QueueStatus::Failed) {
        return Err(AppError::InvalidInput(format!(
            "item is {}; only waiting or failed items can move",
            item.status.as_str()
        )));
    }
    let mut tx = pool.begin().await.map_err(db)?;
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM queue_item WHERE status = 'waiting' AND printer_id IS ? AND id != ? \
         ORDER BY position, id",
    )
    .bind(printer_id)
    .bind(id)
    .fetch_all(&mut *tx)
    .await
    .map_err(db)?;
    let mut order = ids;
    order.insert(index.min(order.len()), id);
    let ts = now();
    // Moving into the fleet lane clears `from_fleet`; it is a fleet item again.
    sqlx::query(
        "UPDATE queue_item SET printer_id = ?, status = 'waiting', error = NULL, from_fleet = 0, \
         updated_at = ? WHERE id = ?",
    )
    .bind(printer_id)
    .bind(&ts)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    for (pos, qid) in order.iter().enumerate() {
        sqlx::query("UPDATE queue_item SET position = ? WHERE id = ?")
            .bind(pos as i64)
            .bind(qid)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
    }
    tx.commit().await.map_err(db)?;
    Ok(order)
}

/// Claim a waiting item for a printer: waiting → sending. False if someone else
/// changed it first (removed, moved to another status).
pub async fn claim(pool: &SqlitePool, id: i64, printer_id: i64) -> Result<bool, AppError> {
    let r = sqlx::query(
        "UPDATE queue_item SET status = 'sending', from_fleet = (printer_id IS NULL), \
         printer_id = ?, error = NULL, updated_at = ? WHERE id = ? AND status = 'waiting'",
    )
    .bind(printer_id)
    .bind(now())
    .bind(id)
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(r.rows_affected() == 1)
}

/// Conditional status change `from` → `to`. False when the item was not in `from`.
pub async fn transition(
    pool: &SqlitePool,
    id: i64,
    from: QueueStatus,
    to: QueueStatus,
    error: Option<&str>,
) -> Result<bool, AppError> {
    let r = sqlx::query(
        "UPDATE queue_item SET status = ?, error = COALESCE(?, error), updated_at = ? \
         WHERE id = ? AND status = ?",
    )
    .bind(to.as_str())
    .bind(error)
    .bind(now())
    .bind(id)
    .bind(from.as_str())
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(r.rows_affected() == 1)
}

/// What `remove` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    /// Row deleted (waiting / failed / finished).
    Deleted,
    /// Upload in progress: marked cancelled; the dispatcher stops before starting.
    Cancelled,
}

/// Remove an item from the board. Refuses printing items (cancel on the printer).
pub async fn remove(pool: &SqlitePool, id: i64) -> Result<Removed, AppError> {
    // Sending → cancelled is the race-sensitive case; try it first.
    if transition(pool, id, QueueStatus::Sending, QueueStatus::Cancelled, None).await? {
        return Ok(Removed::Cancelled);
    }
    let r = sqlx::query("DELETE FROM queue_item WHERE id = ? AND status != 'printing'")
        .bind(id)
        .execute(pool)
        .await
        .map_err(db)?;
    if r.rows_affected() == 1 {
        return Ok(Removed::Deleted);
    }
    let item = get(pool, id).await?;
    Err(AppError::InvalidInput(format!(
        "{} is {}; cancel the print on the printer instead",
        item.file_name,
        item.status.as_str()
    )))
}

/// After a restart nothing is uploading: sending items go back to waiting.
pub async fn reset_sending(pool: &SqlitePool) -> Result<u64, AppError> {
    let r = sqlx::query(
        "UPDATE queue_item SET status = 'waiting', \
         printer_id = CASE WHEN from_fleet = 1 THEN NULL ELSE printer_id END, \
         from_fleet = 0, updated_at = ? WHERE status = 'sending'",
    )
    .bind(now())
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ForemanDb;

    async fn setup() -> (ForemanDb, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = ForemanDb::connect(&tmp.path().join("q.db")).await.unwrap();
        for id in [1, 2] {
            sqlx::query(
                "INSERT INTO printer (id, name, firmware, tags, created_at, updated_at) \
                 VALUES (?, ?, 'moonraker', '[]', 'x', 'x')",
            )
            .bind(id)
            .bind(format!("P{id}"))
            .execute(&db.pool)
            .await
            .unwrap();
        }
        (db, tmp)
    }

    async fn ids(pool: &SqlitePool, lane: Option<i64>) -> Vec<i64> {
        list_waiting(pool, lane)
            .await
            .unwrap()
            .iter()
            .map(|i| i.id)
            .collect()
    }

    #[tokio::test]
    async fn reorder_and_move_between_lanes() {
        let (db, _t) = setup().await;
        let p = &db.pool;
        let a = add(p, 10, "a", Some(1), QueueSource::Manual, "u")
            .await
            .unwrap();
        let b = add(p, 11, "b", Some(1), QueueSource::Manual, "u")
            .await
            .unwrap();
        let c = add(p, 12, "c", Some(1), QueueSource::Manual, "u")
            .await
            .unwrap();
        assert_eq!(ids(p, Some(1)).await, vec![a.id, b.id, c.id]);

        // Reorder within the lane: c to top.
        assert_eq!(
            move_to(p, c.id, Some(1), 0).await.unwrap(),
            vec![c.id, a.id, b.id]
        );
        assert_eq!(ids(p, Some(1)).await, vec![c.id, a.id, b.id]);

        // Move a to printer 2, then to the fleet, then back to printer 1 at index 1.
        move_to(p, a.id, Some(2), 0).await.unwrap();
        assert_eq!(ids(p, Some(2)).await, vec![a.id]);
        assert_eq!(ids(p, Some(1)).await, vec![c.id, b.id]);
        move_to(p, a.id, None, 5).await.unwrap();
        assert_eq!(ids(p, None).await, vec![a.id]);
        assert!(ids(p, Some(2)).await.is_empty());
        move_to(p, a.id, Some(1), 1).await.unwrap();
        assert_eq!(ids(p, Some(1)).await, vec![c.id, a.id, b.id]);
        assert!(ids(p, None).await.is_empty());

        // New items go to the end.
        let d = add(p, 13, "d", Some(1), QueueSource::Slicer, "u")
            .await
            .unwrap();
        assert_eq!(ids(p, Some(1)).await, vec![c.id, a.id, b.id, d.id]);
    }

    #[tokio::test]
    async fn claim_is_exclusive_and_remove_cancels_sending() {
        let (db, _t) = setup().await;
        let p = &db.pool;
        let a = add(p, 10, "a", None, QueueSource::Manual, "u")
            .await
            .unwrap();
        assert!(claim(p, a.id, 1).await.unwrap());
        assert!(!claim(p, a.id, 2).await.unwrap(), "second claim must fail");
        let got = get(p, a.id).await.unwrap();
        assert_eq!(got.status, QueueStatus::Sending);
        assert_eq!(got.printer_id, Some(1));
        assert!(got.from_fleet);

        // Moving a sending item is refused.
        assert!(move_to(p, a.id, None, 0).await.is_err());

        // Remove while sending → cancelled; the dispatcher's sending→printing then fails.
        assert_eq!(remove(p, a.id).await.unwrap(), Removed::Cancelled);
        assert!(
            !transition(p, a.id, QueueStatus::Sending, QueueStatus::Printing, None)
                .await
                .unwrap()
        );

        // Printing items can't be removed.
        let b = add(p, 11, "b", Some(2), QueueSource::Manual, "u")
            .await
            .unwrap();
        assert!(claim(p, b.id, 2).await.unwrap());
        assert!(
            transition(p, b.id, QueueStatus::Sending, QueueStatus::Printing, None)
                .await
                .unwrap()
        );
        assert!(remove(p, b.id).await.is_err());
        assert_eq!(
            active_for_printer(p, 2).await.unwrap().map(|i| i.id),
            Some(b.id)
        );
    }

    #[tokio::test]
    async fn reset_sending_returns_fleet_items_to_fleet() {
        let (db, _t) = setup().await;
        let p = &db.pool;
        let a = add(p, 10, "a", None, QueueSource::Manual, "u")
            .await
            .unwrap();
        let b = add(p, 11, "b", Some(2), QueueSource::Manual, "u")
            .await
            .unwrap();
        claim(p, a.id, 1).await.unwrap();
        claim(p, b.id, 2).await.unwrap();
        assert_eq!(reset_sending(p).await.unwrap(), 2);
        assert_eq!(ids(p, None).await, vec![a.id]);
        assert_eq!(ids(p, Some(2)).await, vec![b.id]);
    }
}
