use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::config::Config;
use crate::db;
use crate::features::activity::service;
use crate::features::jobs;
use crate::features::schedules::service::start as start_schedule;
use crate::Shared;

use super::{Activity, ActivityQuery};

fn config(dir: &Path) -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        data_dir: dir.to_path_buf(),
        web_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
        examples_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
        admin_user: "admin".into(),
        admin_password: Some("test-admin-pass".into()),
        runner: crate::config::RunnerMode::Local,
        container_image: false,
        job_timeout_secs: 60,
        job_output_lines: 8_000,
        job_log_retention_days: 30,
        max_concurrent_jobs: 4,
        max_concurrent_jobs_per_team: 4,
        local_jobs: crate::config::LocalJobs::All,
        project_roots: Vec::new(),
        hsts: false,
        cookie_secure: crate::config::CookieSecure::Auto,
        trusted_proxy: false,
        mcp: true,
        public_url: Vec::new(),
        data_host: None,
        ldap: None,
    }
}

fn boot(label: &str) -> (PathBuf, Shared, Router) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-activity-{label}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("dir");
    let state = crate::boot(config(&dir)).expect("boot");
    let app = crate::router(state.clone());
    (dir, state, app)
}

fn session(state: &Shared, token: &str, username: &str) -> (String, String) {
    let user = state
        .store()
        .user_by_username(username)
        .expect("user lookup")
        .expect("user");
    state
        .store()
        .create_session(token, &user.id)
        .expect("session");
    let teams = state
        .store()
        .list_teams_for_user(&user.id, user.is_admin)
        .expect("teams");
    (user.id, teams[0].0.id.clone())
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    token: &str,
    team: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, format!("koreograf_session={token}"));
    if let Some(team) = team {
        request = request.header("x-team-id", team);
    }
    let body = match body {
        Some(body) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("body");
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

fn item<'a>(items: &'a [Value], action: &str, object_type: &str) -> &'a Value {
    items
        .iter()
        .find(|item| item["action"] == action && item["object_type"] == object_type)
        .unwrap_or_else(|| panic!("missing {action} {object_type}: {items:?}"))
}

