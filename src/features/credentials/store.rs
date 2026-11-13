use rusqlite::{params, Connection};

use crate::db::{Credential, CredentialUsage};
use crate::error::{ApiError, AppResult};
use crate::store::sql::{self, SqlRow};

pub(crate) const INSERT_CREDENTIAL: &str =
    "INSERT INTO credentials (id, team_id, name, kind, secret_blob, created_at, auto, disabled)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)";
pub(crate) const LIST_CREDENTIALS: &str =
    "SELECT id, team_id, name, kind, '' AS secret_blob, created_at, auto, disabled
     FROM credentials WHERE team_id = ?1 AND disabled = 0 ORDER BY name";
pub(crate) const NETBOX_CREDENTIALS: &str =
    "SELECT id, team_id, name, kind, secret_blob, created_at, auto, disabled
     FROM credentials WHERE kind = 'netbox'";
pub(crate) const CREDENTIAL_BY_ID: &str =
    "SELECT id, team_id, name, kind, secret_blob, created_at, auto, disabled
     FROM credentials WHERE id = ?1";
pub(crate) const UPDATE_CREDENTIAL: &str =
    "UPDATE credentials SET name = ?3, secret_blob = ?4, disabled = ?5
     WHERE id = ?1 AND team_id = ?2";
pub(crate) const DELETE_CREDENTIAL: &str = "DELETE FROM credentials WHERE id = ?1 AND team_id = ?2";
pub(crate) const CREDENTIAL_USAGES: &str = "SELECT i.id, i.name, s.id, s.name, NULL, NULL
     FROM inventory_sources s
     JOIN inventories i ON i.id = s.inventory_id
     WHERE s.credential_id = ?1 AND i.team_id = ?2
     UNION ALL
     SELECT NULL, NULL, NULL, NULL, p.id, p.name
     FROM projects p
     WHERE p.credential_id = ?1
       AND (
         p.team_id = ?2
         OR EXISTS (
           SELECT 1 FROM project_teams pt WHERE pt.project_id = p.id AND pt.team_id = ?2
         )
       )
     ORDER BY 2, 4, 6";
pub(crate) const NETBOX_CREDENTIAL_ENDPOINTS: &str =
    "SELECT credential_id, source_json FROM inventory_sources
     WHERE credential_id IS NOT NULL AND kind = 'netbox'
     ORDER BY created_at";

pub(crate) fn insert_credential(c: &Connection, item: &Credential) -> AppResult<()> {
    c.execute(
        INSERT_CREDENTIAL,
        params![
            item.id,
            item.team_id,
            item.name,
            item.kind,
            item.secret_blob,
            item.created_at,
            item.auto,
            item.disabled
        ],
    )?;
    Ok(())
}

pub(crate) fn list_credentials(c: &Connection, team_id: &str) -> AppResult<Vec<Credential>> {
    sql::sqlite_all(c, LIST_CREDENTIALS, [team_id], |row| row_credential(row))
}

pub(crate) fn netbox_credentials(c: &Connection) -> AppResult<Vec<Credential>> {
    sql::sqlite_all(c, NETBOX_CREDENTIALS, [], |row| row_credential(row))
}

pub(crate) fn credential(c: &Connection, id: &str) -> AppResult<Option<Credential>> {
    sql::sqlite_opt(c, CREDENTIAL_BY_ID, [id], |row| row_credential(row))
}

pub(crate) fn update_credential(c: &Connection, item: &Credential) -> AppResult<()> {
    let changed = c.execute(
        UPDATE_CREDENTIAL,
        params![
            item.id,
            item.team_id,
            item.name,
            item.secret_blob,
            item.disabled
        ],
    )?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn delete_credential(c: &Connection, id: &str, team_id: &str) -> AppResult<()> {
    let changed = c.execute(DELETE_CREDENTIAL, params![id, team_id])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn credential_usages(
    c: &Connection,
    credential_id: &str,
    team_id: &str,
) -> AppResult<Vec<CredentialUsage>> {
    sql::sqlite_all(
        c,
        CREDENTIAL_USAGES,
        params![credential_id, team_id],
        |row| {
            Ok(CredentialUsage {
                inventory_id: row.get(0)?,
                inventory_name: row.get(1)?,
                source_id: row.get(2)?,
                source_name: row.get(3)?,
                project_id: row.get(4)?,
                project_name: row.get(5)?,
            })
        },
    )
}

pub(crate) fn netbox_credential_endpoints(c: &Connection) -> AppResult<Vec<(String, String)>> {
    sql::sqlite_all(c, NETBOX_CREDENTIAL_ENDPOINTS, [], |row| {
        Ok((
            SqlRow::get::<String>(row, 0)?,
            SqlRow::get::<String>(row, 1)?,
        ))
    })
}

pub(crate) fn row_credential<R: SqlRow>(row: &R) -> AppResult<Credential> {
    Ok(Credential {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        kind: row.get(3)?,
        secret_blob: row.get(4)?,
        created_at: row.get(5)?,
        auto: row.get(6)?,
        disabled: row.get(7)?,
    })
}

pub(crate) fn row_credential_usage<R: SqlRow>(row: &R) -> AppResult<CredentialUsage> {
    Ok(CredentialUsage {
        inventory_id: row.get(0)?,
        inventory_name: row.get(1)?,
        source_id: row.get(2)?,
        source_name: row.get(3)?,
        project_id: row.get(4)?,
        project_name: row.get(5)?,
    })
}
