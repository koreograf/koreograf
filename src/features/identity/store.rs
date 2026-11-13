use rusqlite::{params, Connection};

use crate::db::{self, ApiToken, Member, Organization, Team, User, UserMembership};
use crate::error::{ApiError, AppResult};
use crate::store::sql::{self, Dialect, SqlRow};

pub(crate) const USER_COLUMNS: &str =
    "id, username, display_name, password_hash, auth_source, is_admin, disabled, created_at, email";

pub(crate) const INSERT_USER: &str =
    "INSERT INTO users (id, username, display_name, password_hash, auth_source, is_admin, created_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

pub(crate) const USER_BY_ID: &str =
    "SELECT id, username, display_name, password_hash, auth_source, is_admin,
        disabled, created_at, email
 FROM users WHERE id = ?1";

pub(crate) const UPDATE_PROFILE_PASSWORD: &str =
    "UPDATE users SET display_name = ?2, email = ?3, password_hash = ?4 WHERE id = ?1";
pub(crate) const UPDATE_PROFILE: &str =
    "UPDATE users SET display_name = ?2, email = ?3 WHERE id = ?1";
pub(crate) const SET_USER_EMAIL: &str = "UPDATE users SET email = ?2 WHERE id = ?1";
pub(crate) const UPDATE_USER_PASSWORD: &str = "UPDATE users SET display_name = ?2, is_admin = ?3,
        disabled = ?4, password_hash = ?5
 WHERE id = ?1";
pub(crate) const UPDATE_USER: &str =
    "UPDATE users SET display_name = ?2, is_admin = ?3, disabled = ?4 WHERE id = ?1";
pub(crate) const DELETE_USER_SESSIONS: &str = "DELETE FROM sessions WHERE user_id = ?1";
pub(crate) const DELETE_USER: &str = "DELETE FROM users WHERE id = ?1";
pub(crate) const OTHER_ACTIVE_ADMINS: &str =
    "SELECT COUNT(*) FROM users WHERE is_admin != 0 AND disabled = 0 AND id != ?1";

pub(crate) const CREATE_SESSION: &str =
    "INSERT INTO sessions (token, user_id, expires_at) VALUES (?1, ?2, ?3)";
pub(crate) const DELETE_SESSION: &str = "DELETE FROM sessions WHERE token = ?1";
pub(crate) const PURGE_SESSIONS: &str = "DELETE FROM sessions WHERE expires_at < ?1";
pub(crate) const USER_BY_SESSION: &str = "SELECT u.id, u.username, u.display_name, u.password_hash,
        u.auth_source, u.is_admin, u.disabled, u.created_at, u.email, s.expires_at
 FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token = ?1";

pub(crate) const CREATE_API_TOKEN: &str =
    "INSERT INTO api_tokens (id, user_id, name, prefix, token_hash, created_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
pub(crate) const LIST_API_TOKENS: &str = "SELECT id, name, prefix, created_at, last_used_at
 FROM api_tokens WHERE user_id = ?1 ORDER BY created_at, name";
pub(crate) const DELETE_API_TOKEN: &str = "DELETE FROM api_tokens WHERE id = ?1 AND user_id = ?2";
pub(crate) const USER_BY_API_TOKEN: &str =
    "SELECT u.id, u.username, u.display_name, u.password_hash,
        u.auth_source, u.is_admin, u.disabled, u.created_at, u.email, t.id
 FROM api_tokens t JOIN users u ON u.id = t.user_id WHERE t.token_hash = ?1";
pub(crate) const TOUCH_API_TOKEN: &str = "UPDATE api_tokens SET last_used_at = ?2 WHERE id = ?1";

pub(crate) fn user_by_username_sql(dialect: Dialect) -> String {
    let predicate = match dialect {
        Dialect::Sqlite => "username = ?1 COLLATE NOCASE",
        Dialect::Postgres => "lower(username) = lower(?1)",
    };
    format!("SELECT {USER_COLUMNS} FROM users WHERE {predicate}")
}

pub(crate) fn list_users_sql(dialect: Dialect) -> String {
    let order = match dialect {
        Dialect::Sqlite => "username COLLATE NOCASE",
        Dialect::Postgres => "lower(username)",
    };
    format!(
        "SELECT {USER_COLUMNS},
                (SELECT COUNT(*) FROM memberships m WHERE m.user_id = users.id)
         FROM users ORDER BY {order}"
    )
}

pub(crate) fn insert_user(
    c: &Connection,
    id: &str,
    username: &str,
    display_name: &str,
    password_hash: Option<&str>,
    auth_source: &str,
    is_admin: bool,
) -> AppResult<()> {
    c.execute(
        INSERT_USER,
        params![
            id,
            username,
            display_name,
            password_hash,
            auth_source,
            is_admin as i64,
            db::now()
        ],
    )?;
    Ok(())
}

pub(crate) fn user_by_username(c: &Connection, username: &str) -> AppResult<Option<User>> {
    sql::sqlite_opt(
        c,
        &user_by_username_sql(Dialect::Sqlite),
        [username],
        |row| row_user(row),
    )
}

pub(crate) fn user_by_id(c: &Connection, id: &str) -> AppResult<Option<User>> {
    sql::sqlite_opt(c, USER_BY_ID, [id], |row| row_user(row))
}

