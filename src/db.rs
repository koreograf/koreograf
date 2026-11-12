use std::fs;
use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

use crate::error::{ApiError, AppResult};

pub(crate) const SCHEMA_VERSION: u32 = 1;
const BASELINE_VERSION: u32 = 1;

pub fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn plus_secs(secs: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::seconds(secs))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

pub(crate) struct Migration {
    pub(crate) version: u32,
    pub(crate) name: &'static str,
    pub(crate) sql: &'static str,
}

pub(crate) const SQLITE_MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "0001_baseline",
    sql: include_str!("../migrations/sqlite/0001_baseline.sql"),
}];

pub(crate) const POSTGRES_MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "0001_baseline",
    sql: include_str!("../migrations/postgres/0001_baseline.sql"),
}];

const META_TABLE: &str = "CREATE TABLE IF NOT EXISTS schema_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
)";

const LEGACY_MARKER: &str = "legacy_reconciled";

fn column_exists(c: &Connection, table: &str, column: &str) -> AppResult<bool> {
    let mut stmt = c.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for name in rows {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn legacy_migrate(c: &Connection) -> AppResult<()> {
    if !column_exists(c, "users", "disabled")? {
        c.execute(
            "ALTER TABLE users ADD COLUMN disabled INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !column_exists(c, "users", "email")? {
        c.execute("ALTER TABLE users ADD COLUMN email TEXT", [])?;
    }
    if !column_exists(c, "teams", "organization_id")? {
        c.execute("ALTER TABLE teams ADD COLUMN organization_id TEXT", [])?;
    }
    if !column_exists(c, "projects", "organization_id")? {
        c.execute("ALTER TABLE projects ADD COLUMN organization_id TEXT", [])?;
    }
    let mut stmt = c.prepare(
        "SELECT id, name FROM teams WHERE organization_id IS NULL OR organization_id = ''",
    )?;
    let pending = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let pending = pending.collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    for (team_id, name) in pending {
        let org_id = format!("org-{team_id}");
        let org_name = match c.query_row(
            "SELECT id FROM organizations WHERE name = ?1",
            [&name],
            |row| row.get::<_, String>(0),
        ) {
            Ok(existing) => {
                c.execute(
                    "UPDATE teams SET organization_id = ?2 WHERE id = ?1",
                    params![team_id, existing],
                )?;
                continue;
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => name,
            Err(err) => return Err(err.into()),
        };
        c.execute(
            "INSERT INTO organizations (id, name, description, created_at) VALUES (?1, ?2, '', ?3)",
            params![org_id, org_name, now()],
        )?;
        c.execute(
            "UPDATE teams SET organization_id = ?2 WHERE id = ?1",
            params![team_id, org_id],
        )?;
    }
    c.execute(
        "UPDATE projects SET organization_id = (
            SELECT organization_id FROM teams WHERE teams.id = projects.team_id
         ) WHERE organization_id IS NULL OR organization_id = ''",
        [],
    )?;
    if !column_exists(c, "inventories", "source_kind")? {
        c.execute(
            "ALTER TABLE inventories ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'ini'",
            [],
        )?;
    }
    if !column_exists(c, "inventories", "source_json")? {
        c.execute(
            "ALTER TABLE inventories ADD COLUMN source_json TEXT NOT NULL DEFAULT '{}'",
            [],
        )?;
    }
    if !column_exists(c, "inventories", "credential_id")? {
        c.execute("ALTER TABLE inventories ADD COLUMN credential_id TEXT", [])?;
    }
    if !column_exists(c, "credentials", "auto")? {
        c.execute(
            "ALTER TABLE credentials ADD COLUMN auto INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !column_exists(c, "credentials", "disabled")? {
        c.execute(
            "ALTER TABLE credentials ADD COLUMN disabled INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    c.execute("UPDATE credentials SET auto = 0 WHERE auto = 1", [])?;
    c.execute(
        "INSERT INTO inventory_sources (
            id, inventory_id, name, kind, source_json, credential_id, body, position, created_at
         )
         SELECT i.id || '-quelle', i.id,
            CASE i.source_kind
                WHEN 'netbox' THEN 'NetBox'
                WHEN 'project' THEN 'Project'
                WHEN 'mixed' THEN 'Mixed'
                ELSE 'Manual'
            END,
            CASE WHEN i.source_kind = 'mixed' THEN 'ini' ELSE i.source_kind END,
            COALESCE(NULLIF(i.source_json, ''), '{}'),
            i.credential_id,
            i.body,
            0,
            i.created_at
         FROM inventories i
         WHERE NOT EXISTS (
            SELECT 1 FROM inventory_sources s WHERE s.inventory_id = i.id
         )",
        [],
    )?;
    if !column_exists(c, "jobs", "parameters")? {
        c.execute(
            "ALTER TABLE jobs ADD COLUMN parameters TEXT NOT NULL DEFAULT '{}'",
            [],
        )?;
    }
    if !column_exists(c, "jobs", "failed_hosts")? {
        c.execute(
            "ALTER TABLE jobs ADD COLUMN failed_hosts TEXT NOT NULL DEFAULT '[]'",
            [],
        )?;
    }
    c.execute(
        "INSERT INTO template_credentials (template_id, credential_id, position)
         SELECT t.id, t.credential_id, 0
         FROM templates t
         WHERE t.credential_id IS NOT NULL
           AND t.credential_id != ''
           AND EXISTS (SELECT 1 FROM credentials c WHERE c.id = t.credential_id)
           AND NOT EXISTS (
                SELECT 1 FROM template_credentials tc WHERE tc.template_id = t.id
           )",
        [],
    )?;
    if !column_exists(c, "environments", "organization_id")? {
        c.execute(
            "ALTER TABLE environments ADD COLUMN organization_id TEXT",
            [],
        )?;
    }
    if !column_exists(c, "environments", "network")? {
        c.execute(
            "ALTER TABLE environments ADD COLUMN network TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    if !column_exists(c, "environments", "tool")? {
        c.execute(
            "ALTER TABLE environments ADD COLUMN tool TEXT NOT NULL DEFAULT 'any'",
            [],
        )?;
    }
    if !column_exists(c, "templates", "no_environment")? {
        c.execute(
            "ALTER TABLE templates ADD COLUMN no_environment INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
        c.execute(
            "UPDATE templates SET no_environment = 1
             WHERE environment_id IS NULL OR environment_id = ''",
            [],
        )?;
    }
    if !column_exists(c, "templates", "update_on_launch")? {
        c.execute(
            "ALTER TABLE templates ADD COLUMN update_on_launch INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    if !column_exists(c, "projects", "admin_saved")? {
        c.execute(
            "ALTER TABLE projects ADD COLUMN admin_saved INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    if !column_exists(c, "projects", "credential_id")? {
        c.execute("ALTER TABLE projects ADD COLUMN credential_id TEXT", [])?;
    }
    for column in ["last_sync_at", "last_sync_commit", "last_sync_status"] {
        if !column_exists(c, "projects", column)? {
            c.execute(
                &format!("ALTER TABLE projects ADD COLUMN {column} TEXT NOT NULL DEFAULT ''"),
                [],
            )?;
        }
    }
    if !column_exists(c, "templates", "admin_saved")? {
        c.execute(
            "ALTER TABLE templates ADD COLUMN admin_saved INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    if !column_exists(c, "templates", "prompt_on_launch")? {
        c.execute(
            "ALTER TABLE templates ADD COLUMN prompt_on_launch INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !column_exists(c, "inventories", "admin_saved")? {
        c.execute(
            "ALTER TABLE inventories ADD COLUMN admin_saved INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    if !column_exists(c, "workflows", "admin_saved")? {
        c.execute(
            "ALTER TABLE workflows ADD COLUMN admin_saved INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS api_tokens (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  prefix TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL,
  last_used_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_api_tokens_user ON api_tokens(user_id);
CREATE TABLE IF NOT EXISTS schedules (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  target_kind TEXT NOT NULL,
  target_id TEXT NOT NULL,
  timezone TEXT NOT NULL,
  start_date TEXT NOT NULL,
  start_time TEXT NOT NULL,
  frequency TEXT NOT NULL,
  weekdays TEXT NOT NULL DEFAULT '',
  end_kind TEXT NOT NULL,
  end_count INTEGER,
  end_date TEXT,
  enabled INTEGER NOT NULL DEFAULT 1,
  next_run_at TEXT,
  missed_at TEXT,
  occurrence_count INTEGER NOT NULL DEFAULT 0,
  created_by TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (target_kind, target_id, name)
);
CREATE INDEX IF NOT EXISTS idx_schedules_due ON schedules(enabled, next_run_at);
CREATE INDEX IF NOT EXISTS idx_schedules_target ON schedules(target_kind, target_id);",
    )?;
    if !column_exists(c, "schedules", "updated_by")? {
        c.execute(
            "ALTER TABLE schedules ADD COLUMN updated_by TEXT NOT NULL DEFAULT ''",
            [],
        )?;
        c.execute(
            "UPDATE schedules SET updated_by = created_by WHERE updated_by = ''",
            [],
        )?;
    }
    if !column_exists(c, "schedules", "launch_block")? {
        c.execute("ALTER TABLE schedules ADD COLUMN launch_block TEXT", [])?;
    }
    c.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_jobs_team_created ON jobs(team_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_jobs_team_status ON jobs(team_id, status, created_at DESC);",
    )?;
    let sources = {
        let mut stmt = c.prepare("SELECT id, source_json FROM inventory_sources")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (id, raw) in sources {
        let Ok(mut value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let Some(object) = value.as_object_mut() else {
            continue;
        };
        let has_ca = object
            .get("ca_file")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty());
        if !has_ca {
            continue;
        }
        object.remove("ca_file");
        if object.get("validate_certs").and_then(Value::as_bool) == Some(false) {
            object.insert("validate_certs".to_string(), Value::Bool(true));
        }
        c.execute(
            "UPDATE inventory_sources SET source_json = ?2 WHERE id = ?1",
            params![id, value.to_string()],
        )?;
    }
    let legacy_ldap_ca = c
        .query_row(
            "SELECT value FROM settings WHERE key = 'ldap_ca_file'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(ca_file) = legacy_ldap_ca.as_deref() {
        if !ca_file.is_empty() {
            let insecure = c
                .query_row(
                    "SELECT value FROM settings WHERE key = 'ldap_tls_insecure'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .is_some_and(|value| value == "1" || value == "true");
            if insecure {
                c.execute(
                    "UPDATE settings SET value = '0' WHERE key = 'ldap_tls_insecure'",
                    [],
                )?;
            }
        }
        c.execute("DELETE FROM settings WHERE key = 'ldap_ca_file'", [])?;
    }
    Ok(())
}

pub(crate) fn schema_version(c: &Connection) -> AppResult<u32> {
    Ok(c.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

fn check_supported(found: u32) -> AppResult<()> {
    if found > SCHEMA_VERSION {
        return Err(ApiError::Internal(format!(
            "The database uses schema {found}, but this build supports {SCHEMA_VERSION}. Upgrade Koreograf before opening it."
        )));
    }
    Ok(())
}

fn legacy_reconciled(c: &Connection) -> AppResult<bool> {
    let value = c
        .query_row(
            "SELECT value FROM schema_meta WHERE key = ?1",
            [LEGACY_MARKER],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(value.is_some())
}

fn mark_legacy_reconciled(c: &Connection) -> AppResult<()> {
    c.execute(
        "INSERT INTO schema_meta (key, value) VALUES (?1, '1')
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [LEGACY_MARKER],
    )?;
    Ok(())
}

fn apply_sqlite_migrations(c: &Connection, migrations: &[Migration]) -> AppResult<()> {
    let mut found = schema_version(c)?;
    for migration in migrations {
        if migration.version <= found {
            continue;
        }
        let transaction = c.unchecked_transaction()?;
        transaction.execute_batch(migration.sql)?;
        transaction.pragma_update(None, "user_version", migration.version)?;
        transaction.execute(
            "INSERT INTO schema_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![format!("migration:{}", migration.version), migration.name],
        )?;
        transaction.commit()?;
        found = migration.version;
    }
    Ok(())
}

pub(crate) fn ensure_schema(c: &Connection) -> AppResult<()> {
    let found = schema_version(c)?;
    check_supported(found)?;
    let had_tables: i64 = c.query_row(
        "SELECT EXISTS (
            SELECT 1 FROM sqlite_master
            WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
         )",
        [],
        |row| row.get(0),
    )?;
    c.execute_batch(META_TABLE)?;
    if found < BASELINE_VERSION {
        apply_sqlite_migrations(c, &SQLITE_MIGRATIONS[..1])?;
    }
    if found <= BASELINE_VERSION && !legacy_reconciled(c)? {
        if had_tables == 1 {
            let transaction = c.unchecked_transaction()?;
            legacy_migrate(&transaction)?;
            mark_legacy_reconciled(&transaction)?;
            transaction.commit()?;
        } else {
            mark_legacy_reconciled(c)?;
        }
    }
    apply_sqlite_migrations(c, SQLITE_MIGRATIONS)
}

pub(crate) async fn ensure_postgres_schema(client: &tokio_postgres::Client) -> AppResult<()> {
    client.batch_execute(META_TABLE).await?;
    let found: Option<String> = client
        .query_opt(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            &[],
        )
        .await?
        .map(|row| row.get(0));
    let found = found
        .as_deref()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    check_supported(found)?;
    apply_postgres_migrations(client, POSTGRES_MIGRATIONS, found).await?;
    if found <= BASELINE_VERSION && !postgres_legacy_reconciled(client).await? {
        mark_postgres_legacy_reconciled(client).await?;
    }
    Ok(())
}

async fn postgres_legacy_reconciled(client: &tokio_postgres::Client) -> AppResult<bool> {
    let value: Option<String> = client
        .query_opt(
            "SELECT value FROM schema_meta WHERE key = $1",
            &[&LEGACY_MARKER],
        )
        .await?
        .map(|row| row.get(0));
    Ok(value.is_some())
}

async fn mark_postgres_legacy_reconciled(client: &tokio_postgres::Client) -> AppResult<()> {
    client
        .execute(
            "INSERT INTO schema_meta (key, value) VALUES ($1, '1')
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            &[&LEGACY_MARKER],
        )
        .await?;
    Ok(())
}

pub(crate) async fn apply_postgres_migrations(
    client: &tokio_postgres::Client,
    migrations: &[Migration],
    mut found: u32,
) -> AppResult<()> {
    for migration in migrations {
        if migration.version <= found {
            continue;
        }
        client.batch_execute("BEGIN").await?;
        let result = async {
            client.batch_execute(migration.sql).await?;
            client
                .execute(
                    "INSERT INTO schema_meta (key, value) VALUES ('schema_version', $1)
                     ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                    &[&migration.version.to_string()],
                )
                .await?;
            client
                .execute(
                    "INSERT INTO schema_meta (key, value) VALUES ($1, $2)
                     ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                    &[&format!("migration:{}", migration.version), &migration.name],
                )
                .await?;
            Ok::<(), ApiError>(())
        }
        .await;
        match result {
            Ok(()) => {
                client.batch_execute("COMMIT").await?;
                found = migration.version;
            }
            Err(err) => {
                let _ = client.batch_execute("ROLLBACK").await;
                return Err(err);
            }
        }
    }
    Ok(())
}

pub fn open(path: &Path) -> AppResult<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    let found = schema_version(&conn)?;
    check_supported(found)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    ensure_schema(&conn)?;
    Ok(conn)
}

#[derive(Clone, Debug)]
pub struct User {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub password_hash: Option<String>,
    pub auth_source: String,
    pub is_admin: bool,
    pub disabled: bool,
    pub email: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApiToken {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct UserMembership {
    pub team_id: String,
    pub team_name: String,
    pub organization_id: String,
    pub organization_name: String,
    pub role: String,
    pub source: String,
}

#[derive(Clone, Debug)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub description: String,
    pub ldap_group: Option<String>,
    pub organization_id: String,
    pub organization_name: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Member {
    pub user_id: String,
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Project {
    pub id: String,
    pub team_id: String,
    pub organization_id: String,
    pub organization_name: String,
    pub name: String,
    pub source_type: String,
    #[serde(serialize_with = "serialize_source")]
    pub source: String,
    pub revision: String,
    pub credential_id: Option<String>,
    pub created_at: String,
    pub team_ids: Vec<String>,
    pub last_sync_at: String,
    pub last_sync_commit: String,
    pub last_sync_status: String,
    /// True when an administrator last saved this project.
    #[serde(skip)]
    pub admin_saved: bool,
}

fn serialize_source<S>(source: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&crate::runner::mask_git_url(source))
}

#[derive(Clone, Debug, Serialize)]
pub struct Inventory {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub body: String,
    pub created_at: String,
    pub source_kind: String,
    pub source_json: String,
    pub credential_id: Option<String>,
    /// True when an administrator last saved this inventory.
    #[serde(skip)]
    pub admin_saved: bool,
}

#[derive(Clone, Debug)]
pub struct InventorySource {
    pub id: String,
    pub inventory_id: String,
    pub name: String,
    pub kind: String,
    pub source_json: String,
    pub credential_id: Option<String>,
    pub body: String,
    pub position: i64,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct Credential {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub kind: String,
    pub secret_blob: String,
    pub created_at: String,
    pub auto: bool,
    pub disabled: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Environment {
    pub id: String,
    pub team_id: Option<String>,
    pub organization_id: Option<String>,
    pub name: String,
    pub tool: String,
    pub image: String,
    pub pull_policy: String,
    pub network: String,
    pub description: String,
    pub created_at: String,
    pub scope: String,
    pub organization_name: String,
    pub team_name: String,
}

#[derive(Clone, Debug)]
pub struct Template {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub description: String,
    pub tool: String,
    pub project_id: Option<String>,
    pub inventory_id: Option<String>,
    pub credential_id: Option<String>,
    pub environment_id: Option<String>,
    /// True when the template explicitly runs on the controller.
    pub no_environment: bool,
    pub entrypoint: String,
    pub extra_vars: String,
    pub spec: String,
    pub created_at: String,
    pub update_on_launch: bool,
    /// True when an administrator last saved this template.
    pub admin_saved: bool,
    /// When set, the launch dialog may replace inventory, credentials, and Ansible options.
    pub prompt_on_launch: bool,
}

#[derive(Clone, Debug)]
pub struct JobQuery {
    pub limit: i64,
    pub offset: i64,
    pub q: String,
    pub status: String,
    pub tool: String,
    pub trigger: String,
    pub template_id: String,
    pub since: String,
    pub until: String,
}

impl Default for JobQuery {
    fn default() -> Self {
        Self {
            limit: 25,
            offset: 0,
            q: String::new(),
            status: String::new(),
            tool: String::new(),
            trigger: String::new(),
            template_id: String::new(),
            since: String::new(),
            until: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: String,
    pub team_id: String,
    pub template_id: Option<String>,
    pub workflow_job_id: Option<String>,
    pub workflow_node_id: Option<String>,
    pub name: String,
    pub tool: String,
    pub status: String,
    pub extra_vars: String,
    pub launched_by: String,
    pub trigger: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub created_at: String,
    /// Resolved launch parameters. Empty for jobs created before this column.
    pub parameters: String,
    /// JSON `{"hosts":[...],"recap":bool}` or a bare host array.
    pub failed_hosts: String,
}

#[derive(Clone, Debug)]
pub struct JobEvent {
    pub id: i64,
    pub ts: String,
    pub stream: String,
    pub line: String,
}

#[derive(Clone, Debug)]
pub struct Workflow {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub description: String,
    pub graph: String,
    pub created_at: String,
    /// True when an administrator last saved this workflow.
    pub admin_saved: bool,
}

#[derive(Clone, Debug)]
pub struct WorkflowJob {
    pub id: String,
    pub workflow_id: Option<String>,
    pub team_id: String,
    pub name: String,
    pub status: String,
    pub extra_vars: String,
    pub launched_by: String,
    pub trigger: String,
    pub created_at: String,
    pub finished_at: Option<String>,
}

#[derive(Clone, Debug)]
pub struct WorkflowNodeRow {
    pub node_id: String,
    pub phase: String,
    pub job_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Hook {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub token: String,
    pub match_alertname: String,
    pub match_status: String,
    pub template_id: Option<String>,
    pub workflow_id: Option<String>,
    pub extra_vars: String,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Delivery {
    pub id: String,
    pub hook_id: String,
    pub hook_name: String,
    pub status: String,
    pub summary: String,
    pub job_id: Option<String>,
    pub workflow_job_id: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Organization {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: String,
}

pub struct CredentialUsage {
    pub inventory_id: Option<String>,
    pub inventory_name: Option<String>,
    pub source_id: Option<String>,
    pub source_name: Option<String>,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
}

pub fn environment_visible(item: &Environment, team: &Team) -> bool {
    if let Some(owner) = item.team_id.as_deref().filter(|id| !id.is_empty()) {
        return owner == team.id;
    }
    if let Some(organization_id) = item.organization_id.as_deref().filter(|id| !id.is_empty()) {
        return !team.organization_id.is_empty() && organization_id == team.organization_id;
    }
    true
}

#[derive(Clone, Debug)]
pub struct JobActivity {
    pub status: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Clone, Copy)]
pub struct NewDelivery<'a> {
    pub id: &'a str,
    pub hook_id: &'a str,
    pub status: &'a str,
    pub summary: &'a str,
    pub payload: &'a str,
    pub job_id: Option<&'a str>,
    pub workflow_job_id: Option<&'a str>,
}

pub fn json_object(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::Object(Default::default()))
}

#[derive(Clone, Debug)]
pub struct Schedule {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub target_kind: String,
    pub target_id: String,
    pub timezone: String,
    pub start_date: String,
    pub start_time: String,
    pub frequency: String,
    pub weekdays: String,
    pub end_kind: String,
    pub end_count: Option<i64>,
    pub end_date: Option<String>,
    pub enabled: bool,
    pub next_run_at: Option<String>,
    pub missed_at: Option<String>,
    pub occurrence_count: i64,
    pub created_by: String,
    pub updated_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub launch_block: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::environments::{environment, insert_environment, list_environments};
    use crate::store::inventories::inventory;
    use crate::store::jobs::{insert_job, list_jobs};
    use crate::store::projects::{list_projects, project};
    use crate::store::teams::{
        grant_manual_membership, insert_organization, insert_team, list_members, team_by_id,
        upsert_membership,
    };
    use crate::store::workflows::workflow;

    fn sample_job(id: &str, team_id: &str, name: &str, created_at: &str) -> Job {
        Job {
            id: id.to_string(),
            team_id: team_id.to_string(),
            template_id: Some("tpl".into()),
            workflow_job_id: None,
            workflow_node_id: None,
            name: name.to_string(),
            tool: "ansible".into(),
            status: "successful".into(),
            extra_vars: "{}".into(),
            launched_by: "tester".into(),
            trigger: "manual".into(),
            started_at: None,
            finished_at: None,
            created_at: created_at.to_string(),
            parameters: "{}".into(),
            failed_hosts: "[]".into(),
        }
    }

    fn forget_legacy_marker(conn: &Connection) {
        conn.execute("DELETE FROM schema_meta WHERE key = ?1", [LEGACY_MARKER])
            .unwrap();
    }

    fn job_database(prefix: &str) -> (std::path::PathBuf, Connection) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-{prefix}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = open(&dir.join("koreograf.db")).unwrap();
        (dir, conn)
    }

    #[test]
    fn job_list_pages_filters_and_counts_literally() {
        let (dir, conn) = job_database("job-list");
        let tools = ["ansible", "shell", "opentofu"];
        let statuses = ["failed", "successful", "running", "canceled"];
        let triggers = ["manual", "schedule", "hook", "workflow"];
        for index in 0..12 {
            let id = format!("job-{index:02}");
            let name = if index == 5 {
                "Odd 100% _quoted '\"".to_string()
            } else {
                format!("Deploy {index}")
            };
            let created = format!("2026-01-01T00:{:02}:00Z", index + 1);
            let mut job = sample_job(&id, "team", &name, &created);
            job.tool = tools[index % tools.len()].into();
            job.status = statuses[index % statuses.len()].into();
            job.trigger = triggers[index % triggers.len()].into();
            job.template_id = Some(if index % 2 == 0 { "tpl-a" } else { "tpl-b" }.into());
            insert_job(&conn, &job).unwrap();
        }
        let mut foreign = sample_job("foreign", "other-team", "Deploy 99", "2026-01-01T00:59:00Z");
        foreign.status = "failed".into();
        foreign.trigger = "schedule".into();
        foreign.template_id = Some("tpl-a".into());
        insert_job(&conn, &foreign).unwrap();

        let list = |query: JobQuery| list_jobs(&conn, "team", &query).unwrap();
        let ids = |jobs: &[Job]| jobs.iter().map(|job| job.id.clone()).collect::<Vec<_>>();

        let (page, total) = list(JobQuery::default());
        assert_eq!(total, 12);
        assert_eq!(page.len(), 12);
        assert_eq!(page.first().unwrap().id, "job-11");

        let (page, total) = list(JobQuery {
            limit: 3,
            offset: 3,
            ..Default::default()
        });
        assert_eq!(total, 12);
        assert_eq!(ids(&page), vec!["job-08", "job-07", "job-06"]);

        let (page, total) = list(JobQuery {
            status: "failed".into(),
            ..Default::default()
        });
        assert_eq!(total, 3);
        assert!(page.iter().all(|job| job.status == "failed"));

        let (page, total) = list(JobQuery {
            tool: "shell".into(),
            ..Default::default()
        });
        assert_eq!(total, 4);
        assert!(page.iter().all(|job| job.tool == "shell"));

        let (page, total) = list(JobQuery {
            trigger: "schedule".into(),
            ..Default::default()
        });
        assert_eq!(total, 3);
        assert!(page.iter().all(|job| job.trigger == "schedule"));

        let (page, total) = list(JobQuery {
            template_id: "tpl-a".into(),
            ..Default::default()
        });
        assert_eq!(total, 6);
        assert!(page
            .iter()
            .all(|job| job.template_id.as_deref() == Some("tpl-a")));

        let (page, total) = list(JobQuery {
            status: "failed".into(),
            tool: "shell".into(),
            ..Default::default()
        });
        assert_eq!(total, 1);
        assert_eq!(page[0].id, "job-04");

        let (_, total) = list(JobQuery {
            since: "2026-01-01T00:08:00Z".into(),
            ..Default::default()
        });
        assert_eq!(total, 5);
        let (_, total) = list(JobQuery {
            until: "2026-01-01T00:04:00Z".into(),
            ..Default::default()
        });
        assert_eq!(total, 4);
        let (_, total) = list(JobQuery {
            since: "2026-01-01T00:03:00Z".into(),
            until: "2026-01-01T00:06:00Z".into(),
            ..Default::default()
        });
        assert_eq!(total, 4);

        for needle in ["odd", "100%", "_", "'", "\"", "DEPLOY"] {
            let (page, total) = list(JobQuery {
                q: needle.into(),
                ..Default::default()
            });
            let expected: usize = if needle == "DEPLOY" { 11 } else { 1 };
            assert_eq!(total as usize, expected, "{needle}");
            assert_eq!(page.len(), expected, "{needle}");
            assert!(page.iter().all(|job| job.team_id == "team"));
        }

        let (page, total) = list(JobQuery {
            q: "schedule".into(),
            status: "successful".into(),
            ..Default::default()
        });
        assert_eq!(total, 3);
        assert_eq!(page[0].id, "job-09");

        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn job_list_caps_the_limit() {
        let (dir, conn) = job_database("job-limit");
        for index in 0..105 {
            let id = format!("job-{index:03}");
            let created = format!("2026-01-{:02}T{:02}:00:00Z", index / 60 + 1, index % 60);
            let job = sample_job(&id, "team", &format!("Job {index}"), &created);
            insert_job(&conn, &job).unwrap();
        }
        let (page, total) = list_jobs(
            &conn,
            "team",
            &JobQuery {
                limit: 1_000,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(total, 105);
        assert_eq!(page.len(), 100);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_template_keeps_running_without_an_environment() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-template-env-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = open(&dir.join("koreograf.db")).unwrap();
        conn.execute("PRAGMA foreign_keys = OFF", []).unwrap();
        conn.execute("ALTER TABLE templates DROP COLUMN no_environment", [])
            .unwrap();
        conn.execute(
            "INSERT INTO templates (
                id, team_id, name, description, tool, environment_id, entrypoint,
                extra_vars, spec, created_at
             ) VALUES
                ('old', 'team', 'Alt', '', 'shell', NULL, '', '{}', '{}', '2020-01-01T00:00:00Z'),
                ('with-env', 'team', 'Alt mit Umgebung', '', 'shell', 'ee', '', '{}', '{}', '2020-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO environments (id, name, tool, image, pull_policy, created_at)
             VALUES ('ee', 'Alt', 'any', 'quay.io/example/ee:1', 'if_not_present', '2020-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        forget_legacy_marker(&conn);
        ensure_schema(&conn).unwrap();
        let old: i64 = conn
            .query_row(
                "SELECT no_environment FROM templates WHERE id = 'old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let with_env: i64 = conn
            .query_row(
                "SELECT no_environment FROM templates WHERE id = 'with-env'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(old, 1);
        assert_eq!(with_env, 0);
        assert_eq!(environment(&conn, "ee").unwrap().unwrap().tool, "any");
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_template_updates_on_launch() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-template-sync-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = open(&dir.join("koreograf.db")).unwrap();
        conn.execute("PRAGMA foreign_keys = OFF", []).unwrap();
        conn.execute("ALTER TABLE templates DROP COLUMN update_on_launch", [])
            .unwrap();
        conn.execute("ALTER TABLE templates DROP COLUMN admin_saved", [])
            .unwrap();
        conn.execute("ALTER TABLE templates DROP COLUMN prompt_on_launch", [])
            .unwrap();
        conn.execute(
            "INSERT INTO templates (
                id, team_id, name, description, tool, entrypoint, extra_vars, spec, created_at
             ) VALUES ('t1', 'team', 'Alt', '', 'shell', '', '{}', '{}', '2020-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        forget_legacy_marker(&conn);
        ensure_schema(&conn).unwrap();
        let value: i64 = conn
            .query_row(
                "SELECT update_on_launch FROM templates WHERE id = 't1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, 1);
        let saved: i64 = conn
            .query_row(
                "SELECT admin_saved FROM templates WHERE id = 't1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(saved, 1);
        let prompt: i64 = conn
            .query_row(
                "SELECT prompt_on_launch FROM templates WHERE id = 't1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(prompt, 0);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_inventory_and_workflow_stay_released() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-release-migrate-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = open(&dir.join("koreograf.db")).unwrap();
        conn.execute("PRAGMA foreign_keys = OFF", []).unwrap();
        conn.execute("ALTER TABLE inventories DROP COLUMN admin_saved", [])
            .unwrap();
        conn.execute("ALTER TABLE workflows DROP COLUMN admin_saved", [])
            .unwrap();
        conn.execute(
            "INSERT INTO inventories (id, team_id, name, body, created_at)
             VALUES ('i1', 'team', 'Old', '', '2020-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO workflows (id, team_id, name, graph, created_at)
             VALUES ('w1', 'team', 'Old', '{}', '2020-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        forget_legacy_marker(&conn);
        ensure_schema(&conn).unwrap();
        let inventory = inventory(&conn, "i1").unwrap().unwrap();
        let workflow = workflow(&conn, "w1").unwrap().unwrap();
        assert!(inventory.admin_saved);
        assert!(workflow.admin_saved);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migration_drops_legacy_ca_files_and_keeps_verification_on() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-migrate-ca-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join("koreograf.db")).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        conn.execute_batch(SQLITE_MIGRATIONS[0].sql).unwrap();
        conn.execute(
            "INSERT INTO inventory_sources (id, inventory_id, name, kind, source_json, created_at)
             VALUES ('s1', 'i1', 'NetBox', 'netbox', ?1, '2020-01-01T00:00:00Z')",
            [r#"{"api_endpoint":"https://netbox.example","validate_certs":false,"ca_file":"/etc/ca.pem"}"#],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('ldap_ca_file', '/etc/ldap.pem')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('ldap_tls_insecure', '1')",
            [],
        )
        .unwrap();
        ensure_schema(&conn).unwrap();
        let raw: String = conn
            .query_row(
                "SELECT source_json FROM inventory_sources WHERE id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert!(value.get("ca_file").is_none(), "{raw}");
        assert_eq!(value["validate_certs"], Value::Bool(true));
        let insecure: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'ldap_tls_insecure'",
                [],
                |row| row.get(0),
            )
            .ok();
        assert_eq!(insecure.as_deref(), Some("0"));
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('ldap_ca_file', '')",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE settings SET value = '1' WHERE key = 'ldap_tls_insecure'",
            [],
        )
        .unwrap();
        forget_legacy_marker(&conn);
        ensure_schema(&conn).unwrap();
        let insecure: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'ldap_tls_insecure'",
                [],
                |row| row.get(0),
            )
            .ok();
        assert_eq!(
            insecure.as_deref(),
            Some("1"),
            "a blank path keeps the opt-out"
        );
        let ca: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'ldap_ca_file'",
                [],
                |row| row.get(0),
            )
            .ok();
        assert!(ca.is_none());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrates_existing_teams_into_organizations() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-migrate-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("koreograf.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE organizations (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE,
                    description TEXT NOT NULL DEFAULT '',
                    created_at TEXT NOT NULL
                );
                CREATE TABLE teams (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE,
                    description TEXT NOT NULL DEFAULT '',
                    ldap_group TEXT,
                    created_at TEXT NOT NULL
                );
                CREATE TABLE projects (
                    id TEXT PRIMARY KEY,
                    team_id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    source_type TEXT NOT NULL,
                    source TEXT NOT NULL,
                    revision TEXT NOT NULL DEFAULT '',
                    created_at TEXT NOT NULL
                );
                INSERT INTO organizations (id, name, description, created_at)
                    VALUES ('keep-me', 'Alpha', '', '2020-01-01T00:00:00Z');
                INSERT INTO teams (id, name, description, created_at)
                    VALUES ('alpha', 'Alpha', '', '2020-01-01T00:00:00Z');
                INSERT INTO teams (id, name, description, created_at)
                    VALUES ('beta', 'Beta', '', '2020-01-01T00:00:00Z');
                INSERT INTO projects (id, team_id, name, source_type, source, created_at)
                    VALUES ('p1', 'beta', 'Demo', 'path', '/tmp', '2020-01-01T00:00:00Z');",
            )
            .unwrap();
        }
        let conn = open(&path).unwrap();
        let alpha = team_by_id(&conn, "alpha").unwrap().unwrap();
        let beta = team_by_id(&conn, "beta").unwrap().unwrap();
        assert_eq!(alpha.organization_id, "keep-me");
        assert_eq!(alpha.organization_name, "Alpha");
        assert_eq!(beta.organization_id, "org-beta");
        assert_eq!(beta.organization_name, "Beta");
        let project = project(&conn, "p1").unwrap().unwrap();
        assert_eq!(project.organization_id, beta.organization_id);
        assert!(project.admin_saved);
        assert!(list_projects(&conn, "beta")
            .unwrap()
            .iter()
            .any(|item| item.id == "p1"));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manual_grant_leaves_an_ldap_membership() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-member-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = open(&dir.join("koreograf.db")).unwrap();
        insert_organization(&conn, "org", "Plattform", "").unwrap();
        insert_team(&conn, "team", "org", "Plattform", "", None).unwrap();
        crate::store::auth::insert_user(&conn, "user", "ada", "Ada", None, "ldap", false).unwrap();
        upsert_membership(&conn, "team", "user", "operator", "ldap").unwrap();
        let err = grant_manual_membership(&conn, "team", "user", "owner").unwrap_err();
        assert_eq!(
            err.to_string(),
            "the_membership_comes_from_ldap_or_active_directory"
        );
        let members = list_members(&conn, "team").unwrap();
        assert_eq!(members[0].role, "operator");
        assert_eq!(members[0].source, "ldap");
        grant_manual_membership(&conn, "team", "user", "viewer").unwrap_err();
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn sample_environment(
        id: &str,
        team_id: Option<&str>,
        organization_id: Option<&str>,
        name: &str,
    ) -> Environment {
        Environment {
            id: id.to_string(),
            team_id: team_id.map(str::to_string),
            organization_id: organization_id.map(str::to_string),
            name: name.to_string(),
            tool: "any".into(),
            image: "quay.io/example/ee:1".into(),
            pull_policy: "if_not_present".into(),
            network: String::new(),
            description: String::new(),
            created_at: now(),
            scope: String::new(),
            organization_name: String::new(),
            team_name: String::new(),
        }
    }

    #[test]
    fn environment_scopes_stay_with_the_team_that_can_see_them() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-env-scope-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = open(&dir.join("koreograf.db")).unwrap();
        insert_organization(&conn, "org-a", "Alpha", "").unwrap();
        insert_organization(&conn, "org-b", "Beta", "").unwrap();
        insert_team(&conn, "team-a", "org-a", "Alpha", "", None).unwrap();
        insert_team(&conn, "team-b", "org-a", "Betrieb", "", None).unwrap();
        insert_team(&conn, "team-c", "org-b", "Nacht", "", None).unwrap();
        insert_environment(&conn, &sample_environment("global", None, None, "Global")).unwrap();
        insert_environment(
            &conn,
            &sample_environment("org", None, Some("org-a"), "Orga"),
        )
        .unwrap();
        insert_environment(
            &conn,
            &sample_environment("private", Some("team-a"), None, "Privat"),
        )
        .unwrap();
        insert_environment(
            &conn,
            &sample_environment("both", Some("team-a"), Some("org-b"), "Beides"),
        )
        .unwrap();

        let team_a = team_by_id(&conn, "team-a").unwrap().unwrap();
        let team_b = team_by_id(&conn, "team-b").unwrap().unwrap();
        let team_c = team_by_id(&conn, "team-c").unwrap().unwrap();
        let names = |team: &Team| {
            list_environments(&conn, team)
                .unwrap()
                .into_iter()
                .map(|item| item.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&team_a), vec!["Global", "Orga", "Beides", "Privat"]);
        assert_eq!(names(&team_b), vec!["Global", "Orga"]);
        assert_eq!(names(&team_c), vec!["Global"]);

        let both = environment(&conn, "both").unwrap().unwrap();
        assert_eq!(both.scope, "team");
        assert_eq!(both.team_name, "Alpha");
        assert_eq!(both.organization_name, "");
        assert!(environment_visible(&both, &team_a));
        assert!(!environment_visible(&both, &team_b));
        assert!(!environment_visible(&both, &team_c));
        let org = environment(&conn, "org").unwrap().unwrap();
        assert_eq!(org.scope, "organization");
        assert_eq!(org.organization_name, "Alpha");
        assert!(environment_visible(&org, &team_b));
        assert!(!environment_visible(&org, &team_c));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrates_environments_without_an_organization_column() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-env-migrate-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("koreograf.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE environments (
                    id TEXT PRIMARY KEY,
                    team_id TEXT,
                    name TEXT NOT NULL,
                    image TEXT NOT NULL,
                    pull_policy TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    created_at TEXT NOT NULL
                );
                INSERT INTO environments (
                    id, team_id, name, image, pull_policy, description, created_at
                ) VALUES (
                    'ee', 'team-a', 'Alt', 'quay.io/ansible/awx-ee:latest',
                    'if_not_present', '', '2020-01-01T00:00:00Z'
                );",
            )
            .unwrap();
        }
        let conn = open(&path).unwrap();
        let item = environment(&conn, "ee").unwrap().unwrap();
        assert_eq!(item.scope, "team");
        assert_eq!(item.team_id.as_deref(), Some("team-a"));
        assert!(item.organization_id.is_none());
        assert_eq!(item.network, "");
        assert_eq!(item.tool, "any");
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_migrations_end_at_the_supported_version() {
        assert_eq!(SQLITE_MIGRATIONS.last().unwrap().version, SCHEMA_VERSION);
        assert_eq!(POSTGRES_MIGRATIONS.last().unwrap().version, SCHEMA_VERSION);
    }

    #[test]
    fn a_failing_migration_keeps_the_sqlite_version() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        let migrations = [Migration {
            version: SCHEMA_VERSION + 1,
            name: "9999_broken",
            sql: "CREATE TABLE broken (id TEXT",
        }];
        assert!(apply_sqlite_migrations(&conn, &migrations).is_err());
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'broken'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
    }

    #[test]
    fn a_legacy_database_is_reconciled_once() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        assert!(legacy_reconciled(&conn).unwrap());
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('ldap_ca_file', '/keep')",
            [],
        )
        .unwrap();
        ensure_schema(&conn).unwrap();
        let kept: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'ldap_ca_file'",
                [],
                |row| row.get(0),
            )
            .ok();
        assert_eq!(kept.as_deref(), Some("/keep"));
        forget_legacy_marker(&conn);
        ensure_schema(&conn).unwrap();
        let removed: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'ldap_ca_file'",
                [],
                |row| row.get(0),
            )
            .ok();
        assert!(removed.is_none());
    }
}
