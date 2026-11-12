use rusqlite::{params, Connection};

use crate::db::Template;
use crate::error::{ApiError, AppResult};
use crate::store::sql::{self, SqlRow};

pub(crate) const INSERT_TEMPLATE: &str = "INSERT INTO templates (
        id, team_id, name, description, tool, project_id, inventory_id, credential_id,
        environment_id, no_environment, entrypoint, extra_vars, spec, created_at, update_on_launch,
        admin_saved, prompt_on_launch
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)";
pub(crate) const UPDATE_TEMPLATE: &str =
    "UPDATE templates SET name = ?3, description = ?4, tool = ?5, project_id = ?6,
        inventory_id = ?7, credential_id = ?8, environment_id = ?9, no_environment = ?10,
        entrypoint = ?11, extra_vars = ?12, spec = ?13, update_on_launch = ?14,
        admin_saved = ?15, prompt_on_launch = ?16
     WHERE id = ?1 AND team_id = ?2";
pub(crate) const LIST_TEMPLATES: &str =
    "SELECT id, team_id, name, description, tool, project_id, inventory_id, credential_id,
        environment_id, no_environment, entrypoint, extra_vars, spec, created_at, update_on_launch,
        admin_saved, prompt_on_launch
     FROM templates WHERE team_id = ?1 ORDER BY name";
pub(crate) const TEMPLATE_BY_ID: &str =
    "SELECT id, team_id, name, description, tool, project_id, inventory_id, credential_id,
        environment_id, no_environment, entrypoint, extra_vars, spec, created_at, update_on_launch,
        admin_saved, prompt_on_launch
     FROM templates WHERE id = ?1";
pub(crate) const DELETE_TEMPLATE: &str = "DELETE FROM templates WHERE id = ?1 AND team_id = ?2";
pub(crate) const DELETE_TEMPLATE_SCHEDULES: &str =
    "DELETE FROM schedules WHERE target_kind = 'template' AND target_id = ?1";
pub(crate) const TEMPLATE_CREDENTIAL_IDS: &str = "SELECT credential_id FROM template_credentials
     WHERE template_id = ?1 ORDER BY position, credential_id";
pub(crate) const LIST_TEMPLATE_CREDENTIAL_LINKS: &str = "SELECT tc.template_id, tc.credential_id
     FROM template_credentials tc
     JOIN templates t ON t.id = tc.template_id
     WHERE t.team_id = ?1
     ORDER BY tc.template_id, tc.position, tc.credential_id";
pub(crate) const DELETE_TEMPLATE_CREDENTIALS: &str =
    "DELETE FROM template_credentials WHERE template_id = ?1";
pub(crate) const INSERT_TEMPLATE_CREDENTIAL: &str =
    "INSERT INTO template_credentials (template_id, credential_id, position)
     VALUES (?1, ?2, ?3)";

pub(crate) fn insert_template(c: &Connection, item: &Template) -> AppResult<()> {
    c.execute(
        INSERT_TEMPLATE,
        params![
            item.id,
            item.team_id,
            item.name,
            item.description,
            item.tool,
            item.project_id,
            item.inventory_id,
            item.credential_id,
            item.environment_id,
            item.no_environment as i64,
            item.entrypoint,
            item.extra_vars,
            item.spec,
            item.created_at,
            item.update_on_launch as i64,
            item.admin_saved as i64,
            item.prompt_on_launch as i64
        ],
    )?;
    Ok(())
}

pub(crate) fn update_template(c: &Connection, item: &Template) -> AppResult<()> {
    let changed = c.execute(
        UPDATE_TEMPLATE,
        params![
            item.id,
            item.team_id,
            item.name,
            item.description,
            item.tool,
            item.project_id,
            item.inventory_id,
            item.credential_id,
            item.environment_id,
            item.no_environment as i64,
            item.entrypoint,
            item.extra_vars,
            item.spec,
            item.update_on_launch as i64,
            item.admin_saved as i64,
            item.prompt_on_launch as i64
        ],
    )?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn list_templates(c: &Connection, team_id: &str) -> AppResult<Vec<Template>> {
    sql::sqlite_all(c, LIST_TEMPLATES, [team_id], |row| row_template(row))
}

pub(crate) fn template(c: &Connection, id: &str) -> AppResult<Option<Template>> {
    sql::sqlite_opt(c, TEMPLATE_BY_ID, [id], |row| row_template(row))
}

pub(crate) fn delete_template(c: &Connection, id: &str, team_id: &str) -> AppResult<()> {
    let changed = c.execute(DELETE_TEMPLATE, params![id, team_id])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    c.execute(DELETE_TEMPLATE_SCHEDULES, [id])?;
    Ok(())
}

pub(crate) fn template_credential_ids(c: &Connection, template_id: &str) -> AppResult<Vec<String>> {
    sql::sqlite_all(c, TEMPLATE_CREDENTIAL_IDS, [template_id], |row| {
        SqlRow::get(row, 0)
    })
}

pub(crate) fn list_template_credential_links(
    c: &Connection,
    team_id: &str,
) -> AppResult<Vec<(String, String)>> {
    sql::sqlite_all(c, LIST_TEMPLATE_CREDENTIAL_LINKS, [team_id], |row| {
        Ok((
            SqlRow::get::<String>(row, 0)?,
            SqlRow::get::<String>(row, 1)?,
        ))
    })
}

pub(crate) fn save_template_credentials(
    c: &Connection,
    template_id: &str,
    ids: &[String],
) -> AppResult<()> {
    c.execute(DELETE_TEMPLATE_CREDENTIALS, [template_id])?;
    for (position, id) in ids.iter().enumerate() {
        c.execute(
            INSERT_TEMPLATE_CREDENTIAL,
            params![template_id, id, position as i64],
        )?;
    }
    Ok(())
}

pub(crate) fn row_template<R: SqlRow>(row: &R) -> AppResult<Template> {
    Ok(Template {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        tool: row.get(4)?,
        project_id: row.get(5)?,
        inventory_id: row.get(6)?,
        credential_id: row.get(7)?,
        environment_id: row.get(8)?,
        no_environment: row.get(9)?,
        entrypoint: row.get(10)?,
        extra_vars: row.get(11)?,
        spec: row.get(12)?,
        created_at: row.get(13)?,
        update_on_launch: row.get(14)?,
        admin_saved: row.get(15)?,
        prompt_on_launch: row.get(16)?,
    })
}

pub(crate) async fn postgres_write_template_credentials<C>(
    client: &C,
    template_id: &str,
    credential_ids: &[String],
) -> AppResult<()>
where
    C: deadpool_postgres::GenericClient,
{
    client
        .execute(&sql::postgres(DELETE_TEMPLATE_CREDENTIALS), &[&template_id])
        .await?;
    for (position, credential_id) in credential_ids.iter().enumerate() {
        client
            .execute(
                &sql::postgres(INSERT_TEMPLATE_CREDENTIAL),
                &[&template_id, &credential_id, &(position as i32)],
            )
            .await?;
    }
    Ok(())
}
