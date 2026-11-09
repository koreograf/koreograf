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
        web_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("web"),
        examples_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("examples"),
        admin_user: "admin".into(),
        admin_password: Some("test-admin-pass".into()),
        runner: RunnerMode::Local,
        container_image: false,
        job_timeout_secs: 60,
        job_output_lines: 8_000,
        local_jobs: LocalJobs::All,
        project_roots: vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("examples")],
        hsts: false,
        cookie_secure: CookieSecure::Auto,
        trusted_proxy: false,
        mcp: false,
        public_url: Vec::new(),
        data_host: None,
        ldap: None,
    }
}

fn data_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-{name}-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
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

async fn login(
    app: &axum::Router,
    username: &str,
    password: &str,
) -> (StatusCode, Value, Option<String>) {
    call(
        app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({ "username": username, "password": password })),
    )
    .await
}

async fn admin_app(name: &str) -> (axum::Router, String, std::path::PathBuf) {
    let dir = data_dir(name);
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);
    let (status, body, cookie) = login(&app, "admin", "test-admin-pass").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    (app, cookie.expect("admin session"), dir)
}

#[tokio::test]
async fn names_and_descriptions_reject_bidi_controls() {
    let (app, cookie, dir) = admin_app("bidi-names").await;
    let marks = [
        '\u{061c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}',
        '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
    ];
    for mark in marks {
        let (status, body, _) = call(
            &app,
            "POST",
            "/api/teams",
            Some(&cookie),
            None,
            Some(json!({ "name": format!("Team{mark}Name"), "description": "" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{mark:?} {body}");
        assert_eq!(body["error"], "the_name_has_invalid_characters", "{mark:?}");
    }
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        None,
        Some(json!({ "name": "Neutral", "description": "plain" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().expect("team id");
    let (status, body, _) = call(
        &app,
        "PUT",
        &format!("/api/teams/{id}"),
        Some(&cookie),
        None,
        Some(json!({ "name": "Re\u{202e}named", "description": "plain" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "the_name_has_invalid_characters");
    let (status, body, _) = call(
        &app,
        "PUT",
        &format!("/api/teams/{id}"),
        Some(&cookie),
        None,
        Some(json!({ "name": "Neutral", "description": "a\u{202e}b" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "the_value_contains_invalid_characters");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn names_are_trimmed_and_bounded_by_characters() {
    let (app, cookie, dir) = admin_app("name-limits").await;
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        None,
        Some(json!({ "name": "  Trimmed  ", "description": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["name"], "Trimmed");

    for name in ["   ", &"x".repeat(121), &"\u{00e9}".repeat(121)] {
        let (status, body, _) = call(
            &app,
            "POST",
            "/api/teams",
            Some(&cookie),
            None,
            Some(json!({ "name": name, "description": "" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name:?} {body}");
        assert_eq!(body["error"], "name_is_missing_or_too_long", "{name:?}");
    }
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        None,
        Some(json!({ "name": "\u{00e9}".repeat(120), "description": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["name"].as_str().unwrap().chars().count(), 120);

    let (status, body, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        None,
        Some(json!({ "name": "a\u{7}b", "description": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "the_name_has_invalid_characters");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_disabled_account_gets_the_generic_sign_in_failure() {
    let (app, cookie, dir) = admin_app("disabled-signin").await;
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        None,
        Some(json!({
            "username": "ada",
            "password": "ada-secret-1",
            "display_name": "Ada",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().expect("user id");
    let (status, disabled, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{id}"),
        Some(&cookie),
        None,
        Some(json!({ "display_name": "Ada", "is_admin": false, "disabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{disabled}");
    assert_eq!(disabled["disabled"], true);

    for (username, password) in [
        ("ada", "ada-secret-1"),
        ("ada", "wrong-password"),
        ("nobody", "wrong-password"),
    ] {
        let (status, body, _) = login(&app, username, password).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{username} {body}");
        assert_eq!(body["error"], "invalid_username_or_password", "{username}");
        assert!(body.get("vars").is_none(), "{username}");
    }
    let _ = std::fs::remove_dir_all(dir);
}
