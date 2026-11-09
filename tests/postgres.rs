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
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
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
async fn postgres_starts_and_serves_the_auth_slice() {
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

    let (status, body, _) = call(&app, "GET", "/api/teams", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body["error"],
        "this_feature_is_not_available_with_postgresql_yet"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
