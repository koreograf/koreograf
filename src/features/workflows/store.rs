use rusqlite::{params, Connection};

use crate::db::{Workflow, WorkflowJob, WorkflowNodeRow};
use crate::error::{ApiError, AppResult};
use crate::store::sql::{self, SqlRow};

pub(crate) const WORKFLOW_COLUMNS: &str =
    "id, team_id, name, description, graph, created_at, admin_saved, version";
pub(crate) const WORKFLOW_JOB_COLUMNS: &str =
    "id, workflow_id, team_id, name, status, extra_vars, launched_by, trigger, created_at, \
     finished_at";

pub(crate) const INSERT_WORKFLOW: &str = "INSERT INTO workflows (
        id, team_id, name, description, graph, created_at, admin_saved, version
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)";

pub(crate) const UPDATE_WORKFLOW: &str = "UPDATE workflows
     SET name = ?3, description = ?4, graph = ?5, admin_saved = ?6,
         version = version + 1
     WHERE id = ?1 AND team_id = ?2
       AND (CAST(?7 AS BIGINT) IS NULL OR version = CAST(?7 AS BIGINT))";

pub(crate) const DELETE_WORKFLOW: &str = "DELETE FROM workflows WHERE id = ?1 AND team_id = ?2";

pub(crate) const DELETE_WORKFLOW_NODES_FOR_WORKFLOW: &str = "DELETE FROM workflow_nodes
     WHERE workflow_job_id IN (SELECT id FROM workflow_jobs WHERE workflow_id = ?1)";

pub(crate) const DELETE_WORKFLOW_JOBS_FOR_WORKFLOW: &str =
    "DELETE FROM workflow_jobs WHERE workflow_id = ?1";

/// Finished runs past the retention cutoff. A run without `finished_at` and a
/// pending or running run never match.
pub(crate) const FINISHED_WORKFLOW_RUNS_BEFORE: &str = "SELECT id FROM workflow_jobs
     WHERE status IN ('successful', 'failed', 'canceled')
       AND finished_at IS NOT NULL
       AND finished_at < ?1
     ORDER BY finished_at, id
     LIMIT ?2";

pub(crate) const DELETE_WORKFLOW_NODES_FOR_RUN: &str =
    "DELETE FROM workflow_nodes WHERE workflow_job_id = ?1";

pub(crate) const DELETE_WORKFLOW_RUN: &str = "DELETE FROM workflow_jobs WHERE id = ?1";

/// Runs and nodes that one prune batch removed.
pub(crate) struct PrunedWorkflowRuns {
    pub(crate) run_ids: Vec<String>,
    pub(crate) nodes: usize,
}

pub(crate) fn workflow_by_id_sql() -> String {
    format!("SELECT {WORKFLOW_COLUMNS} FROM workflows WHERE id = ?1")
}

pub(crate) fn list_workflows_sql() -> String {
    format!("SELECT {WORKFLOW_COLUMNS} FROM workflows WHERE team_id = ?1 ORDER BY name")
}

pub(crate) fn first_workflow_sql() -> String {
    format!(
        "SELECT {WORKFLOW_COLUMNS} FROM workflows WHERE team_id = ?1 ORDER BY created_at LIMIT 1"
    )
}

pub(crate) fn workflow_job_by_id_sql() -> String {
    format!("SELECT {WORKFLOW_JOB_COLUMNS} FROM workflow_jobs WHERE id = ?1")
}

pub(crate) fn list_workflow_jobs_sql() -> String {
    format!(
        "SELECT {WORKFLOW_JOB_COLUMNS} FROM workflow_jobs
         WHERE team_id = ?1 AND (?2 = '' OR workflow_id = ?2)
         ORDER BY created_at DESC, id DESC LIMIT ?3"
    )
}

pub(crate) const WORKFLOW_TEAM: &str = "SELECT team_id FROM workflows WHERE id = ?1";

pub(crate) const INSERT_WORKFLOW_JOB: &str = "INSERT INTO workflow_jobs (
        id, workflow_id, team_id, name, status, extra_vars, launched_by, trigger, created_at,
        finished_at
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)";

pub(crate) const SET_WORKFLOW_STATUS: &str = "UPDATE workflow_jobs SET status = ?2 WHERE id = ?1";

pub(crate) const SET_WORKFLOW_STATUS_FINISHED: &str =
    "UPDATE workflow_jobs SET status = ?2, finished_at = ?3 WHERE id = ?1";

pub(crate) const INSERT_WORKFLOW_NODE: &str =
    "INSERT INTO workflow_nodes (workflow_job_id, node_id, phase, job_id)
     VALUES (?1, ?2, 'pending', NULL)";

pub(crate) const SET_WORKFLOW_NODE: &str =
    "UPDATE workflow_nodes SET phase = ?3, job_id = COALESCE(?4, job_id)
     WHERE workflow_job_id = ?1 AND node_id = ?2";

