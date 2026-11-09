use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use tokio_postgres::Client;

use crate::config::{self, Config, DatabaseConfig, PostgresConfig};
use crate::datalock::lock_data_directory;
use crate::db;
use crate::portable;

const MIN_SERVER_VERSION: u32 = 130_000;

pub fn db_migrate_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter().skip(1);
    args.next().is_some_and(|arg| arg.as_ref() == "db")
        && args.next().is_some_and(|arg| arg.as_ref() == "migrate")
}

pub async fn db_migrate(args: &[String]) -> anyhow::Result<i32> {
    let Some(url) = target_argument(args)? else {
        eprintln!("Usage: koreograf db migrate --to postgres://user@host/database");
        return Ok(2);
    };
    match run(&url).await {
        Ok(()) => Ok(0),
        Err(err) => {
            eprintln!("Database migration failed: {err}");
            Ok(1)
        }
    }
}

fn target_argument(args: &[String]) -> anyhow::Result<Option<String>> {
    let mut target = None;
    let mut args = args.iter().skip(3);
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--to=") {
            if target.is_some() || value.is_empty() {
                return Ok(None);
            }
            target = Some(value.to_string());
        } else if arg == "--to" {
            let Some(value) = args.next() else {
                return Ok(None);
            };
            if target.is_some() || value.is_empty() {
                return Ok(None);
            }
            target = Some(value.clone());
        } else {
            return Ok(None);
        }
    }
    Ok(target)
}

async fn run(url: &str) -> anyhow::Result<()> {
    let target = parse_target(url)?;
    let cfg = Config::from_env();
    let data_dir = std::path::absolute(&cfg.data_dir)?;
    let _lock = lock_data_directory(&data_dir)?;
    let source = data_dir.join("koreograf.db");
    if !source.is_file() {
        anyhow::bail!(
            "{} does not contain a SQLite database to copy.",
            data_dir.display()
        );
    }
    check_source(&source)?;
    let client = crate::backend::connect_client(&target).await?;
    precheck(&client).await?;
    client.batch_execute(&db::postgres_schema()).await?;
    client.batch_execute("BEGIN").await?;
    let copied = async {
        portable::copy_from_sqlite(&client, &source, false).await?;
        let rows = portable::verify_counts(&client, &source).await?;
        portable::set_schema_version(&client).await?;
        Ok::<i64, crate::error::ApiError>(rows)
    }
    .await;
    let rows = match copied {
        Ok(rows) => {
            client.batch_execute("COMMIT").await?;
            rows
        }
        Err(err) => {
            let _ = client.batch_execute("ROLLBACK").await;
            return Err(anyhow::anyhow!(
                "{err}. The target was rolled back and can be reused."
            ));
        }
    };
    println!("Copied {rows} rows into {} tables.", portable::TABLES.len());
    println!("Set this value before the next start and restart Koreograf:");
    println!("KOREOGRAF_DATABASE_URL='{}'", target_url(&target));
    if !target.password().is_empty() {
        println!("The password is not printed. Keep it in KOREOGRAF_DATABASE_PASSWORD_FILE.");
    }
    Ok(())
}

fn parse_target(url: &str) -> anyhow::Result<PostgresConfig> {
    let password_file = env_path("KOREOGRAF_DATABASE_PASSWORD_FILE");
    let ca_file = env_path("KOREOGRAF_DATABASE_CA_FILE");
    match config::parse_database_config(url, password_file, ca_file) {
        Ok(DatabaseConfig::Postgres(target)) => Ok(target),
        Ok(DatabaseConfig::Sqlite) => {
            anyhow::bail!("db migrate needs a postgres:// or postgresql:// target.")
        }
        Err(err) => anyhow::bail!("{err}"),
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn check_source(source: &Path) -> anyhow::Result<()> {
    let conn = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.pragma_update(None, "query_only", true)?;
    let version = db::schema_version(&conn)?;
    if version != db::SCHEMA_VERSION {
        anyhow::bail!(
            "The SQLite database uses schema {version}, but this build copies schema {}. Start the current build once against the SQLite database before copying it.",
            db::SCHEMA_VERSION
        );
    }
    Ok(())
}

async fn precheck(client: &Client) -> anyhow::Result<()> {
    let version: String = client
        .query_one("SHOW server_version_num", &[])
        .await?
        .get(0);
    let version: u32 = version.parse().unwrap_or(0);
    if version < MIN_SERVER_VERSION {
        anyhow::bail!("PostgreSQL {version} is not supported. Use PostgreSQL 13 or newer.");
    }
    let can_create: Option<bool> = client
        .query_one(
            "SELECT has_schema_privilege(current_schema(), 'CREATE')",
            &[],
        )
        .await?
        .get(0);
    if can_create != Some(true) {
        anyhow::bail!("The PostgreSQL user cannot create tables in the current schema.");
    }
    let tables = client
        .query(
            "SELECT table_name FROM information_schema.tables
             WHERE table_schema = current_schema() AND table_type = 'BASE TABLE'",
            &[],
        )
        .await?;
    for row in tables {
        let table: String = row.get(0);
        if table == "schema_meta" {
            continue;
        }
        let sql = format!("SELECT EXISTS (SELECT 1 FROM {} LIMIT 1)", quote(&table));
        let exists: bool = client.query_one(&sql, &[]).await?.get(0);
        if exists {
            anyhow::bail!("The PostgreSQL target is not empty: table {table} already holds rows.");
        }
    }
    Ok(())
}

fn target_url(target: &PostgresConfig) -> String {
    let mut url = match url::Url::parse("postgresql://localhost") {
        Ok(url) => url,
        Err(_) => {
            return format!(
                "postgresql://{}:{}/{}",
                target.host, target.port, target.database
            )
        }
    };
    let _ = url.set_username(&target.user);
    let _ = url.set_password(None);
    let _ = url.set_host(Some(&target.host));
    let _ = url.set_port(Some(target.port));
    url.set_path(&format!("/{}", target.database));
    url.query_pairs_mut()
        .append_pair("sslmode", target.sslmode.as_str());
    url.to_string()
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_is_the_db_migrate_pair_after_the_program_name() {
        assert!(db_migrate_requested([
            "koreograf",
            "db",
            "migrate",
            "--to",
            "x"
        ]));
        assert!(!db_migrate_requested(["koreograf", "migrate"]));
        assert!(!db_migrate_requested(["koreograf", "db"]));
        assert!(!db_migrate_requested(["koreograf", "db", "migrate-all"]));
    }

    #[test]
    fn the_target_is_the_value_after_to() {
        let args = ["koreograf", "db", "migrate", "--to", "postgres://db/x"]
            .map(str::to_string)
            .to_vec();
        assert_eq!(
            target_argument(&args).unwrap().as_deref(),
            Some("postgres://db/x")
        );
        let args = ["koreograf", "db", "migrate", "--to=postgres://db/x"]
            .map(str::to_string)
            .to_vec();
        assert_eq!(
            target_argument(&args).unwrap().as_deref(),
            Some("postgres://db/x")
        );
    }

    #[test]
    fn a_sqlite_target_is_refused() {
        assert!(parse_target("").is_err());
        assert!(parse_target("sqlite:data/koreograf.db").is_err());
    }
}
