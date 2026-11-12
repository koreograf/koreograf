use rusqlite::{params, Connection};
use serde_json::Value;

use crate::db::{Inventory, InventorySource};
use crate::error::{ApiError, AppResult};
use crate::store::sql::{self, SqlRow};

pub(crate) const INSERT_INVENTORY: &str = "INSERT INTO inventories (
        id, team_id, name, body, created_at, source_kind, source_json, credential_id, admin_saved
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";
pub(crate) const UPDATE_INVENTORY: &str =
    "UPDATE inventories SET name = ?3, body = ?4 WHERE id = ?1 AND team_id = ?2";
pub(crate) const LIST_INVENTORIES: &str =
    "SELECT id, team_id, name, body, created_at, source_kind, source_json, credential_id, admin_saved
     FROM inventories WHERE team_id = ?1 ORDER BY name";
pub(crate) const INVENTORY_BY_ID: &str =
    "SELECT id, team_id, name, body, created_at, source_kind, source_json, credential_id, admin_saved
     FROM inventories WHERE id = ?1";
pub(crate) const SET_INVENTORY_ADMIN_SAVED: &str =
    "UPDATE inventories SET admin_saved = ?2 WHERE id = ?1";
pub(crate) const SET_INVENTORY_BODY: &str =
    "UPDATE inventories SET body = ?3 WHERE id = ?1 AND team_id = ?2";
pub(crate) const UPDATE_INVENTORY_SUMMARY: &str = "UPDATE inventories
     SET body = ?2, source_kind = ?3, source_json = ?4, credential_id = ?5
     WHERE id = ?1";

pub(crate) const INSERT_SOURCE: &str = "INSERT INTO inventory_sources (
        id, inventory_id, name, kind, source_json, credential_id, body, position, created_at
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";
pub(crate) const LIST_SOURCES: &str =
    "SELECT id, inventory_id, name, kind, source_json, credential_id, body, position, created_at
     FROM inventory_sources WHERE inventory_id = ?1 ORDER BY position, created_at";
pub(crate) const SOURCE_BY_ID: &str =
    "SELECT id, inventory_id, name, kind, source_json, credential_id, body, position, created_at
     FROM inventory_sources WHERE id = ?1";
pub(crate) const SET_SOURCE_BODY: &str = "UPDATE inventory_sources SET body = ?2 WHERE id = ?1";
pub(crate) const UPDATE_SOURCE: &str = "UPDATE inventory_sources
     SET name = ?2, source_json = ?3, credential_id = ?4, body = ?5
     WHERE id = ?1";
pub(crate) const DELETE_SOURCE: &str =
    "DELETE FROM inventory_sources WHERE id = ?1 AND inventory_id = ?2";

pub(crate) const DELETE_INVENTORY: &str = "DELETE FROM inventories WHERE id = ?1 AND team_id = ?2";
pub(crate) const SOURCE_CREDENTIALS: &str = "SELECT credential_id FROM inventory_sources
     WHERE inventory_id = ?1 AND credential_id IS NOT NULL";
pub(crate) const DELETE_AUTO_CREDENTIAL: &str = "DELETE FROM credentials
     WHERE id = ?1 AND team_id = ?2 AND auto = 1
       AND NOT EXISTS (SELECT 1 FROM inventory_sources s WHERE s.credential_id = ?1)
       AND NOT EXISTS (SELECT 1 FROM inventories i WHERE i.credential_id = ?1)
       AND NOT EXISTS (SELECT 1 FROM templates t WHERE t.credential_id = ?1)
       AND NOT EXISTS (SELECT 1 FROM template_credentials tc WHERE tc.credential_id = ?1)";
pub(crate) const JSON_REFERENCES_CREDENTIAL: &str = "SELECT graph FROM workflows WHERE team_id = ?1
     UNION ALL SELECT parameters FROM jobs WHERE team_id = ?1";
pub(crate) const TEMPLATES_USING_INVENTORY: &str = "SELECT name FROM templates
     WHERE inventory_id = ?1 AND team_id = ?2 ORDER BY name";
pub(crate) const WORKFLOW_GRAPHS: &str = "SELECT name, graph FROM workflows WHERE team_id = ?1";

pub(crate) fn insert_inventory(c: &Connection, item: &Inventory) -> AppResult<()> {
    c.execute(
        INSERT_INVENTORY,
        params![
            item.id,
            item.team_id,
            item.name,
            item.body,
            item.created_at,
            item.source_kind,
            item.source_json,
            item.credential_id,
            item.admin_saved as i64
        ],
    )?;
    Ok(())
}

