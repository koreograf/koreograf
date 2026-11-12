use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use deadpool_postgres::{Manager, ManagerConfig, Pool};
use rusqlite::Connection;
use rustls::pki_types::pem::PemObject;
use tokio_postgres::Client;

use crate::config::{DatabaseConfig, PostgresConfig, SslMode};
use crate::db::{
    self, ApiToken, Credential, CredentialUsage, Delivery, Environment, Hook, Inventory,
    InventorySource, Job, JobActivity, JobEvent, JobQuery, Member, NewDelivery, Organization,
    Project, Schedule, Team, Template, User, UserMembership, Workflow, WorkflowJob,
    WorkflowNodeRow,
};
use crate::error::{ApiError, AppResult};
use crate::store;

pub(crate) trait Backend: Send + Sync {
    fn engine(&self) -> &'static str;
    fn sqlite(&self) -> Option<&Mutex<Connection>>;
    fn export_sqlite(&self, dest: &Path) -> AppResult<()>;
    fn restore_sqlite(&self, source: &Path) -> AppResult<()>;
    fn busy(&self) -> AppResult<bool>;
    fn ping(&self) -> AppResult<()>;
    fn settings_rows(&self) -> AppResult<HashMap<String, String>>;
    fn set_setting(&self, key: &str, value: &str) -> AppResult<()>;
    fn count_teams(&self) -> AppResult<i64>;
    fn seed_state(&self, key: &str) -> AppResult<Option<String>>;
    fn set_seed_state(&self, key: &str, value: &str) -> AppResult<()>;
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
        store::settings::read_rows(&conn)
    }

    fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::settings::set_setting(&conn, key, value)
    }

    fn count_teams(&self) -> AppResult<i64> {
        let conn = lock(&self.conn)?;
        store::teams::count_teams(&conn)
    }

    fn seed_state(&self, key: &str) -> AppResult<Option<String>> {
        let conn = lock(&self.conn)?;
        store::settings::seed_state(&conn, key)
    }

    fn set_seed_state(&self, key: &str, value: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::settings::set_seed_state(&conn, key, value)
    }

    fn user_by_username(&self, username: &str) -> AppResult<Option<User>> {
        let conn = lock(&self.conn)?;
        store::auth::user_by_username(&conn, username)
    }

    fn user_by_session(&self, token: &str) -> AppResult<Option<User>> {
        let conn = lock(&self.conn)?;
        store::auth::user_by_session(&conn, token)
    }

    fn user_by_api_token(&self, token_hash: &str) -> AppResult<Option<User>> {
        let conn = lock(&self.conn)?;
        store::auth::user_by_api_token(&conn, token_hash)
    }

    fn create_session(&self, token: &str, user_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::auth::create_session(&conn, token, user_id)
    }

    fn delete_session(&self, token: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::auth::delete_session(&conn, token)
    }

    fn purge_sessions(&self) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::auth::purge_sessions(&conn)
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
        store::auth::insert_user(
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
        store::auth::user_by_id(&conn, id)
    }

    fn list_users(&self) -> AppResult<Vec<(User, i64)>> {
        let conn = lock(&self.conn)?;
        store::auth::list_users(&conn)
    }

    fn update_profile(
        &self,
        id: &str,
        display_name: &str,
        email: Option<&str>,
        password_hash: Option<&str>,
    ) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::auth::update_profile(&conn, id, display_name, email, password_hash)
    }

    fn set_user_email(&self, id: &str, email: Option<&str>) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::auth::set_user_email(&conn, id, email)
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
        store::auth::update_user(&conn, id, display_name, is_admin, disabled, password_hash)
    }

    fn delete_user_sessions(&self, user_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::auth::delete_user_sessions(&conn, user_id)
    }

    fn delete_user(&self, id: &str) -> AppResult<bool> {
        let conn = lock(&self.conn)?;
        store::auth::delete_user(&conn, id)
    }

    fn other_active_admins(&self, user_id: &str) -> AppResult<i64> {
        let conn = lock(&self.conn)?;
        store::auth::other_active_admins(&conn, user_id)
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
        store::auth::create_api_token(&conn, id, user_id, name, prefix, token_hash, created_at)
    }

    fn list_api_tokens(&self, user_id: &str) -> AppResult<Vec<ApiToken>> {
        let conn = lock(&self.conn)?;
        store::auth::list_api_tokens(&conn, user_id)
    }

    fn delete_api_token(&self, id: &str, user_id: &str) -> AppResult<bool> {
        let conn = lock(&self.conn)?;
        store::auth::delete_api_token(&conn, id, user_id)
    }

    fn list_teams_for_user(&self, user_id: &str, admin: bool) -> AppResult<Vec<(Team, String)>> {
        let conn = lock(&self.conn)?;
        store::teams::list_teams_for_user(&conn, user_id, admin)
    }

    fn list_user_memberships(&self, user_id: &str) -> AppResult<Vec<UserMembership>> {
        let conn = lock(&self.conn)?;
        store::teams::list_user_memberships(&conn, user_id)
    }

    fn replace_settings(&self, rows: &[(String, String)]) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        store::settings::replace(&mut conn, rows)
    }

    fn environment(&self, id: &str) -> AppResult<Option<Environment>> {
        let conn = lock(&self.conn)?;
        store::environments::environment(&conn, id)
    }

    fn team_by_id(&self, id: &str) -> AppResult<Option<Team>> {
        let conn = lock(&self.conn)?;
        store::teams::team_by_id(&conn, id)
    }

    fn role_of(&self, team_id: &str, user_id: &str) -> AppResult<Option<String>> {
        let conn = lock(&self.conn)?;
        store::teams::role_of(&conn, team_id, user_id)
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
        store::teams::insert_team(&conn, id, organization_id, name, description, ldap_group)
    }

    fn update_team(
        &self,
        id: &str,
        name: &str,
        description: &str,
        ldap_group: Option<&str>,
    ) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::teams::update_team(&conn, id, name, description, ldap_group)
    }

    fn set_team_organization(&self, team_id: &str, organization_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::teams::set_team_organization(&conn, team_id, organization_id)
    }

    fn insert_organization(&self, id: &str, name: &str, description: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::teams::insert_organization(&conn, id, name, description)
    }

    fn update_organization(&self, id: &str, name: &str, description: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::teams::update_organization(&conn, id, name, description)
    }

    fn organization(&self, id: &str) -> AppResult<Option<Organization>> {
        let conn = lock(&self.conn)?;
        store::teams::organization(&conn, id)
    }

    fn organization_by_name(&self, name: &str) -> AppResult<Option<Organization>> {
        let conn = lock(&self.conn)?;
        store::teams::organization_by_name(&conn, name)
    }

    fn list_organizations(&self, user_id: &str, admin: bool) -> AppResult<Vec<Organization>> {
        let conn = lock(&self.conn)?;
        store::teams::list_organizations(&conn, user_id, admin)
    }

    fn teams_in_organization(&self, organization_id: &str) -> AppResult<Vec<Team>> {
        let conn = lock(&self.conn)?;
        store::teams::teams_in_organization(&conn, organization_id)
    }

    fn teams_with_ldap(&self) -> AppResult<Vec<Team>> {
        let conn = lock(&self.conn)?;
        store::teams::teams_with_ldap(&conn)
    }

    fn upsert_membership(
        &self,
        team_id: &str,
        user_id: &str,
        role: &str,
        source: &str,
    ) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::teams::upsert_membership(&conn, team_id, user_id, role, source)
    }

    fn grant_manual_membership(&self, team_id: &str, user_id: &str, role: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::teams::grant_manual_membership(&conn, team_id, user_id, role)
    }

    fn list_members(&self, team_id: &str) -> AppResult<Vec<Member>> {
        let conn = lock(&self.conn)?;
        store::teams::list_members(&conn, team_id)
    }

    fn delete_membership(&self, team_id: &str, user_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::teams::delete_membership(&conn, team_id, user_id)
    }

    fn delete_ldap_membership(&self, team_id: &str, user_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::teams::delete_ldap_membership(&conn, team_id, user_id)
    }

    fn list_projects(&self, team_id: &str) -> AppResult<Vec<Project>> {
        let conn = lock(&self.conn)?;
        store::projects::list_projects(&conn, team_id)
    }

    fn project(&self, id: &str) -> AppResult<Option<Project>> {
        let conn = lock(&self.conn)?;
        store::projects::project(&conn, id)
    }

    fn project_visible(&self, id: &str, team_id: &str) -> AppResult<bool> {
        let conn = lock(&self.conn)?;
        store::projects::project_visible(&conn, id, team_id)
    }

    fn insert_project(&self, project: &Project) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::projects::insert_project(&conn, project)
    }

    fn delete_project(&self, id: &str, team_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::projects::delete_project(&conn, id, team_id)
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
        store::projects::set_project_details(&conn, id, name, source_type, source, revision)
    }

    fn set_project_organization(&self, id: &str, organization_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::projects::set_project_organization(&conn, id, organization_id)
    }

    fn replace_project_teams(&self, project_id: &str, team_ids: &[String]) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::projects::replace_project_teams(&conn, project_id, team_ids)
    }

    fn set_project_credential(&self, id: &str, credential_id: Option<&str>) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::projects::set_project_credential(&conn, id, credential_id)
    }

    fn set_project_admin_saved(&self, id: &str, admin_saved: bool) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::projects::set_project_admin_saved(&conn, id, admin_saved)
    }

    fn set_project_sync_success(&self, id: &str, at: &str, commit: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::projects::set_project_sync_success(&conn, id, at, commit)
    }

    fn set_project_sync_failed(&self, id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::projects::set_project_sync_failed(&conn, id)
    }

    fn list_inventories(&self, team_id: &str) -> AppResult<Vec<Inventory>> {
        let conn = lock(&self.conn)?;
        store::inventories::list_inventories(&conn, team_id)
    }

    fn inventory(&self, id: &str) -> AppResult<Option<Inventory>> {
        let conn = lock(&self.conn)?;
        store::inventories::inventory(&conn, id)
    }

    fn insert_inventory(
        &self,
        item: &Inventory,
        sources: &[InventorySource],
        refresh: bool,
    ) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        let tx = conn.transaction()?;
        store::inventories::insert_inventory(&tx, item)?;
        for source in sources {
            store::inventories::insert_inventory_source(&tx, source)?;
        }
        if refresh {
            store::inventories::refresh_inventory_summary(&tx, &item.id)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn update_inventory(&self, id: &str, team_id: &str, name: &str, body: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::inventories::update_inventory(&conn, id, team_id, name, body)
    }

    fn set_inventory_admin_saved(&self, id: &str, admin_saved: bool) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::inventories::set_inventory_admin_saved(&conn, id, admin_saved)
    }

    fn set_inventory_body(&self, id: &str, team_id: &str, body: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::inventories::set_inventory_body(&conn, id, team_id, body)
    }

    fn list_inventory_sources(&self, inventory_id: &str) -> AppResult<Vec<InventorySource>> {
        let conn = lock(&self.conn)?;
        store::inventories::list_inventory_sources(&conn, inventory_id)
    }

    fn inventory_source(&self, id: &str) -> AppResult<Option<InventorySource>> {
        let conn = lock(&self.conn)?;
        store::inventories::inventory_source(&conn, id)
    }

    fn add_inventory_source(
        &self,
        inventory_id: &str,
        source: &InventorySource,
        admin_saved: bool,
    ) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        let tx = conn.transaction()?;
        store::inventories::insert_inventory_source(&tx, source)?;
        store::inventories::refresh_inventory_summary(&tx, inventory_id)?;
        store::inventories::set_inventory_admin_saved(&tx, inventory_id, admin_saved)?;
        tx.commit()?;
        Ok(())
    }

    fn set_inventory_source_body(&self, id: &str, body: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::inventories::set_inventory_source_body(&conn, id, body)
    }

    fn update_inventory_source(
        &self,
        source: &InventorySource,
        admin_saved: bool,
    ) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        let tx = conn.transaction()?;
        store::inventories::update_inventory_source(
            &tx,
            &source.id,
            &source.name,
            &source.source_json,
            source.credential_id.as_deref(),
            &source.body,
        )?;
        store::inventories::refresh_inventory_summary(&tx, &source.inventory_id)?;
        store::inventories::set_inventory_admin_saved(&tx, &source.inventory_id, admin_saved)?;
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
        store::inventories::delete_inventory_source(&tx, id, inventory_id)?;
        store::inventories::refresh_inventory_summary(&tx, inventory_id)?;
        store::inventories::set_inventory_admin_saved(&tx, inventory_id, admin_saved)?;
        tx.commit()?;
        Ok(())
    }

    fn refresh_inventory_summary(&self, inventory_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::inventories::refresh_inventory_summary(&conn, inventory_id)
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
            store::inventories::set_inventory_source_body(&tx, source_id, body)?;
        }
        store::inventories::refresh_inventory_summary(&tx, inventory_id)?;
        store::inventories::set_inventory_admin_saved(&tx, inventory_id, admin_saved)?;
        tx.commit()?;
        Ok(())
    }

    fn delete_inventory(&self, id: &str, team_id: &str) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        store::inventories::delete_inventory(&mut conn, id, team_id)
    }

    fn templates_using_inventory(
        &self,
        inventory_id: &str,
        team_id: &str,
    ) -> AppResult<Vec<String>> {
        let conn = lock(&self.conn)?;
        store::inventories::templates_using_inventory(&conn, inventory_id, team_id)
    }

    fn workflows_using_inventory(
        &self,
        inventory_id: &str,
        team_id: &str,
    ) -> AppResult<Vec<String>> {
        let conn = lock(&self.conn)?;
        store::inventories::workflows_using_inventory(&conn, inventory_id, team_id)
    }

    fn list_credentials(&self, team_id: &str) -> AppResult<Vec<Credential>> {
        let conn = lock(&self.conn)?;
        store::credentials::list_credentials(&conn, team_id)
    }

    fn credential(&self, id: &str) -> AppResult<Option<Credential>> {
        let conn = lock(&self.conn)?;
        store::credentials::credential(&conn, id)
    }

    fn insert_credential(&self, item: &Credential) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::credentials::insert_credential(&conn, item)
    }

    fn update_credential(&self, item: &Credential) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::credentials::update_credential(&conn, item)
    }

    fn delete_credential(&self, id: &str, team_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::credentials::delete_credential(&conn, id, team_id)
    }

    fn credential_usages(
        &self,
        credential_id: &str,
        team_id: &str,
    ) -> AppResult<Vec<CredentialUsage>> {
        let conn = lock(&self.conn)?;
        store::credentials::credential_usages(&conn, credential_id, team_id)
    }

    fn list_environments(&self, team_id: &str) -> AppResult<Vec<Environment>> {
        let conn = lock(&self.conn)?;
        let team = store::teams::team_by_id(&conn, team_id)?.ok_or(ApiError::NotFound)?;
        store::environments::list_environments(&conn, &team)
    }

    fn insert_environment(&self, item: &Environment) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::environments::insert_environment(&conn, item)
    }

    fn update_environment(&self, item: &Environment) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::environments::update_environment(&conn, item)
    }

    fn delete_environment(&self, id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::environments::delete_environment(&conn, id)
    }

    fn list_templates(&self, team_id: &str) -> AppResult<Vec<Template>> {
        let conn = lock(&self.conn)?;
        store::templates::list_templates(&conn, team_id)
    }

    fn template(&self, id: &str) -> AppResult<Option<Template>> {
        let conn = lock(&self.conn)?;
        store::templates::template(&conn, id)
    }

    fn insert_template(&self, item: &Template, credential_ids: &[String]) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        let tx = conn.transaction()?;
        store::templates::insert_template(&tx, item)?;
        store::templates::save_template_credentials(&tx, &item.id, credential_ids)?;
        tx.commit()?;
        Ok(())
    }

    fn update_template(&self, item: &Template, credential_ids: &[String]) -> AppResult<()> {
        let mut conn = lock(&self.conn)?;
        let tx = conn.transaction()?;
        store::templates::update_template(&tx, item)?;
        store::templates::save_template_credentials(&tx, &item.id, credential_ids)?;
        tx.commit()?;
        Ok(())
    }

    fn delete_template(&self, id: &str, team_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::templates::delete_template(&conn, id, team_id)
    }

    fn template_credential_ids(&self, template_id: &str) -> AppResult<Vec<String>> {
        let conn = lock(&self.conn)?;
        store::templates::template_credential_ids(&conn, template_id)
    }

    fn list_template_credential_links(&self, team_id: &str) -> AppResult<Vec<(String, String)>> {
        let conn = lock(&self.conn)?;
        store::templates::list_template_credential_links(&conn, team_id)
    }

    fn list_hooks(&self, team_id: &str) -> AppResult<Vec<Hook>> {
        let conn = lock(&self.conn)?;
        store::hooks::list_hooks(&conn, team_id)
    }

    fn hook(&self, id: &str) -> AppResult<Option<Hook>> {
        let conn = lock(&self.conn)?;
        store::hooks::hook(&conn, id)
    }

    fn insert_hook(&self, item: &Hook) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::hooks::insert_hook(&conn, item)
    }

    fn update_hook(&self, item: &Hook) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::hooks::update_hook(&conn, item)
    }

    fn delete_hook(&self, id: &str, team_id: &str) -> AppResult<()> {
        let conn = lock(&self.conn)?;
        store::hooks::delete_hook(&conn, id, team_id)
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
        store::hooks::first_hook(&conn, team_id)
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
    pool: Pool,
}

