#![allow(clippy::unwrap_used, clippy::expect_used)]

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

async fn app_and_login(dir: &Path) -> (axum::Router, String, String) {
    let app = koreograf::router(koreograf::boot(config(dir)).unwrap());
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

async fn shell_template(
    app: &axum::Router,
    cookie: &str,
    team: &str,
    name: &str,
    prompt_on_launch: bool,
) -> String {
    let (status, template, _) = call(
        app,
        "POST",
        "/api/templates",
        Some(cookie),
        Some(team),
        Some(json!({
            "name": name,
            "tool": "shell",
            "entrypoint": "",
            "prompt_on_launch": prompt_on_launch,
            "extra_vars": {"keep": "template"},
            "spec": {"script": "true"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    template["id"].as_str().unwrap().to_string()
}

async fn cancel(app: &axum::Router, cookie: &str, team: &str, job_id: &str) {
    let (status, body, _) = call(
        app,
        "POST",
        &format!("/api/jobs/{job_id}/cancel"),
        Some(cookie),
        Some(team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn launch_extra_vars_follow_the_prompt_gate() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-launch-overrides-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let (app, cookie, team) = app_and_login(&dir).await;
    let quiet = shell_template(&app, &cookie, &team, "Quiet", false).await;

    let (status, blocked, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{quiet}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {"from_launch": "yes"}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{blocked}");
    assert_eq!(blocked["error"], "prompt_on_launch_is_off");

    let (status, plain, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{quiet}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{plain}");
    assert_eq!(plain["parameters"]["extra_vars"]["keep"], "template");
    cancel(&app, &cookie, &team, plain["id"].as_str().unwrap()).await;

    let (status, empty, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{quiet}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{empty}");
    cancel(&app, &cookie, &team, empty["id"].as_str().unwrap()).await;

    let (status, options, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{quiet}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "extra_vars": {"from_launch": "yes"},
            "launch_with_options": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{options}");
    assert_eq!(options["parameters"]["extra_vars"]["keep"], "template");
    assert_eq!(options["parameters"]["extra_vars"]["from_launch"], "yes");
    assert_eq!(options["parameters"]["launch_with_options"], true);
    cancel(&app, &cookie, &team, options["id"].as_str().unwrap()).await;

    let prompted = shell_template(&app, &cookie, &team, "Prompted", true).await;
    let (status, prompted_job, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{prompted}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {"from_launch": "yes"}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{prompted_job}");
    assert_eq!(prompted_job["parameters"]["extra_vars"]["keep"], "template");
    assert_eq!(
        prompted_job["parameters"]["extra_vars"]["from_launch"],
        "yes"
    );
    cancel(&app, &cookie, &team, prompted_job["id"].as_str().unwrap()).await;

    let (status, stored, _) = call(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    let row = stored
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == quiet)
        .unwrap();
    assert_eq!(row["extra_vars"]["keep"], "template");
    assert!(row["extra_vars"].get("from_launch").is_none());

    let _ = std::fs::remove_dir_all(&dir);
}
