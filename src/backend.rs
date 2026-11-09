use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use futures::executor::block_on;
use rusqlite::Connection;
use rustls::pki_types::pem::PemObject;
use tokio_postgres::Client;

use rusqlite::params;

use crate::config::{DatabaseConfig, PostgresConfig, SslMode};
use crate::db::{self, Environment, Team, User, UserMembership};
use crate::error::{ApiError, AppResult};

pub(crate) trait Backend: Send + Sync {
    fn engine(&self) -> &'static str;
    fn sqlite(&self) -> Option<&Mutex<Connection>>;
    fn ping(&self) -> AppResult<()>;
    fn settings_rows(&self) -> AppResult<HashMap<String, String>>;
    fn user_by_username(&self, username: &str) -> AppResult<Option<User>>;
    fn user_by_session(&self, token: &str) -> AppResult<Option<User>>;
    fn user_by_api_token(&self, token_hash: &str) -> AppResult<Option<User>>;
    fn create_session(&self, token: &str, user_id: &str) -> AppResult<()>;
    fn delete_session(&self, token: &str) -> AppResult<()>;
    fn purge_sessions(&self) -> AppResult<()>;
    fn insert_user(
        &self,
        id: &str,
        username: &str,
        display_name: &str,
        password_hash: Option<&str>,
        auth_source: &str,
        is_admin: bool,
    ) -> AppResult<()>;
    fn list_teams_for_user(&self, user_id: &str, admin: bool) -> AppResult<Vec<(Team, String)>>;
    fn list_user_memberships(&self, user_id: &str) -> AppResult<Vec<UserMembership>>;
    fn replace_settings(&self, rows: &[(String, String)]) -> AppResult<()>;
    fn environment(&self, id: &str) -> AppResult<Option<Environment>>;
}

pub(crate) fn open(database: &DatabaseConfig, data_dir: &Path) -> anyhow::Result<Box<dyn Backend>> {
    match database {
        DatabaseConfig::Sqlite => Ok(Box::new(SqliteBackend::open(&sqlite_path(data_dir))?)),
        DatabaseConfig::Postgres(config) => Ok(Box::new(PostgresBackend::open(config)?)),
    }
}

pub(crate) struct SqliteBackend {
    conn: Mutex<Connection>,
}

impl SqliteBackend {
    fn open(path: &Path) -> AppResult<Self> {
        Ok(Self {
            conn: Mutex::new(db::open(path)?),
        })
    }
}

pub(crate) fn sqlite_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("koreograf.db")
}

fn lock(conn: &Mutex<Connection>) -> AppResult<std::sync::MutexGuard<'_, Connection>> {
    conn.lock()
        .map_err(|_| ApiError::Internal("database lock is damaged".into()))
}

impl Backend for SqliteBackend {
    fn engine(&self) -> &'static str {
        "sqlite"
    }

    fn sqlite(&self) -> Option<&Mutex<Connection>> {
        Some(&self.conn)
    }

    fn ping(&self) -> AppResult<()> {
        lock(&self.conn)?.query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }

    fn settings_rows(&self) -> AppResult<HashMap<String, String>> {
        let conn = lock(&self.conn)?;
        crate::settings::read_rows(&conn)
    }

    fn user_by_username(&self, username: &str) -> AppResult<Option<User>> {
        let conn = lock(&self.conn)?;
        db::user_by_username(&conn, username)
    }

    fn user_by_session(&self, token: &str) -> AppResult<Option<User>> {
        let conn = lock(&self.conn)?;
        db::user_by_session(&conn, token)
    }

    fn user_by_api_token(&self, token_hash: &str) -> AppResult<Option<User>> {
        let conn = lock(&self.conn)?;
        db::user_by_api_token(&conn, token_hash)
    }

    fn create_session(&self, token: &str, user_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::create_session(&conn, token, user_id)
    }

    fn delete_session(&self, token: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::delete_session(&conn, token)
    }

    fn purge_sessions(&self) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::purge_sessions(&conn)
    }

    fn insert_user(
        &self,
        id: &str,
        username: &str,
        display_name: &str,
        password_hash: Option<&str>,
        auth_source: &str,
        is_admin: bool,
    ) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::insert_user(
            &conn,
            id,
            username,
            display_name,
            password_hash,
            auth_source,
            is_admin,
        )
    }

    fn list_teams_for_user(&self, user_id: &str, admin: bool) -> AppResult<Vec<(Team, String)>> {
        let conn = lock(&self.conn)?;
        db::list_teams_for_user(&conn, user_id, admin)
    }

    fn list_user_memberships(&self, user_id: &str) -> AppResult<Vec<UserMembership>> {
        let conn = lock(&self.conn)?;
        db::list_user_memberships(&conn, user_id)
    }

    fn replace_settings(&self, rows: &[(String, String)]) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM settings", [])?;
        for (key, value) in rows {
            tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)",
                params![key, value],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn environment(&self, id: &str) -> AppResult<Option<Environment>> {
        let conn = lock(&self.conn)?;
        db::environment(&conn, id)
    }
}

