use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::error::AppResult;
use crate::store::sql;

pub(crate) const SETTINGS_ROWS: &str = "SELECT key, value FROM settings";
pub(crate) const SETTING: &str = "SELECT value FROM settings WHERE key = ?1";
pub(crate) const DELETE_SETTINGS: &str = "DELETE FROM settings";
pub(crate) const INSERT_SETTING: &str = "INSERT INTO settings (key, value) VALUES (?1, ?2)
     ON CONFLICT (key) DO UPDATE SET value = excluded.value";
pub(crate) const SEED_STATE: &str = "SELECT value FROM seed_state WHERE key = ?1";
pub(crate) const SET_SEED_STATE: &str = "INSERT INTO seed_state (key, value) VALUES (?1, ?2)
     ON CONFLICT (key) DO UPDATE SET value = excluded.value";

pub(crate) fn read_rows(c: &Connection) -> AppResult<HashMap<String, String>> {
    let rows = sql::sqlite_all(c, SETTINGS_ROWS, [], |row| {
        Ok((
            sql::SqlRow::get::<String>(row, 0)?,
            sql::SqlRow::get::<String>(row, 1)?,
        ))
    })?;
    Ok(rows.into_iter().collect())
}

pub(crate) fn setting(c: &Connection, key: &str) -> AppResult<Option<String>> {
    sql::sqlite_opt(c, SETTING, [key], |row| sql::SqlRow::get(row, 0))
}

pub(crate) fn set_setting(c: &Connection, key: &str, value: &str) -> AppResult<()> {
    c.execute(INSERT_SETTING, params![key, value])?;
    Ok(())
}

pub(crate) fn seed_state(c: &Connection, key: &str) -> AppResult<Option<String>> {
    sql::sqlite_opt(c, SEED_STATE, [key], |row| sql::SqlRow::get(row, 0))
}

pub(crate) fn set_seed_state(c: &Connection, key: &str, value: &str) -> AppResult<()> {
    c.execute(SET_SEED_STATE, params![key, value])?;
    Ok(())
}

pub(crate) fn replace(c: &mut Connection, rows: &[(String, String)]) -> AppResult<()> {
    let tx = c.transaction()?;
    tx.execute(DELETE_SETTINGS, [])?;
    for (key, value) in rows {
        tx.execute(INSERT_SETTING, params![key, value])?;
    }
    tx.commit()?;
    Ok(())
}
