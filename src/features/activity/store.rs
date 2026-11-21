use rusqlite::{params, Connection};

use crate::error::AppResult;
use crate::store::sql::{self, SqlRow};

use super::model::{Activity, ActivityQuery};

pub(crate) const ACTIVITY_COLUMNS: &str =
    "id, team_id, actor, action, object_type, object_id, object_name, result, created_at";

pub(crate) const INSERT_ACTIVITY: &str = "INSERT INTO activity (
        id, team_id, actor, action, object_type, object_id, object_name, result, created_at
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";

fn activity_filter() -> &'static str {
    "team_id = ?1
  AND (?2 = '' OR actor = ?2)
  AND (?3 = '' OR action = ?3)
  AND (?4 = '' OR object_type = ?4)
  AND (?5 = '' OR created_at >= ?5)
  AND (?6 = '' OR created_at <= ?6)"
}

pub(crate) fn list_activity_sql() -> String {
    format!(
        "SELECT {ACTIVITY_COLUMNS} FROM activity WHERE {}
         ORDER BY created_at DESC, id DESC LIMIT ?7 OFFSET ?8",
        activity_filter()
    )
}

pub(crate) fn count_activity_sql() -> String {
    format!("SELECT COUNT(*) FROM activity WHERE {}", activity_filter())
}

/// Keeps the newest `keep` rows of one team and drops the rest. The bound is a
/// row count per team; there is no time-based retention.
pub(crate) const PRUNE_ACTIVITY: &str = "DELETE FROM activity
     WHERE team_id = ?1
       AND id NOT IN (
         SELECT id FROM activity WHERE team_id = ?1
         ORDER BY created_at DESC, id DESC LIMIT ?2
       )";

pub(crate) fn insert_activity(c: &Connection, item: &Activity) -> AppResult<()> {
    c.execute(
        INSERT_ACTIVITY,
        params![
            item.id,
            item.team_id,
            item.actor,
            item.action,
            item.object_type,
            item.object_id,
            item.object_name,
            item.result,
            item.created_at
        ],
    )?;
    Ok(())
}

pub(crate) fn list_activity(
    c: &Connection,
    team_id: &str,
    query: &ActivityQuery,
) -> AppResult<(Vec<Activity>, i64)> {
    let limit = query.limit.clamp(1, 100);
    let offset = query.offset.max(0);
    let items = sql::sqlite_all(
        c,
        &list_activity_sql(),
        params![
            team_id,
            &query.actor,
            &query.action,
            &query.object_type,
            &query.since,
            &query.until,
            limit,
            offset
        ],
        sqlite_row_activity,
    )?;
    let total = c.query_row(
        &count_activity_sql(),
        params![
            team_id,
            &query.actor,
            &query.action,
            &query.object_type,
            &query.since,
            &query.until
        ],
        |row| row.get(0),
    )?;
    Ok((items, total))
}

pub(crate) fn prune_activity(c: &Connection, team_id: &str, keep: i64) -> AppResult<()> {
    c.execute(PRUNE_ACTIVITY, params![team_id, keep.max(0)])?;
    Ok(())
}

pub(crate) fn row_activity<R: SqlRow>(row: &R) -> AppResult<Activity> {
    Ok(Activity {
        id: row.get(0)?,
        team_id: row.get(1)?,
        actor: row.get(2)?,
        action: row.get(3)?,
        object_type: row.get(4)?,
        object_id: row.get(5)?,
        object_name: row.get(6)?,
        result: row.get(7)?,
        created_at: row.get(8)?,
    })
}

fn sqlite_row_activity(row: &rusqlite::Row<'_>) -> AppResult<Activity> {
    row_activity(row)
}
