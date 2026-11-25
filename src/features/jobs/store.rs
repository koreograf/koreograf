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

/// Finished jobs past the retention cutoff that still have events to remove.
/// The `EXISTS` keeps a later run from visiting jobs whose events are already
/// gone, so its cost stays with the output that remains rather than the whole
/// history.
pub(crate) const FINISHED_JOBS_WITH_EVENTS_BEFORE: &str = "SELECT id FROM jobs
     WHERE status IN ('successful', 'failed', 'canceled')
       AND finished_at IS NOT NULL
       AND finished_at < ?1
       AND EXISTS (SELECT 1 FROM job_events WHERE job_events.job_id = jobs.id)
     ORDER BY id
     LIMIT ?2";

pub(crate) const DELETE_JOB_EVENTS: &str = "DELETE FROM job_events WHERE job_id = ?1";

/// The line a startup reconciliation leaves on every job it marks failed.
pub(crate) const RESTARTED_BEFORE_FINISH: &str = "The server restarted before this job finished.";

pub(crate) const INTERRUPTED_JOBS: &str =
    "SELECT id FROM jobs WHERE status IN ('pending', 'running') ORDER BY id";

pub(crate) const RECONCILE_JOB: &str = "UPDATE jobs SET status = 'failed', finished_at = ?2
     WHERE id = ?1 AND status IN ('pending', 'running')";

pub(crate) const INTERRUPTED_WORKFLOW_JOBS: &str =
    "SELECT id FROM workflow_jobs WHERE status IN ('pending', 'running') ORDER BY id";

pub(crate) const RECONCILE_WORKFLOW_JOB: &str =
    "UPDATE workflow_jobs SET status = 'failed', finished_at = ?2
     WHERE id = ?1 AND status IN ('pending', 'running')";

pub(crate) const RECONCILE_WORKFLOW_NODES: &str = "UPDATE workflow_nodes
     SET phase = CASE WHEN phase = 'running' THEN 'failed' ELSE 'skipped' END
     WHERE workflow_job_id = ?1 AND phase IN ('pending', 'running')";

/// Jobs and workflow runs that a restart left without a process.
pub(crate) struct Interrupted {
    pub(crate) job_ids: Vec<String>,
    pub(crate) workflow_jobs: i64,
}

/// Finished jobs whose event rows were removed, and how many rows went.
pub(crate) struct PrunedJobLogs {
    pub(crate) job_ids: Vec<String>,
    pub(crate) events: usize,
}

pub(crate) const INSERT_EVENT: &str =
    "INSERT INTO job_events (job_id, ts, stream, line) VALUES (?1, ?2, ?3, ?4)";

pub(crate) const EVENTS_AFTER: &str = "SELECT id, ts, stream, line FROM job_events
     WHERE job_id = ?1 AND id > ?2 AND stream NOT IN ('ansible', 'tofu')
     ORDER BY id LIMIT ?3";

/// Raw log events plus structured events. The event stream uses this so a
/// structured event cannot be skipped when a raw event advances the cursor.
pub(crate) const EVENTS_ALL_AFTER: &str = "SELECT id, ts, stream, line FROM job_events
     WHERE job_id = ?1 AND id > ?2 ORDER BY id LIMIT ?3";

pub(crate) const ANSIBLE_EVENTS_AFTER: &str = "SELECT id, ts, stream, line FROM job_events
     WHERE job_id = ?1 AND id > ?2 AND stream = 'ansible' ORDER BY id LIMIT ?3";

pub(crate) const TOFU_EVENTS_AFTER: &str = "SELECT id, ts, stream, line FROM job_events
     WHERE job_id = ?1 AND id > ?2 AND stream = 'tofu' ORDER BY id LIMIT ?3";

pub(crate) const EVENTS_COUNT_BETWEEN: &str = "SELECT COUNT(*) FROM job_events
     WHERE job_id = ?1 AND id > ?2 AND id < ?3 AND stream NOT IN ('ansible', 'tofu')";

