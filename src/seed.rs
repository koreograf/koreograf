use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use rusqlite::Connection;
use serde_json::json;
use uuid::Uuid;

use crate::backend::Backend;
use crate::config::Config;
use crate::crypto::{self, hash_password};
use crate::db::{
    self, Environment, Hook, Inventory, InventorySource, Project, Template, User, Workflow,
};
use crate::error::AppResult;
use crate::store;

const SEEDED_ENVIRONMENTS_KEY: &str = "default_environments";

struct SeededEnvironment {
    id: &'static str,
    tool: &'static str,
    name: &'static str,
    image: &'static str,
    description: &'static str,
}

const SEEDED_ENVIRONMENTS: [SeededEnvironment; 3] = [
    SeededEnvironment {
        id: "seeded-ansible",
        tool: "ansible",
        name: "Ansible (curated)",
        image: "ghcr.io/ansible-community/community-ee-base:2.21.5-2@sha256:d42f2597080e8e34ce4d1f70f66f987fc841bd0bbeb680f8d234e94ef1afc60a",
        description: "Curated Ansible execution environment. The image is maintained upstream and pinned by digest.",
    },
    SeededEnvironment {
        id: "seeded-opentofu",
        tool: "opentofu",
        name: "OpenTofu (curated)",
        image: "ghcr.io/opentofu/opentofu:1.13.2@sha256:25bdfbe87000b8a88d44c7ef0cca53381da95f6203f7aed1e8cf5b81ecbe18eb",
        description: "Curated OpenTofu execution environment. The image is maintained upstream and pinned by digest.",
    },
    SeededEnvironment {
        id: "seeded-shell",
        tool: "shell",
        name: "Shell (curated)",
        image: "mcr.microsoft.com/devcontainers/base:bookworm@sha256:99f5250dd2d5f8b11046d5662dd94c1e3c1dae56b560c7caad5ceb7edf349046",
        description: "Curated shell environment with bash, curl, jq, and OpenSSH. The image is maintained upstream and pinned by digest.",
    },
];

trait SeedStore {
    fn user_by_username(&self, username: &str) -> AppResult<Option<User>>;
    fn insert_user(
        &self,
        id: &str,
        username: &str,
        display_name: &str,
        password_hash: Option<&str>,
        auth_source: &str,
        is_admin: bool,
    ) -> AppResult<()>;
    fn count_teams(&self) -> AppResult<i64>;
    fn seed_state(&self, key: &str) -> AppResult<Option<String>>;
    fn set_seed_state(&self, key: &str, value: &str) -> AppResult<()>;
    fn setting(&self, key: &str) -> AppResult<Option<String>>;
    fn set_setting(&self, key: &str, value: &str) -> AppResult<()>;
    fn environment_exists(&self, id: &str) -> AppResult<bool>;
    fn insert_environment(&self, item: &Environment) -> AppResult<()>;
    fn insert_organization(&self, id: &str, name: &str, description: &str) -> AppResult<()>;
    fn insert_team(
        &self,
        id: &str,
        organization_id: &str,
        name: &str,
        description: &str,
        ldap_group: Option<&str>,
    ) -> AppResult<()>;
    fn upsert_membership(
        &self,
        team_id: &str,
        user_id: &str,
        role: &str,
        source: &str,
    ) -> AppResult<()>;
    fn insert_project(&self, project: &Project) -> AppResult<()>;
    fn insert_inventory(&self, item: &Inventory, sources: &[InventorySource]) -> AppResult<()>;
    fn insert_template(&self, item: &Template) -> AppResult<()>;
    fn insert_workflow(&self, item: &Workflow) -> AppResult<()>;
    fn insert_hook(&self, item: &Hook) -> AppResult<()>;
}

impl SeedStore for Connection {
    fn user_by_username(&self, username: &str) -> AppResult<Option<User>> {
        store::auth::user_by_username(self, username)
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
        store::auth::insert_user(
            self,
            id,
            username,
            display_name,
            password_hash,
            auth_source,
            is_admin,
        )
    }

    fn count_teams(&self) -> AppResult<i64> {
        store::teams::count_teams(self)
    }

    fn seed_state(&self, key: &str) -> AppResult<Option<String>> {
        store::settings::seed_state(self, key)
    }

    fn set_seed_state(&self, key: &str, value: &str) -> AppResult<()> {
        store::settings::set_seed_state(self, key, value)
    }