pub(crate) struct PostgresBackend {
    client: tokio::sync::Mutex<Client>,
    runtime: Option<tokio::runtime::Runtime>,
}

impl Drop for PostgresBackend {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

impl PostgresBackend {
    pub(crate) fn open(config: &PostgresConfig) -> anyhow::Result<Self> {
        let (client, runtime) = connect(config)?;
        let backend = Self {
            client: tokio::sync::Mutex::new(client),
            runtime: Some(runtime),
        };
        backend.ensure_schema()?;
        Ok(backend)
    }

    fn ensure_schema(&self) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client.batch_execute(&db::postgres_schema()).await?;
            let found: Option<String> = client
                .query_opt(
                    "SELECT value FROM schema_meta WHERE key = 'schema_version'",
                    &[],
                )
                .await?
                .map(|row| row.get(0));
            if let Some(found) = found {
                let version = found.parse::<u32>().unwrap_or(u32::MAX);
                if version > db::SCHEMA_VERSION {
                    return Err(ApiError::Internal(format!(
                        "The database uses schema {version}, but this build supports {}. Upgrade Koreograf before opening it.",
                        db::SCHEMA_VERSION
                    )));
                }
            }
            client
                .execute(
                    "INSERT INTO schema_meta (key, value) VALUES ('schema_version', $1)
                     ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                    &[&db::SCHEMA_VERSION.to_string()],
                )
                .await?;
            Ok(())
        })
    }
}

fn connect(config: &PostgresConfig) -> anyhow::Result<(Client, tokio::runtime::Runtime)> {
    let config = config.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = sender.send(Err(anyhow::anyhow!(
                    "The PostgreSQL client runtime cannot start: {err}"
                )));
                return;
            }
        };
        let result = runtime.block_on(connect_client(&config));
        match result {
            Ok(client) => {
                let _ = sender.send(Ok((client, runtime)));
            }
            Err(err) => {
                let _ = sender.send(Err(err));
            }
        }
    });
    receiver.recv().map_err(|_| {
        anyhow::anyhow!("The PostgreSQL client stopped before the connection was ready.")
    })?
}

