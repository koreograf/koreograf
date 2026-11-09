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
use crate::db::{
    self, ApiToken, Credential, CredentialUsage, Delivery, Environment, Hook, Inventory,
    InventorySource, Job, JobActivity, JobEvent, JobQuery, Member, NewDelivery, Organization,
    Project, Schedule, Team, Template, User, UserMembership, Workflow, WorkflowJob,
    WorkflowNodeRow,
};
use crate::error::{ApiError, AppResult};

pub(crate) trait Backend: Send + Sync {
    fn engine(&self) -> &'static str;
    fn sqlite(&self) -> Option<&Mutex<Connection>>;
    fn export_sqlite(&self, dest: &Path) -> AppResult<()>;
    fn restore_sqlite(&self, source: &Path) -> AppResult<()>;
    fn busy(&self) -> AppResult<bool>;
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
    fn user_by_id(&self, id: &str) -> AppResult<Option<User>>;
    fn list_users(&self) -> AppResult<Vec<(User, i64)>>;
    fn update_profile(
        &self,
        id: &str,
        display_name: &str,
        email: Option<&str>,
        password_hash: Option<&str>,
    ) -> AppResult<()>;
    fn set_user_email(&self, id: &str, email: Option<&str>) -> AppResult<()>;
    fn update_user(
        &self,
        id: &str,
        display_name: &str,
        is_admin: bool,
        disabled: bool,
        password_hash: Option<&str>,
    ) -> AppResult<()>;
    fn delete_user_sessions(&self, user_id: &str) -> AppResult<()>;
    fn delete_user(&self, id: &str) -> AppResult<bool>;
    fn other_active_admins(&self, user_id: &str) -> AppResult<i64>;
    fn create_api_token(
        &self,
        id: &str,
        user_id: &str,
        name: &str,
        prefix: &str,
        token_hash: &str,
        created_at: &str,
    ) -> AppResult<()>;
    fn list_api_tokens(&self, user_id: &str) -> AppResult<Vec<ApiToken>>;
    fn delete_api_token(&self, id: &str, user_id: &str) -> AppResult<bool>;
    fn list_teams_for_user(&self, user_id: &str, admin: bool) -> AppResult<Vec<(Team, String)>>;
    fn list_user_memberships(&self, user_id: &str) -> AppResult<Vec<UserMembership>>;
    fn replace_settings(&self, rows: &[(String, String)]) -> AppResult<()>;
    fn environment(&self, id: &str) -> AppResult<Option<Environment>>;
    fn team_by_id(&self, id: &str) -> AppResult<Option<Team>>;
    fn role_of(&self, team_id: &str, user_id: &str) -> AppResult<Option<String>>;
    fn insert_team(
        &self,
        id: &str,
        organization_id: &str,
        name: &str,
        description: &str,
        ldap_group: Option<&str>,
    ) -> AppResult<()>;
    fn update_team(
        &self,
        id: &str,
        name: &str,
        description: &str,
        ldap_group: Option<&str>,
    ) -> AppResult<()>;
    fn set_team_organization(&self, team_id: &str, organization_id: &str) -> AppResult<()>;
    fn insert_organization(&self, id: &str, name: &str, description: &str) -> AppResult<()>;
    fn update_organization(&self, id: &str, name: &str, description: &str) -> AppResult<()>;
    fn organization(&self, id: &str) -> AppResult<Option<Organization>>;
    fn organization_by_name(&self, name: &str) -> AppResult<Option<Organization>>;
    fn list_organizations(&self, user_id: &str, admin: bool) -> AppResult<Vec<Organization>>;
    fn teams_in_organization(&self, organization_id: &str) -> AppResult<Vec<Team>>;
    fn teams_with_ldap(&self) -> AppResult<Vec<Team>>;
    fn upsert_membership(
        &self,
        team_id: &str,
        user_id: &str,
        role: &str,
        source: &str,
    ) -> AppResult<()>;
    fn grant_manual_membership(&self, team_id: &str, user_id: &str, role: &str) -> AppResult<()>;
    fn list_members(&self, team_id: &str) -> AppResult<Vec<Member>>;
    fn delete_membership(&self, team_id: &str, user_id: &str) -> AppResult<()>;
    fn delete_ldap_membership(&self, team_id: &str, user_id: &str) -> AppResult<()>;
    fn list_projects(&self, team_id: &str) -> AppResult<Vec<Project>>;
    fn project(&self, id: &str) -> AppResult<Option<Project>>;
    fn project_visible(&self, id: &str, team_id: &str) -> AppResult<bool>;
    fn insert_project(&self, project: &Project) -> AppResult<()>;
    fn delete_project(&self, id: &str, team_id: &str) -> AppResult<()>;
    fn set_project_details(
        &self,
        id: &str,
        name: &str,
        source_type: &str,
        source: &str,
        revision: &str,
    ) -> AppResult<()>;
    fn set_project_organization(&self, id: &str, organization_id: &str) -> AppResult<()>;
    fn replace_project_teams(&self, project_id: &str, team_ids: &[String]) -> AppResult<()>;
    fn set_project_credential(&self, id: &str, credential_id: Option<&str>) -> AppResult<()>;
    fn set_project_admin_saved(&self, id: &str, admin_saved: bool) -> AppResult<()>;
    fn set_project_sync_success(&self, id: &str, at: &str, commit: &str) -> AppResult<()>;
    fn set_project_sync_failed(&self, id: &str) -> AppResult<()>;
    fn list_inventories(&self, team_id: &str) -> AppResult<Vec<Inventory>>;
    fn inventory(&self, id: &str) -> AppResult<Option<Inventory>>;
    fn insert_inventory(
        &self,
        item: &Inventory,
        sources: &[InventorySource],
        refresh: bool,
    ) -> AppResult<()>;
    fn update_inventory(&self, id: &str, team_id: &str, name: &str, body: &str) -> AppResult<()>;
    fn set_inventory_admin_saved(&self, id: &str, admin_saved: bool) -> AppResult<()>;
    fn set_inventory_body(&self, id: &str, team_id: &str, body: &str) -> AppResult<()>;
    fn list_inventory_sources(&self, inventory_id: &str) -> AppResult<Vec<InventorySource>>;
    fn inventory_source(&self, id: &str) -> AppResult<Option<InventorySource>>;
    fn add_inventory_source(
        &self,
        inventory_id: &str,
        source: &InventorySource,
        admin_saved: bool,
    ) -> AppResult<()>;
    fn set_inventory_source_body(&self, id: &str, body: &str) -> AppResult<()>;
    fn update_inventory_source(&self, source: &InventorySource, admin_saved: bool)
        -> AppResult<()>;
    fn delete_inventory_source(
        &self,
        id: &str,
        inventory_id: &str,
        admin_saved: bool,
    ) -> AppResult<()>;
    fn refresh_inventory_summary(&self, inventory_id: &str) -> AppResult<()>;
    fn sync_inventory_sources(
        &self,
        inventory_id: &str,
        bodies: &[(String, String)],
        admin_saved: bool,
    ) -> AppResult<()>;
    fn delete_inventory(&self, id: &str, team_id: &str) -> AppResult<()>;
    fn templates_using_inventory(
        &self,
        inventory_id: &str,
        team_id: &str,
    ) -> AppResult<Vec<String>>;
    fn workflows_using_inventory(
        &self,
        inventory_id: &str,
        team_id: &str,
    ) -> AppResult<Vec<String>>;
    fn list_credentials(&self, team_id: &str) -> AppResult<Vec<Credential>>;
    fn credential(&self, id: &str) -> AppResult<Option<Credential>>;
    fn insert_credential(&self, item: &Credential) -> AppResult<()>;
    fn update_credential(&self, item: &Credential) -> AppResult<()>;
    fn delete_credential(&self, id: &str, team_id: &str) -> AppResult<()>;
    fn credential_usages(
        &self,
        credential_id: &str,
        team_id: &str,
    ) -> AppResult<Vec<CredentialUsage>>;
    fn list_environments(&self, team_id: &str) -> AppResult<Vec<Environment>>;
    fn insert_environment(&self, item: &Environment) -> AppResult<()>;
    fn update_environment(&self, item: &Environment) -> AppResult<()>;
    fn delete_environment(&self, id: &str) -> AppResult<()>;
    fn list_templates(&self, team_id: &str) -> AppResult<Vec<Template>>;
    fn template(&self, id: &str) -> AppResult<Option<Template>>;
    fn insert_template(&self, item: &Template, credential_ids: &[String]) -> AppResult<()>;
    fn update_template(&self, item: &Template, credential_ids: &[String]) -> AppResult<()>;
    fn delete_template(&self, id: &str, team_id: &str) -> AppResult<()>;
    fn template_credential_ids(&self, template_id: &str) -> AppResult<Vec<String>>;
    fn list_template_credential_links(&self, team_id: &str) -> AppResult<Vec<(String, String)>>;
    fn list_hooks(&self, team_id: &str) -> AppResult<Vec<Hook>>;
    fn hook(&self, id: &str) -> AppResult<Option<Hook>>;
    fn insert_hook(&self, item: &Hook) -> AppResult<()>;
    fn update_hook(&self, item: &Hook) -> AppResult<()>;
    fn delete_hook(&self, id: &str, team_id: &str) -> AppResult<()>;
    fn list_deliveries(&self, team_id: &str, limit: i64) -> AppResult<Vec<Delivery>>;
    fn insert_delivery(&self, item: &NewDelivery<'_>) -> AppResult<()>;
    fn job(&self, id: &str) -> AppResult<Option<Job>>;
    fn list_jobs(&self, team_id: &str, query: &JobQuery) -> AppResult<(Vec<Job>, i64)>;
    fn insert_job(&self, item: &Job) -> AppResult<()>;
    fn mark_running(&self, id: &str) -> AppResult<bool>;
    fn set_failed_hosts(&self, id: &str, failed_hosts: &str) -> AppResult<()>;
    fn set_job_parameters(&self, id: &str, parameters: &str) -> AppResult<()>;
    fn finish_job(&self, id: &str, status: &str) -> AppResult<bool>;
    fn insert_event(&self, job_id: &str, stream: &str, line: &str) -> AppResult<()>;
    fn events_after(&self, job_id: &str, after: i64, limit: i64) -> AppResult<Vec<JobEvent>>;
    fn events_count_between(&self, job_id: &str, after: i64, before: i64) -> AppResult<i64>;
    fn events_bytes(&self, job_id: &str) -> AppResult<i64>;
    fn events_last(&self, job_id: &str, limit: i64) -> AppResult<Vec<JobEvent>>;
    fn workflow_job(&self, id: &str) -> AppResult<Option<WorkflowJob>>;
    fn insert_workflow_job(&self, item: &WorkflowJob) -> AppResult<()>;
    fn set_workflow_status(&self, id: &str, status: &str, finish: bool) -> AppResult<()>;
    fn insert_workflow_node(&self, workflow_job_id: &str, node_id: &str) -> AppResult<()>;
    fn set_workflow_node(
        &self,
        workflow_job_id: &str,
        node_id: &str,
        phase: &str,
        job_id: Option<&str>,
    ) -> AppResult<()>;
    fn workflow_nodes(&self, workflow_job_id: &str) -> AppResult<Vec<WorkflowNodeRow>>;
    fn recent_jobs(&self, team_id: &str, limit: i64) -> AppResult<Vec<Job>>;
    fn job_activity(&self, team_id: &str, from: &str, to: &str) -> AppResult<Vec<JobActivity>>;
    fn import_report(&self) -> AppResult<Option<serde_json::Value>>;
    fn first_workflow(&self, team_id: &str) -> AppResult<Option<Workflow>>;
    fn first_hook(&self, team_id: &str) -> AppResult<Option<Hook>>;
    fn list_workflows(&self, team_id: &str) -> AppResult<Vec<Workflow>>;
    fn workflow(&self, id: &str) -> AppResult<Option<Workflow>>;
    fn insert_workflow(&self, item: &Workflow) -> AppResult<()>;
    fn update_workflow(&self, item: &Workflow) -> AppResult<()>;
    fn delete_workflow(&self, id: &str, team_id: &str) -> AppResult<()>;
    fn workflow_team(&self, id: &str) -> AppResult<Option<String>>;
    fn list_team_schedules(&self, team_id: &str) -> AppResult<Vec<(Schedule, String)>>;
    fn schedule(&self, id: &str) -> AppResult<Option<Schedule>>;
    fn list_schedules(&self, kind: &str, target_id: &str) -> AppResult<Vec<Schedule>>;
    fn insert_schedule(&self, item: &Schedule) -> AppResult<()>;
    fn update_schedule(&self, item: &Schedule) -> AppResult<()>;
    fn delete_schedule(&self, id: &str, team_id: &str) -> AppResult<()>;
    fn set_schedule_enabled(
        &self,
        id: &str,
        team_id: &str,
        enabled: bool,
        next_run_at: Option<&str>,
        occurrence_count: i64,
    ) -> AppResult<()>;
    fn claim_missed(&self, id: &str, team_id: &str) -> AppResult<Option<String>>;
    fn restore_missed(&self, id: &str, team_id: &str, missed_at: &str) -> AppResult<()>;
    fn due_schedules(&self, now: &str) -> AppResult<Vec<Schedule>>;
    fn advance_schedule(
        &self,
        id: &str,
        next_run_at: Option<&str>,
        occurrence_count: i64,
    ) -> AppResult<()>;
    fn note_miss(&self, id: &str, missed_at: Option<&str>) -> AppResult<()>;
    fn set_launch_block(&self, id: &str, reason: Option<&str>) -> AppResult<()>;
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

    fn export_sqlite(&self, dest: &Path) -> AppResult<()> {
        let live = lock(&self.conn)?;
        let mut snapshot = Connection::open(dest)?;
        crate::backup::copy_database(&live, &mut snapshot)
    }

    fn restore_sqlite(&self, source: &Path) -> AppResult<()> {
        let mut live = lock(&self.conn)?;
        let source = Connection::open(source)?;
        crate::backup::copy_database(&source, &mut live)?;
        db::ensure_schema(&live)
    }

    fn busy(&self) -> AppResult<bool> {
        let live = lock(&self.conn)?;
        crate::backup::busy(&live)
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

    fn user_by_id(&self, id: &str) -> AppResult<Option<User>> {
        let conn = lock(&self.conn)?;
        db::user_by_id(&conn, id)
    }

    fn list_users(&self) -> AppResult<Vec<(User, i64)>> {
        let conn = lock(&self.conn)?;
        db::list_users(&conn)
    }

    fn update_profile(
        &self,
        id: &str,
        display_name: &str,
        email: Option<&str>,
        password_hash: Option<&str>,
    ) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::update_profile(&conn, id, display_name, email, password_hash)
    }

    fn set_user_email(&self, id: &str, email: Option<&str>) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_user_email(&conn, id, email)
    }