pub(crate) const EVENTS_LAST: &str = "SELECT id, ts, stream, line FROM job_events
     WHERE job_id = ?1 AND stream NOT IN ('ansible', 'tofu') ORDER BY id DESC LIMIT ?2";

pub(crate) const INSERT_HOST_RESULT: &str = "INSERT INTO host_results (
        job_id, team_id, inventory_id, host, playbook, role, result,
        ok, changed, failed, unreachable, skipped, ts
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)";

pub(crate) const HOST_HISTORY: &str = "SELECT h.id, h.job_id, j.name, j.status,
        h.host, h.playbook, h.role, h.result,
        h.ok, h.changed, h.failed, h.unreachable, h.skipped, h.ts
     FROM host_results h JOIN jobs j ON j.id = h.job_id
     WHERE h.team_id = ?1 AND h.inventory_id = ?2 AND h.host = ?3
     ORDER BY h.ts DESC, h.id DESC LIMIT ?4 OFFSET ?5";

pub(crate) const HOST_HISTORY_COUNT: &str = "SELECT COUNT(*) FROM host_results h
     JOIN jobs j ON j.id = h.job_id
     WHERE h.team_id = ?1 AND h.inventory_id = ?2 AND h.host = ?3";

pub(crate) const JOB_ACTIVITY: &str = "SELECT j.status, t.name, j.started_at, j.finished_at
     FROM jobs j
     LEFT JOIN templates t ON t.id = j.template_id AND t.team_id = j.team_id
     WHERE j.team_id = ?1
       AND ((j.finished_at IS NOT NULL AND j.finished_at >= ?2 AND j.finished_at <= ?3)
         OR (j.started_at IS NOT NULL AND j.started_at >= ?2 AND j.started_at <= ?3))";

pub(crate) const JOBS_BUSY: &str =
    "SELECT COUNT(*) FROM jobs WHERE status IN ('pending', 'running')";

pub(crate) const TEAM_JOBS_BUSY: &str =
    "SELECT COUNT(*) FROM jobs WHERE team_id = ?1 AND status IN ('pending', 'running')";

pub(crate) const JOBS_BY_STATUS: &str = "SELECT COUNT(*) FROM jobs WHERE status = ?1";

pub(crate) const BUSY_JOB_COUNTS: &str = "SELECT status, COUNT(*) FROM jobs
     WHERE team_id = ?1 AND status IN ('pending', 'running') GROUP BY status";

pub(crate) const OLDEST_PENDING_JOB: &str = "SELECT created_at FROM jobs
     WHERE team_id = ?1 AND status = 'pending'
     ORDER BY created_at ASC, id ASC LIMIT 1";

pub(crate) const GLOBAL_RUNNING_JOBS: &str = "SELECT COUNT(*) FROM jobs WHERE status = 'running'";

/// Finished jobs in the attention window, grouped by status. The overview
/// turns these rows into the failed and success-rate tiles.
pub(crate) const FINISHED_JOB_COUNTS: &str = "SELECT status, COUNT(*) FROM jobs
     WHERE team_id = ?1 AND status IN ('successful', 'failed', 'canceled')
       AND finished_at IS NOT NULL AND finished_at >= ?2
     GROUP BY status";

/// The newest failed and canceled jobs in the attention window. The overview
/// lists them even when newer successful jobs push them out of the recents.
pub(crate) const FAILED_JOB_ITEMS_LIMIT: i64 = 5;

pub(crate) fn failed_jobs_sql() -> String {
    format!(
        "SELECT {JOB_COLUMNS} FROM jobs
         WHERE team_id = ?1 AND status IN ('failed', 'canceled')
           AND finished_at IS NOT NULL AND finished_at >= ?2
         ORDER BY finished_at DESC, id DESC LIMIT ?3"
    )
}