async fn connect_client(config: &PostgresConfig) -> anyhow::Result<Client> {
    let mut pg = tokio_postgres::Config::new();
    pg.host(&config.host)
        .port(config.port)
        .dbname(&config.database)
        .user(&config.user)
        .application_name("koreograf")
        .connect_timeout(Duration::from_secs(config.connect_timeout_secs))
        .ssl_mode(match config.sslmode {
            SslMode::Disable => tokio_postgres::config::SslMode::Disable,
            SslMode::Prefer => tokio_postgres::config::SslMode::Prefer,
            SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => {
                tokio_postgres::config::SslMode::Require
            }
        });
    if !config.password().is_empty() {
        pg.password(config.password());
    }
    let target = config.target();
    match config.sslmode {
        SslMode::Disable => {
            let (client, connection) = pg
                .connect(tokio_postgres::NoTls)
                .await
                .map_err(|err| connection_error(err, &target))?;
            tokio::spawn(async move {
                if let Err(err) = connection.await {
                    tracing::error!(error = %err, "PostgreSQL connection ended");
                }
            });
            Ok(client)
        }
        _ => {
            let tls = tls_connector(config)?;
            let (client, connection) = pg
                .connect(tls)
                .await
                .map_err(|err| connection_error(err, &target))?;
            tokio::spawn(async move {
                if let Err(err) = connection.await {
                    tracing::error!(error = %err, "PostgreSQL connection ended");
                }
            });
            Ok(client)
        }
    }
}

