use axum::http::StatusCode;
use serde_json::json;

use crate::common::call_cookie as call;

use crate::common::*;

#[tokio::test]
async fn a_team_description_stops_at_one_thousand_characters() {
    let harness = TestApp::boot("desc", Options::default());
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

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Lang",
            "description": "ä".repeat(1000),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(
        created["description"].as_str().unwrap().chars().count(),
        1000
    );

    let (status, rejected, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Zu lang",
            "description": "ä".repeat(1001),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["error"], "description_is_too_long");

    let (status, named, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(team),
        Some(json!({ "name": "ü".repeat(120) })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{named}");
    let (status, too_long, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(team),
        Some(json!({ "name": "ü".repeat(121) })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{too_long}");
    assert_eq!(too_long["error"], "name_is_missing_or_too_long");
    let _ = std::fs::remove_dir_all(&dir);
}