    fn update_user(
        &self,
        id: &str,
        display_name: &str,
        is_admin: bool,
        disabled: bool,
        password_hash: Option<&str>,
    ) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::update_user(&conn, id, display_name, is_admin, disabled, password_hash)
    }

    fn delete_user_sessions(&self, user_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::delete_user_sessions(&conn, user_id)
    }

    fn delete_user(&self, id: &str) -> AppResult<bool> {
        let conn = lock(&self.conn)?;
        db::delete_user(&conn, id)
    }

    fn other_active_admins(&self, user_id: &str) -> AppResult<i64> {
        let conn = lock(&self.conn)?;
        db::other_active_admins(&conn, user_id)
    }

    fn create_api_token(
        &self,
        id: &str,
        user_id: &str,
        name: &str,
        prefix: &str,
        token_hash: &str,
        created_at: &str,
    ) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::create_api_token(&conn, id, user_id, name, prefix, token_hash, created_at)
    }

    fn list_api_tokens(&self, user_id: &str) -> AppResult<Vec<ApiToken>> {
        let conn = lock(&self.conn)?;
        db::list_api_tokens(&conn, user_id)
    }

    fn delete_api_token(&self, id: &str, user_id: &str) -> AppResult<bool> {
        let conn = lock(&self.conn)?;
        db::delete_api_token(&conn, id, user_id)
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

    fn team_by_id(&self, id: &str) -> AppResult<Option<Team>> {
        let conn = lock(&self.conn)?;
        db::team_by_id(&conn, id)
    }

    fn role_of(&self, team_id: &str, user_id: &str) -> AppResult<Option<String>> {
        let conn = lock(&self.conn)?;
        db::role_of(&conn, team_id, user_id)
    }

    fn insert_team(
        &self,
        id: &str,
        organization_id: &str,
        name: &str,
        description: &str,
        ldap_group: Option<&str>,
    ) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::insert_team(&conn, id, organization_id, name, description, ldap_group)
    }

    fn update_team(
        &self,
        id: &str,
        name: &str,
        description: &str,
        ldap_group: Option<&str>,
    ) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::update_team(&conn, id, name, description, ldap_group)
    }

    fn set_team_organization(&self, team_id: &str, organization_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_team_organization(&conn, team_id, organization_id)
    }

    fn insert_organization(&self, id: &str, name: &str, description: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::insert_organization(&conn, id, name, description)
    }

    fn update_organization(&self, id: &str, name: &str, description: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::update_organization(&conn, id, name, description)
    }

    fn organization(&self, id: &str) -> AppResult<Option<Organization>> {
        let conn = lock(&self.conn)?;
        db::organization(&conn, id)
    }

    fn organization_by_name(&self, name: &str) -> AppResult<Option<Organization>> {
        let conn = lock(&self.conn)?;
        db::organization_by_name(&conn, name)
    }

    fn list_organizations(&self, user_id: &str, admin: bool) -> AppResult<Vec<Organization>> {
        let conn = lock(&self.conn)?;
        db::list_organizations(&conn, user_id, admin)
    }

    fn teams_in_organization(&self, organization_id: &str) -> AppResult<Vec<Team>> {
        let conn = lock(&self.conn)?;
        db::teams_in_organization(&conn, organization_id)
    }

    fn teams_with_ldap(&self) -> AppResult<Vec<Team>> {
        let conn = lock(&self.conn)?;
        db::teams_with_ldap(&conn)
    }

    fn upsert_membership(
        &self,
        team_id: &str,
        user_id: &str,
        role: &str,
        source: &str,
    ) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::upsert_membership(&conn, team_id, user_id, role, source)
    }

    fn grant_manual_membership(&self, team_id: &str, user_id: &str, role: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::grant_manual_membership(&conn, team_id, user_id, role)
    }

    fn list_members(&self, team_id: &str) -> AppResult<Vec<Member>> {
        let conn = lock(&self.conn)?;
        db::list_members(&conn, team_id)
    }

    fn delete_membership(&self, team_id: &str, user_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::delete_membership(&conn, team_id, user_id)
    }

    fn delete_ldap_membership(&self, team_id: &str, user_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::delete_ldap_membership(&conn, team_id, user_id)
    }

    fn list_projects(&self, team_id: &str) -> AppResult<Vec<Project>> {
        let conn = lock(&self.conn)?;
        db::list_projects(&conn, team_id)
    }

    fn project(&self, id: &str) -> AppResult<Option<Project>> {
        let conn = lock(&self.conn)?;
        db::project(&conn, id)
    }

    fn project_visible(&self, id: &str, team_id: &str) -> AppResult<bool> {
        let conn = lock(&self.conn)?;
        db::project_visible(&conn, id, team_id)
    }

    fn insert_project(&self, project: &Project) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::insert_project(&conn, project)
    }

    fn delete_project(&self, id: &str, team_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::delete_project(&conn, id, team_id)
    }

    fn set_project_details(
        &self,
        id: &str,
        name: &str,
        source_type: &str,
        source: &str,
        revision: &str,
    ) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_project_details(&conn, id, name, source_type, source, revision)
    }

    fn set_project_organization(&self, id: &str, organization_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_project_organization(&conn, id, organization_id)
    }

    fn replace_project_teams(&self, project_id: &str, team_ids: &[String]) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::replace_project_teams(&conn, project_id, team_ids)
    }

    fn set_project_credential(&self, id: &str, credential_id: Option<&str>) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_project_credential(&conn, id, credential_id)
    }

    fn set_project_admin_saved(&self, id: &str, admin_saved: bool) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_project_admin_saved(&conn, id, admin_saved)
    }

    fn set_project_sync_success(&self, id: &str, at: &str, commit: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_project_sync_success(&conn, id, at, commit)
    }

    fn set_project_sync_failed(&self, id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_project_sync_failed(&conn, id)
    }

    fn list_inventories(&self, team_id: &str) -> AppResult<Vec<Inventory>> {
        let conn = lock(&self.conn)?;
        db::list_inventories(&conn, team_id)
    }

    fn inventory(&self, id: &str) -> AppResult<Option<Inventory>> {
        let conn = lock(&self.conn)?;
        db::inventory(&conn, id)
    }

    fn insert_inventory(
        &self,
        item: &Inventory,
        sources: &[InventorySource],
        refresh: bool,
    ) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        let tx = conn.transaction()?;
        db::insert_inventory(&tx, item)?;
        for source in sources {
            db::insert_inventory_source(&tx, source)?;
        }
        if refresh {
            db::refresh_inventory_summary(&tx, &item.id)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn update_inventory(&self, id: &str, team_id: &str, name: &str, body: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::update_inventory(&conn, id, team_id, name, body)
    }

    fn set_inventory_admin_saved(&self, id: &str, admin_saved: bool) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_inventory_admin_saved(&conn, id, admin_saved)
    }

    fn set_inventory_body(&self, id: &str, team_id: &str, body: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_inventory_body(&conn, id, team_id, body)
    }

    fn list_inventory_sources(&self, inventory_id: &str) -> AppResult<Vec<InventorySource>> {
        let conn = lock(&self.conn)?;
        db::list_inventory_sources(&conn, inventory_id)
    }

    fn inventory_source(&self, id: &str) -> AppResult<Option<InventorySource>> {
        let conn = lock(&self.conn)?;
        db::inventory_source(&conn, id)
    }

    fn add_inventory_source(
        &self,
        inventory_id: &str,
        source: &InventorySource,
        admin_saved: bool,
    ) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        let tx = conn.transaction()?;
        db::insert_inventory_source(&tx, source)?;
        db::refresh_inventory_summary(&tx, inventory_id)?;
        db::set_inventory_admin_saved(&tx, inventory_id, admin_saved)?;
        tx.commit()?;
        Ok(())
    }

    fn set_inventory_source_body(&self, id: &str, body: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_inventory_source_body(&conn, id, body)
    }

    fn update_inventory_source(
        &self,
        source: &InventorySource,
        admin_saved: bool,
    ) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        let tx = conn.transaction()?;
        db::update_inventory_source(
            &tx,
            &source.id,
            &source.name,
            &source.source_json,
            source.credential_id.as_deref(),
            &source.body,
        )?;
        db::refresh_inventory_summary(&tx, &source.inventory_id)?;
        db::set_inventory_admin_saved(&tx, &source.inventory_id, admin_saved)?;
        tx.commit()?;
        Ok(())
    }

    fn delete_inventory_source(
        &self,
        id: &str,
        inventory_id: &str,
        admin_saved: bool,
    ) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        let tx = conn.transaction()?;
        db::delete_inventory_source(&tx, id, inventory_id)?;
        db::refresh_inventory_summary(&tx, inventory_id)?;
        db::set_inventory_admin_saved(&tx, inventory_id, admin_saved)?;
        tx.commit()?;
        Ok(())
    }

    fn refresh_inventory_summary(&self, inventory_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::refresh_inventory_summary(&conn, inventory_id)
    }

    fn sync_inventory_sources(
        &self,
        inventory_id: &str,
        bodies: &[(String, String)],
        admin_saved: bool,
    ) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        let tx = conn.transaction()?;
        for (source_id, body) in bodies {
            db::set_inventory_source_body(&tx, source_id, body)?;
        }
        db::refresh_inventory_summary(&tx, inventory_id)?;
        db::set_inventory_admin_saved(&tx, inventory_id, admin_saved)?;
        tx.commit()?;
        Ok(())
    }

    fn delete_inventory(&self, id: &str, team_id: &str) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        db::delete_inventory(&mut conn, id, team_id)
    }

    fn templates_using_inventory(
        &self,
        inventory_id: &str,
        team_id: &str,
    ) -> AppResult<Vec<String>> {
        let conn = lock(&self.conn)?;
        db::templates_using_inventory(&conn, inventory_id, team_id)
    }

    fn workflows_using_inventory(
        &self,
        inventory_id: &str,
        team_id: &str,
    ) -> AppResult<Vec<String>> {
        let conn = lock(&self.conn)?;
        db::workflows_using_inventory(&conn, inventory_id, team_id)
    }

    fn list_credentials(&self, team_id: &str) -> AppResult<Vec<Credential>> {
        let conn = lock(&self.conn)?;
        db::list_credentials(&conn, team_id)
    }

    fn credential(&self, id: &str) -> AppResult<Option<Credential>> {
        let conn = lock(&self.conn)?;
        db::credential(&conn, id)
    }

    fn insert_credential(&self, item: &Credential) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::insert_credential(&conn, item)
    }

    fn update_credential(&self, item: &Credential) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::update_credential(&conn, item)
    }

    fn delete_credential(&self, id: &str, team_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::delete_credential(&conn, id, team_id)
    }

    fn credential_usages(
        &self,
        credential_id: &str,
        team_id: &str,
    ) -> AppResult<Vec<CredentialUsage>> {
        let conn = lock(&self.conn)?;
        db::credential_usages(&conn, credential_id, team_id)
    }

    fn list_environments(&self, team_id: &str) -> AppResult<Vec<Environment>> {
        let conn = lock(&self.conn)?;
        let team = db::team_by_id(&conn, team_id)?.ok_or(ApiError::NotFound)?;
        db::list_environments(&conn, &team)
    }

    fn insert_environment(&self, item: &Environment) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::insert_environment(&conn, item)
    }

    fn update_environment(&self, item: &Environment) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::update_environment(&conn, item)
    }

    fn delete_environment(&self, id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::delete_environment(&conn, id)
    }

    fn list_templates(&self, team_id: &str) -> AppResult<Vec<Template>> {
        let conn = lock(&self.conn)?;
        db::list_templates(&conn, team_id)
    }

    fn template(&self, id: &str) -> AppResult<Option<Template>> {
        let conn = lock(&self.conn)?;
        db::template(&conn, id)
    }

    fn insert_template(&self, item: &Template, credential_ids: &[String]) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        let tx = conn.transaction()?;
        db::insert_template(&tx, item)?;
        db::save_template_credentials(&tx, &item.id, credential_ids)?;
        tx.commit()?;
        Ok(())
    }

    fn update_template(&self, item: &Template, credential_ids: &[String]) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        let tx = conn.transaction()?;
        db::update_template(&tx, item)?;
        db::save_template_credentials(&tx, &item.id, credential_ids)?;
        tx.commit()?;
        Ok(())
    }

    fn delete_template(&self, id: &str, team_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::delete_template(&conn, id, team_id)
    }

    fn template_credential_ids(&self, template_id: &str) -> AppResult<Vec<String>> {
        let conn = lock(&self.conn)?;
        db::template_credential_ids(&conn, template_id)
    }

    fn list_template_credential_links(&self, team_id: &str) -> AppResult<Vec<(String, String)>> {
        let conn = lock(&self.conn)?;
        db::list_template_credential_links(&conn, team_id)
    }

    fn list_hooks(&self, team_id: &str) -> AppResult<Vec<Hook>> {
        let conn = lock(&self.conn)?;
        db::list_hooks(&conn, team_id)
    }

    fn hook(&self, id: &str) -> AppResult<Option<Hook>> {
        let conn = lock(&self.conn)?;
        db::hook(&conn, id)
    }

    fn insert_hook(&self, item: &Hook) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::insert_hook(&conn, item)
    }

    fn update_hook(&self, item: &Hook) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::update_hook(&conn, item)
    }

    fn delete_hook(&self, id: &str, team_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::delete_hook(&conn, id, team_id)
    }

    fn list_deliveries(&self, team_id: &str, limit: i64) -> AppResult<Vec<Delivery>> {
        let conn = lock(&self.conn)?;
        db::list_deliveries(&conn, team_id, limit)
    }

    fn insert_delivery(&self, item: &NewDelivery<'_>) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::insert_delivery(&conn, *item)
    }

    fn job(&self, id: &str) -> AppResult<Option<Job>> {
        let conn = lock(&self.conn)?;
        db::job(&conn, id)
    }

    fn list_jobs(&self, team_id: &str, query: &JobQuery) -> AppResult<(Vec<Job>, i64)> {
        let conn = lock(&self.conn)?;
        db::list_jobs(&conn, team_id, query)
    }

    fn insert_job(&self, item: &Job) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::insert_job(&conn, item)
    }

    fn mark_running(&self, id: &str) -> AppResult<bool> {
        let conn = lock(&self.conn)?;
        db::mark_running(&conn, id)
    }

    fn set_failed_hosts(&self, id: &str, failed_hosts: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_failed_hosts(&conn, id, failed_hosts)
    }

    fn set_job_parameters(&self, id: &str, parameters: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_job_parameters(&conn, id, parameters)
    }

    fn finish_job(&self, id: &str, status: &str) -> AppResult<bool> {
        let conn = lock(&self.conn)?;
        db::finish_job(&conn, id, status)
    }

    fn insert_event(&self, job_id: &str, stream: &str, line: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::insert_event(&conn, job_id, stream, line)
    }

    fn events_after(&self, job_id: &str, after: i64, limit: i64) -> AppResult<Vec<JobEvent>> {
        let conn = lock(&self.conn)?;
        db::events_after(&conn, job_id, after, limit)
    }

    fn events_count_between(&self, job_id: &str, after: i64, before: i64) -> AppResult<i64> {
        let conn = lock(&self.conn)?;
        db::events_count_between(&conn, job_id, after, before)
    }

    fn events_bytes(&self, job_id: &str) -> AppResult<i64> {
        let conn = lock(&self.conn)?;
        db::events_bytes(&conn, job_id)
    }

    fn events_last(&self, job_id: &str, limit: i64) -> AppResult<Vec<JobEvent>> {
        let conn = lock(&self.conn)?;
        db::events_last(&conn, job_id, limit)
    }

    fn workflow_job(&self, id: &str) -> AppResult<Option<WorkflowJob>> {
        let conn = lock(&self.conn)?;
        db::workflow_job(&conn, id)
    }

    fn insert_workflow_job(&self, item: &WorkflowJob) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::insert_workflow_job(&conn, item)
    }

    fn set_workflow_status(&self, id: &str, status: &str, finish: bool) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_workflow_status(&conn, id, status, finish)
    }

    fn insert_workflow_node(&self, workflow_job_id: &str, node_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::insert_workflow_node(&conn, workflow_job_id, node_id)
    }

    fn set_workflow_node(
        &self,
        workflow_job_id: &str,
        node_id: &str,
        phase: &str,
        job_id: Option<&str>,
    ) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_workflow_node(&conn, workflow_job_id, node_id, phase, job_id)
    }

    fn workflow_nodes(&self, workflow_job_id: &str) -> AppResult<Vec<WorkflowNodeRow>> {
        let conn = lock(&self.conn)?;
        db::workflow_nodes(&conn, workflow_job_id)
    }

    fn recent_jobs(&self, team_id: &str, limit: i64) -> AppResult<Vec<Job>> {
        let conn = lock(&self.conn)?;
        let (jobs, _) = db::list_jobs(
            &conn,
            team_id,
            &db::JobQuery {
                limit,
                ..Default::default()
            },
        )?;
        Ok(jobs)
    }

    fn job_activity(&self, team_id: &str, from: &str, to: &str) -> AppResult<Vec<JobActivity>> {
        let conn = lock(&self.conn)?;
        db::job_activity(&conn, team_id, from, to)
    }

    fn import_report(&self) -> AppResult<Option<serde_json::Value>> {
        let conn = lock(&self.conn)?;
        crate::migration::report(&conn)
    }

    fn first_workflow(&self, team_id: &str) -> AppResult<Option<Workflow>> {
        let conn = lock(&self.conn)?;
        db::first_workflow(&conn, team_id)
    }

    fn first_hook(&self, team_id: &str) -> AppResult<Option<Hook>> {
        let conn = lock(&self.conn)?;
        db::first_hook(&conn, team_id)
    }

    fn list_workflows(&self, team_id: &str) -> AppResult<Vec<Workflow>> {
        let conn = lock(&self.conn)?;
        db::list_workflows(&conn, team_id)
    }

    fn workflow(&self, id: &str) -> AppResult<Option<Workflow>> {
        let conn = lock(&self.conn)?;
        db::workflow(&conn, id)
    }

    fn insert_workflow(&self, item: &Workflow) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::insert_workflow(&conn, item)
    }

    fn update_workflow(&self, item: &Workflow) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::update_workflow(&conn, item)
    }

    fn delete_workflow(&self, id: &str, team_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::delete_workflow(&conn, id, team_id)
    }

    fn workflow_team(&self, id: &str) -> AppResult<Option<String>> {
        let conn = lock(&self.conn)?;
        Ok(db::workflow(&conn, id)?.map(|item| item.team_id))
    }

    fn list_team_schedules(&self, team_id: &str) -> AppResult<Vec<(Schedule, String)>> {
        let conn = lock(&self.conn)?;
        db::list_team_schedules(&conn, team_id)
    }

    fn schedule(&self, id: &str) -> AppResult<Option<Schedule>> {
        let conn = lock(&self.conn)?;
        db::schedule(&conn, id)
    }

    fn list_schedules(&self, kind: &str, target_id: &str) -> AppResult<Vec<Schedule>> {
        let conn = lock(&self.conn)?;
        db::list_schedules(&conn, kind, target_id)
    }

    fn insert_schedule(&self, item: &Schedule) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::insert_schedule(&conn, item)
    }

    fn update_schedule(&self, item: &Schedule) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::update_schedule(&conn, item)
    }

    fn delete_schedule(&self, id: &str, team_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::delete_schedule(&conn, id, team_id)
    }

    fn set_schedule_enabled(
        &self,
        id: &str,
        team_id: &str,
        enabled: bool,
        next_run_at: Option<&str>,
        occurrence_count: i64,
    ) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_schedule_enabled(&conn, id, team_id, enabled, next_run_at, occurrence_count)
    }

    fn claim_missed(&self, id: &str, team_id: &str) -> AppResult<Option<String>> {
        let mut conn = lock(&self.conn)?;
        db::claim_missed(&mut conn, id, team_id)
    }

    fn restore_missed(&self, id: &str, team_id: &str, missed_at: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::restore_missed(&conn, id, team_id, missed_at)
    }

    fn due_schedules(&self, now: &str) -> AppResult<Vec<Schedule>> {
        let conn = lock(&self.conn)?;
        db::due_schedules(&conn, now)
    }

    fn advance_schedule(
        &self,
        id: &str,
        next_run_at: Option<&str>,
        occurrence_count: i64,
    ) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::advance_schedule(&conn, id, next_run_at, occurrence_count)
    }

    fn note_miss(&self, id: &str, missed_at: Option<&str>) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::note_miss(&conn, id, missed_at)
    }

    fn set_launch_block(&self, id: &str, reason: Option<&str>) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        db::set_launch_block(&conn, id, reason)
    }
}

