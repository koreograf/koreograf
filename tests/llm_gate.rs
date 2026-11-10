use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
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

struct EnvGuard;

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for key in [
            "KOREOGRAF_LLM",
            "KOREOGRAF_LLM_URL",
            "KOREOGRAF_LLM_MODEL",
            "KOREOGRAF_LLM_API_KEY",
            "KOREOGRAF_LLM_API_KEY_FILE",
            "KOREOGRAF_LLM_MAX_TOKENS",
            "KOREOGRAF_LLM_TIMEOUT_SECS",
        ] {
            std::env::remove_var(key);
        }
    }
}

#[tokio::test]
async fn the_assistant_route_is_off_until_the_environment_enables_it() {
    let _guard = EnvGuard;
    for key in [
        "KOREOGRAF_LLM",
        "KOREOGRAF_LLM_URL",
        "KOREOGRAF_LLM_MODEL",
        "KOREOGRAF_LLM_API_KEY",
        "KOREOGRAF_LLM_API_KEY_FILE",
    ] {
        std::env::remove_var(key);
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-llm-gate-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");

    let (status, body, _) = call(&app, "GET", "/api/assistant", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "sign_in_is_required");

    let (status, body, _) = call(&app, "GET", "/api/assistant", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "assistant_is_not_enabled");

    let (status, body, _) = call(
        &app,
        "POST",
        "/api/assistant/chat",
        Some(&cookie),
        Some(json!({"message": "hello"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "assistant_is_not_enabled");

    std::env::set_var("KOREOGRAF_LLM", "1");
    let (status, body, _) = call(&app, "GET", "/api/assistant", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "assistant_is_not_configured");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    std::env::set_var("KOREOGRAF_LLM_URL", format!("http://{address}/v1"));
    std::env::set_var("KOREOGRAF_LLM_MODEL", "local-model");
    std::env::set_var("KOREOGRAF_LLM_API_KEY", "sk-test-secret");
    let (status, body, _) = call(&app, "GET", "/api/assistant", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], true);
    assert_eq!(body["model"], "local-model");
    assert!(!body.to_string().contains("sk-test-secret"));

    let (status, body, _) = call(
        &app,
        "POST",
        "/api/assistant/chat",
        Some(&cookie),
        Some(json!({"message": "hello", "history": [{"role": "system", "content": "x"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "assistant_history_is_invalid");
    assert!(!body.to_string().contains("sk-test-secret"));

    let (status, body, _) = call(
        &app,
        "POST",
        "/api/assistant/chat",
        Some(&cookie),
        Some(json!({"message": "hello"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"], "assistant_upstream_unreachable");
    assert!(!body.to_string().contains("sk-test-secret"));

    let _ = std::fs::remove_dir_all(&dir);
}