    fn setting(&self, key: &str) -> AppResult<Option<String>> {
        store::settings::setting(self, key)
    }

    fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
        store::settings::set_setting(self, key, value)
    }

    fn environment_exists(&self, id: &str) -> AppResult<bool> {
        Ok(store::environments::environment(self, id)?.is_some())
    }

    fn insert_environment(&self, item: &Environment) -> AppResult<()> {
        store::environments::insert_environment(self, item)
    }

    fn insert_organization(&self, id: &str, name: &str, description: &str) -> AppResult<()> {
        store::teams::insert_organization(self, id, name, description)
    }

    fn insert_team(
        &self,
        id: &str,
        organization_id: &str,
        name: &str,
        description: &str,
        ldap_group: Option<&str>,
    ) -> AppResult<()> {
        store::teams::insert_team(self, id, organization_id, name, description, ldap_group)
    }

    fn upsert_membership(
        &self,
        team_id: &str,
        user_id: &str,
        role: &str,
        source: &str,
    ) -> AppResult<()> {
        store::teams::upsert_membership(self, team_id, user_id, role, source)
    }

    fn insert_project(&self, project: &Project) -> AppResult<()> {
        store::projects::insert_project(self, project)
    }

    fn insert_inventory(&self, item: &Inventory, sources: &[InventorySource]) -> AppResult<()> {
        store::inventories::insert_inventory(self, item)?;
        for source in sources {
            store::inventories::insert_inventory_source(self, source)?;
        }
        Ok(())
    }

    fn insert_template(&self, item: &Template) -> AppResult<()> {
        store::templates::insert_template(self, item)
    }

    fn insert_workflow(&self, item: &Workflow) -> AppResult<()> {
        crate::store::workflows::insert_workflow(self, item)
    }

    fn insert_hook(&self, item: &Hook) -> AppResult<()> {
        store::hooks::insert_hook(self, item)
    }
}

impl<T: Backend + ?Sized> SeedStore for T {
    fn user_by_username(&self, username: &str) -> AppResult<Option<User>> {
        Backend::user_by_username(self, username)
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
        Backend::insert_user(
            self,
            id,
            username,
            display_name,
            password_hash,
            auth_source,
            is_admin,
        )
    }

    fn count_teams(&self) -> AppResult<i64> {
        Backend::count_teams(self)
    }

    fn seed_state(&self, key: &str) -> AppResult<Option<String>> {
        Backend::seed_state(self, key)
    }

    fn set_seed_state(&self, key: &str, value: &str) -> AppResult<()> {
        Backend::set_seed_state(self, key, value)
    }

    fn setting(&self, key: &str) -> AppResult<Option<String>> {
        let mut rows = Backend::settings_rows(self)?;
        Ok(rows.remove(key))
    }

    fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
        Backend::set_setting(self, key, value)
    }

    fn environment_exists(&self, id: &str) -> AppResult<bool> {
        Ok(Backend::environment(self, id)?.is_some())
    }

    fn insert_environment(&self, item: &Environment) -> AppResult<()> {
        Backend::insert_environment(self, item)
    }

    fn insert_organization(&self, id: &str, name: &str, description: &str) -> AppResult<()> {
        Backend::insert_organization(self, id, name, description)
    }

    fn insert_team(
        &self,
        id: &str,
        organization_id: &str,
        name: &str,
        description: &str,
        ldap_group: Option<&str>,
    ) -> AppResult<()> {
        Backend::insert_team(self, id, organization_id, name, description, ldap_group)
    }

    fn upsert_membership(
        &self,
        team_id: &str,
        user_id: &str,
        role: &str,
        source: &str,
    ) -> AppResult<()> {
        Backend::upsert_membership(self, team_id, user_id, role, source)
    }

    fn insert_project(&self, project: &Project) -> AppResult<()> {
        Backend::insert_project(self, project)
    }

    fn insert_inventory(&self, item: &Inventory, sources: &[InventorySource]) -> AppResult<()> {
        Backend::insert_inventory(self, item, sources, false)
    }

    fn insert_template(&self, item: &Template) -> AppResult<()> {
        Backend::insert_template(self, item, &[])
    }

    fn insert_workflow(&self, item: &Workflow) -> AppResult<()> {
        Backend::insert_workflow(self, item)
    }

    fn insert_hook(&self, item: &Hook) -> AppResult<()> {
        Backend::insert_hook(self, item)
    }
}

