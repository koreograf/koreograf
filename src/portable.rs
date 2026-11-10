use std::path::Path;

use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags};
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, Row};

use crate::db;
use crate::error::{ApiError, AppResult};

pub(crate) const TABLES: &[&str] = &[
    "users",
    "organizations",
    "teams",
    "credentials",
    "projects",
    "inventories",
    "environments",
    "templates",
    "memberships",
    "inventory_sources",
    "template_credentials",
    "workflows",
    "workflow_jobs",
    "jobs",
    "job_events",
    "workflow_nodes",
    "hooks",
    "hook_deliveries",
    "project_teams",
    "settings",
    "seed_state",
    "schedules",
    "api_tokens",
    "sessions",
];

#[derive(Clone, Copy)]
enum Kind {
    Text,
    Small,
    Integer,
    Big,
    Bool,
}

struct Column {
    name: String,
    kind: Kind,
}

pub(crate) async fn to_sqlite(client: &Client, dest: &Path) -> AppResult<()> {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = dest.as_os_str().to_os_string();
        name.push(suffix);
        let _ = std::fs::remove_file(std::path::PathBuf::from(name));
    }
    let sqlite = db::open(dest)?;
    sqlite.pragma_update(None, "foreign_keys", "OFF")?;
    for table in TABLES {
        let target = sqlite_columns(&sqlite, table)?;
        let columns = postgres_columns(client, table).await?;
        let columns: Vec<&Column> = columns
            .iter()
            .filter(|column| target.contains(&column.name))
            .collect();
        if columns.is_empty() {
            continue;
        }
        let names = columns
            .iter()
            .map(|column| quote(&column.name))
            .collect::<Vec<_>>()
            .join(", ");
        let rows = client
            .query(&format!("SELECT {names} FROM {}", quote(table)), &[])
            .await?;
        let placeholders = (1..=columns.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT INTO {} ({names}) VALUES ({placeholders})",
            quote(table)
        );
        let transaction = sqlite.unchecked_transaction()?;
        for row in &rows {
            let values = row_values(row, &columns)?;
            transaction.execute(&sql, rusqlite::params_from_iter(values))?;
        }
        transaction.commit()?;
    }
    sqlite.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}

pub(crate) async fn copy_from_sqlite(
    client: &Client,
    source: &Path,
    replace: bool,
) -> AppResult<()> {
    copy_from_sqlite_with(client, source, replace, |_, _| {}).await
}