pub(crate) fn list_users(c: &Connection) -> AppResult<Vec<(User, i64)>> {
    sql::sqlite_all(c, &list_users_sql(Dialect::Sqlite), [], |row| {
        Ok((row_user(row)?, row.get(9)?))
    })
}

pub(crate) fn update_profile(
    c: &Connection,
    id: &str,
    display_name: &str,
    email: Option<&str>,
    password_hash: Option<&str>,
) -> AppResult<()> {
    if let Some(password_hash) = password_hash {
        c.execute(
            UPDATE_PROFILE_PASSWORD,
            params![id, display_name, email, password_hash],
        )?;
    } else {
        c.execute(UPDATE_PROFILE, params![id, display_name, email])?;
    }
    Ok(())
}

pub(crate) fn set_user_email(c: &Connection, id: &str, email: Option<&str>) -> AppResult<()> {
    c.execute(SET_USER_EMAIL, params![id, email])?;
    Ok(())
}

pub(crate) fn update_user(
    c: &Connection,
    id: &str,
    display_name: &str,
    is_admin: bool,
    disabled: bool,
    password_hash: Option<&str>,
) -> AppResult<()> {
    if let Some(password_hash) = password_hash {
        c.execute(
            UPDATE_USER_PASSWORD,
            params![
                id,
                display_name,
                is_admin as i64,
                disabled as i64,
                password_hash
            ],
        )?;
    } else {
        c.execute(
            UPDATE_USER,
            params![id, display_name, is_admin as i64, disabled as i64],
        )?;
    }
    Ok(())
}

pub(crate) fn delete_user_sessions(c: &Connection, user_id: &str) -> AppResult<()> {
    c.execute(DELETE_USER_SESSIONS, [user_id])?;
    Ok(())
}

pub(crate) fn delete_user(c: &Connection, id: &str) -> AppResult<bool> {
    Ok(c.execute(DELETE_USER, [id])? > 0)
}

pub(crate) fn other_active_admins(c: &Connection, user_id: &str) -> AppResult<i64> {
    sql::sqlite_opt(c, OTHER_ACTIVE_ADMINS, [user_id], |row| SqlRow::get(row, 0))
        .map(|count| count.unwrap_or(0))
}

pub(crate) fn create_session(c: &Connection, token: &str, user_id: &str) -> AppResult<()> {
    c.execute(
        CREATE_SESSION,
        params![token, user_id, db::plus_secs(7 * 24 * 3600)],
    )?;
    Ok(())
}

pub(crate) fn delete_session(c: &Connection, token: &str) -> AppResult<()> {
    c.execute(DELETE_SESSION, [token])?;
    Ok(())
}

pub(crate) fn purge_sessions(c: &Connection) -> AppResult<()> {
    c.execute(PURGE_SESSIONS, [db::now()])?;
    Ok(())
}

pub(crate) fn user_by_session(c: &Connection, token: &str) -> AppResult<Option<User>> {
    let found = sql::sqlite_opt(c, USER_BY_SESSION, [token], |row| {
        Ok((row_user(row)?, SqlRow::get::<String>(row, 9)?))
    })?;
    let Some((user, expires)) = found else {
        return Ok(None);
    };
    if expires < db::now() {
        delete_session(c, token)?;
        return Ok(None);
    }
    Ok(Some(user))
}

pub(crate) fn create_api_token(
    c: &Connection,
    id: &str,
    user_id: &str,
    name: &str,
    prefix: &str,
    token_hash: &str,
    created_at: &str,
) -> AppResult<()> {
    c.execute(
        CREATE_API_TOKEN,
        params![id, user_id, name, prefix, token_hash, created_at],
    )?;
    Ok(())
}

pub(crate) fn list_api_tokens(c: &Connection, user_id: &str) -> AppResult<Vec<ApiToken>> {
    sql::sqlite_all(c, LIST_API_TOKENS, [user_id], |row| row_api_token(row))
}

pub(crate) fn delete_api_token(c: &Connection, id: &str, user_id: &str) -> AppResult<bool> {
    Ok(c.execute(DELETE_API_TOKEN, params![id, user_id])? > 0)
}

pub(crate) fn user_by_api_token(c: &Connection, token_hash: &str) -> AppResult<Option<User>> {
    let found = sql::sqlite_opt(c, USER_BY_API_TOKEN, [token_hash], |row| {
        Ok((row_user(row)?, SqlRow::get::<String>(row, 9)?))
    })?;
    let Some((user, token_id)) = found else {
        return Ok(None);
    };
    c.execute(TOUCH_API_TOKEN, params![token_id, db::now()])?;
    Ok(Some(user))
}

pub(crate) fn row_user<R: SqlRow>(row: &R) -> AppResult<User> {
    Ok(User {
        id: row.get(0)?,
        username: row.get(1)?,
        display_name: row.get(2)?,
        password_hash: row.get(3)?,
        auth_source: row.get(4)?,
        is_admin: row.get(5)?,
        disabled: row.get(6)?,
        created_at: row.get(7)?,
        email: row.get(8)?,
    })
}

pub(crate) fn row_api_token<R: SqlRow>(row: &R) -> AppResult<ApiToken> {
    Ok(ApiToken {
        id: row.get(0)?,
        name: row.get(1)?,
        prefix: row.get(2)?,
        created_at: row.get(3)?,
        last_used_at: row.get(4)?,
    })
}

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