pub(crate) const WORKFLOW_NODES: &str =
    "SELECT node_id, phase, job_id FROM workflow_nodes WHERE workflow_job_id = ?1";

pub(crate) const WORKFLOW_JOBS_BUSY: &str =
    "SELECT COUNT(*) FROM workflow_jobs WHERE status IN ('pending', 'running')";

/// Running runs plus failed runs that finished inside the window. The
/// dashboard counts the same rows without the list limit, so a busy team does
/// not undercount or lose a run behind older history.
pub(crate) const WORKFLOW_GLANCE_COUNTS: &str = "SELECT
        COALESCE(SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END), 0),
        COALESCE(SUM(CASE WHEN status = 'failed'
            AND COALESCE(finished_at, created_at) >= ?2 THEN 1 ELSE 0 END), 0)
     FROM workflow_jobs WHERE team_id = ?1";

pub(crate) fn workflow_glance_items_sql() -> String {
    format!(
        "SELECT {WORKFLOW_JOB_COLUMNS} FROM workflow_jobs
         WHERE team_id = ?1
           AND (status = 'running'
             OR (status = 'failed' AND COALESCE(finished_at, created_at) >= ?2))
         ORDER BY CASE WHEN status = 'running' THEN 0 ELSE 1 END,
                  CASE WHEN status = 'running' THEN created_at END ASC,
                  CASE WHEN status = 'failed' THEN created_at END DESC,
                  id
         LIMIT ?3"
    )
}

pub(crate) struct GlanceWorkflowRuns {
    pub(crate) running: i64,
    pub(crate) failed: i64,
    pub(crate) items: Vec<WorkflowJob>,
}

pub(crate) fn glance_workflow_runs(
    c: &Connection,
    team_id: &str,
    cutoff: &str,
    limit: i64,
) -> AppResult<GlanceWorkflowRuns> {
    let counts = sql::sqlite_opt(c, WORKFLOW_GLANCE_COUNTS, params![team_id, cutoff], |row| {
        Ok((SqlRow::get::<i64>(row, 0)?, SqlRow::get::<i64>(row, 1)?))
    })?;
    let (running, failed) = counts.unwrap_or((0, 0));
    let items = sql::sqlite_all(
        c,
        &workflow_glance_items_sql(),
        params![team_id, cutoff, limit.clamp(1, 100)],
        sqlite_row_workflow_job,
    )?;
    Ok(GlanceWorkflowRuns {
        running,
        failed,
        items,
    })
}

pub(crate) fn insert_workflow(c: &Connection, item: &Workflow) -> AppResult<()> {
    c.execute(
        INSERT_WORKFLOW,
        params![
            item.id,
            item.team_id,
            item.name,
            item.description,
            item.graph,
            item.created_at,
            item.admin_saved as i64,
            item.version
        ],
    )?;
    Ok(())
}