pub fn seed(backend: &dyn Backend, cfg: &Config) -> AppResult<Option<String>> {
    let generated = if let Some(conn) = backend.sqlite() {
        let conn = conn.lock().map_err(|_| {
            crate::error::ApiError::Internal("The SQLite database lock is damaged.".into())
        })?;
        let tx = conn.unchecked_transaction()?;
        store::auth::purge_sessions(&tx)?;
        let generated = seed_with(&*tx, cfg)?;
        tx.commit()?;
        generated
    } else {
        backend.purge_sessions()?;
        seed_with(backend, cfg)?
    };
    if let Some(password) = &generated {
        write_password_file(&cfg.data_dir.join("initial-admin-password"), password)?;
    }
    Ok(generated)
}

fn seed_with<S: SeedStore + ?Sized>(store: &S, cfg: &Config) -> AppResult<Option<String>> {
    let generated = ensure_admin(store, cfg)?;
    if environments_seeded_by_env() {
        seed_environments(store)?;
    }
    if store.count_teams()? == 0 {
        seed_demo(store, cfg)?;
    }
    Ok(generated)
}

fn admin_credentials(cfg: &Config) -> (String, Option<String>) {
    match &cfg.admin_password {
        Some(password) => (password.clone(), None),
        None => {
            let password = crypto::random_password();
            (password.clone(), Some(password))
        }
    }
}

fn ensure_admin<S: SeedStore + ?Sized>(store: &S, cfg: &Config) -> AppResult<Option<String>> {
    if store.user_by_username(&cfg.admin_user)?.is_some() {
        return Ok(None);
    }
    let (password, generated) = admin_credentials(cfg);
    if password.len() < 8 {
        tracing::warn!("The admin password is shorter than 8 characters.");
    }
    store.insert_user(
        &Uuid::new_v4().to_string(),
        &cfg.admin_user,
        "Administrator",
        Some(&hash_password(&password)),
        "local",
        true,
    )?;
    Ok(generated)
}

fn environments_seeded_by_env() -> bool {
    match std::env::var("KOREOGRAF_SEED_ENVIRONMENTS") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

fn seed_environments<S: SeedStore + ?Sized>(store: &S) -> AppResult<()> {
    if store.seed_state(SEEDED_ENVIRONMENTS_KEY)?.as_deref() == Some("1") {
        return Ok(());
    }
    for item in &SEEDED_ENVIRONMENTS {
        if !store.environment_exists(item.id)? {
            store.insert_environment(&Environment {
                id: item.id.to_string(),
                team_id: None,
                organization_id: None,
                name: item.name.to_string(),
                tool: item.tool.to_string(),
                image: item.image.to_string(),
                pull_policy: "if_not_present".to_string(),
                network: String::new(),
                description: item.description.to_string(),
                created_at: db::now(),
                scope: "global".to_string(),
                organization_name: String::new(),
                team_name: String::new(),
            })?;
        }
        let key = format!("default_environment_{}", item.tool);
        if store.setting(&key)?.unwrap_or_default().trim().is_empty() {
            store.set_setting(&key, item.id)?;
        }
    }
    store.set_seed_state(SEEDED_ENVIRONMENTS_KEY, "1")
}

fn write_password_file(path: &Path, password: &str) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| crate::error::ApiError::Internal(err.to_string()))?;
    }
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts
        .open(path)
        .map_err(|err| crate::error::ApiError::Internal(err.to_string()))?;
    writeln!(file, "{password}")
        .map_err(|err| crate::error::ApiError::Internal(err.to_string()))?;
    tracing::warn!(
        path = %path.display(),
        "The initial admin password is in this file. Delete it after reading."
    );
    Ok(())
}

