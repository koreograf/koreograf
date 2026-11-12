use rusqlite::{params, Connection};

use crate::db::{self, Schedule};
use crate::error::{ApiError, AppResult};
use crate::store::sql::{self, Dialect, SqlRow};

pub(crate) const SCHEDULE_COLUMNS: &str =
    "id, team_id, name, target_kind, target_id, timezone, start_date, start_time, frequency, \
     weekdays, end_kind, end_count, end_date, enabled, next_run_at, missed_at, occurrence_count, \
     created_by, created_at, updated_at, updated_by, launch_block";

pub(crate) const INSERT_SCHEDULE: &str = "INSERT INTO schedules (
        id, team_id, name, target_kind, target_id, timezone, start_date, start_time, frequency,
        weekdays, end_kind, end_count, end_date, enabled, next_run_at, missed_at, occurrence_count,
        created_by, created_at, updated_at, updated_by, launch_block
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
        ?19, ?20, ?21, ?22)";

pub(crate) const UPDATE_SCHEDULE: &str = "UPDATE schedules SET
        name = ?2, timezone = ?3, start_date = ?4, start_time = ?5, frequency = ?6, weekdays = ?7,
        end_kind = ?8, end_count = ?9, end_date = ?10, enabled = ?11, next_run_at = ?12,
        missed_at = ?13, occurrence_count = ?14, updated_at = ?15, updated_by = ?16,
        launch_block = ?17
     WHERE id = ?1 AND team_id = ?18";

pub(crate) const DELETE_SCHEDULE: &str = "DELETE FROM schedules WHERE id = ?1 AND team_id = ?2";

pub(crate) const DELETE_SCHEDULES_FOR: &str =
    "DELETE FROM schedules WHERE target_kind = ?1 AND target_id = ?2";

pub(crate) fn schedule_by_id_sql() -> String {
    format!("SELECT {SCHEDULE_COLUMNS} FROM schedules WHERE id = ?1")
}

pub(crate) fn list_schedules_sql() -> String {
    format!(
        "SELECT {SCHEDULE_COLUMNS} FROM schedules
         WHERE target_kind = ?1 AND target_id = ?2 ORDER BY name"
    )
}

pub(crate) fn list_team_schedules_sql() -> String {
    format!(
        "SELECT {SCHEDULE_COLUMNS},
            COALESCE(
                CASE target_kind
                    WHEN 'template' THEN (
                        SELECT name FROM templates WHERE id = schedules.target_id
                    )
                    WHEN 'workflow' THEN (
                        SELECT name FROM workflows WHERE id = schedules.target_id
                    )
                    ELSE ''
                END,
                ''
            )
         FROM schedules WHERE team_id = ?1 ORDER BY name"
    )
}

pub(crate) fn due_schedules_sql() -> String {
    format!(
        "SELECT {SCHEDULE_COLUMNS} FROM schedules
         WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1
         ORDER BY next_run_at"
    )
}

pub(crate) const SET_SCHEDULE_ENABLED: &str = "UPDATE schedules SET enabled = ?3,
        next_run_at = ?4, occurrence_count = ?5, updated_at = ?6
     WHERE id = ?1 AND team_id = ?2";

pub(crate) const CLAIM_MISSED: &str = "SELECT missed_at FROM schedules
     WHERE id = ?1 AND team_id = ?2 AND missed_at IS NOT NULL";

pub(crate) const CLAIM_MISSED_LOCKED: &str = "SELECT missed_at FROM schedules
     WHERE id = ?1 AND team_id = ?2 AND missed_at IS NOT NULL FOR UPDATE";

pub(crate) const CLEAR_MISSED: &str = "UPDATE schedules SET missed_at = NULL, updated_at = ?3
     WHERE id = ?1 AND team_id = ?2 AND missed_at IS NOT NULL";

pub(crate) const RESTORE_MISSED: &str = "UPDATE schedules SET missed_at = ?3, updated_at = ?4
     WHERE id = ?1 AND team_id = ?2 AND missed_at IS NULL";

pub(crate) const ADVANCE_SCHEDULE: &str =
    "UPDATE schedules SET next_run_at = ?2, occurrence_count = ?3, updated_at = ?4 WHERE id = ?1";

pub(crate) const NOTE_MISS: &str =
    "UPDATE schedules SET missed_at = ?2, updated_at = ?3 WHERE id = ?1";

pub(crate) const SET_LAUNCH_BLOCK: &str =
    "UPDATE schedules SET launch_block = ?2, updated_at = ?3 WHERE id = ?1";

pub(crate) fn claim_missed_sql(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Sqlite => CLAIM_MISSED,
        Dialect::Postgres => CLAIM_MISSED_LOCKED,
    }
}

pub(crate) fn insert_schedule(c: &Connection, item: &Schedule) -> AppResult<()> {
    c.execute(
        INSERT_SCHEDULE,
        params![
            item.id,
            item.team_id,
            item.name,
            item.target_kind,
            item.target_id,
            item.timezone,
            item.start_date,
            item.start_time,
            item.frequency,
            item.weekdays,
            item.end_kind,
            item.end_count,
            item.end_date,
            item.enabled as i64,
            item.next_run_at,
            item.missed_at,
            item.occurrence_count,
            item.created_by,
            item.created_at,
            item.updated_at,
            item.updated_by,
            item.launch_block,
        ],
    )?;
    Ok(())
}

