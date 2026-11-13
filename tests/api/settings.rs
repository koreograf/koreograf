use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use koreograf::{LocalJobs, RunnerMode};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::common::{call_bearer_team as call_bearer, call_cookie as call};

use crate::common::*;

#[tokio::test]
async fn responses_carry_standard_headers() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-headers-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir, &Options::default())).unwrap();
    assert!(!state.cfg.hsts);
    let app = koreograf::router(state);
    for path in [
        "/",
        "/js/features/legacy.js",
        "/api/me",
        "/healthz",
        "/metrics",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let headers = response.headers();
        assert_eq!(headers["x-content-type-options"], "nosniff", "{path}");
        assert_eq!(headers["x-frame-options"], "DENY", "{path}");
        let policy = headers["content-security-policy"].to_str().unwrap();
        assert!(policy.contains("frame-ancestors 'none'"), "{path} {policy}");
        assert!(
            policy.contains("style-src-attr 'unsafe-inline'"),
            "{path} {policy}"
        );
        assert_eq!(headers["referrer-policy"], "no-referrer", "{path}");
        assert_eq!(
            headers["permissions-policy"], "camera=(), microphone=(), geolocation=()",
            "{path}"
        );
        assert!(headers.get("strict-transport-security").is_none(), "{path}");
    }
    drop(app);

    let mut enabled = config(&dir, &Options::default());
    enabled.hsts = true;
    let state = koreograf::boot(enabled).unwrap();
    let app = koreograf::router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.headers()["strict-transport-security"],
        "max-age=31536000"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn owner_can_edit_an_execution_environment() {
    let harness = TestApp::boot("env", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/features/legacy.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
    let runtime = script
        .split_once("async function pageRuntime")
        .expect("runtime page")
        .1
        .split_once("function openEnvironmentDialog")
        .expect("environment form")
        .0;
    let environments = runtime
        .find("execution_environments")
        .expect("environment heading");
    let collections = runtime.find("cached_collections").expect("cache heading");
    assert!(environments < collections);
    assert!(runtime.contains("#/environments/"));

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();
    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/environments",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let id = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "AWX-EE")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let image = "quay.io/ansible/awx-ee@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "EE neu",
            "image": image,
            "tool": "ansible",
            "pull_policy": "always",
            "network": "koreograf_default",
            "description": "angepasst"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "EE neu");
    assert_eq!(updated["image"], image);
    assert_eq!(updated["pull_policy"], "always");
    assert_eq!(updated["network"], "koreograf_default");
    assert_eq!(updated["scope"], "team");
    assert_eq!(updated["team_id"].as_str(), Some(team.as_str()));
    assert!(updated["organization_id"].is_null());
    let (status, again, _) = call(
        &app,
        "GET",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["description"], "angepasst");
    assert_eq!(again["network"], "koreograf_default");
    let (status, rejected, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "EE neu",
            "image": "bad image",
            "tool": "ansible",
            "pull_policy": "always"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["error"], "the_image_name_is_invalid");
    let (status, rejected, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "EE neu",
            "image": image,
            "tool": "ansible",
            "pull_policy": "always",
            "network": "--net=host"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["error"], "the_network_name_is_invalid");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_environment_tool_limits_the_templates_that_can_use_it() {
    let harness = TestApp::boot("env-tool", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap();

    let (status, environment, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Tofu",
            "image": "ghcr.io/opentofu/opentofu:latest",
            "tool": "opentofu"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{environment}");
    assert_eq!(environment["tool"], "opentofu");
    let environment_id = environment["id"].as_str().unwrap();

    let (status, rejected, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Ansible",
            "tool": "ansible",
            "entrypoint": "site.yml",
            "environment_id": environment_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(
        rejected["error"],
        "the_execution_environment_is_for_another_tool"
    );

    let (status, accepted, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Tofu",
            "tool": "opentofu",
            "entrypoint": "module",
            "environment_id": environment_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{accepted}");
    assert_eq!(accepted["environment_id"], environment_id);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn editing_an_environment_clears_a_tool_default_it_no_longer_fits() {
    let harness = TestApp::boot("env-default-edit", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap();

    let (status, environment, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Default EE",
            "image": "quay.io/ansible/awx-ee:latest",
            "tool": "ansible",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{environment}");
    let id = environment["id"].as_str().unwrap();

    let settings_body = |ansible: &str, opentofu: &str| {
        json!({
            "ldap": {"url": "", "mode": "ldap"},
            "proxy": {"url": ""},
            "runner": "local",
            "job_timeout_secs": 60,
            "default_environment_ansible": ansible,
            "default_environment_opentofu": opentofu
        })
    };

    let (status, settings, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        Some(team),
        Some(settings_body(id, "")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{settings}");
    assert_eq!(settings["default_environment_ansible"], id);

    let (status, renamed, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Default EE renamed",
            "image": "quay.io/ansible/awx-ee:latest",
            "tool": "ansible",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["name"], "Default EE renamed");
    let (status, kept, _) = call(
        &app,
        "GET",
        "/api/settings",
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["default_environment_ansible"], id);

    let (status, changed, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Default EE renamed",
            "image": "ghcr.io/opentofu/opentofu:latest",
            "tool": "opentofu",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert_eq!(changed["tool"], "opentofu");
    let (status, cleared, _) = call(
        &app,
        "GET",
        "/api/settings",
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["default_environment_ansible"], "");

    let (status, again, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        Some(team),
        Some(settings_body("", id)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["default_environment_opentofu"], id);

    let (status, moved, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Default EE renamed",
            "image": "ghcr.io/opentofu/opentofu:latest",
            "tool": "opentofu",
            "scope": "team"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["scope"], "team");
    let (status, cleared, _) = call(
        &app,
        "GET",
        "/api/settings",
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["default_environment_opentofu"], "");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_opentofu_template_without_an_environment_uses_the_tool_default() {
    let harness = TestApp::boot("env-default", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap();

    let (status, environment, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Tofu default",
            "image": "ghcr.io/opentofu/opentofu:latest",
            "tool": "opentofu",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{environment}");
    let environment_id = environment["id"].as_str().unwrap();

    let (status, settings, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        Some(team),
        Some(json!({
            "ldap": {"url": "", "mode": "ldap"},
            "proxy": {"url": ""},
            "runner": "local",
            "job_timeout_secs": 60,
            "default_environment_opentofu": environment_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{settings}");
    assert_eq!(settings["default_environment_opentofu"], environment_id);

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Tofu ohne Umgebung",
            "tool": "opentofu",
            "entrypoint": "module"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    assert!(template["environment_id"].is_null());
    assert_eq!(template["no_environment"], false);
    let template_id = template["id"].as_str().unwrap();

    let job = launch_and_wait(&app, &cookie, team, template_id).await;
    assert_eq!(job["parameters"]["environment_id"], environment_id);
    assert_eq!(job["parameters"]["environment_name"], "Tofu default");
    assert_eq!(
        job["parameters"]["environment_image"],
        "ghcr.io/opentofu/opentofu:latest"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_fresh_install_seeds_a_default_environment_per_tool() {
    let harness = TestApp::boot("seeded-ee", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap();

    let (status, environments, _) = call(
        &app,
        "GET",
        "/api/environments",
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{environments}");
    let global = environments
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["scope"] == "global")
        .collect::<Vec<_>>();
    assert_eq!(global.len(), 3, "{environments}");
    for tool in ["ansible", "opentofu", "shell"] {
        assert!(
            global.iter().any(|row| row["tool"] == tool),
            "missing {tool}: {environments}"
        );
    }

    let (status, settings, _) = call(
        &app,
        "GET",
        "/api/settings",
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{settings}");
    for tool in ["ansible", "opentofu", "shell"] {
        let id = settings
            .get(format!("default_environment_{tool}"))
            .and_then(Value::as_str)
            .unwrap_or("");
        assert!(!id.is_empty(), "missing default for {tool}: {settings}");
        assert!(
            global
                .iter()
                .any(|row| row["id"] == id && row["tool"] == tool),
            "default for {tool} is not the seeded environment: {settings}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_job_without_a_default_environment_fails_before_it_starts() {
    let dir = temp_dir("no-default-ee");
    let mut cfg = config(&dir, &Options::default());
    cfg.container_image = true;
    let state = koreograf::boot(cfg).unwrap();
    let app = koreograf::router(state);

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap();

    let (status, cleared, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        Some(team),
        Some(json!({
            "ldap": {"url": "", "mode": "ldap"},
            "proxy": {"url": ""},
            "runner": "auto",
            "job_timeout_secs": 60,
            "default_environment_ansible": "",
            "default_environment_opentofu": "",
            "default_environment_shell": ""
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Shell ohne Standard",
            "tool": "shell",
            "entrypoint": "run.sh"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();

    let (status, rejected, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["error"], "no_default_execution_environment");
    assert_eq!(rejected["vars"]["tool"], "Shell");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_changed_environment_tool_fails_the_job_before_the_container_starts() {
    let dir = temp_dir("env-tool-job");
    let mut cfg = config(&dir, &Options::default());
    cfg.runner = RunnerMode::Docker;
    let state = koreograf::boot(cfg).unwrap();
    let app = koreograf::router(state);

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap();

    let (status, environment, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Any",
            "image": "quay.io/example/any:1",
            "tool": "shell"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{environment}");
    let environment_id = environment["id"].as_str().unwrap();

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Shell",
            "tool": "shell",
            "entrypoint": "",
            "environment_id": environment_id,
            "spec": {"script": "true"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();

    let (status, changed, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{environment_id}"),
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Any",
            "image": "quay.io/example/any:1",
            "tool": "ansible"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert_eq!(changed["tool"], "ansible");

    let job = launch_and_wait(&app, &cookie, team, template_id).await;
    assert_eq!(job["status"], "failed", "{}", job["events"]);
    assert!(
        has_line(
            &job,
            "system",
            "the_execution_environment_is_for_another_tool"
        ),
        "{}",
        job["events"]
    );
    assert!(
        !job["events"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["line"]
                .as_str()
                .is_some_and(|line| line.starts_with("Execution Environment ")))),
        "{}",
        job["events"]
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn environment_scopes_follow_the_selected_team() {
    let harness = TestApp::boot("env-scope", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/features/legacy.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
    assert!(script.contains("applies_to"));
    assert!(script.contains("this_team"));
    assert!(script.contains("this_organization"));
    assert!(script.contains("organization_name"));
    assert!(script.contains("team_name"));
    assert!(script.contains("global_applies_to_every_team_the_organization_applies"));
    assert!(script.contains("canManageEnvironment"));
    assert!(script.contains("scopeLabel(row)"));

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();
    let org_id = me["teams"][0]["organization_id"]
        .as_str()
        .unwrap()
        .to_string();
    let org_name = me["teams"][0]["organization_name"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, other_org, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Andere", "description": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other_org}");
    let other_org_id = other_org["id"].as_str().unwrap();
    let (status, sibling, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Betrieb",
            "description": "Gleiches Organisation",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{sibling}");
    let sibling = sibling["id"].as_str().unwrap().to_string();
    let (status, foreign, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Nacht",
            "description": "Andere Organisation",
            "organization_id": other_org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{foreign}");
    let foreign = foreign["id"].as_str().unwrap().to_string();

    let (status, _user, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "ada",
            "display_name": "Ada",
            "password": "ada-secret-1",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    for (target, role) in [(&team, "owner"), (&sibling, "owner")] {
        let (status, joined, _) = call(
            &app,
            "POST",
            &format!("/api/teams/{target}/members"),
            Some(&cookie),
            Some(&team),
            Some(json!({"username": "ada", "role": role})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{joined}");
    }
    let (status, ada_me, ada_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "ada", "password": "ada-secret-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ada_me}");
    let ada_cookie = ada_cookie.expect("ada cookie");

    let (status, private_env, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Privat",
            "image": "quay.io/example/private:1",
            "tool": "ansible",
            "pull_policy": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{private_env}");
    assert_eq!(private_env["scope"], "team");
    assert_eq!(private_env["team_id"].as_str(), Some(team.as_str()));
    assert!(private_env["organization_id"].is_null());
    let private_id = private_env["id"].as_str().unwrap().to_string();

    for network in ["host", "container:koreograf-koreograf-1"] {
        let (status, denied_network, _) = call(
            &app,
            "POST",
            "/api/environments",
            Some(&ada_cookie),
            Some(&team),
            Some(json!({
                "name": "Privilegiert",
                "image": "quay.io/example/private:1",
                "tool": "ansible",
                "network": network
            })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{denied_network}");
        assert_eq!(denied_network["error"], "you_are_not_allowed_to_do_this");
    }
    let (status, denied_update, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{private_id}"),
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Privat",
            "image": "quay.io/example/private:1",
            "tool": "ansible",
            "network": "host"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied_update}");
    assert_eq!(denied_update["error"], "you_are_not_allowed_to_do_this");
    let (status, admin_network, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Hostnetz",
            "image": "quay.io/example/host:1",
            "tool": "ansible",
            "network": "host"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{admin_network}");
    assert_eq!(admin_network["network"], "host");

    let (status, org_env, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Orga",
            "image": "quay.io/example/orga:1",
            "tool": "shell",
            "scope": "organization"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org_env}");
    assert_eq!(org_env["scope"], "organization");
    assert_eq!(org_env["organization_id"], org_id);
    assert_eq!(org_env["organization_name"], org_name);
    assert!(org_env["team_id"].is_null());
    let org_env_id = org_env["id"].as_str().unwrap().to_string();

    let (status, denied_global, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Nicht global",
            "image": "quay.io/example/nope:1",
            "tool": "ansible",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied_global}");
    assert_eq!(denied_global["error"], "you_are_not_allowed_to_do_this");
    let (status, bad_scope, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Kaputt",
            "image": "quay.io/example/bad:1",
            "tool": "ansible",
            "scope": "world"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_scope}");
    assert_eq!(bad_scope["error"], "scope_is_team_organization_or_global");

    let (status, global_env, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Geteilt",
            "image": "quay.io/example/global:1",
            "tool": "shell",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{global_env}");
    assert_eq!(global_env["scope"], "global");
    assert!(global_env["team_id"].is_null());
    assert!(global_env["organization_id"].is_null());
    let global_id = global_env["id"].as_str().unwrap().to_string();

    let names = |list: &Value| {
        list.as_array()
            .unwrap()
            .iter()
            .map(|item| item["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let (status, home, _) = call(
        &app,
        "GET",
        "/api/environments",
        Some(&ada_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{home}");
    let home_names = names(&home);
    assert!(home_names.contains(&"AWX-EE".to_string()), "{home}");
    assert!(home_names.contains(&"Privat".to_string()));
    assert!(home_names.contains(&"Orga".to_string()));
    assert!(home_names.contains(&"Geteilt".to_string()));
    let rank = |scope: &str| match scope {
        "global" => 0,
        "organization" => 1,
        "team" => 2,
        _ => 9,
    };
    let scopes = home
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["scope"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(scopes.windows(2).all(|pair| rank(pair[0]) <= rank(pair[1])));

    let (status, same_org, _) = call(
        &app,
        "GET",
        "/api/environments",
        Some(&ada_cookie),
        Some(&sibling),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{same_org}");
    let same_names = names(&same_org);
    assert!(same_names.contains(&"Geteilt".to_string()), "{same_org}");
    assert!(same_names.contains(&"Orga".to_string()), "{same_org}");
    assert!(!same_names.iter().any(|name| name == "Privat"));
    assert!(!same_names.iter().any(|name| name == "AWX-EE"));

    let (status, other, _) = call(
        &app,
        "GET",
        "/api/environments",
        Some(&cookie),
        Some(&foreign),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{other}");
    let other_names = names(&other);
    assert!(other_names.contains(&"Geteilt".to_string()), "{other}");
    assert!(!other_names.iter().any(|name| name == "Orga"));
    assert!(!other_names.iter().any(|name| name == "Privat"));
    assert!(!other_names.iter().any(|name| name == "AWX-EE"));

    let (status, seen, _) = call(
        &app,
        "GET",
        &format!("/api/environments/{global_id}"),
        Some(&ada_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{seen}");
    assert_eq!(seen["scope"], "global");
    let (status, forbidden, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{global_id}"),
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Geteilt",
            "image": "quay.io/example/global:1",
            "tool": "shell",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{forbidden}");
    assert_eq!(forbidden["error"], "you_are_not_allowed_to_do_this");
    let (status, _, _) = call(
        &app,
        "DELETE",
        &format!("/api/environments/{global_id}"),
        Some(&ada_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, hidden, _) = call(
        &app,
        "GET",
        &format!("/api/environments/{private_id}"),
        Some(&cookie),
        Some(&foreign),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");

    let (status, kept, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{org_env_id}"),
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Orga",
            "image": "quay.io/example/orga:2",
            "tool": "shell",
            "description": "bleibt"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["scope"], "organization");
    assert_eq!(kept["image"], "quay.io/example/orga:2");
    let (status, promote, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{org_env_id}"),
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Orga",
            "image": "quay.io/example/orga:2",
            "tool": "shell",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{promote}");
    let (status, still_org, _) = call(
        &app,
        "GET",
        &format!("/api/environments/{org_env_id}"),
        Some(&ada_cookie),
        Some(&sibling),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{still_org}");
    assert_eq!(still_org["scope"], "organization");
    let (status, sibling_edit, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{org_env_id}"),
        Some(&ada_cookie),
        Some(&sibling),
        Some(json!({
            "name": "Orga",
            "image": "quay.io/example/orga:3",
            "tool": "shell",
            "description": "geschwister"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sibling_edit}");
    assert_eq!(sibling_edit["scope"], "organization");
    assert_eq!(sibling_edit["organization_id"], org_id);

    let shell = |name: &str, environment_id: &str| {
        json!({
            "name": name,
            "tool": "shell",
            "entrypoint": "",
            "environment_id": environment_id,
            "spec": {"script": "true"}
        })
    };
    let (status, used, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&foreign),
        Some(shell("Mit Global", &global_id)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{used}");
    assert_eq!(used["environment_id"], global_id);
    let (status, blocked, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&foreign),
        Some(shell("Mit Orga", &org_env_id)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{blocked}");
    assert_eq!(
        blocked["error"],
        "the_execution_environment_is_not_visible_to_this"
    );
    let (status, allowed, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&ada_cookie),
        Some(&sibling),
        Some(shell("Mit Orga erlaubt", &org_env_id)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{allowed}");
    assert_eq!(allowed["environment_id"], org_env_id);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_privileged_environment_network_is_refused_for_a_non_admin_launch() {
    let dir = temp_dir("env-network-admin");
    let mut cfg = config(&dir, &Options::default());
    cfg.local_jobs = LocalJobs::Admins;
    cfg.runner = RunnerMode::Docker;
    let state = koreograf::boot(cfg).unwrap();
    let app = koreograf::router(state);

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, _user, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "owner",
            "display_name": "Owner",
            "password": "owner-pass",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, joined, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(&team),
        Some(json!({"username": "owner", "role": "owner"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{joined}");
    let (status, owner_me, owner_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "owner", "password": "owner-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{owner_me}");
    let owner_cookie = owner_cookie.expect("owner cookie");

    let (status, environment, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Hostnetz",
            "image": "quay.io/example/host:1",
            "tool": "shell",
            "network": "host"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{environment}");
    let environment_id = environment["id"].as_str().unwrap();
    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Mit Hostnetz",
            "tool": "shell",
            "entrypoint": "",
            "environment_id": environment_id,
            "spec": {"script": "true"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();

    let job = launch_and_wait(&app, &owner_cookie, &team, template_id).await;
    assert_eq!(job["status"], "failed", "{}", job["events"]);
    assert!(
        has_line(
            &job,
            "system",
            "The execution environment network needs an administrator."
        ),
        "{}",
        job["events"]
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn cross_origin_api_writes_are_refused() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    struct Call<'a> {
        method: &'a str,
        host: &'a str,
        origin: Option<&'a str>,
        referer: Option<&'a str>,
        cookie: &'a str,
        team: &'a str,
        body: &'a str,
    }

    async fn send(app: axum::Router, call: Call<'_>) -> StatusCode {
        let mut builder = Request::builder()
            .method(call.method)
            .uri("/api/inventories")
            .header("cookie", call.cookie)
            .header("x-team-id", call.team)
            .header("content-type", "application/json")
            .header("host", call.host);
        if let Some(origin) = call.origin {
            builder = builder.header("origin", origin);
        }
        if let Some(referer) = call.referer {
            builder = builder.header("referer", referer);
        }
        app.oneshot(builder.body(Body::from(call.body.to_string())).unwrap())
            .await
            .unwrap()
            .status()
    }

    async fn boot(
        name: &str,
        public_url: Vec<String>,
        nanos: u128,
    ) -> (axum::Router, String, String) {
        let dir = std::env::temp_dir().join(format!("koreograf-origin-{name}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = config(&dir, &Options::default());
        cfg.public_url = public_url;
        let state = koreograf::boot(cfg).unwrap();
        let app = koreograf::router(state);
        let (status, me, cookie) = call(
            &app,
            "POST",
            "/api/login",
            None,
            None,
            Some(json!({"username": "admin", "password": "test-admin-pass"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{me}");
        let cookie = cookie.expect("session cookie");
        let team = me["teams"][0]["id"].as_str().unwrap().to_string();
        (app, cookie, team)
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let body = json!({"name": "Origin", "body": "[web]\nhost-1\n"}).to_string();

    let (app, cookie, team) = boot(
        "public",
        vec!["https://koreograf.example.com".to_string()],
        nanos,
    )
    .await;
    let status = send(
        app,
        Call {
            method: "POST",
            host: "127.0.0.1:8080",
            origin: Some("https://koreograf.example.com"),
            referer: None,
            cookie: &cookie,
            team: &team,
            body: &body,
        },
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "configured public url");

    let (app, cookie, team) = boot("default", Vec::new(), nanos).await;
    let status = send(
        app.clone(),
        Call {
            method: "POST",
            host: "koreograf.test:8080",
            origin: Some("https://other.example"),
            referer: None,
            cookie: &cookie,
            team: &team,
            body: &body,
        },
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "foreign origin");

    let status = send(
        app.clone(),
        Call {
            method: "POST",
            host: "koreograf.test:8080",
            origin: Some("http://koreograf.test:9999"),
            referer: None,
            cookie: &cookie,
            team: &team,
            body: &body,
        },
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "other port");

    let status = send(
        app.clone(),
        Call {
            method: "POST",
            host: "koreograf.test",
            origin: Some("http://koreograf.test:9999"),
            referer: None,
            cookie: &cookie,
            team: &team,
            body: &body,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "other port without a host port"
    );

    let status = send(
        app.clone(),
        Call {
            method: "POST",
            host: "koreograf.example.com",
            origin: Some("https://koreograf.example.com"),
            referer: None,
            cookie: &cookie,
            team: &team,
            body: &body,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "host fallback assumes http until the public url or trusted proxy is set"
    );

    let status = send(
        app.clone(),
        Call {
            method: "POST",
            host: "koreograf.test:8080",
            origin: Some("null"),
            referer: None,
            cookie: &cookie,
            team: &team,
            body: &body,
        },
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "opaque origin");

    let status = send(
        app.clone(),
        Call {
            method: "POST",
            host: "koreograf.test:8080",
            origin: None,
            referer: Some("https://other.example/page"),
            cookie: &cookie,
            team: &team,
            body: &body,
        },
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "foreign referer");

    let status = send(
        app.clone(),
        Call {
            method: "POST",
            host: "[::1]:8080",
            origin: Some("http://[::1]:8080"),
            referer: None,
            cookie: &cookie,
            team: &team,
            body: &body,
        },
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "ipv6 same origin");

    let body = json!({"name": "Origin two", "body": "[web]\nhost-2\n"}).to_string();
    let status = send(
        app.clone(),
        Call {
            method: "POST",
            host: "koreograf.test:8080",
            origin: None,
            referer: None,
            cookie: &cookie,
            team: &team,
            body: &body,
        },
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "no origin allows scripts");

    let status = send(
        app,
        Call {
            method: "DELETE",
            host: "koreograf.test:8080",
            origin: Some("https://other.example"),
            referer: None,
            cookie: &cookie,
            team: &team,
            body: "",
        },
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "delete is guarded");
}

#[tokio::test]
async fn a_trusted_proxy_uses_the_forwarded_proto() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    let dir = temp_dir("proxy-origin");
    let mut cfg = config(&dir, &Options::default());
    cfg.trusted_proxy = true;
    let app = koreograf::router(koreograf::boot(cfg).unwrap());
    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();
    let body = json!({"name": "Proxy", "body": "[web]\nhost-1\n"}).to_string();

    let request = Request::builder()
        .method("POST")
        .uri("/api/inventories")
        .header("cookie", &cookie)
        .header("x-team-id", &team)
        .header("content-type", "application/json")
        .header("host", "127.0.0.1:8080")
        .header("x-forwarded-host", "koreograf.example.com")
        .header("x-forwarded-proto", "https")
        .header("origin", "https://koreograf.example.com")
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn settings_responses_have_the_documented_shape() {
    let harness = TestApp::boot("settings-shape", Options::default());
    let app = harness.app.clone();

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/v1/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");

    let body = json!({
        "ldap": {
            "url": "ldap://ldap.example.com",
            "mode": "ad",
            "bind_dn": "cn=svc,dc=example,dc=com",
            "bind_password": "bind-secret-value",
            "user_base": "ou=users,dc=example,dc=com",
            "user_filter": "(sAMAccountName={username})",
            "username_attr": "sAMAccountName",
            "group_attr": "memberOf",
            "group_base": "ou=groups,dc=example,dc=com",
            "starttls": true,
            "tls_insecure": false
        },
        "proxy": {
            "url": "http://proxy.example:3128",
            "username": "proxy-user",
            "password": "proxy-secret-value",
            "no_proxy": "localhost,127.0.0.1"
        },
        "runner": "docker",
        "job_timeout_secs": 90,
        "job_output_lines": 4000,
        "default_environment_ansible": "",
        "default_environment_opentofu": "",
        "default_environment_shell": "",
        "llm": {
            "enabled": true,
            "provider": "ollama",
            "base_url": "http://127.0.0.1:11434/v1",
            "model": "llama3",
            "api_key": "llm-secret-value"
        }
    });
    let expected = json!({
        "source": "saved",
        "ldap": {
            "url": "ldap://ldap.example.com",
            "mode": "ad",
            "bind_dn": "cn=svc,dc=example,dc=com",
            "bind_password_set": true,
            "user_base": "ou=users,dc=example,dc=com",
            "user_filter": "(sAMAccountName={username})",
            "username_attr": "sAMAccountName",
            "group_attr": "memberOf",
            "group_base": "ou=groups,dc=example,dc=com",
            "starttls": true,
            "tls_insecure": false
        },
        "proxy": {
            "url": "http://proxy.example:3128",
            "username": "proxy-user",
            "password_set": true,
            "no_proxy": "localhost,127.0.0.1",
            "unsupported_env": false
        },
        "runner": "docker",
        "job_timeout_secs": 90,
        "job_output_lines": 4000,
        "default_environment_ansible": "",
        "default_environment_opentofu": "",
        "default_environment_shell": "",
        "llm": {
            "enabled": true,
            "provider": "ollama",
            "base_url": "http://127.0.0.1:11434/v1",
            "model": "llama3",
            "api_key_set": true
        }
    });

    let (status, saved, _) = call(
        &app,
        "PUT",
        "/api/v1/settings",
        Some(&cookie),
        None,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved, expected);
    for secret in [
        "bind-secret-value",
        "proxy-secret-value",
        "llm-secret-value",
    ] {
        assert!(!saved.to_string().contains(secret), "{saved}");
    }

    let (status, current, _) =
        call(&app, "GET", "/api/v1/settings", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{current}");
    assert_eq!(current, expected);
}

#[tokio::test]
async fn admin_settings_hide_secrets_and_apply_without_restart() {
    let harness = TestApp::boot("settings-smoke", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/features/legacy.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
    assert!(script.contains("#/settings"));
    assert!(script.contains("settings"));
    assert!(script.contains("bind_password"));
    assert!(script.contains("proxy_address"));
    assert!(script.contains("use_environment_variables"));
    assert!(script.contains("data-stored-mask=\"1\""));
    assert!(!script.contains("private_ca_file"));
    assert!(!script.contains("ldap_ca_file"));
    assert!(!script.contains("name=ca_file"));
    assert!(!script.contains("an_absolute_pem_path"));
    assert!(!script.contains("bind-secret-value"));
    assert!(!script.contains("proxy-secret-value"));

    let (status, denied, _) = call(&app, "GET", "/api/settings", None, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{denied}");
    assert_eq!(denied["error"], "sign_in_is_required");

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "ada",
            "display_name": "Ada Lovelace",
            "password": "ada-secret-1",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let (status, ada_me, ada_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "ada", "password": "ada-secret-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ada_me}");
    let ada_cookie = ada_cookie.expect("ada session");
    let (status, forbidden, _) =
        call(&app, "GET", "/api/settings", Some(&ada_cookie), None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{forbidden}");
    assert_eq!(forbidden["error"], "you_are_not_allowed_to_do_this");

    let (status, current, _) = call(&app, "GET", "/api/settings", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{current}");
    assert_eq!(current["source"], "env");
    {
        let round_trip = current.clone();
        let (status, echoed, _) = call(
            &app,
            "PUT",
            "/api/settings",
            Some(&cookie),
            None,
            Some(round_trip),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "read-only fields rejected: {echoed}"
        );
    }
    assert_eq!(current["runner"], "local");
    assert_eq!(current["ldap"]["url"], "");
    assert!(current["ldap"].get("ca_file").is_none());
    assert_eq!(current["ldap"]["bind_password_set"], false);
    assert_eq!(current["proxy"]["password_set"], false);
    assert!(!current.to_string().contains("password\":"));

    let (status, blocked, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&ada_cookie),
        None,
        Some(json!({"reset": true})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{blocked}");

    let body = json!({
        "ldap": {
            "url": "ldap://ldap.example.com",
            "mode": "ad",
            "bind_dn": "cn=svc,dc=example,dc=com",
            "bind_password": "bind-secret-value",
            "user_base": "ou=users,dc=example,dc=com",
            "starttls": true,
            "tls_insecure": false
        },
        "proxy": {
            "url": "http://proxy.example:3128",
            "username": "proxy-user",
            "password": "proxy-secret-value",
            "no_proxy": "localhost,127.0.0.1"
        },
        "runner": "docker",
        "job_timeout_secs": 90,
        "job_output_lines": 4000
    });
    let (status, saved, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let saved_text = saved.to_string();
    assert!(!saved_text.contains("bind-secret-value"), "{saved_text}");
    assert!(!saved_text.contains("proxy-secret-value"), "{saved_text}");
    assert_eq!(saved["source"], "saved");
    assert_eq!(saved["ldap"]["mode"], "ad");
    assert!(saved["ldap"].get("ca_file").is_none());
    assert_eq!(saved["ldap"]["user_filter"], "(sAMAccountName={username})");
    assert_eq!(saved["ldap"]["bind_password_set"], true);
    assert_eq!(saved["proxy"]["password_set"], true);
    assert_eq!(saved["proxy"]["url"], "http://proxy.example:3128");
    assert_eq!(saved["runner"], "docker");
    assert_eq!(saved["job_timeout_secs"], 90);
    assert_eq!(saved["job_output_lines"], 4000);
    assert!(saved["ldap"].get("bind_password").is_none());
    assert!(saved["proxy"].get("password").is_none());

    let (status, echoed, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(saved.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{echoed}");
    assert_eq!(echoed["ldap"]["bind_password_set"], true);
    assert_eq!(echoed["proxy"]["password_set"], true);
    assert!(!echoed.to_string().contains("bind-secret-value"));
    assert!(!echoed.to_string().contains("proxy-secret-value"));

    let (status, partial, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(json!({"job_timeout_secs": 60})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{partial}");
    assert_eq!(partial["error"], "the_settings_body_is_incomplete");
    assert_eq!(partial["vars"]["fields"], "ldap, proxy, runner");

    let (status, bad_runner, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(json!({
            "ldap": {
                "url": "",
                "mode": "ldap"
            },
            "proxy": {
                "url": ""
            },
            "runner": "bogus",
            "job_timeout_secs": 60
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_runner}");
    assert_eq!(bad_runner["error"], "the_runner_mode_is_invalid");

    let (status, overview, _) = call(
        &app,
        "GET",
        "/api/overview",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert_eq!(overview["ldap"], true);
    assert_eq!(overview["runner_mode"], "docker");
    assert!(
        overview["runtime"].is_null()
            || overview["runtime"] == "docker"
            || overview["runtime"] == "podman",
        "{overview}"
    );
    assert_same_runner(&app, &cookie, &team, &overview).await;

    let (status, kept, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(json!({
            "ldap": {
                "url": "ldap://ldap.example.com",
                "mode": "ad",
                "bind_dn": "cn=svc,dc=example,dc=com",
                "bind_password": "",
                "user_base": "ou=users,dc=example,dc=com"
            },
            "proxy": {
                "url": "http://proxy.example:3128",
                "username": "proxy-user",
                "password": "",
                "no_proxy": "localhost,127.0.0.1"
            },
            "runner": "docker",
            "job_timeout_secs": 90
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["ldap"]["bind_password_set"], true);
    assert_eq!(kept["proxy"]["password_set"], true);

    let (status, rejected, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(json!({
            "ldap": {
                "url": "ldap://ldap.example.com",
                "mode": "ad",
                "bind_dn": "cn=svc,dc=example,dc=com",
                "user_base": "ou=users,dc=example,dc=com"
            },
            "proxy": {
                "url": "http://user:secret@proxy.example:3128",
                "username": "proxy-user"
            },
            "runner": "local",
            "job_timeout_secs": 45
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(
        rejected["error"],
        "put_the_credentials_in_their_own_fields_not"
    );
    let (status, still, _) = call(&app, "GET", "/api/settings", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{still}");
    assert_eq!(still["runner"], "docker");
    assert_eq!(still["proxy"]["password_set"], true);
    assert!(!still.to_string().contains("proxy-secret-value"));

    let (status, cleared, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(json!({
            "ldap": {
                "url": "",
                "mode": "ldap",
                "bind_dn": "cn=svc,dc=example,dc=com",
                "user_base": "ou=users,dc=example,dc=com"
            },
            "proxy": {
                "url": "http://proxy.example:3128",
                "username": "proxy-user",
                "no_proxy": "localhost"
            },
            "runner": "auto",
            "job_timeout_secs": 30,
            "clear": ["ldap_bind_password", "proxy_password"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["ldap"]["url"], "");
    assert_eq!(cleared["ldap"]["bind_password_set"], false);
    assert_eq!(cleared["proxy"]["password_set"], false);
    assert_eq!(cleared["runner"], "auto");

    let (status, overview, _) = call(
        &app,
        "GET",
        "/api/overview",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert_eq!(overview["ldap"], false);
    assert_eq!(overview["runner_mode"], "auto");
    assert!(
        overview["runtime"].is_null()
            || overview["runtime"] == "docker"
            || overview["runtime"] == "podman",
        "{overview}"
    );
    assert_same_runner(&app, &cookie, &team, &overview).await;

    let (status, reset, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(json!({"reset": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reset}");
    assert_eq!(reset["source"], "env");
    assert_eq!(reset["runner"], "local");
    assert_eq!(reset["job_timeout_secs"], 60);
    assert_eq!(reset["job_output_lines"], 8000);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn json_bodies_are_rejected_by_kind_and_objects_reject_arrays() {
    let harness = TestApp::boot("json-kind", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, templates, _) = call(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{templates}");
    let health_id = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Healthcheck")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let launch = format!("/api/templates/{health_id}/launch");

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Sleeper",
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": "sleep 30\necho should-not-run\n"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let sleeper_id = created["id"].as_str().unwrap();
    let sleeper_launch = format!("/api/templates/{sleeper_id}/launch");

    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Manual",
            "sources": [{"kind": "ini", "name": "Hosts", "body": "web-1\n"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let source_id = inventory["sources"][0]["id"].as_str().unwrap();
    let source = format!(
        "/api/inventories/{}/sources/{source_id}",
        inventory["id"].as_str().unwrap()
    );

    let (status, workflows, _) = call(
        &app,
        "GET",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{workflows}");
    let workflow_id = workflows.as_array().unwrap()[0]["id"].as_str().unwrap();
    let workflow_launch = format!("/api/workflows/{workflow_id}/launch");

    let before = job_count(&app, &cookie, &team).await;
    let invalid = (StatusCode::BAD_REQUEST, "json_body_invalid");
    let wrong_type = json!({"inventory_id": 1}).to_string();
    expect_json_error(
        &app,
        "POST",
        &launch,
        Some(&cookie),
        Some(&team),
        br#"{"limit":"#,
        invalid,
    )
    .await;
    expect_json_error(
        &app,
        "POST",
        &launch,
        Some(&cookie),
        Some(&team),
        b"",
        invalid,
    )
    .await;
    expect_json_error(
        &app,
        "POST",
        &launch,
        Some(&cookie),
        Some(&team),
        wrong_type.as_bytes(),
        invalid,
    )
    .await;
    expect_json_error(
        &app,
        "POST",
        &launch,
        Some(&cookie),
        Some(&team),
        b"[]",
        invalid,
    )
    .await;
    expect_json_error(
        &app,
        "POST",
        &launch,
        Some(&cookie),
        Some(&team),
        b"null",
        invalid,
    )
    .await;
    expect_json_error(&app, "POST", "/api/login", None, None, b"{}", invalid).await;
    expect_json_error(
        &app,
        "POST",
        &workflow_launch,
        Some(&cookie),
        Some(&team),
        b"[]",
        invalid,
    )
    .await;
    expect_json_error(
        &app,
        "PUT",
        &source,
        Some(&cookie),
        Some(&team),
        b"[]",
        invalid,
    )
    .await;
    let (status, patched, _) = call(
        &app,
        "PUT",
        &source,
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(patched["sources"][0]["name"], "Hosts");

    let mut huge = vec![b' '; 2_097_152 + 64];
    huge[..1].copy_from_slice(b"{");
    expect_json_error(
        &app,
        "POST",
        &launch,
        Some(&cookie),
        Some(&team),
        &huge,
        (StatusCode::PAYLOAD_TOO_LARGE, "request_too_large"),
    )
    .await;
    let signed_out = (StatusCode::UNAUTHORIZED, "sign_in_is_required");
    expect_json_error(&app, "POST", &launch, None, Some(&team), &huge, signed_out).await;
    expect_json_error(&app, "POST", &launch, None, Some(&team), b"[]", signed_out).await;
    assert_eq!(job_count(&app, &cookie, &team).await, before);

    let (status, started, _) = call(
        &app,
        "POST",
        &sleeper_launch,
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    let job_id = started["id"].as_str().unwrap();
    assert_eq!(job_count(&app, &cookie, &team).await, before + 1);
    let relaunch = format!("/api/jobs/{job_id}/relaunch");
    expect_json_error(
        &app,
        "POST",
        &relaunch,
        Some(&cookie),
        Some(&team),
        b"[]",
        invalid,
    )
    .await;
    assert_eq!(job_count(&app, &cookie, &team).await, before + 1);

    let (status, canceled, _) = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/cancel"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    assert_eq!(canceled["status"], "canceled");
    let (status, again, _) = call(
        &app,
        "POST",
        &relaunch,
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{again}");
    let again_id = again["id"].as_str().unwrap();
    assert_ne!(again_id, job_id);
    let (status, stopped, _) = call(
        &app,
        "POST",
        &format!("/api/jobs/{again_id}/cancel"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stopped}");
    assert_eq!(stopped["status"], "canceled");

    let (status, hooks, _) =
        call(&app, "GET", "/api/hooks", Some(&cookie), Some(&team), None).await;
    assert_eq!(status, StatusCode::OK, "{hooks}");
    let hook = &hooks.as_array().unwrap()[0];
    let (status, ignored, _) = call(
        &app,
        "POST",
        &format!(
            "/hooks/{}/{}",
            hook["id"].as_str().unwrap(),
            hook["token"].as_str().unwrap()
        ),
        None,
        None,
        Some(json!([])),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ignored}");
    assert_eq!(ignored["status"], "ignored");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn unknown_fields_are_rejected_on_write_endpoints() {
    let dir = temp_dir("unknown");
    let origin = temp_dir("unknown-origin");
    let mut cfg = config(&dir, &Options::default());
    cfg.project_roots.push(origin.clone());
    let state = koreograf::boot(cfg).unwrap();
    let app = koreograf::router(state);

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();
    let organization = me["teams"][0]["organization_id"]
        .as_str()
        .unwrap()
        .to_string();

    let project = json!({
        "name": "Unknown project",
        "source_type": "git",
        "source": format!("file://{}", origin.display())
    });
    let mut rejected = project.clone();
    rejected["credential_name"] = json!("ignored");
    expect_unknown_field(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        &rejected,
        "credential_name",
    )
    .await;
    let (status, project, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(project),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap().to_string();

    let assignment = json!({"organization_id": organization, "team_ids": []});
    let mut rejected = assignment.clone();
    rejected["credential_name"] = json!("ignored");
    expect_unknown_field(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "credential_name",
    )
    .await;
    let (status, assigned, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        Some(assignment),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{assigned}");

    let inventory = json!({"name": "Unknown inventory", "body": "[all]\nbox\n"});
    let mut rejected = inventory.clone();
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(inventory),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap().to_string();

    let update = json!({"name": "Unknown inventory", "body": ""});
    let mut rejected = update.clone();
    rejected["sources"] = json!([]);
    expect_unknown_field(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "sources",
    )
    .await;
    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        Some(update),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");

    let source = json!({"kind": "ini", "name": "Extra source", "body": "web-2\n"});
    let mut rejected = source.clone();
    rejected["group_name_prefix"] = json!(true);
    expect_unknown_field(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "group_name_prefix",
    )
    .await;
    let (status, inventory, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources"),
        Some(&cookie),
        Some(&team),
        Some(source),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{inventory}");
    let source_id = inventory["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Extra source")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let source_update = json!({"body": "web-3\n"});
    let mut rejected = source_update.clone();
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, inventory, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team),
        Some(source_update),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{inventory}");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/dcim/devices/", axum::routing::get(nb_devices));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });
    let netbox_credential = create_credential(
        &app,
        &cookie,
        &team,
        "Unknown NetBox",
        "netbox",
        json!({"token": "secret-token", "url": format!("http://{addr}")}),
    )
    .await;
    let netbox = json!({
        "name": "Unknown NetBox",
        "credential_id": netbox_credential,
        "query_filters": ["status=active"],
        "virtual_machines": false
    });
    let mut rejected = netbox.clone();
    rejected["group_name_prefix"] = json!(true);
    expect_unknown_field(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        &rejected,
        "group_name_prefix",
    )
    .await;
    let (status, netbox, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(netbox),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{netbox}");

    let template = json!({
        "name": "Unknown template",
        "tool": "shell",
        "spec": {"script": "true"}
    });
    let mut rejected = template.clone();
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(template),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    let template_update = json!({
        "name": "Unknown template changed",
        "tool": "shell",
        "spec": {"script": "true"}
    });
    let mut rejected = template_update.clone();
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, changed, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        Some(template_update),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");

    let mut rejected = daily_schedule("Unknown schedule");
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, schedule, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Unknown schedule")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{schedule}");
    let schedule_id = schedule["id"].as_str().unwrap().to_string();

    let mut rejected = daily_schedule("Unknown schedule changed");
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, schedule, _) = call(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Unknown schedule changed")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{schedule}");

    let credential = json!({
        "name": "Unknown credential",
        "kind": "ssh",
        "secret": {"username": "deploy", "password": "secret"}
    });
    let mut rejected = credential.clone();
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(credential),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"].as_str().unwrap().to_string();

    let credential_update = json!({
        "name": "Unknown credential changed",
        "secret": {"username": "deploy", "password": "secret"}
    });
    let mut rejected = credential_update.clone();
    rejected["kind"] = json!("ssh");
    expect_unknown_field(
        &app,
        "PUT",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "kind",
    )
    .await;
    let (status, credential, _) = call(
        &app,
        "PUT",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team),
        Some(credential_update),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{credential}");

    let mut rejected = json!({"reset": true});
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, settings, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(json!({"reset": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{settings}");

    let hook = json!({
        "name": "Unknown hook",
        "match_alertname": "Alert",
        "match_status": "firing",
        "template_id": template_id
    });
    let mut rejected = hook.clone();
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "POST",
        "/api/hooks",
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, hook, _) = call(
        &app,
        "POST",
        "/api/hooks",
        Some(&cookie),
        Some(&team),
        Some(hook),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");

    let (status, workflow, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Unknown workflow",
            "graph": {"nodes": [], "edges": []}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let workflow_id = workflow["id"].as_str().unwrap().to_string();
    let mut rejected = json!({
        "name": "Unknown workflow",
        "graph": {"nodes": [], "edges": []}
    });
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "PUT",
        &format!("/api/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, changed, _) = call(
        &app,
        "PUT",
        &format!("/api/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Unknown workflow changed",
            "graph": {"nodes": [], "edges": []}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&origin);
}

#[test]
fn the_image_starts_the_binary_entrypoint_behind_a_static_init() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dockerfile = std::fs::read_to_string(root.join("Dockerfile")).expect("Dockerfile");
    assert!(!dockerfile.contains("entrypoint.sh"));
    assert!(dockerfile.contains("ARG TINI_VERSION=0.19.0"));
    assert!(dockerfile.contains("ARG TINI_SHA256_AMD64="));
    assert!(dockerfile.contains("ARG TINI_SHA256_ARM64="));
    assert!(dockerfile.contains(
        "https://github.com/krallin/tini/releases/download/v$TINI_VERSION/tini-static-$tini_arch"
    ));
    assert!(dockerfile.contains("install -D -m 755 /tmp/tini-static /out/sbin/tini"));
    assert!(dockerfile.contains("USER 10001:10001"));
    assert!(dockerfile.contains(
        "ENTRYPOINT [\"/sbin/tini\", \"--\", \"/usr/local/bin/koreograf\", \"entrypoint\"]"
    ));
    assert!(dockerfile.contains("CMD [\"koreograf\"]"));
    assert!(dockerfile.contains("CMD [\"/usr/local/bin/koreograf\", \"healthcheck\"]"));
    let docs = std::fs::read_to_string(root.join("docs/installation.md")).expect("installation.md");
    assert!(docs.contains("`/sbin/tini` is PID 1"));
    assert!(docs.contains("`user: \"10001:10001\"`"));
    assert!(docs.contains("koreograf entrypoint"));
    assert!(docs.contains("cargo run"));
}

#[test]
fn the_image_labels_name_the_version_and_the_source() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dockerfile = std::fs::read_to_string(root.join("Dockerfile")).expect("Dockerfile");
    assert!(dockerfile.contains("org.opencontainers.image.version=\"${VERSION}\""));
    assert!(dockerfile.contains("org.opencontainers.image.source=\"${SOURCE}\""));
    assert!(dockerfile.contains("org.opencontainers.image.revision=\"${GIT_SHA}\""));
    assert!(dockerfile.contains("KOREOGRAF_GIT_SHA=${GIT_SHA}"));
    assert!(dockerfile.contains("KOREOGRAF_BUILT_AT=${BUILT_AT}"));
    let version = env!("CARGO_PKG_VERSION");
    assert!(dockerfile.contains("# x-release-please-start-version"));
    assert!(dockerfile.contains(&format!("ARG VERSION={version}\n")));
    assert!(dockerfile.contains("# x-release-please-end"));
    assert!(!dockerfile.contains(&format!("ARG VERSION={version} #")));
    let release =
        std::fs::read_to_string(root.join(".github/workflows/release.yml")).expect("release.yml");
    assert!(release.contains("VERSION=${{ steps.tags.outputs.version }}"));
    assert!(release.contains("SOURCE=https://github.com/koreograf/koreograf"));
    assert!(release.contains("GIT_SHA=${{ steps.build-info.outputs.sha }}"));
    assert!(release.contains("BUILT_AT=${{ steps.build-info.outputs.date }}"));
}

#[test]
fn the_packages_ship_a_systemd_unit_and_a_pinned_nfpm_config() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let nfpm = std::fs::read_to_string(root.join("packaging/nfpm.yaml")).expect("nfpm.yaml");
    assert!(nfpm.contains("version_schema: none"));
    assert!(nfpm.contains("openssh-client"));
    assert!(nfpm.contains("openssh-clients"));
    assert!(nfpm.contains("ca-certificates"));
    assert!(nfpm.contains("type: config|noreplace"));
    assert!(!nfpm.contains("type: ghost"));
    assert!(nfpm.contains("/lib/systemd/system/koreograf.service"));
    assert!(nfpm.contains("/etc/koreograf/koreograf.env"));
    let unit =
        std::fs::read_to_string(root.join("packaging/koreograf.service")).expect("unit file");
    assert!(unit.contains("User=koreograf"));
    assert!(unit.contains("Group=koreograf"));
    assert!(unit.contains("EnvironmentFile=/etc/koreograf/koreograf.env"));
    assert!(unit.contains("ExecStart=/usr/bin/koreograf"));
    assert!(unit.contains("Restart=on-failure"));
    assert!(!unit.contains("entrypoint"));
    let env = std::fs::read_to_string(root.join("packaging/koreograf.env")).expect("env file");
    assert!(env.contains("KOREOGRAF_DATA=/var/lib/koreograf"));
    assert!(env.contains("KOREOGRAF_BIND=127.0.0.1:8080"));
    let release =
        std::fs::read_to_string(root.join(".github/workflows/release.yml")).expect("release.yml");
    assert!(release.contains("NFPM_VERSION: 2.48.0"));
    assert!(release.contains("dist/koreograf_${version}_${PACKAGE_ARCH}.deb"));
    assert!(release.contains("dist/koreograf_${version}_${PACKAGE_ARCH}.rpm"));
    assert!(release.contains("gh release upload \"$TAG\" dist/*.deb dist/*.rpm"));
    assert!(release.contains("gh release view \"$TAG\" >/dev/null"));
    let ci = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("ci.yml");
    assert!(ci.contains("--packager deb"));
    assert!(!ci.contains("--packager rpm"));
    let docs = std::fs::read_to_string(root.join("docs/installation.md")).expect("installation.md");
    assert!(docs.contains("sudo apt install ./koreograf_<version>_amd64.deb"));
    assert!(docs.contains("sudo dnf install ./koreograf_<version>_amd64.rpm"));
}

#[tokio::test]
async fn version_is_available_without_signing_in() {
    let harness = TestApp::boot("version", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();
    let (status, body, _) = call(&app, "GET", "/api/version", None, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn runner_preflight_is_admin_only_and_leaks_no_secrets() {
    let harness = TestApp::boot("preflight-smoke", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, denied, _) = call(&app, "GET", "/api/runner/preflight", None, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{denied}");

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("admin cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, report, _) = call(
        &app,
        "GET",
        "/api/runner/preflight",
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["mode"], "local");
    assert!(report["tools"]
        .as_array()
        .is_some_and(|tools| tools.len() == 5));
    assert!(report["runtime"].is_object());
    assert!(report["missing"].is_array());
    let body = report.to_string();
    assert!(!body.contains("KOREOGRAF_"), "{body}");
    assert!(!body.contains("\"PATH\""), "{body}");
    if let Ok(path) = std::env::var("PATH") {
        assert!(!body.contains(&path), "{body}");
    }

    let (status, overview, _) = call(
        &app,
        "GET",
        "/api/overview",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert!(overview["preflight"].is_object(), "{overview}");
    assert_eq!(overview["preflight"]["ok"], report["ok"]);

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "preflight-viewer",
            "display_name": "Preflight Viewer",
            "password": "viewer-pass",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let (status, member, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(&team),
        Some(json!({"username": "preflight-viewer", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{member}");
    let (status, viewer_me, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "preflight-viewer", "password": "viewer-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer_me}");
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    let (status, forbidden, _) = call(
        &app,
        "GET",
        "/api/runner/preflight",
        Some(&viewer_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{forbidden}");
    assert_eq!(forbidden["error"], "you_are_not_allowed_to_do_this");
    let (status, viewer_overview, _) = call(
        &app,
        "GET",
        "/api/overview",
        Some(&viewer_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer_overview}");
    assert!(viewer_overview["preflight"].is_null(), "{viewer_overview}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn dashboard_series_comes_from_stored_jobs_and_is_team_scoped() {
    let harness = TestApp::boot("dashboard", Options::default());
    let state = harness.state.clone();
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("admin cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, other, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        None,
        Some(json!({"name": "Dashboard Other", "description": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_team = other["id"].as_str().unwrap().to_string();

    let (status, empty_team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        None,
        Some(json!({"name": "Dashboard Empty", "description": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{empty_team}");
    let empty_team = empty_team["id"].as_str().unwrap().to_string();

    let now = chrono::Utc::now();
    let stamp = |ago: chrono::Duration| (now - ago).format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let today = stamp(chrono::Duration::zero());
    let yesterday = stamp(chrono::Duration::days(1) + chrono::Duration::minutes(1));
    let older = stamp(chrono::Duration::days(3) + chrono::Duration::minutes(1));
    {
        let db = state.db().expect("database");
        let add = |id: &str,
                   team_id: &str,
                   status: &str,
                   started: Option<&str>,
                   finished: Option<&str>| {
            db.execute(
                "INSERT INTO jobs (
                    id, team_id, name, tool, status, extra_vars, launched_by, trigger,
                    started_at, finished_at, created_at
                 ) VALUES (?1, ?2, ?1, 'shell', ?3, '{}', 'admin', 'manual', ?4, ?5, ?6)",
                (id, team_id, status, started, finished, today.as_str()),
            )
            .unwrap();
        };
        add(
            "today-successful",
            &team,
            "successful",
            Some(&today),
            Some(&today),
        );
        add("today-failed", &team, "failed", Some(&today), Some(&today));
        add(
            "today-canceled",
            &team,
            "canceled",
            Some(&today),
            Some(&today),
        );
        add("today-running", &team, "running", Some(&today), None);
        add(
            "yesterday-successful",
            &team,
            "successful",
            Some(&yesterday),
            Some(&yesterday),
        );
        add(
            "older-successful",
            &team,
            "successful",
            Some(&older),
            Some(&older),
        );
        add(
            "other-team-successful",
            &other_team,
            "successful",
            Some(&today),
            Some(&today),
        );
        add("pending-without-start", &team, "pending", None, None);
    }

    let cache_root = dir.join("cache").join(&team);
    std::fs::create_dir_all(cache_root.join("collections/ansible_collections/acme/tools")).unwrap();
    std::fs::create_dir_all(cache_root.join("roles/common")).unwrap();
    std::fs::create_dir_all(cache_root.join("tofu")).unwrap();
    std::fs::write(cache_root.join("tofu/provider.bin"), [0_u8; 123]).unwrap();

    let total = |series: &Value, key: &str| {
        series
            .as_array()
            .unwrap()
            .iter()
            .map(|day| day[key].as_u64().unwrap())
            .sum::<u64>()
    };

    let (status, day, _) = call(
        &app,
        "GET",
        "/api/dashboard?days=1&tz=UTC",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{day}");
    assert_eq!(day["range_days"], 1);
    assert_eq!(day["series"].as_array().unwrap().len(), 1);
    assert_eq!(day["series"][0]["date"], day["to"].as_str().unwrap()[..10]);
    assert_eq!(day["series"][0]["successful"], 1);
    assert_eq!(day["series"][0]["failed"], 1);
    assert_eq!(day["series"][0]["canceled"], 1);
    assert_eq!(day["series"][0]["started"], 4);
    assert_eq!(day["cache"]["collections"], 1);
    assert_eq!(day["cache"]["roles"], 1);
    assert_eq!(day["cache"]["tofu_bytes"], 123);

    let (status, week, _) = call(
        &app,
        "GET",
        "/api/dashboard?days=7&tz=UTC",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{week}");
    assert_eq!(week["range_days"], 7);
    assert_eq!(week["series"].as_array().unwrap().len(), 7);
    assert_eq!(total(&week["series"], "successful"), 3);
    assert_eq!(total(&week["series"], "failed"), 1);
    assert_eq!(total(&week["series"], "canceled"), 1);
    assert_eq!(total(&week["series"], "started"), 6);

    let (status, capped, _) = call(
        &app,
        "GET",
        "/api/dashboard?days=30&tz=UTC",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{capped}");
    assert_eq!(capped["range_days"], 14);
    assert_eq!(capped["series"].as_array().unwrap().len(), 14);
    assert_eq!(total(&capped["series"], "successful"), 3);

    let (status, defaulted, _) = call(
        &app,
        "GET",
        "/api/dashboard?tz=UTC",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{defaulted}");
    assert_eq!(defaulted["range_days"], 1);
    assert_eq!(defaulted["series"].as_array().unwrap().len(), 1);

    let (status, isolated, _) = call(
        &app,
        "GET",
        "/api/dashboard?days=7&tz=UTC",
        Some(&cookie),
        Some(&other_team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{isolated}");
    assert_eq!(total(&isolated["series"], "successful"), 1);
    assert_eq!(total(&isolated["series"], "started"), 1);
    assert_eq!(isolated["cache"]["tofu_bytes"], 0);

    let (status, empty, _) = call(
        &app,
        "GET",
        "/api/dashboard?days=14&tz=UTC",
        Some(&cookie),
        Some(&empty_team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert_eq!(empty["range_days"], 14);
    assert_eq!(total(&empty["series"], "started"), 0);
    assert_eq!(total(&empty["series"], "successful"), 0);
    assert_eq!(empty["cache"]["collections"], 0);

    let (status, _viewer, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "dashboard-viewer",
            "display_name": "Dashboard Viewer",
            "password": "viewer-pass",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _member, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(&team),
        Some(json!({"username": "dashboard-viewer", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, viewer_me, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "dashboard-viewer", "password": "viewer-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer_me}");
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    let (status, viewer, _) = call(
        &app,
        "GET",
        "/api/dashboard?days=7&tz=UTC",
        Some(&viewer_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer}");
    assert_eq!(total(&viewer["series"], "successful"), 3);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn mcp_tools_call_the_http_api() {
    let harness = TestApp::boot("mcp", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();
    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, projects, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{projects}");
    let project_id = projects
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Demo")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, templates, _) = call(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{templates}");
    let template_id = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Healthcheck")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        None,
        Some(json!({
            "username": "mcp-viewer",
            "display_name": "MCP viewer",
            "password": "viewer-pass",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(&team),
        Some(json!({"username": "mcp-viewer", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, viewer_me, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "mcp-viewer", "password": "viewer-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer_me}");
    let viewer_cookie = viewer_cookie.expect("viewer cookie");

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        None,
        Some(json!({
            "username": "mcp-operator",
            "display_name": "MCP operator",
            "password": "operator-pass",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(&team),
        Some(json!({"username": "mcp-operator", "role": "operator"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, operator_me, operator_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "mcp-operator", "password": "operator-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{operator_me}");
    let operator_cookie = operator_cookie.expect("operator cookie");

    let (status, admin_token, _) = call(
        &app,
        "POST",
        "/api/me/tokens",
        Some(&cookie),
        None,
        Some(json!({"name": "mcp-admin"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let admin_secret = admin_token["secret"].as_str().unwrap().to_string();
    let (status, viewer_token, _) = call(
        &app,
        "POST",
        "/api/me/tokens",
        Some(&viewer_cookie),
        None,
        Some(json!({"name": "mcp-viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let viewer_secret = viewer_token["secret"].as_str().unwrap().to_string();
    let (status, operator_token, _) = call(
        &app,
        "POST",
        "/api/me/tokens",
        Some(&operator_cookie),
        None,
        Some(json!({"name": "mcp-operator"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let operator_secret = operator_token["secret"].as_str().unwrap().to_string();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let served = app.clone();
    let server = tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });
    let url = format!("http://{address}");

    let client =
        koreograf::mcp::Client::new(url.clone(), admin_secret, Some(team.clone())).unwrap();
    assert_eq!(client.whoami(None).await.unwrap()["username"], "admin");
    assert_eq!(
        client.version(None).await.unwrap()["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert!(client
        .list_projects(None)
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "Demo"));
    assert!(client.list_inventories(None).await.unwrap().is_array());
    assert!(client.list_credentials(None).await.unwrap().is_array());
    assert!(client
        .list_templates(None)
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "Healthcheck"));
    assert!(client.list_workflows(None).await.unwrap().is_array());
    assert!(client.list_schedules(None).await.unwrap().is_array());
    let jobs = client
        .list_jobs(koreograf::mcp::JobQuery {
            limit: Some(5),
            status: Some("successful"),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(jobs["items"].is_array());

    let launched = client
        .launch_template(&template_id, serde_json::Map::new(), None)
        .await
        .unwrap();
    let job_id = launched["id"].as_str().unwrap().to_string();
    let mut final_job = Value::Null;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let job = client.get_job(&job_id, None).await.unwrap();
        if matches!(
            job["status"].as_str(),
            Some("successful" | "failed" | "canceled")
        ) {
            final_job = job;
            break;
        }
    }
    assert_eq!(final_job["status"], "successful", "job: {final_job}");
    let output = client.job_output(&job_id, None).await.unwrap();
    assert!(output.contains("healthcheck ok"), "{output}");
    let synced = client.sync_project(&project_id, None).await.unwrap();
    assert!(synced["path"].is_string(), "{synced}");

    let viewer =
        koreograf::mcp::Client::new(url, viewer_secret.clone(), Some(team.clone())).unwrap();
    let denied = viewer
        .launch_template(&template_id, serde_json::Map::new(), None)
        .await
        .unwrap_err();
    let (route_status, route_body) = call_bearer(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        &viewer_secret,
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(denied.status, Some(route_status.as_u16()));
    assert_eq!(denied.key, route_body["error"].as_str().unwrap());

    let session = mcp_session(&app, &operator_secret).await;
    let (status, bytes, _) = mcp_request(
        &app,
        Some(&operator_secret),
        Some(&session),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "launch_template",
                "arguments": {
                    "id": template_id,
                    "extra_vars": {"from_mcp_http": "yes"},
                    "team_id": team
                }
            }
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let message = sse_message(&bytes);
    assert_eq!(message["result"]["isError"], true, "{message}");
    assert!(
        message["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("prompt_on_launch_is_off"),
        "{message}"
    );

    let viewer_session = mcp_session(&app, &viewer_secret).await;
    let (status, bytes, _) = mcp_request(
        &app,
        Some(&viewer_secret),
        Some(&viewer_session),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "launch_template",
                "arguments": {"id": template_id, "extra_vars": {}, "team_id": team}
            }
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert!(
        sse_message(&bytes)
            .to_string()
            .contains("you_are_not_allowed_to_do_this"),
        "{}",
        String::from_utf8_lossy(&bytes)
    );

    let (status, bytes, _) = mcp_request(&app, None, None, mcp_initialize_body()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let refused: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(refused["error"], "sign_in_is_required");

    let (status, bytes, _) =
        mcp_request(&app, Some("koro_unknown"), None, mcp_initialize_body()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let refused: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(refused["error"], "sign_in_is_required");

    server.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn mcp_http_routes_can_be_turned_off() {
    let dir = temp_dir("mcp-off");
    let mut cfg = config(&dir, &Options::default());
    cfg.mcp = false;
    let state = koreograf::boot(cfg).unwrap();
    let app = koreograf::router(state);

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let (status, me, _) = call(&app, "GET", "/api/me", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");

    let (status, _, _) = mcp_request(&app, None, None, mcp_initialize_body()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let _ = std::fs::remove_dir_all(&dir);
}
