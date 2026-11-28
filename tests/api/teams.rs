use axum::http::StatusCode;
use serde_json::json;

use crate::common::call_cookie as call;

use crate::common::*;

#[tokio::test(flavor = "multi_thread")]
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

async fn add_member(app: &axum::Router, cookie: &str, team: &str, username: &str, role: &str) {
    let (status, created, _) = call(
        app,
        "POST",
        "/api/users",
        Some(cookie),
        Some(team),
        Some(json!({
            "username": username,
            "display_name": username,
            "password": format!("{username}-pass"),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let (status, added, _) = call(
        app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(cookie),
        Some(team),
        Some(json!({"username": username, "role": role})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{added}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admin_cannot_remove_their_own_membership_but_can_remove_an_owner() {
    let harness = TestApp::boot("team-admin-self-removal", Options::default());
    let app = harness.app.clone();
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
    let admin_id = me["id"].as_str().unwrap();
    let team = me["teams"][0]["id"].as_str().unwrap();

    let (status, refused, _) = call(
        &app,
        "DELETE",
        &format!("/api/teams/{team}/members/{admin_id}"),
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["error"], "you_cannot_remove_yourself_from_the_team");
    let remaining = harness
        .sql_i64(&format!(
            "SELECT COUNT(*) FROM memberships WHERE team_id = '{team}' AND user_id = '{admin_id}'"
        ))
        .await;
    assert_eq!(remaining, 1);

    add_member(&app, &cookie, team, "second-owner", "owner").await;
    let (status, members, _) = call(
        &app,
        "GET",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{members}");
    let second = members
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["username"] == "second-owner")
        .expect("second owner");
    let second_id = second["user_id"].as_str().unwrap();

    let (status, removed, _) = call(
        &app,
        "DELETE",
        &format!("/api/teams/{team}/members/{second_id}"),
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{removed}");
    let left = harness
        .sql_i64(&format!(
            "SELECT COUNT(*) FROM memberships WHERE team_id = '{team}' AND user_id = '{second_id}'"
        ))
        .await;
    assert_eq!(left, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_non_admin_owner_still_cannot_remove_themselves() {
    let harness = TestApp::boot("team-owner-self-removal", Options::default());
    let app = harness.app.clone();
    let (status, me, admin_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let admin_cookie = admin_cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap();

    add_member(&app, &admin_cookie, team, "team-owner", "owner").await;
    let (status, owner, owner_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "team-owner", "password": "team-owner-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{owner}");
    let owner_cookie = owner_cookie.expect("owner cookie");
    let owner_id = owner["id"].as_str().unwrap();

    let (status, refused, _) = call(
        &app,
        "DELETE",
        &format!("/api/teams/{team}/members/{owner_id}"),
        Some(&owner_cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["error"], "you_cannot_remove_yourself_from_the_team");
    let remaining = harness
        .sql_i64(&format!(
            "SELECT COUNT(*) FROM memberships WHERE team_id = '{team}' AND user_id = '{owner_id}'"
        ))
        .await;
    assert_eq!(remaining, 1);
}
