use rusqlite::{params, Connection};

use crate::db::Environment;
use crate::error::{ApiError, AppResult};
use crate::store::sql::{self, SqlRow};

pub(crate) const INSERT_ENVIRONMENT: &str =
    "INSERT INTO environments (
        id, team_id, organization_id, name, tool, image, pull_policy, network, description, created_at
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)";
pub(crate) const LIST_ENVIRONMENTS: &str =
    "SELECT e.id, e.team_id, e.organization_id, e.name, e.image, e.pull_policy, \
     e.network, e.description, e.created_at, COALESCE(o.name, ''), COALESCE(t.name, ''), e.tool
     FROM environments e
     LEFT JOIN organizations o ON o.id = e.organization_id
     LEFT JOIN teams t ON t.id = e.team_id
     WHERE (
         (e.team_id IS NULL OR e.team_id = '')
         AND (e.organization_id IS NULL OR e.organization_id = '')
     ) OR (
         (e.team_id IS NULL OR e.team_id = '')
         AND e.organization_id = ?1
         AND ?1 != ''
     ) OR e.team_id = ?2
     ORDER BY
       CASE
         WHEN e.team_id IS NOT NULL AND e.team_id != '' THEN 2
         WHEN e.organization_id IS NOT NULL AND e.organization_id != '' THEN 1
         ELSE 0
       END,
       e.name";
pub(crate) const ENVIRONMENT_BY_ID: &str =
    "SELECT e.id, e.team_id, e.organization_id, e.name, e.image, e.pull_policy, \
     e.network, e.description, e.created_at, COALESCE(o.name, ''), COALESCE(t.name, ''), e.tool
     FROM environments e
     LEFT JOIN organizations o ON o.id = e.organization_id
     LEFT JOIN teams t ON t.id = e.team_id
     WHERE e.id = ?1";
pub(crate) const UPDATE_ENVIRONMENT: &str = "UPDATE environments
     SET team_id = ?2, organization_id = ?3, name = ?4, tool = ?5, image = ?6,
         pull_policy = ?7, network = ?8, description = ?9
     WHERE id = ?1";
pub(crate) const DELETE_ENVIRONMENT: &str = "DELETE FROM environments WHERE id = ?1";
pub(crate) const DELETE_DEFAULT_SETTING: &str =
    "DELETE FROM settings WHERE key = ?1 AND value = ?2";

pub(crate) fn insert_environment(c: &Connection, item: &Environment) -> AppResult<()> {
    c.execute(
        INSERT_ENVIRONMENT,
        params![
            item.id,
            item.team_id,
            item.organization_id,
            item.name,
            item.tool,
            item.image,
            item.pull_policy,
            item.network,
            item.description,
            item.created_at
        ],
    )?;
    Ok(())
}

pub(crate) fn list_environments(
    c: &Connection,
    team: &crate::db::Team,
) -> AppResult<Vec<Environment>> {
    sql::sqlite_all(
        c,
        LIST_ENVIRONMENTS,
        params![team.organization_id, team.id],
        |row| row_environment(row),
    )
}

pub(crate) fn environment(c: &Connection, id: &str) -> AppResult<Option<Environment>> {
    sql::sqlite_opt(c, ENVIRONMENT_BY_ID, [id], |row| row_environment(row))
}

pub(crate) fn update_environment(c: &Connection, item: &Environment) -> AppResult<()> {
    let changed = c.execute(
        UPDATE_ENVIRONMENT,
        params![
            item.id,
            item.team_id,
            item.organization_id,
            item.name,
            item.tool,
            item.image,
            item.pull_policy,
            item.network,
            item.description
        ],
    )?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    let global = item.team_id.is_none() && item.organization_id.is_none();
    for key in crate::settings::DEFAULT_ENVIRONMENT_KEYS {
        let tool = key.trim_start_matches("default_environment_");
        if global && crate::runner::environment_fits_tool(tool, &item.tool) {
            continue;
        }
        c.execute(DELETE_DEFAULT_SETTING, params![key, item.id])?;
    }
    Ok(())
}

pub(crate) fn delete_environment(c: &Connection, id: &str) -> AppResult<()> {
    let changed = c.execute(DELETE_ENVIRONMENT, [id])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    for key in crate::settings::DEFAULT_ENVIRONMENT_KEYS {
        c.execute(DELETE_DEFAULT_SETTING, params![key, id])?;
    }
    Ok(())
}

pub(crate) fn row_environment<R: SqlRow>(row: &R) -> AppResult<Environment> {
    let team_id = optional_id(row.get(1)?);
    let organization_id = optional_id(row.get(2)?);
    let organization_name: String = row.get(9)?;
    let team_name: String = row.get(10)?;
    let scope = if team_id.is_some() {
        "team"
    } else if organization_id.is_some() {
        "organization"
    } else {
        "global"
    };
    Ok(Environment {
        id: row.get(0)?,
        team_id,
        organization_id,
        name: row.get(3)?,
        tool: row.get(11)?,
        image: row.get(4)?,
        pull_policy: row.get(5)?,
        network: row.get(6)?,
        description: row.get(7)?,
        created_at: row.get(8)?,
        scope: scope.to_string(),
        organization_name: if scope == "organization" {
            organization_name
        } else {
            String::new()
        },
        team_name: if scope == "team" {
            team_name
        } else {
            String::new()
        },
    })
}

fn optional_id(value: Option<String>) -> Option<String> {
    value.and_then(|item| {
        let item = item.trim().to_string();
        if item.is_empty() {
            None
        } else {
            Some(item)
        }
    })
}
