use std::fs;
use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

use crate::error::{ApiError, AppResult};

pub fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn plus_secs(secs: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::seconds(secs))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  username TEXT NOT NULL UNIQUE COLLATE NOCASE,
  display_name TEXT NOT NULL,
  password_hash TEXT,
  auth_source TEXT NOT NULL,
  is_admin INTEGER NOT NULL DEFAULT 0,
  disabled INTEGER NOT NULL DEFAULT 0,
  email TEXT,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
  token TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS teams (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL DEFAULT '',
  ldap_group TEXT,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS memberships (
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role TEXT NOT NULL,
  source TEXT NOT NULL,
  PRIMARY KEY (team_id, user_id)
);
CREATE TABLE IF NOT EXISTS projects (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  source_type TEXT NOT NULL,
  source TEXT NOT NULL,
  revision TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  UNIQUE (team_id, name)
);
CREATE TABLE IF NOT EXISTS inventories (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  body TEXT NOT NULL,
  created_at TEXT NOT NULL,
  source_kind TEXT NOT NULL DEFAULT 'ini',
  source_json TEXT NOT NULL DEFAULT '{}',
  credential_id TEXT,
  UNIQUE (team_id, name)
);
CREATE TABLE IF NOT EXISTS inventory_sources (
  id TEXT PRIMARY KEY,
  inventory_id TEXT NOT NULL REFERENCES inventories(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  source_json TEXT NOT NULL DEFAULT '{}',
  credential_id TEXT,
  body TEXT NOT NULL DEFAULT '',
  position INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS credentials (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  secret_blob TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE (team_id, name)
);
CREATE TABLE IF NOT EXISTS environments (
  id TEXT PRIMARY KEY,
  team_id TEXT REFERENCES teams(id) ON DELETE CASCADE,
  organization_id TEXT,
  name TEXT NOT NULL,
  image TEXT NOT NULL,
  pull_policy TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS templates (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  tool TEXT NOT NULL,
  project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
  inventory_id TEXT REFERENCES inventories(id) ON DELETE SET NULL,
  credential_id TEXT REFERENCES credentials(id) ON DELETE SET NULL,
  environment_id TEXT REFERENCES environments(id) ON DELETE SET NULL,
  entrypoint TEXT NOT NULL DEFAULT '',
  extra_vars TEXT NOT NULL DEFAULT '{}',
  spec TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  UNIQUE (team_id, name)
);
CREATE TABLE IF NOT EXISTS template_credentials (
  template_id TEXT NOT NULL REFERENCES templates(id) ON DELETE CASCADE,
  credential_id TEXT NOT NULL REFERENCES credentials(id) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  PRIMARY KEY (template_id, credential_id)
);
CREATE TABLE IF NOT EXISTS workflow_jobs (
  id TEXT PRIMARY KEY,
  workflow_id TEXT,
  team_id TEXT NOT NULL,
  name TEXT NOT NULL,
  status TEXT NOT NULL,
  extra_vars TEXT NOT NULL DEFAULT '{}',
  launched_by TEXT NOT NULL,
  trigger TEXT NOT NULL,
  created_at TEXT NOT NULL,
  finished_at TEXT
);
CREATE TABLE IF NOT EXISTS jobs (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL,
  template_id TEXT,
  workflow_job_id TEXT,
  workflow_node_id TEXT,
  name TEXT NOT NULL,
  tool TEXT NOT NULL,
  status TEXT NOT NULL,
  extra_vars TEXT NOT NULL DEFAULT '{}',
  launched_by TEXT NOT NULL,
  trigger TEXT NOT NULL,
  started_at TEXT,
  finished_at TEXT,
  created_at TEXT NOT NULL,
  parameters TEXT NOT NULL DEFAULT '{}',
  failed_hosts TEXT NOT NULL DEFAULT '[]'
);
CREATE TABLE IF NOT EXISTS job_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  job_id TEXT NOT NULL,
  ts TEXT NOT NULL,
  stream TEXT NOT NULL,
  line TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS workflows (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  graph TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE (team_id, name)
);
CREATE TABLE IF NOT EXISTS workflow_nodes (
  workflow_job_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  phase TEXT NOT NULL,
  job_id TEXT,
  PRIMARY KEY (workflow_job_id, node_id)
);
CREATE TABLE IF NOT EXISTS hooks (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  token TEXT NOT NULL UNIQUE,
  match_alertname TEXT NOT NULL,
  match_status TEXT NOT NULL DEFAULT 'firing',
  template_id TEXT,
  workflow_id TEXT,
  extra_vars TEXT NOT NULL DEFAULT '{}',
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL,
  UNIQUE (team_id, name)
);
CREATE TABLE IF NOT EXISTS hook_deliveries (
  id TEXT PRIMARY KEY,
  hook_id TEXT NOT NULL,
  status TEXT NOT NULL,
  summary TEXT NOT NULL,
  payload TEXT NOT NULL,
  job_id TEXT,
  workflow_job_id TEXT,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS organizations (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS project_teams (
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  PRIMARY KEY (project_id, team_id)
);
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_jobs_team ON jobs(team_id, created_at);
CREATE INDEX IF NOT EXISTS idx_events_job ON job_events(job_id, id);
CREATE INDEX IF NOT EXISTS idx_members_user ON memberships(user_id);
"#;

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

fn migrate(c: &Connection) -> AppResult<()> {
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
    Ok(())
}

pub(crate) fn ensure_schema(c: &Connection) -> AppResult<()> {
    migrate(c)
}

pub fn open(path: &Path) -> AppResult<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(SCHEMA)?;
    migrate(&conn)?;
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
    pub source: String,
    pub revision: String,
    pub created_at: String,
    pub team_ids: Vec<String>,
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
}

#[derive(Clone, Debug, Serialize)]
pub struct Environment {
    pub id: String,
    pub team_id: Option<String>,
    pub organization_id: Option<String>,
    pub name: String,
    pub image: String,
    pub pull_policy: String,
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
    pub entrypoint: String,
    pub extra_vars: String,
    pub spec: String,
    pub created_at: String,
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

fn flag(value: i64) -> bool {
    value != 0
}

pub fn insert_user(
    c: &Connection,
    id: &str,
    username: &str,
    display_name: &str,
    password_hash: Option<&str>,
    auth_source: &str,
    is_admin: bool,
) -> AppResult<()> {
    c.execute(
        "INSERT INTO users (id, username, display_name, password_hash, auth_source, is_admin, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, username, display_name, password_hash, auth_source, is_admin as i64, now()],
    )?;
    Ok(())
}

const USER_SELECT: &str =
    "id, username, display_name, password_hash, auth_source, is_admin, disabled, created_at, email";

pub fn user_by_username(c: &Connection, username: &str) -> AppResult<Option<User>> {
    c.query_row(
        &format!("SELECT {USER_SELECT} FROM users WHERE username = ?1 COLLATE NOCASE"),
        [username],
        map_user,
    )
    .optional()
    .map_err(Into::into)
}

pub fn user_by_id(c: &Connection, id: &str) -> AppResult<Option<User>> {
    c.query_row(
        &format!("SELECT {USER_SELECT} FROM users WHERE id = ?1"),
        [id],
        map_user,
    )
    .optional()
    .map_err(Into::into)
}

pub fn list_users(c: &Connection) -> AppResult<Vec<(User, i64)>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {USER_SELECT},
                (SELECT COUNT(*) FROM memberships m WHERE m.user_id = users.id)
         FROM users ORDER BY username COLLATE NOCASE"
    ))?;
    let rows = stmt.query_map([], |row| Ok((map_user(row)?, row.get::<_, i64>(9)?)))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn list_user_memberships(c: &Connection, user_id: &str) -> AppResult<Vec<UserMembership>> {
    let mut stmt = c.prepare(
        "SELECT t.id, t.name, COALESCE(t.organization_id, ''), COALESCE(o.name, ''), m.role, m.source
         FROM memberships m
         JOIN teams t ON t.id = m.team_id
         LEFT JOIN organizations o ON o.id = t.organization_id
         WHERE m.user_id = ?1
         ORDER BY o.name, t.name",
    )?;
    let rows = stmt.query_map([user_id], |row| {
        Ok(UserMembership {
            team_id: row.get(0)?,
            team_name: row.get(1)?,
            organization_id: row.get(2)?,
            organization_name: row.get(3)?,
            role: row.get(4)?,
            source: row.get(5)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn update_profile(
    c: &Connection,
    id: &str,
    display_name: &str,
    email: Option<&str>,
    password_hash: Option<&str>,
) -> AppResult<()> {
    if let Some(password_hash) = password_hash {
        c.execute(
            "UPDATE users SET display_name = ?2, email = ?3, password_hash = ?4 WHERE id = ?1",
            params![id, display_name, email, password_hash],
        )?;
    } else {
        c.execute(
            "UPDATE users SET display_name = ?2, email = ?3 WHERE id = ?1",
            params![id, display_name, email],
        )?;
    }
    Ok(())
}

pub fn set_user_email(c: &Connection, id: &str, email: Option<&str>) -> AppResult<()> {
    c.execute(
        "UPDATE users SET email = ?2 WHERE id = ?1",
        params![id, email],
    )?;
    Ok(())
}

pub fn update_user(
    c: &Connection,
    id: &str,
    display_name: &str,
    is_admin: bool,
    disabled: bool,
    password_hash: Option<&str>,
) -> AppResult<()> {
    if let Some(password_hash) = password_hash {
        c.execute(
            "UPDATE users SET display_name = ?2, is_admin = ?3, disabled = ?4, password_hash = ?5 WHERE id = ?1",
            params![id, display_name, is_admin as i64, disabled as i64, password_hash],
        )?;
    } else {
        c.execute(
            "UPDATE users SET display_name = ?2, is_admin = ?3, disabled = ?4 WHERE id = ?1",
            params![id, display_name, is_admin as i64, disabled as i64],
        )?;
    }
    Ok(())
}

pub fn delete_user_sessions(c: &Connection, user_id: &str) -> AppResult<()> {
    c.execute("DELETE FROM sessions WHERE user_id = ?1", [user_id])?;
    Ok(())
}

pub fn delete_user(c: &Connection, id: &str) -> AppResult<bool> {
    Ok(c.execute("DELETE FROM users WHERE id = ?1", [id])? > 0)
}

pub fn other_active_admins(c: &Connection, user_id: &str) -> AppResult<i64> {
    c.query_row(
        "SELECT COUNT(*) FROM users WHERE is_admin != 0 AND disabled = 0 AND id != ?1",
        [user_id],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

pub fn membership_source(
    c: &Connection,
    team_id: &str,
    user_id: &str,
) -> AppResult<Option<String>> {
    c.query_row(
        "SELECT source FROM memberships WHERE team_id = ?1 AND user_id = ?2",
        params![team_id, user_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub fn grant_manual_membership(
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

fn map_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        username: row.get(1)?,
        display_name: row.get(2)?,
        password_hash: row.get(3)?,
        auth_source: row.get(4)?,
        is_admin: flag(row.get(5)?),
        disabled: flag(row.get(6)?),
        created_at: row.get(7)?,
        email: row.get(8)?,
    })
}

pub fn create_session(c: &Connection, token: &str, user_id: &str) -> AppResult<()> {
    c.execute(
        "INSERT INTO sessions (token, user_id, expires_at) VALUES (?1, ?2, ?3)",
        params![token, user_id, plus_secs(7 * 24 * 3600)],
    )?;
    Ok(())
}

pub fn delete_session(c: &Connection, token: &str) -> AppResult<()> {
    c.execute("DELETE FROM sessions WHERE token = ?1", [token])?;
    Ok(())
}

pub fn purge_sessions(c: &Connection) -> AppResult<()> {
    c.execute("DELETE FROM sessions WHERE expires_at < ?1", [now()])?;
    Ok(())
}

pub fn user_by_session(c: &Connection, token: &str) -> AppResult<Option<User>> {
    let found = c
        .query_row(
            "SELECT u.id, u.username, u.display_name, u.password_hash, u.auth_source, u.is_admin, u.disabled, u.created_at, u.email, s.expires_at
             FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token = ?1",
            [token],
            |row| {
                let expires: String = row.get(9)?;
                Ok((map_user(row)?, expires))
            },
        )
        .optional()?;
    let Some((user, expires)) = found else {
        return Ok(None);
    };
    if expires < now() {
        delete_session(c, token)?;
        return Ok(None);
    }
    Ok(Some(user))
}

pub fn insert_team(
    c: &Connection,
    id: &str,
    organization_id: &str,
    name: &str,
    description: &str,
    ldap_group: Option<&str>,
) -> AppResult<()> {
    c.execute(
        "INSERT INTO teams (id, organization_id, name, description, ldap_group, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, organization_id, name, description, ldap_group, now()],
    )?;
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct Organization {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: String,
}

pub fn insert_organization(
    c: &Connection,
    id: &str,
    name: &str,
    description: &str,
) -> AppResult<()> {
    c.execute(
        "INSERT INTO organizations (id, name, description, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![id, name, description, now()],
    )?;
    Ok(())
}

pub fn update_organization(
    c: &Connection,
    id: &str,
    name: &str,
    description: &str,
) -> AppResult<()> {
    let n = c.execute(
        "UPDATE organizations SET name = ?2, description = ?3 WHERE id = ?1",
        params![id, name, description],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub fn organization(c: &Connection, id: &str) -> AppResult<Option<Organization>> {
    c.query_row(
        "SELECT id, name, description, created_at FROM organizations WHERE id = ?1",
        [id],
        map_organization,
    )
    .optional()
    .map_err(Into::into)
}

pub fn organization_by_name(c: &Connection, name: &str) -> AppResult<Option<Organization>> {
    c.query_row(
        "SELECT id, name, description, created_at FROM organizations WHERE name = ?1",
        [name],
        map_organization,
    )
    .optional()
    .map_err(Into::into)
}

pub fn list_organizations(
    c: &Connection,
    user_id: &str,
    admin: bool,
) -> AppResult<Vec<Organization>> {
    if admin {
        let mut stmt =
            c.prepare("SELECT id, name, description, created_at FROM organizations ORDER BY name")?;
        let rows = stmt.query_map([], map_organization)?;
        return rows.collect::<Result<Vec<_>, _>>().map_err(Into::into);
    }
    let mut stmt = c.prepare(
        "SELECT DISTINCT o.id, o.name, o.description, o.created_at
         FROM organizations o
         JOIN teams t ON t.organization_id = o.id
         JOIN memberships m ON m.team_id = t.id
         WHERE m.user_id = ?1
         ORDER BY o.name",
    )?;
    let rows = stmt.query_map([user_id], map_organization)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn teams_in_organization(c: &Connection, organization_id: &str) -> AppResult<Vec<Team>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {TEAM_SELECT} FROM teams t
         LEFT JOIN organizations o ON o.id = t.organization_id
         WHERE t.organization_id = ?1 ORDER BY t.name"
    ))?;
    let rows = stmt.query_map([organization_id], map_team)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn set_team_organization(
    c: &Connection,
    team_id: &str,
    organization_id: &str,
) -> AppResult<()> {
    let n = c.execute(
        "UPDATE teams SET organization_id = ?2 WHERE id = ?1",
        params![team_id, organization_id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

fn map_organization(row: &rusqlite::Row<'_>) -> rusqlite::Result<Organization> {
    Ok(Organization {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        created_at: row.get(3)?,
    })
}

pub fn update_team(
    c: &Connection,
    id: &str,
    name: &str,
    description: &str,
    ldap_group: Option<&str>,
) -> AppResult<()> {
    let n = c.execute(
        "UPDATE teams SET name = ?2, description = ?3, ldap_group = ?4 WHERE id = ?1",
        params![id, name, description, ldap_group],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

const TEAM_SELECT: &str = "t.id, t.name, t.description, t.ldap_group, t.created_at, \
     COALESCE(t.organization_id, ''), COALESCE(o.name, '')";

pub fn team_by_id(c: &Connection, id: &str) -> AppResult<Option<Team>> {
    c.query_row(
        &format!(
            "SELECT {TEAM_SELECT} FROM teams t
             LEFT JOIN organizations o ON o.id = t.organization_id WHERE t.id = ?1"
        ),
        [id],
        map_team,
    )
    .optional()
    .map_err(Into::into)
}

pub fn count_teams(c: &Connection) -> AppResult<i64> {
    c.query_row("SELECT COUNT(*) FROM teams", [], |row| row.get(0))
        .map_err(Into::into)
}

fn map_team(row: &rusqlite::Row<'_>) -> rusqlite::Result<Team> {
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

pub fn list_teams_for_user(
    c: &Connection,
    user_id: &str,
    admin: bool,
) -> AppResult<Vec<(Team, String)>> {
    let sql = if admin {
        format!(
            "SELECT {TEAM_SELECT},
                    COALESCE((SELECT role FROM memberships m WHERE m.team_id = t.id AND m.user_id = ?1), 'owner')
             FROM teams t LEFT JOIN organizations o ON o.id = t.organization_id ORDER BY t.name"
        )
    } else {
        format!(
            "SELECT {TEAM_SELECT}, m.role
             FROM teams t JOIN memberships m ON m.team_id = t.id
             LEFT JOIN organizations o ON o.id = t.organization_id
             WHERE m.user_id = ?1 ORDER BY t.name"
        )
    };
    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map([user_id], |row| {
        Ok((map_team(row)?, row.get::<_, String>(7)?))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn teams_with_ldap(c: &Connection) -> AppResult<Vec<Team>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {TEAM_SELECT} FROM teams t
         LEFT JOIN organizations o ON o.id = t.organization_id
         WHERE t.ldap_group IS NOT NULL AND t.ldap_group != ''"
    ))?;
    let rows = stmt.query_map([], map_team)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn upsert_membership(
    c: &Connection,
    team_id: &str,
    user_id: &str,
    role: &str,
    source: &str,
) -> AppResult<()> {
    c.execute(
        "INSERT INTO memberships (team_id, user_id, role, source) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(team_id, user_id) DO UPDATE SET role = excluded.role, source = excluded.source
         WHERE memberships.source = 'ldap' OR excluded.source = 'manual'",
        params![team_id, user_id, role, source],
    )?;
    Ok(())
}

pub fn delete_ldap_membership(c: &Connection, team_id: &str, user_id: &str) -> AppResult<()> {
    c.execute(
        "DELETE FROM memberships WHERE team_id = ?1 AND user_id = ?2 AND source = 'ldap'",
        params![team_id, user_id],
    )?;
    Ok(())
}

pub fn delete_membership(c: &Connection, team_id: &str, user_id: &str) -> AppResult<()> {
    c.execute(
        "DELETE FROM memberships WHERE team_id = ?1 AND user_id = ?2",
        params![team_id, user_id],
    )?;
    Ok(())
}

pub fn role_of(c: &Connection, team_id: &str, user_id: &str) -> AppResult<Option<String>> {
    c.query_row(
        "SELECT role FROM memberships WHERE team_id = ?1 AND user_id = ?2",
        params![team_id, user_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub fn list_members(c: &Connection, team_id: &str) -> AppResult<Vec<Member>> {
    let mut stmt = c.prepare(
        "SELECT u.id, u.username, u.display_name, m.role, m.source
         FROM memberships m JOIN users u ON u.id = m.user_id
         WHERE m.team_id = ?1 ORDER BY u.username",
    )?;
    let rows = stmt.query_map([team_id], |row| {
        Ok(Member {
            user_id: row.get(0)?,
            username: row.get(1)?,
            display_name: row.get(2)?,
            role: row.get(3)?,
            source: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

const PROJECT_SELECT: &str =
    "p.id, p.team_id, COALESCE(p.organization_id, ''), COALESCE(o.name, ''), \
     p.name, p.source_type, p.source, p.revision, p.created_at";

pub fn insert_project(c: &Connection, project: &Project) -> AppResult<()> {
    c.execute(
        "INSERT INTO projects (
            id, team_id, organization_id, name, source_type, source, revision, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            project.id,
            project.team_id,
            project.organization_id,
            project.name,
            project.source_type,
            project.source,
            project.revision,
            project.created_at
        ],
    )?;
    replace_project_teams(c, &project.id, &project.team_ids)?;
    Ok(())
}

pub fn list_projects(c: &Connection, team_id: &str) -> AppResult<Vec<Project>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {PROJECT_SELECT}
         FROM projects p
         JOIN teams t ON t.id = ?1
         LEFT JOIN organizations o ON o.id = p.organization_id
         WHERE p.organization_id = t.organization_id
           AND (
             NOT EXISTS (SELECT 1 FROM project_teams pt WHERE pt.project_id = p.id)
             OR EXISTS (
               SELECT 1 FROM project_teams pt WHERE pt.project_id = p.id AND pt.team_id = t.id
             )
           )
         ORDER BY p.name"
    ))?;
    let rows = stmt.query_map([team_id], map_project)?;
    let mut projects = rows.collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    for project in &mut projects {
        project.team_ids = project_team_ids(c, &project.id)?;
    }
    Ok(projects)
}

pub fn project(c: &Connection, id: &str) -> AppResult<Option<Project>> {
    let mut found = c
        .query_row(
            &format!(
                "SELECT {PROJECT_SELECT} FROM projects p
                 LEFT JOIN organizations o ON o.id = p.organization_id WHERE p.id = ?1"
            ),
            [id],
            map_project,
        )
        .optional()?;
    if let Some(project) = &mut found {
        project.team_ids = project_team_ids(c, &project.id)?;
    }
    Ok(found)
}

pub fn project_visible(c: &Connection, id: &str, team_id: &str) -> AppResult<bool> {
    let found: Option<i64> = c
        .query_row(
            "SELECT 1 FROM projects p
             JOIN teams t ON t.id = ?2
             WHERE p.id = ?1 AND p.organization_id = t.organization_id
               AND (
                 NOT EXISTS (SELECT 1 FROM project_teams pt WHERE pt.project_id = p.id)
                 OR EXISTS (
                   SELECT 1 FROM project_teams pt WHERE pt.project_id = p.id AND pt.team_id = t.id
                 )
               )",
            params![id, team_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

pub fn project_team_ids(c: &Connection, project_id: &str) -> AppResult<Vec<String>> {
    let mut stmt =
        c.prepare("SELECT team_id FROM project_teams WHERE project_id = ?1 ORDER BY team_id")?;
    let rows = stmt.query_map([project_id], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn replace_project_teams(
    c: &Connection,
    project_id: &str,
    team_ids: &[String],
) -> AppResult<()> {
    // SQL savepoint: `seed` already holds a transaction, and `Connection::savepoint` needs `&mut`.
    c.execute("SAVEPOINT koreograf_project_teams", [])?;
    let result = write_project_teams(c, project_id, team_ids);
    if result.is_ok() {
        c.execute("RELEASE koreograf_project_teams", [])?;
    } else {
        let _ = c.execute("ROLLBACK TO koreograf_project_teams", []);
        let _ = c.execute("RELEASE koreograf_project_teams", []);
    }
    result
}

fn write_project_teams(c: &Connection, project_id: &str, team_ids: &[String]) -> AppResult<()> {
    c.execute(
        "DELETE FROM project_teams WHERE project_id = ?1",
        [project_id],
    )?;
    let mut seen = std::collections::BTreeSet::new();
    for team_id in team_ids {
        if !seen.insert(team_id.as_str()) {
            continue;
        }
        c.execute(
            "INSERT INTO project_teams (project_id, team_id) VALUES (?1, ?2)",
            params![project_id, team_id],
        )?;
    }
    Ok(())
}

pub fn set_project_organization(c: &Connection, id: &str, organization_id: &str) -> AppResult<()> {
    let n = c.execute(
        "UPDATE projects SET organization_id = ?2 WHERE id = ?1",
        params![id, organization_id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

fn map_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        team_id: row.get(1)?,
        organization_id: row.get(2)?,
        organization_name: row.get(3)?,
        name: row.get(4)?,
        source_type: row.get(5)?,
        source: row.get(6)?,
        revision: row.get(7)?,
        created_at: row.get(8)?,
        team_ids: Vec::new(),
    })
}

pub fn delete_project(c: &Connection, id: &str, team_id: &str) -> AppResult<()> {
    if !project_visible(c, id, team_id)? {
        return Err(ApiError::NotFound);
    }
    let n = c.execute("DELETE FROM projects WHERE id = ?1", [id])?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub fn insert_inventory(c: &Connection, item: &Inventory) -> AppResult<()> {
    c.execute(
        "INSERT INTO inventories (
            id, team_id, name, body, created_at, source_kind, source_json, credential_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            item.id,
            item.team_id,
            item.name,
            item.body,
            item.created_at,
            item.source_kind,
            item.source_json,
            item.credential_id
        ],
    )?;
    Ok(())
}

pub fn update_inventory(
    c: &Connection,
    id: &str,
    team_id: &str,
    name: &str,
    body: &str,
) -> AppResult<()> {
    let n = c.execute(
        "UPDATE inventories SET name = ?3, body = ?4 WHERE id = ?1 AND team_id = ?2",
        params![id, team_id, name, body],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

const INVENTORY_SELECT: &str =
    "id, team_id, name, body, created_at, source_kind, source_json, credential_id";

pub fn list_inventories(c: &Connection, team_id: &str) -> AppResult<Vec<Inventory>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {INVENTORY_SELECT} FROM inventories WHERE team_id = ?1 ORDER BY name"
    ))?;
    let rows = stmt.query_map([team_id], map_inventory)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn inventory(c: &Connection, id: &str) -> AppResult<Option<Inventory>> {
    c.query_row(
        &format!("SELECT {INVENTORY_SELECT} FROM inventories WHERE id = ?1"),
        [id],
        map_inventory,
    )
    .optional()
    .map_err(Into::into)
}

pub fn set_inventory_body(c: &Connection, id: &str, team_id: &str, body: &str) -> AppResult<()> {
    let n = c.execute(
        "UPDATE inventories SET body = ?3 WHERE id = ?1 AND team_id = ?2",
        params![id, team_id, body],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

fn map_inventory(row: &rusqlite::Row<'_>) -> rusqlite::Result<Inventory> {
    Ok(Inventory {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        body: row.get(3)?,
        created_at: row.get(4)?,
        source_kind: row.get(5)?,
        source_json: row.get(6)?,
        credential_id: row.get(7)?,
    })
}

pub fn insert_inventory_source(c: &Connection, item: &InventorySource) -> AppResult<()> {
    c.execute(
        "INSERT INTO inventory_sources (
            id, inventory_id, name, kind, source_json, credential_id, body, position, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            item.id,
            item.inventory_id,
            item.name,
            item.kind,
            item.source_json,
            item.credential_id,
            item.body,
            item.position,
            item.created_at
        ],
    )?;
    Ok(())
}

pub fn list_inventory_sources(
    c: &Connection,
    inventory_id: &str,
) -> AppResult<Vec<InventorySource>> {
    let mut stmt = c.prepare(
        "SELECT id, inventory_id, name, kind, source_json, credential_id, body, position, created_at
         FROM inventory_sources WHERE inventory_id = ?1 ORDER BY position, created_at",
    )?;
    let rows = stmt.query_map([inventory_id], |row| {
        Ok(InventorySource {
            id: row.get(0)?,
            inventory_id: row.get(1)?,
            name: row.get(2)?,
            kind: row.get(3)?,
            source_json: row.get(4)?,
            credential_id: row.get(5)?,
            body: row.get(6)?,
            position: row.get(7)?,
            created_at: row.get(8)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn inventory_source(c: &Connection, id: &str) -> AppResult<Option<InventorySource>> {
    c.query_row(
        "SELECT id, inventory_id, name, kind, source_json, credential_id, body, position, created_at
         FROM inventory_sources WHERE id = ?1",
        [id],
        |row| {
            Ok(InventorySource {
                id: row.get(0)?,
                inventory_id: row.get(1)?,
                name: row.get(2)?,
                kind: row.get(3)?,
                source_json: row.get(4)?,
                credential_id: row.get(5)?,
                body: row.get(6)?,
                position: row.get(7)?,
                created_at: row.get(8)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub fn set_inventory_source_body(c: &Connection, id: &str, body: &str) -> AppResult<()> {
    c.execute(
        "UPDATE inventory_sources SET body = ?2 WHERE id = ?1",
        params![id, body],
    )?;
    Ok(())
}

pub fn rename_inventory_source(c: &Connection, id: &str, name: &str) -> AppResult<()> {
    c.execute(
        "UPDATE inventory_sources SET name = ?2 WHERE id = ?1",
        params![id, name],
    )?;
    Ok(())
}

pub fn delete_inventory_source(c: &Connection, id: &str, inventory_id: &str) -> AppResult<()> {
    let n = c.execute(
        "DELETE FROM inventory_sources WHERE id = ?1 AND inventory_id = ?2",
        params![id, inventory_id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub fn refresh_inventory_summary(c: &Connection, inventory_id: &str) -> AppResult<()> {
    let sources = list_inventory_sources(c, inventory_id)?;
    let (kind, source_json, credential_id, body) = match sources.as_slice() {
        [] => ("ini".to_string(), "{}".to_string(), None, String::new()),
        [one] => (
            one.kind.clone(),
            one.source_json.clone(),
            one.credential_id.clone(),
            one.body.clone(),
        ),
        many => {
            let body = many
                .iter()
                .map(|source| source.body.trim_end_matches(['\n', '\r']))
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            ("mixed".to_string(), "{}".to_string(), None, body)
        }
    };
    c.execute(
        "UPDATE inventories
         SET body = ?2, source_kind = ?3, source_json = ?4, credential_id = ?5
         WHERE id = ?1",
        params![inventory_id, body, kind, source_json, credential_id],
    )?;
    Ok(())
}

pub fn delete_inventory(c: &Connection, id: &str, team_id: &str) -> AppResult<()> {
    let n = c.execute(
        "DELETE FROM inventories WHERE id = ?1 AND team_id = ?2",
        params![id, team_id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub fn insert_credential(c: &Connection, item: &Credential) -> AppResult<()> {
    c.execute(
        "INSERT INTO credentials (id, team_id, name, kind, secret_blob, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            item.id,
            item.team_id,
            item.name,
            item.kind,
            item.secret_blob,
            item.created_at
        ],
    )?;
    Ok(())
}

pub fn list_credentials(c: &Connection, team_id: &str) -> AppResult<Vec<Credential>> {
    let mut stmt = c.prepare(
        "SELECT id, team_id, name, kind, '' AS secret_blob, created_at
         FROM credentials WHERE team_id = ?1 ORDER BY name",
    )?;
    let rows = stmt.query_map([team_id], map_credential)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn credential(c: &Connection, id: &str) -> AppResult<Option<Credential>> {
    c.query_row(
        "SELECT id, team_id, name, kind, secret_blob, created_at FROM credentials WHERE id = ?1",
        [id],
        map_credential,
    )
    .optional()
    .map_err(Into::into)
}

fn map_credential(row: &rusqlite::Row<'_>) -> rusqlite::Result<Credential> {
    Ok(Credential {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        kind: row.get(3)?,
        secret_blob: row.get(4)?,
        created_at: row.get(5)?,
    })
}

pub fn update_credential(c: &Connection, item: &Credential) -> AppResult<()> {
    let n = c.execute(
        "UPDATE credentials SET name = ?3, secret_blob = ?4 WHERE id = ?1 AND team_id = ?2",
        params![item.id, item.team_id, item.name, item.secret_blob],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub fn delete_credential(c: &Connection, id: &str, team_id: &str) -> AppResult<()> {
    let n = c.execute(
        "DELETE FROM credentials WHERE id = ?1 AND team_id = ?2",
        params![id, team_id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub fn insert_environment(c: &Connection, item: &Environment) -> AppResult<()> {
    c.execute(
        "INSERT INTO environments (
            id, team_id, organization_id, name, image, pull_policy, description, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            item.id,
            item.team_id,
            item.organization_id,
            item.name,
            item.image,
            item.pull_policy,
            item.description,
            item.created_at
        ],
    )?;
    Ok(())
}

const ENVIRONMENT_SELECT: &str = "\
SELECT e.id, e.team_id, e.organization_id, e.name, e.image, e.pull_policy, \
       e.description, e.created_at, COALESCE(o.name, ''), COALESCE(t.name, '') \
FROM environments e \
LEFT JOIN organizations o ON o.id = e.organization_id \
LEFT JOIN teams t ON t.id = e.team_id";

pub fn list_environments(c: &Connection, team: &Team) -> AppResult<Vec<Environment>> {
    let mut stmt = c.prepare(&format!(
        "{ENVIRONMENT_SELECT}
         WHERE (
             (e.team_id IS NULL OR e.team_id = '')
             AND (e.organization_id IS NULL OR e.organization_id = '')
         ) OR (
             (e.team_id IS NULL OR e.team_id = '')
             AND e.organization_id = ?1
             AND ?1 != ''
         ) OR e.team_id = ?2
         ORDER BY
           CASE
             WHEN e.team_id IS NOT NULL AND e.team_id != '' THEN 2
             WHEN e.organization_id IS NOT NULL AND e.organization_id != '' THEN 1
             ELSE 0
           END,
           e.name"
    ))?;
    let rows = stmt.query_map(params![team.organization_id, team.id], map_environment)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn environment(c: &Connection, id: &str) -> AppResult<Option<Environment>> {
    c.query_row(
        &format!("{ENVIRONMENT_SELECT} WHERE e.id = ?1"),
        [id],
        map_environment,
    )
    .optional()
    .map_err(Into::into)
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

fn map_environment(row: &rusqlite::Row<'_>) -> rusqlite::Result<Environment> {
    let team_id = optional_id(row.get(1)?);
    let organization_id = optional_id(row.get(2)?);
    let organization_name: String = row.get(8)?;
    let team_name: String = row.get(9)?;
    let scope = if team_id.is_some() {
        "team"
    } else if organization_id.is_some() {
        "organization"
    } else {
        "global"
    };
    Ok(Environment {
        id: row.get(0)?,
        team_id,
        organization_id,
        name: row.get(3)?,
        image: row.get(4)?,
        pull_policy: row.get(5)?,
        description: row.get(6)?,
        created_at: row.get(7)?,
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

fn optional_id(value: Option<String>) -> Option<String> {
    value.and_then(|item| {
        let item = item.trim().to_string();
        if item.is_empty() {
            None
        } else {
            Some(item)
        }
    })
}

pub fn update_environment(c: &Connection, item: &Environment) -> AppResult<()> {
    let n = c.execute(
        "UPDATE environments
         SET team_id=?2, organization_id=?3, name=?4, image=?5, pull_policy=?6, description=?7
         WHERE id=?1",
        params![
            item.id,
            item.team_id,
            item.organization_id,
            item.name,
            item.image,
            item.pull_policy,
            item.description
        ],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub fn delete_environment(c: &Connection, id: &str) -> AppResult<()> {
    let n = c.execute("DELETE FROM environments WHERE id = ?1", [id])?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub fn insert_template(c: &Connection, item: &Template) -> AppResult<()> {
    c.execute(
        "INSERT INTO templates (
            id, team_id, name, description, tool, project_id, inventory_id, credential_id,
            environment_id, entrypoint, extra_vars, spec, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            item.id,
            item.team_id,
            item.name,
            item.description,
            item.tool,
            item.project_id,
            item.inventory_id,
            item.credential_id,
            item.environment_id,
            item.entrypoint,
            item.extra_vars,
            item.spec,
            item.created_at
        ],
    )?;
    Ok(())
}

pub fn update_template(c: &Connection, item: &Template) -> AppResult<()> {
    let n = c.execute(
        "UPDATE templates SET name=?3, description=?4, tool=?5, project_id=?6, inventory_id=?7,
         credential_id=?8, environment_id=?9, entrypoint=?10, extra_vars=?11, spec=?12
         WHERE id=?1 AND team_id=?2",
        params![
            item.id,
            item.team_id,
            item.name,
            item.description,
            item.tool,
            item.project_id,
            item.inventory_id,
            item.credential_id,
            item.environment_id,
            item.entrypoint,
            item.extra_vars,
            item.spec
        ],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub fn list_templates(c: &Connection, team_id: &str) -> AppResult<Vec<Template>> {
    let mut stmt = c.prepare(
        "SELECT id, team_id, name, description, tool, project_id, inventory_id, credential_id,
                environment_id, entrypoint, extra_vars, spec, created_at
         FROM templates WHERE team_id = ?1 ORDER BY name",
    )?;
    let rows = stmt.query_map([team_id], map_template)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn template(c: &Connection, id: &str) -> AppResult<Option<Template>> {
    c.query_row(
        "SELECT id, team_id, name, description, tool, project_id, inventory_id, credential_id,
                environment_id, entrypoint, extra_vars, spec, created_at
         FROM templates WHERE id = ?1",
        [id],
        map_template,
    )
    .optional()
    .map_err(Into::into)
}

fn map_template(row: &rusqlite::Row<'_>) -> rusqlite::Result<Template> {
    Ok(Template {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        tool: row.get(4)?,
        project_id: row.get(5)?,
        inventory_id: row.get(6)?,
        credential_id: row.get(7)?,
        environment_id: row.get(8)?,
        entrypoint: row.get(9)?,
        extra_vars: row.get(10)?,
        spec: row.get(11)?,
        created_at: row.get(12)?,
    })
}

pub fn delete_template(c: &Connection, id: &str, team_id: &str) -> AppResult<()> {
    let n = c.execute(
        "DELETE FROM templates WHERE id = ?1 AND team_id = ?2",
        params![id, team_id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub fn template_credential_ids(c: &Connection, template_id: &str) -> AppResult<Vec<String>> {
    let mut stmt = c.prepare(
        "SELECT credential_id FROM template_credentials
         WHERE template_id = ?1 ORDER BY position, credential_id",
    )?;
    let rows = stmt.query_map([template_id], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn list_template_credential_links(
    c: &Connection,
    team_id: &str,
) -> AppResult<Vec<(String, String)>> {
    let mut stmt = c.prepare(
        "SELECT tc.template_id, tc.credential_id
         FROM template_credentials tc
         JOIN templates t ON t.id = tc.template_id
         WHERE t.team_id = ?1
         ORDER BY tc.template_id, tc.position, tc.credential_id",
    )?;
    let rows = stmt.query_map([team_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn save_template_credentials(
    c: &Connection,
    template_id: &str,
    ids: &[String],
) -> AppResult<()> {
    c.execute(
        "DELETE FROM template_credentials WHERE template_id = ?1",
        [template_id],
    )?;
    for (position, id) in ids.iter().enumerate() {
        c.execute(
            "INSERT INTO template_credentials (template_id, credential_id, position)
             VALUES (?1, ?2, ?3)",
            params![template_id, id, position as i64],
        )?;
    }
    Ok(())
}

const JOB_SELECT: &str = "id, team_id, template_id, workflow_job_id, workflow_node_id, name, tool, status, extra_vars, launched_by, trigger, started_at, finished_at, created_at, parameters, failed_hosts";

pub fn insert_job(c: &Connection, job: &Job) -> AppResult<()> {
    c.execute(
        "INSERT INTO jobs (
            id, team_id, template_id, workflow_job_id, workflow_node_id, name, tool, status,
            extra_vars, launched_by, trigger, started_at, finished_at, created_at,
            parameters, failed_hosts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            job.id,
            job.team_id,
            job.template_id,
            job.workflow_job_id,
            job.workflow_node_id,
            job.name,
            job.tool,
            job.status,
            job.extra_vars,
            job.launched_by,
            job.trigger,
            job.started_at,
            job.finished_at,
            job.created_at,
            job.parameters,
            job.failed_hosts
        ],
    )?;
    Ok(())
}

pub fn job(c: &Connection, id: &str) -> AppResult<Option<Job>> {
    c.query_row(
        &format!("SELECT {JOB_SELECT} FROM jobs WHERE id = ?1"),
        [id],
        map_job,
    )
    .optional()
    .map_err(Into::into)
}

fn map_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    Ok(Job {
        id: row.get(0)?,
        team_id: row.get(1)?,
        template_id: row.get(2)?,
        workflow_job_id: row.get(3)?,
        workflow_node_id: row.get(4)?,
        name: row.get(5)?,
        tool: row.get(6)?,
        status: row.get(7)?,
        extra_vars: row.get(8)?,
        launched_by: row.get(9)?,
        trigger: row.get(10)?,
        started_at: row.get(11)?,
        finished_at: row.get(12)?,
        created_at: row.get(13)?,
        parameters: row.get(14)?,
        failed_hosts: row.get(15)?,
    })
}

pub fn list_jobs(c: &Connection, team_id: &str, limit: i64) -> AppResult<Vec<Job>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {JOB_SELECT} FROM jobs WHERE team_id = ?1 ORDER BY created_at DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![team_id, limit], map_job)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn mark_running(c: &Connection, id: &str) -> AppResult<bool> {
    let n = c.execute(
        "UPDATE jobs SET status = 'running', started_at = COALESCE(started_at, ?2)
         WHERE id = ?1 AND status = 'pending'",
        params![id, now()],
    )?;
    Ok(n == 1)
}

pub fn set_failed_hosts(c: &Connection, id: &str, failed_hosts: &str) -> AppResult<()> {
    c.execute(
        "UPDATE jobs SET failed_hosts = ?2 WHERE id = ?1",
        params![id, failed_hosts],
    )?;
    Ok(())
}

pub fn finish_job(c: &Connection, id: &str, status: &str) -> AppResult<bool> {
    let n = c.execute(
        "UPDATE jobs SET status = ?2, finished_at = ?3
         WHERE id = ?1 AND status IN ('pending', 'running')",
        params![id, status, now()],
    )?;
    Ok(n == 1)
}

pub fn insert_event(c: &Connection, job_id: &str, stream: &str, line: &str) -> AppResult<()> {
    c.execute(
        "INSERT INTO job_events (job_id, ts, stream, line) VALUES (?1, ?2, ?3, ?4)",
        params![job_id, now(), stream, line],
    )?;
    Ok(())
}

pub fn events_after(
    c: &Connection,
    job_id: &str,
    after: i64,
    limit: i64,
) -> AppResult<Vec<JobEvent>> {
    let mut stmt = c.prepare(
        "SELECT id, ts, stream, line FROM job_events
         WHERE job_id = ?1 AND id > ?2 ORDER BY id LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![job_id, after, limit], |row| {
        Ok(JobEvent {
            id: row.get(0)?,
            ts: row.get(1)?,
            stream: row.get(2)?,
            line: row.get(3)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn insert_workflow(c: &Connection, item: &Workflow) -> AppResult<()> {
    c.execute(
        "INSERT INTO workflows (id, team_id, name, description, graph, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            item.id,
            item.team_id,
            item.name,
            item.description,
            item.graph,
            item.created_at
        ],
    )?;
    Ok(())
}

pub fn update_workflow(c: &Connection, item: &Workflow) -> AppResult<()> {
    let n = c.execute(
        "UPDATE workflows SET name = ?3, description = ?4, graph = ?5 WHERE id = ?1 AND team_id = ?2",
        params![item.id, item.team_id, item.name, item.description, item.graph],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub fn list_workflows(c: &Connection, team_id: &str) -> AppResult<Vec<Workflow>> {
    let mut stmt = c.prepare(
        "SELECT id, team_id, name, description, graph, created_at FROM workflows WHERE team_id = ?1 ORDER BY name",
    )?;
    let rows = stmt.query_map([team_id], map_workflow)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn workflow(c: &Connection, id: &str) -> AppResult<Option<Workflow>> {
    c.query_row(
        "SELECT id, team_id, name, description, graph, created_at FROM workflows WHERE id = ?1",
        [id],
        map_workflow,
    )
    .optional()
    .map_err(Into::into)
}

fn map_workflow(row: &rusqlite::Row<'_>) -> rusqlite::Result<Workflow> {
    Ok(Workflow {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        graph: row.get(4)?,
        created_at: row.get(5)?,
    })
}

pub fn delete_workflow(c: &Connection, id: &str, team_id: &str) -> AppResult<()> {
    let n = c.execute(
        "DELETE FROM workflows WHERE id = ?1 AND team_id = ?2",
        params![id, team_id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub fn first_workflow(c: &Connection, team_id: &str) -> AppResult<Option<Workflow>> {
    c.query_row(
        "SELECT id, team_id, name, description, graph, created_at FROM workflows
         WHERE team_id = ?1 ORDER BY created_at LIMIT 1",
        [team_id],
        map_workflow,
    )
    .optional()
    .map_err(Into::into)
}

pub fn insert_workflow_job(c: &Connection, item: &WorkflowJob) -> AppResult<()> {
    c.execute(
        "INSERT INTO workflow_jobs (
            id, workflow_id, team_id, name, status, extra_vars, launched_by, trigger, created_at, finished_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            item.id,
            item.workflow_id,
            item.team_id,
            item.name,
            item.status,
            item.extra_vars,
            item.launched_by,
            item.trigger,
            item.created_at,
            item.finished_at
        ],
    )?;
    Ok(())
}

pub fn workflow_job(c: &Connection, id: &str) -> AppResult<Option<WorkflowJob>> {
    c.query_row(
        "SELECT id, workflow_id, team_id, name, status, extra_vars, launched_by, trigger, created_at, finished_at
         FROM workflow_jobs WHERE id = ?1",
        [id],
        map_workflow_job,
    )
    .optional()
    .map_err(Into::into)
}

fn map_workflow_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowJob> {
    Ok(WorkflowJob {
        id: row.get(0)?,
        workflow_id: row.get(1)?,
        team_id: row.get(2)?,
        name: row.get(3)?,
        status: row.get(4)?,
        extra_vars: row.get(5)?,
        launched_by: row.get(6)?,
        trigger: row.get(7)?,
        created_at: row.get(8)?,
        finished_at: row.get(9)?,
    })
}

pub fn set_workflow_status(c: &Connection, id: &str, status: &str, finish: bool) -> AppResult<()> {
    if finish {
        c.execute(
            "UPDATE workflow_jobs SET status = ?2, finished_at = ?3 WHERE id = ?1",
            params![id, status, now()],
        )?;
    } else {
        c.execute(
            "UPDATE workflow_jobs SET status = ?2 WHERE id = ?1",
            params![id, status],
        )?;
    }
    Ok(())
}

pub fn insert_workflow_node(c: &Connection, workflow_job_id: &str, node_id: &str) -> AppResult<()> {
    c.execute(
        "INSERT INTO workflow_nodes (workflow_job_id, node_id, phase, job_id) VALUES (?1, ?2, 'pending', NULL)",
        params![workflow_job_id, node_id],
    )?;
    Ok(())
}

pub fn set_workflow_node(
    c: &Connection,
    workflow_job_id: &str,
    node_id: &str,
    phase: &str,
    job_id: Option<&str>,
) -> AppResult<()> {
    c.execute(
        "UPDATE workflow_nodes SET phase = ?3, job_id = COALESCE(?4, job_id)
         WHERE workflow_job_id = ?1 AND node_id = ?2",
        params![workflow_job_id, node_id, phase, job_id],
    )?;
    Ok(())
}

pub fn workflow_nodes(c: &Connection, workflow_job_id: &str) -> AppResult<Vec<WorkflowNodeRow>> {
    let mut stmt =
        c.prepare("SELECT node_id, phase, job_id FROM workflow_nodes WHERE workflow_job_id = ?1")?;
    let rows = stmt.query_map([workflow_job_id], |row| {
        Ok(WorkflowNodeRow {
            node_id: row.get(0)?,
            phase: row.get(1)?,
            job_id: row.get(2)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn insert_hook(c: &Connection, item: &Hook) -> AppResult<()> {
    c.execute(
        "INSERT INTO hooks (
            id, team_id, name, token, match_alertname, match_status, template_id, workflow_id,
            extra_vars, enabled, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            item.id,
            item.team_id,
            item.name,
            item.token,
            item.match_alertname,
            item.match_status,
            item.template_id,
            item.workflow_id,
            item.extra_vars,
            item.enabled as i64,
            item.created_at
        ],
    )?;
    Ok(())
}

pub fn list_hooks(c: &Connection, team_id: &str) -> AppResult<Vec<Hook>> {
    let mut stmt = c.prepare(
        "SELECT id, team_id, name, token, match_alertname, match_status, template_id, workflow_id,
                extra_vars, enabled, created_at
         FROM hooks WHERE team_id = ?1 ORDER BY name",
    )?;
    let rows = stmt.query_map([team_id], map_hook)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn hook(c: &Connection, id: &str) -> AppResult<Option<Hook>> {
    c.query_row(
        "SELECT id, team_id, name, token, match_alertname, match_status, template_id, workflow_id,
                extra_vars, enabled, created_at
         FROM hooks WHERE id = ?1",
        [id],
        map_hook,
    )
    .optional()
    .map_err(Into::into)
}

fn map_hook(row: &rusqlite::Row<'_>) -> rusqlite::Result<Hook> {
    Ok(Hook {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        token: row.get(3)?,
        match_alertname: row.get(4)?,
        match_status: row.get(5)?,
        template_id: row.get(6)?,
        workflow_id: row.get(7)?,
        extra_vars: row.get(8)?,
        enabled: flag(row.get(9)?),
        created_at: row.get(10)?,
    })
}

pub fn delete_hook(c: &Connection, id: &str, team_id: &str) -> AppResult<()> {
    let n = c.execute(
        "DELETE FROM hooks WHERE id = ?1 AND team_id = ?2",
        params![id, team_id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub fn first_hook(c: &Connection, team_id: &str) -> AppResult<Option<Hook>> {
    c.query_row(
        "SELECT id, team_id, name, token, match_alertname, match_status, template_id, workflow_id,
                extra_vars, enabled, created_at
         FROM hooks WHERE team_id = ?1 ORDER BY created_at LIMIT 1",
        [team_id],
        map_hook,
    )
    .optional()
    .map_err(Into::into)
}

pub struct NewDelivery<'a> {
    pub id: &'a str,
    pub hook_id: &'a str,
    pub status: &'a str,
    pub summary: &'a str,
    pub payload: &'a str,
    pub job_id: Option<&'a str>,
    pub workflow_job_id: Option<&'a str>,
}

pub fn insert_delivery(c: &Connection, item: NewDelivery<'_>) -> AppResult<()> {
    c.execute(
        "INSERT INTO hook_deliveries (
            id, hook_id, status, summary, payload, job_id, workflow_job_id, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            item.id,
            item.hook_id,
            item.status,
            item.summary,
            item.payload,
            item.job_id,
            item.workflow_job_id,
            now()
        ],
    )?;
    Ok(())
}

pub fn list_deliveries(c: &Connection, team_id: &str, limit: i64) -> AppResult<Vec<Delivery>> {
    let mut stmt = c.prepare(
        "SELECT d.id, d.hook_id, h.name, d.status, d.summary, d.job_id, d.workflow_job_id, d.created_at
         FROM hook_deliveries d JOIN hooks h ON h.id = d.hook_id
         WHERE h.team_id = ?1 ORDER BY d.created_at DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![team_id, limit], |row| {
        Ok(Delivery {
            id: row.get(0)?,
            hook_id: row.get(1)?,
            hook_name: row.get(2)?,
            status: row.get(3)?,
            summary: row.get(4)?,
            job_id: row.get(5)?,
            workflow_job_id: row.get(6)?,
            created_at: row.get(7)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn json_object(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::Object(Default::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        insert_user(&conn, "user", "ada", "Ada", None, "ldap", false).unwrap();
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
            image: "quay.io/example/ee:1".into(),
            pull_policy: "if_not_present".into(),
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
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