fn tls_connector(
    config: &PostgresConfig,
) -> anyhow::Result<tokio_postgres_rustls::MakeRustlsConnect> {
    let mut roots = rustls::RootCertStore::empty();
    if let Some(path) = &config.ca_file {
        let certs = rustls::pki_types::CertificateDer::pem_file_iter(path)
            .map_err(|_| anyhow::anyhow!("KOREOGRAF_DATABASE_CA_FILE cannot be read."))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| anyhow::anyhow!("KOREOGRAF_DATABASE_CA_FILE is not valid PEM."))?;
        if certs.is_empty() {
            return Err(anyhow::anyhow!(
                "KOREOGRAF_DATABASE_CA_FILE holds no certificate."
            ));
        }
        for cert in certs {
            roots
                .add(cert)
                .map_err(|_| anyhow::anyhow!("KOREOGRAF_DATABASE_CA_FILE is not valid PEM."))?;
        }
    } else {
        let native = rustls_native_certs::load_native_certs();
        for cert in native.certs {
            roots.add(cert).ok();
        }
    }
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let client = rustls::ClientConfig::builder_with_provider(provider.into())
        .with_safe_default_protocol_versions()
        .map_err(|err| anyhow::anyhow!("The PostgreSQL TLS client cannot start: {err}"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(tokio_postgres_rustls::MakeRustlsConnect::new(client))
}

fn connection_error(err: tokio_postgres::Error, target: &str) -> anyhow::Error {
    if let Some(db) = err.as_db_error() {
        let message = match db.code().code() {
            "28P01" | "28000" => {
                format!("PostgreSQL at {target} rejected the database user or password.")
            }
            "3D000" => format!("PostgreSQL at {target} has no database with that name."),
            "42501" => format!("PostgreSQL at {target} denied the required rights to the user."),
            _ => format!(
                "PostgreSQL at {target} refused the connection: {}",
                db.message()
            ),
        };
        return anyhow::anyhow!(message);
    }
    anyhow::anyhow!(
        "PostgreSQL at {target} is unreachable: {:#}",
        anyhow::Error::new(err)
    )
}

fn row_user(row: &tokio_postgres::Row) -> AppResult<User> {
    let is_admin: i32 = row.try_get(5)?;
    let disabled: i32 = row.try_get(6)?;
    Ok(User {
        id: row.try_get(0)?,
        username: row.try_get(1)?,
        display_name: row.try_get(2)?,
        password_hash: row.try_get(3)?,
        auth_source: row.try_get(4)?,
        is_admin: is_admin != 0,
        disabled: disabled != 0,
        created_at: row.try_get(7)?,
        email: row.try_get(8)?,
    })
}

fn row_team(row: &tokio_postgres::Row) -> AppResult<Team> {
    Ok(Team {
        id: row.try_get(0)?,
        name: row.try_get(1)?,
        description: row.try_get(2)?,
        ldap_group: row.try_get(3)?,
        created_at: row.try_get(4)?,
        organization_id: row.try_get(5)?,
        organization_name: row.try_get(6)?,
    })
}

fn present(value: Option<String>) -> Option<String> {
    value.and_then(|item| {
        let item = item.trim().to_string();
        if item.is_empty() {
            None
        } else {
            Some(item)
        }
    })
}

fn row_environment(row: &tokio_postgres::Row) -> AppResult<Environment> {
    let team_id = present(row.try_get(1)?);
    let organization_id = present(row.try_get(2)?);
    let organization_name: String = row.try_get(9)?;
    let team_name: String = row.try_get(10)?;
    let scope = if team_id.is_some() {
        "team"
    } else if organization_id.is_some() {
        "organization"
    } else {
        "global"
    };
    Ok(Environment {
        id: row.try_get(0)?,
        team_id,
        organization_id,
        name: row.try_get(3)?,
        tool: row.try_get(11)?,
        image: row.try_get(4)?,
        pull_policy: row.try_get(5)?,
        network: row.try_get(6)?,
        description: row.try_get(7)?,
        created_at: row.try_get(8)?,
        scope: scope.to_string(),
        organization_name: if scope == "organization" {
            organization_name
        } else {
            String::new()
        },
        team_name: if scope == "team" {
            team_name
        } else {
            String::new()
        },
    })
}

const USER_COLUMNS: &str =
    "id, username, display_name, password_hash, auth_source, is_admin, disabled, created_at, email";

const TEAM_COLUMNS: &str = "t.id, t.name, t.description, t.ldap_group, t.created_at, \
     COALESCE(t.organization_id, ''), COALESCE(o.name, '')";

impl Backend for PostgresBackend {
    fn engine(&self) -> &'static str {
        "postgresql"
    }

    fn sqlite(&self) -> Option<&Mutex<Connection>> {
        None
    }

    fn ping(&self) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client.query_one("SELECT 1", &[]).await?;
            Ok(())
        })
    }

    fn settings_rows(&self) -> AppResult<HashMap<String, String>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client.query("SELECT key, value FROM settings", &[]).await?;
            Ok(rows
                .into_iter()
                .map(|row| (row.get(0), row.get(1)))
                .collect())
        })
    }

    fn user_by_username(&self, username: &str) -> AppResult<Option<User>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    &format!("SELECT {USER_COLUMNS} FROM users WHERE lower(username) = lower($1)"),
                    &[&username],
                )
                .await?;
            row.as_ref().map(row_user).transpose()
        })
    }

    fn user_by_session(&self, token: &str) -> AppResult<Option<User>> {
        let row = block_on(async {
            let client = self.client.lock().await;
            client
                .query_opt(
                    "SELECT u.id, u.username, u.display_name, u.password_hash, u.auth_source,
                            u.is_admin, u.disabled, u.created_at, u.email, s.expires_at
                     FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token = $1",
                    &[&token],
                )
                .await
        })?;
        let Some(row) = row else {
            return Ok(None);
        };
        let expires: String = row.try_get(9)?;
        if expires < db::now() {
            self.delete_session(token)?;
            return Ok(None);
        }
        Ok(Some(row_user(&row)?))
    }

    fn user_by_api_token(&self, token_hash: &str) -> AppResult<Option<User>> {
        let row = block_on(async {
            let client = self.client.lock().await;
            client
                .query_opt(
                    "SELECT u.id, u.username, u.display_name, u.password_hash, u.auth_source,
                            u.is_admin, u.disabled, u.created_at, u.email, t.id
                     FROM api_tokens t JOIN users u ON u.id = t.user_id WHERE t.token_hash = $1",
                    &[&token_hash],
                )
                .await
        })?;
        let Some(row) = row else {
            return Ok(None);
        };
        let token_id: String = row.try_get(9)?;
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "UPDATE api_tokens SET last_used_at = $2 WHERE id = $1",
                    &[&token_id, &db::now()],
                )
                .await
        })?;
        Ok(Some(row_user(&row)?))
    }

    fn create_session(&self, token: &str, user_id: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO sessions (token, user_id, expires_at) VALUES ($1, $2, $3)",
                    &[&token, &user_id, &db::plus_secs(7 * 24 * 3600)],
                )
                .await?;
            Ok(())
        })
    }

    fn delete_session(&self, token: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute("DELETE FROM sessions WHERE token = $1", &[&token])
                .await?;
            Ok(())
        })
    }

    fn purge_sessions(&self) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute("DELETE FROM sessions WHERE expires_at < $1", &[&db::now()])
                .await?;
            Ok(())
        })
    }

    fn insert_user(
        &self,
        id: &str,
        username: &str,
        display_name: &str,
        password_hash: Option<&str>,
        auth_source: &str,
        is_admin: bool,
    ) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO users
                        (id, username, display_name, password_hash, auth_source, is_admin, created_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    &[
                        &id,
                        &username,
                        &display_name,
                        &password_hash,
                        &auth_source,
                        &(is_admin as i32),
                        &db::now(),
                    ],
                )
                .await?;
            Ok(())
        })
    }

    fn list_teams_for_user(&self, user_id: &str, admin: bool) -> AppResult<Vec<(Team, String)>> {
        let sql = if admin {
            format!(
                "SELECT {TEAM_COLUMNS},
                        COALESCE((SELECT role FROM memberships m WHERE m.team_id = t.id AND m.user_id = $1), 'owner')
                 FROM teams t LEFT JOIN organizations o ON o.id = t.organization_id ORDER BY t.name"
            )
        } else {
            format!(
                "SELECT {TEAM_COLUMNS}, m.role
                 FROM teams t JOIN memberships m ON m.team_id = t.id
                 LEFT JOIN organizations o ON o.id = t.organization_id
                 WHERE m.user_id = $1 ORDER BY t.name"
            )
        };
        block_on(async {
            let client = self.client.lock().await;
            let rows = client.query(&sql, &[&user_id]).await?;
            rows.iter()
                .map(|row| Ok((row_team(row)?, row.try_get::<_, String>(7)?)))
                .collect()
        })
    }

    fn list_user_memberships(&self, user_id: &str) -> AppResult<Vec<UserMembership>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    "SELECT t.id, t.name, COALESCE(t.organization_id, ''), COALESCE(o.name, ''), m.role, m.source
                     FROM memberships m
                     JOIN teams t ON t.id = m.team_id
                     LEFT JOIN organizations o ON o.id = t.organization_id
                     WHERE m.user_id = $1
                     ORDER BY o.name, t.name",
                    &[&user_id],
                )
                .await?;
            rows.iter()
                .map(|row| {
                    Ok(UserMembership {
                        team_id: row.try_get(0)?,
                        team_name: row.try_get(1)?,
                        organization_id: row.try_get(2)?,
                        organization_name: row.try_get(3)?,
                        role: row.try_get(4)?,
                        source: row.try_get(5)?,
                    })
                })
                .collect()
        })
    }

    fn replace_settings(&self, rows: &[(String, String)]) -> AppResult<()> {
        let keys: Vec<&str> = rows.iter().map(|(key, _)| key.as_str()).collect();
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            for (key, value) in rows {
                transaction
                    .execute(
                        "INSERT INTO settings (key, value) VALUES ($1, $2)
                         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                        &[key, value],
                    )
                    .await?;
            }
            transaction
                .execute("DELETE FROM settings WHERE NOT (key = ANY($1))", &[&keys])
                .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn environment(&self, id: &str) -> AppResult<Option<Environment>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    "SELECT e.id, e.team_id, e.organization_id, e.name, e.image, e.pull_policy,
                            e.network, e.description, e.created_at, COALESCE(o.name, ''),
                            COALESCE(t.name, ''), e.tool
                     FROM environments e
                     LEFT JOIN organizations o ON o.id = e.organization_id
                     LEFT JOIN teams t ON t.id = e.team_id
                     WHERE e.id = $1",
                    &[&id],
                )
                .await?;
            row.as_ref().map(row_environment).transpose()
        })
    }
}
