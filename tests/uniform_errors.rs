use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use koreograf::{Config, CookieSecure, LocalJobs, RunnerMode, Shared};
use rusqlite::params;
use serde_json::{json, Value};
use tower::ServiceExt;

fn config(dir: &Path) -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        data_dir: dir.to_path_buf(),
        web_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
        examples_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
        admin_user: "admin".into(),
        admin_password: Some("test-admin-pass".into()),
        runner: RunnerMode::Local,
        container_image: false,
        job_timeout_secs: 60,
        job_output_lines: 8_000,
        local_jobs: LocalJobs::All,
        project_roots: vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")],
        hsts: false,
        cookie_secure: CookieSecure::Auto,
        trusted_proxy: false,
        mcp: true,
        public_url: Vec::new(),
        data_host: None,
        ldap: None,
    }
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    team: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, Option<String>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    if let Some(team) = team {
        builder = builder.header("x-team-id", team);
    }
    let request = if let Some(body) = body {
        builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::to_string);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).to_string()))
    };
    (status, parsed, cookie)
}

async fn login(app: &axum::Router, username: &str, password: &str) -> String {
    let (status, body, cookie) = call(
        app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": username, "password": password})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    cookie.expect("session cookie")
}

async fn add_member(
    app: &axum::Router,
    admin: &str,
    team: &str,
    username: &str,
    role: &str,
) -> String {
    let (status, body, _) = call(
        app,
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
    let (status, body, _) = call(
        app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(admin),
        Some(team),
        Some(json!({"username": username, "role": role})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    login(app, username, "member-pass").await
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
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-uniform-paths-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);
    let admin = login(&app, "admin", "test-admin-pass").await;
    let (status, me, _) = call(&app, "GET", "/api/me", Some(&admin), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let operator = add_member(&app, &admin, &team, "path-operator", "operator").await;
    let viewer = add_member(&app, &admin, &team, "path-viewer", "viewer").await;
    let missing = std::env::temp_dir()
        .join(format!("koreograf-missing-{nanos}"))
        .display()
        .to_string();

    for source in [&missing, "/etc"] {
        let (status, body, _) = call(
            &app,
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
        let (status, body, _) = call(
            &app,
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

    let (status, body, _) = call(
        &app,
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

    let (status, body, _) = call(
        &app,
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

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn foreign_ids_answer_404_not_found() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-uniform-ids-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);
    let admin = login(&app, "admin", "test-admin-pass").await;
    let (status, me, _) = call(&app, "GET", "/api/me", Some(&admin), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();
    let owner = add_member(&app, &admin, &team, "foreign-owner", "owner").await;

    let (status, other, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&admin),
        Some(&team),
        Some(json!({"name": "Foreign team"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_team = other["id"].as_str().unwrap().to_string();

    let (status, credential, _) = call(
        &app,
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

    let (status, foreign_ssh, _) = call(
        &app,
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

    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&admin),
        Some(&other_team),
        Some(json!({"name": "Foreign inventory", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let foreign_inventory = inventory["id"].as_str().unwrap().to_string();

    let (status, project, _) = call(
        &app,
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
        let (status, body, _) = call(
            &app,
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

    let (status, body, _) = call(
        &app,
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
        let (status, body, _) = call(&app, "GET", &uri, Some(&owner), Some(&team), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
        assert_eq!(body["error"], "not_found", "{uri}: {body}");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn foreign_ids_in_templates_and_workflows_answer_404() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-uniform-work-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);
    let admin = login(&app, "admin", "test-admin-pass").await;
    let (status, me, _) = call(&app, "GET", "/api/me", Some(&admin), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();
    let owner = add_member(&app, &admin, &team, "work-owner", "owner").await;

    let (status, other, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&admin),
        Some(&team),
        Some(json!({"name": "Work foreign team"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_team = other["id"].as_str().unwrap().to_string();

    let (status, credential, _) = call(
        &app,
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

    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&admin),
        Some(&other_team),
        Some(json!({"name": "Work foreign inventory", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let foreign_inventory = inventory["id"].as_str().unwrap().to_string();

    let (status, template, _) = call(
        &app,
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
        let (status, body, _) = call(
            &app,
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
        let (status, body, _) = call(
            &app,
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
        let (status, body, _) = call(
            &app,
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
        let (status, body, _) = call(
            &app,
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

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn foreign_ids_on_relaunch_answer_404() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-uniform-relaunch-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state.clone());
    let admin = login(&app, "admin", "test-admin-pass").await;
    let (status, me, _) = call(&app, "GET", "/api/me", Some(&admin), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, other, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&admin),
        Some(&team),
        Some(json!({"name": "Relaunch foreign team"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_team = other["id"].as_str().unwrap().to_string();

    let (status, credential, _) = call(
        &app,
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

    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&admin),
        Some(&other_team),
        Some(json!({"name": "Relaunch foreign inventory", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let foreign_inventory = inventory["id"].as_str().unwrap().to_string();

    let (status, template, _) = call(
        &app,
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
        let (status, body, _) = call(
            &app,
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

    let _ = std::fs::remove_dir_all(&dir);
}
