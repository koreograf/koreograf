use rusqlite::{params, Connection};

use crate::db::{self, Member, Organization, Team, UserMembership};
use crate::error::{ApiError, AppResult};
use crate::store::sql::{self, SqlRow};

pub(crate) const TEAM_COLUMNS: &str = "t.id, t.name, t.description, t.ldap_group, t.created_at, \
     COALESCE(t.organization_id, ''), COALESCE(o.name, '')";

pub(crate) const MEMBERSHIPS_FOR_USER: &str =
    "SELECT t.id, t.name, COALESCE(t.organization_id, ''), COALESCE(o.name, ''), m.role, m.source
     FROM memberships m
     JOIN teams t ON t.id = m.team_id
     LEFT JOIN organizations o ON o.id = t.organization_id
     WHERE m.user_id = ?1
     ORDER BY o.name, t.name";
pub(crate) const MEMBERSHIP_SOURCE: &str =
    "SELECT source FROM memberships WHERE team_id = ?1 AND user_id = ?2";
pub(crate) const INSERT_TEAM: &str =
    "INSERT INTO teams (id, organization_id, name, description, ldap_group, created_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
pub(crate) const UPDATE_TEAM: &str =
    "UPDATE teams SET name = ?2, description = ?3, ldap_group = ?4 WHERE id = ?1";
pub(crate) const SET_TEAM_ORGANIZATION: &str =
    "UPDATE teams SET organization_id = ?2 WHERE id = ?1";
pub(crate) const TEAM_BY_ID: &str =
    "SELECT t.id, t.name, t.description, t.ldap_group, t.created_at,
     COALESCE(t.organization_id, ''), COALESCE(o.name, '')
 FROM teams t LEFT JOIN organizations o ON o.id = t.organization_id WHERE t.id = ?1";
pub(crate) const TEAMS_IN_ORGANIZATION: &str = "SELECT t.id, t.name, t.description, t.ldap_group,
     t.created_at, COALESCE(t.organization_id, ''), COALESCE(o.name, '')
 FROM teams t LEFT JOIN organizations o ON o.id = t.organization_id
 WHERE t.organization_id = ?1 ORDER BY t.name";
pub(crate) const TEAMS_WITH_LDAP: &str = "SELECT t.id, t.name, t.description, t.ldap_group,
     t.created_at, COALESCE(t.organization_id, ''), COALESCE(o.name, '')
 FROM teams t LEFT JOIN organizations o ON o.id = t.organization_id
 WHERE t.ldap_group IS NOT NULL AND t.ldap_group != ''";
pub(crate) const COUNT_TEAMS: &str = "SELECT COUNT(*) FROM teams";
pub(crate) const UPSERT_MEMBERSHIP: &str =
    "INSERT INTO memberships (team_id, user_id, role, source) VALUES (?1, ?2, ?3, ?4)
     ON CONFLICT (team_id, user_id) DO UPDATE
     SET role = excluded.role, source = excluded.source
     WHERE memberships.source = 'ldap' OR excluded.source = 'manual'";
pub(crate) const DELETE_MEMBERSHIP: &str =
    "DELETE FROM memberships WHERE team_id = ?1 AND user_id = ?2";
pub(crate) const DELETE_LDAP_MEMBERSHIP: &str =
    "DELETE FROM memberships WHERE team_id = ?1 AND user_id = ?2 AND source = 'ldap'";
pub(crate) const ROLE_OF: &str = "SELECT role FROM memberships WHERE team_id = ?1 AND user_id = ?2";
pub(crate) const LIST_MEMBERS: &str = "SELECT u.id, u.username, u.display_name, m.role, m.source
     FROM memberships m JOIN users u ON u.id = m.user_id
     WHERE m.team_id = ?1 ORDER BY u.username";

pub(crate) const INSERT_ORGANIZATION: &str =
    "INSERT INTO organizations (id, name, description, created_at) VALUES (?1, ?2, ?3, ?4)";
pub(crate) const UPDATE_ORGANIZATION: &str =
    "UPDATE organizations SET name = ?2, description = ?3 WHERE id = ?1";
