use rusqlite::{params, Connection};

use crate::db::{self, Job, JobActivity, JobEvent, JobQuery};
use crate::error::AppResult;
use crate::store::sql::{self, Dialect, SqlRow};

pub(crate) const JOB_COLUMNS: &str =
    "id, team_id, template_id, workflow_job_id, workflow_node_id, \
     name, tool, status, extra_vars, launched_by, trigger, started_at, finished_at, created_at, \
     parameters, failed_hosts";

pub(crate) const INSERT_JOB: &str = "INSERT INTO jobs (
        id, team_id, template_id, workflow_job_id, workflow_node_id, name, tool, status,
        extra_vars, launched_by, trigger, started_at, finished_at, created_at, parameters,
        failed_hosts
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)";

pub(crate) const MARK_RUNNING: &str =
    "UPDATE jobs SET status = 'running', started_at = COALESCE(started_at, ?2)
     WHERE id = ?1 AND status = 'pending'";

pub(crate) const SET_FAILED_HOSTS: &str = "UPDATE jobs SET failed_hosts = ?2 WHERE id = ?1";

pub(crate) const SET_JOB_PARAMETERS: &str = "UPDATE jobs SET parameters = ?2 WHERE id = ?1";

pub(crate) const FINISH_JOB: &str = "UPDATE jobs SET status = ?2, finished_at = ?3
     WHERE id = ?1 AND status IN ('pending', 'running')";

pub(crate) const INSERT_EVENT: &str =
    "INSERT INTO job_events (job_id, ts, stream, line) VALUES (?1, ?2, ?3, ?4)";

pub(crate) const EVENTS_AFTER: &str = "SELECT id, ts, stream, line FROM job_events
     WHERE job_id = ?1 AND id > ?2 ORDER BY id LIMIT ?3";

pub(crate) const EVENTS_COUNT_BETWEEN: &str =
    "SELECT COUNT(*) FROM job_events WHERE job_id = ?1 AND id > ?2 AND id < ?3";

pub(crate) const EVENTS_LAST: &str = "SELECT id, ts, stream, line FROM job_events
     WHERE job_id = ?1 ORDER BY id DESC LIMIT ?2";

pub(crate) const JOB_ACTIVITY: &str = "SELECT status, started_at, finished_at FROM jobs
     WHERE team_id = ?1
       AND ((finished_at IS NOT NULL AND finished_at >= ?2 AND finished_at <= ?3)
         OR (started_at IS NOT NULL AND started_at >= ?2 AND started_at <= ?3))";

pub(crate) const JOBS_BUSY: &str =
    "SELECT COUNT(*) FROM jobs WHERE status IN ('pending', 'running')";

fn job_filter(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Sqlite => {
            "team_id = ?1
  AND (?2 = ''
    OR instr(lower(name), lower(?2)) > 0
    OR instr(lower(tool), lower(?2)) > 0
    OR instr(lower(status), lower(?2)) > 0
    OR instr(lower(trigger), lower(?2)) > 0)
  AND (?3 = '' OR status = ?3)
  AND (?4 = '' OR tool = ?4)
  AND (?5 = '' OR trigger = ?5)
  AND (?6 = '' OR template_id = ?6)
  AND (?7 = '' OR created_at >= ?7)
  AND (?8 = '' OR created_at <= ?8)"
        }
        Dialect::Postgres => {
            "team_id = ?1
  AND (?2 = ''
    OR strpos(lower(name), lower(?2)) > 0
    OR strpos(lower(tool), lower(?2)) > 0
    OR strpos(lower(status), lower(?2)) > 0
    OR strpos(lower(trigger), lower(?2)) > 0)
  AND (?3 = '' OR status = ?3)
  AND (?4 = '' OR tool = ?4)
  AND (?5 = '' OR trigger = ?5)
  AND (?6 = '' OR template_id = ?6)
  AND (?7 = '' OR created_at >= ?7)
  AND (?8 = '' OR created_at <= ?8)"
        }
    }
}

