use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use koreograf::{CookieSecure, LocalJobs};
use serde_json::{json, Value};

use crate::common::{
    call_bearer_team as call_bearer, call_cookie as call, call_raw_headers as call_raw,
};

use crate::common::*;

#[tokio::test]
async fn session_cookie_secure_follows_the_setting_and_the_proxy() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-cookie-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let login = br#"{"username":"admin","password":"test-admin-pass"}"#.to_vec();

    let state = koreograf::boot(config(&dir, &Options::default())).unwrap();
    let app = koreograf::router(state);
    let (status, _, headers) = call_raw(
        &app,
        "POST",
        "/api/login",
        None,
        &[
            ("content-type", "application/json"),
            ("x-forwarded-proto", "https"),
        ],
        Some(login.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let set_cookie = headers["set-cookie"].to_str().unwrap();
    assert!(set_cookie.contains("HttpOnly"), "{set_cookie}");
    assert!(!set_cookie.contains("Secure"), "{set_cookie}");
    drop(app);

    let mut forced = config(&dir, &Options::default());
    forced.cookie_secure = CookieSecure::True;
    let state = koreograf::boot(forced).unwrap();
    let app = koreograf::router(state);
    let (status, _, headers) = call_raw(
        &app,
        "POST",
        "/api/login",
        None,
        &[("content-type", "application/json")],
        Some(login.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let set_cookie = headers["set-cookie"].to_str().unwrap().to_string();
    assert!(set_cookie.ends_with("; Secure"), "{set_cookie}");
    let session = set_cookie.split(';').next().unwrap();
    let (status, _, headers) =
        call_raw(&app, "POST", "/api/logout", Some(session), &[], None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let cleared = headers["set-cookie"].to_str().unwrap();
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    assert!(cleared.ends_with("; Secure"), "{cleared}");
    drop(app);

    let mut automatic = config(&dir, &Options::default());
    automatic.trusted_proxy = true;
    let state = koreograf::boot(automatic).unwrap();
    let app = koreograf::router(state);
    let (status, _, headers) = call_raw(
        &app,
        "POST",
        "/api/login",
        None,
        &[
            ("content-type", "application/json"),
            ("x-forwarded-proto", "https"),
        ],
        Some(login.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers["set-cookie"]
        .to_str()
        .unwrap()
        .ends_with("; Secure"));
    let (status, _, headers) = call_raw(
        &app,
        "POST",
        "/api/login",
        None,
        &[("content-type", "application/json")],
        Some(login),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!headers["set-cookie"].to_str().unwrap().contains("Secure"));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn admin_can_list_and_update_users() {
    let harness = TestApp::boot("users", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/features/legacy.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
    assert!(script.contains("search_users"));
    assert!(script.contains("#/users"));
    assert!(script.contains("user_2"));

    let (status, denied, _) = call(&app, "GET", "/api/users", None, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{denied}");

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
    let admin_id = me["id"].as_str().unwrap().to_string();
    let admin_name = me["display_name"].as_str().unwrap().to_string();
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "ada",
            "display_name": "Ada Lovelace",
            "password": "ada-secret-1",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let ada_id = created["id"].as_str().unwrap().to_string();
    assert!(created.get("password_hash").is_none());

    let (status, duplicate, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "Ada",
            "display_name": "Noch Ada",
            "password": "ada-secret-1",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{duplicate}");
    assert_eq!(duplicate["error"], "that_username_is_already_used");

    let (status, member, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(&team),
        Some(json!({"username": "ada", "role": "operator"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{member}");

    let (status, ada_me, ada_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "ada", "password": "ada-secret-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ada_me}");
    let ada_cookie = ada_cookie.expect("ada session");
    let (status, forbidden, _) = call(
        &app,
        "GET",
        "/api/users",
        Some(&ada_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{forbidden}");

    let (status, users, _) = call(&app, "GET", "/api/users", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{users}");
    let names: Vec<_> = users
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["username"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"admin"), "{users}");
    assert!(names.contains(&"ada"), "{users}");
    assert!(users
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item.get("password_hash").is_none()));

    let (status, detail, _) = call(
        &app,
        "GET",
        &format!("/api/users/{ada_id}"),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["memberships"][0]["role"], "operator");
    assert_eq!(detail["memberships"][0]["source"], "manual");
    assert!(detail.get("password_hash").is_none());

    let (status, promoted, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(&team),
        Some(json!({"username": "ada", "role": "owner"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{promoted}");
    let (status, detail, _) = call(
        &app,
        "GET",
        &format!("/api/users/{ada_id}"),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["memberships"][0]["role"], "owner");

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{ada_id}"),
        Some(&cookie),
        None,
        Some(json!({
            "display_name": "Ada L.",
            "is_admin": false,
            "disabled": false,
            "password": "ada-secret-2"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["display_name"], "Ada L.");
    assert!(updated.get("password_hash").is_none());

    let (status, old_login, _) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "ada", "password": "ada-secret-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{old_login}");
    let (status, stale, _) = call(&app, "GET", "/api/me", Some(&ada_cookie), None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{stale}");

    let (status, fresh, fresh_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "ada", "password": "ada-secret-2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{fresh}");
    let fresh_cookie = fresh_cookie.expect("new ada session");

    let (status, disabled, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{ada_id}"),
        Some(&cookie),
        None,
        Some(json!({
            "display_name": "Ada L.",
            "is_admin": false,
            "disabled": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{disabled}");
    assert_eq!(disabled["disabled"], true);
    let (status, blocked, _) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "ada", "password": "ada-secret-2"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{blocked}");
    assert_eq!(blocked["error"], "invalid_username_or_password");
    let (status, ended, _) = call(&app, "GET", "/api/me", Some(&fresh_cookie), None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{ended}");

    let (status, enabled, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{ada_id}"),
        Some(&cookie),
        None,
        Some(json!({
            "display_name": "Ada L.",
            "is_admin": false,
            "disabled": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{enabled}");
    let (status, back, _) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "ada", "password": "ada-secret-2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{back}");

    let (status, self_off, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{admin_id}"),
        Some(&cookie),
        None,
        Some(json!({
            "display_name": admin_name,
            "is_admin": true,
            "disabled": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{self_off}");
    assert_eq!(self_off["error"], "you_cannot_disable_your_own_account");
    let (status, demote, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{admin_id}"),
        Some(&cookie),
        None,
        Some(json!({
            "display_name": admin_name,
            "is_admin": false,
            "disabled": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{demote}");
    assert_eq!(demote["error"], "the_last_administrator_stays");

    let (status, own, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{admin_id}"),
        Some(&cookie),
        None,
        Some(json!({
            "display_name": admin_name,
            "is_admin": true,
            "disabled": false,
            "password": "admin-kept-session"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{own}");
    assert!(own.get("password_hash").is_none());
    let (status, still, _) = call(&app, "GET", "/api/me", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{still}");

    let (status, self_delete, _) = call(
        &app,
        "DELETE",
        &format!("/api/users/{admin_id}"),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{self_delete}");
    assert_eq!(self_delete["error"], "you_cannot_delete_your_own_account");

    let (status, removed, _) = call(
        &app,
        "DELETE",
        &format!("/api/users/{ada_id}"),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{removed}");
    let (status, missing, _) = call(
        &app,
        "GET",
        &format!("/api/users/{ada_id}"),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn signed_in_user_can_read_their_own_account() {
    let harness = TestApp::boot("account", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/features/legacy.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
    assert!(script.contains("my_account"));
    assert!(script.contains("#/account"));
    assert!(script.contains("data-lang-select"));
    assert!(script.contains("GER"));
    assert!(script.contains("ENG"));
    let (status, style, _) = call(&app, "GET", "/app.css", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let style = style.as_str().unwrap_or("");
    assert!(style.contains("[hidden]"));
    assert!(style.contains("display: none !important"));
    assert!(style.contains("color: var(--ink)"));

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
    assert_eq!(me["auth_source"], "local");
    assert!(me["email"].is_null());
    assert_eq!(me["memberships"][0]["role"], "owner");
    assert_eq!(me["memberships"][0]["organization_name"], "Plattform");
    assert!(me.get("password_hash").is_none());

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "nia",
            "display_name": "Nia",
            "password": "nia-secret-1",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let (status, _member, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(&team),
        Some(json!({"username": "nia", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, nia, nia_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "nia", "password": "nia-secret-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{nia}");
    let nia_cookie = nia_cookie.expect("nia session");
    assert_eq!(nia["auth_source"], "local");
    assert_eq!(nia["memberships"][0]["role"], "viewer");
    assert_eq!(nia["memberships"][0]["source"], "manual");
    assert_eq!(nia["memberships"][0]["team_name"], "Plattform");
    assert_eq!(nia["memberships"][0]["organization_name"], "Plattform");
    let (status, forbidden, _) =
        call(&app, "GET", "/api/users", Some(&nia_cookie), None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{forbidden}");

    expect_unknown_field(
        &app,
        "PUT",
        "/api/me",
        Some(&nia_cookie),
        None,
        &json!({
            "display_name": "Nia Klein",
            "email": "nia@example.com",
            "is_admin": true
        }),
        "is_admin",
    )
    .await;
    let (status, saved, _) = call(
        &app,
        "PUT",
        "/api/me",
        Some(&nia_cookie),
        None,
        Some(json!({
            "display_name": "Nia Klein",
            "email": "nia@example.com"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["display_name"], "Nia Klein");
    assert_eq!(saved["email"], "nia@example.com");
    assert_eq!(saved["is_admin"], false);

    let (status, broken, _) = call(
        &app,
        "PUT",
        "/api/me",
        Some(&nia_cookie),
        None,
        Some(json!({"display_name": "Nia Klein", "email": "keine"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{broken}");
    assert_eq!(broken["error"], "email_is_invalid");

    let (status, changed, _) = call(
        &app,
        "PUT",
        "/api/me",
        Some(&nia_cookie),
        None,
        Some(json!({
            "display_name": "Nia Klein",
            "email": "nia@example.com",
            "password": "nia-secret-2"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert!(changed.get("password_hash").is_none());
    let (status, still, _) = call(&app, "GET", "/api/me", Some(&nia_cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{still}");
    assert_eq!(still["email"], "nia@example.com");
    let (status, old_login, _) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "nia", "password": "nia-secret-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{old_login}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_rejected_sign_in_uses_one_message_and_keeps_the_username() {
    let harness = TestApp::boot("signin", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/features/legacy.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
    assert!(script.contains("invalid_username_or_password"));
    assert!(script.contains("value=\"${esc(username || \"\")}\""));
    assert!(script.contains("[name=password]"));
    assert!(script.contains("password.focus()"));

    let (status, missing, _) = call(&app, "GET", "/api/me", None, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{missing}");
    assert_eq!(missing["error"], "sign_in_is_required");

    for (name, password) in [
        ("admin", "wrong-password"),
        ("nobody", "wrong-password"),
        ("admin", ""),
    ] {
        let (status, rejected, _) = call(
            &app,
            "POST",
            "/api/login",
            None,
            None,
            Some(json!({ "username": name, "password": password })),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{name} {rejected}");
        assert_eq!(rejected["error"], "invalid_username_or_password", "{name}");
    }

    let (status, me, _) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn api_tokens_authenticate_and_revoke() {
    let harness = TestApp::boot("tokens", Options::default());
    let state = harness.state.clone();
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
    let cookie = cookie.expect("admin cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, viewer, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "viewer",
            "display_name": "Viewer",
            "password": "viewer-pass",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{viewer}");
    let (status, operator, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "operator",
            "display_name": "Operator",
            "password": "operator-pass",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{operator}");
    let operator_id = operator["id"].as_str().unwrap().to_string();
    for (username, role) in [("viewer", "viewer"), ("operator", "operator")] {
        let (status, _, _) = call(
            &app,
            "POST",
            &format!("/api/teams/{team}/members"),
            Some(&cookie),
            Some(&team),
            Some(json!({"username": username, "role": role})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (_, _, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "viewer", "password": "viewer-pass"})),
    )
    .await;
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    let (_, _, operator_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "operator", "password": "operator-pass"})),
    )
    .await;
    let operator_cookie = operator_cookie.expect("operator cookie");

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Token job",
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": "true"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    let (status, blank, _) = call(
        &app,
        "POST",
        "/api/me/tokens",
        Some(&viewer_cookie),
        None,
        Some(json!({"name": "  "})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(blank["error"], "name_is_missing_or_too_long");
    let (status, long, _) = call(
        &app,
        "POST",
        "/api/me/tokens",
        Some(&viewer_cookie),
        None,
        Some(json!({"name": "a".repeat(121)})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(long["error"], "name_is_missing_or_too_long");

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/me/tokens",
        Some(&viewer_cookie),
        None,
        Some(json!({"name": "viewer-tool"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["name"], "viewer-tool");
    let viewer_secret = created["secret"].as_str().unwrap().to_string();
    let viewer_token_id = created["id"].as_str().unwrap().to_string();
    assert!(viewer_secret.starts_with("koro_"));
    assert!(created["prefix"]
        .as_str()
        .is_some_and(|prefix| prefix.starts_with("koro_")));

    let (status, admin_tokens, _) =
        call(&app, "GET", "/api/me/tokens", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!admin_tokens.to_string().contains(&viewer_token_id));

    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/me/tokens",
        Some(&viewer_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let list = listed.as_array().unwrap();
    assert!(list.iter().any(|item| item["id"] == viewer_token_id));
    for item in list {
        assert!(item.get("secret").is_none());
        assert!(item.get("token_hash").is_none());
        assert!(item.get("prefix").is_some());
        assert!(item.get("created_at").is_some());
    }
    assert!(!listed.to_string().contains(&viewer_secret));

    let (status, bearer_me) = call_bearer(&app, "GET", "/api/me", &viewer_secret, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bearer_me["username"], "viewer");
    assert!(!bearer_me.to_string().contains(&viewer_secret));
    assert!(bearer_me.get("tokens").is_none());
    let (status, bearer_tokens) =
        call_bearer(&app, "GET", "/api/me/tokens", &viewer_secret, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!bearer_tokens.to_string().contains(&viewer_secret));
    let (status, team_templates) = call_bearer(
        &app,
        "GET",
        "/api/templates",
        &viewer_secret,
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{team_templates}");
    let (status, missing_team) =
        call_bearer(&app, "GET", "/api/templates", &viewer_secret, None, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(missing_team["error"], "x_team_id_is_missing");

    let (status, bytes, _) = call_raw(
        &app,
        "GET",
        "/api/me",
        Some(&viewer_cookie),
        &[("authorization", "Bearer koro_bad")],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let rejected: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(rejected["error"], "sign_in_is_required");
    let (status, bytes, _) = call_raw(
        &app,
        "GET",
        "/api/me",
        Some(&cookie),
        &[("authorization", &format!("Bearer {viewer_secret}"))],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let wins: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(wins["username"], "viewer");

    let (status, denied) = call_bearer(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        &viewer_secret,
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(denied["error"], "you_are_not_allowed_to_do_this");

    let (status, operator_token, _) = call(
        &app,
        "POST",
        "/api/me/tokens",
        Some(&operator_cookie),
        None,
        Some(json!({"name": "operator-tool"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let operator_secret = operator_token["secret"].as_str().unwrap().to_string();
    let operator_token_id = operator_token["id"].as_str().unwrap().to_string();
    let (status, launched) = call_bearer(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        &operator_secret,
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");

    let (status, _, _) = call(
        &app,
        "DELETE",
        &format!("/api/me/tokens/{viewer_token_id}"),
        Some(&viewer_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, revoked) = call_bearer(&app, "GET", "/api/me", &viewer_secret, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(revoked["error"], "sign_in_is_required");
    let (status, still_signed_in, _) =
        call(&app, "GET", "/api/me", Some(&viewer_cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{still_signed_in}");

    let (status, not_found, _) = call(
        &app,
        "DELETE",
        &format!("/api/me/tokens/{operator_token_id}"),
        Some(&viewer_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(not_found["error"], "not_found");
    let (status, unknown) = call_bearer(&app, "GET", "/api/me", "koro_unknown", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown["error"], "sign_in_is_required");
    let truncated = &operator_secret[..operator_secret.len() - 4];
    let (status, shortened) = call_bearer(&app, "GET", "/api/me", truncated, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(shortened["error"], "sign_in_is_required");

    let (status, _, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{operator_id}"),
        Some(&cookie),
        None,
        Some(json!({
            "display_name": "Operator",
            "is_admin": false,
            "disabled": true,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, disabled) =
        call_bearer(&app, "GET", "/api/me", &operator_secret, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(disabled["error"], "sign_in_is_required");
    let (status, _, _) = call(
        &app,
        "DELETE",
        &format!("/api/users/{operator_id}"),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    {
        let db = state.db().unwrap();
        let left: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM api_tokens WHERE user_id = ?1",
                [&operator_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn api_token_follows_the_local_runner_rule() {
    let dir = temp_dir("token-local");
    let mut cfg = config(&dir, &Options::default());
    cfg.local_jobs = LocalJobs::Admins;
    let state = koreograf::boot(cfg).unwrap();
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
    let cookie = cookie.expect("admin cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "operator",
            "display_name": "Operator",
            "password": "operator-pass",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(&team),
        Some(json!({"username": "operator", "role": "operator"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, _, operator_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "operator", "password": "operator-pass"})),
    )
    .await;
    let operator_cookie = operator_cookie.expect("operator cookie");

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Local token job",
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": "true"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    let (status, admin_token, _) = call(
        &app,
        "POST",
        "/api/me/tokens",
        Some(&cookie),
        None,
        Some(json!({"name": "admin-tool"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let admin_secret = admin_token["secret"].as_str().unwrap().to_string();
    let (status, operator_token, _) = call(
        &app,
        "POST",
        "/api/me/tokens",
        Some(&operator_cookie),
        None,
        Some(json!({"name": "operator-tool"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let operator_secret = operator_token["secret"].as_str().unwrap().to_string();

    let (status, refused) = call_bearer(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        &operator_secret,
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["error"], "local_runs_are_limited_to_administrators");
    let (status, launched) = call_bearer(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        &admin_secret,
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn session_endpoints_return_the_documented_shape() {
    let harness = TestApp::boot("session-shape", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, login, cookie) = call(
        &app,
        "POST",
        "/api/v1/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let cookie = cookie.expect("session cookie");
    let admin_id = login["id"].clone();
    let team_id = login["teams"][0]["id"].clone();
    let org_id = login["teams"][0]["organization_id"].clone();
    assert!(admin_id.is_string());
    assert!(team_id.is_string());
    assert!(org_id.is_string());
    let profile = json!({
        "auth_source": "local",
        "display_name": "Administrator",
        "email": null,
        "id": admin_id.clone(),
        "is_admin": true,
        "memberships": [{
            "organization_id": org_id.clone(),
            "organization_name": "Plattform",
            "role": "owner",
            "source": "manual",
            "team_id": team_id.clone(),
            "team_name": "Plattform"
        }],
        "teams": [{
            "description": "Sample team. An LDAP group can later use this group name.",
            "id": team_id.clone(),
            "ldap_group": null,
            "name": "Plattform",
            "organization_id": org_id.clone(),
            "organization_name": "Plattform",
            "role": "owner"
        }],
        "username": "admin"
    });
    assert_eq!(login, profile);

    let (status, me, _) = call(&app, "GET", "/api/v1/me", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me, profile);

    let (status, saved, _) = call(
        &app,
        "PUT",
        "/api/v1/me",
        Some(&cookie),
        None,
        Some(json!({"display_name": "Administrator", "email": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved, profile);

    let (status, token, _) = call(
        &app,
        "POST",
        "/api/v1/me/tokens",
        Some(&cookie),
        None,
        Some(json!({"name": "shape"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{token}");
    let token_id = token["id"].clone();
    assert!(token_id.is_string());
    assert!(token["created_at"].is_string());
    assert!(token["prefix"].is_string());
    assert!(token["secret"].is_string());
    assert_eq!(
        token,
        json!({
            "created_at": token["created_at"].clone(),
            "id": token_id.clone(),
            "last_used_at": null,
            "name": "shape",
            "prefix": token["prefix"].clone(),
            "secret": token["secret"].clone()
        })
    );

    let (status, tokens, _) =
        call(&app, "GET", "/api/v1/me/tokens", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    assert_eq!(
        tokens,
        json!([{
            "created_at": token["created_at"].clone(),
            "id": token_id.clone(),
            "last_used_at": null,
            "name": "shape",
            "prefix": token["prefix"].clone()
        }])
    );

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/v1/users",
        Some(&cookie),
        None,
        Some(json!({
            "username": "shape",
            "display_name": "Shape User",
            "password": "shape-secret-1",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let user_id = created["id"].clone();
    let created_at = created["created_at"].clone();
    assert!(user_id.is_string());
    assert!(created_at.is_string());
    let account = json!({
        "auth_source": "local",
        "created_at": created_at.clone(),
        "disabled": false,
        "display_name": "Shape User",
        "email": null,
        "id": user_id.clone(),
        "is_admin": false,
        "team_count": 0,
        "username": "shape"
    });
    assert_eq!(created, account);

    let (status, users, _) = call(&app, "GET", "/api/v1/users", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{users}");
    assert_eq!(
        users,
        json!([
            {
                "auth_source": "local",
                "created_at": users[0]["created_at"].clone(),
                "disabled": false,
                "display_name": "Administrator",
                "email": null,
                "id": admin_id.clone(),
                "is_admin": true,
                "team_count": 1,
                "username": "admin"
            },
            account.clone()
        ])
    );

    let (status, detail, _) = call(
        &app,
        "GET",
        &format!(
            "/api/v1/users/{}",
            user_id.as_str().expect("user id is a string")
        ),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(
        detail,
        json!({
            "auth_source": "local",
            "created_at": created_at.clone(),
            "disabled": false,
            "display_name": "Shape User",
            "email": null,
            "id": user_id.clone(),
            "is_admin": false,
            "memberships": [],
            "team_count": 0,
            "username": "shape"
        })
    );

    let team = team_id.as_str().expect("team id is a string");
    let (status, _, _) = call(
        &app,
        "POST",
        &format!("/api/v1/teams/{team}/members"),
        Some(&cookie),
        Some(team),
        Some(json!({"username": "shape", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, detail, _) = call(
        &app,
        "GET",
        &format!("/api/v1/users/{}", user_id.as_str().unwrap()),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(
        detail,
        json!({
            "auth_source": "local",
            "created_at": created_at.clone(),
            "disabled": false,
            "display_name": "Shape User",
            "email": null,
            "id": user_id.clone(),
            "is_admin": false,
            "memberships": [{
                "organization_id": org_id.clone(),
                "organization_name": "Plattform",
                "role": "viewer",
                "source": "manual",
                "team_id": team_id.clone(),
                "team_name": "Plattform"
            }],
            "team_count": 1,
            "username": "shape"
        })
    );

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/v1/users/{}", user_id.as_str().unwrap()),
        Some(&cookie),
        None,
        Some(json!({
            "display_name": "Shape User 2",
            "is_admin": false,
            "disabled": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(
        updated,
        json!({
            "auth_source": "local",
            "created_at": created_at,
            "disabled": false,
            "display_name": "Shape User 2",
            "email": null,
            "id": user_id.clone(),
            "is_admin": false,
            "memberships": [{
                "organization_id": org_id,
                "organization_name": "Plattform",
                "role": "viewer",
                "source": "manual",
                "team_id": team_id,
                "team_name": "Plattform"
            }],
            "team_count": 1,
            "username": "shape"
        })
    );

    let (status, _, _) = call(
        &app,
        "DELETE",
        &format!("/api/v1/me/tokens/{}", token_id.as_str().unwrap()),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = call(
        &app,
        "DELETE",
        &format!("/api/v1/users/{}", user_id.as_str().unwrap()),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let _ = std::fs::remove_dir_all(&dir);
}