pub(crate) struct PostgresBackend {
    client: futures::lock::Mutex<Client>,
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
            client: futures::lock::Mutex::new(client),
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

pub(crate) async fn connect_client(config: &PostgresConfig) -> anyhow::Result<Client> {
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

fn row_api_token(row: &tokio_postgres::Row) -> AppResult<ApiToken> {
    Ok(ApiToken {
        id: row.try_get(0)?,
        name: row.try_get(1)?,
        prefix: row.try_get(2)?,
        created_at: row.try_get(3)?,
        last_used_at: row.try_get(4)?,
    })
}

fn row_job(row: &tokio_postgres::Row) -> AppResult<Job> {
    Ok(Job {
        id: row.try_get(0)?,
        team_id: row.try_get(1)?,
        template_id: row.try_get(2)?,
        workflow_job_id: row.try_get(3)?,
        workflow_node_id: row.try_get(4)?,
        name: row.try_get(5)?,
        tool: row.try_get(6)?,
        status: row.try_get(7)?,
        extra_vars: row.try_get(8)?,
        launched_by: row.try_get(9)?,
        trigger: row.try_get(10)?,
        started_at: row.try_get(11)?,
        finished_at: row.try_get(12)?,
        created_at: row.try_get(13)?,
        parameters: row.try_get(14)?,
        failed_hosts: row.try_get(15)?,
    })
}

fn row_job_activity(row: &tokio_postgres::Row) -> AppResult<JobActivity> {
    Ok(JobActivity {
        status: row.try_get(0)?,
        started_at: row.try_get(1)?,
        finished_at: row.try_get(2)?,
    })
}

fn row_job_event(row: &tokio_postgres::Row) -> AppResult<JobEvent> {
    Ok(JobEvent {
        id: row.try_get(0)?,
        ts: row.try_get(1)?,
        stream: row.try_get(2)?,
        line: row.try_get(3)?,
    })
}

fn row_workflow_job(row: &tokio_postgres::Row) -> AppResult<WorkflowJob> {
    Ok(WorkflowJob {
        id: row.try_get(0)?,
        workflow_id: row.try_get(1)?,
        team_id: row.try_get(2)?,
        name: row.try_get(3)?,
        status: row.try_get(4)?,
        extra_vars: row.try_get(5)?,
        launched_by: row.try_get(6)?,
        trigger: row.try_get(7)?,
        created_at: row.try_get(8)?,
        finished_at: row.try_get(9)?,
    })
}

fn row_workflow_node(row: &tokio_postgres::Row) -> AppResult<WorkflowNodeRow> {
    Ok(WorkflowNodeRow {
        node_id: row.try_get(0)?,
        phase: row.try_get(1)?,
        job_id: row.try_get(2)?,
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

const JOB_COLUMNS: &str = "id, team_id, template_id, workflow_job_id, workflow_node_id, name, \
     tool, status, extra_vars, launched_by, trigger, started_at, finished_at, created_at, \
     parameters, failed_hosts";

const JOB_FILTER: &str = "team_id = $1
  AND ($2 = ''
    OR strpos(lower(name), lower($2)) > 0
    OR strpos(lower(tool), lower($2)) > 0
    OR strpos(lower(status), lower($2)) > 0
    OR strpos(lower(trigger), lower($2)) > 0)
  AND ($3 = '' OR status = $3)
  AND ($4 = '' OR tool = $4)
  AND ($5 = '' OR trigger = $5)
  AND ($6 = '' OR template_id = $6)
  AND ($7 = '' OR created_at >= $7)
  AND ($8 = '' OR created_at <= $8)";

const TEAM_COLUMNS: &str = "t.id, t.name, t.description, t.ldap_group, t.created_at, \
     COALESCE(t.organization_id, ''), COALESCE(o.name, '')";

const PROJECT_COLUMNS: &str =
    "p.id, p.team_id, COALESCE(p.organization_id, ''), COALESCE(o.name, ''), \
     p.name, p.source_type, p.source, p.revision, p.credential_id, p.created_at, p.admin_saved, \
     p.last_sync_at, p.last_sync_commit, p.last_sync_status";

const INVENTORY_COLUMNS: &str =
    "id, team_id, name, body, created_at, source_kind, source_json, credential_id, admin_saved";

const SOURCE_COLUMNS: &str = "id, inventory_id, name, kind, source_json, credential_id, body, \
     position, created_at";

const CREDENTIAL_COLUMNS: &str = "id, team_id, name, kind, secret_blob, created_at, auto, disabled";

const TEMPLATE_COLUMNS: &str =
    "id, team_id, name, description, tool, project_id, inventory_id, credential_id, \
     environment_id, no_environment, entrypoint, extra_vars, spec, created_at, update_on_launch, \
     admin_saved, prompt_on_launch";

const HOOK_COLUMNS: &str = "id, team_id, name, token, match_alertname, match_status, template_id, \
     workflow_id, extra_vars, enabled, created_at";

const WORKFLOW_COLUMNS: &str = "id, team_id, name, description, graph, created_at, admin_saved";

const DELIVERY_COLUMNS: &str =
    "d.id, d.hook_id, h.name, d.status, d.summary, d.job_id, d.workflow_job_id, d.created_at";

const SCHEDULE_COLUMNS: &str =
    "id, team_id, name, target_kind, target_id, timezone, start_date, start_time, frequency, \
     weekdays, end_kind, end_count, end_date, enabled, next_run_at, missed_at, occurrence_count, \
     created_by, created_at, updated_at, updated_by, launch_block";

fn row_organization(row: &tokio_postgres::Row) -> AppResult<Organization> {
    Ok(Organization {
        id: row.try_get(0)?,
        name: row.try_get(1)?,
        description: row.try_get(2)?,
        created_at: row.try_get(3)?,
    })
}

fn row_member(row: &tokio_postgres::Row) -> AppResult<Member> {
    Ok(Member {
        user_id: row.try_get(0)?,
        username: row.try_get(1)?,
        display_name: row.try_get(2)?,
        role: row.try_get(3)?,
        source: row.try_get(4)?,
    })
}

fn row_project(row: &tokio_postgres::Row) -> AppResult<Project> {
    let admin_saved: i32 = row.try_get(10)?;
    Ok(Project {
        id: row.try_get(0)?,
        team_id: row.try_get(1)?,
        organization_id: row.try_get(2)?,
        organization_name: row.try_get(3)?,
        name: row.try_get(4)?,
        source_type: row.try_get(5)?,
        source: row.try_get(6)?,
        revision: row.try_get(7)?,
        credential_id: row.try_get(8)?,
        created_at: row.try_get(9)?,
        team_ids: Vec::new(),
        admin_saved: admin_saved != 0,
        last_sync_at: row.try_get(11)?,
        last_sync_commit: row.try_get(12)?,
        last_sync_status: row.try_get(13)?,
    })
}

fn row_inventory(row: &tokio_postgres::Row) -> AppResult<Inventory> {
    let admin_saved: i32 = row.try_get(8)?;
    Ok(Inventory {
        id: row.try_get(0)?,
        team_id: row.try_get(1)?,
        name: row.try_get(2)?,
        body: row.try_get(3)?,
        created_at: row.try_get(4)?,
        source_kind: row.try_get(5)?,
        source_json: row.try_get(6)?,
        credential_id: row.try_get(7)?,
        admin_saved: admin_saved != 0,
    })
}

fn row_inventory_source(row: &tokio_postgres::Row) -> AppResult<InventorySource> {
    let position: i32 = row.try_get(7)?;
    Ok(InventorySource {
        id: row.try_get(0)?,
        inventory_id: row.try_get(1)?,
        name: row.try_get(2)?,
        kind: row.try_get(3)?,
        source_json: row.try_get(4)?,
        credential_id: row.try_get(5)?,
        body: row.try_get(6)?,
        position: i64::from(position),
        created_at: row.try_get(8)?,
    })
}

fn row_credential(row: &tokio_postgres::Row) -> AppResult<Credential> {
    let auto: i32 = row.try_get(6)?;
    let disabled: i32 = row.try_get(7)?;
    Ok(Credential {
        id: row.try_get(0)?,
        team_id: row.try_get(1)?,
        name: row.try_get(2)?,
        kind: row.try_get(3)?,
        secret_blob: row.try_get(4)?,
        created_at: row.try_get(5)?,
        auto: auto != 0,
        disabled: disabled != 0,
    })
}

fn row_credential_usage(row: &tokio_postgres::Row) -> AppResult<CredentialUsage> {
    Ok(CredentialUsage {
        inventory_id: row.try_get(0)?,
        inventory_name: row.try_get(1)?,
        source_id: row.try_get(2)?,
        source_name: row.try_get(3)?,
        project_id: row.try_get(4)?,
        project_name: row.try_get(5)?,
    })
}

fn row_template(row: &tokio_postgres::Row) -> AppResult<Template> {
    let no_environment: i32 = row.try_get(9)?;
    let update_on_launch: i32 = row.try_get(14)?;
    let admin_saved: i32 = row.try_get(15)?;
    let prompt_on_launch: i32 = row.try_get(16)?;
    Ok(Template {
        id: row.try_get(0)?,
        team_id: row.try_get(1)?,
        name: row.try_get(2)?,
        description: row.try_get(3)?,
        tool: row.try_get(4)?,
        project_id: row.try_get(5)?,
        inventory_id: row.try_get(6)?,
        credential_id: row.try_get(7)?,
        environment_id: row.try_get(8)?,
        no_environment: no_environment != 0,
        entrypoint: row.try_get(10)?,
        extra_vars: row.try_get(11)?,
        spec: row.try_get(12)?,
        created_at: row.try_get(13)?,
        update_on_launch: update_on_launch != 0,
        admin_saved: admin_saved != 0,
        prompt_on_launch: prompt_on_launch != 0,
    })
}

fn row_hook(row: &tokio_postgres::Row) -> AppResult<Hook> {
    let enabled: i32 = row.try_get(9)?;
    Ok(Hook {
        id: row.try_get(0)?,
        team_id: row.try_get(1)?,
        name: row.try_get(2)?,
        token: row.try_get(3)?,
        match_alertname: row.try_get(4)?,
        match_status: row.try_get(5)?,
        template_id: row.try_get(6)?,
        workflow_id: row.try_get(7)?,
        extra_vars: row.try_get(8)?,
        enabled: enabled != 0,
        created_at: row.try_get(10)?,
    })
}

fn row_workflow(row: &tokio_postgres::Row) -> AppResult<Workflow> {
    let admin_saved: i32 = row.try_get(6)?;
    Ok(Workflow {
        id: row.try_get(0)?,
        team_id: row.try_get(1)?,
        name: row.try_get(2)?,
        description: row.try_get(3)?,
        graph: row.try_get(4)?,
        created_at: row.try_get(5)?,
        admin_saved: admin_saved != 0,
    })
}

fn row_delivery(row: &tokio_postgres::Row) -> AppResult<Delivery> {
    Ok(Delivery {
        id: row.try_get(0)?,
        hook_id: row.try_get(1)?,
        hook_name: row.try_get(2)?,
        status: row.try_get(3)?,
        summary: row.try_get(4)?,
        job_id: row.try_get(5)?,
        workflow_job_id: row.try_get(6)?,
        created_at: row.try_get(7)?,
    })
}

fn row_schedule(row: &tokio_postgres::Row) -> AppResult<Schedule> {
    let enabled: i32 = row.try_get(13)?;
    let occurrence_count: i32 = row.try_get(16)?;
    let end_count: Option<i32> = row.try_get(11)?;
    Ok(Schedule {
        id: row.try_get(0)?,
        team_id: row.try_get(1)?,
        name: row.try_get(2)?,
        target_kind: row.try_get(3)?,
        target_id: row.try_get(4)?,
        timezone: row.try_get(5)?,
        start_date: row.try_get(6)?,
        start_time: row.try_get(7)?,
        frequency: row.try_get(8)?,
        weekdays: row.try_get(9)?,
        end_kind: row.try_get(10)?,
        end_count: end_count.map(i64::from),
        end_date: row.try_get(12)?,
        enabled: enabled != 0,
        next_run_at: row.try_get(14)?,
        missed_at: row.try_get(15)?,
        occurrence_count: i64::from(occurrence_count),
        created_by: row.try_get(17)?,
        created_at: row.try_get(18)?,
        updated_at: row.try_get(19)?,
        updated_by: row.try_get(20)?,
        launch_block: row.try_get(21)?,
    })
}

async fn pg_project_team_ids<C>(client: &C, project_id: &str) -> AppResult<Vec<String>>
where
    C: tokio_postgres::GenericClient + Sync,
{
    let rows = client
        .query(
            "SELECT team_id FROM project_teams WHERE project_id = $1 ORDER BY team_id",
            &[&project_id],
        )
        .await?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

async fn pg_write_project_teams<C>(
    client: &C,
    project_id: &str,
    team_ids: &[String],
) -> AppResult<()>
where
    C: tokio_postgres::GenericClient + Sync,
{
    client
        .execute(
            "DELETE FROM project_teams WHERE project_id = $1",
            &[&project_id],
        )
        .await?;
    let mut seen = std::collections::BTreeSet::new();
    for team_id in team_ids {
        if !seen.insert(team_id.as_str()) {
            continue;
        }
        client
            .execute(
                "INSERT INTO project_teams (project_id, team_id) VALUES ($1, $2)",
                &[&project_id, &team_id],
            )
            .await?;
    }
    Ok(())
}

async fn pg_write_template_credentials<C>(
    client: &C,
    template_id: &str,
    credential_ids: &[String],
) -> AppResult<()>
where
    C: tokio_postgres::GenericClient + Sync,
{
    client
        .execute(
            "DELETE FROM template_credentials WHERE template_id = $1",
            &[&template_id],
        )
        .await?;
    for (position, credential_id) in credential_ids.iter().enumerate() {
        client
            .execute(
                "INSERT INTO template_credentials (template_id, credential_id, position)
                 VALUES ($1, $2, $3)",
                &[&template_id, &credential_id, &(position as i32)],
            )
            .await?;
    }
    Ok(())
}

async fn pg_insert_source<C>(client: &C, source: &InventorySource) -> AppResult<()>
where
    C: tokio_postgres::GenericClient + Sync,
{
    client
        .execute(
            "INSERT INTO inventory_sources (
                id, inventory_id, name, kind, source_json, credential_id, body, position,
                created_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            &[
                &source.id,
                &source.inventory_id,
                &source.name,
                &source.kind,
                &source.source_json,
                &source.credential_id,
                &source.body,
                &(source.position as i32),
                &source.created_at,
            ],
        )
        .await?;
    Ok(())
}

async fn pg_refresh_summary<C>(client: &C, inventory_id: &str) -> AppResult<()>
where
    C: tokio_postgres::GenericClient + Sync,
{
    let rows = client
        .query(
            &format!(
                "SELECT {SOURCE_COLUMNS} FROM inventory_sources
                 WHERE inventory_id = $1 ORDER BY position, created_at"
            ),
            &[&inventory_id],
        )
        .await?;
    let sources = rows
        .iter()
        .map(row_inventory_source)
        .collect::<AppResult<Vec<_>>>()?;
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
    client
        .execute(
            "UPDATE inventories
             SET body = $2, source_kind = $3, source_json = $4, credential_id = $5
             WHERE id = $1",
            &[&inventory_id, &body, &kind, &source_json, &credential_id],
        )
        .await?;
    Ok(())
}

async fn pg_json_references_credential<C>(
    client: &C,
    team_id: &str,
    credential_id: &str,
) -> AppResult<bool>
where
    C: tokio_postgres::GenericClient + Sync,
{
    let rows = client
        .query(
            "SELECT graph FROM workflows WHERE team_id = $1
             UNION ALL SELECT parameters FROM jobs WHERE team_id = $1",
            &[&team_id],
        )
        .await?;
    for row in rows {
        let raw: String = row.get(0);
        match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(value) if json_contains_string(&value, credential_id) => return Ok(true),
            Ok(_) => {}
            Err(_) => return Ok(true),
        }
    }
    Ok(false)
}

fn json_contains_string(value: &serde_json::Value, needle: &str) -> bool {
    match value {
        serde_json::Value::String(text) => text == needle,
        serde_json::Value::Array(items) => {
            items.iter().any(|item| json_contains_string(item, needle))
        }
        serde_json::Value::Object(map) => {
            map.values().any(|item| json_contains_string(item, needle))
        }
        _ => false,
    }
}

impl Backend for PostgresBackend {
    fn engine(&self) -> &'static str {
        "postgresql"
    }

    fn sqlite(&self) -> Option<&Mutex<Connection>> {
        None
    }

    fn export_sqlite(&self, dest: &Path) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            crate::portable::to_sqlite(&client, dest).await
        })
    }

    fn restore_sqlite(&self, source: &Path) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client.batch_execute("BEGIN").await?;
            match crate::portable::copy_from_sqlite(&client, source, true).await {
                Ok(()) => {
                    client.batch_execute("COMMIT").await?;
                    Ok(())
                }
                Err(err) => {
                    let _ = client.batch_execute("ROLLBACK").await;
                    Err(err)
                }
            }
        })
    }

    fn busy(&self) -> AppResult<bool> {
        block_on(async {
            let client = self.client.lock().await;
            let jobs: i64 = client
                .query_one(
                    "SELECT COUNT(*) FROM jobs WHERE status IN ('pending', 'running')",
                    &[],
                )
                .await?
                .get(0);
            let flows: i64 = client
                .query_one(
                    "SELECT COUNT(*) FROM workflow_jobs WHERE status IN ('pending', 'running')",
                    &[],
                )
                .await?
                .get(0);
            Ok(jobs > 0 || flows > 0)
        })
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

    fn user_by_id(&self, id: &str) -> AppResult<Option<User>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    &format!("SELECT {USER_COLUMNS} FROM users WHERE id = $1"),
                    &[&id],
                )
                .await?;
            row.as_ref().map(row_user).transpose()
        })
    }

    fn list_users(&self) -> AppResult<Vec<(User, i64)>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!(
                        "SELECT {USER_COLUMNS},
                                (SELECT COUNT(*) FROM memberships m WHERE m.user_id = users.id)
                         FROM users ORDER BY lower(username)"
                    ),
                    &[],
                )
                .await?;
            rows.iter()
                .map(|row| Ok((row_user(row)?, row.try_get::<_, i64>(9)?)))
                .collect()
        })
    }

    fn update_profile(
        &self,
        id: &str,
        display_name: &str,
        email: Option<&str>,
        password_hash: Option<&str>,
    ) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            if let Some(password_hash) = password_hash {
                client
                    .execute(
                        "UPDATE users SET display_name = $2, email = $3, password_hash = $4
                         WHERE id = $1",
                        &[&id, &display_name, &email, &password_hash],
                    )
                    .await?;
            } else {
                client
                    .execute(
                        "UPDATE users SET display_name = $2, email = $3 WHERE id = $1",
                        &[&id, &display_name, &email],
                    )
                    .await?;
            }
            Ok(())
        })
    }

    fn set_user_email(&self, id: &str, email: Option<&str>) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute("UPDATE users SET email = $2 WHERE id = $1", &[&id, &email])
                .await?;
            Ok(())
        })
    }

    fn update_user(
        &self,
        id: &str,
        display_name: &str,
        is_admin: bool,
        disabled: bool,
        password_hash: Option<&str>,
    ) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            if let Some(password_hash) = password_hash {
                client
                    .execute(
                        "UPDATE users SET display_name = $2, is_admin = $3, disabled = $4,
                            password_hash = $5
                         WHERE id = $1",
                        &[
                            &id,
                            &display_name,
                            &(is_admin as i32),
                            &(disabled as i32),
                            &password_hash,
                        ],
                    )
                    .await?;
            } else {
                client
                    .execute(
                        "UPDATE users SET display_name = $2, is_admin = $3, disabled = $4
                         WHERE id = $1",
                        &[&id, &display_name, &(is_admin as i32), &(disabled as i32)],
                    )
                    .await?;
            }
            Ok(())
        })
    }

    fn delete_user_sessions(&self, user_id: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute("DELETE FROM sessions WHERE user_id = $1", &[&user_id])
                .await?;
            Ok(())
        })
    }

    fn delete_user(&self, id: &str) -> AppResult<bool> {
        block_on(async {
            let client = self.client.lock().await;
            Ok(client
                .execute("DELETE FROM users WHERE id = $1", &[&id])
                .await?
                > 0)
        })
    }

    fn other_active_admins(&self, user_id: &str) -> AppResult<i64> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_one(
                    "SELECT COUNT(*) FROM users
                     WHERE is_admin != 0 AND disabled = 0 AND id != $1",
                    &[&user_id],
                )
                .await?;
            Ok(row.get(0))
        })
    }

    fn create_api_token(
        &self,
        id: &str,
        user_id: &str,
        name: &str,
        prefix: &str,
        token_hash: &str,
        created_at: &str,
    ) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO api_tokens (id, user_id, name, prefix, token_hash, created_at)
                     VALUES ($1, $2, $3, $4, $5, $6)",
                    &[&id, &user_id, &name, &prefix, &token_hash, &created_at],
                )
                .await?;
            Ok(())
        })
    }

    fn list_api_tokens(&self, user_id: &str) -> AppResult<Vec<ApiToken>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    "SELECT id, name, prefix, created_at, last_used_at
                     FROM api_tokens WHERE user_id = $1 ORDER BY created_at, name",
                    &[&user_id],
                )
                .await?;
            rows.iter().map(row_api_token).collect()
        })
    }

    fn delete_api_token(&self, id: &str, user_id: &str) -> AppResult<bool> {
        block_on(async {
            let client = self.client.lock().await;
            Ok(client
                .execute(
                    "DELETE FROM api_tokens WHERE id = $1 AND user_id = $2",
                    &[&id, &user_id],
                )
                .await?
                > 0)
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

    fn team_by_id(&self, id: &str) -> AppResult<Option<Team>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    &format!(
                        "SELECT {TEAM_COLUMNS} FROM teams t
                         LEFT JOIN organizations o ON o.id = t.organization_id WHERE t.id = $1"
                    ),
                    &[&id],
                )
                .await?;
            row.as_ref().map(row_team).transpose()
        })
    }

    fn role_of(&self, team_id: &str, user_id: &str) -> AppResult<Option<String>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    "SELECT role FROM memberships WHERE team_id = $1 AND user_id = $2",
                    &[&team_id, &user_id],
                )
                .await?;
            Ok(row.map(|row| row.get(0)))
        })
    }

    fn insert_team(
        &self,
        id: &str,
        organization_id: &str,
        name: &str,
        description: &str,
        ldap_group: Option<&str>,
    ) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO teams (id, organization_id, name, description, ldap_group, created_at)
                     VALUES ($1, $2, $3, $4, $5, $6)",
                    &[
                        &id,
                        &organization_id,
                        &name,
                        &description,
                        &ldap_group,
                        &db::now(),
                    ],
                )
                .await?;
            Ok(())
        })
    }

    fn update_team(
        &self,
        id: &str,
        name: &str,
        description: &str,
        ldap_group: Option<&str>,
    ) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE teams SET name = $2, description = $3, ldap_group = $4 WHERE id = $1",
                    &[&id, &name, &description, &ldap_group],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn set_team_organization(&self, team_id: &str, organization_id: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE teams SET organization_id = $2 WHERE id = $1",
                    &[&team_id, &organization_id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn insert_organization(&self, id: &str, name: &str, description: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO organizations (id, name, description, created_at)
                     VALUES ($1, $2, $3, $4)",
                    &[&id, &name, &description, &db::now()],
                )
                .await?;
            Ok(())
        })
    }

    fn update_organization(&self, id: &str, name: &str, description: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE organizations SET name = $2, description = $3 WHERE id = $1",
                    &[&id, &name, &description],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn organization(&self, id: &str) -> AppResult<Option<Organization>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    "SELECT id, name, description, created_at FROM organizations WHERE id = $1",
                    &[&id],
                )
                .await?;
            row.as_ref().map(row_organization).transpose()
        })
    }

    fn organization_by_name(&self, name: &str) -> AppResult<Option<Organization>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    "SELECT id, name, description, created_at FROM organizations WHERE name = $1",
                    &[&name],
                )
                .await?;
            row.as_ref().map(row_organization).transpose()
        })
    }

    fn list_organizations(&self, user_id: &str, admin: bool) -> AppResult<Vec<Organization>> {
        let sql = if admin {
            "SELECT id, name, description, created_at FROM organizations ORDER BY name".to_string()
        } else {
            "SELECT DISTINCT o.id, o.name, o.description, o.created_at
             FROM organizations o
             JOIN teams t ON t.organization_id = o.id
             JOIN memberships m ON m.team_id = t.id
             WHERE m.user_id = $1
             ORDER BY o.name"
                .to_string()
        };
        block_on(async {
            let client = self.client.lock().await;
            let rows = if admin {
                client.query(&sql, &[]).await?
            } else {
                client.query(&sql, &[&user_id]).await?
            };
            rows.iter().map(row_organization).collect()
        })
    }

    fn teams_in_organization(&self, organization_id: &str) -> AppResult<Vec<Team>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!(
                        "SELECT {TEAM_COLUMNS} FROM teams t
                         LEFT JOIN organizations o ON o.id = t.organization_id
                         WHERE t.organization_id = $1 ORDER BY t.name"
                    ),
                    &[&organization_id],
                )
                .await?;
            rows.iter().map(row_team).collect()
        })
    }

    fn teams_with_ldap(&self) -> AppResult<Vec<Team>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!(
                        "SELECT {TEAM_COLUMNS} FROM teams t
                         LEFT JOIN organizations o ON o.id = t.organization_id
                         WHERE t.ldap_group IS NOT NULL AND t.ldap_group != ''"
                    ),
                    &[],
                )
                .await?;
            rows.iter().map(row_team).collect()
        })
    }

    fn upsert_membership(
        &self,
        team_id: &str,
        user_id: &str,
        role: &str,
        source: &str,
    ) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO memberships (team_id, user_id, role, source)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT (team_id, user_id) DO UPDATE
                     SET role = excluded.role, source = excluded.source
                     WHERE memberships.source = 'ldap' OR excluded.source = 'manual'",
                    &[&team_id, &user_id, &role, &source],
                )
                .await?;
            Ok(())
        })
    }

    fn grant_manual_membership(&self, team_id: &str, user_id: &str, role: &str) -> AppResult<()> {
        let source = block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    "SELECT source FROM memberships WHERE team_id = $1 AND user_id = $2",
                    &[&team_id, &user_id],
                )
                .await?;
            Ok::<Option<String>, ApiError>(row.map(|row| row.get(0)))
        })?;
        if source.as_deref() == Some("ldap") {
            return Err(ApiError::bad(
                "the_membership_comes_from_ldap_or_active_directory",
            ));
        }
        self.upsert_membership(team_id, user_id, role, "manual")
    }

    fn list_members(&self, team_id: &str) -> AppResult<Vec<Member>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    "SELECT u.id, u.username, u.display_name, m.role, m.source
                     FROM memberships m JOIN users u ON u.id = m.user_id
                     WHERE m.team_id = $1 ORDER BY u.username",
                    &[&team_id],
                )
                .await?;
            rows.iter().map(row_member).collect()
        })
    }

    fn delete_membership(&self, team_id: &str, user_id: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "DELETE FROM memberships WHERE team_id = $1 AND user_id = $2",
                    &[&team_id, &user_id],
                )
                .await?;
            Ok(())
        })
    }

    fn delete_ldap_membership(&self, team_id: &str, user_id: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "DELETE FROM memberships WHERE team_id = $1 AND user_id = $2 AND source = 'ldap'",
                    &[&team_id, &user_id],
                )
                .await?;
            Ok(())
        })
    }

    fn list_projects(&self, team_id: &str) -> AppResult<Vec<Project>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!(
                        "SELECT {PROJECT_COLUMNS}
                         FROM projects p
                         JOIN teams t ON t.id = $1
                         LEFT JOIN organizations o ON o.id = p.organization_id
                         WHERE p.organization_id = t.organization_id
                           AND (
                             NOT EXISTS (SELECT 1 FROM project_teams pt WHERE pt.project_id = p.id)
                             OR EXISTS (
                               SELECT 1 FROM project_teams pt
                               WHERE pt.project_id = p.id AND pt.team_id = t.id
                             )
                           )
                         ORDER BY p.name"
                    ),
                    &[&team_id],
                )
                .await?;
            let mut projects = rows
                .iter()
                .map(row_project)
                .collect::<AppResult<Vec<_>>>()?;
            for project in &mut projects {
                project.team_ids = pg_project_team_ids(&*client, &project.id).await?;
            }
            Ok(projects)
        })
    }

    fn project(&self, id: &str) -> AppResult<Option<Project>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    &format!(
                        "SELECT {PROJECT_COLUMNS} FROM projects p
                         LEFT JOIN organizations o ON o.id = p.organization_id WHERE p.id = $1"
                    ),
                    &[&id],
                )
                .await?;
            let Some(row) = row else {
                return Ok(None);
            };
            let mut project = row_project(&row)?;
            project.team_ids = pg_project_team_ids(&*client, &project.id).await?;
            Ok(Some(project))
        })
    }

    fn project_visible(&self, id: &str, team_id: &str) -> AppResult<bool> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    "SELECT 1 FROM projects p
                     JOIN teams t ON t.id = $2
                     WHERE p.id = $1 AND p.organization_id = t.organization_id
                       AND (
                         NOT EXISTS (SELECT 1 FROM project_teams pt WHERE pt.project_id = p.id)
                         OR EXISTS (
                           SELECT 1 FROM project_teams pt
                           WHERE pt.project_id = p.id AND pt.team_id = t.id
                         )
                       )",
                    &[&id, &team_id],
                )
                .await?;
            Ok(row.is_some())
        })
    }

    fn insert_project(&self, project: &Project) -> AppResult<()> {
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            transaction
                .execute(
                    "INSERT INTO projects (
                        id, team_id, organization_id, name, source_type, source, revision,
                        credential_id, created_at, admin_saved
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                    &[
                        &project.id,
                        &project.team_id,
                        &project.organization_id,
                        &project.name,
                        &project.source_type,
                        &project.source,
                        &project.revision,
                        &project.credential_id,
                        &project.created_at,
                        &(project.admin_saved as i32),
                    ],
                )
                .await?;
            pg_write_project_teams(&transaction, &project.id, &project.team_ids).await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn delete_project(&self, id: &str, team_id: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "DELETE FROM projects p
                     WHERE p.id = $1
                       AND EXISTS (
                         SELECT 1 FROM teams t
                         WHERE t.id = $2 AND t.organization_id = p.organization_id
                           AND (
                             NOT EXISTS (
                               SELECT 1 FROM project_teams pt WHERE pt.project_id = p.id
                             )
                             OR EXISTS (
                               SELECT 1 FROM project_teams pt
                               WHERE pt.project_id = p.id AND pt.team_id = t.id
                             )
                           )
                       )",
                    &[&id, &team_id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn set_project_details(
        &self,
        id: &str,
        name: &str,
        source_type: &str,
        source: &str,
        revision: &str,
    ) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE projects
                     SET name = $2, source_type = $3, source = $4, revision = $5
                     WHERE id = $1",
                    &[&id, &name, &source_type, &source, &revision],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn set_project_organization(&self, id: &str, organization_id: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE projects SET organization_id = $2 WHERE id = $1",
                    &[&id, &organization_id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn replace_project_teams(&self, project_id: &str, team_ids: &[String]) -> AppResult<()> {
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            pg_write_project_teams(&transaction, project_id, team_ids).await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn set_project_credential(&self, id: &str, credential_id: Option<&str>) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE projects SET credential_id = $2 WHERE id = $1",
                    &[&id, &credential_id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn set_project_admin_saved(&self, id: &str, admin_saved: bool) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE projects SET admin_saved = $2 WHERE id = $1",
                    &[&id, &(admin_saved as i32)],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn set_project_sync_success(&self, id: &str, at: &str, commit: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE projects
                     SET last_sync_at = $2, last_sync_commit = $3, last_sync_status = 'success'
                     WHERE id = $1",
                    &[&id, &at, &commit],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn set_project_sync_failed(&self, id: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE projects SET last_sync_status = 'failed' WHERE id = $1",
                    &[&id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn list_inventories(&self, team_id: &str) -> AppResult<Vec<Inventory>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!(
                        "SELECT {INVENTORY_COLUMNS} FROM inventories
                         WHERE team_id = $1 ORDER BY name"
                    ),
                    &[&team_id],
                )
                .await?;
            rows.iter().map(row_inventory).collect()
        })
    }

    fn inventory(&self, id: &str) -> AppResult<Option<Inventory>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    &format!("SELECT {INVENTORY_COLUMNS} FROM inventories WHERE id = $1"),
                    &[&id],
                )
                .await?;
            row.as_ref().map(row_inventory).transpose()
        })
    }

    fn insert_inventory(
        &self,
        item: &Inventory,
        sources: &[InventorySource],
        refresh: bool,
    ) -> AppResult<()> {
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            transaction
                .execute(
                    "INSERT INTO inventories (
                        id, team_id, name, body, created_at, source_kind, source_json,
                        credential_id, admin_saved
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                    &[
                        &item.id,
                        &item.team_id,
                        &item.name,
                        &item.body,
                        &item.created_at,
                        &item.source_kind,
                        &item.source_json,
                        &item.credential_id,
                        &(item.admin_saved as i32),
                    ],
                )
                .await?;
            for source in sources {
                pg_insert_source(&transaction, source).await?;
            }
            if refresh {
                pg_refresh_summary(&transaction, &item.id).await?;
            }
            transaction.commit().await?;
            Ok(())
        })
    }

    fn update_inventory(&self, id: &str, team_id: &str, name: &str, body: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE inventories SET name = $3, body = $4
                     WHERE id = $1 AND team_id = $2",
                    &[&id, &team_id, &name, &body],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn set_inventory_admin_saved(&self, id: &str, admin_saved: bool) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "UPDATE inventories SET admin_saved = $2 WHERE id = $1",
                    &[&id, &(admin_saved as i32)],
                )
                .await?;
            Ok(())
        })
    }

    fn set_inventory_body(&self, id: &str, team_id: &str, body: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE inventories SET body = $3 WHERE id = $1 AND team_id = $2",
                    &[&id, &team_id, &body],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn list_inventory_sources(&self, inventory_id: &str) -> AppResult<Vec<InventorySource>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!(
                        "SELECT {SOURCE_COLUMNS} FROM inventory_sources
                         WHERE inventory_id = $1 ORDER BY position, created_at"
                    ),
                    &[&inventory_id],
                )
                .await?;
            rows.iter().map(row_inventory_source).collect()
        })
    }

    fn inventory_source(&self, id: &str) -> AppResult<Option<InventorySource>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    &format!("SELECT {SOURCE_COLUMNS} FROM inventory_sources WHERE id = $1"),
                    &[&id],
                )
                .await?;
            row.as_ref().map(row_inventory_source).transpose()
        })
    }

    fn add_inventory_source(
        &self,
        inventory_id: &str,
        source: &InventorySource,
        admin_saved: bool,
    ) -> AppResult<()> {
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            pg_insert_source(&transaction, source).await?;
            pg_refresh_summary(&transaction, inventory_id).await?;
            transaction
                .execute(
                    "UPDATE inventories SET admin_saved = $2 WHERE id = $1",
                    &[&inventory_id, &(admin_saved as i32)],
                )
                .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn set_inventory_source_body(&self, id: &str, body: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "UPDATE inventory_sources SET body = $2 WHERE id = $1",
                    &[&id, &body],
                )
                .await?;
            Ok(())
        })
    }

    fn update_inventory_source(
        &self,
        source: &InventorySource,
        admin_saved: bool,
    ) -> AppResult<()> {
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            let changed = transaction
                .execute(
                    "UPDATE inventory_sources
                     SET name = $2, source_json = $3, credential_id = $4, body = $5
                     WHERE id = $1",
                    &[
                        &source.id,
                        &source.name,
                        &source.source_json,
                        &source.credential_id,
                        &source.body,
                    ],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            pg_refresh_summary(&transaction, &source.inventory_id).await?;
            transaction
                .execute(
                    "UPDATE inventories SET admin_saved = $2 WHERE id = $1",
                    &[&source.inventory_id, &(admin_saved as i32)],
                )
                .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn delete_inventory_source(
        &self,
        id: &str,
        inventory_id: &str,
        admin_saved: bool,
    ) -> AppResult<()> {
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            let changed = transaction
                .execute(
                    "DELETE FROM inventory_sources WHERE id = $1 AND inventory_id = $2",
                    &[&id, &inventory_id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            pg_refresh_summary(&transaction, inventory_id).await?;
            transaction
                .execute(
                    "UPDATE inventories SET admin_saved = $2 WHERE id = $1",
                    &[&inventory_id, &(admin_saved as i32)],
                )
                .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn refresh_inventory_summary(&self, inventory_id: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            pg_refresh_summary(&*client, inventory_id).await
        })
    }

    fn sync_inventory_sources(
        &self,
        inventory_id: &str,
        bodies: &[(String, String)],
        admin_saved: bool,
    ) -> AppResult<()> {
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            for (source_id, body) in bodies {
                transaction
                    .execute(
                        "UPDATE inventory_sources SET body = $2 WHERE id = $1",
                        &[source_id, body],
                    )
                    .await?;
            }
            pg_refresh_summary(&transaction, inventory_id).await?;
            transaction
                .execute(
                    "UPDATE inventories SET admin_saved = $2 WHERE id = $1",
                    &[&inventory_id, &(admin_saved as i32)],
                )
                .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn delete_inventory(&self, id: &str, team_id: &str) -> AppResult<()> {
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            let rows = transaction
                .query(
                    "SELECT credential_id FROM inventory_sources
                     WHERE inventory_id = $1 AND credential_id IS NOT NULL",
                    &[&id],
                )
                .await?;
            let credential_ids: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
            let changed = transaction
                .execute(
                    "DELETE FROM inventories WHERE id = $1 AND team_id = $2",
                    &[&id, &team_id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            for credential_id in credential_ids {
                if pg_json_references_credential(&transaction, team_id, &credential_id).await? {
                    continue;
                }
                transaction
                    .execute(
                        "DELETE FROM credentials
                         WHERE id = $1 AND team_id = $2 AND auto = 1
                           AND NOT EXISTS (
                             SELECT 1 FROM inventory_sources s WHERE s.credential_id = $1
                           )
                           AND NOT EXISTS (
                             SELECT 1 FROM inventories i WHERE i.credential_id = $1
                           )
                           AND NOT EXISTS (
                             SELECT 1 FROM templates t WHERE t.credential_id = $1
                           )
                           AND NOT EXISTS (
                             SELECT 1 FROM template_credentials tc WHERE tc.credential_id = $1
                           )",
                        &[&credential_id, &team_id],
                    )
                    .await?;
            }
            transaction.commit().await?;
            Ok(())
        })
    }

    fn templates_using_inventory(
        &self,
        inventory_id: &str,
        team_id: &str,
    ) -> AppResult<Vec<String>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    "SELECT name FROM templates
                     WHERE inventory_id = $1 AND team_id = $2 ORDER BY name",
                    &[&inventory_id, &team_id],
                )
                .await?;
            Ok(rows.iter().map(|row| row.get(0)).collect())
        })
    }

    fn workflows_using_inventory(
        &self,
        inventory_id: &str,
        team_id: &str,
    ) -> AppResult<Vec<String>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    "SELECT name, graph FROM workflows WHERE team_id = $1",
                    &[&team_id],
                )
                .await?;
            let mut names = Vec::new();
            for row in rows {
                let name: String = row.get(0);
                let graph: String = row.get(1);
                let referenced = match serde_json::from_str::<crate::workflow::Graph>(&graph) {
                    Ok(graph) => graph
                        .nodes
                        .iter()
                        .any(|node| node.overrides.inventory_id.as_deref() == Some(inventory_id)),
                    Err(_) => true,
                };
                if referenced {
                    names.push(name);
                }
            }
            Ok(names)
        })
    }

    fn list_credentials(&self, team_id: &str) -> AppResult<Vec<Credential>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    "SELECT id, team_id, name, kind, '' AS secret_blob, created_at, auto, disabled
                     FROM credentials WHERE team_id = $1 AND disabled = 0 ORDER BY name",
                    &[&team_id],
                )
                .await?;
            rows.iter().map(row_credential).collect()
        })
    }

    fn credential(&self, id: &str) -> AppResult<Option<Credential>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    &format!("SELECT {CREDENTIAL_COLUMNS} FROM credentials WHERE id = $1"),
                    &[&id],
                )
                .await?;
            row.as_ref().map(row_credential).transpose()
        })
    }

    fn insert_credential(&self, item: &Credential) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO credentials
                        (id, team_id, name, kind, secret_blob, created_at, auto, disabled)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                    &[
                        &item.id,
                        &item.team_id,
                        &item.name,
                        &item.kind,
                        &item.secret_blob,
                        &item.created_at,
                        &(item.auto as i32),
                        &(item.disabled as i32),
                    ],
                )
                .await?;
            Ok(())
        })
    }

    fn update_credential(&self, item: &Credential) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE credentials SET name = $3, secret_blob = $4, disabled = $5
                     WHERE id = $1 AND team_id = $2",
                    &[
                        &item.id,
                        &item.team_id,
                        &item.name,
                        &item.secret_blob,
                        &(item.disabled as i32),
                    ],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn delete_credential(&self, id: &str, team_id: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "DELETE FROM credentials WHERE id = $1 AND team_id = $2",
                    &[&id, &team_id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn credential_usages(
        &self,
        credential_id: &str,
        team_id: &str,
    ) -> AppResult<Vec<CredentialUsage>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    "SELECT i.id, i.name, s.id, s.name, NULL::TEXT, NULL::TEXT
                     FROM inventory_sources s
                     JOIN inventories i ON i.id = s.inventory_id
                     WHERE s.credential_id = $1 AND i.team_id = $2
                     UNION ALL
                     SELECT NULL::TEXT, NULL::TEXT, NULL::TEXT, NULL::TEXT, p.id, p.name
                     FROM projects p
                     WHERE p.credential_id = $1
                       AND (
                         p.team_id = $2
                         OR EXISTS (
                           SELECT 1 FROM project_teams pt
                           WHERE pt.project_id = p.id AND pt.team_id = $2
                         )
                       )
                     ORDER BY 2, 4, 6",
                    &[&credential_id, &team_id],
                )
                .await?;
            rows.iter().map(row_credential_usage).collect()
        })
    }

    fn list_environments(&self, team_id: &str) -> AppResult<Vec<Environment>> {
        let team = self.team_by_id(team_id)?.ok_or(ApiError::NotFound)?;
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    "SELECT e.id, e.team_id, e.organization_id, e.name, e.image, e.pull_policy,
                            e.network, e.description, e.created_at, COALESCE(o.name, ''),
                            COALESCE(t.name, ''), e.tool
                     FROM environments e
                     LEFT JOIN organizations o ON o.id = e.organization_id
                     LEFT JOIN teams t ON t.id = e.team_id
                     WHERE (
                         (e.team_id IS NULL OR e.team_id = '')
                         AND (e.organization_id IS NULL OR e.organization_id = '')
                     ) OR (
                         (e.team_id IS NULL OR e.team_id = '')
                         AND e.organization_id = $1
                         AND $1 != ''
                     ) OR e.team_id = $2
                     ORDER BY
                       CASE
                         WHEN e.team_id IS NOT NULL AND e.team_id != '' THEN 2
                         WHEN e.organization_id IS NOT NULL AND e.organization_id != '' THEN 1
                         ELSE 0
                       END,
                       e.name",
                    &[&team.organization_id, &team.id],
                )
                .await?;
            rows.iter().map(row_environment).collect()
        })
    }

    fn insert_environment(&self, item: &Environment) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO environments (
                        id, team_id, organization_id, name, tool, image, pull_policy, network,
                        description, created_at
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                    &[
                        &item.id,
                        &item.team_id,
                        &item.organization_id,
                        &item.name,
                        &item.tool,
                        &item.image,
                        &item.pull_policy,
                        &item.network,
                        &item.description,
                        &item.created_at,
                    ],
                )
                .await?;
            Ok(())
        })
    }

    fn update_environment(&self, item: &Environment) -> AppResult<()> {
        let global = item.team_id.is_none() && item.organization_id.is_none();
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            let changed = transaction
                .execute(
                    "UPDATE environments
                     SET team_id = $2, organization_id = $3, name = $4, tool = $5, image = $6,
                         pull_policy = $7, network = $8, description = $9
                     WHERE id = $1",
                    &[
                        &item.id,
                        &item.team_id,
                        &item.organization_id,
                        &item.name,
                        &item.tool,
                        &item.image,
                        &item.pull_policy,
                        &item.network,
                        &item.description,
                    ],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            for key in crate::settings::DEFAULT_ENVIRONMENT_KEYS {
                let tool = key.trim_start_matches("default_environment_");
                if global && crate::runner::environment_fits_tool(tool, &item.tool) {
                    continue;
                }
                transaction
                    .execute(
                        "DELETE FROM settings WHERE key = $1 AND value = $2",
                        &[&key, &item.id],
                    )
                    .await?;
            }
            transaction.commit().await?;
            Ok(())
        })
    }

    fn delete_environment(&self, id: &str) -> AppResult<()> {
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            let changed = transaction
                .execute("DELETE FROM environments WHERE id = $1", &[&id])
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            for key in crate::settings::DEFAULT_ENVIRONMENT_KEYS {
                transaction
                    .execute(
                        "DELETE FROM settings WHERE key = $1 AND value = $2",
                        &[&key, &id],
                    )
                    .await?;
            }
            transaction.commit().await?;
            Ok(())
        })
    }

    fn list_templates(&self, team_id: &str) -> AppResult<Vec<Template>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!(
                        "SELECT {TEMPLATE_COLUMNS} FROM templates WHERE team_id = $1 ORDER BY name"
                    ),
                    &[&team_id],
                )
                .await?;
            rows.iter().map(row_template).collect()
        })
    }

    fn template(&self, id: &str) -> AppResult<Option<Template>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    &format!("SELECT {TEMPLATE_COLUMNS} FROM templates WHERE id = $1"),
                    &[&id],
                )
                .await?;
            row.as_ref().map(row_template).transpose()
        })
    }

    fn insert_template(&self, item: &Template, credential_ids: &[String]) -> AppResult<()> {
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            transaction
                .execute(
                    "INSERT INTO templates (
                        id, team_id, name, description, tool, project_id, inventory_id,
                        credential_id, environment_id, no_environment, entrypoint, extra_vars,
                        spec, created_at, update_on_launch, admin_saved, prompt_on_launch
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                        $15, $16, $17)",
                    &[
                        &item.id,
                        &item.team_id,
                        &item.name,
                        &item.description,
                        &item.tool,
                        &item.project_id,
                        &item.inventory_id,
                        &item.credential_id,
                        &item.environment_id,
                        &(item.no_environment as i32),
                        &item.entrypoint,
                        &item.extra_vars,
                        &item.spec,
                        &item.created_at,
                        &(item.update_on_launch as i32),
                        &(item.admin_saved as i32),
                        &(item.prompt_on_launch as i32),
                    ],
                )
                .await?;
            pg_write_template_credentials(&transaction, &item.id, credential_ids).await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn update_template(&self, item: &Template, credential_ids: &[String]) -> AppResult<()> {
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            let changed = transaction
                .execute(
                    "UPDATE templates SET name = $3, description = $4, tool = $5,
                        project_id = $6, inventory_id = $7, credential_id = $8,
                        environment_id = $9, no_environment = $10, entrypoint = $11,
                        extra_vars = $12, spec = $13, update_on_launch = $14, admin_saved = $15,
                        prompt_on_launch = $16
                     WHERE id = $1 AND team_id = $2",
                    &[
                        &item.id,
                        &item.team_id,
                        &item.name,
                        &item.description,
                        &item.tool,
                        &item.project_id,
                        &item.inventory_id,
                        &item.credential_id,
                        &item.environment_id,
                        &(item.no_environment as i32),
                        &item.entrypoint,
                        &item.extra_vars,
                        &item.spec,
                        &(item.update_on_launch as i32),
                        &(item.admin_saved as i32),
                        &(item.prompt_on_launch as i32),
                    ],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            pg_write_template_credentials(&transaction, &item.id, credential_ids).await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn delete_template(&self, id: &str, team_id: &str) -> AppResult<()> {
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            let changed = transaction
                .execute(
                    "DELETE FROM templates WHERE id = $1 AND team_id = $2",
                    &[&id, &team_id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            transaction
                .execute(
                    "DELETE FROM schedules WHERE target_kind = 'template' AND target_id = $1",
                    &[&id],
                )
                .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn template_credential_ids(&self, template_id: &str) -> AppResult<Vec<String>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    "SELECT credential_id FROM template_credentials
                     WHERE template_id = $1 ORDER BY position, credential_id",
                    &[&template_id],
                )
                .await?;
            Ok(rows.iter().map(|row| row.get(0)).collect())
        })
    }

    fn list_template_credential_links(&self, team_id: &str) -> AppResult<Vec<(String, String)>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    "SELECT tc.template_id, tc.credential_id
                     FROM template_credentials tc
                     JOIN templates t ON t.id = tc.template_id
                     WHERE t.team_id = $1
                     ORDER BY tc.template_id, tc.position, tc.credential_id",
                    &[&team_id],
                )
                .await?;
            Ok(rows.iter().map(|row| (row.get(0), row.get(1))).collect())
        })
    }

    fn list_hooks(&self, team_id: &str) -> AppResult<Vec<Hook>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!("SELECT {HOOK_COLUMNS} FROM hooks WHERE team_id = $1 ORDER BY name"),
                    &[&team_id],
                )
                .await?;
            rows.iter().map(row_hook).collect()
        })
    }

    fn hook(&self, id: &str) -> AppResult<Option<Hook>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    &format!("SELECT {HOOK_COLUMNS} FROM hooks WHERE id = $1"),
                    &[&id],
                )
                .await?;
            row.as_ref().map(row_hook).transpose()
        })
    }

    fn insert_hook(&self, item: &Hook) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO hooks (
                        id, team_id, name, token, match_alertname, match_status, template_id,
                        workflow_id, extra_vars, enabled, created_at
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                    &[
                        &item.id,
                        &item.team_id,
                        &item.name,
                        &item.token,
                        &item.match_alertname,
                        &item.match_status,
                        &item.template_id,
                        &item.workflow_id,
                        &item.extra_vars,
                        &(item.enabled as i32),
                        &item.created_at,
                    ],
                )
                .await?;
            Ok(())
        })
    }

    fn update_hook(&self, item: &Hook) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE hooks SET name = $3, match_alertname = $4, match_status = $5,
                        template_id = $6, workflow_id = $7, extra_vars = $8
                     WHERE id = $1 AND team_id = $2",
                    &[
                        &item.id,
                        &item.team_id,
                        &item.name,
                        &item.match_alertname,
                        &item.match_status,
                        &item.template_id,
                        &item.workflow_id,
                        &item.extra_vars,
                    ],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn delete_hook(&self, id: &str, team_id: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "DELETE FROM hooks WHERE id = $1 AND team_id = $2",
                    &[&id, &team_id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn list_deliveries(&self, team_id: &str, limit: i64) -> AppResult<Vec<Delivery>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!(
                        "SELECT {DELIVERY_COLUMNS}
                         FROM hook_deliveries d JOIN hooks h ON h.id = d.hook_id
                         WHERE h.team_id = $1 ORDER BY d.created_at DESC LIMIT $2"
                    ),
                    &[&team_id, &limit],
                )
                .await?;
            rows.iter().map(row_delivery).collect()
        })
    }

    fn insert_delivery(&self, item: &NewDelivery<'_>) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO hook_deliveries (
                        id, hook_id, status, summary, payload, job_id, workflow_job_id, created_at
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                    &[
                        &item.id,
                        &item.hook_id,
                        &item.status,
                        &item.summary,
                        &item.payload,
                        &item.job_id,
                        &item.workflow_job_id,
                        &db::now(),
                    ],
                )
                .await?;
            Ok(())
        })
    }

    fn job(&self, id: &str) -> AppResult<Option<Job>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    &format!("SELECT {JOB_COLUMNS} FROM jobs WHERE id = $1"),
                    &[&id],
                )
                .await?;
            row.as_ref().map(row_job).transpose()
        })
    }

    fn list_jobs(&self, team_id: &str, query: &JobQuery) -> AppResult<(Vec<Job>, i64)> {
        let limit = query.limit.clamp(1, 100);
        let offset = query.offset.max(0);
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!(
                        "SELECT {JOB_COLUMNS} FROM jobs WHERE {JOB_FILTER}
                         ORDER BY created_at DESC, id DESC LIMIT $9 OFFSET $10"
                    ),
                    &[
                        &team_id,
                        &query.q,
                        &query.status,
                        &query.tool,
                        &query.trigger,
                        &query.template_id,
                        &query.since,
                        &query.until,
                        &limit,
                        &offset,
                    ],
                )
                .await?;
            let items = rows.iter().map(row_job).collect::<AppResult<Vec<_>>>()?;
            let total = client
                .query_one(
                    &format!("SELECT COUNT(*) FROM jobs WHERE {JOB_FILTER}"),
                    &[
                        &team_id,
                        &query.q,
                        &query.status,
                        &query.tool,
                        &query.trigger,
                        &query.template_id,
                        &query.since,
                        &query.until,
                    ],
                )
                .await?
                .get(0);
            Ok((items, total))
        })
    }

    fn insert_job(&self, item: &Job) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO jobs (
                        id, team_id, template_id, workflow_job_id, workflow_node_id, name, tool,
                        status, extra_vars, launched_by, trigger, started_at, finished_at,
                        created_at, parameters, failed_hosts
                     ) VALUES (
                        $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                        $16
                     )",
                    &[
                        &item.id,
                        &item.team_id,
                        &item.template_id,
                        &item.workflow_job_id,
                        &item.workflow_node_id,
                        &item.name,
                        &item.tool,
                        &item.status,
                        &item.extra_vars,
                        &item.launched_by,
                        &item.trigger,
                        &item.started_at,
                        &item.finished_at,
                        &item.created_at,
                        &item.parameters,
                        &item.failed_hosts,
                    ],
                )
                .await?;
            Ok(())
        })
    }

    fn mark_running(&self, id: &str) -> AppResult<bool> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE jobs SET status = 'running', started_at = COALESCE(started_at, $2)
                     WHERE id = $1 AND status = 'pending'",
                    &[&id, &db::now()],
                )
                .await?;
            Ok(changed == 1)
        })
    }

    fn set_failed_hosts(&self, id: &str, failed_hosts: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "UPDATE jobs SET failed_hosts = $2 WHERE id = $1",
                    &[&id, &failed_hosts],
                )
                .await?;
            Ok(())
        })
    }

    fn set_job_parameters(&self, id: &str, parameters: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "UPDATE jobs SET parameters = $2 WHERE id = $1",
                    &[&id, &parameters],
                )
                .await?;
            Ok(())
        })
    }

    fn finish_job(&self, id: &str, status: &str) -> AppResult<bool> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE jobs SET status = $2, finished_at = $3
                     WHERE id = $1 AND status IN ('pending', 'running')",
                    &[&id, &status, &db::now()],
                )
                .await?;
            Ok(changed == 1)
        })
    }

    fn insert_event(&self, job_id: &str, stream: &str, line: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO job_events (job_id, ts, stream, line) VALUES ($1, $2, $3, $4)",
                    &[&job_id, &db::now(), &stream, &line],
                )
                .await?;
            Ok(())
        })
    }

    fn events_after(&self, job_id: &str, after: i64, limit: i64) -> AppResult<Vec<JobEvent>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    "SELECT id, ts, stream, line FROM job_events
                     WHERE job_id = $1 AND id > $2 ORDER BY id LIMIT $3",
                    &[&job_id, &after, &limit],
                )
                .await?;
            rows.iter().map(row_job_event).collect()
        })
    }

    fn events_count_between(&self, job_id: &str, after: i64, before: i64) -> AppResult<i64> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_one(
                    "SELECT COUNT(*) FROM job_events
                     WHERE job_id = $1 AND id > $2 AND id < $3",
                    &[&job_id, &after, &before],
                )
                .await?;
            Ok(row.get(0))
        })
    }

    fn events_bytes(&self, job_id: &str) -> AppResult<i64> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_one(
                    "SELECT COALESCE(SUM(octet_length(line) + 1), 0)
                     FROM job_events WHERE job_id = $1",
                    &[&job_id],
                )
                .await?;
            Ok(row.get(0))
        })
    }

    fn events_last(&self, job_id: &str, limit: i64) -> AppResult<Vec<JobEvent>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    "SELECT id, ts, stream, line FROM job_events
                     WHERE job_id = $1 ORDER BY id DESC LIMIT $2",
                    &[&job_id, &limit],
                )
                .await?;
            let mut events = rows
                .iter()
                .map(row_job_event)
                .collect::<AppResult<Vec<_>>>()?;
            events.reverse();
            Ok(events)
        })
    }

    fn workflow_job(&self, id: &str) -> AppResult<Option<WorkflowJob>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    "SELECT id, workflow_id, team_id, name, status, extra_vars, launched_by,
                            trigger, created_at, finished_at
                     FROM workflow_jobs WHERE id = $1",
                    &[&id],
                )
                .await?;
            row.as_ref().map(row_workflow_job).transpose()
        })
    }

    fn insert_workflow_job(&self, item: &WorkflowJob) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO workflow_jobs (
                        id, workflow_id, team_id, name, status, extra_vars, launched_by,
                        trigger, created_at, finished_at
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                    &[
                        &item.id,
                        &item.workflow_id,
                        &item.team_id,
                        &item.name,
                        &item.status,
                        &item.extra_vars,
                        &item.launched_by,
                        &item.trigger,
                        &item.created_at,
                        &item.finished_at,
                    ],
                )
                .await?;
            Ok(())
        })
    }

    fn set_workflow_status(&self, id: &str, status: &str, finish: bool) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            if finish {
                client
                    .execute(
                        "UPDATE workflow_jobs SET status = $2, finished_at = $3 WHERE id = $1",
                        &[&id, &status, &db::now()],
                    )
                    .await?;
            } else {
                client
                    .execute(
                        "UPDATE workflow_jobs SET status = $2 WHERE id = $1",
                        &[&id, &status],
                    )
                    .await?;
            }
            Ok(())
        })
    }

    fn insert_workflow_node(&self, workflow_job_id: &str, node_id: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO workflow_nodes (workflow_job_id, node_id, phase, job_id)
                     VALUES ($1, $2, 'pending', NULL)",
                    &[&workflow_job_id, &node_id],
                )
                .await?;
            Ok(())
        })
    }

    fn set_workflow_node(
        &self,
        workflow_job_id: &str,
        node_id: &str,
        phase: &str,
        job_id: Option<&str>,
    ) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "UPDATE workflow_nodes SET phase = $3, job_id = COALESCE($4, job_id)
                     WHERE workflow_job_id = $1 AND node_id = $2",
                    &[&workflow_job_id, &node_id, &phase, &job_id],
                )
                .await?;
            Ok(())
        })
    }

    fn workflow_nodes(&self, workflow_job_id: &str) -> AppResult<Vec<WorkflowNodeRow>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    "SELECT node_id, phase, job_id FROM workflow_nodes
                     WHERE workflow_job_id = $1",
                    &[&workflow_job_id],
                )
                .await?;
            rows.iter().map(row_workflow_node).collect()
        })
    }

    fn recent_jobs(&self, team_id: &str, limit: i64) -> AppResult<Vec<Job>> {
        let limit = limit.clamp(1, 100);
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!(
                        "SELECT {JOB_COLUMNS} FROM jobs WHERE team_id = $1
                         ORDER BY created_at DESC, id DESC LIMIT $2"
                    ),
                    &[&team_id, &limit],
                )
                .await?;
            rows.iter().map(row_job).collect()
        })
    }

    fn job_activity(&self, team_id: &str, from: &str, to: &str) -> AppResult<Vec<JobActivity>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    "SELECT status, started_at, finished_at FROM jobs
                     WHERE team_id = $1
                       AND ((finished_at IS NOT NULL AND finished_at >= $2
                             AND finished_at <= $3)
                         OR (started_at IS NOT NULL AND started_at >= $2
                             AND started_at <= $3))",
                    &[&team_id, &from, &to],
                )
                .await?;
            rows.iter().map(row_job_activity).collect()
        })
    }

    fn import_report(&self) -> AppResult<Option<serde_json::Value>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    "SELECT value FROM settings WHERE key = $1",
                    &[&crate::migration::REPORT_KEY],
                )
                .await?;
            Ok(row.and_then(|row| {
                let raw: String = row.get(0);
                serde_json::from_str(&raw).ok()
            }))
        })
    }

    fn first_workflow(&self, team_id: &str) -> AppResult<Option<Workflow>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    &format!(
                        "SELECT {WORKFLOW_COLUMNS} FROM workflows
                         WHERE team_id = $1 ORDER BY created_at LIMIT 1"
                    ),
                    &[&team_id],
                )
                .await?;
            row.as_ref().map(row_workflow).transpose()
        })
    }

    fn first_hook(&self, team_id: &str) -> AppResult<Option<Hook>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    &format!(
                        "SELECT {HOOK_COLUMNS} FROM hooks
                         WHERE team_id = $1 ORDER BY created_at LIMIT 1"
                    ),
                    &[&team_id],
                )
                .await?;
            row.as_ref().map(row_hook).transpose()
        })
    }

    fn list_workflows(&self, team_id: &str) -> AppResult<Vec<Workflow>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!(
                        "SELECT {WORKFLOW_COLUMNS} FROM workflows
                         WHERE team_id = $1 ORDER BY name"
                    ),
                    &[&team_id],
                )
                .await?;
            rows.iter().map(row_workflow).collect()
        })
    }

    fn workflow(&self, id: &str) -> AppResult<Option<Workflow>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    &format!("SELECT {WORKFLOW_COLUMNS} FROM workflows WHERE id = $1"),
                    &[&id],
                )
                .await?;
            row.as_ref().map(row_workflow).transpose()
        })
    }

    fn insert_workflow(&self, item: &Workflow) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO workflows (
                        id, team_id, name, description, graph, created_at, admin_saved
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    &[
                        &item.id,
                        &item.team_id,
                        &item.name,
                        &item.description,
                        &item.graph,
                        &item.created_at,
                        &(item.admin_saved as i32),
                    ],
                )
                .await?;
            Ok(())
        })
    }

    fn update_workflow(&self, item: &Workflow) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE workflows
                     SET name = $3, description = $4, graph = $5, admin_saved = $6
                     WHERE id = $1 AND team_id = $2",
                    &[
                        &item.id,
                        &item.team_id,
                        &item.name,
                        &item.description,
                        &item.graph,
                        &(item.admin_saved as i32),
                    ],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn delete_workflow(&self, id: &str, team_id: &str) -> AppResult<()> {
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            let changed = transaction
                .execute(
                    "DELETE FROM workflows WHERE id = $1 AND team_id = $2",
                    &[&id, &team_id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            transaction
                .execute(
                    "DELETE FROM schedules WHERE target_kind = 'workflow' AND target_id = $1",
                    &[&id],
                )
                .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn workflow_team(&self, id: &str) -> AppResult<Option<String>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt("SELECT team_id FROM workflows WHERE id = $1", &[&id])
                .await?;
            Ok(row.map(|row| row.get(0)))
        })
    }

    fn list_team_schedules(&self, team_id: &str) -> AppResult<Vec<(Schedule, String)>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!(
                        "SELECT {SCHEDULE_COLUMNS},
                            COALESCE(
                                CASE target_kind
                                    WHEN 'template' THEN (
                                        SELECT name FROM templates WHERE id = schedules.target_id
                                    )
                                    WHEN 'workflow' THEN (
                                        SELECT name FROM workflows WHERE id = schedules.target_id
                                    )
                                    ELSE ''
                                END,
                                ''
                            )
                         FROM schedules
                         WHERE team_id = $1
                         ORDER BY name"
                    ),
                    &[&team_id],
                )
                .await?;
            rows.iter()
                .map(|row| Ok((row_schedule(row)?, row.try_get(22)?)))
                .collect()
        })
    }

    fn schedule(&self, id: &str) -> AppResult<Option<Schedule>> {
        block_on(async {
            let client = self.client.lock().await;
            let row = client
                .query_opt(
                    &format!("SELECT {SCHEDULE_COLUMNS} FROM schedules WHERE id = $1"),
                    &[&id],
                )
                .await?;
            row.as_ref().map(row_schedule).transpose()
        })
    }

    fn list_schedules(&self, kind: &str, target_id: &str) -> AppResult<Vec<Schedule>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!(
                        "SELECT {SCHEDULE_COLUMNS} FROM schedules
                         WHERE target_kind = $1 AND target_id = $2 ORDER BY name"
                    ),
                    &[&kind, &target_id],
                )
                .await?;
            rows.iter().map(row_schedule).collect()
        })
    }

    fn insert_schedule(&self, item: &Schedule) -> AppResult<()> {
        let end_count = item.end_count.map(|count| count as i32);
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "INSERT INTO schedules (
                        id, team_id, name, target_kind, target_id, timezone, start_date, start_time,
                        frequency, weekdays, end_kind, end_count, end_date, enabled, next_run_at,
                        missed_at, occurrence_count, created_by, created_at, updated_at, updated_by,
                        launch_block
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                        $15, $16, $17, $18, $19, $20, $21, $22)",
                    &[
                        &item.id,
                        &item.team_id,
                        &item.name,
                        &item.target_kind,
                        &item.target_id,
                        &item.timezone,
                        &item.start_date,
                        &item.start_time,
                        &item.frequency,
                        &item.weekdays,
                        &item.end_kind,
                        &end_count,
                        &item.end_date,
                        &(item.enabled as i32),
                        &item.next_run_at,
                        &item.missed_at,
                        &(item.occurrence_count as i32),
                        &item.created_by,
                        &item.created_at,
                        &item.updated_at,
                        &item.updated_by,
                        &item.launch_block,
                    ],
                )
                .await?;
            Ok(())
        })
    }

    fn update_schedule(&self, item: &Schedule) -> AppResult<()> {
        let end_count = item.end_count.map(|count| count as i32);
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE schedules SET
                        name = $2, timezone = $3, start_date = $4, start_time = $5, frequency = $6,
                        weekdays = $7, end_kind = $8, end_count = $9, end_date = $10,
                        enabled = $11, next_run_at = $12, missed_at = $13, occurrence_count = $14,
                        updated_at = $15, updated_by = $16, launch_block = $17
                     WHERE id = $1 AND team_id = $18",
                    &[
                        &item.id,
                        &item.name,
                        &item.timezone,
                        &item.start_date,
                        &item.start_time,
                        &item.frequency,
                        &item.weekdays,
                        &item.end_kind,
                        &end_count,
                        &item.end_date,
                        &(item.enabled as i32),
                        &item.next_run_at,
                        &item.missed_at,
                        &(item.occurrence_count as i32),
                        &item.updated_at,
                        &item.updated_by,
                        &item.launch_block,
                        &item.team_id,
                    ],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn delete_schedule(&self, id: &str, team_id: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "DELETE FROM schedules WHERE id = $1 AND team_id = $2",
                    &[&id, &team_id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn set_schedule_enabled(
        &self,
        id: &str,
        team_id: &str,
        enabled: bool,
        next_run_at: Option<&str>,
        occurrence_count: i64,
    ) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            let changed = client
                .execute(
                    "UPDATE schedules SET enabled = $3, next_run_at = $4,
                        occurrence_count = $5, updated_at = $6
                     WHERE id = $1 AND team_id = $2",
                    &[
                        &id,
                        &team_id,
                        &(enabled as i32),
                        &next_run_at,
                        &(occurrence_count as i32),
                        &db::now(),
                    ],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            Ok(())
        })
    }

    fn claim_missed(&self, id: &str, team_id: &str) -> AppResult<Option<String>> {
        block_on(async {
            let mut client = self.client.lock().await;
            let transaction = client.transaction().await?;
            let row = transaction
                .query_opt(
                    "SELECT missed_at FROM schedules
                     WHERE id = $1 AND team_id = $2 AND missed_at IS NOT NULL
                     FOR UPDATE",
                    &[&id, &team_id],
                )
                .await?;
            let Some(row) = row else {
                return Ok(None);
            };
            let missed_at: String = row.get(0);
            transaction
                .execute(
                    "UPDATE schedules SET missed_at = NULL, updated_at = $3
                     WHERE id = $1 AND team_id = $2 AND missed_at IS NOT NULL",
                    &[&id, &team_id, &db::now()],
                )
                .await?;
            transaction.commit().await?;
            Ok(Some(missed_at))
        })
    }

    fn restore_missed(&self, id: &str, team_id: &str, missed_at: &str) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "UPDATE schedules SET missed_at = $3, updated_at = $4
                     WHERE id = $1 AND team_id = $2 AND missed_at IS NULL",
                    &[&id, &team_id, &missed_at, &db::now()],
                )
                .await?;
            Ok(())
        })
    }

    fn due_schedules(&self, now: &str) -> AppResult<Vec<Schedule>> {
        block_on(async {
            let client = self.client.lock().await;
            let rows = client
                .query(
                    &format!(
                        "SELECT {SCHEDULE_COLUMNS} FROM schedules
                         WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= $1
                         ORDER BY next_run_at"
                    ),
                    &[&now],
                )
                .await?;
            rows.iter().map(row_schedule).collect()
        })
    }

    fn advance_schedule(
        &self,
        id: &str,
        next_run_at: Option<&str>,
        occurrence_count: i64,
    ) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "UPDATE schedules SET next_run_at = $2, occurrence_count = $3, updated_at = $4
                     WHERE id = $1",
                    &[&id, &next_run_at, &(occurrence_count as i32), &db::now()],
                )
                .await?;
            Ok(())
        })
    }

    fn note_miss(&self, id: &str, missed_at: Option<&str>) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "UPDATE schedules SET missed_at = $2, updated_at = $3 WHERE id = $1",
                    &[&id, &missed_at, &db::now()],
                )
                .await?;
            Ok(())
        })
    }

    fn set_launch_block(&self, id: &str, reason: Option<&str>) -> AppResult<()> {
        block_on(async {
            let client = self.client.lock().await;
            client
                .execute(
                    "UPDATE schedules SET launch_block = $2, updated_at = $3 WHERE id = $1",
                    &[&id, &reason, &db::now()],
                )
                .await?;
            Ok(())
        })
    }
}