fn seed_demo<S: SeedStore + ?Sized>(store: &S, cfg: &Config) -> AppResult<()> {
    let admin = store.user_by_username(&cfg.admin_user)?.ok_or_else(|| {
        crate::error::ApiError::Internal("Admin user is missing after creation.".into())
    })?;
    let organization_id = Uuid::new_v4().to_string();
    store.insert_organization(
        &organization_id,
        "Plattform",
        "Demo organization for the sample team.",
    )?;
    let team_id = Uuid::new_v4().to_string();
    store.insert_team(
        &team_id,
        &organization_id,
        "Plattform",
        "Sample team. An LDAP group can later use this group name.",
        None,
    )?;
    store.upsert_membership(&team_id, &admin.id, "owner", "manual")?;

    let project_id = Uuid::new_v4().to_string();
    let source = cfg.examples_dir.join("demo");
    store.insert_project(&Project {
        id: project_id.clone(),
        team_id: team_id.clone(),
        organization_id,
        organization_name: "Plattform".into(),
        name: "Demo".into(),
        source_type: "path".into(),
        source: source.display().to_string(),
        revision: String::new(),
        credential_id: None,
        created_at: db::now(),
        team_ids: Vec::new(),
        last_sync_at: String::new(),
        last_sync_commit: String::new(),
        last_sync_status: String::new(),
        admin_saved: true,
    })?;

    let inventory_id = Uuid::new_v4().to_string();
    let inventory_source = InventorySource {
        id: Uuid::new_v4().to_string(),
        inventory_id: inventory_id.clone(),
        name: "manual".into(),
        kind: "ini".into(),
        source_json: "{}".into(),
        credential_id: None,
        body: "[all]\nlocalhost ansible_connection=local\n".into(),
        position: 0,
        created_at: db::now(),
    };
    store.insert_inventory(
        &Inventory {
            id: inventory_id.clone(),
            team_id: team_id.clone(),
            name: "local".into(),
            body: "[all]\nlocalhost ansible_connection=local\n".into(),
            created_at: db::now(),
            source_kind: "ini".into(),
            source_json: "{}".into(),
            credential_id: None,
            admin_saved: true,
        },
        std::slice::from_ref(&inventory_source),
    )?;

    store.insert_environment(&Environment {
        id: Uuid::new_v4().to_string(),
        team_id: Some(team_id.clone()),
        organization_id: None,
        name: "AWX-EE".into(),
        tool: "any".into(),
        image: "quay.io/ansible/awx-ee:latest".into(),
        pull_policy: "if_not_present".into(),
        network: String::new(),
        description: "Same idea as an AWX execution environment. The Galaxy cache stays mounted on the host and is not cleared after a run.".into(),
        created_at: db::now(),
        scope: "team".into(),
        organization_name: String::new(),
        team_name: String::new(),
    })?;

    let heal_vars = json!({
        "alertname": "manual",
        "instance": "localhost",
        "heal_action": "noop"
    })
    .to_string();
    let plan_spec = json!({"action": "plan"}).to_string();
    let health_id = insert_template(
        store,
        DemoTemplate {
            team_id: &team_id,
            project_id: &project_id,
            inventory_id: None,
            name: "Healthcheck",
            description: "A short shell check that runs without Ansible.",
            tool: "shell",
            entrypoint: "health.sh",
            extra_vars: "{}",
            spec: "{}",
        },
    )?;
    let heal_id = insert_template(
        store,
        DemoTemplate {
            team_id: &team_id,
            project_id: &project_id,
            inventory_id: Some(&inventory_id),
            name: "Self-Heal",
            description: "The playbook writes a report and deletes only file names that are named explicitly in the job directory.",
            tool: "ansible",
            entrypoint: "site.yml",
            extra_vars: &heal_vars,
            spec: "{}",
        },
    )?;
    let escalate_id = insert_template(
        store,
        DemoTemplate {
            team_id: &team_id,
            project_id: &project_id,
            inventory_id: None,
            name: "Eskalation",
            description: "Runs when the health check fails.",
            tool: "shell",
            entrypoint: "escalate.sh",
            extra_vars: "{}",
            spec: "{}",
        },
    )?;
    insert_template(
        store,
        DemoTemplate {
            team_id: &team_id,
            project_id: &project_id,
            inventory_id: None,
            name: "Infrastruktur-Plan",
            description: "OpenTofu plan with a persistent plugin cache.",
            tool: "opentofu",
            entrypoint: "tofu",
            extra_vars: "{}",
            spec: &plan_spec,
        },
    )?;

    let graph = json!({
        "nodes": [
            {"id": "check", "template_id": health_id, "name": "Healthcheck", "x": 48, "y": 168, "convergence": "any"},
            {"id": "heal", "template_id": heal_id, "name": "Self-Heal", "x": 360, "y": 72, "convergence": "any"},
            {"id": "escalate", "template_id": escalate_id, "name": "Eskalation", "x": 360, "y": 268, "convergence": "any"}
        ],
        "edges": [
            {"from": "check", "to": "heal", "when": "success"},
            {"from": "check", "to": "escalate", "when": "failure"}
        ]
    });
    let workflow_id = Uuid::new_v4().to_string();
    store.insert_workflow(&Workflow {
        id: workflow_id.clone(),
        team_id: team_id.clone(),
        name: "Self-Heal Ablauf".into(),
        description: "Health check, then the playbook, or the escalation when it fails.".into(),
        graph: graph.to_string(),
        created_at: db::now(),
        admin_saved: true,
    })?;

    store.insert_hook(&Hook {
        id: Uuid::new_v4().to_string(),
        team_id,
        name: "DiskSpaceLow".into(),
        token: crypto::random_token(),
        match_alertname: "DiskSpaceLow".into(),
        match_status: "firing".into(),
        template_id: None,
        workflow_id: Some(workflow_id),
        extra_vars: json!({
            "heal_action": "cleanup-tmp",
            "incident_report_cleanup_files": ["stale.tmp"]
        })
        .to_string(),
        enabled: true,
        created_at: db::now(),
    })?;
    Ok(())
}

