#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use common::{Options, TestApp};
use koreograf::Shared;
use rusqlite::params;
use serde_json::{json, Value};

async fn add_member(app: &TestApp, admin: &str, team: &str, username: &str, role: &str) -> String {
    let (status, body) = app
        .call(
            "POST",
            "/api/users",
            Some(admin),
            None,
            Some(json!({
                "username": username,
                "display_name": username,
                "password": "member-pass"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, body) = app
        .call(
            "POST",
            &format!("/api/teams/{team}/members"),
            Some(admin),
            Some(team),
            Some(json!({"username": username, "role": role})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    app.login(username, "member-pass").await
}

fn demo() -> String {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/demo")
        .display()
        .to_string()
}

fn insert_failed_job(state: &Shared, id: &str, team: &str, template: &str, parameters: Value) {
    let db = state.db().unwrap();
    db.execute(
        "INSERT INTO jobs (
            id, team_id, template_id, workflow_job_id, workflow_node_id, name, tool, status,
            extra_vars, launched_by, trigger, started_at, finished_at, created_at,
            parameters, failed_hosts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            id,
            team,
            template,
            Option::<String>::None,
            Option::<String>::None,
            "Relaunch",
            "ansible",
            "failed",
            "{}",
            "admin",
            "manual",
            Option::<String>::None,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
            parameters.to_string(),
            "[]"
        ],
    )
    .unwrap();
}

#[tokio::test]
async fn non_admins_share_one_key_for_missing_and_disallowed_project_paths() {
    let app = TestApp::boot("uniform-paths", Options::default());
    let admin = app.login("admin", common::ADMIN_PASSWORD).await;
    let (status, me) = app.call("GET", "/api/me", Some(&admin), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let operator = add_member(&app, &admin, &team, "path-operator", "operator").await;
    let viewer = add_member(&app, &admin, &team, "path-viewer", "viewer").await;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let missing = std::env::temp_dir()
        .join(format!("koreograf-missing-{nanos}"))
        .display()
        .to_string();

    for source in [&missing, "/etc"] {
        let (status, body) = app
            .call(
                "POST",
                "/api/projects",
                Some(&operator),
                Some(&team),
                Some(json!({
                    "name": "Path project",
                    "source_type": "path",
                    "source": source
                })),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "the_project_path_is_not_allowed", "{body}");
    }

    for source in [&missing, "/etc"] {
        let (status, body) = app
            .call(
                "POST",
                "/api/projects",
                Some(&viewer),
                Some(&team),
                Some(json!({
                    "name": "Path project",
                    "source_type": "path",
                    "source": source
                })),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error"], "you_are_not_allowed_to_do_this", "{body}");
    }

    let (status, body) = app
        .call(
            "POST",
            "/api/projects",
            Some(&admin),
            Some(&team),
            Some(json!({
                "name": "Admin missing path",
                "source_type": "path",
                "source": missing
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["error"], "the_project_path_is_not_a_directory",
        "{body}"
    );

    let (status, body) = app
        .call(
            "POST",
            "/api/projects",
            Some(&admin),
            Some(&team),
            Some(json!({
                "name": "Admin denied path",
                "source_type": "path",
                "source": "/etc"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "the_project_path_is_not_allowed", "{body}");
}

#[tokio::test]
async fn foreign_ids_answer_404_not_found() {
    let app = TestApp::boot("uniform-ids", Options::default());
    let admin = app.login("admin", common::ADMIN_PASSWORD).await;
    let (status, me) = app.call("GET", "/api/me", Some(&admin), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();
    let owner = add_member(&app, &admin, &team, "foreign-owner", "owner").await;

    let (status, other) = app
        .call(
            "POST",
            "/api/teams",
            Some(&admin),
            Some(&team),
            Some(json!({"name": "Foreign team"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_team = other["id"].as_str().unwrap().to_string();

    let (status, credential) = app
        .call(
            "POST",
            "/api/credentials",
            Some(&admin),
            Some(&other_team),
            Some(json!({
                "name": "Foreign SCM",
                "kind": "scm",
                "secret": {"password": "foreign-token"}
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let foreign_credential = credential["id"].as_str().unwrap().to_string();

    let (status, foreign_ssh) = app
        .call(
            "POST",
            "/api/credentials",
            Some(&admin),
            Some(&other_team),
            Some(json!({
                "name": "Foreign SSH",
                "kind": "ssh",
                "secret": {"username": "foreign", "password": "foreign-pass"}
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{foreign_ssh}");
    let foreign_ssh = foreign_ssh["id"].as_str().unwrap().to_string();

    let (status, inventory) = app
        .call(
            "POST",
            "/api/inventories",
            Some(&admin),
            Some(&other_team),
            Some(json!({"name": "Foreign inventory", "body": "[web]\nweb-1\n"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let foreign_inventory = inventory["id"].as_str().unwrap().to_string();

    let (status, project) = app
        .call(
            "POST",
            "/api/projects",
            Some(&admin),
            Some(&other_team),
            Some(json!({
                "name": "Foreign project",
                "source_type": "path",
                "source": demo()
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let foreign_project = project["id"].as_str().unwrap().to_string();

    for credential_id in [
        foreign_credential.as_str(),
        foreign_ssh.as_str(),
        "missing-credential-id",
    ] {
        let (status, body) = app
            .call(
                "POST",
                "/api/projects",
                Some(&owner),
                Some(&team),
                Some(json!({
                    "name": "Foreign credential project",
                    "source_type": "path",
                    "source": demo(),
                    "credential_id": credential_id
                })),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["error"], "not_found", "{body}");
    }

    let (status, body) = app
        .call(
            "POST",
            "/api/inventories",
            Some(&owner),
            Some(&team),
            Some(json!({
                "name": "Foreign NetBox inventory",
                "sources": [{
                    "kind": "netbox",
                    "credential_id": foreign_credential,
                    "api_endpoint": "http://127.0.0.1:9"
                }]
            })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "not_found", "{body}");

    for uri in [
        format!("/api/inventories/{foreign_inventory}"),
        format!("/api/projects/{foreign_project}"),
        format!("/api/credentials/{foreign_credential}"),
    ] {
        let (status, body) = app.call("GET", &uri, Some(&owner), Some(&team), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
        assert_eq!(body["error"], "not_found", "{uri}: {body}");
    }
}

#[tokio::test]
async fn foreign_ids_in_templates_and_workflows_answer_404() {
    let app = TestApp::boot("uniform-work", Options::default());
    let admin = app.login("admin", common::ADMIN_PASSWORD).await;
    let (status, me) = app.call("GET", "/api/me", Some(&admin), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();
    let owner = add_member(&app, &admin, &team, "work-owner", "owner").await;

    let (status, other) = app
        .call(
            "POST",
            "/api/teams",
            Some(&admin),
            Some(&team),
            Some(json!({"name": "Work foreign team"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_team = other["id"].as_str().unwrap().to_string();

    let (status, credential) = app
        .call(
            "POST",
            "/api/credentials",
            Some(&admin),
            Some(&other_team),
            Some(json!({
                "name": "Work foreign credential",
                "kind": "scm",
                "secret": {"password": "foreign-token"}
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let foreign_credential = credential["id"].as_str().unwrap().to_string();

    let (status, inventory) = app
        .call(
            "POST",
            "/api/inventories",
            Some(&admin),
            Some(&other_team),
            Some(json!({"name": "Work foreign inventory", "body": "[web]\nweb-1\n"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let foreign_inventory = inventory["id"].as_str().unwrap().to_string();

    let (status, template) = app
        .call(
            "POST",
            "/api/templates",
            Some(&owner),
            Some(&team),
            Some(json!({
                "name": "Node template",
                "tool": "ansible",
                "entrypoint": "site.yml"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    for (index, inventory_id) in [foreign_inventory.as_str(), "missing-inventory-id"]
        .into_iter()
        .enumerate()
    {
        let (status, body) = app
            .call(
                "POST",
                "/api/templates",
                Some(&owner),
                Some(&team),
                Some(json!({
                    "name": format!("Bad inventory {index}"),
                    "tool": "ansible",
                    "entrypoint": "site.yml",
                    "inventory_id": inventory_id
                })),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["error"], "not_found", "{body}");
    }

    for (index, credential_id) in [foreign_credential.as_str(), "missing-credential-id"]
        .into_iter()
        .enumerate()
    {
        let (status, body) = app
            .call(
                "POST",
                "/api/templates",
                Some(&owner),
                Some(&team),
                Some(json!({
                    "name": format!("Bad credential {index}"),
                    "tool": "ansible",
                    "entrypoint": "site.yml",
                    "credential_ids": [credential_id]
                })),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["error"], "not_found", "{body}");
    }

    for (index, inventory_id) in [foreign_inventory.as_str(), "missing-inventory-id"]
        .into_iter()
        .enumerate()
    {
        let (status, body) = app
            .call(
                "POST",
                "/api/workflows",
                Some(&owner),
                Some(&team),
                Some(json!({
                    "name": format!("Bad workflow inventory {index}"),
                    "graph": {
                        "nodes": [{
                            "id": "one",
                            "template_id": template_id,
                            "name": "One",
                            "overrides": {"inventory_id": inventory_id}
                        }],
                        "edges": []
                    }
                })),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["error"], "not_found", "{body}");
    }

    for (index, credential_id) in [foreign_credential.as_str(), "missing-credential-id"]
        .into_iter()
        .enumerate()
    {
        let (status, body) = app
            .call(
                "POST",
                "/api/workflows",
                Some(&owner),
                Some(&team),
                Some(json!({
                    "name": format!("Bad workflow credential {index}"),
                    "graph": {
                        "nodes": [{
                            "id": "one",
                            "template_id": template_id,
                            "name": "One",
                            "overrides": {"credential_id": credential_id}
                        }],
                        "edges": []
                    }
                })),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["error"], "not_found", "{body}");
    }
}

#[tokio::test]
async fn foreign_ids_on_relaunch_answer_404() {
    let app = TestApp::boot("uniform-relaunch", Options::default());
    let state = app.state.clone();
    let admin = app.login("admin", common::ADMIN_PASSWORD).await;
    let (status, me) = app.call("GET", "/api/me", Some(&admin), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, other) = app
        .call(
            "POST",
            "/api/teams",
            Some(&admin),
            Some(&team),
            Some(json!({"name": "Relaunch foreign team"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_team = other["id"].as_str().unwrap().to_string();

    let (status, credential) = app
        .call(
            "POST",
            "/api/credentials",
            Some(&admin),
            Some(&other_team),
            Some(json!({
                "name": "Relaunch foreign credential",
                "kind": "scm",
                "secret": {"password": "foreign-token"}
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let foreign_credential = credential["id"].as_str().unwrap().to_string();

    let (status, inventory) = app
        .call(
            "POST",
            "/api/inventories",
            Some(&admin),
            Some(&other_team),
            Some(json!({"name": "Relaunch foreign inventory", "body": "[web]\nweb-1\n"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let foreign_inventory = inventory["id"].as_str().unwrap().to_string();

    let (status, template) = app
        .call(
            "POST",
            "/api/templates",
            Some(&admin),
            Some(&team),
            Some(json!({
                "name": "Relaunch template",
                "tool": "ansible",
                "entrypoint": "site.yml"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    let cases = [
        (
            "foreign-inventory",
            json!({"tool": "ansible", "inventory_id": foreign_inventory}),
        ),
        (
            "unknown-inventory",
            json!({"tool": "ansible", "inventory_id": "missing-inventory-id"}),
        ),
        (
            "foreign-credential",
            json!({"tool": "ansible", "credential_id": foreign_credential}),
        ),
        (
            "unknown-credential",
            json!({"tool": "ansible", "credential_id": "missing-credential-id"}),
        ),
    ];
    for (id, parameters) in cases {
        insert_failed_job(&state, id, &team, &template_id, parameters);
        let (status, body) = app
            .call(
                "POST",
                &format!("/api/jobs/{id}/relaunch"),
                Some(&admin),
                None,
                Some(json!({})),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{id}: {body}");
        assert_eq!(body["error"], "not_found", "{id}: {body}");
    }
}
