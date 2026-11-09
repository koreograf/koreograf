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

async fn postgres_session(
    url: &str,
    user_id: &str,
    username: &str,
    memberships: &[(&str, &str)],
    token: &str,
) {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("postgres connection");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO users
                (id, username, display_name, password_hash, auth_source, is_admin, created_at)
             VALUES ($1, $2, $3, NULL, 'local', 0,
                to_char(now(), 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'))",
            &[&user_id, &username, &username],
        )
        .await
        .expect("insert postgres user");
    for (team_id, role) in memberships {
        client
            .execute(
                "INSERT INTO memberships (team_id, user_id, role, source)
                 VALUES ($1, $2, $3, 'manual')",
                &[team_id, &user_id, role],
            )
            .await
            .expect("insert postgres membership");
    }
    client
        .execute(
            "INSERT INTO sessions (token, user_id, expires_at)
             VALUES ($1, $2, '9999-12-31T23:59:59Z')",
            &[&token, &user_id],
        )
        .await
        .expect("insert postgres session");
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

    let (status, second_team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(json!({
            "name": format!("Second team {nanos}"),
            "description": "Postgres second team",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{second_team}");
    let second_team_id = second_team["id"]
        .as_str()
        .expect("second team id")
        .to_string();

    let nonmember_username = format!("nonmember{nanos}");
    let viewer_username = format!("viewer{nanos}");
    let nonmember_token = format!("nonmember-session-{nanos}");
    let viewer_token = format!("viewer-session-{nanos}");
    postgres_session(
        &url,
        &format!("user-{nanos}-nonmember"),
        &nonmember_username,
        &[(&second_team_id, "owner")],
        &nonmember_token,
    )
    .await;
    postgres_session(
        &url,
        &format!("user-{nanos}-viewer"),
        &viewer_username,
        &[(&team_id, "viewer")],
        &viewer_token,
    )
    .await;
    let nonmember_cookie = format!("koreograf_session={nonmember_token}");
    let viewer_cookie = format!("koreograf_session={viewer_token}");

    for (actor, actor_cookie) in [
        ("non-member", &nonmember_cookie),
        ("viewer", &viewer_cookie),
    ] {
        let (status, denied, _) = call_team(
            &app,
            "POST",
            "/api/credentials",
            Some(actor_cookie),
            Some(&team_id),
            Some(json!({
                "name": format!("Denied {actor}"),
                "kind": "scm",
                "secret": {"username": "git", "password": "secret"}
            })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{actor} create: {denied}");

        let (status, denied, _) = call_team(
            &app,
            "GET",
            &format!("/api/credentials/{credential_id}"),
            Some(actor_cookie),
            Some(&team_id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{actor} get: {denied}");

        let (status, denied, _) = call_team(
            &app,
            "DELETE",
            &format!("/api/projects/{project_id}"),
            Some(actor_cookie),
            Some(&team_id),
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{actor} project delete: {denied}"
        );
    }

    let (status, _, _) = call_team(
        &app,
        "GET",
        &format!("/api/credentials/{credential_id}"),
        Some(&nonmember_cookie),
        Some(&second_team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

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

    let (status, environment, _) = call_team(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Environment {nanos}"),
            "image": "quay.io/example/ee:1",
            "tool": "ansible",
            "pull_policy": "if_not_present",
            "network": "",
            "description": "Postgres environment",
            "scope": "team"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{environment}");
    let environment_id = environment["id"]
        .as_str()
        .expect("environment id")
        .to_string();

    let (status, environments, _) = call_team(
        &app,
        "GET",
        "/api/environments",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(environments
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == environment_id)));

    let (status, opened_environment, _) = call_team(
        &app,
        "GET",
        &format!("/api/environments/{environment_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(opened_environment["tool"], "ansible");

    let (status, updated_environment, _) = call_team(
        &app,
        "PUT",
        &format!("/api/environments/{environment_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Environment {nanos} renamed"),
            "image": "quay.io/example/ee:2",
            "tool": "ansible",
            "pull_policy": "always",
            "network": "",
            "description": "Updated environment",
            "scope": "team"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated_environment}");
    assert_eq!(updated_environment["pull_policy"], "always");

    let (status, invalid_environment, _) = call_team(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Invalid environment {nanos}"),
            "image": "quay.io/example/ee:1",
            "tool": "docker"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid_environment["error"],
        "choose_a_tool_for_the_environment"
    );

    let (status, invalid_network, _) = call_team(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Invalid network {nanos}"),
            "image": "quay.io/example/ee:1",
            "tool": "ansible",
            "network": "bad network!"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid_network["error"], "the_network_name_is_invalid");

    let mut credential_ids = Vec::new();
    for (label, kind, secret) in [
        (
            "ssh",
            "ssh",
            json!({"username": "ansible", "password": "secret"}),
        ),
        (
            "vault-a",
            "vault",
            json!({"password": "one", "vault_id": "prod"}),
        ),
        (
            "vault-b",
            "vault",
            json!({"password": "two", "vault_id": "dev"}),
        ),
        (
            "vault-duplicate",
            "vault",
            json!({"password": "three", "vault_id": "prod"}),
        ),
        ("vault-c", "vault", json!({"password": "four"})),
        ("vault-d", "vault", json!({"password": "five"})),
        (
            "scm",
            "scm",
            json!({"username": "git", "password": "secret"}),
        ),
    ] {
        let (status, credential, _) = call_team(
            &app,
            "POST",
            "/api/credentials",
            Some(&cookie),
            Some(&team_id),
            Some(json!({
                "name": format!("{label} {nanos}"),
                "kind": kind,
                "secret": secret
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{label}: {credential}");
        credential_ids.push(
            credential["id"]
                .as_str()
                .expect("credential id")
                .to_string(),
        );
    }
    let (ssh_id, vault_a, vault_b, vault_duplicate, vault_c, vault_d, scm_id) = (
        &credential_ids[0],
        &credential_ids[1],
        &credential_ids[2],
        &credential_ids[3],
        &credential_ids[4],
        &credential_ids[5],
        &credential_ids[6],
    );

    let template_body = |name: &str, ids: Vec<&String>, no_environment: bool| {
        json!({
            "name": name,
            "description": "Postgres template",
            "tool": "ansible",
            "credential_ids": ids,
            "environment_id": environment_id,
            "no_environment": no_environment,
            "entrypoint": "site.yml",
            "prompt_on_launch": true
        })
    };

    let (status, duplicate_vault, _) = call_team(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team_id),
        Some(template_body(
            &format!("Duplicate vault {nanos}"),
            vec![ssh_id, vault_a, vault_duplicate],
            false,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        duplicate_vault["error"],
        "this_vault_id_is_already_selected"
    );

    let (status, duplicate_default, _) = call_team(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team_id),
        Some(template_body(
            &format!("Duplicate default vault {nanos}"),
            vec![vault_c, vault_d],
            false,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        duplicate_default["error"],
        "only_one_vault_credential_may_omit_the_vault"
    );

    let (status, template, _) = call_team(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team_id),
        Some(template_body(
            &format!("Template {nanos}"),
            vec![ssh_id, vault_a, vault_b],
            false,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().expect("template id").to_string();
    assert_eq!(template["credential_id"], ssh_id.as_str());
    assert_eq!(template["credential_ids"].as_array().map(Vec::len), Some(3));
    assert_eq!(template["prompt_on_launch"], true);
    assert_eq!(template["no_environment"], false);

    let (status, templates, _) = call_team(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(templates
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == template_id)));

    let (status, updated_template, _) = call_team(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Template {nanos} renamed"),
            "description": "Updated template",
            "tool": "ansible",
            "credential_ids": [ssh_id],
            "no_environment": true,
            "entrypoint": "site.yml",
            "prompt_on_launch": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated_template}");
    assert_eq!(updated_template["credential_ids"], json!([ssh_id]));
    assert_eq!(updated_template["no_environment"], true);
    assert_eq!(updated_template["prompt_on_launch"], false);

    let (status, wrong_kind, _) = call_team(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Template {nanos} renamed"),
            "tool": "ansible",
            "credential_ids": [scm_id],
            "entrypoint": "site.yml"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        wrong_kind["error"],
        "this_credential_kind_does_not_fit_the_tool"
    );

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let schedule_body = |name: &str, time: &str| {
        json!({
            "name": name,
            "timezone": "Europe/Berlin",
            "start_date": today,
            "start_time": time,
            "frequency": "daily",
            "weekdays": [],
            "end_kind": "never"
        })
    };

    let (status, invalid_schedule, _) = call_team(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Invalid schedule {nanos}"),
            "timezone": "Europe/Berlin",
            "start_date": today,
            "start_time": "03:00",
            "frequency": "yearly",
            "end_kind": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid_schedule["error"], "schedule_repeat_is_invalid");

    let (status, schedule, _) = call_team(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team_id),
        Some(schedule_body(&format!("Schedule {nanos}"), "03:00")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{schedule}");
    let schedule_id = schedule["id"].as_str().expect("schedule id").to_string();
    assert!(schedule["next_run_at"].is_string());

    let (status, schedules, _) = call_team(
        &app,
        "GET",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(schedules
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == schedule_id)));

    let (status, team_schedules, _) = call_team(
        &app,
        "GET",
        "/api/schedules",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(team_schedules
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == schedule_id
            && item["target_name"] == format!("Template {nanos} renamed"))));

    let (status, updated_schedule, _) = call_team(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(schedule_body(&format!("Schedule {nanos} renamed"), "04:00")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated_schedule}");
    assert_eq!(
        updated_schedule["name"],
        format!("Schedule {nanos} renamed")
    );

    let (status, disabled, _) = call_team(
        &app,
        "POST",
        &format!("/api/schedules/{schedule_id}/enabled"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{disabled}");
    assert_eq!(disabled["enabled"], false);

    let (status, hook, _) = call_team(
        &app,
        "POST",
        "/api/hooks",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Hook {nanos}"),
            "match_alertname": "HighCPU",
            "match_status": "firing",
            "template_id": template_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");
    let hook_id = hook["id"].as_str().expect("hook id").to_string();
    assert!(hook["token"]
        .as_str()
        .is_some_and(|token| !token.is_empty()));

    let (status, hooks, _) = call_team(
        &app,
        "GET",
        "/api/hooks",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(hooks
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == hook_id)));

    let (status, updated_hook, _) = call_team(
        &app,
        "PUT",
        &format!("/api/hooks/{hook_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Hook {nanos} renamed"),
            "match_alertname": "HighCPU",
            "match_status": "resolved",
            "template_id": template_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated_hook}");
    assert_eq!(updated_hook["match_status"], "resolved");

    let (status, invalid_hook, _) = call_team(
        &app,
        "PUT",
        &format!("/api/hooks/{hook_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Hook {nanos} renamed"),
            "match_alertname": "HighCPU",
            "match_status": "broken",
            "template_id": template_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid_hook["error"], "status_is_firing_resolved_or_any");

    for (label, method, uri, body) in [
        (
            "environment",
            "POST",
            "/api/environments".to_string(),
            Some(json!({
                "name": format!("Denied environment {nanos}"),
                "image": "quay.io/example/ee:1",
                "tool": "ansible"
            })),
        ),
        (
            "template",
            "POST",
            "/api/templates".to_string(),
            Some(json!({
                "name": format!("Denied template {nanos}"),
                "tool": "ansible",
                "entrypoint": "site.yml"
            })),
        ),
        (
            "schedule",
            "POST",
            format!("/api/templates/{template_id}/schedules"),
            Some(schedule_body(&format!("Denied schedule {nanos}"), "05:00")),
        ),
        (
            "hook",
            "POST",
            "/api/hooks".to_string(),
            Some(json!({
                "name": format!("Denied hook {nanos}"),
                "match_alertname": "HighCPU",
                "template_id": template_id
            })),
        ),
    ] {
        let (status, denied, _) = call_team(
            &app,
            method,
            &uri,
            Some(&viewer_cookie),
            Some(&team_id),
            body,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{label}: {denied}");
    }

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/hooks/{hook_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/environments/{environment_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    for credential_id in &credential_ids {
        let (status, _, _) = call_team(
            &app,
            "DELETE",
            &format!("/api/credentials/{credential_id}"),
            Some(&cookie),
            Some(&team_id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{credential_id}");
    }

    let (status, body, _) = call(&app, "GET", "/api/users", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body["error"],
        "this_feature_is_not_available_with_postgresql_yet"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
