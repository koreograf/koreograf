#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use serde_json::json;

use common::{drop_database, postgres_guard, postgres_test_url, Options, TestApp};

async fn auth_flow(app: &TestApp) {
    let (cookie, team) = app.sign_in().await;

    let (status, me) = app.call("GET", "/api/me", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["username"], "admin");
    assert_eq!(me["is_admin"], true);

    let (status, created) = app
        .call(
            "POST",
            "/api/users",
            Some(&cookie),
            Some(&team),
            Some(json!({
                "username": "shared-flow",
                "display_name": "Shared Flow",
                "password": "shared-flow-pass",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let user_id = created["id"].as_str().expect("user id").to_string();

    let (status, duplicate) = app
        .call(
            "POST",
            "/api/users",
            Some(&cookie),
            Some(&team),
            Some(json!({
                "username": "Shared-Flow",
                "display_name": "Shared Flow Again",
                "password": "shared-flow-pass",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{duplicate}");
    assert_eq!(duplicate["error"], "that_username_is_already_used");

    let (status, users) = app
        .call("GET", "/api/users", Some(&cookie), None, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{users}");
    assert!(users
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == user_id)));

    let (status, detail) = app
        .call(
            "GET",
            &format!("/api/users/{user_id}"),
            Some(&cookie),
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["display_name"], "Shared Flow");

    let (status, updated) = app
        .call(
            "PUT",
            &format!("/api/users/{user_id}"),
            Some(&cookie),
            None,
            Some(json!({
                "display_name": "Shared Flow Renamed",
                "is_admin": false,
                "disabled": false,
                "password": "shared-flow-pass-2",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["display_name"], "Shared Flow Renamed");

    let user_cookie = app.login("shared-flow", "shared-flow-pass-2").await;

    let (status, user_me) = app
        .call("GET", "/api/me", Some(&user_cookie), None, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{user_me}");
    assert_eq!(user_me["username"], "shared-flow");

    let (status, settings) = app
        .call("GET", "/api/settings", Some(&cookie), None, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{settings}");
    let (status, saved) = app
        .call(
            "PUT",
            "/api/settings",
            Some(&cookie),
            None,
            Some(json!({
                "ldap": {},
                "proxy": {},
                "runner": "local",
                "job_timeout_secs": 60,
                "job_output_lines": 0
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["source"], "saved");
    let (status, reread) = app
        .call("GET", "/api/settings", Some(&cookie), None, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{reread}");
    assert_eq!(reread["runner"], "local");

    let (status, token) = app
        .call(
            "POST",
            "/api/me/tokens",
            Some(&user_cookie),
            None,
            Some(json!({"name": "shared-flow-tool"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{token}");
    let secret = token["secret"].as_str().expect("token secret").to_string();
    let token_id = token["id"].as_str().expect("token id").to_string();

    let (status, listed) = app
        .call("GET", "/api/me/tokens", Some(&user_cookie), None, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(listed
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == token_id)));
    assert!(!listed.to_string().contains(&secret));

    let (status, bearer_me) = app.call_bearer("GET", "/api/me", &secret, None).await;
    assert_eq!(status, StatusCode::OK, "{bearer_me}");
    assert_eq!(bearer_me["username"], "shared-flow");

    let (status, _) = app
        .call(
            "DELETE",
            &format!("/api/me/tokens/{token_id}"),
            Some(&user_cookie),
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, revoked) = app.call_bearer("GET", "/api/me", &secret, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{revoked}");

    let (status, _) = app
        .call("POST", "/api/logout", Some(&user_cookie), None, None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, after_logout) = app
        .call("GET", "/api/me", Some(&user_cookie), None, None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{after_logout}");

    let (status, _) = app
        .call(
            "DELETE",
            &format!("/api/users/{user_id}"),
            Some(&cookie),
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, deleted) = app
        .call(
            "POST",
            "/api/login",
            None,
            None,
            Some(json!({"username": "shared-flow", "password": "shared-flow-pass-2"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{deleted}");
    assert_eq!(deleted["error"], "invalid_username_or_password");
}

#[tokio::test]
async fn shared_auth_flow_runs_on_sqlite() {
    let app = TestApp::boot("shared-auth-sqlite", Options::default());
    auth_flow(&app).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_auth_flow_runs_on_postgres() {
    let _guard = postgres_guard().await;
    let Some(app) = TestApp::boot_postgres("shared-auth-postgres", Options::default()).await else {
        return;
    };
    auth_flow(&app).await;
    let base = postgres_test_url().expect("postgres test url");
    let name = app.database_name.clone().expect("database name");
    drop(app);
    drop_database(&base, &name).await;
}
