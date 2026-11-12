use rusqlite::{params, Connection};

use crate::db::{self, ApiToken, User};
use crate::error::AppResult;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_lookup_uses_an_explicit_dialect_predicate() {
        let sqlite = user_by_username_sql(Dialect::Sqlite);
        let postgres = user_by_username_sql(Dialect::Postgres);
        assert!(sqlite.contains("COLLATE NOCASE"), "{sqlite}");
        assert!(!sqlite.contains("lower(username)"), "{sqlite}");
        assert!(
            postgres.contains("lower(username) = lower(?1)"),
            "{postgres}"
        );
        assert!(!postgres.contains("COLLATE NOCASE"), "{postgres}");
    }

    #[test]
    fn list_users_orders_with_an_explicit_dialect_expression() {
        assert!(list_users_sql(Dialect::Sqlite).contains("ORDER BY username COLLATE NOCASE"));
        assert!(list_users_sql(Dialect::Postgres).contains("ORDER BY lower(username)"));
    }
}