#[tokio::test]
async fn sync_template_write_and_launch_are_recorded() {
    let (dir, state, app) = boot("api");
    let (_, team) = session(&state, "admin-session", "admin");

    let (status, projects) = call(
        &app,
        "GET",
        "/api/v1/projects",
        "admin-session",
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{projects}");
    let project = &projects[0];
    let project_id = project["id"].as_str().expect("project id");
    let project_name = project["name"].as_str().expect("project name");

    let (status, synced) = call(
        &app,
        "POST",
        &format!("/api/v1/projects/{project_id}/sync"),
        "admin-session",
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");

    let (status, template) = call(
        &app,
        "POST",
        "/api/v1/templates",
        "admin-session",
        Some(&team),
        Some(json!({
            "name": "Activity probe",
            "tool": "shell",
            "spec": {"script": "#!/bin/sh\ntrue\n"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().expect("template id");

    let (status, launched) = call(
        &app,
        "POST",
        &format!("/api/v1/templates/{template_id}/launch"),
        "admin-session",
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");

    let (status, log) = call(
        &app,
        "GET",
        "/api/v1/activity?limit=100",
        "admin-session",
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{log}");
    let items = log["items"].as_array().expect("items");

    let sync = item(items, "sync", "project");
    assert_eq!(sync["actor"], "admin");
    assert_eq!(sync["object_id"], project_id);
    assert_eq!(sync["object_name"], project_name);
    assert_eq!(sync["result"], "success");
    chrono::DateTime::parse_from_rfc3339(sync["created_at"].as_str().expect("created"))
        .expect("time");

    let write = item(items, "create", "template");
    assert_eq!(write["actor"], "admin");
    assert_eq!(write["object_id"], template_id);
    assert_eq!(write["object_name"], "Activity probe");
    assert_eq!(write["result"], "success");

    let launch = item(items, "launch", "job");
    assert_eq!(launch["actor"], "admin");
    assert_eq!(launch["object_name"], "Activity probe");
    assert_eq!(launch["result"], "success");

    let (status, filtered) = call(
        &app,
        "GET",
        "/api/v1/activity?actor=admin&action=create&object_type=template&limit=100",
        "admin-session",
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{filtered}");
    assert_eq!(filtered["total"], 1, "{filtered}");

    let (status, future) = call(
        &app,
        "GET",
        "/api/v1/activity?since=2999-01-01T00:00:00Z",
        "admin-session",
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{future}");
    assert_eq!(future["total"], 0, "{future}");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn activity_is_newest_first_and_prunes_to_the_bound() {
    let (dir, state, _app) = boot("prune");
    let (_, team) = session(&state, "admin-session", "admin");
    for index in 1..=5 {
        state
            .store()
            .insert_activity(&Activity {
                id: format!("entry-{index}"),
                team_id: team.clone(),
                actor: "admin".into(),
                action: "create".into(),
                object_type: "project".into(),
                object_id: format!("project-{index}"),
                object_name: format!("Project {index}"),
                result: "success".into(),
                created_at: format!("2026-01-01T00:00:0{index}Z"),
            })
            .expect("insert");
    }
    let query = ActivityQuery {
        limit: 100,
        ..ActivityQuery::default()
    };
    let (items, total) = state.store().list_activity(&team, &query).expect("list");
    assert_eq!(total, 5);
    let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, ["entry-5", "entry-4", "entry-3", "entry-2", "entry-1"]);

    state.store().prune_activity(&team, 3).expect("prune");
    let (items, total) = state.store().list_activity(&team, &query).expect("list");
    assert_eq!(total, 3);
    let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, ["entry-5", "entry-4", "entry-3"]);
    assert_eq!(service::RETENTION, 10_000);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_viewer_reads_the_team_log_but_not_another_team() {
    let (dir, state, app) = boot("scope");
    let (_, team) = session(&state, "admin-session", "admin");
    let organization_id = state
        .store()
        .team_by_id(&team)
        .expect("team")
        .expect("team row")
        .organization_id;
    state
        .store()
        .insert_team("team-two", &organization_id, "Second team", "", None)
        .expect("team two");
    state
        .store()
        .insert_user("viewer-id", "viewer", "Viewer", None, "local", false)
        .expect("viewer");
    state
        .store()
        .grant_manual_membership(&team, "viewer-id", "viewer")
        .expect("membership");
    state
        .store()
        .create_session("viewer-session", "viewer-id")
        .expect("session");

    service::record(
        state.store(),
        &team,
        "viewer",
        "create",
        "project",
        "p1",
        "First team",
        "success",
    )
    .expect("first team entry");
    service::record(
        state.store(),
        "team-two",
        "admin",
        "create",
        "project",
        "p2",
        "Second team",
        "success",
    )
    .expect("second team entry");

    let (status, own) = call(
        &app,
        "GET",
        "/api/v1/activity",
        "viewer-session",
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{own}");
    assert_eq!(own["total"], 1, "{own}");
    assert_eq!(own["items"][0]["team_id"], team);

    let (status, foreign) = call(
        &app,
        "GET",
        "/api/v1/activity",
        "viewer-session",
        Some("team-two"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{foreign}");

    let (status, admin_view) = call(
        &app,
        "GET",
        "/api/v1/activity",
        "admin-session",
        Some("team-two"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{admin_view}");
    assert_eq!(admin_view["total"], 1, "{admin_view}");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_failed_permission_check_records_nothing() {
    let (dir, state, app) = boot("denied");
    let (_, team) = session(&state, "admin-session", "admin");
    state
        .store()
        .insert_user("viewer-id", "viewer", "Viewer", None, "local", false)
        .expect("viewer");
    state
        .store()
        .grant_manual_membership(&team, "viewer-id", "viewer")
        .expect("membership");
    state
        .store()
        .create_session("viewer-session", "viewer-id")
        .expect("session");

    let (status, denied) = call(
        &app,
        "POST",
        "/api/v1/templates",
        "viewer-session",
        Some(&team),
        Some(json!({"name": "Denied", "tool": "shell", "spec": {"script": "true"}})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");

    let (status, log) = call(
        &app,
        "GET",
        "/api/v1/activity",
        "admin-session",
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{log}");
    assert_eq!(log["total"], 0, "{log}");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn schedule_and_hook_launches_keep_the_launch_principal() {
    let (dir, state, app) = boot("principal");
    let (_, team) = session(&state, "admin-session", "admin");
    let health = state
        .store()
        .list_templates(&team)
        .expect("templates")
        .into_iter()
        .find(|item| item.name == "Healthcheck")
        .expect("healthcheck");
    let now = db::now();

    let schedule = db::Schedule {
        id: "schedule-1".into(),
        team_id: team.clone(),
        name: "Nightly check".into(),
        target_kind: "template".into(),
        target_id: health.id.clone(),
        timezone: "UTC".into(),
        start_date: "2026-01-01".into(),
        start_time: "00:00".into(),
        frequency: "daily".into(),
        weekdays: String::new(),
        end_kind: "never".into(),
        end_count: None,
        end_date: None,
        enabled: true,
        next_run_at: None,
        missed_at: None,
        occurrence_count: 0,
        created_by: "alice".into(),
        updated_by: "alice".into(),
        created_at: now.clone(),
        updated_at: now.clone(),
        launch_block: None,
    };
    start_schedule(&state, &schedule, "alice").expect("schedule launch");

    let hook = db::Hook {
        id: "hook-1".into(),
        team_id: team.clone(),
        name: "Alert hook".into(),
        token: "token".into(),
        match_alertname: "DiskSpaceLow".into(),
        match_status: "firing".into(),
        template_id: Some(health.id.clone()),
        workflow_id: None,
        extra_vars: "{}".into(),
        enabled: true,
        created_at: now,
    };
    let dispatch = jobs::accept_hook(
        &state,
        &hook,
        &json!({
            "status": "firing",
            "alerts": [{"status": "firing", "labels": {"alertname": "DiskSpaceLow"}}]
        }),
        None,
    )
    .expect("hook launch");
    assert_eq!(dispatch.status, "accepted");

    let (status, log) = call(
        &app,
        "GET",
        "/api/v1/activity?limit=100",
        "admin-session",
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{log}");
    let items = log["items"].as_array().expect("items");
    assert!(
        items.iter().any(|item| item["actor"] == "alice"
            && item["action"] == "launch"
            && item["object_type"] == "job"),
        "{log}"
    );
    assert!(
        items.iter().any(|item| item["actor"] == "hook:Alert hook"
            && item["action"] == "launch"
            && item["object_type"] == "job"),
        "{log}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn source_and_environment_writes_are_recorded() {
    let (dir, state, app) = boot("children");
    let (_, team) = session(&state, "admin-session", "admin");

    let (status, inventory) = call(
        &app,
        "POST",
        "/api/v1/inventories",
        "admin-session",
        Some(&team),
        Some(json!({"name": "Activity inventory", "body": "[all]\nlocalhost\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().expect("inventory id");

    let (status, added) = call(
        &app,
        "POST",
        &format!("/api/v1/inventories/{inventory_id}/sources"),
        "admin-session",
        Some(&team),
        Some(json!({"kind": "ini", "name": "Extra source", "body": "[all]\nlocalhost\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{added}");
    let source_id = added["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .find(|source| source["name"] == "Extra source")
        .expect("source")["id"]
        .as_str()
        .expect("source id")
        .to_string();

    let (status, renamed) = call(
        &app,
        "PUT",
        &format!("/api/v1/inventories/{inventory_id}/sources/{source_id}"),
        "admin-session",
        Some(&team),
        Some(json!({
            "name": "Extra source renamed",
            "body": "[all]\nlocalhost\n"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    let (status, removed) = call(
        &app,
        "DELETE",
        &format!("/api/v1/inventories/{inventory_id}/sources/{source_id}"),
        "admin-session",
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{removed}");

    let (status, environment) = call(
        &app,
        "POST",
        "/api/v1/environments",
        "admin-session",
        Some(&team),
        Some(json!({"name": "Activity environment", "image": "alpine:3", "tool": "shell"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{environment}");
    let environment_id = environment["id"].as_str().expect("environment id");
    let (status, updated) = call(
        &app,
        "PUT",
        &format!("/api/v1/environments/{environment_id}"),
        "admin-session",
        Some(&team),
        Some(json!({"name": "Activity environment renamed", "image": "alpine:3", "tool": "shell"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    let (status, removed) = call(
        &app,
        "DELETE",
        &format!("/api/v1/environments/{environment_id}"),
        "admin-session",
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{removed}");

    let (status, log) = call(
        &app,
        "GET",
        "/api/v1/activity?limit=100",
        "admin-session",
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{log}");
    let items = log["items"].as_array().expect("items");
    let has = |action: &str, object_type: &str, name: &str| {
        items.iter().any(|item| {
            item["action"] == action
                && item["object_type"] == object_type
                && item["object_name"] == name
        })
    };
    assert!(has("create", "inventory_source", "Extra source"), "{log}");
    assert!(
        has("update", "inventory_source", "Extra source renamed"),
        "{log}"
    );
    assert!(
        has("delete", "inventory_source", "Extra source renamed"),
        "{log}"
    );
    assert!(
        has("create", "environment", "Activity environment"),
        "{log}"
    );
    assert!(
        has("update", "environment", "Activity environment renamed"),
        "{log}"
    );
    assert!(
        has("delete", "environment", "Activity environment renamed"),
        "{log}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn secrets_and_extra_vars_never_reach_the_activity_log() {
    let (dir, state, app) = boot("secrets");
    let (_, team) = session(&state, "admin-session", "admin");
    let planted = "PLANTED-SECRET-559";

    let (status, credential) = call(
        &app,
        "POST",
        "/api/v1/credentials",
        "admin-session",
        Some(&team),
        Some(json!({
            "name": "Activity credential",
            "kind": "ssh",
            "secret": {"username": "deploy", "password": planted}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");

    let (status, template) = call(
        &app,
        "POST",
        "/api/v1/templates",
        "admin-session",
        Some(&team),
        Some(json!({
            "name": "Secret probe",
            "tool": "shell",
            "extra_vars": {"token": planted},
            "prompt_on_launch": true,
            "spec": {"script": "#!/bin/sh\ntrue\n"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().expect("template id");

    let (status, launched) = call(
        &app,
        "POST",
        &format!("/api/v1/templates/{template_id}/launch"),
        "admin-session",
        Some(&team),
        Some(json!({"extra_vars": {"token": planted}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");

    let (status, log) = call(
        &app,
        "GET",
        "/api/v1/activity?limit=100",
        "admin-session",
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{log}");
    let text = serde_json::to_string(&log).expect("json");
    assert!(!text.contains(planted), "{text}");
    let items = log["items"].as_array().expect("items");
    assert!(
        items
            .iter()
            .any(|item| item["object_name"] == "Activity credential"),
        "{log}"
    );
    assert!(
        items
            .iter()
            .any(|item| item["object_name"] == "Secret probe"),
        "{log}"
    );
    let _ = std::fs::remove_dir_all(dir);
}
