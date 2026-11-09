use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use koreograf::{Config, CookieSecure, LocalJobs, RunnerMode};
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
        mcp: false,
        data_host: None,
        ldap: None,
    }
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, Option<String>) {
    call_team(app, method, uri, cookie, None, body).await
}

async fn call_team(
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

#[tokio::test]
async fn postgres_serves_the_auth_and_resource_slice() {
    let Ok(url) = std::env::var("KOREOGRAF_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is not set");
        return;
    };
    if url.trim().is_empty() {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is empty");
        return;
    }
    let database = koreograf::parse_database_config(&url, None, None).expect("postgres url");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-postgres-{nanos}"));
    let mut cfg = config(&dir);
    cfg.admin_user = format!("koro{nanos}");
    let state = koreograf::boot_with(cfg, database).expect("boot");
    assert_eq!(state.engine(), "postgresql");
    let app = koreograf::router(state);

    let (status, health, _) = call(&app, "GET", "/healthz", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(health, Value::String("ok (postgresql)".into()));

    let (status, version, _) = call(&app, "GET", "/api/version", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(version["version"], env!("CARGO_PKG_VERSION"));

    let (status, _, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({"username": format!("koro{nanos}"), "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("session cookie");

    let (status, me, _) = call(&app, "GET", "/api/me", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["username"], format!("koro{nanos}"));
    assert_eq!(me["is_admin"], true);

    let (status, settings, _) = call(&app, "GET", "/api/settings", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings["runner"], "local");

    let (status, saved, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        Some(json!({
            "ldap": {},
            "proxy": {},
            "runner": "local",
            "job_timeout_secs": 60,
            "job_output_lines": 0
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["source"], "saved");

    let (status, _, _) = call(&app, "GET", "/api/teams", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);

    let (status, org, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        Some(json!({
            "name": format!("Org {nanos}"),
            "description": "Postgres organization"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org}");
    let org_id = org["id"].as_str().expect("organization id").to_string();

    let (status, created_org, _) = call(
        &app,
        "GET",
        &format!("/api/organizations/{org_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created_org["name"], format!("Org {nanos}"));

    let (status, updated_org, _) = call(
        &app,
        "PUT",
        &format!("/api/organizations/{org_id}"),
        Some(&cookie),
        Some(json!({
            "name": format!("Org {nanos} renamed"),
            "description": "Updated"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated_org["description"], "Updated");

    let (status, list, _) = call(&app, "GET", "/api/organizations", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(list.as_array().is_some_and(|items| !items.is_empty()));

    let (status, team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(json!({
            "name": format!("Team {nanos}"),
            "description": "Postgres team",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{team}");
    let team_id = team["id"].as_str().expect("team id").to_string();

    let (status, updated_team, _) = call(
        &app,
        "PUT",
        &format!("/api/teams/{team_id}"),
        Some(&cookie),
        Some(json!({
            "name": format!("Team {nanos} renamed"),
            "description": "Updated team",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated_team["description"], "Updated team");

    let (status, members, _) = call(
        &app,
        "GET",
        &format!("/api/teams/{team_id}/members"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(members.as_array().is_some_and(|items| !items.is_empty()));

    let (status, credential, _) = call_team(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Credential {nanos}"),
            "kind": "scm",
            "secret": {"username": "git", "password": "secret"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"]
        .as_str()
        .expect("credential id")
        .to_string();

    let (status, credentials, _) = call_team(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(credentials
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == credential_id)));

    let (status, opened, _) = call_team(
        &app,
        "GET",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(opened["kind"], "scm");

    let (status, saved, _) = call_team(
        &app,
        "PUT",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Credential {nanos} renamed"),
            "secret": {"password": "next-secret"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["name"], format!("Credential {nanos} renamed"));

    let project_source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples");
    let (status, project, _) = call_team(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Project {nanos}"),
            "source_type": "path",
            "source": project_source,
            "team_ids": [],
            "credential_id": credential_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().expect("project id").to_string();

    let (status, projects, _) = call_team(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(projects
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == project_id)));

    let (status, fetched, _) = call_team(
        &app,
        "GET",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["organization_id"], org_id);

    let (status, updated, _) = call_team(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({"name": format!("Project {nanos} renamed")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], format!("Project {nanos} renamed"));

    let (status, conflict, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict["error"], "credential_is_in_use");

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, inventory, _) = call_team(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Inventory {nanos}"),
            "body": "[all]\nlocalhost ansible_connection=local\n"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().expect("inventory id").to_string();

    let (status, inventories, _) = call_team(
        &app,
        "GET",
        "/api/inventories",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(inventories
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == inventory_id)));

    let (status, updated, _) = call_team(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Inventory {nanos} renamed"),
            "body": "[all]\nlocalhost ansible_connection=local\n[web]\nweb1\n"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["host_count"], 2);

    let (status, with_source, _) = call_team(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "kind": "ini",
            "name": "extra",
            "body": "[db]\ndb1\n"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{with_source}");
    let source_id = with_source["sources"]
        .as_array()
        .and_then(|items| items.last())
        .and_then(|item| item["id"].as_str())
        .expect("source id")
        .to_string();

    let (status, source, _) = call_team(
        &app,
        "GET",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(source["kind"], "ini");

    let (status, updated_source, _) = call_team(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({"name": "extra renamed", "body": "[db]\ndb1\ndb2\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated_source}");
    assert_eq!(updated_source["name"], format!("Inventory {nanos} renamed"));

    let (status, after_delete, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after_delete["source_count"], 1);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body, _) = call(&app, "GET", "/api/users", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body["error"],
        "this_feature_is_not_available_with_postgresql_yet"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