pub(crate) async fn copy_from_sqlite_with<F>(
    client: &Client,
    source: &Path,
    replace: bool,
    mut progress: F,
) -> AppResult<()>
where
    F: FnMut(&'static str, i64),
{
    let sqlite = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    if replace {
        for table in TABLES.iter().rev() {
            client
                .execute(&format!("DELETE FROM {}", quote(table)), &[])
                .await?;
        }
    }
    for table in TABLES {
        let source_columns = sqlite_columns(&sqlite, table)?;
        let columns = postgres_columns(client, table).await?;
        let columns: Vec<&Column> = columns
            .iter()
            .filter(|column| source_columns.contains(&column.name))
            .collect();
        if columns.is_empty() {
            continue;
        }
        let names = columns
            .iter()
            .map(|column| quote(&column.name))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = (1..=columns.len())
            .map(|index| format!("${index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let insert = format!(
            "INSERT INTO {} ({names}) VALUES ({placeholders})",
            quote(table)
        );
        let rows = read_sqlite_rows(&sqlite, table, &columns)?;
        let copied = rows.len() as i64;
        for values in rows {
            let params = values
                .iter()
                .zip(&columns)
                .map(|(value, column)| sqlite_param(value, column.kind))
                .collect::<AppResult<Vec<_>>>()?;
            let params = params
                .iter()
                .map(|value| value.as_ref() as &(dyn ToSql + Sync))
                .collect::<Vec<_>>();
            if let Err(err) = client.execute(&insert, &params).await {
                return Err(ApiError::Internal(format!(
                    "Copying table {table} failed: {err}"
                )));
            }
        }
        progress(table, copied);
    }
    reset_sequences(client).await
}

fn read_sqlite_rows(
    sqlite: &Connection,
    table: &str,
    columns: &[&Column],
) -> AppResult<Vec<Vec<Value>>> {
    let names = columns
        .iter()
        .map(|column| quote(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let mut statement = sqlite.prepare(&format!("SELECT {names} FROM {}", quote(table)))?;
    let mut rows = statement.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let mut values = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            values.push(row.get::<_, Value>(index)?);
        }
        out.push(values);
    }
    Ok(out)
}

pub(crate) async fn verify_counts(client: &Client, source: &Path) -> AppResult<i64> {
    let sqlite = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut total = 0;
    for table in TABLES {
        let expected: i64 = sqlite.query_row(
            &format!("SELECT COUNT(*) FROM {}", quote(table)),
            [],
            |row| row.get(0),
        )?;
        let row = client
            .query_one(&format!("SELECT COUNT(*) FROM {}", quote(table)), &[])
            .await?;
        let actual: i64 = row.get(0);
        if expected != actual {
            return Err(ApiError::Internal(format!(
                "The copy of table {table} holds {actual} rows instead of {expected}."
            )));
        }
        total += actual;
    }
    Ok(total)
}

pub(crate) async fn set_schema_version(client: &Client) -> AppResult<()> {
    client
        .execute(
            "INSERT INTO schema_meta (key, value) VALUES ('schema_version', $1)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            &[&db::SCHEMA_VERSION.to_string()],
        )
        .await?;
    Ok(())
}

async fn postgres_columns(client: &Client, table: &str) -> AppResult<Vec<Column>> {
    let rows = client
        .query(
            "SELECT column_name, data_type
             FROM information_schema.columns
             WHERE table_schema = current_schema() AND table_name = $1
             ORDER BY ordinal_position",
            &[&table],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| {
            let name: String = row.get(0);
            let data_type: String = row.get(1);
            Column {
                name,
                kind: match data_type.as_str() {
                    "smallint" => Kind::Small,
                    "integer" => Kind::Integer,
                    "bigint" => Kind::Big,
                    "boolean" => Kind::Bool,
                    _ => Kind::Text,
                },
            }
        })
        .collect())
}

fn sqlite_columns(sqlite: &Connection, table: &str) -> AppResult<Vec<String>> {
    let mut statement = sqlite.prepare(&format!("PRAGMA table_info({})", quote(table)))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    let mut names = Vec::new();
    for name in rows {
        names.push(name?);
    }
    Ok(names)
}

fn row_values(row: &Row, columns: &[&Column]) -> AppResult<Vec<Value>> {
    let mut values = Vec::with_capacity(columns.len());
    for (index, column) in columns.iter().enumerate() {
        values.push(match column.kind {
            Kind::Text => match row.try_get::<usize, Option<String>>(index)? {
                Some(value) => Value::Text(value),
                None => Value::Null,
            },
            Kind::Small => match row.try_get::<usize, Option<i16>>(index)? {
                Some(value) => Value::Integer(i64::from(value)),
                None => Value::Null,
            },
            Kind::Integer => match row.try_get::<usize, Option<i32>>(index)? {
                Some(value) => Value::Integer(i64::from(value)),
                None => Value::Null,
            },
            Kind::Big => match row.try_get::<usize, Option<i64>>(index)? {
                Some(value) => Value::Integer(value),
                None => Value::Null,
            },
            Kind::Bool => match row.try_get::<usize, Option<bool>>(index)? {
                Some(value) => Value::Integer(i64::from(value)),
                None => Value::Null,
            },
        });
    }
    Ok(values)
}

fn sqlite_param(value: &Value, kind: Kind) -> AppResult<Box<dyn ToSql + Sync + Send>> {
    let value: Box<dyn ToSql + Sync + Send> = match kind {
        Kind::Text => Box::new(sqlite_text(value)),
        Kind::Small => Box::new(sqlite_integer(value)?.map(|value| value as i16)),
        Kind::Integer => Box::new(sqlite_integer(value)?.map(|value| value as i32)),
        Kind::Big => Box::new(sqlite_integer(value)?),
        Kind::Bool => Box::new(sqlite_bool(value)?),
    };
    Ok(value)
}

fn sqlite_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Integer(value) => Some(value.to_string()),
        Value::Real(value) => Some(value.to_string()),
        Value::Text(value) => Some(value.clone()),
        Value::Blob(value) => Some(String::from_utf8_lossy(value).to_string()),
    }
}

fn sqlite_integer(value: &Value) -> AppResult<Option<i64>> {
    match value {
        Value::Null => Ok(None),
        Value::Integer(value) => Ok(Some(*value)),
        Value::Real(value) => Ok(Some(*value as i64)),
        Value::Text(value) => value
            .parse::<i64>()
            .map(Some)
            .map_err(|_| ApiError::Internal(format!("A copied value is not an integer: {value}."))),
        Value::Blob(_) => Err(ApiError::Internal(
            "A copied value is a binary blob where a number is required.".into(),
        )),
    }
}

fn sqlite_bool(value: &Value) -> AppResult<Option<bool>> {
    match value {
        Value::Null => Ok(None),
        Value::Integer(value) => Ok(Some(*value != 0)),
        Value::Real(value) => Ok(Some(*value != 0.0)),
        Value::Text(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "t" | "yes" => Ok(Some(true)),
            "0" | "false" | "f" | "no" => Ok(Some(false)),
            _ => Err(ApiError::Internal(format!(
                "A copied value is not a boolean: {value}."
            ))),
        },
        Value::Blob(_) => Err(ApiError::Internal(
            "A copied value is a binary blob where a boolean is required.".into(),
        )),
    }
}

async fn reset_sequences(client: &Client) -> AppResult<()> {
    for table in TABLES {
        let rows = client
            .query(
                "SELECT column_name, pg_get_serial_sequence($1, column_name)
                 FROM information_schema.columns
                 WHERE table_schema = current_schema()
                   AND table_name = $1
                   AND is_identity = 'YES'",
                &[&table],
            )
            .await?;
        for row in rows {
            let column: String = row.get(0);
            let sequence: Option<String> = row.get(1);
            let Some(sequence) = sequence else {
                continue;
            };
            let column = quote(&column);
            let table = quote(table);
            let sql = format!(
                "SELECT setval($1::text::regclass,
                    COALESCE((SELECT MAX({column}) FROM {table}), 1),
                    (SELECT MAX({column}) FROM {table}) IS NOT NULL)"
            );
            client.query_one(&sql, &[&sequence]).await?;
        }
    }
    Ok(())
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