pub(crate) fn update_workflow(
    c: &Connection,
    item: &Workflow,
    expected_version: Option<i64>,
) -> AppResult<()> {
    let changed = c.execute(
        UPDATE_WORKFLOW,
        params![
            item.id,
            item.team_id,
            item.name,
            item.description,
            item.graph,
            item.admin_saved as i64,
            expected_version
        ],
    )?;
    if changed == 0 {
        if let Some(current) = workflow(c, &item.id)? {
            if current.team_id == item.team_id {
                return Err(ApiError::conflict_vars(
                    "workflow_changed",
                    [("version", current.version.to_string())],
                ));
            }
        }
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn list_workflows(c: &Connection, team_id: &str) -> AppResult<Vec<Workflow>> {
    sql::sqlite_all(c, &list_workflows_sql(), [team_id], sqlite_row_workflow)
}

pub(crate) fn workflow(c: &Connection, id: &str) -> AppResult<Option<Workflow>> {
    sql::sqlite_opt(c, &workflow_by_id_sql(), [id], sqlite_row_workflow)
}

/// Deletes the workflow, its schedules, and every run and node it owns in one
/// transaction, so a deleted workflow leaves no history behind.
pub(crate) fn delete_workflow(c: &mut Connection, id: &str, team_id: &str) -> AppResult<()> {
    let tx = c.transaction()?;
    let changed = tx.execute(DELETE_WORKFLOW, params![id, team_id])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    tx.execute(DELETE_WORKFLOW_NODES_FOR_WORKFLOW, params![id])?;
    tx.execute(DELETE_WORKFLOW_JOBS_FOR_WORKFLOW, params![id])?;
    tx.execute(
        crate::features::schedules::store::DELETE_SCHEDULES_FOR,
        params!["workflow", id],
    )?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn first_workflow(c: &Connection, team_id: &str) -> AppResult<Option<Workflow>> {
    sql::sqlite_opt(c, &first_workflow_sql(), [team_id], sqlite_row_workflow)
}

pub(crate) fn insert_workflow_job(c: &Connection, item: &WorkflowJob) -> AppResult<()> {
    c.execute(
        INSERT_WORKFLOW_JOB,
        params![
            item.id,
            item.workflow_id,
            item.team_id,
            item.name,
            item.status,
            item.extra_vars,
            item.launched_by,
            item.trigger,
            item.created_at,
            item.finished_at
        ],
    )?;
    Ok(())
}

/// Deletes up to `limit` finished runs older than `cutoff` and their nodes in
/// one transaction. The caller repeats the call until it returns no runs, so
/// the database lock is released between batches.
pub(crate) fn prune_workflow_runs(
    c: &mut Connection,
    cutoff: &str,
    limit: i64,
) -> AppResult<PrunedWorkflowRuns> {
    let tx = c.transaction()?;
    let run_ids: Vec<String> = sql::sqlite_all(
        &tx,
        FINISHED_WORKFLOW_RUNS_BEFORE,
        params![cutoff, limit],
        |row| SqlRow::get(row, 0),
    )?;
    let mut nodes = 0usize;
    for id in &run_ids {
        nodes = nodes.saturating_add(tx.execute(DELETE_WORKFLOW_NODES_FOR_RUN, [id])?);
        tx.execute(DELETE_WORKFLOW_RUN, [id])?;
    }
    tx.commit()?;
    Ok(PrunedWorkflowRuns { run_ids, nodes })
}

pub(crate) fn workflow_job(c: &Connection, id: &str) -> AppResult<Option<WorkflowJob>> {
    sql::sqlite_opt(c, &workflow_job_by_id_sql(), [id], sqlite_row_workflow_job)
}

pub(crate) fn list_workflow_jobs(
    c: &Connection,
    team_id: &str,
    workflow_id: &str,
    limit: i64,
) -> AppResult<Vec<WorkflowJob>> {
    sql::sqlite_all(
        c,
        &list_workflow_jobs_sql(),
        params![team_id, workflow_id, limit.clamp(1, 1000)],
        sqlite_row_workflow_job,
    )
}

pub(crate) fn set_workflow_status(
    c: &Connection,
    id: &str,
    status: &str,
    finish: bool,
) -> AppResult<()> {
    if finish {
        c.execute(
            SET_WORKFLOW_STATUS_FINISHED,
            params![id, status, crate::db::now()],
        )?;
    } else {
        c.execute(SET_WORKFLOW_STATUS, params![id, status])?;
    }
    Ok(())
}

pub(crate) fn insert_workflow_node(
    c: &Connection,
    workflow_job_id: &str,
    node_id: &str,
) -> AppResult<()> {
    c.execute(INSERT_WORKFLOW_NODE, params![workflow_job_id, node_id])?;
    Ok(())
}

pub(crate) fn set_workflow_node(
    c: &Connection,
    workflow_job_id: &str,
    node_id: &str,
    phase: &str,
    job_id: Option<&str>,
) -> AppResult<()> {
    c.execute(
        SET_WORKFLOW_NODE,
        params![workflow_job_id, node_id, phase, job_id],
    )?;
    Ok(())
}

pub(crate) fn workflow_nodes(
    c: &Connection,
    workflow_job_id: &str,
) -> AppResult<Vec<WorkflowNodeRow>> {
    sql::sqlite_all(
        c,
        WORKFLOW_NODES,
        [workflow_job_id],
        sqlite_row_workflow_node,
    )
}

pub(crate) fn row_workflow<R: SqlRow>(row: &R) -> AppResult<Workflow> {
    Ok(Workflow {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        graph: row.get(4)?,
        created_at: row.get(5)?,
        admin_saved: row.get(6)?,
        version: row.get(7)?,
    })
}

pub(crate) fn row_workflow_job<R: SqlRow>(row: &R) -> AppResult<WorkflowJob> {
    Ok(WorkflowJob {
        id: row.get(0)?,
        workflow_id: row.get(1)?,
        team_id: row.get(2)?,
        name: row.get(3)?,
        status: row.get(4)?,
        extra_vars: row.get(5)?,
        launched_by: row.get(6)?,
        trigger: row.get(7)?,
        created_at: row.get(8)?,
        finished_at: row.get(9)?,
    })
}

pub(crate) fn row_workflow_node<R: SqlRow>(row: &R) -> AppResult<WorkflowNodeRow> {
    Ok(WorkflowNodeRow {
        node_id: row.get(0)?,
        phase: row.get(1)?,
        job_id: row.get(2)?,
    })
}

fn sqlite_row_workflow(row: &rusqlite::Row<'_>) -> AppResult<Workflow> {
    row_workflow(row)
}

fn sqlite_row_workflow_job(row: &rusqlite::Row<'_>) -> AppResult<WorkflowJob> {
    row_workflow_job(row)
}

fn sqlite_row_workflow_node(row: &rusqlite::Row<'_>) -> AppResult<WorkflowNodeRow> {
    row_workflow_node(row)
}