pub(crate) const ORGANIZATION_BY_ID: &str =
    "SELECT id, name, description, created_at FROM organizations WHERE id = ?1";
pub(crate) const ORGANIZATION_BY_NAME: &str =
    "SELECT id, name, description, created_at FROM organizations WHERE name = ?1";
pub(crate) const ORGANIZATIONS_ALL: &str =
    "SELECT id, name, description, created_at FROM organizations ORDER BY name";
pub(crate) const ORGANIZATIONS_FOR_USER: &str =
    "SELECT DISTINCT o.id, o.name, o.description, o.created_at
     FROM organizations o
     JOIN teams t ON t.organization_id = o.id
     JOIN memberships m ON m.team_id = t.id
     WHERE m.user_id = ?1
     ORDER BY o.name";

pub(crate) fn teams_for_user_sql(admin: bool) -> String {
    if admin {
        format!(
            "SELECT {TEAM_COLUMNS},
                    COALESCE((SELECT role FROM memberships m WHERE m.team_id = t.id AND m.user_id = ?1), 'owner')
             FROM teams t LEFT JOIN organizations o ON o.id = t.organization_id ORDER BY t.name"
        )
    } else {
        format!(
            "SELECT {TEAM_COLUMNS}, m.role
             FROM teams t JOIN memberships m ON m.team_id = t.id
             LEFT JOIN organizations o ON o.id = t.organization_id
             WHERE m.user_id = ?1 ORDER BY t.name"
        )
    }
}

pub(crate) fn list_user_memberships(
    c: &Connection,
    user_id: &str,
) -> AppResult<Vec<UserMembership>> {
    sql::sqlite_all(c, MEMBERSHIPS_FOR_USER, [user_id], |row| {
        row_user_membership(row)
    })
}

pub(crate) fn membership_source(
    c: &Connection,
    team_id: &str,
    user_id: &str,
) -> AppResult<Option<String>> {
    sql::sqlite_opt(c, MEMBERSHIP_SOURCE, params![team_id, user_id], |row| {
        SqlRow::get(row, 0)
    })
}

pub(crate) fn grant_manual_membership(
    c: &Connection,
    team_id: &str,
    user_id: &str,
    role: &str,
) -> AppResult<()> {
    if membership_source(c, team_id, user_id)?.as_deref() == Some("ldap") {
        return Err(ApiError::bad(
            "the_membership_comes_from_ldap_or_active_directory",
        ));
    }
    upsert_membership(c, team_id, user_id, role, "manual")
}

pub(crate) fn insert_team(
    c: &Connection,
    id: &str,
    organization_id: &str,
    name: &str,
    description: &str,
    ldap_group: Option<&str>,
) -> AppResult<()> {
    c.execute(
        INSERT_TEAM,
        params![
            id,
            organization_id,
            name,
            description,
            ldap_group,
            db::now()
        ],
    )?;
    Ok(())
}

