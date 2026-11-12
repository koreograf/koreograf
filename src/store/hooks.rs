use rusqlite::{params, Connection};

use crate::db::Hook;
use crate::error::{ApiError, AppResult};
use crate::store::sql::{self, SqlRow};

pub(crate) const INSERT_HOOK: &str = "INSERT INTO hooks (
        id, team_id, name, token, match_alertname, match_status, template_id, workflow_id,
        extra_vars, enabled, created_at
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)";
pub(crate) const LIST_HOOKS: &str =
    "SELECT id, team_id, name, token, match_alertname, match_status,
     template_id, workflow_id, extra_vars, enabled, created_at
 FROM hooks WHERE team_id = ?1 ORDER BY name";
pub(crate) const HOOK_BY_ID: &str =
    "SELECT id, team_id, name, token, match_alertname, match_status,
     template_id, workflow_id, extra_vars, enabled, created_at
 FROM hooks WHERE id = ?1";
pub(crate) const FIRST_HOOK: &str =
    "SELECT id, team_id, name, token, match_alertname, match_status,
     template_id, workflow_id, extra_vars, enabled, created_at
 FROM hooks WHERE team_id = ?1 ORDER BY created_at LIMIT 1";
pub(crate) const UPDATE_HOOK: &str =
    "UPDATE hooks SET name = ?3, match_alertname = ?4, match_status = ?5,
        template_id = ?6, workflow_id = ?7, extra_vars = ?8
     WHERE id = ?1 AND team_id = ?2";
pub(crate) const DELETE_HOOK: &str = "DELETE FROM hooks WHERE id = ?1 AND team_id = ?2";

pub(crate) fn insert_hook(c: &Connection, item: &Hook) -> AppResult<()> {
    c.execute(
        INSERT_HOOK,
        params![
            item.id,
            item.team_id,
            item.name,
            item.token,
            item.match_alertname,
            item.match_status,
            item.template_id,
            item.workflow_id,
            item.extra_vars,
            item.enabled as i64,
            item.created_at
        ],
    )?;
    Ok(())
}

pub(crate) fn list_hooks(c: &Connection, team_id: &str) -> AppResult<Vec<Hook>> {
    sql::sqlite_all(c, LIST_HOOKS, [team_id], |row| row_hook(row))
}

pub(crate) fn hook(c: &Connection, id: &str) -> AppResult<Option<Hook>> {
    sql::sqlite_opt(c, HOOK_BY_ID, [id], |row| row_hook(row))
}

pub(crate) fn first_hook(c: &Connection, team_id: &str) -> AppResult<Option<Hook>> {
    sql::sqlite_opt(c, FIRST_HOOK, [team_id], |row| row_hook(row))
}

pub(crate) fn update_hook(c: &Connection, item: &Hook) -> AppResult<()> {
    let changed = c.execute(
        UPDATE_HOOK,
        params![
            item.id,
            item.team_id,
            item.name,
            item.match_alertname,
            item.match_status,
            item.template_id,
            item.workflow_id,
            item.extra_vars
        ],
    )?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn delete_hook(c: &Connection, id: &str, team_id: &str) -> AppResult<()> {
    let changed = c.execute(DELETE_HOOK, params![id, team_id])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn row_hook<R: SqlRow>(row: &R) -> AppResult<Hook> {
    Ok(Hook {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        token: row.get(3)?,
        match_alertname: row.get(4)?,
        match_status: row.get(5)?,
        template_id: row.get(6)?,
        workflow_id: row.get(7)?,
        extra_vars: row.get(8)?,
        enabled: row.get(9)?,
        created_at: row.get(10)?,
    })
}
