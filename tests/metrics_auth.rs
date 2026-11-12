#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;

use axum::http::StatusCode;
use common::{call_raw, Options, TestApp, ADMIN_PASSWORD};

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
    let app = TestApp::boot("metrics-auth", Options::default());

    let (status, body, _) = call_raw(&app.app, "GET", "/metrics", None, &[], None).await;
    let body = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("# TYPE koreograf_jobs_started_total counter"),
        "{body}"
    );

    let cookie = app.login("admin", ADMIN_PASSWORD).await;

    std::env::set_var("KOREOGRAF_METRICS_TOKEN", "scrape-secret");
    let (status, body, _) = call_raw(&app.app, "GET", "/metrics", None, &[], None).await;
    let body = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("sign_in_is_required"), "{body}");

    let (status, _, _) = call_raw(
        &app.app,
        "GET",
        "/metrics",
        None,
        &[("authorization", "Bearer wrong-secret")],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body, _) = call_raw(
        &app.app,
        "GET",
        "/metrics",
        None,
        &[("authorization", "Bearer scrape-secret")],
        None,
    )
    .await;
    let body = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("# TYPE koreograf_cache_collections gauge"),
        "{body}"
    );
    assert!(!body.contains("scrape-secret"), "{body}");

    let (status, body, _) = call_raw(&app.app, "GET", "/metrics", Some(&cookie), &[], None).await;
    let body = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("# TYPE koreograf_jobs_finished_total counter"),
        "{body}"
    );

    std::env::remove_var("KOREOGRAF_METRICS_TOKEN");
}
