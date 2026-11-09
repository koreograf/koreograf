use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration, SecondsFormat, Utc};
use http_body_util::BodyExt;
use koreograf::{Config, CookieSecure, LdapConfig, LocalJobs, RunnerMode};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
    call_team(app, method, uri, cookie, None, body).await
}

async fn call_team(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    team: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, Option<String>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    if let Some(team) = team {
        builder = builder.header("x-team-id", team);
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

async fn call_bearer(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
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
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).to_string()))
    };
    (status, parsed)
}

async fn postgres_session(
    url: &str,
    user_id: &str,
    username: &str,
    memberships: &[(&str, &str)],
    token: &str,
) {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("postgres connection");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO users
                (id, username, display_name, password_hash, auth_source, is_admin, created_at)
             VALUES ($1, $2, $3, NULL, 'local', 0,
                to_char(now(), 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'))",
            &[&user_id, &username, &username],
        )
        .await
        .expect("insert postgres user");
    for (team_id, role) in memberships {
        client
            .execute(
                "INSERT INTO memberships (team_id, user_id, role, source)
                 VALUES ($1, $2, $3, 'manual')",
                &[team_id, &user_id, role],
            )
            .await
            .expect("insert postgres membership");
    }
    client
        .execute(
            "INSERT INTO sessions (token, user_id, expires_at)
             VALUES ($1, $2, '9999-12-31T23:59:59Z')",
            &[&token, &user_id],
        )
        .await
        .expect("insert postgres session");
}

async fn postgres_job(
    url: &str,
    id: &str,
    team_id: &str,
    name: &str,
    status: &str,
    started_at: &str,
    finished_at: &str,
) {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("postgres connection");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO jobs (
                id, team_id, name, tool, status, extra_vars, launched_by, trigger,
                started_at, finished_at, created_at, parameters, failed_hosts
             ) VALUES ($1, $2, $3, 'shell', $4, '{}', 'test', 'manual', $5, $6, $5, '{}', '[]')",
            &[&id, &team_id, &name, &status, &started_at, &finished_at],
        )
        .await
        .expect("insert postgres job");
}

struct PostgresJob<'a> {
    id: &'a str,
    name: &'a str,
    status: &'a str,
    trigger: &'a str,
    created_at: &'a str,
}

async fn postgres_job_record(
    url: &str,
    team_id: &str,
    template_id: Option<&str>,
    job: PostgresJob<'_>,
) {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("postgres connection");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO jobs (
                id, team_id, template_id, name, tool, status, extra_vars, launched_by, trigger,
                started_at, finished_at, created_at, parameters, failed_hosts
             ) VALUES ($1, $2, $3, $4, 'shell', $5, '{}', 'admin', $6, NULL, NULL, $7, '{}',
                '[]')",
            &[
                &job.id,
                &team_id,
                &template_id,
                &job.name,
                &job.status,
                &job.trigger,
                &job.created_at,
            ],
        )
        .await
        .expect("insert postgres job");
}

async fn postgres_job_lines(url: &str, job_id: &str, prefix: &str, count: i64) {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("postgres connection");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO job_events (job_id, ts, stream, line)
             SELECT $1, '2026-01-01T00:00:00Z', 'stdout', $2 || '-' || g
             FROM generate_series(1, $3::bigint) AS g",
            &[&job_id, &prefix, &count],
        )
        .await
        .expect("insert postgres job events");
}

async fn postgres_job_bulk_lines(url: &str, job_id: &str, count: i64, bytes: i32) {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("postgres connection");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO job_events (job_id, ts, stream, line)
             SELECT $1, '2026-01-01T00:00:00Z', 'stdout', repeat('x', $2)
             FROM generate_series(1, $3::bigint)",
            &[&job_id, &bytes, &count],
        )
        .await
        .expect("insert postgres bulk job events");
}

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("git is not installed");
    assert!(status.success(), "git {args:?} in {}", dir.display());
}

static POSTGRES_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

async fn postgres_guard() -> tokio::sync::MutexGuard<'static, ()> {
    POSTGRES_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

const PORTABLE_TABLES: &[&str] = &[
    "users",
    "organizations",
    "teams",
    "credentials",
    "projects",
    "inventories",
    "environments",
    "templates",
    "memberships",
    "inventory_sources",
    "template_credentials",
    "workflows",
    "workflow_jobs",
    "jobs",
    "job_events",
    "workflow_nodes",
    "hooks",
    "hook_deliveries",
    "project_teams",
    "settings",
    "seed_state",
    "schedules",
    "api_tokens",
    "sessions",
];

async fn postgres_client(url: &str) -> tokio_postgres::Client {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("postgres connection");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

fn postgres_test_url() -> Option<String> {
    let Ok(url) = std::env::var("KOREOGRAF_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is not set");
        return None;
    };
    if url.trim().is_empty() {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is empty");
        return None;
    }
    Some(url)
}

fn database_url(base: &str, name: &str) -> String {
    let mut url = url::Url::parse(base).expect("postgres url");
    url.set_path(&format!("/{name}"));
    url.to_string()
}

async fn create_database(base: &str, name: &str) -> String {
    let client = postgres_client(base).await;
    client
        .batch_execute(&format!("CREATE DATABASE \"{name}\""))
        .await
        .expect("create database");
    database_url(base, name)
}

async fn drop_database(base: &str, name: &str) {
    let client = postgres_client(base).await;
    let _ = client
        .batch_execute(&format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"))
        .await;
}

async fn call_raw(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Vec<u8>>,
) -> (StatusCode, Vec<u8>, Option<String>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder
        .body(Body::from(body.unwrap_or_default()))
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::to_string);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes()
        .to_vec();
    (status, bytes, cookie)
}

fn zip_entries(bytes: &[u8]) -> Vec<String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).expect("zip archive");
    (0..archive.len())
        .map(|index| {
            archive
                .by_index(index)
                .expect("zip entry")
                .name()
                .expect("zip name")
                .to_string()
        })
        .collect()
}

fn migrate_command(data_dir: &Path, target: &str) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_koreograf"))
        .args(["db", "migrate", "--to", target])
        .env("KOREOGRAF_DATA", data_dir)
        .env_remove("KOREOGRAF_DATABASE_URL")
        .env_remove("KOREOGRAF_DATABASE_PASSWORD_FILE")
        .env_remove("KOREOGRAF_DATABASE_CA_FILE")
        .output()
        .expect("run db migrate")
}

fn sqlite_counts(path: &Path) -> Vec<(&'static str, i64)> {
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("sqlite source");
    PORTABLE_TABLES
        .iter()
        .map(|table| {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                    row.get(0)
                })
                .expect("source count");
            (*table, count)
        })
        .collect()
}

fn seed_sqlite_source(dir: &Path) -> std::sync::Arc<koreograf::AppState> {
    let state = koreograf::boot(config(dir)).expect("sqlite boot");
    {
        let conn = state.db().expect("sqlite database");
        for line in ["first", "second"] {
            conn.execute(
                "INSERT INTO job_events (job_id, ts, stream, line)
                 VALUES ('job-copy', '2026-01-01T00:00:00Z', 'stdout', ?1)",
                [line],
            )
            .expect("job event");
        }
    }
    state
}

