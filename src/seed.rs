use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use rusqlite::Connection;
use serde_json::json;
use uuid::Uuid;

use crate::config::Config;
use crate::crypto::{self, hash_password};
use crate::db::{self, Environment, Hook, Inventory, InventorySource, Project, Template, Workflow};
use crate::error::AppResult;

pub fn seed(conn: &Connection, cfg: &Config) -> AppResult<Option<String>> {
    let tx = conn.unchecked_transaction()?;
    db::purge_sessions(&tx)?;
    let generated = ensure_admin(&tx, cfg)?;
    if db::count_teams(&tx)? == 0 {
        seed_demo(&tx, cfg)?;
    }
    tx.commit()?;
    if let Some(password) = &generated {
        write_password_file(&cfg.data_dir.join("initial-admin-password"), password)?;
    }
    Ok(generated)
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
        tracing::warn!("Das Admin-Passwort ist kürzer als 8 Zeichen.");
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
        "Initiales Admin-Passwort liegt in dieser Datei. Danach löschen."
    );
    Ok(())
}

fn seed_demo(conn: &Connection, cfg: &Config) -> AppResult<()> {
    let admin = db::user_by_username(conn, &cfg.admin_user)?
        .ok_or_else(|| crate::error::ApiError::Internal("Admin fehlt nach dem Anlegen.".into()))?;
    let organization_id = Uuid::new_v4().to_string();
    db::insert_organization(
        conn,
        &organization_id,
        "Plattform",
        "Beispiel-Organisation für das Demo-Team.",
    )?;
    let team_id = Uuid::new_v4().to_string();
    db::insert_team(
        conn,
        &team_id,
        &organization_id,
        "Plattform",
        "Beispiel-Team. LDAP-Gruppe kann später auf den Gruppennamen gesetzt werden.",
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
            created_at: db::now(),
            team_ids: Vec::new(),
        },
    )?;

    let inventory_id = Uuid::new_v4().to_string();
    db::insert_inventory(
        conn,
        &Inventory {
            id: inventory_id.clone(),
            team_id: team_id.clone(),
            name: "Lokal".into(),
            body: "[all]\nlocalhost ansible_connection=local\n".into(),
            created_at: db::now(),
            source_kind: "ini".into(),
            source_json: "{}".into(),
            credential_id: None,
        },
    )?;
    db::insert_inventory_source(
        conn,
        &InventorySource {
            id: Uuid::new_v4().to_string(),
            inventory_id: inventory_id.clone(),
            name: "Manuell".into(),
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
            image: "quay.io/ansible/awx-ee:latest".into(),
            pull_policy: "if_not_present".into(),
            description: "Gleiches Prinzip wie ein AWX Execution Environment. Der Galaxy-Cache bleibt auf dem Host gemountet und wird nicht pro Lauf geleert.".into(),
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
            description: "Schneller Shell-Check, läuft ohne Ansible.",
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
            description: "Playbook schreibt einen Bericht und löscht nur explizit genannte Dateinamen im Job-Verzeichnis.",
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
            description: "Läuft, wenn der Healthcheck fehlschlägt.",
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
            description: "OpenTofu plan mit persistentem Plugin-Cache.",
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
            description: "Healthcheck, bei Erfolg das Playbook, bei Fehler die Eskalation.".into(),
            graph: graph.to_string(),
            created_at: db::now(),
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
            entrypoint: item.entrypoint.to_string(),
            extra_vars: item.extra_vars.to_string(),
            spec: item.spec.to_string(),
            created_at: db::now(),
        },
    )?;
    Ok(id)
}
