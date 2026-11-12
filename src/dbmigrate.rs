use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use tokio_postgres::Client;

use crate::config::{self, Config, DatabaseConfig, PostgresConfig, SslMode};
use crate::datalock::lock_data_directory;
use crate::db;
use crate::error::ApiError;
use crate::portable;

const MIN_SERVER_VERSION: u32 = 130_000;

pub const DATABASE_CHECK_IDS: [&str; 4] = ["reachable", "tls", "server_version", "rights"];

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CheckResult {
    pub id: &'static str,
    pub ok: bool,
    pub key: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TableProgress {
    pub table: &'static str,
    pub rows: i64,
    pub done: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct AuditEntry {
    pub user: String,
    pub at: String,
    pub host: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct MigrationStatus {
    pub running: bool,
    pub done: bool,
    pub failed: bool,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub target_host: String,
    pub url: String,
    pub rows: i64,
    pub error: Option<&'static str>,
    pub tables: Vec<TableProgress>,
    pub audit: Option<AuditEntry>,
}

pub(crate) struct PendingTarget {
    pub id: String,
    pub config: PostgresConfig,
}

#[derive(Default)]
pub(crate) struct Runtime {
    pub pending: Option<PendingTarget>,
    pub status: MigrationStatus,
    pub maintenance_since: Option<String>,
}

impl MigrationStatus {
    pub(crate) fn empty_tables() -> Vec<TableProgress> {
        portable::TABLES
            .iter()
            .map(|table| TableProgress {
                table,
                rows: 0,
                done: false,
            })
            .collect()
    }
}

pub(crate) fn parse_form_target(
    host: &str,
    port: u16,
    database: &str,
    user: &str,
    password: &str,
    sslmode: &str,
    ca_file: Option<&str>,
) -> Result<PostgresConfig, ApiError> {
    let host = host.trim();
    let database = database.trim();
    let user = user.trim();
    let sslmode = sslmode.trim();
    let sslmode = if sslmode.is_empty() {
        "require"
    } else {
        sslmode
    };
    if host.is_empty() || database.is_empty() || user.is_empty() || port == 0 {
        return Err(ApiError::bad("database_target_is_invalid"));
    }
    let mode =
        SslMode::parse(sslmode).ok_or_else(|| ApiError::bad("database_target_is_invalid"))?;
    let mut url = url::Url::parse("postgresql://localhost")
        .map_err(|_| ApiError::bad("database_target_is_invalid"))?;
    url.set_username(user)
        .map_err(|_| ApiError::bad("database_target_is_invalid"))?;
    url.set_password((!password.is_empty()).then_some(password))
        .map_err(|_| ApiError::bad("database_target_is_invalid"))?;
    url.set_host(Some(host))
        .map_err(|_| ApiError::bad("database_target_is_invalid"))?;
    url.set_port(Some(port))
        .map_err(|_| ApiError::bad("database_target_is_invalid"))?;
    url.set_path(&format!("/{database}"));
    url.query_pairs_mut().append_pair("sslmode", mode.as_str());
    let ca_file = ca_file
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    match config::parse_database_config(url.as_str(), None, ca_file) {
        Ok(DatabaseConfig::Postgres(target)) => Ok(target),
        _ => Err(ApiError::bad("database_target_is_invalid")),
    }
}

pub(crate) async fn run_checks(target: &PostgresConfig, source: &Path) -> Vec<CheckResult> {
    let mut checks = Vec::with_capacity(6);
    match check_source(source) {
        Ok(()) => checks.push(check(
            "source_schema",
            true,
            "database_check_source_schema_ok",
        )),
        Err(_) => checks.push(check(
            "source_schema",
            false,
            "database_check_source_schema_failed",
        )),
    }
    match crate::backend::connect_client(target).await {
        Err(err) => {
            tracing::warn!(error = %redact(&err.to_string(), target), "database target is unreachable");
            for id in DATABASE_CHECK_IDS {
                let key = if id == "reachable" {
                    "database_check_unreachable"
                } else {
                    "database_check_not_run"
                };
                checks.push(check(id, false, key));
            }
            checks.push(check("empty", false, "database_check_not_run"));
        }
        Ok(client) => {
            checks.push(check("reachable", true, "database_check_reachable_ok"));
            checks.push(tls_check(&client, target.sslmode).await);
            checks.push(server_version_check(&client).await);
            checks.push(rights_check(&client).await);
            checks.push(empty_check(&client).await);
        }
    }
    checks
}

fn check(id: &'static str, ok: bool, key: &'static str) -> CheckResult {
    CheckResult { id, ok, key }
}

pub(crate) fn all_passed(checks: &[CheckResult]) -> bool {
    checks.iter().all(|item| item.ok)
}

pub(crate) fn audit_entry(user: &str, target: &PostgresConfig) -> AuditEntry {
    AuditEntry {
        user: user.to_string(),
        at: db::now(),
        host: target.target(),
    }
}

pub(crate) fn audit_line(entry: &AuditEntry) -> String {
    format!(
        "database migration: user={} at={} target={}",
        entry.user, entry.at, entry.host
    )
}

fn redact(message: &str, target: &PostgresConfig) -> String {
    let password = target.password();
    if password.is_empty() {
        message.to_string()
    } else {
        message.replace(password, "***")
    }
}

async fn tls_check(client: &Client, mode: SslMode) -> CheckResult {
    if mode == SslMode::Disable {
        return check("tls", true, "database_check_tls_disabled");
    }
    let encrypted = client
        .query_one(
            "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            &[],
        )
        .await
        .ok()
        .and_then(|row| row.get::<_, Option<bool>>(0));
    match encrypted {
        Some(true) => check("tls", true, "database_check_tls_ok"),
        Some(false) if mode == SslMode::Prefer => check("tls", true, "database_check_tls_disabled"),
        Some(false) => check("tls", false, "database_check_tls_failed"),
        None => check("tls", true, "database_check_tls_ok"),
    }
}

async fn server_version_check(client: &Client) -> CheckResult {
    let version = client
        .query_one("SHOW server_version_num", &[])
        .await
        .ok()
        .and_then(|row| row.get::<_, String>(0).parse::<u32>().ok())
        .unwrap_or(0);
    if version >= MIN_SERVER_VERSION {
        check("server_version", true, "database_check_server_version_ok")
    } else {
        check(
            "server_version",
            false,
            "database_check_server_version_failed",
        )
    }
}

async fn rights_check(client: &Client) -> CheckResult {
    let can_create = client
        .query_one(
            "SELECT has_schema_privilege(current_schema(), 'CREATE')",
            &[],
        )
        .await
        .ok()
        .and_then(|row| row.get::<_, Option<bool>>(0));
    if can_create == Some(true) {
        check("rights", true, "database_check_rights_ok")
    } else {
        check("rights", false, "database_check_rights_failed")
    }
}

async fn empty_check(client: &Client) -> CheckResult {
    let tables = match client
        .query(
            "SELECT table_name FROM information_schema.tables
             WHERE table_schema = current_schema() AND table_type = 'BASE TABLE'",
            &[],
        )
        .await
    {
        Ok(tables) => tables,
        Err(_) => return check("empty", false, "database_check_not_empty_failed"),
    };
    for row in tables {
        let table: String = row.get(0);
        if table == "schema_meta" {
            continue;
        }
        let sql = format!("SELECT EXISTS (SELECT 1 FROM {} LIMIT 1)", quote(&table));
        match client.query_one(&sql, &[]).await {
            Ok(row) => {
                let exists: bool = row.get(0);
                if exists {
                    return check("empty", false, "database_check_not_empty_failed");
                }
            }
            Err(_) => return check("empty", false, "database_check_not_empty_failed"),
        }
    }
    check("empty", true, "database_check_empty_ok")
}

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
    let rows = match copy_source(&client, &source, |_, _| {}).await {
        Ok(rows) => rows,
        Err(err) => {
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

pub(crate) async fn copy_source<F>(
    client: &Client,
    source: &Path,
    progress: F,
) -> anyhow::Result<i64>
where
    F: FnMut(&'static str, i64),
{
    client.batch_execute(&db::postgres_schema()).await?;
    client.batch_execute("BEGIN").await?;
    let copied = async {
        portable::copy_from_sqlite_with(client, source, false, progress).await?;
        let rows = portable::verify_counts(client, source).await?;
        portable::set_schema_version(client).await?;
        Ok::<i64, ApiError>(rows)
    }
    .await;
    match copied {
        Ok(rows) => {
            client.batch_execute("COMMIT").await?;
            Ok(rows)
        }
        Err(err) => {
            let _ = client.batch_execute("ROLLBACK").await;
            Err(anyhow::anyhow!("{err}"))
        }
    }
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

pub(crate) fn check_source(source: &Path) -> anyhow::Result<()> {
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

pub(crate) async fn precheck(client: &Client) -> anyhow::Result<()> {
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

pub(crate) fn target_url(target: &PostgresConfig) -> String {
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

    #[test]
    fn the_form_target_keeps_the_password_out_of_the_url_and_audit_line() {
        let target = parse_form_target(
            "db.example",
            5432,
            "koreograf",
            "koreograf",
            "s3cret-password",
            "verify-full",
            None,
        )
        .expect("target");
        let url = target_url(&target);
        assert!(!url.contains("s3cret-password"), "{url}");
        assert!(url.contains("sslmode=verify-full"), "{url}");
        let audit = audit_entry("admin", &target);
        let line = audit_line(&audit);
        assert!(line.contains("admin"), "{line}");
        assert!(line.contains("db.example:5432/koreograf"), "{line}");
        assert!(!line.contains("s3cret-password"), "{line}");
    }

    #[test]
    fn an_incomplete_target_is_refused() {
        assert!(parse_form_target("", 5432, "db", "user", "pw", "require", None).is_err());
        assert!(parse_form_target("host", 0, "db", "user", "pw", "require", None).is_err());
        assert!(parse_form_target("host", 5432, "", "user", "pw", "require", None).is_err());
        assert!(parse_form_target("host", 5432, "db", "", "pw", "require", None).is_err());
        assert!(parse_form_target("host", 5432, "db", "user", "pw", "broken", None).is_err());
    }
}