/// The hosts whose latest result failed, was unreachable, or is older than the
/// stale cutoff. The window keeps one row per inventory and host; the count
/// and the limited list share the same latest-row definition.
const LATEST_TEAM_HOSTS: &str = "SELECT h.inventory_id, i.name AS inventory_name, h.host,
            h.result, h.ts
     FROM (
        SELECT inventory_id, host, result, ts,
               ROW_NUMBER() OVER (
                   PARTITION BY inventory_id, host ORDER BY ts DESC, id DESC
               ) AS latest
        FROM host_results
        WHERE team_id = ?1 AND inventory_id IS NOT NULL AND inventory_id <> ''
     ) h
     JOIN inventories i ON i.id = h.inventory_id AND i.team_id = ?1
     WHERE h.latest = 1";

/// A host whose inventory was deleted has no row here, so the glance cannot
/// point at a missing inventory. The window still keeps one row per inventory
/// and host, and the count uses the same inventory check.
pub(crate) const HOST_GLANCE_COUNTS: &str = "SELECT
        COALESCE(SUM(CASE WHEN latest.result IN ('failed', 'unreachable') THEN 1 ELSE 0 END), 0),
        COALESCE(SUM(CASE WHEN latest.ts < ?2 THEN 1 ELSE 0 END), 0)
     FROM (
        SELECT h.result, h.ts,
               ROW_NUMBER() OVER (
                   PARTITION BY h.inventory_id, h.host ORDER BY h.ts DESC, h.id DESC
               ) AS latest
        FROM host_results h
        JOIN inventories i ON i.id = h.inventory_id AND i.team_id = h.team_id
        WHERE h.team_id = ?1 AND h.inventory_id IS NOT NULL AND h.inventory_id <> ''
     ) latest WHERE latest.latest = 1";

/// The live inventory join decides which hosts the dashboard shows. Per-host
/// history keeps its rows when an inventory is deleted; the glance stays quiet
/// about that stale history.
pub(crate) fn host_glance_sql() -> String {
    format!(
        "SELECT hosts.inventory_id, hosts.inventory_name, hosts.host, hosts.result, hosts.ts
         FROM ({LATEST_TEAM_HOSTS}) hosts
         WHERE hosts.result IN ('failed', 'unreachable') OR hosts.ts < ?2
         ORDER BY CASE WHEN hosts.result IN ('failed', 'unreachable') THEN 0 ELSE 1 END,
                  hosts.ts DESC, hosts.host
         LIMIT ?3"
    )
}

/// A running job must stay visible when the queue is longer than the limit.
/// Running jobs come first, newest first; pending jobs follow, oldest first.
pub(crate) fn busy_jobs_sql() -> String {
    format!(
        "SELECT {JOB_COLUMNS} FROM jobs
         WHERE team_id = ?1 AND status IN ('pending', 'running')
         ORDER BY CASE WHEN status = 'running' THEN 0 ELSE 1 END,
                  CASE WHEN status = 'running' THEN created_at END DESC,
                  CASE WHEN status = 'pending' THEN created_at END ASC,
                  id ASC LIMIT ?2"
    )
}

pub(crate) struct GlanceJobs {
    pub(crate) running: i64,
    pub(crate) pending: i64,
    pub(crate) global_running: i64,
    pub(crate) oldest_pending_at: Option<String>,
    pub(crate) failed_24h: i64,
    pub(crate) successful_24h: i64,
    pub(crate) finished_24h: i64,
    pub(crate) items: Vec<Job>,
    pub(crate) failed_items: Vec<Job>,
}

/// A template that finished jobs used recently, newest finish first.
pub(crate) struct LaunchedTemplate {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) tool: String,
    pub(crate) last_launched_at: String,
}