pub(crate) fn update_team(
    c: &Connection,
    id: &str,
    name: &str,
    description: &str,
    ldap_group: Option<&str>,
) -> AppResult<()> {
    let changed = c.execute(UPDATE_TEAM, params![id, name, description, ldap_group])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn set_team_organization(
    c: &Connection,
    team_id: &str,
    organization_id: &str,
) -> AppResult<()> {
    let changed = c.execute(SET_TEAM_ORGANIZATION, params![team_id, organization_id])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn insert_organization(
    c: &Connection,
    id: &str,
    name: &str,
    description: &str,
) -> AppResult<()> {
    c.execute(
        INSERT_ORGANIZATION,
        params![id, name, description, db::now()],
    )?;
    Ok(())
}

pub(crate) fn update_organization(
    c: &Connection,
    id: &str,
    name: &str,
    description: &str,
) -> AppResult<()> {
    let changed = c.execute(UPDATE_ORGANIZATION, params![id, name, description])?;
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(crate) fn organization(c: &Connection, id: &str) -> AppResult<Option<Organization>> {
    sql::sqlite_opt(c, ORGANIZATION_BY_ID, [id], |row| row_organization(row))
}

pub(crate) fn organization_by_name(c: &Connection, name: &str) -> AppResult<Option<Organization>> {
    sql::sqlite_opt(c, ORGANIZATION_BY_NAME, [name], |row| row_organization(row))
}

pub(crate) fn list_organizations(
    c: &Connection,
    user_id: &str,
    admin: bool,
) -> AppResult<Vec<Organization>> {
    if admin {
        sql::sqlite_all(c, ORGANIZATIONS_ALL, [], |row| row_organization(row))
    } else {
        sql::sqlite_all(c, ORGANIZATIONS_FOR_USER, [user_id], |row| {
            row_organization(row)
        })
    }
}

pub(crate) fn teams_in_organization(c: &Connection, organization_id: &str) -> AppResult<Vec<Team>> {
    sql::sqlite_all(c, TEAMS_IN_ORGANIZATION, [organization_id], |row| {
        row_team(row)
    })
}

pub(crate) fn team_by_id(c: &Connection, id: &str) -> AppResult<Option<Team>> {
    sql::sqlite_opt(c, TEAM_BY_ID, [id], |row| row_team(row))
}

pub(crate) fn count_teams(c: &Connection) -> AppResult<i64> {
    sql::sqlite_opt(c, COUNT_TEAMS, [], |row| SqlRow::get(row, 0)).map(|count| count.unwrap_or(0))
}

pub(crate) fn list_teams_for_user(
    c: &Connection,
    user_id: &str,
    admin: bool,
) -> AppResult<Vec<(Team, String)>> {
    sql::sqlite_all(c, &teams_for_user_sql(admin), [user_id], |row| {
        Ok((row_team(row)?, SqlRow::get(row, 7)?))
    })
}

pub(crate) fn teams_with_ldap(c: &Connection) -> AppResult<Vec<Team>> {
    sql::sqlite_all(c, TEAMS_WITH_LDAP, [], |row| row_team(row))
}

pub(crate) fn upsert_membership(
    c: &Connection,
    team_id: &str,
    user_id: &str,
    role: &str,
    source: &str,
) -> AppResult<()> {
    c.execute(UPSERT_MEMBERSHIP, params![team_id, user_id, role, source])?;
    Ok(())
}

pub(crate) fn delete_ldap_membership(
    c: &Connection,
    team_id: &str,
    user_id: &str,
) -> AppResult<()> {
    c.execute(DELETE_LDAP_MEMBERSHIP, params![team_id, user_id])?;
    Ok(())
}

pub(crate) fn delete_membership(c: &Connection, team_id: &str, user_id: &str) -> AppResult<()> {
    c.execute(DELETE_MEMBERSHIP, params![team_id, user_id])?;
    Ok(())
}

pub(crate) fn role_of(c: &Connection, team_id: &str, user_id: &str) -> AppResult<Option<String>> {
    sql::sqlite_opt(c, ROLE_OF, params![team_id, user_id], |row| {
        SqlRow::get(row, 0)
    })
}

pub(crate) fn list_members(c: &Connection, team_id: &str) -> AppResult<Vec<Member>> {
    sql::sqlite_all(c, LIST_MEMBERS, [team_id], |row| row_member(row))
}

pub(crate) fn row_team<R: SqlRow>(row: &R) -> AppResult<Team> {
    Ok(Team {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        ldap_group: row.get(3)?,
        created_at: row.get(4)?,
        organization_id: row.get(5)?,
        organization_name: row.get(6)?,
    })
}

pub(crate) fn row_organization<R: SqlRow>(row: &R) -> AppResult<Organization> {
    Ok(Organization {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        created_at: row.get(3)?,
    })
}

pub(crate) fn row_member<R: SqlRow>(row: &R) -> AppResult<Member> {
    Ok(Member {
        user_id: row.get(0)?,
        username: row.get(1)?,
        display_name: row.get(2)?,
        role: row.get(3)?,
        source: row.get(4)?,
    })
}

pub(crate) fn row_user_membership<R: SqlRow>(row: &R) -> AppResult<UserMembership> {
    Ok(UserMembership {
        team_id: row.get(0)?,
        team_name: row.get(1)?,
        organization_id: row.get(2)?,
        organization_name: row.get(3)?,
        role: row.get(4)?,
        source: row.get(5)?,
    })
}