pub(crate) fn job_by_id_sql() -> String {
    format!("SELECT {JOB_COLUMNS} FROM jobs WHERE id = ?1")
}

pub(crate) fn recent_jobs_sql() -> String {
    format!(
        "SELECT {JOB_COLUMNS} FROM jobs WHERE team_id = ?1
         ORDER BY created_at DESC, id DESC LIMIT ?2"
    )
}

pub(crate) fn list_jobs_sql(dialect: Dialect) -> String {
    format!(
        "SELECT {JOB_COLUMNS} FROM jobs WHERE {}
         ORDER BY created_at DESC, id DESC LIMIT ?9 OFFSET ?10",
        job_filter(dialect)
    )
}

pub(crate) fn count_jobs_sql(dialect: Dialect) -> String {
    format!("SELECT COUNT(*) FROM jobs WHERE {}", job_filter(dialect))
}

pub(crate) fn events_bytes_sql(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Sqlite => {
            "SELECT COALESCE(SUM(LENGTH(CAST(line AS BLOB)) + 1), 0) FROM job_events WHERE job_id = ?1"
        }
        Dialect::Postgres => {
            "SELECT COALESCE(SUM(octet_length(line) + 1), 0) FROM job_events WHERE job_id = ?1"
        }
    }
}

pub(crate) fn insert_job(c: &Connection, job: &Job) -> AppResult<()> {
    c.execute(
        INSERT_JOB,
        params![
            job.id,
            job.team_id,
            job.template_id,
            job.workflow_job_id,
            job.workflow_node_id,
            job.name,
            job.tool,
            job.status,
            job.extra_vars,
            job.launched_by,
            job.trigger,
            job.started_at,
            job.finished_at,
            job.created_at,
            job.parameters,
            job.failed_hosts
        ],
    )?;
    Ok(())
}

pub(crate) fn job(c: &Connection, id: &str) -> AppResult<Option<Job>> {
    sql::sqlite_opt(c, &job_by_id_sql(), [id], sqlite_row_job)
}

pub(crate) fn list_jobs(
    c: &Connection,
    team_id: &str,
    query: &JobQuery,
) -> AppResult<(Vec<Job>, i64)> {
    let limit = query.limit.clamp(1, 100);
    let offset = query.offset.max(0);
    let items = sql::sqlite_all(
        c,
        &list_jobs_sql(Dialect::Sqlite),
        params![
            team_id,
            &query.q,
            &query.status,
            &query.tool,
            &query.trigger,
            &query.template_id,
            &query.since,
            &query.until,
            limit,
            offset
        ],
        sqlite_row_job,
    )?;
    let total = c.query_row(
        &count_jobs_sql(Dialect::Sqlite),
        params![
            team_id,
            &query.q,
            &query.status,
            &query.tool,
            &query.trigger,
            &query.template_id,
            &query.since,
            &query.until
        ],
        |row| row.get(0),
    )?;
    Ok((items, total))
}

pub(crate) fn recent_jobs(c: &Connection, team_id: &str, limit: i64) -> AppResult<Vec<Job>> {
    sql::sqlite_all(
        c,
        &recent_jobs_sql(),
        params![team_id, limit.clamp(1, 100)],
        sqlite_row_job,
    )
}

pub(crate) fn job_activity(
    c: &Connection,
    team_id: &str,
    from: &str,
    to: &str,
) -> AppResult<Vec<JobActivity>> {
    sql::sqlite_all(
        c,
        JOB_ACTIVITY,
        params![team_id, from, to],
        sqlite_row_job_activity,
    )
}

pub(crate) fn mark_running(c: &Connection, id: &str) -> AppResult<bool> {
    let changed = c.execute(MARK_RUNNING, params![id, db::now()])?;
    Ok(changed == 1)
}