#[tokio::test]
async fn postgres_serves_workflows_hooks_and_project_sync() {
    let Ok(url) = std::env::var("KOREOGRAF_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is not set");
        return;
    };
    if url.trim().is_empty() {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is empty");
        return;
    }
    let _guard = postgres_guard().await;
    let database = koreograf::parse_database_config(&url, None, None).expect("postgres url");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-postgres-workflows-{nanos}"));
    let origin = std::env::temp_dir().join(format!("koreograf-postgres-origin-{nanos}"));
    std::fs::create_dir_all(&origin).expect("origin");
    let mut cfg = config(&dir);
    cfg.admin_user = format!("wfadmin{nanos}");
    cfg.project_roots.push(origin.clone());
    let state = koreograf::boot_with(cfg, database).expect("boot");
    assert_eq!(state.engine(), "postgresql");
    let app = koreograf::router(state);

    let (status, _, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({
            "username": format!("wfadmin{nanos}"),
            "password": "test-admin-pass"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("session cookie");

    let (status, org, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        Some(json!({
            "name": format!("Workflow org {nanos}"),
            "description": "Postgres workflow organization"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org}");
    let org_id = org["id"].as_str().expect("organization id").to_string();

    let (status, team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(json!({
            "name": format!("Workflow team {nanos}"),
            "description": "Postgres workflow team",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{team}");
    let team_id = team["id"].as_str().expect("team id").to_string();

    let (status, template, _) = call_team(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Workflow template {nanos}"),
            "description": "Postgres workflow template",
            "tool": "ansible",
            "entrypoint": "site.yml",
            "no_environment": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().expect("template id").to_string();

    let workflow_body = |name: &str, graph: Value| {
        json!({
            "name": name,
            "description": "Postgres workflow",
            "graph": graph
        })
    };

    let (status, workflow, _) = call_team(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team_id),
        Some(workflow_body(
            &format!("Workflow {nanos}"),
            json!({
                "nodes": [{
                    "id": "start",
                    "template_id": template_id,
                    "name": "Start"
                }],
                "edges": []
            }),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let workflow_id = workflow["id"].as_str().expect("workflow id").to_string();
    assert_eq!(workflow["graph"]["nodes"].as_array().map(Vec::len), Some(1));

    let (status, workflows, _) = call_team(
        &app,
        "GET",
        "/api/workflows",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(workflows
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == workflow_id)));

    let (status, fetched, _) = call_team(
        &app,
        "GET",
        &format!("/api/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["name"], format!("Workflow {nanos}"));

    let (status, updated, _) = call_team(
        &app,
        "PUT",
        &format!("/api/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(workflow_body(
            &format!("Workflow {nanos} renamed"),
            json!({
                "nodes": [
                    {"id": "start", "template_id": template_id, "name": "Start"},
                    {"id": "finish", "template_id": template_id, "name": "Finish"}
                ],
                "edges": [{"from": "start", "to": "finish", "when": "success"}]
            }),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["graph"]["edges"].as_array().map(Vec::len), Some(1));

    let (status, cycle, _) = call_team(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team_id),
        Some(workflow_body(
            &format!("Cycle {nanos}"),
            json!({
                "nodes": [
                    {"id": "a", "template_id": template_id, "name": "A"},
                    {"id": "b", "template_id": template_id, "name": "B"}
                ],
                "edges": [
                    {"from": "a", "to": "b", "when": "success"},
                    {"from": "b", "to": "a", "when": "success"}
                ]
            }),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(cycle["error"], "the_workflow_contains_a_cycle");

    let (status, missing, _) = call_team(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team_id),
        Some(workflow_body(
            &format!("Missing template {nanos}"),
            json!({
                "nodes": [{
                    "id": "start",
                    "template_id": format!("missing-{nanos}"),
                    "name": "Missing"
                }],
                "edges": []
            }),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(missing["error"], "template.missing_named");

    let (status, hook, _) = call_team(
        &app,
        "POST",
        "/api/hooks",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Workflow hook {nanos}"),
            "match_alertname": "HighCPU",
            "match_status": "firing",
            "template_id": template_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");
    let hook_id = hook["id"].as_str().expect("hook id").to_string();
    let token = hook["token"].as_str().expect("hook token").to_string();

    let (status, wrong_token, _) = call(
        &app,
        "POST",
        &format!("/hooks/{hook_id}/wrong-token"),
        None,
        Some(json!({
            "status": "firing",
            "alerts": [{"status": "firing", "labels": {"alertname": "HighCPU"}}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong_token["error"], "sign_in_is_required");

    let (status, ignored, _) = call(
        &app,
        "POST",
        &format!("/hooks/{hook_id}/{token}"),
        None,
        Some(json!({
            "status": "firing",
            "alerts": [{"status": "firing", "labels": {"alertname": "Other"}}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ignored}");
    assert_eq!(ignored["status"], "ignored");

    let (status, test_hook, _) = call_team(
        &app,
        "POST",
        "/api/hooks",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Resolved hook {nanos}"),
            "match_alertname": "HighCPU",
            "match_status": "resolved",
            "template_id": template_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{test_hook}");
    let test_hook_id = test_hook["id"].as_str().expect("test hook id").to_string();

    let (status, tested, _) = call(
        &app,
        "POST",
        &format!("/api/hooks/{test_hook_id}/test"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{tested}");
    assert_eq!(tested["status"], "ignored");

    let (status, deliveries, _) = call_team(
        &app,
        "GET",
        &format!("/api/hooks/{hook_id}/deliveries"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(deliveries.as_array().is_some_and(|items| items
        .iter()
        .any(|item| item["hook_id"] == hook_id && item["status"] == "ignored")));

    let nonmember_username = format!("wfnonmember{nanos}");
    let viewer_username = format!("wfviewer{nanos}");
    let nonmember_token = format!("wfnonmember-session-{nanos}");
    let viewer_token = format!("wfviewer-session-{nanos}");
    postgres_session(
        &url,
        &format!("user-{nanos}-wf-nonmember"),
        &nonmember_username,
        &[],
        &nonmember_token,
    )
    .await;
    postgres_session(
        &url,
        &format!("user-{nanos}-wf-viewer"),
        &viewer_username,
        &[(&team_id, "viewer")],
        &viewer_token,
    )
    .await;
    let nonmember_cookie = format!("koreograf_session={nonmember_token}");
    let viewer_cookie = format!("koreograf_session={viewer_token}");

    let (status, denied, _) = call_team(
        &app,
        "POST",
        "/api/workflows",
        Some(&viewer_cookie),
        Some(&team_id),
        Some(workflow_body(
            &format!("Denied workflow {nanos}"),
            json!({"nodes": [], "edges": []}),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");

    let (status, denied, _) = call(
        &app,
        "GET",
        &format!("/api/workflows/{workflow_id}"),
        Some(&nonmember_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");

    let (status, denied, _) = call(
        &app,
        "POST",
        &format!("/api/hooks/{test_hook_id}/test"),
        Some(&viewer_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");

    let (status, hooks, _) = call_team(
        &app,
        "GET",
        "/api/hooks",
        Some(&viewer_cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{hooks}");
    assert!(hooks
        .as_array()
        .is_some_and(|items| items.iter().all(|item| item["token"] == "")));

    git(&origin, &["init", "-b", "main"]);
    git(&origin, &["config", "user.email", "test@example.com"]);
    git(&origin, &["config", "user.name", "Koreograf Test"]);
    std::fs::write(origin.join("marker.txt"), "one\n").expect("marker");
    git(&origin, &["add", "marker.txt"]);
    git(&origin, &["commit", "-m", "one"]);
    let source = format!("file://{}", origin.display());

    let (status, project, _) = call_team(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Sync project {nanos}"),
            "source_type": "git",
            "source": source
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().expect("project id").to_string();

    let (status, synced, _) = call_team(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/sync"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");

    let (status, stored, _) = call_team(
        &app,
        "GET",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored["last_sync_status"], "success");
    assert!(stored["last_sync_at"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert!(stored["last_sync_commit"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));

    let (status, broken, _) = call_team(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Broken sync {nanos}"),
            "source_type": "git",
            "source": format!("file:///tmp/koreograf-missing-{nanos}")
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{broken}");
    let broken_id = broken["id"]
        .as_str()
        .expect("broken project id")
        .to_string();

    let (status, failed, _) = call_team(
        &app,
        "POST",
        &format!("/api/projects/{broken_id}/sync"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{failed}");

    let (status, broken_stored, _) = call_team(
        &app,
        "GET",
        &format!("/api/projects/{broken_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(broken_stored["last_sync_status"], "failed");

    let (status, _, _) = call_team(
        &app,
        "GET",
        &format!("/api/projects/{project_id}"),
        Some(&viewer_cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, denied, _) = call_team(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/sync"),
        Some(&viewer_cookie),
        Some(&team_id),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call_team(
        &app,
        "GET",
        &format!("/api/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/hooks/{hook_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/hooks/{test_hook_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/projects/{broken_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let _ = std::fs::remove_dir_all(&origin);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn postgres_first_run_seeds_the_demo_and_curated_environments() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let name = format!("koreograf_seed_{nanos}");
    let target = create_database(&url, &name).await;
    let dir = std::env::temp_dir().join(format!("koreograf-postgres-seed-{nanos}"));
    let mut cfg = config(&dir);
    cfg.admin_user = format!("seedadmin{nanos}");
    let database = koreograf::parse_database_config(&target, None, None).expect("postgres url");

    let state = koreograf::boot_with(cfg.clone(), database.clone()).expect("first boot");
    assert_eq!(state.engine(), "postgresql");
    drop(state);

    let client = postgres_client(&target).await;
    let count = |sql: &'static str| {
        let client = &client;
        async move {
            client
                .query_one(sql, &[])
                .await
                .expect("row count")
                .get::<_, i64>(0)
        }
    };
    assert_eq!(count("SELECT COUNT(*) FROM teams").await, 1);
    assert_eq!(count("SELECT COUNT(*) FROM organizations").await, 1);
    assert_eq!(
        count("SELECT COUNT(*) FROM memberships WHERE role = 'owner'").await,
        1
    );
    assert_eq!(count("SELECT COUNT(*) FROM projects").await, 1);
    assert_eq!(count("SELECT COUNT(*) FROM inventories").await, 1);
    assert_eq!(count("SELECT COUNT(*) FROM inventory_sources").await, 1);
    assert_eq!(count("SELECT COUNT(*) FROM templates").await, 4);
    assert_eq!(count("SELECT COUNT(*) FROM workflows").await, 1);
    assert_eq!(count("SELECT COUNT(*) FROM hooks").await, 1);
    assert_eq!(count("SELECT COUNT(*) FROM environments").await, 4);
    assert_eq!(
        count("SELECT COUNT(*) FROM environments WHERE id LIKE 'seeded-%'").await,
        3
    );
    let team: String = client
        .query_one("SELECT name FROM teams", &[])
        .await
        .expect("team")
        .get(0);
    assert_eq!(team, "Plattform");
    let project: String = client
        .query_one("SELECT name FROM projects", &[])
        .await
        .expect("project")
        .get(0);
    assert_eq!(project, "Demo");
    let workflow: String = client
        .query_one("SELECT name FROM workflows", &[])
        .await
        .expect("workflow")
        .get(0);
    assert_eq!(workflow, "Self-Heal Ablauf");
    for (tool, id) in [
        ("ansible", "seeded-ansible"),
        ("opentofu", "seeded-opentofu"),
        ("shell", "seeded-shell"),
    ] {
        let value: String = client
            .query_one(
                "SELECT value FROM settings WHERE key = $1",
                &[&format!("default_environment_{tool}")],
            )
            .await
            .expect("default environment")
            .get(0);
        assert_eq!(value, id);
    }

    client
        .execute(
            "UPDATE environments SET name = 'Renamed by admin' WHERE id = 'seeded-ansible'",
            &[],
        )
        .await
        .expect("rename environment");
    client
        .execute("DELETE FROM environments WHERE id = 'seeded-opentofu'", &[])
        .await
        .expect("delete environment");

    let state = koreograf::boot_with(cfg, database).expect("second boot");
    assert_eq!(state.engine(), "postgresql");
    drop(state);

    let name: String = client
        .query_one(
            "SELECT name FROM environments WHERE id = 'seeded-ansible'",
            &[],
        )
        .await
        .expect("renamed environment")
        .get(0);
    assert_eq!(name, "Renamed by admin");
    assert_eq!(
        count("SELECT COUNT(*) FROM environments WHERE id = 'seeded-opentofu'").await,
        0
    );
    assert_eq!(count("SELECT COUNT(*) FROM environments").await, 3);
    assert_eq!(count("SELECT COUNT(*) FROM teams").await, 1);
    assert_eq!(count("SELECT COUNT(*) FROM projects").await, 1);
    assert_eq!(count("SELECT COUNT(*) FROM inventories").await, 1);
    assert_eq!(count("SELECT COUNT(*) FROM templates").await, 4);
    assert_eq!(count("SELECT COUNT(*) FROM workflows").await, 1);
    assert_eq!(count("SELECT COUNT(*) FROM hooks").await, 1);

    drop(client);
    drop_database(&url, &name).await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn postgres_serves_the_auth_and_resource_slice() {
    let Ok(url) = std::env::var("KOREOGRAF_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is not set");
        return;
    };
    if url.trim().is_empty() {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is empty");
        return;
    }
    let _guard = postgres_guard().await;
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

    let (status, _, _) = call(&app, "GET", "/api/teams", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);

    let (status, org, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        Some(json!({
            "name": format!("Org {nanos}"),
            "description": "Postgres organization"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org}");
    let org_id = org["id"].as_str().expect("organization id").to_string();

    let (status, created_org, _) = call(
        &app,
        "GET",
        &format!("/api/organizations/{org_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created_org["name"], format!("Org {nanos}"));

    let (status, updated_org, _) = call(
        &app,
        "PUT",
        &format!("/api/organizations/{org_id}"),
        Some(&cookie),
        Some(json!({
            "name": format!("Org {nanos} renamed"),
            "description": "Updated"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated_org["description"], "Updated");

    let (status, list, _) = call(&app, "GET", "/api/organizations", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(list.as_array().is_some_and(|items| !items.is_empty()));

    let (status, team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(json!({
            "name": format!("Team {nanos}"),
            "description": "Postgres team",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{team}");
    let team_id = team["id"].as_str().expect("team id").to_string();

    let (status, updated_team, _) = call(
        &app,
        "PUT",
        &format!("/api/teams/{team_id}"),
        Some(&cookie),
        Some(json!({
            "name": format!("Team {nanos} renamed"),
            "description": "Updated team",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated_team["description"], "Updated team");

    let (status, members, _) = call(
        &app,
        "GET",
        &format!("/api/teams/{team_id}/members"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(members.as_array().is_some_and(|items| !items.is_empty()));

    let (status, credential, _) = call_team(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Credential {nanos}"),
            "kind": "scm",
            "secret": {"username": "git", "password": "secret"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"]
        .as_str()
        .expect("credential id")
        .to_string();

    let (status, credentials, _) = call_team(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(credentials
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == credential_id)));

    let (status, opened, _) = call_team(
        &app,
        "GET",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(opened["kind"], "scm");

    let (status, saved, _) = call_team(
        &app,
        "PUT",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Credential {nanos} renamed"),
            "secret": {"password": "next-secret"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["name"], format!("Credential {nanos} renamed"));

    let project_source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples");
    let (status, project, _) = call_team(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Project {nanos}"),
            "source_type": "path",
            "source": project_source,
            "team_ids": [],
            "credential_id": credential_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().expect("project id").to_string();

    let (status, projects, _) = call_team(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(projects
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == project_id)));

    let (status, fetched, _) = call_team(
        &app,
        "GET",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["organization_id"], org_id);

    let (status, updated, _) = call_team(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({"name": format!("Project {nanos} renamed")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], format!("Project {nanos} renamed"));

    let (status, second_team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(json!({
            "name": format!("Second team {nanos}"),
            "description": "Postgres second team",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{second_team}");
    let second_team_id = second_team["id"]
        .as_str()
        .expect("second team id")
        .to_string();

    let nonmember_username = format!("nonmember{nanos}");
    let viewer_username = format!("viewer{nanos}");
    let nonmember_token = format!("nonmember-session-{nanos}");
    let viewer_token = format!("viewer-session-{nanos}");
    postgres_session(
        &url,
        &format!("user-{nanos}-nonmember"),
        &nonmember_username,
        &[(&second_team_id, "owner")],
        &nonmember_token,
    )
    .await;
    postgres_session(
        &url,
        &format!("user-{nanos}-viewer"),
        &viewer_username,
        &[(&team_id, "viewer")],
        &viewer_token,
    )
    .await;
    let nonmember_cookie = format!("koreograf_session={nonmember_token}");
    let viewer_cookie = format!("koreograf_session={viewer_token}");

    for (actor, actor_cookie) in [
        ("non-member", &nonmember_cookie),
        ("viewer", &viewer_cookie),
    ] {
        let (status, denied, _) = call_team(
            &app,
            "POST",
            "/api/credentials",
            Some(actor_cookie),
            Some(&team_id),
            Some(json!({
                "name": format!("Denied {actor}"),
                "kind": "scm",
                "secret": {"username": "git", "password": "secret"}
            })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{actor} create: {denied}");

        let (status, denied, _) = call_team(
            &app,
            "GET",
            &format!("/api/credentials/{credential_id}"),
            Some(actor_cookie),
            Some(&team_id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{actor} get: {denied}");

        let (status, denied, _) = call_team(
            &app,
            "DELETE",
            &format!("/api/projects/{project_id}"),
            Some(actor_cookie),
            Some(&team_id),
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{actor} project delete: {denied}"
        );
    }

    let (status, _, _) = call_team(
        &app,
        "GET",
        &format!("/api/credentials/{credential_id}"),
        Some(&nonmember_cookie),
        Some(&second_team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, conflict, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict["error"], "credential_is_in_use");

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, inventory, _) = call_team(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Inventory {nanos}"),
            "body": "[all]\nlocalhost ansible_connection=local\n"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().expect("inventory id").to_string();

    let (status, inventories, _) = call_team(
        &app,
        "GET",
        "/api/inventories",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(inventories
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == inventory_id)));

    let (status, updated, _) = call_team(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Inventory {nanos} renamed"),
            "body": "[all]\nlocalhost ansible_connection=local\n[web]\nweb1\n"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["host_count"], 2);

    let (status, with_source, _) = call_team(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "kind": "ini",
            "name": "extra",
            "body": "[db]\ndb1\n"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{with_source}");
    let source_id = with_source["sources"]
        .as_array()
        .and_then(|items| items.last())
        .and_then(|item| item["id"].as_str())
        .expect("source id")
        .to_string();

    let (status, source, _) = call_team(
        &app,
        "GET",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(source["kind"], "ini");

    let (status, updated_source, _) = call_team(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({"name": "extra renamed", "body": "[db]\ndb1\ndb2\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated_source}");
    assert_eq!(updated_source["name"], format!("Inventory {nanos} renamed"));

    let (status, after_delete, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after_delete["source_count"], 1);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, environment, _) = call_team(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Environment {nanos}"),
            "image": "quay.io/example/ee:1",
            "tool": "ansible",
            "pull_policy": "if_not_present",
            "network": "",
            "description": "Postgres environment",
            "scope": "team"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{environment}");
    let environment_id = environment["id"]
        .as_str()
        .expect("environment id")
        .to_string();

    let (status, environments, _) = call_team(
        &app,
        "GET",
        "/api/environments",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(environments
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == environment_id)));

    let (status, opened_environment, _) = call_team(
        &app,
        "GET",
        &format!("/api/environments/{environment_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(opened_environment["tool"], "ansible");

    let (status, updated_environment, _) = call_team(
        &app,
        "PUT",
        &format!("/api/environments/{environment_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Environment {nanos} renamed"),
            "image": "quay.io/example/ee:2",
            "tool": "ansible",
            "pull_policy": "always",
            "network": "",
            "description": "Updated environment",
            "scope": "team"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated_environment}");
    assert_eq!(updated_environment["pull_policy"], "always");

    let (status, invalid_environment, _) = call_team(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Invalid environment {nanos}"),
            "image": "quay.io/example/ee:1",
            "tool": "docker"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid_environment["error"],
        "choose_a_tool_for_the_environment"
    );

    let (status, invalid_network, _) = call_team(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Invalid network {nanos}"),
            "image": "quay.io/example/ee:1",
            "tool": "ansible",
            "network": "bad network!"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid_network["error"], "the_network_name_is_invalid");

    let mut credential_ids = Vec::new();
    for (label, kind, secret) in [
        (
            "ssh",
            "ssh",
            json!({"username": "ansible", "password": "secret"}),
        ),
        (
            "vault-a",
            "vault",
            json!({"password": "one", "vault_id": "prod"}),
        ),
        (
            "vault-b",
            "vault",
            json!({"password": "two", "vault_id": "dev"}),
        ),
        (
            "vault-duplicate",
            "vault",
            json!({"password": "three", "vault_id": "prod"}),
        ),
        ("vault-c", "vault", json!({"password": "four"})),
        ("vault-d", "vault", json!({"password": "five"})),
        (
            "scm",
            "scm",
            json!({"username": "git", "password": "secret"}),
        ),
    ] {
        let (status, credential, _) = call_team(
            &app,
            "POST",
            "/api/credentials",
            Some(&cookie),
            Some(&team_id),
            Some(json!({
                "name": format!("{label} {nanos}"),
                "kind": kind,
                "secret": secret
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{label}: {credential}");
        credential_ids.push(
            credential["id"]
                .as_str()
                .expect("credential id")
                .to_string(),
        );
    }
    let (ssh_id, vault_a, vault_b, vault_duplicate, vault_c, vault_d, scm_id) = (
        &credential_ids[0],
        &credential_ids[1],
        &credential_ids[2],
        &credential_ids[3],
        &credential_ids[4],
        &credential_ids[5],
        &credential_ids[6],
    );

    let template_body = |name: &str, ids: Vec<&String>, no_environment: bool| {
        json!({
            "name": name,
            "description": "Postgres template",
            "tool": "ansible",
            "credential_ids": ids,
            "environment_id": environment_id,
            "no_environment": no_environment,
            "entrypoint": "site.yml",
            "prompt_on_launch": true
        })
    };

    let (status, duplicate_vault, _) = call_team(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team_id),
        Some(template_body(
            &format!("Duplicate vault {nanos}"),
            vec![ssh_id, vault_a, vault_duplicate],
            false,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        duplicate_vault["error"],
        "this_vault_id_is_already_selected"
    );

    let (status, duplicate_default, _) = call_team(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team_id),
        Some(template_body(
            &format!("Duplicate default vault {nanos}"),
            vec![vault_c, vault_d],
            false,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        duplicate_default["error"],
        "only_one_vault_credential_may_omit_the_vault"
    );

    let (status, template, _) = call_team(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team_id),
        Some(template_body(
            &format!("Template {nanos}"),
            vec![ssh_id, vault_a, vault_b],
            false,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().expect("template id").to_string();
    assert_eq!(template["credential_id"], ssh_id.as_str());
    assert_eq!(template["credential_ids"].as_array().map(Vec::len), Some(3));
    assert_eq!(template["prompt_on_launch"], true);
    assert_eq!(template["no_environment"], false);

    let (status, templates, _) = call_team(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(templates
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == template_id)));

    let (status, updated_template, _) = call_team(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Template {nanos} renamed"),
            "description": "Updated template",
            "tool": "ansible",
            "credential_ids": [ssh_id],
            "no_environment": true,
            "entrypoint": "site.yml",
            "prompt_on_launch": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated_template}");
    assert_eq!(updated_template["credential_ids"], json!([ssh_id]));
    assert_eq!(updated_template["no_environment"], true);
    assert_eq!(updated_template["prompt_on_launch"], false);

    let (status, wrong_kind, _) = call_team(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Template {nanos} renamed"),
            "tool": "ansible",
            "credential_ids": [scm_id],
            "entrypoint": "site.yml"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        wrong_kind["error"],
        "this_credential_kind_does_not_fit_the_tool"
    );

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let schedule_body = |name: &str, time: &str| {
        json!({
            "name": name,
            "timezone": "Europe/Berlin",
            "start_date": today,
            "start_time": time,
            "frequency": "daily",
            "weekdays": [],
            "end_kind": "never"
        })
    };

    let (status, invalid_schedule, _) = call_team(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Invalid schedule {nanos}"),
            "timezone": "Europe/Berlin",
            "start_date": today,
            "start_time": "03:00",
            "frequency": "yearly",
            "end_kind": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid_schedule["error"], "schedule_repeat_is_invalid");

    let (status, schedule, _) = call_team(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team_id),
        Some(schedule_body(&format!("Schedule {nanos}"), "03:00")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{schedule}");
    let schedule_id = schedule["id"].as_str().expect("schedule id").to_string();
    assert!(schedule["next_run_at"].is_string());

    let (status, schedules, _) = call_team(
        &app,
        "GET",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(schedules
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == schedule_id)));

    let (status, team_schedules, _) = call_team(
        &app,
        "GET",
        "/api/schedules",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(team_schedules
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == schedule_id
            && item["target_name"] == format!("Template {nanos} renamed"))));

    let (status, updated_schedule, _) = call_team(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(schedule_body(&format!("Schedule {nanos} renamed"), "04:00")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated_schedule}");
    assert_eq!(
        updated_schedule["name"],
        format!("Schedule {nanos} renamed")
    );

    let (status, disabled, _) = call_team(
        &app,
        "POST",
        &format!("/api/schedules/{schedule_id}/enabled"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{disabled}");
    assert_eq!(disabled["enabled"], false);

    let (status, hook, _) = call_team(
        &app,
        "POST",
        "/api/hooks",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Hook {nanos}"),
            "match_alertname": "HighCPU",
            "match_status": "firing",
            "template_id": template_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");
    let hook_id = hook["id"].as_str().expect("hook id").to_string();
    assert!(hook["token"]
        .as_str()
        .is_some_and(|token| !token.is_empty()));

    let (status, hooks, _) = call_team(
        &app,
        "GET",
        "/api/hooks",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(hooks
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == hook_id)));

    let (status, updated_hook, _) = call_team(
        &app,
        "PUT",
        &format!("/api/hooks/{hook_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Hook {nanos} renamed"),
            "match_alertname": "HighCPU",
            "match_status": "resolved",
            "template_id": template_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated_hook}");
    assert_eq!(updated_hook["match_status"], "resolved");

    let (status, invalid_hook, _) = call_team(
        &app,
        "PUT",
        &format!("/api/hooks/{hook_id}"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Hook {nanos} renamed"),
            "match_alertname": "HighCPU",
            "match_status": "broken",
            "template_id": template_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid_hook["error"], "status_is_firing_resolved_or_any");

    for (label, method, uri, body) in [
        (
            "environment",
            "POST",
            "/api/environments".to_string(),
            Some(json!({
                "name": format!("Denied environment {nanos}"),
                "image": "quay.io/example/ee:1",
                "tool": "ansible"
            })),
        ),
        (
            "template",
            "POST",
            "/api/templates".to_string(),
            Some(json!({
                "name": format!("Denied template {nanos}"),
                "tool": "ansible",
                "entrypoint": "site.yml"
            })),
        ),
        (
            "schedule",
            "POST",
            format!("/api/templates/{template_id}/schedules"),
            Some(schedule_body(&format!("Denied schedule {nanos}"), "05:00")),
        ),
        (
            "hook",
            "POST",
            "/api/hooks".to_string(),
            Some(json!({
                "name": format!("Denied hook {nanos}"),
                "match_alertname": "HighCPU",
                "template_id": template_id
            })),
        ),
    ] {
        let (status, denied, _) = call_team(
            &app,
            method,
            &uri,
            Some(&viewer_cookie),
            Some(&team_id),
            body,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{label}: {denied}");
    }

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/hooks/{hook_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call_team(
        &app,
        "DELETE",
        &format!("/api/environments/{environment_id}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    for credential_id in &credential_ids {
        let (status, _, _) = call_team(
            &app,
            "DELETE",
            &format!("/api/credentials/{credential_id}"),
            Some(&cookie),
            Some(&team_id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{credential_id}");
    }

    let (status, users, _) = call(&app, "GET", "/api/users", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK, "{users}");
    assert!(users.as_array().is_some_and(|items| items
        .iter()
        .any(|item| item["username"] == format!("koro{nanos}"))));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn postgres_serves_users_tokens_and_overview() {
    let Ok(url) = std::env::var("KOREOGRAF_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is not set");
        return;
    };
    if url.trim().is_empty() {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is empty");
        return;
    }
    let _guard = postgres_guard().await;
    let database = koreograf::parse_database_config(&url, None, None).expect("postgres url");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-postgres-users-{nanos}"));
    let mut cfg = config(&dir);
    cfg.admin_user = format!("adm{nanos}");
    let state = koreograf::boot_with(cfg, database).expect("boot");
    assert_eq!(state.engine(), "postgresql");
    let app = koreograf::router(state);

    let admin_name = format!("adm{nanos}");
    let (status, _, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({"username": admin_name, "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("session cookie");
    let (status, admin, _) = call(&app, "GET", "/api/me", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    let admin_id = admin["id"].as_str().expect("admin id").to_string();

    let (status, org, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        Some(json!({
            "name": format!("Users org {nanos}"),
            "description": "Postgres users organization"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org}");
    let org_id = org["id"].as_str().expect("organization id").to_string();

    let (status, team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(json!({
            "name": format!("Users team {nanos}"),
            "description": "Postgres users team",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{team}");
    let team_id = team["id"].as_str().expect("team id").to_string();

    let (status, other_team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(json!({
            "name": format!("Other team {nanos}"),
            "description": "Second Postgres team",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other_team}");
    let other_team_id = other_team["id"]
        .as_str()
        .expect("other team id")
        .to_string();

    let (status, overview, _) = call_team(
        &app,
        "GET",
        "/api/overview",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert_eq!(overview["jobs"].as_array().map(Vec::len), Some(0));
    assert_eq!(overview["deliveries"].as_array().map(Vec::len), Some(0));
    assert!(overview["workflow_name"].is_null());
    assert!(overview["hook_name"].is_null());
    assert!(overview["cache_bytes"].is_u64());

    let (status, empty_dashboard, _) = call_team(
        &app,
        "GET",
        "/api/dashboard?days=5&tz=UTC",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{empty_dashboard}");
    assert_eq!(empty_dashboard["range_days"], 5);
    assert_eq!(empty_dashboard["series"].as_array().map(Vec::len), Some(5));
    assert!(empty_dashboard["series"]
        .as_array()
        .is_some_and(|items| items.iter().all(|item| item["started"] == 0)));

    let now = Utc::now();
    let today = now.date_naive();
    let stamp = |value: chrono::DateTime<Utc>| value.to_rfc3339_opts(SecondsFormat::Secs, true);
    let yesterday = (today - Duration::days(1))
        .and_hms_opt(12, 0, 0)
        .expect("yesterday noon")
        .and_utc();
    let two_days_ago = (today - Duration::days(2))
        .and_hms_opt(12, 0, 0)
        .expect("two days ago noon")
        .and_utc();
    let completed = now - Duration::seconds(1);
    postgres_job(
        &url,
        &format!("job-today-{nanos}"),
        &team_id,
        "Today",
        "failed",
        &stamp(now - Duration::seconds(2)),
        &stamp(completed),
    )
    .await;
    postgres_job(
        &url,
        &format!("job-yesterday-{nanos}"),
        &team_id,
        "Yesterday",
        "successful",
        &stamp(yesterday),
        &stamp(yesterday + Duration::hours(1)),
    )
    .await;
    postgres_job(
        &url,
        &format!("job-two-days-{nanos}"),
        &team_id,
        "Two days ago",
        "canceled",
        &stamp(two_days_ago),
        &stamp(two_days_ago + Duration::hours(1)),
    )
    .await;

    let (status, dashboard, _) = call_team(
        &app,
        "GET",
        "/api/dashboard?days=5&tz=UTC",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dashboard}");
    let series = dashboard["series"].as_array().expect("series");
    assert_eq!(series.len(), 5);
    let bucket = |date: &str| {
        series
            .iter()
            .position(|item| item["date"] == date)
            .unwrap_or_else(|| panic!("missing bucket {date} in {series:?}"))
    };
    let today_bucket = bucket(&(now - Duration::seconds(2)).date_naive().to_string());
    assert_eq!(series[today_bucket]["started"], 1);
    assert_eq!(series[today_bucket]["failed"], 1);
    let yesterday_bucket = bucket(&(today - Duration::days(1)).to_string());
    assert_eq!(series[yesterday_bucket]["started"], 1);
    assert_eq!(series[yesterday_bucket]["successful"], 1);
    let two_days_bucket = bucket(&(today - Duration::days(2)).to_string());
    assert_eq!(series[two_days_bucket]["started"], 1);
    assert_eq!(series[two_days_bucket]["canceled"], 1);
    assert_eq!(dashboard["cache"]["roles"], 0);

    let (status, scoped_dashboard, _) = call_team(
        &app,
        "GET",
        "/api/dashboard?days=5&tz=UTC",
        Some(&cookie),
        Some(&other_team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{scoped_dashboard}");
    assert!(scoped_dashboard["series"]
        .as_array()
        .is_some_and(|items| items.iter().all(|item| item["started"] == 0)));

    let (status, scoped_overview, _) = call_team(
        &app,
        "GET",
        "/api/overview",
        Some(&cookie),
        Some(&other_team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{scoped_overview}");
    assert_eq!(scoped_overview["jobs"].as_array().map(Vec::len), Some(0));

    let (status, listed_overview, _) = call_team(
        &app,
        "GET",
        "/api/overview",
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed_overview}");
    assert_eq!(listed_overview["jobs"].as_array().map(Vec::len), Some(3));

    let username = format!("user{nanos}");
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(json!({
            "username": username,
            "password": "user-password",
            "display_name": "Ada",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let user_id = created["id"].as_str().expect("user id").to_string();
    assert_eq!(created["username"], username);
    assert_eq!(created["team_count"], 0);

    let (status, users, _) = call(&app, "GET", "/api/users", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK, "{users}");
    assert!(users
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == user_id)));

    let (status, detail, _) = call(
        &app,
        "GET",
        &format!("/api/users/{user_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["display_name"], "Ada");
    assert_eq!(detail["memberships"].as_array().map(Vec::len), Some(0));

    let (status, _, user_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({"username": username, "password": "user-password"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_cookie = user_cookie.expect("user session cookie");

    let (status, profile, _) = call(
        &app,
        "PUT",
        "/api/me",
        Some(&user_cookie),
        Some(json!({
            "display_name": "Ada Lovelace",
            "email": "ada@example.com",
            "password": "new-user-password"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{profile}");
    assert_eq!(profile["display_name"], "Ada Lovelace");
    assert_eq!(profile["email"], "ada@example.com");

    let (status, old_password, _) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({"username": username, "password": "user-password"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{old_password}");
    let (status, _, user_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({"username": username, "password": "new-user-password"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_cookie = user_cookie.expect("user session cookie");

    let (status, denied, _) = call(&app, "GET", "/api/users", Some(&user_cookie), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    let (status, denied, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&user_cookie),
        Some(json!({
            "username": format!("other{nanos}"),
            "password": "user-password",
            "display_name": "Other",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    let (status, denied, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{admin_id}"),
        Some(&user_cookie),
        Some(json!({
            "display_name": "Hijacked",
            "is_admin": true,
            "disabled": false,
            "password": ""
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    let (status, denied, _) = call(
        &app,
        "DELETE",
        &format!("/api/users/{user_id}"),
        Some(&user_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{user_id}"),
        Some(&cookie),
        Some(json!({
            "display_name": "Ada Lovelace",
            "is_admin": false,
            "disabled": false,
            "password": ""
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["display_name"], "Ada Lovelace");

    let (status, token, _) = call(
        &app,
        "POST",
        "/api/me/tokens",
        Some(&user_cookie),
        Some(json!({"name": "automation"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{token}");
    let token_id = token["id"].as_str().expect("token id").to_string();
    let secret = token["secret"].as_str().expect("token secret").to_string();
    assert!(token.get("token_hash").is_none());

    let (status, tokens, _) = call(&app, "GET", "/api/me/tokens", Some(&user_cookie), None).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    assert!(tokens
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == token_id)));

    let (status, via_token) = call_bearer(&app, "GET", "/api/me", &secret, None).await;
    assert_eq!(status, StatusCode::OK, "{via_token}");
    assert_eq!(via_token["username"], username);

    let (status, foreign, _) = call(
        &app,
        "DELETE",
        &format!("/api/me/tokens/{token_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{foreign}");

    let (status, _, _) = call(
        &app,
        "DELETE",
        &format!("/api/me/tokens/{token_id}"),
        Some(&user_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, revoked) = call_bearer(&app, "GET", "/api/me", &secret, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{revoked}");
    assert_eq!(revoked["error"], "sign_in_is_required");

    let (status, disabled, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{user_id}"),
        Some(&cookie),
        Some(json!({
            "display_name": "Ada Lovelace",
            "is_admin": false,
            "disabled": true,
            "password": ""
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{disabled}");
    assert_eq!(disabled["disabled"], true);

    let (status, _, _) = call(&app, "GET", "/api/me", Some(&user_cookie), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, disabled_login, _) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({"username": username, "password": "new-user-password"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(disabled_login["error"], "the_account_is_disabled");

    let (status, self_disable, _) = call(
        &app,
        "PUT",
        &format!("/api/users/{admin_id}"),
        Some(&cookie),
        Some(json!({
            "display_name": "Administrator",
            "is_admin": true,
            "disabled": true,
            "password": ""
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{self_disable}");
    assert_eq!(self_disable["error"], "you_cannot_disable_your_own_account");

    let (status, self_delete, _) = call(
        &app,
        "DELETE",
        &format!("/api/users/{admin_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{self_delete}");
    assert_eq!(self_delete["error"], "you_cannot_delete_your_own_account");

    let (status, _, _) = call(
        &app,
        "DELETE",
        &format!("/api/users/{user_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, gone, _) = call(
        &app,
        "GET",
        &format!("/api/users/{user_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{gone}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn postgres_serves_jobs_and_their_output() {
    let Ok(url) = std::env::var("KOREOGRAF_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is not set");
        return;
    };
    if url.trim().is_empty() {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is empty");
        return;
    }
    let _guard = postgres_guard().await;
    let database = koreograf::parse_database_config(&url, None, None).expect("postgres url");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-postgres-jobs-{nanos}"));
    let mut cfg = config(&dir);
    let admin_name = format!("jobadm{nanos}");
    cfg.admin_user = admin_name.clone();
    let state = koreograf::boot_with(cfg, database).expect("boot");
    assert_eq!(state.engine(), "postgresql");
    let app = koreograf::router(state);

    let (status, _, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({"username": admin_name, "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("session cookie");

    let (status, org, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        Some(json!({
            "name": format!("Jobs org {nanos}"),
            "description": "Postgres jobs organization"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org}");
    let org_id = org["id"].as_str().expect("organization id").to_string();

    let (status, team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(json!({
            "name": format!("Jobs team {nanos}"),
            "description": "Postgres jobs team",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{team}");
    let team_id = team["id"].as_str().expect("team id").to_string();

    let create_template = |name: String, script: &'static str| {
        let app = app.clone();
        let cookie = cookie.clone();
        let team = team_id.clone();
        async move {
            let (status, body, _) = call_team(
                &app,
                "POST",
                "/api/templates",
                Some(&cookie),
                Some(&team),
                Some(json!({
                    "name": name,
                    "description": "Postgres job template",
                    "tool": "shell",
                    "entrypoint": "",
                    "no_environment": true,
                    "spec": {"script": script}
                })),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{body}");
            body["id"].as_str().expect("template id").to_string()
        }
    };
    let success_template = create_template(
        format!("Postgres success {nanos}"),
        "echo postgres-job-line",
    )
    .await;
    let fail_template = create_template(
        format!("Postgres fail {nanos}"),
        "echo postgres-failure; exit 1",
    )
    .await;
    let sleep_template =
        create_template(format!("Postgres sleep {nanos}"), "sleep 30; echo done").await;

    let (status, launched, _) = call_team(
        &app,
        "POST",
        &format!("/api/templates/{success_template}/launch"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({"extra_vars": {"from_postgres": true}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");
    let success_job = launched["id"].as_str().expect("job id").to_string();
    assert_eq!(launched["status"], "pending");
    assert_eq!(launched["parameters"]["tool"], "shell");
    assert_eq!(launched["parameters"]["extra_vars"]["from_postgres"], true);

    let mut successful = Value::Null;
    for _ in 0..80 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let (status, body, _) = call_team(
            &app,
            "GET",
            &format!("/api/jobs/{success_job}"),
            Some(&cookie),
            Some(&team_id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if body["status"] == "successful" {
            successful = body;
            break;
        }
    }
    assert_eq!(successful["status"], "successful", "{successful}");
    assert!(successful["events"].as_array().is_some_and(|events| events
        .iter()
        .any(|event| event["line"] == "postgres-job-line")));
    assert_eq!(successful["output_capped"], false);

    let output_request = Request::builder()
        .method("GET")
        .uri(format!("/api/jobs/{success_job}/output"))
        .header("cookie", &cookie)
        .header("x-team-id", &team_id)
        .body(Body::empty())
        .unwrap();
    let output = app.clone().oneshot(output_request).await.unwrap();
    assert_eq!(output.status(), StatusCode::OK);
    let output_body = output.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&output_body).contains("postgres-job-line"));

    let (status, denied, _) = call_team(
        &app,
        "POST",
        &format!("/api/jobs/{success_job}/relaunch"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    assert_eq!(
        denied["error"],
        "only_a_failed_or_canceled_job_can_be_relaunched"
    );

    let (status, failed_launch, _) = call_team(
        &app,
        "POST",
        &format!("/api/templates/{fail_template}/launch"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{failed_launch}");
    let failed_job = failed_launch["id"].as_str().expect("job id").to_string();
    let mut failed = Value::Null;
    for _ in 0..80 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let (status, body, _) = call_team(
            &app,
            "GET",
            &format!("/api/jobs/{failed_job}"),
            Some(&cookie),
            Some(&team_id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if body["status"] == "failed" {
            failed = body;
            break;
        }
    }
    assert_eq!(failed["status"], "failed", "{failed}");

    let (status, relaunched, _) = call_team(
        &app,
        "POST",
        &format!("/api/jobs/{failed_job}/relaunch"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{relaunched}");
    assert_eq!(relaunched["trigger"], "relaunch");
    assert_ne!(relaunched["id"], failed["id"]);
    let relaunched_id = relaunched["id"].as_str().expect("job id").to_string();
    let _ = call_team(
        &app,
        "POST",
        &format!("/api/jobs/{relaunched_id}/cancel"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({})),
    )
    .await;

    let (status, canceled_launch, _) = call_team(
        &app,
        "POST",
        &format!("/api/templates/{sleep_template}/launch"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{canceled_launch}");
    let sleep_job = canceled_launch["id"].as_str().expect("job id").to_string();
    let (status, canceled, _) = call_team(
        &app,
        "POST",
        &format!("/api/jobs/{sleep_job}/cancel"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    assert_eq!(canceled["status"], "canceled");

    let (status, relaunched_sleep, _) = call_team(
        &app,
        "POST",
        &format!("/api/jobs/{sleep_job}/relaunch"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{relaunched_sleep}");
    assert_eq!(relaunched_sleep["trigger"], "relaunch");
    let relaunched_sleep_id = relaunched_sleep["id"].as_str().expect("job id").to_string();
    let _ = call_team(
        &app,
        "POST",
        &format!("/api/jobs/{relaunched_sleep_id}/cancel"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({})),
    )
    .await;

    let (status, workflow, _) = call_team(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team_id),
        Some(json!({
            "name": format!("Jobs workflow {nanos}"),
            "description": "Postgres jobs workflow",
            "graph": {
                "nodes": [{
                    "id": "start",
                    "template_id": success_template,
                    "name": "Start"
                }],
                "edges": []
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let workflow_id = workflow["id"].as_str().expect("workflow id").to_string();
    let (status, run, _) = call_team(
        &app,
        "POST",
        &format!("/api/workflows/{workflow_id}/launch"),
        Some(&cookie),
        Some(&team_id),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{run}");
    let run_id = run["id"].as_str().expect("workflow job id").to_string();
    assert_eq!(run["status"], "running");
    let mut finished = Value::Null;
    for _ in 0..80 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let (status, body, _) = call_team(
            &app,
            "GET",
            &format!("/api/workflow-jobs/{run_id}"),
            Some(&cookie),
            Some(&team_id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if matches!(
            body["status"].as_str(),
            Some("successful" | "failed" | "canceled")
        ) {
            finished = body;
            break;
        }
    }
    assert_eq!(finished["status"], "successful", "{finished}");
    assert_eq!(finished["nodes"][0]["phase"], "successful", "{finished}");
    assert!(finished["nodes"][0]["job_id"].is_string(), "{finished}");

    let token = format!("joblist{nanos}");
    let list_dates = [
        "2026-01-01T00:00:00Z",
        "2026-01-02T00:00:00Z",
        "2026-01-03T00:00:00Z",
    ];
    let list_statuses = ["successful", "failed", "canceled"];
    let list_triggers = ["manual", "relaunch", "workflow"];
    for index in 0..3 {
        let id = format!("{token}-{index}");
        postgres_job_record(
            &url,
            &team_id,
            Some(&success_template),
            PostgresJob {
                id: &id,
                name: &id,
                status: list_statuses[index],
                trigger: list_triggers[index],
                created_at: list_dates[index],
            },
        )
        .await;
    }

    let list_call = |query: String| {
        let app = app.clone();
        let cookie = cookie.clone();
        let team_id = team_id.clone();
        async move {
            call_team(
                &app,
                "GET",
                &format!("/api/jobs?{query}"),
                Some(&cookie),
                Some(&team_id),
                None,
            )
            .await
        }
    };
    let (status, listed, _) = list_call(format!("q={token}&limit=2")).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["total"], 3);
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(2));
    let (status, second_page, _) = list_call(format!("q={token}&limit=2&offset=2")).await;
    assert_eq!(status, StatusCode::OK, "{second_page}");
    assert_eq!(second_page["total"], 3);
    assert_eq!(second_page["items"].as_array().map(Vec::len), Some(1));
    let (_, filtered, _) = list_call(format!("q={token}&status=failed")).await;
    assert_eq!(filtered["total"], 1);
    assert_eq!(filtered["items"][0]["status"], "failed");
    let (_, by_tool, _) = list_call(format!("q={token}&tool=shell")).await;
    assert_eq!(by_tool["total"], 3);
    let (_, by_trigger, _) = list_call(format!("q={token}&trigger=workflow")).await;
    assert_eq!(by_trigger["total"], 1);
    let (_, by_template, _) = list_call(format!("q={token}&template_id={success_template}")).await;
    assert_eq!(by_template["total"], 3);
    let (_, by_date, _) = list_call(format!(
        "q={token}&since=2026-01-02T00:00:00Z&until=2026-01-02T23:59:59Z"
    ))
    .await;
    assert_eq!(by_date["total"], 1);
    assert_eq!(by_date["items"][0]["id"], format!("{token}-1"));

    let gap_job = format!("{token}-gap");
    postgres_job_record(
        &url,
        &team_id,
        Some(&success_template),
        PostgresJob {
            id: &gap_job,
            name: "Gap job",
            status: "successful",
            trigger: "manual",
            created_at: "2026-01-04T00:00:00Z",
        },
    )
    .await;
    postgres_job_lines(&url, &gap_job, "gap", 2_500).await;
    let (status, gap_detail, _) = call_team(
        &app,
        "GET",
        &format!("/api/jobs/{gap_job}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{gap_detail}");
    let events = gap_detail["events"].as_array().expect("events");
    assert_eq!(events.len(), 2_001);
    let gaps: Vec<&Value> = events
        .iter()
        .filter(|event| event["stream"] == "gap")
        .collect();
    assert_eq!(gaps.len(), 1, "{gap_detail}");
    assert_eq!(gaps[0]["line"], "500");
    assert_eq!(gap_detail["output_capped"], false);
    let output_request = Request::builder()
        .method("GET")
        .uri(format!("/api/jobs/{gap_job}/output"))
        .header("cookie", &cookie)
        .header("x-team-id", &team_id)
        .body(Body::empty())
        .unwrap();
    let output = app.clone().oneshot(output_request).await.unwrap();
    assert_eq!(output.status(), StatusCode::OK);
    let output_text =
        String::from_utf8_lossy(&output.into_body().collect().await.unwrap().to_bytes())
            .to_string();
    assert!(output_text.contains("gap-1"), "{output_text}");
    assert!(output_text.contains("gap-2500"), "{output_text}");

    let capped_job = format!("{token}-capped");
    postgres_job_record(
        &url,
        &team_id,
        Some(&success_template),
        PostgresJob {
            id: &capped_job,
            name: "Capped job",
            status: "successful",
            trigger: "manual",
            created_at: "2026-01-05T00:00:00Z",
        },
    )
    .await;
    postgres_job_bulk_lines(&url, &capped_job, 9, 1_000_000).await;
    let (status, capped_detail, _) = call_team(
        &app,
        "GET",
        &format!("/api/jobs/{capped_job}"),
        Some(&cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{capped_detail}");
    assert_eq!(capped_detail["output_capped"], true);
    let output_request = Request::builder()
        .method("GET")
        .uri(format!("/api/jobs/{capped_job}/output"))
        .header("cookie", &cookie)
        .header("x-team-id", &team_id)
        .body(Body::empty())
        .unwrap();
    let output = app.clone().oneshot(output_request).await.unwrap();
    assert_eq!(output.status(), StatusCode::OK);
    let output_body = output.into_body().collect().await.unwrap().to_bytes();
    assert!(
        String::from_utf8_lossy(&output_body).contains("output truncated"),
        "download did not mark the truncation"
    );
    assert!(output_body.len() < 8 * 1024 * 1024 + 100);

    let viewer_id = format!("viewer{nanos}");
    let viewer_name = format!("viewer{nanos}");
    let viewer_token = format!("viewer-session-{nanos}");
    postgres_session(
        &url,
        &viewer_id,
        &viewer_name,
        &[(&team_id, "viewer")],
        &viewer_token,
    )
    .await;
    let viewer_cookie = format!("koreograf_session={viewer_token}");
    let (status, denied, _) = call_team(
        &app,
        "POST",
        &format!("/api/templates/{success_template}/launch"),
        Some(&viewer_cookie),
        Some(&team_id),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    let (status, denied, _) = call_team(
        &app,
        "POST",
        &format!("/api/jobs/{success_job}/cancel"),
        Some(&viewer_cookie),
        Some(&team_id),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    let (status, denied, _) = call_team(
        &app,
        "POST",
        &format!("/api/jobs/{failed_job}/relaunch"),
        Some(&viewer_cookie),
        Some(&team_id),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");

    let outsider_id = format!("outsider{nanos}");
    let outsider_name = format!("outsider{nanos}");
    let outsider_token = format!("outsider-session-{nanos}");
    postgres_session(&url, &outsider_id, &outsider_name, &[], &outsider_token).await;
    let outsider_cookie = format!("koreograf_session={outsider_token}");
    let (status, denied, _) = call(
        &app,
        "GET",
        &format!("/api/jobs/{success_job}"),
        Some(&outsider_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    let (status, denied, _) = call_team(
        &app,
        "GET",
        "/api/jobs",
        Some(&outsider_cookie),
        Some(&team_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    let (status, denied, _) = call(
        &app,
        "GET",
        &format!("/api/jobs/{success_job}/output"),
        Some(&outsider_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn postgres_backup_restore_round_trip_keeps_the_previous_archive() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let database = koreograf::parse_database_config(&url, None, None).expect("postgres url");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-postgres-backup-{nanos}"));
    let mut cfg = config(&dir);
    cfg.admin_user = format!("pgbackupadmin{nanos}");
    let state = koreograf::boot_with(cfg, database).expect("boot");
    assert_eq!(state.engine(), "postgresql");
    let app = koreograf::router(state);

    let (status, _, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({
            "username": format!("pgbackupadmin{nanos}"),
            "password": "test-admin-pass"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("session cookie");

    let before = format!("Before backup {nanos}");
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        Some(json!({ "name": before, "description": "Portable backup" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    let (status, archive, _) =
        call_raw(&app, "GET", "/api/admin/backup", Some(&cookie), &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let entries = zip_entries(&archive);
    assert!(
        entries.iter().any(|name| name == "manifest.json"),
        "{entries:?}"
    );
    assert!(
        entries.iter().any(|name| name == "koreograf.db"),
        "{entries:?}"
    );
    assert!(
        entries.iter().any(|name| name == "secret.key"),
        "{entries:?}"
    );

    let after = format!("After backup {nanos}");
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        Some(json!({ "name": after, "description": "After the backup" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    let (status, body, _) = call_raw(
        &app,
        "POST",
        "/api/admin/backup",
        Some(&cookie),
        &[
            ("x-restore-confirm", "ERSETZEN"),
            ("content-type", "application/zip"),
        ],
        Some(archive),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let restored: Value = serde_json::from_slice(&body).expect("restore body");
    assert_eq!(restored["engine"], "postgresql");

    let (status, organizations, _) =
        call(&app, "GET", "/api/organizations", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK, "{organizations}");
    let names: Vec<&str> = organizations
        .as_array()
        .expect("organizations")
        .iter()
        .filter_map(|item| item["name"].as_str())
        .collect();
    assert!(names.contains(&before.as_str()), "{names:?}");
    assert!(!names.contains(&after.as_str()), "{names:?}");

    let previous_path = dir.join("restore-previous.zip");
    let previous = std::fs::read(&previous_path).expect("previous archive");
    assert!(zip_entries(&previous)
        .iter()
        .any(|name| name == "manifest.json"));

    let (status, body, _) = call_raw(
        &app,
        "POST",
        "/api/admin/backup",
        Some(&cookie),
        &[
            ("x-restore-confirm", "ERSETZEN"),
            ("content-type", "application/zip"),
        ],
        Some(previous),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let (status, organizations, _) =
        call(&app, "GET", "/api/organizations", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK, "{organizations}");
    let names: Vec<&str> = organizations
        .as_array()
        .expect("organizations")
        .iter()
        .filter_map(|item| item["name"].as_str())
        .collect();
    assert!(names.contains(&after.as_str()), "{names:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn db_migrate_copies_every_table_and_resets_sequences() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let name = format!("koreograf_migrate_{nanos}");
    let target = create_database(&url, &name).await;
    let dir = std::env::temp_dir().join(format!("koreograf-migrate-source-{nanos}"));
    let state = seed_sqlite_source(&dir);
    drop(state);

    let output = migrate_command(&dir, &target);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("KOREOGRAF_DATABASE_URL='"), "{stdout}");
    assert!(stdout.contains(&name), "{stdout}");
    assert!(stdout.contains("sslmode=disable"), "{stdout}");

    let client = postgres_client(&target).await;
    for (table, expected) in sqlite_counts(&dir.join("koreograf.db")) {
        let actual: i64 = client
            .query_one(&format!("SELECT COUNT(*) FROM \"{table}\""), &[])
            .await
            .expect("target count")
            .get(0);
        assert_eq!(actual, expected, "table {table}");
    }
    client
        .execute(
            "INSERT INTO job_events (job_id, ts, stream, line)
             VALUES ('job-copy', '2026-01-01T00:00:00Z', 'stdout', 'third')",
            &[],
        )
        .await
        .expect("insert after copy");
    let next: i64 = client
        .query_one("SELECT MAX(id) FROM job_events", &[])
        .await
        .expect("max id")
        .get(0);
    assert_eq!(next, 3);

    let again = migrate_command(&dir, &target);
    assert!(!again.status.success());
    assert!(
        String::from_utf8_lossy(&again.stderr).contains("target is not empty"),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );

    drop(client);
    drop_database(&url, &name).await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn db_migrate_refuses_while_the_data_directory_is_in_use() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-migrate-locked-{nanos}"));
    let state = seed_sqlite_source(&dir);
    drop(state);
    let before = std::fs::read(dir.join("koreograf.db")).expect("sqlite file");
    let _lock = koreograf::lock_data_directory(&dir).expect("data directory lock");

    let output = migrate_command(&dir, &url);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("already in use"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after = std::fs::read(dir.join("koreograf.db")).expect("sqlite file");
    assert_eq!(before, after);

    drop(_lock);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn db_migrate_failure_leaves_the_target_reusable() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let name = format!("koreograf_migrate_retry_{nanos}");
    let target = create_database(&url, &name).await;
    let dir = std::env::temp_dir().join(format!("koreograf-migrate-retry-{nanos}"));
    let state = seed_sqlite_source(&dir);
    drop(state);

    let first = migrate_command(&dir, &target);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );

    let client = postgres_client(&target).await;
    let tables = PORTABLE_TABLES
        .iter()
        .map(|table| format!("\"{table}\""))
        .collect::<Vec<_>>()
        .join(", ");
    client
        .batch_execute(&format!("TRUNCATE TABLE {tables} RESTART IDENTITY CASCADE"))
        .await
        .expect("truncate target");
    client
        .batch_execute(
            "CREATE OR REPLACE FUNCTION koreograf_fail_migrate() RETURNS trigger
             LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'migration blocked'; END $$;
             CREATE TRIGGER koreograf_fail_migrate BEFORE INSERT ON users
             FOR EACH ROW EXECUTE FUNCTION koreograf_fail_migrate();",
        )
        .await
        .expect("failure trigger");

    let failed = migrate_command(&dir, &target);
    assert!(!failed.status.success());
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("rolled back and can be reused"),
        "{}",
        String::from_utf8_lossy(&failed.stderr)
    );
    for table in PORTABLE_TABLES {
        let count: i64 = client
            .query_one(&format!("SELECT COUNT(*) FROM \"{table}\""), &[])
            .await
            .expect("target count")
            .get(0);
        assert_eq!(count, 0, "table {table} kept rows after a failed run");
    }

    client
        .batch_execute(
            "DROP TRIGGER koreograf_fail_migrate ON users;
             DROP FUNCTION koreograf_fail_migrate();",
        )
        .await
        .expect("drop failure trigger");

    let retried = migrate_command(&dir, &target);
    assert!(
        retried.status.success(),
        "{}",
        String::from_utf8_lossy(&retried.stderr)
    );
    let users: i64 = client
        .query_one("SELECT COUNT(*) FROM users", &[])
        .await
        .expect("users count")
        .get(0);
    assert!(users > 0);

    drop(client);
    drop_database(&url, &name).await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn postgres_ldap_sign_in_provisions_the_user_and_syncs_teams() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let name = format!("koreograf_ldap_{nanos}");
    let target = create_database(&url, &name).await;
    let dir = std::env::temp_dir().join(format!("koreograf-postgres-ldap-{nanos}"));
    let ldap_username = format!("ldap{nanos}");
    let ldap_email = format!("{ldap_username}@example.com");
    let (ldap_url, ldap_server) = ldap_stub(ldap_username.clone()).await;
    let mut cfg = config(&dir);
    cfg.admin_user = format!("ldapadmin{nanos}");
    cfg.ldap = Some(LdapConfig {
        url: ldap_url,
        mode: "ldap".into(),
        bind_dn: "cn=reader,dc=example,dc=com".into(),
        bind_password: "reader-password".into(),
        user_base: "ou=people,dc=example,dc=com".into(),
        user_filter: "(uid={username})".into(),
        username_attr: "uid".into(),
        group_attr: "memberOf".into(),
        group_base: String::new(),
        starttls: false,
        tls_insecure: false,
    });
    let database = koreograf::parse_database_config(&target, None, None).expect("postgres url");
    let state = koreograf::boot_with(cfg, database).expect("boot");
    assert_eq!(state.engine(), "postgresql");
    let app = koreograf::router(state);

    let (status, _, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({"username": format!("ldapadmin{nanos}"), "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let admin = cookie.expect("admin session");

    let (status, org, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&admin),
        Some(json!({
            "name": format!("LDAP org {nanos}"),
            "description": "Postgres LDAP organization"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org}");
    let org_id = org["id"].as_str().expect("organization id").to_string();
    let group = "cn=Plattform,ou=groups,dc=example,dc=com";
    let mut team_ids = Vec::new();
    for suffix in ["A", "B"] {
        let (status, team, _) = call(
            &app,
            "POST",
            "/api/teams",
            Some(&admin),
            Some(json!({
                "name": format!("LDAP team {suffix} {nanos}"),
                "description": "Postgres LDAP team",
                "ldap_group": group,
                "organization_id": org_id
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{team}");
        team_ids.push(team["id"].as_str().expect("team id").to_string());
    }

    let (status, profile, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({"username": ldap_username.clone(), "password": "correct horse"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{profile}");
    assert_eq!(profile["auth_source"], "ldap");
    assert_eq!(profile["display_name"], "Directory User");
    assert_eq!(profile["email"], ldap_email);
    let ldap_cookie = cookie.expect("ldap session");
    let memberships = profile["memberships"]
        .as_array()
        .expect("memberships")
        .clone();
    for team_id in &team_ids {
        let membership = memberships
            .iter()
            .find(|item| item["team_id"] == *team_id)
            .expect("ldap membership");
        assert_eq!(membership["role"], "operator");
        assert_eq!(membership["source"], "ldap");
    }

    let (status, _, _) = call(
        &app,
        "PUT",
        &format!("/api/teams/{}", team_ids[1]),
        Some(&admin),
        Some(json!({
            "name": format!("LDAP team B {nanos}"),
            "description": "Postgres LDAP team",
            "ldap_group": "cn=Other,ou=groups,dc=example,dc=com",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({"username": ldap_username.clone(), "password": "correct horse"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, profile, _) = call(&app, "GET", "/api/me", Some(&ldap_cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    let memberships = profile["memberships"].as_array().expect("memberships");
    assert!(memberships
        .iter()
        .any(|item| item["team_id"] == team_ids[0]));
    assert!(!memberships
        .iter()
        .any(|item| item["team_id"] == team_ids[1]));

    ldap_server.abort();
    drop_database(&url, &name).await;
    let _ = std::fs::remove_dir_all(&dir);
}

async fn ldap_stub(username: String) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("ldap stub bind");
    let addr = listener.local_addr().expect("ldap stub address");
    let username = std::sync::Arc::new(username);
    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let username = username.clone();
            tokio::spawn(async move {
                let _ = serve_ldap(&mut stream, &username).await;
            });
        }
    });
    (format!("ldap://{addr}"), handle)
}

async fn serve_ldap(stream: &mut tokio::net::TcpStream, username: &str) -> std::io::Result<()> {
    while let Some(message) = read_ldap_message(stream).await? {
        let Some(body) = ber_body(&message) else {
            return Ok(());
        };
        let Some((tag, _, id_len)) = ber_read(body) else {
            return Ok(());
        };
        if tag != 0x02 {
            return Ok(());
        }
        let message_id = &body[..id_len];
        let Some((op, _, _)) = ber_read(&body[id_len..]) else {
            return Ok(());
        };
        match op {
            0x60 => {
                let mut content = message_id.to_vec();
                content.extend(ldap_result(0x61, 0));
                stream.write_all(&ber(0x30, &content)).await?;
            }
            0x63 => {
                for response in [ldap_entry(username), ldap_result(0x65, 0)] {
                    let mut content = message_id.to_vec();
                    content.extend(response);
                    stream.write_all(&ber(0x30, &content)).await?;
                }
            }
            _ => return Ok(()),
        }
    }
    Ok(())
}

async fn read_ldap_message(stream: &mut tokio::net::TcpStream) -> std::io::Result<Option<Vec<u8>>> {
    let mut tag = [0u8; 1];
    match stream.read_exact(&mut tag).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let mut first = [0u8; 1];
    stream.read_exact(&mut first).await?;
    let mut message = vec![tag[0], first[0]];
    let len = if first[0] & 0x80 == 0 {
        usize::from(first[0])
    } else {
        let count = usize::from(first[0] & 0x7f);
        let mut bytes = vec![0u8; count];
        stream.read_exact(&mut bytes).await?;
        message.extend_from_slice(&bytes);
        bytes
            .iter()
            .fold(0usize, |value, byte| (value << 8) | usize::from(*byte))
    };
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    message.extend_from_slice(&body);
    Ok(Some(message))
}

fn ber_body(input: &[u8]) -> Option<&[u8]> {
    ber_read(input).map(|(_, body, _)| body)
}

fn ber_read(input: &[u8]) -> Option<(u8, &[u8], usize)> {
    if input.len() < 2 {
        return None;
    }
    let tag = input[0];
    let first = usize::from(input[1]);
    let (len, header) = if first & 0x80 == 0 {
        (first, 2)
    } else {
        let count = first & 0x7f;
        if count == 0 || input.len() < 2 + count {
            return None;
        }
        let len = input[2..2 + count]
            .iter()
            .fold(0usize, |value, byte| (value << 8) | usize::from(*byte));
        (len, 2 + count)
    };
    let end = header.checked_add(len)?;
    if input.len() < end {
        return None;
    }
    Some((tag, &input[header..end], end))
}

fn ber(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = body.len();
    if len < 128 {
        out.push(len as u8);
    } else {
        let mut bytes = Vec::new();
        let mut remaining = len;
        while remaining > 0 {
            bytes.push((remaining & 0xff) as u8);
            remaining >>= 8;
        }
        bytes.reverse();
        out.push(0x80 | bytes.len() as u8);
        out.extend_from_slice(&bytes);
    }
    out.extend_from_slice(body);
    out
}

fn ldap_result(tag: u8, code: u8) -> Vec<u8> {
    let body = vec![0x0a, 0x01, code, 0x04, 0x00, 0x04, 0x00];
    ber(tag, &body)
}

fn ldap_entry(username: &str) -> Vec<u8> {
    let email = format!("{username}@example.com");
    let attributes = [
        ("uid", vec![username]),
        ("cn", vec!["Directory User"]),
        ("displayName", vec!["Directory User"]),
        ("mail", vec![email.as_str()]),
        ("memberOf", vec!["cn=Plattform,ou=groups,dc=example,dc=com"]),
    ];
    let mut list = Vec::new();
    for (name, values) in attributes {
        let mut partial = ber(0x04, name.as_bytes());
        let mut set = Vec::new();
        for value in values {
            set.extend(ber(0x04, value.as_bytes()));
        }
        partial.extend(ber(0x31, &set));
        list.extend(ber(0x30, &partial));
    }
    let mut body = ber(
        0x04,
        format!("uid={username},ou=people,dc=example,dc=com").as_bytes(),
    );
    body.extend(ber(0x30, &list));
    ber(0x64, &body)
}
