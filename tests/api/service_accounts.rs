use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::common::{
    call_bearer_team as call_bearer, call_cookie as call, mcp_initialize_body, mcp_request,
    mcp_session, sse_message, Options, TestApp,
};

async fn admin_login(app: &axum::Router) -> (String, String) {
    let (status, me, cookie) = call(
        app,
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
    (cookie, team)
}

async fn create_user(
    app: &axum::Router,
    cookie: &str,
    team: &str,
    username: &str,
    role: &str,
) -> String {
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
    let id = created["id"].as_str().unwrap().to_string();
    let (status, _, _) = call(
        app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(cookie),
        Some(team),
        Some(json!({"username": username, "role": role})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    id
}

async fn login_user(app: &axum::Router, username: &str) -> String {
    let (status, me, cookie) = call(
        app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": username, "password": format!("{username}-pass")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    cookie.expect("user cookie")
}

async fn create_service_account(
    app: &axum::Router,
    cookie: &str,
    username: &str,
    team: &str,
    role: &str,
) -> Value {
    let (status, created, _) = call(
        app,
        "POST",
        "/api/service-accounts",
        Some(cookie),
        Some(team),
        Some(json!({
            "username": username,
            "display_name": username,
            "teams": [{"team_id": team, "role": role}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created
}

async fn mint_token(
    app: &axum::Router,
    cookie: &str,
    account_id: &str,
    name: &str,
    scope: &str,
) -> (String, String) {
    let (status, created, _) = call(
        app,
        "POST",
        &format!("/api/service-accounts/{account_id}/tokens"),
        Some(cookie),
        None,
        Some(json!({"name": name, "scope": scope})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    (
        created["id"].as_str().unwrap().to_string(),
        created["secret"].as_str().unwrap().to_string(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_account_token_outlives_its_creator() {
    let harness = TestApp::boot("service-account-creator", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();
    let (admin_cookie, team) = admin_login(&app).await;
    let owner_id = create_user(&app, &admin_cookie, &team, "owner", "owner").await;
    let owner_cookie = login_user(&app, "owner").await;

    let account = create_service_account(&app, &owner_cookie, "ci-deploy", &team, "operator").await;
    let account_id = account["id"].as_str().unwrap().to_string();
    assert_eq!(account["auth_source"], "service");
    assert_eq!(account["created_by"], owner_id);
    assert_eq!(account["created_by_username"], "owner");
    assert_eq!(account["memberships"][0]["role"], "operator");

    let (status, token, _) = call(
        &app,
        "POST",
        &format!("/api/service-accounts/{account_id}/tokens"),
        Some(&owner_cookie),
        None,
        Some(json!({"name": "pipeline"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{token}");
    assert_eq!(token["scope"], "write");
    assert!(token["expires_at"].as_str().is_some());
    let secret = token["secret"].as_str().unwrap().to_string();

    let (status, me) = call_bearer(&app, "GET", "/api/me", &secret, None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["username"], "ci-deploy");

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&owner_cookie),
        Some(&team),
        Some(json!({
            "name": "Service account job",
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": "true"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();
    let (status, launched) = call_bearer(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        &secret,
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");

    let (status, activity, _) = call(
        &app,
        "GET",
        "/api/activity?actor=ci-deploy&limit=100",
        Some(&owner_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{activity}");
    let items = activity["items"].as_array().unwrap();
    assert!(
        items.iter().any(|item| item["action"] == "launch"
            && item["object_type"] == "job"
            && item["actor_created_by"] == "owner"),
        "{activity}"
    );

    let (status, _, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{owner_id}"),
        Some(&admin_cookie),
        None,
        Some(json!({"display_name": "owner", "is_admin": false, "disabled": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, still) = call_bearer(&app, "GET", "/api/me", &secret, None, None).await;
    assert_eq!(status, StatusCode::OK, "{still}");
    assert_eq!(still["username"], "ci-deploy");

    let (status, _, _) = call(
        &app,
        "DELETE",
        &format!("/api/users/{owner_id}"),
        Some(&admin_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, still) = call_bearer(&app, "GET", "/api/me", &secret, None, None).await;
    assert_eq!(status, StatusCode::OK, "{still}");

    let (status, detail, _) = call(
        &app,
        "GET",
        &format!("/api/service-accounts/{account_id}"),
        Some(&admin_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert!(detail["created_by"].is_null(), "{detail}");
    assert!(detail["created_by_username"].is_null(), "{detail}");
    assert_eq!(detail["disabled"], false, "{detail}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_account_cannot_sign_in_or_mint_tokens() {
    let harness = TestApp::boot("service-account-signin", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();
    let (admin_cookie, team) = admin_login(&app).await;
    let account = create_service_account(&app, &admin_cookie, "ci-bot", &team, "operator").await;
    let account_id = account["id"].as_str().unwrap().to_string();
    let (_, secret) = mint_token(&app, &admin_cookie, &account_id, "pipeline", "write").await;

    let (status, rejected, _) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "ci-bot", "password": "anything"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{rejected}");
    assert_eq!(rejected["error"], "invalid_username_or_password");

    for (method, path) in [
        ("GET", "/api/me/tokens"),
        ("POST", "/api/me/tokens"),
        ("DELETE", "/api/me/tokens/unknown"),
        ("GET", "/api/service-accounts"),
    ] {
        let body = if method == "POST" || method == "DELETE" {
            Some(json!({"name": "nested"}))
        } else {
            None
        };
        let (status, refused) = call_bearer(&app, method, path, &secret, None, body).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {path} reached a service-account token: {refused}"
        );
    }
    let (status, refused) = call_bearer(
        &app,
        "POST",
        &format!("/api/service-accounts/{account_id}/tokens"),
        &secret,
        None,
        Some(json!({"name": "nested"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a service-account token minted another token: {refused}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn operators_cannot_create_service_accounts_or_their_tokens() {
    let harness = TestApp::boot("service-account-roles", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();
    let (admin_cookie, team) = admin_login(&app).await;
    let _owner_id = create_user(&app, &admin_cookie, &team, "owner", "owner").await;
    let _operator_id = create_user(&app, &admin_cookie, &team, "operator", "operator").await;
    let owner_cookie = login_user(&app, "owner").await;
    let operator_cookie = login_user(&app, "operator").await;

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/service-accounts",
        Some(&operator_cookie),
        None,
        Some(json!({
            "username": "ci-operator",
            "display_name": "CI operator",
            "teams": [{"team_id": team, "role": "viewer"}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let account = create_service_account(&app, &owner_cookie, "ci-owned", &team, "viewer").await;
    let account_id = account["id"].as_str().unwrap().to_string();
    let (status, _, _) = call(
        &app,
        "POST",
        &format!("/api/service-accounts/{account_id}/tokens"),
        Some(&operator_cookie),
        None,
        Some(json!({"name": "operator-token"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = call(
        &app,
        "GET",
        &format!("/api/service-accounts/{account_id}/tokens"),
        Some(&operator_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, overdrawn, _) = call(
        &app,
        "POST",
        "/api/service-accounts",
        Some(&owner_cookie),
        None,
        Some(json!({
            "username": "ci-admin",
            "display_name": "CI admin",
            "is_admin": true,
            "teams": [{"team_id": team, "role": "owner"}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{overdrawn}");
    assert_eq!(
        overdrawn["error"],
        "a_service_account_is_never_an_administrator"
    );

    let (status, promoted, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{account_id}"),
        Some(&admin_cookie),
        None,
        Some(json!({"display_name": "ci-owned", "is_admin": true, "disabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{promoted}");
    assert_eq!(
        promoted["error"],
        "a_service_account_is_never_an_administrator"
    );

    let (status, no_team, _) = call(
        &app,
        "POST",
        "/api/service-accounts",
        Some(&owner_cookie),
        None,
        Some(json!({"username": "ci-empty", "display_name": "CI empty", "teams": []})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{no_team}");
    assert_eq!(
        no_team["error"],
        "a_service_account_needs_at_least_one_team"
    );

    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/service-accounts",
        Some(&operator_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(listed.as_array().unwrap().is_empty(), "{listed}");

    let (status, other, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&admin_cookie),
        None,
        Some(json!({"name": "Other team"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_team = other["id"].as_str().unwrap().to_string();
    let (status, _, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{other_team}/members"),
        Some(&admin_cookie),
        Some(&other_team),
        Some(json!({"username": "owner", "role": "operator"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, outside, _) = call(
        &app,
        "POST",
        "/api/service-accounts",
        Some(&owner_cookie),
        None,
        Some(json!({
            "username": "ci-outside",
            "display_name": "CI outside",
            "teams": [{"team_id": other_team, "role": "viewer"}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{outside}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_service_token_uses_safe_routes_and_mcp_read_tools() {
    let harness = TestApp::boot("service-account-read", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();
    let (admin_cookie, team) = admin_login(&app).await;
    let account = create_service_account(&app, &admin_cookie, "ci-read", &team, "operator").await;
    let account_id = account["id"].as_str().unwrap().to_string();
    let (_, secret) = mint_token(&app, &admin_cookie, &account_id, "read-only", "read").await;

    let (status, templates) =
        call_bearer(&app, "GET", "/api/templates", &secret, Some(&team), None).await;
    assert_eq!(status, StatusCode::OK, "{templates}");

    let (status, refused) = call_bearer(
        &app,
        "POST",
        "/api/templates",
        &secret,
        Some(&team),
        Some(
            json!({"name": "Nope", "tool": "shell", "entrypoint": "", "spec": {"script": "true"}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(refused["error"], "token_is_read_only");

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&admin_cookie),
        Some(&team),
        Some(json!({
            "name": "Read token job",
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": "true"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    let session = mcp_session(&app, &secret).await;
    assert!(!session.is_empty());
    let (status, bytes, _) = mcp_request(
        &app,
        Some(&secret),
        Some(&session),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": "whoami", "arguments": {"team_id": team}}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let whoami = sse_message(&bytes);
    assert_eq!(whoami["result"]["isError"], false, "{whoami}");
    assert!(whoami["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .contains("ci-read"));

    let jobs_before = harness.sql_i64("SELECT COUNT(*) FROM jobs").await;
    let (status, bytes, _) = mcp_request(
        &app,
        Some(&secret),
        Some(&session),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "launch_template",
                "arguments": {"id": template_id, "extra_vars": {}, "team_id": team}
            }
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let refused = sse_message(&bytes);
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    assert!(refused["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .contains("token_is_read_only"));
    let jobs_after = harness.sql_i64("SELECT COUNT(*) FROM jobs").await;
    assert_eq!(
        jobs_after, jobs_before,
        "a read service token started a job"
    );

    let (status, bytes, _) = mcp_request(&app, Some(&secret), None, mcp_initialize_body()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_service_token_works_over_mcp_stdio_too() {
    use std::process::Stdio;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::process::{ChildStdin, ChildStdout, Command};

    let harness = TestApp::boot("service-account-stdio", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();
    let (admin_cookie, team) = admin_login(&app).await;
    let account = create_service_account(&app, &admin_cookie, "ci-stdio", &team, "operator").await;
    let account_id = account["id"].as_str().unwrap().to_string();
    let (_, secret) = mint_token(&app, &admin_cookie, &account_id, "stdio-read", "read").await;

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&admin_cookie),
        Some(&team),
        Some(json!({
            "name": "Stdio read job",
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": "true"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let served = app.clone();
    let server = tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });

    let mut child = Command::new(env!("CARGO_BIN_EXE_koreograf"))
        .arg("mcp")
        .env("KOREOGRAF_URL", format!("http://{address}"))
        .env("KOREOGRAF_TOKEN", &secret)
        .env_remove("KOREOGRAF_TEAM_ID")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn koreograf mcp");
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();

    async fn send(stdin: &mut ChildStdin, message: Value) {
        let mut line = serde_json::to_vec(&message).unwrap();
        line.push(b'\n');
        stdin.write_all(&line).await.unwrap();
        stdin.flush().await.unwrap();
    }
    async fn receive(lines: &mut tokio::io::Lines<BufReader<ChildStdout>>, id: u64) -> Value {
        loop {
            let line = tokio::time::timeout(std::time::Duration::from_secs(20), lines.next_line())
                .await
                .expect("mcp stdio response timed out")
                .expect("mcp stdio read")
                .expect("mcp stdio closed");
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                if value["id"] == id {
                    return value;
                }
            }
        }
    }

    send(&mut stdin, mcp_initialize_body()).await;
    let initialized = receive(&mut lines, 1).await;
    assert!(initialized.get("result").is_some(), "{initialized}");
    send(
        &mut stdin,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": "whoami", "arguments": {"team_id": team}}
        }),
    )
    .await;
    let whoami = receive(&mut lines, 2).await;
    assert_eq!(whoami["result"]["isError"], false, "{whoami}");
    assert!(whoami["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .contains("ci-stdio"));

    let jobs_before = harness.sql_i64("SELECT COUNT(*) FROM jobs").await;
    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "launch_template",
                "arguments": {"id": template_id, "extra_vars": {}, "team_id": team}
            }
        }),
    )
    .await;
    let refused = receive(&mut lines, 3).await;
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    assert!(refused["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .contains("token_is_read_only"));
    assert_eq!(
        harness.sql_i64("SELECT COUNT(*) FROM jobs").await,
        jobs_before,
        "a refused stdio write started a job"
    );

    let _ = child.kill().await;
    let _ = child.wait().await;
    server.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn disabling_and_deleting_a_service_account_stops_its_tokens() {
    let harness = TestApp::boot("service-account-disable", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();
    let (admin_cookie, team) = admin_login(&app).await;
    let account = create_service_account(&app, &admin_cookie, "ci-worker", &team, "operator").await;
    let account_id = account["id"].as_str().unwrap().to_string();
    let (_, secret) = mint_token(&app, &admin_cookie, &account_id, "worker", "write").await;

    let (status, personal, _) = call(
        &app,
        "POST",
        "/api/me/tokens",
        Some(&admin_cookie),
        None,
        Some(json!({"name": "human-token"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{personal}");
    let personal_secret = personal["secret"].as_str().unwrap().to_string();

    let (status, _, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{account_id}"),
        Some(&admin_cookie),
        None,
        Some(json!({"display_name": "ci-worker", "is_admin": false, "disabled": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, denied) = call_bearer(&app, "GET", "/api/me", &secret, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{denied}");
    assert_eq!(denied["error"], "sign_in_is_required");
    let (status, still) = call_bearer(&app, "GET", "/api/me", &personal_secret, None, None).await;
    assert_eq!(status, StatusCode::OK, "{still}");
    assert_eq!(still["username"], "admin");

    let (status, _, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{account_id}"),
        Some(&admin_cookie),
        None,
        Some(json!({"display_name": "ci-worker", "is_admin": false, "disabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call_bearer(&app, "GET", "/api/me", &secret, None, None).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, _) = call(
        &app,
        "DELETE",
        &format!("/api/service-accounts/{account_id}"),
        Some(&admin_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, gone) = call_bearer(&app, "GET", "/api/me", &secret, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{gone}");
    assert_eq!(
        harness
            .sql_i64(&format!(
                "SELECT COUNT(*) FROM api_tokens WHERE user_id = '{account_id}'"
            ))
            .await,
        0
    );
    assert_eq!(
        harness
            .sql_i64(&format!(
                "SELECT COUNT(*) FROM memberships WHERE user_id = '{account_id}'"
            ))
            .await,
        0
    );
    let (status, still) = call_bearer(&app, "GET", "/api/me", &personal_secret, None, None).await;
    assert_eq!(status, StatusCode::OK, "{still}");

    let _ = std::fs::remove_dir_all(&dir);
}