pub(crate) fn set_failed_hosts(c: &Connection, id: &str, failed_hosts: &str) -> AppResult<()> {
    c.execute(SET_FAILED_HOSTS, params![id, failed_hosts])?;
    Ok(())
}

pub(crate) fn set_job_parameters(c: &Connection, id: &str, parameters: &str) -> AppResult<()> {
    c.execute(SET_JOB_PARAMETERS, params![id, parameters])?;
    Ok(())
}

pub(crate) fn finish_job(c: &Connection, id: &str, status: &str) -> AppResult<bool> {
    let changed = c.execute(FINISH_JOB, params![id, status, db::now()])?;
    Ok(changed == 1)
}

pub(crate) fn insert_event(
    c: &Connection,
    job_id: &str,
    stream: &str,
    line: &str,
) -> AppResult<()> {
    c.execute(INSERT_EVENT, params![job_id, db::now(), stream, line])?;
    Ok(())
}

pub(crate) fn events_after(
    c: &Connection,
    job_id: &str,
    after: i64,
    limit: i64,
) -> AppResult<Vec<JobEvent>> {
    sql::sqlite_all(
        c,
        EVENTS_AFTER,
        params![job_id, after, limit],
        sqlite_row_job_event,
    )
}

pub(crate) fn events_count_between(
    c: &Connection,
    job_id: &str,
    after: i64,
    before: i64,
) -> AppResult<i64> {
    sql::sqlite_opt(
        c,
        EVENTS_COUNT_BETWEEN,
        params![job_id, after, before],
        |row| SqlRow::get(row, 0),
    )?
    .ok_or_else(|| crate::error::ApiError::Internal("The event count is missing.".into()))
}

pub(crate) fn events_bytes(c: &Connection, job_id: &str) -> AppResult<i64> {
    sql::sqlite_opt(c, events_bytes_sql(Dialect::Sqlite), [job_id], |row| {
        SqlRow::get(row, 0)
    })?
    .ok_or_else(|| crate::error::ApiError::Internal("The event size is missing.".into()))
}

pub(crate) fn events_last(c: &Connection, job_id: &str, limit: i64) -> AppResult<Vec<JobEvent>> {
    let mut events = sql::sqlite_all(c, EVENTS_LAST, params![job_id, limit], sqlite_row_job_event)?;
    events.reverse();
    Ok(events)
}

pub(crate) fn row_job<R: SqlRow>(row: &R) -> AppResult<Job> {
    Ok(Job {
        id: row.get(0)?,
        team_id: row.get(1)?,
        template_id: row.get(2)?,
        workflow_job_id: row.get(3)?,
        workflow_node_id: row.get(4)?,
        name: row.get(5)?,
        tool: row.get(6)?,
        status: row.get(7)?,
        extra_vars: row.get(8)?,
        launched_by: row.get(9)?,
        trigger: row.get(10)?,
        started_at: row.get(11)?,
        finished_at: row.get(12)?,
        created_at: row.get(13)?,
        parameters: row.get(14)?,
        failed_hosts: row.get(15)?,
    })
}

pub(crate) fn row_job_activity<R: SqlRow>(row: &R) -> AppResult<JobActivity> {
    Ok(JobActivity {
        status: row.get(0)?,
        started_at: row.get(1)?,
        finished_at: row.get(2)?,
    })
}

pub(crate) fn row_job_event<R: SqlRow>(row: &R) -> AppResult<JobEvent> {
    Ok(JobEvent {
        id: row.get(0)?,
        ts: row.get(1)?,
        stream: row.get(2)?,
        line: row.get(3)?,
    })
}

fn sqlite_row_job(row: &rusqlite::Row<'_>) -> AppResult<Job> {
    row_job(row)
}

fn sqlite_row_job_activity(row: &rusqlite::Row<'_>) -> AppResult<JobActivity> {
    row_job_activity(row)
}

fn sqlite_row_job_event(row: &rusqlite::Row<'_>) -> AppResult<JobEvent> {
    row_job_event(row)
}
