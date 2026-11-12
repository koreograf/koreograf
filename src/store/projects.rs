use rusqlite::{params, Connection};

use crate::db::Project;
use crate::error::{ApiError, AppResult};
use crate::store::sql::{self, SqlRow};

pub(crate) const INSERT_PROJECT: &str = "INSERT INTO projects (
        id, team_id, organization_id, name, source_type, source, revision, credential_id,
        created_at, admin_saved
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)";
pub(crate) const LIST_PROJECTS: &str =
    "SELECT p.id, p.team_id, COALESCE(p.organization_id, ''), COALESCE(o.name, ''), \
     p.name, p.source_type, p.source, p.revision, p.credential_id, p.created_at, p.admin_saved, \
     p.last_sync_at, p.last_sync_commit, p.last_sync_status
     FROM projects p
     JOIN teams t ON t.id = ?1
     LEFT JOIN organizations o ON o.id = p.organization_id
     WHERE p.organization_id = t.organization_id
       AND (
         NOT EXISTS (SELECT 1 FROM project_teams pt WHERE pt.project_id = p.id)
         OR EXISTS (
           SELECT 1 FROM project_teams pt WHERE pt.project_id = p.id AND pt.team_id = t.id
         )
       )
     ORDER BY p.name";
pub(crate) const PROJECT_BY_ID: &str =
    "SELECT p.id, p.team_id, COALESCE(p.organization_id, ''), COALESCE(o.name, ''), \
     p.name, p.source_type, p.source, p.revision, p.credential_id, p.created_at, p.admin_saved, \
     p.last_sync_at, p.last_sync_commit, p.last_sync_status
     FROM projects p
     LEFT JOIN organizations o ON o.id = p.organization_id WHERE p.id = ?1";
pub(crate) const PROJECT_VISIBLE: &str = "SELECT 1 FROM projects p
     JOIN teams t ON t.id = ?2
     WHERE p.id = ?1 AND p.organization_id = t.organization_id
       AND (
         NOT EXISTS (SELECT 1 FROM project_teams pt WHERE pt.project_id = p.id)
         OR EXISTS (
           SELECT 1 FROM project_teams pt WHERE pt.project_id = p.id AND pt.team_id = t.id
         )
       )";
pub(crate) const DELETE_PROJECT: &str = "DELETE FROM projects
     WHERE id = ?1
       AND EXISTS (
         SELECT 1 FROM teams t
         WHERE t.id = ?2 AND t.organization_id = projects.organization_id
           AND (
             NOT EXISTS (SELECT 1 FROM project_teams pt WHERE pt.project_id = projects.id)
             OR EXISTS (
               SELECT 1 FROM project_teams pt
               WHERE pt.project_id = projects.id AND pt.team_id = t.id
             )
           )
       )";
pub(crate) const PROJECT_TEAM_IDS: &str =
    "SELECT team_id FROM project_teams WHERE project_id = ?1 ORDER BY team_id";
pub(crate) const DELETE_PROJECT_TEAMS: &str = "DELETE FROM project_teams WHERE project_id = ?1";
pub(crate) const INSERT_PROJECT_TEAM: &str =
    "INSERT INTO project_teams (project_id, team_id) VALUES (?1, ?2)";
pub(crate) const SET_PROJECT_ORGANIZATION: &str =
    "UPDATE projects SET organization_id = ?2 WHERE id = ?1";
pub(crate) const SET_PROJECT_DETAILS: &str =
    "UPDATE projects SET name = ?2, source_type = ?3, source = ?4, revision = ?5 WHERE id = ?1";
pub(crate) const SET_PROJECT_CREDENTIAL: &str =
    "UPDATE projects SET credential_id = ?2 WHERE id = ?1";
pub(crate) const SET_PROJECT_SYNC_SUCCESS: &str =
    "UPDATE projects SET last_sync_at = ?2, last_sync_commit = ?3, last_sync_status = 'success'
     WHERE id = ?1";
pub(crate) const SET_PROJECT_SYNC_FAILED: &str =
    "UPDATE projects SET last_sync_status = 'failed' WHERE id = ?1";
pub(crate) const SET_PROJECT_ADMIN_SAVED: &str =
    "UPDATE projects SET admin_saved = ?2 WHERE id = ?1";

pub(crate) fn insert_project(c: &Connection, project: &Project) -> AppResult<()> {
    c.execute(
        INSERT_PROJECT,
        params![
            project.id,
            project.team_id,
            project.organization_id,
            project.name,
            project.source_type,
            project.source,
            project.revision,
            project.credential_id,
            project.created_at,
            project.admin_saved as i64
        ],
    )?;
    replace_project_teams(c, &project.id, &project.team_ids)
}

pub(crate) fn list_projects(c: &Connection, team_id: &str) -> AppResult<Vec<Project>> {
    let mut projects = sql::sqlite_all(c, LIST_PROJECTS, [team_id], |row| row_project(row))?;
    for project in &mut projects {
        project.team_ids = project_team_ids(c, &project.id)?;
    }
    Ok(projects)
}