/// Distinct templates behind the team's recent finished jobs. A job may run a
/// template many times, so `GROUP BY` keeps one row per template and
/// `MAX(finished_at)` orders them by their latest launch. The `finished_at`
/// cutoff keeps the aggregate off the whole job history, and the template join
/// keeps the answer team-scoped even when a job row names another team's
/// template.
pub(crate) const RECENT_LAUNCHED_TEMPLATES: &str = "SELECT t.id, t.name, t.tool,
        MAX(j.finished_at) AS last_launched_at
     FROM jobs j
     JOIN templates t ON t.id = j.template_id
     WHERE j.team_id = ?1 AND t.team_id = ?1
       AND j.template_id IS NOT NULL
       AND j.finished_at IS NOT NULL
       AND j.finished_at >= ?2
       AND j.status IN ('successful', 'failed', 'canceled')
     GROUP BY t.id, t.name, t.tool
     ORDER BY last_launched_at DESC, t.name
     LIMIT ?3";

pub(crate) fn row_launched_template<R: SqlRow>(row: &R) -> AppResult<LaunchedTemplate> {
    Ok(LaunchedTemplate {
        id: row.get(0)?,
        name: row.get(1)?,
        tool: row.get(2)?,
        last_launched_at: row.get(3)?,
    })
}

fn sqlite_row_launched_template(row: &rusqlite::Row<'_>) -> AppResult<LaunchedTemplate> {
    row_launched_template(row)
}

pub(crate) fn recent_launched_templates(
    c: &Connection,
    team_id: &str,
    cutoff: &str,
    limit: i64,
) -> AppResult<Vec<LaunchedTemplate>> {
    sql::sqlite_all(
        c,
        RECENT_LAUNCHED_TEMPLATES,
        params![team_id, cutoff, limit.clamp(1, 20)],
        sqlite_row_launched_template,
    )
}

pub(crate) struct GlanceHost {
    pub(crate) inventory_id: String,
    pub(crate) inventory_name: String,
    pub(crate) host: String,
    pub(crate) result: String,
    pub(crate) ts: String,
}

pub(crate) struct GlanceHosts {
    pub(crate) failed: i64,
    pub(crate) stale: i64,
    pub(crate) items: Vec<GlanceHost>,
}

