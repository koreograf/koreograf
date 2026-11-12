#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use koreograf::{Config, CookieSecure, LocalJobs, RunnerMode};
use tower::ServiceExt;

fn config(dir: &Path) -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        data_dir: dir.to_path_buf(),
        data_host: None,
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
        ldap: None,
        hsts: false,
        cookie_secure: CookieSecure::Auto,
        trusted_proxy: false,
        mcp: true,
        public_url: Vec::new(),
    }
}

async fn get(
    app: &axum::Router,
    authorization: Option<&str>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().uri("/metrics");
    if let Some(value) = authorization {
        builder = builder.header("authorization", value);
    }
    if let Some(value) = cookie {
        builder = builder.header("cookie", value);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn login(app: &axum::Router) -> String {
    let request = Request::builder()
        .method("POST")
        .uri("/api/login")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"username":"admin","password":"test-admin-pass"}"#,
        ))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::to_string)
        .expect("session cookie")
}

#[test]
fn the_deployments_forward_the_metrics_token() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let compose =
        std::fs::read_to_string(root.join("docker-compose.yml")).expect("docker-compose.yml");
    assert!(
        compose.contains("KOREOGRAF_METRICS_TOKEN: ${KOREOGRAF_METRICS_TOKEN:-}"),
        "{compose}"
    );
    let deployment =
        std::fs::read_to_string(root.join("deploy/helm/koreograf/templates/deployment.yaml"))
            .expect("deployment.yaml");
    assert!(
        deployment.contains("name: KOREOGRAF_METRICS_TOKEN"),
        "{deployment}"
    );
    assert!(
        deployment.contains("existingSecretKeys.metricsToken"),
        "{deployment}"
    );
    let secret = std::fs::read_to_string(root.join("deploy/helm/koreograf/templates/secret.yaml"))
        .expect("secret.yaml");
    assert!(secret.contains("metrics.token"), "{secret}");
    let values = std::fs::read_to_string(root.join("deploy/helm/koreograf/values.yaml"))
        .expect("values.yaml");
    assert!(values.contains("metrics:\n  token: \"\""), "{values}");
}

#[tokio::test]
async fn metrics_require_the_configured_token_or_an_admin_session() {
    std::env::remove_var("KOREOGRAF_METRICS_TOKEN");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-metrics-auth-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, body) = get(&app, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("# TYPE koreograf_jobs_started_total counter"),
        "{body}"
    );

    let cookie = login(&app).await;

    std::env::set_var("KOREOGRAF_METRICS_TOKEN", "scrape-secret");
    let (status, body) = get(&app, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("sign_in_is_required"), "{body}");

    let (status, _) = get(&app, Some("Bearer wrong-secret"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body) = get(&app, Some("Bearer scrape-secret"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("# TYPE koreograf_cache_collections gauge"),
        "{body}"
    );
    assert!(!body.contains("scrape-secret"), "{body}");

    let (status, body) = get(&app, None, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("# TYPE koreograf_jobs_finished_total counter"),
        "{body}"
    );

    std::env::remove_var("KOREOGRAF_METRICS_TOKEN");
    let _ = std::fs::remove_dir_all(&dir);
}
