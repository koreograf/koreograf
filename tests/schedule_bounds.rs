#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Months;
use http_body_util::BodyExt;
use koreograf::{Config, CookieSecure, LocalJobs, RunnerMode};
use serde_json::{json, Value};
use tower::ServiceExt;

fn config(dir: &std::path::Path) -> Config {
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

fn schedule(start_date: &str) -> Value {
    json!({
        "name": "Bound",
        "timezone": "Europe/Berlin",
        "start_date": start_date,
        "start_time": "06:00",
        "frequency": "once",
        "end_kind": "never"
    })
}

#[tokio::test]
async fn a_start_more_than_ten_years_ahead_is_rejected() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-schedule-bound-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
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

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Scheduled",
            "tool": "shell",
            "update_on_launch": false,
            "spec": {"script": "echo scheduled"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();
    let schedules = format!("/api/templates/{template_id}/schedules");

    let today = chrono::Utc::now()
        .with_timezone(&chrono_tz::Europe::Berlin)
        .date_naive();
    let limit = today
        .checked_add_months(Months::new(120))
        .expect("ten years");
    let exact = limit.format("%Y-%m-%d").to_string();
    let too_far = limit
        .succ_opt()
        .expect("next day")
        .format("%Y-%m-%d")
        .to_string();

    let (status, far, _) = call(
        &app,
        "POST",
        &schedules,
        Some(&cookie),
        Some(&team),
        Some(schedule("9999-12-31")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{far}");
    assert_eq!(far["error"], "schedule_date_is_too_far_in_the_future");

    let (status, edge, _) = call(
        &app,
        "POST",
        &schedules,
        Some(&cookie),
        Some(&team),
        Some(schedule(&too_far)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{edge}");
    assert_eq!(edge["error"], "schedule_date_is_too_far_in_the_future");

    let (status, accepted, _) = call(
        &app,
        "POST",
        &schedules,
        Some(&cookie),
        Some(&team),
        Some(schedule(&exact)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{accepted}");
    let schedule_id = accepted["id"].as_str().unwrap().to_string();

    let (status, moved, _) = call(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        Some(schedule("9999-12-31")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{moved}");
    assert_eq!(moved["error"], "schedule_date_is_too_far_in_the_future");

    let mut preview = schedule("9999-12-31");
    preview.as_object_mut().unwrap().remove("name");
    let (status, preview, _) = call(
        &app,
        "POST",
        "/api/schedules/preview",
        Some(&cookie),
        Some(&team),
        Some(preview),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{preview}");
    assert_eq!(preview["error"], "schedule_date_is_too_far_in_the_future");
}