struct DemoTemplate<'a> {
    team_id: &'a str,
    project_id: &'a str,
    inventory_id: Option<&'a str>,
    name: &'a str,
    description: &'a str,
    tool: &'a str,
    entrypoint: &'a str,
    extra_vars: &'a str,
    spec: &'a str,
}

fn insert_template<S: SeedStore + ?Sized>(store: &S, item: DemoTemplate<'_>) -> AppResult<String> {
    let id = Uuid::new_v4().to_string();
    store.insert_template(&Template {
        id: id.clone(),
        team_id: item.team_id.to_string(),
        name: item.name.to_string(),
        description: item.description.to_string(),
        tool: item.tool.to_string(),
        project_id: Some(item.project_id.to_string()),
        inventory_id: item.inventory_id.map(str::to_string),
        credential_id: None,
        environment_id: None,
        no_environment: true,
        entrypoint: item.entrypoint.to_string(),
        extra_vars: item.extra_vars.to_string(),
        spec: item.spec.to_string(),
        created_at: db::now(),
        update_on_launch: true,
        admin_saved: true,
        prompt_on_launch: false,
    })?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> (std::path::PathBuf, Connection) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-seed-ee-{nanos}"));
        std::fs::create_dir_all(&dir).expect("dir");
        let conn = db::open(&dir.join("koreograf.db")).expect("database");
        (dir, conn)
    }

    fn environment_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM environments", [], |row| row.get(0))
            .expect("count")
    }

    fn default_for(conn: &Connection, tool: &str) -> String {
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [format!("default_environment_{tool}")],
            |row| row.get(0),
        )
        .expect("default")
    }

    #[test]
    fn seeding_twice_adds_one_environment_per_tool_and_keeps_the_defaults() {
        let (dir, conn) = database();
        seed_environments(&conn).expect("first seed");
        seed_environments(&conn).expect("second seed");
        assert_eq!(environment_count(&conn), 3);
        for item in &SEEDED_ENVIRONMENTS {
            assert_eq!(default_for(&conn, item.tool), item.id);
            let image: String = conn
                .query_row(
                    "SELECT image FROM environments WHERE id = ?1",
                    [item.id],
                    |row| row.get(0),
                )
                .expect("image");
            assert_eq!(image, item.image);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_renamed_or_deleted_seeded_environment_is_not_recreated() {
        let (dir, conn) = database();
        seed_environments(&conn).expect("seed");
        conn.execute(
            "UPDATE environments SET name = 'Renamed by admin' WHERE id = 'seeded-ansible'",
            [],
        )
        .expect("rename");
        store::environments::delete_environment(&conn, "seeded-opentofu").expect("delete");
        seed_environments(&conn).expect("second seed");
        let name: String = conn
            .query_row(
                "SELECT name FROM environments WHERE id = 'seeded-ansible'",
                [],
                |row| row.get(0),
            )
            .expect("renamed");
        assert_eq!(name, "Renamed by admin");
        let deleted: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM environments WHERE id = 'seeded-opentofu'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(deleted, 0);
        assert_eq!(environment_count(&conn), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
