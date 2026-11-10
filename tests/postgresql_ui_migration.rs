use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use koreograf::{Config, CookieSecure, LocalJobs, RunnerMode};
use serde_json::{json, Value};
use tower::ServiceExt;

const PASSWORD: &str = "S3cret-Database-Password-Should-Not-Leak";

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

async fn login(app: &axum::Router, username: &str, password: &str) -> String {
    let (status, body, cookie) = call(
        app,
        "POST",
        "/api/login",
        None,
        Some(json!({"username": username, "password": password})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    cookie.expect("session cookie")
}

fn body_text(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn assert_no_password(value: &Value, password: &str) {
    if password.is_empty() {
        return;
    }
    let text = body_text(value);
    assert!(
        !text.contains(&format!(":{password}@")),
        "the response contains the database password in a URL: {value}"
    );
    if let Some(url) = value["migration"]["url"].as_str() {
        let parsed = url::Url::parse(url).expect("migration url");
        assert!(
            parsed.password().is_none(),
            "the migration url contains the password: {value}"
        );
    }
    if let Some(audit) = value["migration"]["audit"].as_object() {
        let text = serde_json::to_string(audit).unwrap_or_default();
        assert!(
            !text.contains(password),
            "the audit entry contains the password: {value}"
        );
    }
}

fn target_body(host: &str, port: u16, password: &str) -> Value {
    json!({
        "host": host,
        "port": port,
        "database": "koreograf",
        "user": "koreograf",
        "password": password,
        "sslmode": "disable",
        "ca_file": null
    })
}

#[tokio::test]
async fn failed_checks_refuse_to_start_and_never_echo_the_password() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-db-wizard-{nanos}"));
    let state = koreograf::boot(config(&dir)).expect("boot");
    let app = koreograf::router(state.clone());
    let admin = login(&app, "admin", "test-admin-pass").await;

    let (status, body, _) = call(
        &app,
        "POST",
        "/api/admin/database/check",
        Some(&admin),
        Some(target_body("127.0.0.1", 1, PASSWORD)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["all_passed"], false);
    assert!(body["id"].is_null(), "{body}");
    assert!(!body_text(&body).contains(PASSWORD), "{body}");
    let checks = body["checks"].as_array().expect("checks");
    assert!(checks.iter().any(|item| item["ok"] == false), "{body}");
    assert!(
        checks
            .iter()
            .any(|item| item["id"] == "reachable" && item["ok"] == false),
        "{body}"
    );

    let (status, body, _) = call(
        &app,
        "POST",
        "/api/admin/database/migrate",
        Some(&admin),
        Some(json!({"id": "not-a-check"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "run_the_checks_again", "{body}");
    assert!(!body_text(&body).contains(PASSWORD), "{body}");

    let (status, body, _) = call(&app, "GET", "/api/maintenance", None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["active"], false, "{body}");

    let (status, body, _) = call(
        &app,
        "GET",
        "/api/admin/database/status",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["migration"]["running"], false, "{body}");
    assert!(body["migration"]["audit"].is_null(), "{body}");
    assert!(!body_text(&body).contains(PASSWORD), "{body}");

    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_wizard_routes_need_an_administrator() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-db-wizard-admin-{nanos}"));
    let state = koreograf::boot(config(&dir)).expect("boot");
    let app = koreograf::router(state.clone());
    let admin = login(&app, "admin", "test-admin-pass").await;
    let (status, body, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&admin),
        Some(json!({
            "username": "viewer",
            "password": "viewer-pass",
            "display_name": "Viewer",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let viewer = login(&app, "viewer", "viewer-pass").await;

    for (method, uri, body) in [
        ("GET", "/api/admin/database/status", None),
        (
            "POST",
            "/api/admin/database/check",
            Some(target_body("127.0.0.1", 1, PASSWORD)),
        ),
        (
            "POST",
            "/api/admin/database/migrate",
            Some(json!({"id": "x"})),
        ),
    ] {
        let (status, body, _) = call(&app, method, uri, Some(&viewer), body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {body}");
    }

    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_wizard_and_the_maintenance_banner_are_in_the_ui_and_catalogs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    assert!(
        script.contains("id=\"settings-panel-database\""),
        "database panel"
    );
    assert!(script.contains("databaseTargetBody"), "database form");
    assert!(script.contains("databaseChecksHtml"), "check list");
    assert!(script.contains("databaseProgressHtml"), "table progress");
    assert!(script.contains("databaseSwitchHtml"), "switch instructions");
    assert!(script.contains("maintenanceBanner"), "maintenance banner");
    assert!(
        script.contains("loadMaintenance()"),
        "the banner is not loaded"
    );
    assert!(script.contains("/api/admin/database/check"), "check route");
    assert!(
        script.contains("/api/admin/database/migrate"),
        "start route"
    );

    let mut catalogs = Vec::new();
    for language in ["de", "en"] {
        let path = root.join(format!("web/i18n/{language}.json"));
        let text = std::fs::read_to_string(&path).expect("catalog");
        let value: serde_json::Value = serde_json::from_str(&text).expect("catalog json");
        let object = value.as_object().expect("catalog object");
        let keys = object.keys().cloned().collect::<Vec<_>>();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "{language} is not alphabetical");
        for key in [
            "move_database_to_postgresql",
            "step_1_target",
            "step_2_checks",
            "step_3_migration",
            "step_4_switch",
            "maintenance_mode_banner",
            "no_new_jobs_during_maintenance",
            "database_migration_done",
            "database_migration_failed",
            "database_check_unreachable",
            "run_the_checks_again",
            "finish_or_cancel_running_jobs",
            "restart_koreograf_after_the_change",
            "rollback_switches_the_url_back",
        ] {
            assert!(
                object
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|item| !item.trim().is_empty()),
                "{key} is missing from {language}.json"
            );
        }
        catalogs.push(
            object
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
        );
    }
    assert_eq!(catalogs[0], catalogs[1], "the catalogs differ");

    let docs = std::fs::read_to_string(root.join("docs/postgresql.md")).expect("docs page");
    assert!(docs.contains("Docker Compose"), "Docker example");
    assert!(docs.contains("OS package"), "package example");
    assert!(docs.contains("Helm"), "Helm example");
    assert!(docs.contains("Roll back"), "rollback");
    for page in ["installation.md", "configuration.md", "backup-restore.md"] {
        let text = std::fs::read_to_string(root.join("docs").join(page)).expect(page);
        assert!(
            text.contains("postgresql.md"),
            "{page} does not link the PostgreSQL page"
        );
    }
}

fn postgres_test_url() -> Option<String> {
    let Ok(url) = std::env::var("KOREOGRAF_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is not set");
        return None;
    };
    (!url.trim().is_empty()).then_some(url)
}

async fn postgres_client(url: &str) -> tokio_postgres::Client {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("postgres connection");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

static POSTGRES_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

async fn postgres_guard() -> tokio::sync::MutexGuard<'static, ()> {
    POSTGRES_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

#[tokio::test]
async fn the_ui_migration_copies_rows_enters_maintenance_and_returns_a_password_free_url() {
    let Some(base) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let name = format!("koreograf_ui_migrate_{nanos}");
    let base_client = postgres_client(&base).await;
    base_client
        .batch_execute(&format!("CREATE DATABASE \"{name}\""))
        .await
        .expect("create database");
    drop(base_client);
    let mut target_url = url::Url::parse(&base).expect("base url");
    target_url.set_path(&format!("/{name}"));
    let target_url = target_url.to_string();
    let parsed = url::Url::parse(&target_url).expect("target url");
    let host = parsed.host_str().expect("host").to_string();
    let port = parsed.port().unwrap_or(5432);
    let user = parsed.username().to_string();
    let target_password = parsed.password().unwrap_or_default().to_string();
    let sslmode = parsed
        .query_pairs()
        .find(|(key, _)| key == "sslmode")
        .map(|(_, value)| value.to_string())
        .unwrap_or_else(|| "disable".to_string());

    let dir = std::env::temp_dir().join(format!("koreograf-ui-migrate-source-{nanos}"));
    let state = koreograf::boot(config(&dir)).expect("boot");
    let app = koreograf::router(state.clone());
    let admin = login(&app, "admin", "test-admin-pass").await;

    let body = json!({
        "host": host,
        "port": port,
        "database": name,
        "user": user,
        "password": target_password,
        "sslmode": sslmode,
        "ca_file": null
    });
    let (status, checked, _) = call(
        &app,
        "POST",
        "/api/admin/database/check",
        Some(&admin),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{checked}");
    assert_eq!(checked["all_passed"], true, "{checked}");
    assert_no_password(&checked, &target_password);
    let id = checked["id"].as_str().expect("check id").to_string();

    let (status, started, _) = call(
        &app,
        "POST",
        "/api/admin/database/migrate",
        Some(&admin),
        Some(json!({"id": id})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");

    let mut last = Value::Null;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (status, status_body, _) = call(
            &app,
            "GET",
            "/api/admin/database/status",
            Some(&admin),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{status_body}");
        last = status_body;
        if last["migration"]["done"] == true || last["migration"]["failed"] == true {
            break;
        }
    }
    assert_eq!(last["migration"]["done"], true, "{last}");
    assert_eq!(last["migration"]["failed"], false, "{last}");
    assert!(
        last["migration"]["rows"].as_i64().unwrap_or(0) > 0,
        "{last}"
    );
    assert_eq!(last["maintenance"]["active"], true, "{last}");
    assert_no_password(&last, &target_password);
    assert!(
        last["migration"]["url"].as_str().is_some_and(|url| {
            url::Url::parse(url)
                .is_ok_and(|parsed| parsed.path().contains(&name) && parsed.password().is_none())
        }),
        "{last}"
    );
    assert!(
        last["migration"]["audit"]["user"] == "admin",
        "the audit entry names the user: {last}"
    );
    assert_no_password(&last, &target_password);

    let (status, public, _) = call(&app, "GET", "/api/maintenance", None, None).await;
    assert_eq!(status, StatusCode::OK, "{public}");
    assert_eq!(public["active"], true, "{public}");

    let (status, refused, _) = call(
        &app,
        "POST",
        "/api/admin/database/migrate",
        Some(&admin),
        Some(json!({"id": "again"})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{refused}");
    assert_eq!(refused["error"], "maintenance_mode_is_active", "{refused}");

    let target = postgres_client(&target_url).await;
    let users: i64 = target
        .query_one("SELECT COUNT(*) FROM users", &[])
        .await
        .expect("users count")
        .get(0);
    assert!(users > 0, "the target has no users");
    drop(target);
    drop(app);

    let base_client = postgres_client(&base).await;
    let _ = base_client
        .batch_execute(&format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"))
        .await;
    drop(base_client);
    let _ = std::fs::remove_dir_all(&dir);
}