pub(crate) const PENDING: &str = "pending";

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
  AND (?7 = '' OR created_at >= ?7 OR started_at >= ?7 OR finished_at >= ?7)
  AND (?8 = '' OR created_at <= ?8 OR started_at <= ?8 OR finished_at <= ?8)"
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
  AND (?7 = '' OR created_at >= ?7 OR started_at >= ?7 OR finished_at >= ?7)
  AND (?8 = '' OR created_at <= ?8 OR started_at <= ?8 OR finished_at <= ?8)"
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
            "SELECT COALESCE(SUM(LENGTH(CAST(line AS BLOB)) + 1), 0) FROM job_events
             WHERE job_id = ?1 AND stream NOT IN ('ansible', 'tofu')"
        }
        Dialect::Postgres => {
            "SELECT COALESCE(SUM(octet_length(line) + 1), 0) FROM job_events
             WHERE job_id = ?1 AND stream NOT IN ('ansible', 'tofu')"
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

pub(crate) fn glance_jobs(
    c: &Connection,
    team_id: &str,
    cutoff: &str,
    limit: i64,
) -> AppResult<GlanceJobs> {
    let counts: Vec<(String, i64)> = sql::sqlite_all(c, BUSY_JOB_COUNTS, [team_id], |row| {
        Ok((SqlRow::get(row, 0)?, SqlRow::get(row, 1)?))
    })?;
    let mut running = 0;
    let mut pending = 0;
    for (status, count) in counts {
        match status.as_str() {
            "running" => running = count,
            "pending" => pending = count,
            _ => {}
        }
    }
    let finished: Vec<(String, i64)> =
        sql::sqlite_all(c, FINISHED_JOB_COUNTS, params![team_id, cutoff], |row| {
            Ok((SqlRow::get(row, 0)?, SqlRow::get(row, 1)?))
        })?;
    let mut failed_24h = 0;
    let mut successful_24h = 0;
    let mut finished_24h = 0;
    for (status, count) in finished {
        finished_24h += count;
        match status.as_str() {
            "failed" => failed_24h = count,
            "successful" => successful_24h = count,
            _ => {}
        }
    }
    let oldest_pending_at =
        sql::sqlite_opt(c, OLDEST_PENDING_JOB, [team_id], |row| SqlRow::get(row, 0))?;
    let global_running =
        sql::sqlite_opt(c, GLOBAL_RUNNING_JOBS, [], |row| SqlRow::get(row, 0))?.unwrap_or(0);
    let items = sql::sqlite_all(
        c,
        &busy_jobs_sql(),
        params![team_id, limit.clamp(1, 100)],
        sqlite_row_job,
    )?;
    let failed_items = sql::sqlite_all(
        c,
        &failed_jobs_sql(),
        params![team_id, cutoff, FAILED_JOB_ITEMS_LIMIT],
        sqlite_row_job,
    )?;
    Ok(GlanceJobs {
        running,
        pending,
        global_running,
        oldest_pending_at,
        failed_24h,
        successful_24h,
        finished_24h,
        items,
        failed_items,
    })
}

pub(crate) fn glance_hosts(
    c: &Connection,
    team_id: &str,
    cutoff: &str,
    limit: i64,
) -> AppResult<GlanceHosts> {
    let counts = sql::sqlite_opt(c, HOST_GLANCE_COUNTS, params![team_id, cutoff], |row| {
        Ok((SqlRow::get::<i64>(row, 0)?, SqlRow::get::<i64>(row, 1)?))
    })?;
    let mut failed = 0;
    let mut stale = 0;
    if let Some((bad, old)) = counts {
        failed = bad;
        stale = old;
    }
    let items = sql::sqlite_all(
        c,
        &host_glance_sql(),
        params![team_id, cutoff, limit.clamp(1, 100)],
        sqlite_row_glance_host,
    )?;
    Ok(GlanceHosts {
        failed,
        stale,
        items,
    })
}

pub(crate) fn row_glance_host<R: SqlRow>(row: &R) -> AppResult<GlanceHost> {
    Ok(GlanceHost {
        inventory_id: row.get(0)?,
        inventory_name: row.get(1)?,
        host: row.get(2)?,
        result: row.get(3)?,
        ts: row.get(4)?,
    })
}

fn sqlite_row_glance_host(row: &rusqlite::Row<'_>) -> AppResult<GlanceHost> {
    row_glance_host(row)
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

/// Deletes the events of up to `limit` finished jobs older than `cutoff`.
///
/// One transaction covers the batch and each job's events go in a single
/// statement. A caller repeats the call until it returns no jobs, so the
/// database lock is released between batches. The jobs row stays, so history,
/// metrics and host results keep working. The work directories are removed by
/// the caller.
pub(crate) fn prune_job_logs(
    c: &mut Connection,
    cutoff: &str,
    limit: i64,
) -> AppResult<PrunedJobLogs> {
    let tx = c.transaction()?;
    let job_ids: Vec<String> = sql::sqlite_all(
        &tx,
        FINISHED_JOBS_WITH_EVENTS_BEFORE,
        params![cutoff, limit],
        |row| SqlRow::get(row, 0),
    )?;
    let mut events = 0usize;
    for id in &job_ids {
        events = events.saturating_add(tx.execute(DELETE_JOB_EVENTS, [id])?);
    }
    tx.commit()?;
    Ok(PrunedJobLogs { job_ids, events })
}

/// Fails every job and workflow run that a stopped process left unfinished.
///
/// One transaction covers the status updates and the system event lines, so a
/// failure leaves the whole set as it was.
pub(crate) fn reconcile_interrupted(c: &mut Connection, line: &str) -> AppResult<Interrupted> {
    let tx = c.transaction()?;
    let now = db::now();
    let job_ids: Vec<String> =
        sql::sqlite_all(&tx, INTERRUPTED_JOBS, [], |row| SqlRow::get(row, 0))?;
    for id in &job_ids {
        tx.execute(RECONCILE_JOB, params![id, &now])?;
        insert_event(&tx, id, "system", line)?;
    }
    let workflow_ids: Vec<String> = sql::sqlite_all(&tx, INTERRUPTED_WORKFLOW_JOBS, [], |row| {
        SqlRow::get(row, 0)
    })?;
    for id in &workflow_ids {
        tx.execute(RECONCILE_WORKFLOW_JOB, params![id, &now])?;
        tx.execute(RECONCILE_WORKFLOW_NODES, params![id])?;
    }
    tx.commit()?;
    Ok(Interrupted {
        job_ids,
        workflow_jobs: i64::try_from(workflow_ids.len()).unwrap_or(i64::MAX),
    })
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

/// Every event, including the structured `ansible` stream. The SSE handler
/// routes each row by its stream.
pub(crate) fn events_all_after(
    c: &Connection,
    job_id: &str,
    after: i64,
    limit: i64,
) -> AppResult<Vec<JobEvent>> {
    sql::sqlite_all(
        c,
        EVENTS_ALL_AFTER,
        params![job_id, after, limit],
        sqlite_row_job_event,
    )
}

/// Structured Ansible events only. The raw log queries leave this stream out.
pub(crate) fn ansible_events_after(
    c: &Connection,
    job_id: &str,
    after: i64,
    limit: i64,
) -> AppResult<Vec<JobEvent>> {
    sql::sqlite_all(
        c,
        ANSIBLE_EVENTS_AFTER,
        params![job_id, after, limit],
        sqlite_row_job_event,
    )
}

/// Structured OpenTofu events only. The raw log queries leave this stream out.
pub(crate) fn tofu_events_after(
    c: &Connection,
    job_id: &str,
    after: i64,
    limit: i64,
) -> AppResult<Vec<JobEvent>> {
    sql::sqlite_all(
        c,
        TOFU_EVENTS_AFTER,
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

pub(crate) fn insert_host_results(
    c: &Connection,
    job_id: &str,
    team_id: &str,
    inventory_id: Option<&str>,
    runs: &[db::HostRun],
    at: &str,
) -> AppResult<()> {
    for run in runs {
        c.execute(
            INSERT_HOST_RESULT,
            params![
                job_id,
                team_id,
                inventory_id,
                run.host,
                run.playbook,
                run.role,
                run.result,
                count(run.ok),
                count(run.changed),
                count(run.failed),
                count(run.unreachable),
                count(run.skipped),
                at
            ],
        )?;
    }
    Ok(())
}

fn count(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(crate) fn host_history(
    c: &Connection,
    team_id: &str,
    inventory_id: &str,
    host: &str,
    limit: i64,
    offset: i64,
) -> AppResult<(Vec<db::HostHistoryEntry>, i64)> {
    let entries = sql::sqlite_all(
        c,
        HOST_HISTORY,
        params![team_id, inventory_id, host, limit, offset],
        sqlite_row_host_history,
    )?;
    let total = sql::sqlite_opt(
        c,
        HOST_HISTORY_COUNT,
        params![team_id, inventory_id, host],
        |row| SqlRow::get(row, 0),
    )?
    .unwrap_or(0);
    Ok((entries, total))
}

pub(crate) fn row_host_history<R: SqlRow>(row: &R) -> AppResult<db::HostHistoryEntry> {
    Ok(db::HostHistoryEntry {
        id: row.get(0)?,
        job_id: row.get(1)?,
        job_name: row.get(2)?,
        job_status: row.get(3)?,
        host: row.get(4)?,
        playbook: row.get(5)?,
        role: row.get(6)?,
        result: row.get(7)?,
        ok: row.get(8)?,
        changed: row.get(9)?,
        failed: row.get(10)?,
        unreachable: row.get(11)?,
        skipped: row.get(12)?,
        time: row.get(13)?,
    })
}

fn sqlite_row_host_history(row: &rusqlite::Row<'_>) -> AppResult<db::HostHistoryEntry> {
    row_host_history(row)
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
        template_name: row.get(1)?,
        started_at: row.get(2)?,
        finished_at: row.get(3)?,
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
