#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::net::SocketAddr;
use std::path::Path;

use axum::http::StatusCode;
use common::{call, call_raw_peer, Options, TestApp, ADMIN_PASSWORD};
use serde_json::json;

const LOOPBACK_PEER: &str = "127.0.0.1:49152";
const LOOPBACK_PEER_V6: &str = "[::1]:49152";
const REMOTE_PEER: &str = "203.0.113.7:49152";

fn peer(value: &str) -> SocketAddr {
    value.parse().expect("peer address")
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
async fn metrics_allow_loopback_by_default_and_require_the_token_or_an_admin_session() {
    std::env::remove_var("KOREOGRAF_METRICS_TOKEN");
    let app = TestApp::boot("metrics-auth", Options::default());

    // Without a token, a scrape from the process or a sidecar on loopback
    // keeps working.
    for loopback in [LOOPBACK_PEER, LOOPBACK_PEER_V6] {
        let (status, body, _) =
            call_raw_peer(&app.app, "GET", "/metrics", peer(loopback), None, &[], None).await;
        let body = String::from_utf8_lossy(&body);
        assert_eq!(status, StatusCode::OK, "{loopback}: {body}");
        assert!(
            body.contains("# TYPE koreograf_jobs_started_total counter"),
            "{body}"
        );
        assert!(
            body.contains("# TYPE koreograf_job_duration_seconds histogram"),
            "{body}"
        );
        assert!(
            body.contains("# TYPE koreograf_jobs_pending gauge"),
            "{body}"
        );
        assert!(
            body.contains("# TYPE koreograf_jobs_running gauge"),
            "{body}"
        );
        assert!(body.contains("\nkoreograf_jobs_pending 0\n"), "{body}");
        assert!(body.contains("\nkoreograf_jobs_running 0\n"), "{body}");
    }

    // A non-loopback peer gets the keyed error and no metrics text. Forwarded
    // headers must not grant the loopback exception.
    let (status, body, _) = call_raw_peer(
        &app.app,
        "GET",
        "/metrics",
        peer(REMOTE_PEER),
        None,
        &[("x-forwarded-for", "127.0.0.1"), ("x-real-ip", "127.0.0.1")],
        None,
    )
    .await;
    let body = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("sign_in_is_required"), "{body}");
    assert!(!body.contains("koreograf_"), "{body}");

    let (status, body, _) = call_raw_peer(
        &app.app,
        "GET",
        "/healthz",
        peer(REMOTE_PEER),
        None,
        &[],
        None,
    )
    .await;
    let body = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("ok ("), "{body}");

    let cookie = app.login("admin", ADMIN_PASSWORD).await;

    // A regular personal API token is not the metrics token.
    let (status, created) = call(
        &app.app,
        "POST",
        "/api/me/tokens",
        Some(&cookie),
        None,
        Some(json!({"name": "metrics-reader"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let personal_token = created["secret"]
        .as_str()
        .expect("personal token")
        .to_string();

    std::env::set_var("KOREOGRAF_METRICS_TOKEN", "scrape-secret");

    // Once the token is set, even loopback needs it.
    let (status, body, _) = call_raw_peer(
        &app.app,
        "GET",
        "/metrics",
        peer(LOOPBACK_PEER),
        None,
        &[],
        None,
    )
    .await;
    let body = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("sign_in_is_required"), "{body}");

    let (status, _, _) = call_raw_peer(
        &app.app,
        "GET",
        "/metrics",
        peer(REMOTE_PEER),
        None,
        &[("authorization", "Bearer wrong-secret")],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let personal = format!("Bearer {personal_token}");
    let (status, body, _) = call_raw_peer(
        &app.app,
        "GET",
        "/metrics",
        peer(REMOTE_PEER),
        None,
        &[("authorization", personal.as_str())],
        None,
    )
    .await;
    let body = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("sign_in_is_required"), "{body}");
    assert!(!body.contains("koreograf_"), "{body}");

    let (status, body, _) = call_raw_peer(
        &app.app,
        "GET",
        "/metrics",
        peer(REMOTE_PEER),
        None,
        &[("authorization", "Bearer scrape-secret")],
        None,
    )
    .await;
    let body = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("# TYPE koreograf_cache_collections gauge"),
        "{body}"
    );
    assert!(!body.contains("scrape-secret"), "{body}");

    let (status, body, _) = call_raw_peer(
        &app.app,
        "GET",
        "/metrics",
        peer(REMOTE_PEER),
        Some(&cookie),
        &[],
        None,
    )
    .await;
    let body = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("# TYPE koreograf_jobs_finished_total counter"),
        "{body}"
    );
    assert!(!body.contains("scrape-secret"), "{body}");

    let (status, body, _) = call_raw_peer(
        &app.app,
        "GET",
        "/healthz",
        peer(REMOTE_PEER),
        None,
        &[],
        None,
    )
    .await;
    let body = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("ok ("), "{body}");

    std::env::remove_var("KOREOGRAF_METRICS_TOKEN");
}