pub(crate) fn update_schedule(c: &Connection, item: &Schedule) -> AppResult<()> {
    let changed = c.execute(
        UPDATE_SCHEDULE,
        params![
            item.id,
            item.name,
            item.timezone,
            item.start_date,
            item.start_time,
            item.frequency,
            item.weekdays,
            item.end_kind,
            item.end_count,
            item.end_date,
            item.enabled as i64,
            item.next_run_at,
            item.missed_at,
            item.occurrence_count,
            item.updated_at,
            item.updated_by,
            item.launch_block,
            item.team_id,
        ],
    )?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn schedule(c: &Connection, id: &str) -> AppResult<Option<Schedule>> {
    sql::sqlite_opt(c, &schedule_by_id_sql(), [id], sqlite_row_schedule)
}

pub(crate) fn list_schedules(
    c: &Connection,
    kind: &str,
    target_id: &str,
) -> AppResult<Vec<Schedule>> {
    sql::sqlite_all(
        c,
        &list_schedules_sql(),
        params![kind, target_id],
        sqlite_row_schedule,
    )
}

pub(crate) fn list_team_schedules(
    c: &Connection,
    team_id: &str,
) -> AppResult<Vec<(Schedule, String)>> {
    sql::sqlite_all(c, &list_team_schedules_sql(), [team_id], |row| {
        Ok((row_schedule(row)?, row.get(22)?))
    })
}

pub(crate) fn due_schedules(c: &Connection, now: &str) -> AppResult<Vec<Schedule>> {
    sql::sqlite_all(c, &due_schedules_sql(), [now], sqlite_row_schedule)
}

pub(crate) fn delete_schedule(c: &Connection, id: &str, team_id: &str) -> AppResult<()> {
    let changed = c.execute(DELETE_SCHEDULE, params![id, team_id])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn claim_missed(
    c: &mut Connection,
    id: &str,
    team_id: &str,
) -> AppResult<Option<String>> {
    let tx = c.transaction()?;
    let missed_at = match tx.query_row(CLAIM_MISSED, params![id, team_id], |row| row.get(0)) {
        Ok(value) => value,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let changed = tx.execute(CLEAR_MISSED, params![id, team_id, db::now()])?;
    tx.commit()?;
    if changed == 1 {
        Ok(Some(missed_at))
    } else {
        Ok(None)
    }
}

pub(crate) fn restore_missed(
    c: &Connection,
    id: &str,
    team_id: &str,
    missed_at: &str,
) -> AppResult<()> {
    c.execute(RESTORE_MISSED, params![id, team_id, missed_at, db::now()])?;
    Ok(())
}

pub(crate) fn set_schedule_enabled(
    c: &Connection,
    id: &str,
    team_id: &str,
    enabled: bool,
    next_run_at: Option<&str>,
    occurrence_count: i64,
) -> AppResult<()> {
    let changed = c.execute(
        SET_SCHEDULE_ENABLED,
        params![
            id,
            team_id,
            enabled as i64,
            next_run_at,
            occurrence_count,
            db::now()
        ],
    )?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn advance_schedule(
    c: &Connection,
    id: &str,
    next_run_at: Option<&str>,
    occurrence_count: i64,
) -> AppResult<()> {
    c.execute(
        ADVANCE_SCHEDULE,
        params![id, next_run_at, occurrence_count, db::now()],
    )?;
    Ok(())
}

pub(crate) fn note_miss(c: &Connection, id: &str, missed_at: Option<&str>) -> AppResult<()> {
    c.execute(NOTE_MISS, params![id, missed_at, db::now()])?;
    Ok(())
}

pub(crate) fn set_launch_block(c: &Connection, id: &str, reason: Option<&str>) -> AppResult<()> {
    c.execute(SET_LAUNCH_BLOCK, params![id, reason, db::now()])?;
    Ok(())
}

pub(crate) fn row_schedule<R: SqlRow>(row: &R) -> AppResult<Schedule> {
    let end_count: Option<i32> = row.get(11)?;
    let occurrence_count: i32 = row.get(16)?;
    Ok(Schedule {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        target_kind: row.get(3)?,
        target_id: row.get(4)?,
        timezone: row.get(5)?,
        start_date: row.get(6)?,
        start_time: row.get(7)?,
        frequency: row.get(8)?,
        weekdays: row.get(9)?,
        end_kind: row.get(10)?,
        end_count: end_count.map(i64::from),
        end_date: row.get(12)?,
        enabled: row.get(13)?,
        next_run_at: row.get(14)?,
        missed_at: row.get(15)?,
        occurrence_count: i64::from(occurrence_count),
        created_by: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
        updated_by: row.get(20)?,
        launch_block: row.get(21)?,
    })
}

fn sqlite_row_schedule(row: &rusqlite::Row<'_>) -> AppResult<Schedule> {
    row_schedule(row)
}