impl PostgresBackend {
    pub(crate) fn open(config: &PostgresConfig) -> anyhow::Result<Self> {
        let backend = Self {
            pool: build_pool(config)?,
        };
        backend.ensure_schema()?;
        Ok(backend)
    }

    fn run<F, T>(&self, future: F) -> AppResult<T>
    where
        F: std::future::Future<Output = AppResult<T>>,
    {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            ApiError::Internal("The PostgreSQL backend needs a Tokio runtime.".to_string())
        })?;
        if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
            return Err(ApiError::Internal(
                "The PostgreSQL backend needs a multi-thread Tokio runtime.".to_string(),
            ));
        }
        tokio::task::block_in_place(|| handle.block_on(future))
    }

    fn ensure_schema(&self) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            db::ensure_postgres_schema(&client).await
        })
    }
}

fn build_pool(config: &PostgresConfig) -> anyhow::Result<Pool> {
    let manager = match config.sslmode {
        SslMode::Disable => Manager::from_config(
            postgres_config(config),
            tokio_postgres::NoTls,
            ManagerConfig::default(),
        ),
        _ => Manager::from_config(
            postgres_config(config),
            tls_connector(config)?,
            ManagerConfig::default(),
        ),
    };
    Pool::builder(manager)
        .max_size(config.pool_size())
        .build()
        .map_err(|err| anyhow::anyhow!("The PostgreSQL connection pool cannot start: {err}"))
}

