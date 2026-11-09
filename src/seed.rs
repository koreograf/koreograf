use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use serde_json::json;
use uuid::Uuid;

use crate::backend::Backend;
use crate::config::Config;
use crate::crypto::{self, hash_password};
use crate::db::{self, Environment, Hook, Inventory, InventorySource, Project, Template, Workflow};
use crate::error::AppResult;

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

pub fn seed(conn: &Connection, cfg: &Config) -> AppResult<Option<String>> {
    let tx = conn.unchecked_transaction()?;
    db::purge_sessions(&tx)?;
    let generated = ensure_admin(&tx, cfg)?;
    if environments_seeded_by_env() {
        seed_environments(&tx)?;
    }
    if db::count_teams(&tx)? == 0 {
        seed_demo(&tx, cfg)?;
    }
    tx.commit()?;
    if let Some(password) = &generated {
        write_password_file(&cfg.data_dir.join("initial-admin-password"), password)?;
    }
    Ok(generated)
}

pub(crate) fn seed_postgres(backend: &dyn Backend, cfg: &Config) -> AppResult<Option<String>> {
    backend.purge_sessions()?;
    if backend.user_by_username(&cfg.admin_user)?.is_some() {
        return Ok(None);
    }
    let (password, generated) = admin_credentials(cfg);
    backend.insert_user(
        &Uuid::new_v4().to_string(),
        &cfg.admin_user,
        "Administrator",
        Some(&hash_password(&password)),
        "local",
        true,
    )?;
    if let Some(password) = &generated {
        write_password_file(&cfg.data_dir.join("initial-admin-password"), password)?;
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

fn ensure_admin(conn: &Connection, cfg: &Config) -> AppResult<Option<String>> {
    if db::user_by_username(conn, &cfg.admin_user)?.is_some() {
        return Ok(None);
    }
    let (password, generated) = match &cfg.admin_password {
        Some(password) => (password.clone(), None),
        None => {
            let password = crypto::random_password();
            (password.clone(), Some(password))
        }
    };
    if password.len() < 8 {
        tracing::warn!("The admin password is shorter than 8 characters.");
    }
    db::insert_user(
        conn,
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

pub(crate) fn seed_environments(conn: &Connection) -> AppResult<()> {
    let done: Option<String> = conn
        .query_row(
            "SELECT value FROM seed_state WHERE key = ?1",
            [SEEDED_ENVIRONMENTS_KEY],
            |row| row.get(0),
        )
        .optional()?;
    if done.as_deref() == Some("1") {
        return Ok(());
    }
    for item in &SEEDED_ENVIRONMENTS {
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM environments WHERE id = ?1",
            [item.id],
            |row| row.get(0),
        )?;
        if exists == 0 {
            db::insert_environment(
                conn,
                &Environment {
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
                },
            )?;
        }
        let key = format!("default_environment_{}", item.tool);
        let current: Option<String> = conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [&key], |row| {
                row.get(0)
            })
            .optional()?;
        if current.unwrap_or_default().trim().is_empty() {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![key, item.id],
            )?;
        }
    }
    conn.execute(
        "INSERT INTO seed_state (key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = '1'",
        [SEEDED_ENVIRONMENTS_KEY],
    )?;
    Ok(())
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

fn seed_demo(conn: &Connection, cfg: &Config) -> AppResult<()> {
    let admin = db::user_by_username(conn, &cfg.admin_user)?.ok_or_else(|| {
        crate::error::ApiError::Internal("Admin user is missing after creation.".into())
    })?;
    let organization_id = Uuid::new_v4().to_string();
    db::insert_organization(
        conn,
        &organization_id,
        "Plattform",
        "Demo organization for the sample team.",
    )?;
    let team_id = Uuid::new_v4().to_string();
    db::insert_team(
        conn,
        &team_id,
        &organization_id,
        "Plattform",
        "Sample team. An LDAP group can later use this group name.",
        None,
    )?;
    db::upsert_membership(conn, &team_id, &admin.id, "owner", "manual")?;

    let project_id = Uuid::new_v4().to_string();
    let source = cfg.examples_dir.join("demo");
    db::insert_project(
        conn,
        &Project {
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
        },
    )?;

    let inventory_id = Uuid::new_v4().to_string();
    db::insert_inventory(
        conn,
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
    )?;
    db::insert_inventory_source(
        conn,
        &InventorySource {
            id: Uuid::new_v4().to_string(),
            inventory_id: inventory_id.clone(),
            name: "manual".into(),
            kind: "ini".into(),
            source_json: "{}".into(),
            credential_id: None,
            body: "[all]\nlocalhost ansible_connection=local\n".into(),
            position: 0,
            created_at: db::now(),
        },
    )?;

    let environment_id = Uuid::new_v4().to_string();
    db::insert_environment(
        conn,
        &Environment {
            id: environment_id,
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
        },
    )?;

    let heal_vars = json!({
        "alertname": "manual",
        "instance": "localhost",
        "heal_action": "noop"
    })
    .to_string();
    let plan_spec = json!({"action": "plan"}).to_string();
    let health_id = insert_template(
        conn,
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
        conn,
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
        conn,
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
        conn,
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
    db::insert_workflow(
        conn,
        &Workflow {
            id: workflow_id.clone(),
            team_id: team_id.clone(),
            name: "Self-Heal Ablauf".into(),
            description: "Health check, then the playbook, or the escalation when it fails.".into(),
            graph: graph.to_string(),
            created_at: db::now(),
            admin_saved: true,
        },
    )?;

    db::insert_hook(
        conn,
        &Hook {
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
        },
    )?;
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

fn insert_template(conn: &Connection, item: DemoTemplate<'_>) -> AppResult<String> {
    let id = Uuid::new_v4().to_string();
    db::insert_template(
        conn,
        &Template {
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
        },
    )?;
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
        db::delete_environment(&conn, "seeded-opentofu").expect("delete");
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
