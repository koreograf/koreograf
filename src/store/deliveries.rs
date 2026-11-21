use rusqlite::{params, Connection};

use crate::db::{self, Delivery, NewDelivery};
use crate::error::AppResult;
use crate::store::sql::{self, SqlRow};

pub(crate) const INSERT_DELIVERY: &str = "INSERT INTO hook_deliveries (
        id, hook_id, status, summary, payload, job_id, workflow_job_id, created_at
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)";

pub(crate) const LIST_DELIVERIES: &str =
    "SELECT d.id, d.hook_id, h.name, d.status, d.summary, d.job_id, d.workflow_job_id, d.created_at
     FROM hook_deliveries d JOIN hooks h ON h.id = d.hook_id
     WHERE h.team_id = ?1 ORDER BY d.created_at DESC LIMIT ?2";

pub(crate) const DELETE_DELIVERIES_FOR_HOOK: &str =
    "DELETE FROM hook_deliveries WHERE hook_id = ?1";

pub(crate) const PRUNE_EXPIRED_DELIVERIES: &str = "DELETE FROM hook_deliveries
     WHERE id IN (
        SELECT d.id FROM hook_deliveries d JOIN hooks h ON h.id = d.hook_id
        WHERE d.created_at < ?1
        ORDER BY d.created_at, d.id
        LIMIT ?2
     )";

/// Rows beyond the newest `keep` deliveries of each team, oldest first. The
/// ranking restarts from the rows that are still present, so a batch loop
/// converges on the bound.
pub(crate) const PRUNE_OVER_LIMIT_DELIVERIES: &str = "DELETE FROM hook_deliveries
     WHERE id IN (
        SELECT id FROM (
            SELECT d.id, ROW_NUMBER() OVER (
                PARTITION BY h.team_id ORDER BY d.created_at DESC, d.id DESC
            ) AS rn
            FROM hook_deliveries d JOIN hooks h ON h.id = d.hook_id
        ) ranked
        WHERE rn > ?1
        ORDER BY rn DESC
        LIMIT ?2
     )";

pub(crate) fn insert_delivery(c: &Connection, item: NewDelivery<'_>) -> AppResult<()> {
    c.execute(
        INSERT_DELIVERY,
        params![
            item.id,
            item.hook_id,
            item.status,
            item.summary,
            item.payload,
            item.job_id,
            item.workflow_job_id,
            db::now()
        ],
    )?;
    Ok(())
}

pub(crate) fn list_deliveries(
    c: &Connection,
    team_id: &str,
    limit: i64,
) -> AppResult<Vec<Delivery>> {
    sql::sqlite_all(
        c,
        LIST_DELIVERIES,
        params![team_id, limit],
        sqlite_row_delivery,
    )
}

/// Deletes up to `limit` deliveries that are older than `cutoff` or beyond
/// the newest `keep` rows of their team in one transaction. The caller repeats
/// the call until it returns fewer than `limit` rows, so the database lock is
/// released between batches.
pub(crate) fn prune_hook_deliveries(
    c: &mut Connection,
    cutoff: &str,
    keep: i64,
    limit: i64,
) -> AppResult<usize> {
    let tx = c.transaction()?;
    let mut removed = tx.execute(PRUNE_EXPIRED_DELIVERIES, params![cutoff, limit])?;
    let remaining = limit.saturating_sub(i64::try_from(removed).unwrap_or(limit));
    if remaining > 0 {
        removed = removed
            .saturating_add(tx.execute(PRUNE_OVER_LIMIT_DELIVERIES, params![keep, remaining])?);
    }
    tx.commit()?;
    Ok(removed)
}

pub(crate) fn row_delivery<R: SqlRow>(row: &R) -> AppResult<Delivery> {
    Ok(Delivery {
        id: row.get(0)?,
        hook_id: row.get(1)?,
        hook_name: row.get(2)?,
        status: row.get(3)?,
        summary: row.get(4)?,
        job_id: row.get(5)?,
        workflow_job_id: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn sqlite_row_delivery(row: &rusqlite::Row<'_>) -> AppResult<Delivery> {
    row_delivery(row)
}