fn pool_error(err: deadpool_postgres::PoolError) -> ApiError {
    match err {
        deadpool_postgres::PoolError::Backend(err) => err.into(),
        other => ApiError::Internal(format!(
            "The PostgreSQL connection pool is unavailable: {other}"
        )),
    }
}

fn postgres_config(config: &PostgresConfig) -> tokio_postgres::Config {
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
    pg
}

pub(crate) async fn connect_client(config: &PostgresConfig) -> anyhow::Result<Client> {
    let pg = postgres_config(config);
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

const WORKFLOW_COLUMNS: &str = "id, team_id, name, description, graph, created_at, admin_saved";

const DELIVERY_COLUMNS: &str =
    "d.id, d.hook_id, h.name, d.status, d.summary, d.job_id, d.workflow_job_id, d.created_at";

const SCHEDULE_COLUMNS: &str =
    "id, team_id, name, target_kind, target_id, timezone, start_date, start_time, frequency, \
     weekdays, end_kind, end_count, end_date, enabled, next_run_at, missed_at, occurrence_count, \
     created_by, created_at, updated_at, updated_by, launch_block";

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

impl Backend for PostgresBackend {
    fn engine(&self) -> &'static str {
        "postgresql"
    }

    fn sqlite(&self) -> Option<&Mutex<Connection>> {
        None
    }

    fn export_sqlite(&self, dest: &Path) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            crate::portable::to_sqlite(&client, dest).await
        })
    }

    fn restore_sqlite(&self, source: &Path) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client.query_one("SELECT 1", &[]).await?;
            Ok(())
        })
    }

    fn settings_rows(&self) -> AppResult<HashMap<String, String>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(&store::sql::postgres(store::settings::SETTINGS_ROWS), &[])
                .await?;
            Ok(rows
                .into_iter()
                .map(|row| (row.get(0), row.get(1)))
                .collect())
        })
    }

    fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::settings::INSERT_SETTING),
                    &[&key, &value],
                )
                .await?;
            Ok(())
        })
    }

    fn count_teams(&self) -> AppResult<i64> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_one(&store::sql::postgres(store::teams::COUNT_TEAMS), &[])
                .await?;
            Ok(row.get(0))
        })
    }

    fn seed_state(&self, key: &str) -> AppResult<Option<String>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(&store::sql::postgres(store::settings::SEED_STATE), &[&key])
                .await?;
            Ok(row.map(|row| row.get(0)))
        })
    }

    fn set_seed_state(&self, key: &str, value: &str) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::settings::SET_SEED_STATE),
                    &[&key, &value],
                )
                .await?;
            Ok(())
        })
    }

    fn user_by_username(&self, username: &str) -> AppResult<Option<User>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let sql = store::sql::Dialect::Postgres.render(&store::auth::user_by_username_sql(
                store::sql::Dialect::Postgres,
            ));
            let row = client.query_opt(&sql, &[&username]).await?;
            row.as_ref().map(store::auth::row_user).transpose()
        })
    }

    fn user_by_session(&self, token: &str) -> AppResult<Option<User>> {
        let row = self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .query_opt(
                    &store::sql::postgres(store::auth::USER_BY_SESSION),
                    &[&token],
                )
                .await
                .map_err(ApiError::from)
        })?;
        let Some(row) = row else {
            return Ok(None);
        };
        let expires: String = row.try_get(9)?;
        if expires < db::now() {
            self.delete_session(token)?;
            return Ok(None);
        }
        Ok(Some(store::auth::row_user(&row)?))
    }

    fn user_by_api_token(&self, token_hash: &str) -> AppResult<Option<User>> {
        let row = self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .query_opt(
                    &store::sql::postgres(store::auth::USER_BY_API_TOKEN),
                    &[&token_hash],
                )
                .await
                .map_err(ApiError::from)
        })?;
        let Some(row) = row else {
            return Ok(None);
        };
        let token_id: String = row.try_get(9)?;
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::auth::TOUCH_API_TOKEN),
                    &[&token_id, &db::now()],
                )
                .await
                .map_err(ApiError::from)
        })?;
        Ok(Some(store::auth::row_user(&row)?))
    }

    fn create_session(&self, token: &str, user_id: &str) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::auth::CREATE_SESSION),
                    &[&token, &user_id, &db::plus_secs(7 * 24 * 3600)],
                )
                .await?;
            Ok(())
        })
    }

    fn delete_session(&self, token: &str) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::auth::DELETE_SESSION),
                    &[&token],
                )
                .await?;
            Ok(())
        })
    }

    fn purge_sessions(&self) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::auth::PURGE_SESSIONS),
                    &[&db::now()],
                )
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::auth::INSERT_USER),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(&store::sql::postgres(store::auth::USER_BY_ID), &[&id])
                .await?;
            row.as_ref().map(store::auth::row_user).transpose()
        })
    }

    fn list_users(&self) -> AppResult<Vec<(User, i64)>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let sql = store::sql::Dialect::Postgres
                .render(&store::auth::list_users_sql(store::sql::Dialect::Postgres));
            let rows = client.query(&sql, &[]).await?;
            rows.iter()
                .map(|row| Ok((store::auth::row_user(row)?, row.try_get::<_, i64>(9)?)))
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            if let Some(password_hash) = password_hash {
                client
                    .execute(
                        &store::sql::postgres(store::auth::UPDATE_PROFILE_PASSWORD),
                        &[&id, &display_name, &email, &password_hash],
                    )
                    .await?;
            } else {
                client
                    .execute(
                        &store::sql::postgres(store::auth::UPDATE_PROFILE),
                        &[&id, &display_name, &email],
                    )
                    .await?;
            }
            Ok(())
        })
    }

    fn set_user_email(&self, id: &str, email: Option<&str>) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::auth::SET_USER_EMAIL),
                    &[&id, &email],
                )
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            if let Some(password_hash) = password_hash {
                client
                    .execute(
                        &store::sql::postgres(store::auth::UPDATE_USER_PASSWORD),
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
                        &store::sql::postgres(store::auth::UPDATE_USER),
                        &[&id, &display_name, &(is_admin as i32), &(disabled as i32)],
                    )
                    .await?;
            }
            Ok(())
        })
    }

    fn delete_user_sessions(&self, user_id: &str) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::auth::DELETE_USER_SESSIONS),
                    &[&user_id],
                )
                .await?;
            Ok(())
        })
    }

    fn delete_user(&self, id: &str) -> AppResult<bool> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            Ok(client
                .execute(&store::sql::postgres(store::auth::DELETE_USER), &[&id])
                .await?
                > 0)
        })
    }

    fn other_active_admins(&self, user_id: &str) -> AppResult<i64> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_one(
                    &store::sql::postgres(store::auth::OTHER_ACTIVE_ADMINS),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::auth::CREATE_API_TOKEN),
                    &[&id, &user_id, &name, &prefix, &token_hash, &created_at],
                )
                .await?;
            Ok(())
        })
    }

    fn list_api_tokens(&self, user_id: &str) -> AppResult<Vec<ApiToken>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::auth::LIST_API_TOKENS),
                    &[&user_id],
                )
                .await?;
            rows.iter().map(store::auth::row_api_token).collect()
        })
    }

    fn delete_api_token(&self, id: &str, user_id: &str) -> AppResult<bool> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            Ok(client
                .execute(
                    &store::sql::postgres(store::auth::DELETE_API_TOKEN),
                    &[&id, &user_id],
                )
                .await?
                > 0)
        })
    }

    fn list_teams_for_user(&self, user_id: &str, admin: bool) -> AppResult<Vec<(Team, String)>> {
        let sql = store::sql::postgres(&store::teams::teams_for_user_sql(admin));
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client.query(&sql, &[&user_id]).await?;
            rows.iter()
                .map(|row| Ok((store::teams::row_team(row)?, row.try_get::<_, String>(7)?)))
                .collect()
        })
    }

    fn list_user_memberships(&self, user_id: &str) -> AppResult<Vec<UserMembership>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::teams::MEMBERSHIPS_FOR_USER),
                    &[&user_id],
                )
                .await?;
            rows.iter().map(store::teams::row_user_membership).collect()
        })
    }

    fn replace_settings(&self, rows: &[(String, String)]) -> AppResult<()> {
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
            let transaction = client.transaction().await?;
            transaction
                .execute(&store::sql::postgres(store::settings::DELETE_SETTINGS), &[])
                .await?;
            for (key, value) in rows {
                transaction
                    .execute(
                        &store::sql::postgres(store::settings::INSERT_SETTING),
                        &[key, value],
                    )
                    .await?;
            }
            transaction.commit().await?;
            Ok(())
        })
    }

    fn environment(&self, id: &str) -> AppResult<Option<Environment>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(
                    &store::sql::postgres(store::environments::ENVIRONMENT_BY_ID),
                    &[&id],
                )
                .await?;
            row.as_ref()
                .map(store::environments::row_environment)
                .transpose()
        })
    }

    fn team_by_id(&self, id: &str) -> AppResult<Option<Team>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(&store::sql::postgres(store::teams::TEAM_BY_ID), &[&id])
                .await?;
            row.as_ref().map(store::teams::row_team).transpose()
        })
    }

    fn role_of(&self, team_id: &str, user_id: &str) -> AppResult<Option<String>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(
                    &store::sql::postgres(store::teams::ROLE_OF),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::teams::INSERT_TEAM),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::teams::UPDATE_TEAM),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::teams::SET_TEAM_ORGANIZATION),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::teams::INSERT_ORGANIZATION),
                    &[&id, &name, &description, &db::now()],
                )
                .await?;
            Ok(())
        })
    }

    fn update_organization(&self, id: &str, name: &str, description: &str) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::teams::UPDATE_ORGANIZATION),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(
                    &store::sql::postgres(store::teams::ORGANIZATION_BY_ID),
                    &[&id],
                )
                .await?;
            row.as_ref().map(store::teams::row_organization).transpose()
        })
    }

    fn organization_by_name(&self, name: &str) -> AppResult<Option<Organization>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(
                    &store::sql::postgres(store::teams::ORGANIZATION_BY_NAME),
                    &[&name],
                )
                .await?;
            row.as_ref().map(store::teams::row_organization).transpose()
        })
    }

    fn list_organizations(&self, user_id: &str, admin: bool) -> AppResult<Vec<Organization>> {
        let sql = store::sql::postgres(if admin {
            store::teams::ORGANIZATIONS_ALL
        } else {
            store::teams::ORGANIZATIONS_FOR_USER
        });
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = if admin {
                client.query(&sql, &[]).await?
            } else {
                client.query(&sql, &[&user_id]).await?
            };
            rows.iter().map(store::teams::row_organization).collect()
        })
    }

    fn teams_in_organization(&self, organization_id: &str) -> AppResult<Vec<Team>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::teams::TEAMS_IN_ORGANIZATION),
                    &[&organization_id],
                )
                .await?;
            rows.iter().map(store::teams::row_team).collect()
        })
    }

    fn teams_with_ldap(&self) -> AppResult<Vec<Team>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(&store::sql::postgres(store::teams::TEAMS_WITH_LDAP), &[])
                .await?;
            rows.iter().map(store::teams::row_team).collect()
        })
    }

    fn upsert_membership(
        &self,
        team_id: &str,
        user_id: &str,
        role: &str,
        source: &str,
    ) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::teams::UPSERT_MEMBERSHIP),
                    &[&team_id, &user_id, &role, &source],
                )
                .await?;
            Ok(())
        })
    }

    fn grant_manual_membership(&self, team_id: &str, user_id: &str, role: &str) -> AppResult<()> {
        let source = self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(
                    &store::sql::postgres(store::teams::MEMBERSHIP_SOURCE),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::teams::LIST_MEMBERS),
                    &[&team_id],
                )
                .await?;
            rows.iter().map(store::teams::row_member).collect()
        })
    }

    fn delete_membership(&self, team_id: &str, user_id: &str) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::teams::DELETE_MEMBERSHIP),
                    &[&team_id, &user_id],
                )
                .await?;
            Ok(())
        })
    }

    fn delete_ldap_membership(&self, team_id: &str, user_id: &str) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::teams::DELETE_LDAP_MEMBERSHIP),
                    &[&team_id, &user_id],
                )
                .await?;
            Ok(())
        })
    }

    fn list_projects(&self, team_id: &str) -> AppResult<Vec<Project>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::projects::LIST_PROJECTS),
                    &[&team_id],
                )
                .await?;
            let mut projects = rows
                .iter()
                .map(store::projects::row_project)
                .collect::<AppResult<Vec<_>>>()?;
            for project in &mut projects {
                project.team_ids =
                    store::projects::postgres_project_team_ids(&client, &project.id).await?;
            }
            Ok(projects)
        })
    }

    fn project(&self, id: &str) -> AppResult<Option<Project>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(
                    &store::sql::postgres(store::projects::PROJECT_BY_ID),
                    &[&id],
                )
                .await?;
            let Some(row) = row else {
                return Ok(None);
            };
            let mut project = store::projects::row_project(&row)?;
            project.team_ids =
                store::projects::postgres_project_team_ids(&client, &project.id).await?;
            Ok(Some(project))
        })
    }

    fn project_visible(&self, id: &str, team_id: &str) -> AppResult<bool> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(
                    &store::sql::postgres(store::projects::PROJECT_VISIBLE),
                    &[&id, &team_id],
                )
                .await?;
            Ok(row.is_some())
        })
    }

    fn insert_project(&self, project: &Project) -> AppResult<()> {
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
            let transaction = client.transaction().await?;
            transaction
                .execute(
                    &store::sql::postgres(store::projects::INSERT_PROJECT),
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
            store::projects::postgres_write_project_teams(
                &transaction,
                &project.id,
                &project.team_ids,
            )
            .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn delete_project(&self, id: &str, team_id: &str) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::projects::DELETE_PROJECT),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::projects::SET_PROJECT_DETAILS),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::projects::SET_PROJECT_ORGANIZATION),
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
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
            let transaction = client.transaction().await?;
            store::projects::postgres_write_project_teams(&transaction, project_id, team_ids)
                .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn set_project_credential(&self, id: &str, credential_id: Option<&str>) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::projects::SET_PROJECT_CREDENTIAL),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::projects::SET_PROJECT_ADMIN_SAVED),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::projects::SET_PROJECT_SYNC_SUCCESS),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::projects::SET_PROJECT_SYNC_FAILED),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::inventories::LIST_INVENTORIES),
                    &[&team_id],
                )
                .await?;
            rows.iter().map(store::inventories::row_inventory).collect()
        })
    }

    fn inventory(&self, id: &str) -> AppResult<Option<Inventory>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(
                    &store::sql::postgres(store::inventories::INVENTORY_BY_ID),
                    &[&id],
                )
                .await?;
            row.as_ref()
                .map(store::inventories::row_inventory)
                .transpose()
        })
    }

    fn insert_inventory(
        &self,
        item: &Inventory,
        sources: &[InventorySource],
        refresh: bool,
    ) -> AppResult<()> {
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
            let transaction = client.transaction().await?;
            transaction
                .execute(
                    &store::sql::postgres(store::inventories::INSERT_INVENTORY),
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
                store::inventories::postgres_insert_source(&transaction, source).await?;
            }
            if refresh {
                store::inventories::postgres_refresh_summary(&transaction, &item.id).await?;
            }
            transaction.commit().await?;
            Ok(())
        })
    }

    fn update_inventory(&self, id: &str, team_id: &str, name: &str, body: &str) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::inventories::UPDATE_INVENTORY),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::inventories::SET_INVENTORY_ADMIN_SAVED),
                    &[&id, &(admin_saved as i32)],
                )
                .await?;
            Ok(())
        })
    }

    fn set_inventory_body(&self, id: &str, team_id: &str, body: &str) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::inventories::SET_INVENTORY_BODY),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::inventories::LIST_SOURCES),
                    &[&inventory_id],
                )
                .await?;
            rows.iter()
                .map(store::inventories::row_inventory_source)
                .collect()
        })
    }

    fn inventory_source(&self, id: &str) -> AppResult<Option<InventorySource>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(
                    &store::sql::postgres(store::inventories::SOURCE_BY_ID),
                    &[&id],
                )
                .await?;
            row.as_ref()
                .map(store::inventories::row_inventory_source)
                .transpose()
        })
    }

    fn add_inventory_source(
        &self,
        inventory_id: &str,
        source: &InventorySource,
        admin_saved: bool,
    ) -> AppResult<()> {
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
            let transaction = client.transaction().await?;
            store::inventories::postgres_insert_source(&transaction, source).await?;
            store::inventories::postgres_refresh_summary(&transaction, inventory_id).await?;
            transaction
                .execute(
                    &store::sql::postgres(store::inventories::SET_INVENTORY_ADMIN_SAVED),
                    &[&inventory_id, &(admin_saved as i32)],
                )
                .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn set_inventory_source_body(&self, id: &str, body: &str) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::inventories::SET_SOURCE_BODY),
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
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
            let transaction = client.transaction().await?;
            let changed = transaction
                .execute(
                    &store::sql::postgres(store::inventories::UPDATE_SOURCE),
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
            store::inventories::postgres_refresh_summary(&transaction, &source.inventory_id)
                .await?;
            transaction
                .execute(
                    &store::sql::postgres(store::inventories::SET_INVENTORY_ADMIN_SAVED),
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
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
            let transaction = client.transaction().await?;
            let changed = transaction
                .execute(
                    &store::sql::postgres(store::inventories::DELETE_SOURCE),
                    &[&id, &inventory_id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            store::inventories::postgres_refresh_summary(&transaction, inventory_id).await?;
            transaction
                .execute(
                    &store::sql::postgres(store::inventories::SET_INVENTORY_ADMIN_SAVED),
                    &[&inventory_id, &(admin_saved as i32)],
                )
                .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn refresh_inventory_summary(&self, inventory_id: &str) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            store::inventories::postgres_refresh_summary(&client, inventory_id).await
        })
    }

    fn sync_inventory_sources(
        &self,
        inventory_id: &str,
        bodies: &[(String, String)],
        admin_saved: bool,
    ) -> AppResult<()> {
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
            let transaction = client.transaction().await?;
            for (source_id, body) in bodies {
                transaction
                    .execute(
                        &store::sql::postgres(store::inventories::SET_SOURCE_BODY),
                        &[source_id, body],
                    )
                    .await?;
            }
            store::inventories::postgres_refresh_summary(&transaction, inventory_id).await?;
            transaction
                .execute(
                    &store::sql::postgres(store::inventories::SET_INVENTORY_ADMIN_SAVED),
                    &[&inventory_id, &(admin_saved as i32)],
                )
                .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn delete_inventory(&self, id: &str, team_id: &str) -> AppResult<()> {
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
            let transaction = client.transaction().await?;
            let rows = transaction
                .query(
                    &store::sql::postgres(store::inventories::SOURCE_CREDENTIALS),
                    &[&id],
                )
                .await?;
            let credential_ids: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
            let changed = transaction
                .execute(
                    &store::sql::postgres(store::inventories::DELETE_INVENTORY),
                    &[&id, &team_id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            for credential_id in credential_ids {
                if store::inventories::postgres_json_references_credential(
                    &transaction,
                    team_id,
                    &credential_id,
                )
                .await?
                {
                    continue;
                }
                transaction
                    .execute(
                        &store::sql::postgres(store::inventories::DELETE_AUTO_CREDENTIAL),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::inventories::TEMPLATES_USING_INVENTORY),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::inventories::WORKFLOW_GRAPHS),
                    &[&team_id],
                )
                .await?;
            let mut names = Vec::new();
            for row in rows {
                let name: String = row.get(0);
                let graph: String = row.get(1);
                if store::inventories::workflow_references_inventory(&graph, inventory_id) {
                    names.push(name);
                }
            }
            Ok(names)
        })
    }

    fn list_credentials(&self, team_id: &str) -> AppResult<Vec<Credential>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::credentials::LIST_CREDENTIALS),
                    &[&team_id],
                )
                .await?;
            rows.iter()
                .map(store::credentials::row_credential)
                .collect()
        })
    }

    fn credential(&self, id: &str) -> AppResult<Option<Credential>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(
                    &store::sql::postgres(store::credentials::CREDENTIAL_BY_ID),
                    &[&id],
                )
                .await?;
            row.as_ref()
                .map(store::credentials::row_credential)
                .transpose()
        })
    }

    fn insert_credential(&self, item: &Credential) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::credentials::INSERT_CREDENTIAL),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::credentials::UPDATE_CREDENTIAL),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::credentials::DELETE_CREDENTIAL),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::credentials::CREDENTIAL_USAGES),
                    &[&credential_id, &team_id],
                )
                .await?;
            rows.iter()
                .map(store::credentials::row_credential_usage)
                .collect()
        })
    }

    fn list_environments(&self, team_id: &str) -> AppResult<Vec<Environment>> {
        let team = self.team_by_id(team_id)?.ok_or(ApiError::NotFound)?;
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::environments::LIST_ENVIRONMENTS),
                    &[&team.organization_id, &team.id],
                )
                .await?;
            rows.iter()
                .map(store::environments::row_environment)
                .collect()
        })
    }

    fn insert_environment(&self, item: &Environment) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::environments::INSERT_ENVIRONMENT),
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
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
            let transaction = client.transaction().await?;
            let changed = transaction
                .execute(
                    &store::sql::postgres(store::environments::UPDATE_ENVIRONMENT),
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
                        &store::sql::postgres(store::environments::DELETE_DEFAULT_SETTING),
                        &[&key, &item.id],
                    )
                    .await?;
            }
            transaction.commit().await?;
            Ok(())
        })
    }

    fn delete_environment(&self, id: &str) -> AppResult<()> {
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
            let transaction = client.transaction().await?;
            let changed = transaction
                .execute(
                    &store::sql::postgres(store::environments::DELETE_ENVIRONMENT),
                    &[&id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            for key in crate::settings::DEFAULT_ENVIRONMENT_KEYS {
                transaction
                    .execute(
                        &store::sql::postgres(store::environments::DELETE_DEFAULT_SETTING),
                        &[&key, &id],
                    )
                    .await?;
            }
            transaction.commit().await?;
            Ok(())
        })
    }

    fn list_templates(&self, team_id: &str) -> AppResult<Vec<Template>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::templates::LIST_TEMPLATES),
                    &[&team_id],
                )
                .await?;
            rows.iter().map(store::templates::row_template).collect()
        })
    }

    fn template(&self, id: &str) -> AppResult<Option<Template>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(
                    &store::sql::postgres(store::templates::TEMPLATE_BY_ID),
                    &[&id],
                )
                .await?;
            row.as_ref().map(store::templates::row_template).transpose()
        })
    }

    fn insert_template(&self, item: &Template, credential_ids: &[String]) -> AppResult<()> {
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
            let transaction = client.transaction().await?;
            transaction
                .execute(
                    &store::sql::postgres(store::templates::INSERT_TEMPLATE),
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
            store::templates::postgres_write_template_credentials(
                &transaction,
                &item.id,
                credential_ids,
            )
            .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn update_template(&self, item: &Template, credential_ids: &[String]) -> AppResult<()> {
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
            let transaction = client.transaction().await?;
            let changed = transaction
                .execute(
                    &store::sql::postgres(store::templates::UPDATE_TEMPLATE),
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
            store::templates::postgres_write_template_credentials(
                &transaction,
                &item.id,
                credential_ids,
            )
            .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn delete_template(&self, id: &str, team_id: &str) -> AppResult<()> {
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
            let transaction = client.transaction().await?;
            let changed = transaction
                .execute(
                    &store::sql::postgres(store::templates::DELETE_TEMPLATE),
                    &[&id, &team_id],
                )
                .await?;
            if changed == 0 {
                return Err(ApiError::NotFound);
            }
            transaction
                .execute(
                    &store::sql::postgres(store::templates::DELETE_TEMPLATE_SCHEDULES),
                    &[&id],
                )
                .await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    fn template_credential_ids(&self, template_id: &str) -> AppResult<Vec<String>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::templates::TEMPLATE_CREDENTIAL_IDS),
                    &[&template_id],
                )
                .await?;
            Ok(rows.iter().map(|row| row.get(0)).collect())
        })
    }

    fn list_template_credential_links(&self, team_id: &str) -> AppResult<Vec<(String, String)>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(
                    &store::sql::postgres(store::templates::LIST_TEMPLATE_CREDENTIAL_LINKS),
                    &[&team_id],
                )
                .await?;
            Ok(rows.iter().map(|row| (row.get(0), row.get(1))).collect())
        })
    }

    fn list_hooks(&self, team_id: &str) -> AppResult<Vec<Hook>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let rows = client
                .query(&store::sql::postgres(store::hooks::LIST_HOOKS), &[&team_id])
                .await?;
            rows.iter().map(store::hooks::row_hook).collect()
        })
    }

    fn hook(&self, id: &str) -> AppResult<Option<Hook>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(&store::sql::postgres(store::hooks::HOOK_BY_ID), &[&id])
                .await?;
            row.as_ref().map(store::hooks::row_hook).transpose()
        })
    }

    fn insert_hook(&self, item: &Hook) -> AppResult<()> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            client
                .execute(
                    &store::sql::postgres(store::hooks::INSERT_HOOK),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::hooks::UPDATE_HOOK),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let changed = client
                .execute(
                    &store::sql::postgres(store::hooks::DELETE_HOOK),
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt(&store::sql::postgres(store::hooks::FIRST_HOOK), &[&team_id])
                .await?;
            row.as_ref().map(store::hooks::row_hook).transpose()
        })
    }

    fn list_workflows(&self, team_id: &str) -> AppResult<Vec<Workflow>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
            let row = client
                .query_opt("SELECT team_id FROM workflows WHERE id = $1", &[&id])
                .await?;
            Ok(row.map(|row| row.get(0)))
        })
    }

    fn list_team_schedules(&self, team_id: &str) -> AppResult<Vec<(Schedule, String)>> {
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let mut client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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
        self.run(async {
            let client = self.pool.get().await.map_err(pool_error)?;
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

#[cfg(test)]
mod migration_tests {
    use super::*;

    async fn postgres_version(client: &tokio_postgres::Client) -> u32 {
        let value: String = client
            .query_one(
                "SELECT value FROM schema_meta WHERE key = 'schema_version'",
                &[],
            )
            .await
            .expect("schema version")
            .get(0);
        value.parse().expect("version number")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failing_migration_keeps_the_postgres_version() {
        let Ok(url) = std::env::var("KOREOGRAF_TEST_POSTGRES_URL") else {
            eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is not set");
            return;
        };
        if url.trim().is_empty() {
            eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is empty");
            return;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let name = format!("koreograf_migration_{}_{nanos}", std::process::id());
        let mut base = url::Url::parse(&url).expect("test postgres url");
        let mut admin_url = base.clone();
        admin_url.set_path("/postgres");
        let (admin, admin_connection) =
            tokio_postgres::connect(admin_url.as_str(), tokio_postgres::NoTls)
                .await
                .expect("postgres admin connection");
        tokio::spawn(async move {
            let _ = admin_connection.await;
        });
        admin
            .batch_execute(&format!("CREATE DATABASE \"{name}\""))
            .await
            .expect("create database");
        base.set_path(&format!("/{name}"));
        let (client, connection) = tokio_postgres::connect(base.as_str(), tokio_postgres::NoTls)
            .await
            .expect("postgres connection");
        tokio::spawn(async move {
            let _ = connection.await;
        });

        db::ensure_postgres_schema(&client)
            .await
            .expect("baseline schema");
        assert_eq!(postgres_version(&client).await, db::SCHEMA_VERSION);

        let migrations = [db::Migration {
            version: db::SCHEMA_VERSION + 1,
            name: "9999_broken",
            sql: "CREATE TABLE broken (id TEXT",
        }];
        assert!(
            db::apply_postgres_migrations(&client, &migrations, db::SCHEMA_VERSION)
                .await
                .is_err()
        );
        assert_eq!(postgres_version(&client).await, db::SCHEMA_VERSION);
        let broken: Option<String> = client
            .query_opt("SELECT to_regclass('broken')::text", &[])
            .await
            .expect("table lookup")
            .and_then(|row| row.get(0));
        assert!(broken.is_none());

        drop(client);
        admin
            .execute(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
                 WHERE datname = $1 AND pid <> pg_backend_pid()",
                &[&name],
            )
            .await
            .expect("terminate connections");
        admin
            .batch_execute(&format!("DROP DATABASE \"{name}\""))
            .await
            .expect("drop database");
    }
}
