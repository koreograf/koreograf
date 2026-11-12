use rusqlite::{params, Connection};

use crate::db::{Workflow, WorkflowJob, WorkflowNodeRow};
use crate::error::{ApiError, AppResult};
use crate::store::sql::{self, SqlRow};

pub(crate) const WORKFLOW_COLUMNS: &str =
    "id, team_id, name, description, graph, created_at, admin_saved";
pub(crate) const WORKFLOW_JOB_COLUMNS: &str =
    "id, workflow_id, team_id, name, status, extra_vars, launched_by, trigger, created_at, \
     finished_at";

pub(crate) const INSERT_WORKFLOW: &str = "INSERT INTO workflows (
        id, team_id, name, description, graph, created_at, admin_saved
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

pub(crate) const UPDATE_WORKFLOW: &str = "UPDATE workflows
     SET name = ?3, description = ?4, graph = ?5, admin_saved = ?6
     WHERE id = ?1 AND team_id = ?2";

pub(crate) const DELETE_WORKFLOW: &str = "DELETE FROM workflows WHERE id = ?1 AND team_id = ?2";

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
            item.admin_saved as i64
        ],
    )?;
    Ok(())
}

pub(crate) fn update_workflow(c: &Connection, item: &Workflow) -> AppResult<()> {
    let changed = c.execute(
        UPDATE_WORKFLOW,
        params![
            item.id,
            item.team_id,
            item.name,
            item.description,
            item.graph,
            item.admin_saved as i64
        ],
    )?;
    if changed == 0 {
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

pub(crate) fn delete_workflow(c: &Connection, id: &str, team_id: &str) -> AppResult<()> {
    let changed = c.execute(DELETE_WORKFLOW, params![id, team_id])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    c.execute(
        crate::store::schedules::DELETE_SCHEDULES_FOR,
        params!["workflow", id],
    )?;
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

pub(crate) fn workflow_job(c: &Connection, id: &str) -> AppResult<Option<WorkflowJob>> {
    sql::sqlite_opt(c, &workflow_job_by_id_sql(), [id], sqlite_row_workflow_job)
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