pub(crate) fn project(c: &Connection, id: &str) -> AppResult<Option<Project>> {
    let mut found = sql::sqlite_opt(c, PROJECT_BY_ID, [id], |row| row_project(row))?;
    if let Some(project) = &mut found {
        project.team_ids = project_team_ids(c, &project.id)?;
    }
    Ok(found)
}

pub(crate) fn project_visible(c: &Connection, id: &str, team_id: &str) -> AppResult<bool> {
    let found = sql::sqlite_opt(c, PROJECT_VISIBLE, params![id, team_id], |row| {
        SqlRow::get::<i64>(row, 0)
    })?;
    Ok(found.is_some())
}

pub(crate) fn project_team_ids(c: &Connection, project_id: &str) -> AppResult<Vec<String>> {
    sql::sqlite_all(c, PROJECT_TEAM_IDS, [project_id], |row| SqlRow::get(row, 0))
}

pub(crate) fn replace_project_teams(
    c: &Connection,
    project_id: &str,
    team_ids: &[String],
) -> AppResult<()> {
    // SQL savepoint: `seed` already holds a transaction, and `Connection::savepoint` needs `&mut`.
    c.execute("SAVEPOINT koreograf_project_teams", [])?;
    let result = write_project_teams(c, project_id, team_ids);
    if result.is_ok() {
        c.execute("RELEASE koreograf_project_teams", [])?;
    } else {
        let _ = c.execute("ROLLBACK TO koreograf_project_teams", []);
        let _ = c.execute("RELEASE koreograf_project_teams", []);
    }
    result
}

fn write_project_teams(c: &Connection, project_id: &str, team_ids: &[String]) -> AppResult<()> {
    c.execute(DELETE_PROJECT_TEAMS, [project_id])?;
    let mut seen = std::collections::BTreeSet::new();
    for team_id in team_ids {
        if !seen.insert(team_id.as_str()) {
            continue;
        }
        c.execute(INSERT_PROJECT_TEAM, params![project_id, team_id])?;
    }
    Ok(())
}

pub(crate) fn set_project_organization(
    c: &Connection,
    id: &str,
    organization_id: &str,
) -> AppResult<()> {
    let changed = c.execute(SET_PROJECT_ORGANIZATION, params![id, organization_id])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn set_project_details(
    c: &Connection,
    id: &str,
    name: &str,
    source_type: &str,
    source: &str,
    revision: &str,
) -> AppResult<()> {
    let changed = c.execute(
        SET_PROJECT_DETAILS,
        params![id, name, source_type, source, revision],
    )?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn set_project_credential(
    c: &Connection,
    id: &str,
    credential_id: Option<&str>,
) -> AppResult<()> {
    let changed = c.execute(SET_PROJECT_CREDENTIAL, params![id, credential_id])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn set_project_sync_success(
    c: &Connection,
    id: &str,
    at: &str,
    commit: &str,
) -> AppResult<()> {
    let changed = c.execute(SET_PROJECT_SYNC_SUCCESS, params![id, at, commit])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn set_project_sync_failed(c: &Connection, id: &str) -> AppResult<()> {
    let changed = c.execute(SET_PROJECT_SYNC_FAILED, [id])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn set_project_admin_saved(
    c: &Connection,
    id: &str,
    admin_saved: bool,
) -> AppResult<()> {
    let changed = c.execute(SET_PROJECT_ADMIN_SAVED, params![id, admin_saved as i64])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn delete_project(c: &Connection, id: &str, team_id: &str) -> AppResult<()> {
    let changed = c.execute(DELETE_PROJECT, params![id, team_id])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn row_project<R: SqlRow>(row: &R) -> AppResult<Project> {
    Ok(Project {
        id: row.get(0)?,
        team_id: row.get(1)?,
        organization_id: row.get(2)?,
        organization_name: row.get(3)?,
        name: row.get(4)?,
        source_type: row.get(5)?,
        source: row.get(6)?,
        revision: row.get(7)?,
        credential_id: row.get(8)?,
        created_at: row.get(9)?,
        team_ids: Vec::new(),
        admin_saved: row.get(10)?,
        last_sync_at: row.get(11)?,
        last_sync_commit: row.get(12)?,
        last_sync_status: row.get(13)?,
    })
}

pub(crate) async fn postgres_project_team_ids<C>(
    client: &C,
    project_id: &str,
) -> AppResult<Vec<String>>
where
    C: deadpool_postgres::GenericClient,
{
    let rows = client
        .query(&sql::postgres(PROJECT_TEAM_IDS), &[&project_id])
        .await?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

pub(crate) async fn postgres_write_project_teams<C>(
    client: &C,
    project_id: &str,
    team_ids: &[String],
) -> AppResult<()>
where
    C: deadpool_postgres::GenericClient,
{
    client
        .execute(&sql::postgres(DELETE_PROJECT_TEAMS), &[&project_id])
        .await?;
    let mut seen = std::collections::BTreeSet::new();
    for team_id in team_ids {
        if !seen.insert(team_id.as_str()) {
            continue;
        }
        client
            .execute(
                &sql::postgres(INSERT_PROJECT_TEAM),
                &[&project_id, &team_id],
            )
            .await?;
    }
    Ok(())
}