pub(crate) fn update_inventory(
    c: &Connection,
    id: &str,
    team_id: &str,
    name: &str,
    body: &str,
) -> AppResult<()> {
    let changed = c.execute(UPDATE_INVENTORY, params![id, team_id, name, body])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn list_inventories(c: &Connection, team_id: &str) -> AppResult<Vec<Inventory>> {
    sql::sqlite_all(c, LIST_INVENTORIES, [team_id], |row| row_inventory(row))
}

pub(crate) fn inventory(c: &Connection, id: &str) -> AppResult<Option<Inventory>> {
    sql::sqlite_opt(c, INVENTORY_BY_ID, [id], |row| row_inventory(row))
}

pub(crate) fn set_inventory_admin_saved(
    c: &Connection,
    id: &str,
    admin_saved: bool,
) -> AppResult<()> {
    c.execute(SET_INVENTORY_ADMIN_SAVED, params![id, admin_saved as i64])?;
    Ok(())
}

pub(crate) fn set_inventory_body(
    c: &Connection,
    id: &str,
    team_id: &str,
    body: &str,
) -> AppResult<()> {
    let changed = c.execute(SET_INVENTORY_BODY, params![id, team_id, body])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn insert_inventory_source(c: &Connection, item: &InventorySource) -> AppResult<()> {
    c.execute(
        INSERT_SOURCE,
        params![
            item.id,
            item.inventory_id,
            item.name,
            item.kind,
            item.source_json,
            item.credential_id,
            item.body,
            item.position,
            item.created_at
        ],
    )?;
    Ok(())
}

pub(crate) fn list_inventory_sources(
    c: &Connection,
    inventory_id: &str,
) -> AppResult<Vec<InventorySource>> {
    sql::sqlite_all(c, LIST_SOURCES, [inventory_id], |row| {
        row_inventory_source(row)
    })
}

pub(crate) fn inventory_source(c: &Connection, id: &str) -> AppResult<Option<InventorySource>> {
    sql::sqlite_opt(c, SOURCE_BY_ID, [id], |row| row_inventory_source(row))
}

pub(crate) fn set_inventory_source_body(c: &Connection, id: &str, body: &str) -> AppResult<()> {
    c.execute(SET_SOURCE_BODY, params![id, body])?;
    Ok(())
}

pub(crate) fn update_inventory_source(
    c: &Connection,
    id: &str,
    name: &str,
    source_json: &str,
    credential_id: Option<&str>,
    body: &str,
) -> AppResult<()> {
    let changed = c.execute(
        UPDATE_SOURCE,
        params![id, name, source_json, credential_id, body],
    )?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn delete_inventory_source(
    c: &Connection,
    id: &str,
    inventory_id: &str,
) -> AppResult<()> {
    let changed = c.execute(DELETE_SOURCE, params![id, inventory_id])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn refresh_inventory_summary(c: &Connection, inventory_id: &str) -> AppResult<()> {
    let sources = list_inventory_sources(c, inventory_id)?;
    let (kind, source_json, credential_id, body) = summary(&sources);
    c.execute(
        UPDATE_INVENTORY_SUMMARY,
        params![inventory_id, body, kind, source_json, credential_id],
    )?;
    Ok(())
}

pub(crate) fn summary(sources: &[InventorySource]) -> (String, String, Option<String>, String) {
    match sources {
        [] => ("ini".to_string(), "{}".to_string(), None, String::new()),
        [one] => (
            one.kind.clone(),
            one.source_json.clone(),
            one.credential_id.clone(),
            one.body.clone(),
        ),
        many => {
            let body = many
                .iter()
                .map(|source| source.body.trim_end_matches(['\n', '\r']))
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            ("mixed".to_string(), "{}".to_string(), None, body)
        }
    }
}

fn json_references_credential(c: &Connection, team_id: &str, id: &str) -> AppResult<bool> {
    let rows = sql::sqlite_all(c, JSON_REFERENCES_CREDENTIAL, [team_id], |row| {
        SqlRow::get::<String>(row, 0)
    })?;
    for raw in rows {
        match serde_json::from_str::<Value>(&raw) {
            Ok(value) if json_contains_string(&value, id) => return Ok(true),
            Ok(_) => {}
            Err(_) => return Ok(true),
        }
    }
    Ok(false)
}

fn json_contains_string(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text == needle,
        Value::Array(items) => items.iter().any(|item| json_contains_string(item, needle)),
        Value::Object(map) => map.values().any(|item| json_contains_string(item, needle)),
        _ => false,
    }
}

pub(crate) fn templates_using_inventory(
    c: &Connection,
    inventory_id: &str,
    team_id: &str,
) -> AppResult<Vec<String>> {
    sql::sqlite_all(
        c,
        TEMPLATES_USING_INVENTORY,
        params![inventory_id, team_id],
        |row| SqlRow::get(row, 0),
    )
}

pub(crate) fn workflows_using_inventory(
    c: &Connection,
    inventory_id: &str,
    team_id: &str,
) -> AppResult<Vec<String>> {
    let rows = sql::sqlite_all(c, WORKFLOW_GRAPHS, [team_id], |row| {
        Ok((
            SqlRow::get::<String>(row, 0)?,
            SqlRow::get::<String>(row, 1)?,
        ))
    })?;
    let mut names = Vec::new();
    for (name, graph) in rows {
        if workflow_references_inventory(&graph, inventory_id) {
            names.push(name);
        }
    }
    Ok(names)
}

pub(crate) fn workflow_references_inventory(graph: &str, inventory_id: &str) -> bool {
    match serde_json::from_str::<crate::workflow::Graph>(graph) {
        Ok(graph) => graph
            .nodes
            .iter()
            .any(|node| node.overrides.inventory_id.as_deref() == Some(inventory_id)),
        Err(_) => true,
    }
}

pub(crate) fn delete_inventory(c: &mut Connection, id: &str, team_id: &str) -> AppResult<()> {
    let tx = c.transaction()?;
    let credential_ids = sql::sqlite_all(&tx, SOURCE_CREDENTIALS, [id], |row| {
        SqlRow::get::<String>(row, 0)
    })?;
    let changed = tx.execute(DELETE_INVENTORY, params![id, team_id])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    for credential_id in credential_ids {
        if json_references_credential(&tx, team_id, &credential_id)? {
            continue;
        }
        tx.execute(DELETE_AUTO_CREDENTIAL, params![credential_id, team_id])?;
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn row_inventory<R: SqlRow>(row: &R) -> AppResult<Inventory> {
    Ok(Inventory {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        body: row.get(3)?,
        created_at: row.get(4)?,
        source_kind: row.get(5)?,
        source_json: row.get(6)?,
        credential_id: row.get(7)?,
        admin_saved: row.get(8)?,
    })
}

pub(crate) fn row_inventory_source<R: SqlRow>(row: &R) -> AppResult<InventorySource> {
    Ok(InventorySource {
        id: row.get(0)?,
        inventory_id: row.get(1)?,
        name: row.get(2)?,
        kind: row.get(3)?,
        source_json: row.get(4)?,
        credential_id: row.get(5)?,
        body: row.get(6)?,
        position: i64::from(row.get::<i32>(7)?),
        created_at: row.get(8)?,
    })
}

pub(crate) async fn postgres_insert_source<C>(client: &C, source: &InventorySource) -> AppResult<()>
where
    C: deadpool_postgres::GenericClient,
{
    client
        .execute(
            &sql::postgres(INSERT_SOURCE),
            &[
                &source.id,
                &source.inventory_id,
                &source.name,
                &source.kind,
                &source.source_json,
                &source.credential_id,
                &source.body,
                &(source.position as i32),
                &source.created_at,
            ],
        )
        .await?;
    Ok(())
}

pub(crate) async fn postgres_refresh_summary<C>(client: &C, inventory_id: &str) -> AppResult<()>
where
    C: deadpool_postgres::GenericClient,
{
    let rows = client
        .query(&sql::postgres(LIST_SOURCES), &[&inventory_id])
        .await?;
    let sources = rows
        .iter()
        .map(row_inventory_source)
        .collect::<AppResult<Vec<_>>>()?;
    let (kind, source_json, credential_id, body) = summary(&sources);
    client
        .execute(
            &sql::postgres(UPDATE_INVENTORY_SUMMARY),
            &[&inventory_id, &body, &kind, &source_json, &credential_id],
        )
        .await?;
    Ok(())
}

pub(crate) async fn postgres_json_references_credential<C>(
    client: &C,
    team_id: &str,
    credential_id: &str,
) -> AppResult<bool>
where
    C: deadpool_postgres::GenericClient,
{
    let rows = client
        .query(&sql::postgres(JSON_REFERENCES_CREDENTIAL), &[&team_id])
        .await?;
    for row in rows {
        let raw: String = row.get(0);
        match serde_json::from_str::<Value>(&raw) {
            Ok(value) if json_contains_string(&value, credential_id) => return Ok(true),
            Ok(_) => {}
            Err(_) => return Ok(true),
        }
    }
    Ok(false)
}
