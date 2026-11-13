#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;
use std::time::Duration;

use axum::http::StatusCode;
use common::{
    create_database, drop_database, postgres_client, postgres_guard, postgres_test_url, Options,
    TestApp,
};
use serde_json::{json, Value};

const PASSWORD: &str = "S3cret-Database-Password-Should-Not-Leak";

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
        assert_eq!(
            audit.len(),
            3,
            "the audit entry has more than user, time, and host: {value}"
        );
        assert!(
            audit["at"].is_string(),
            "the audit entry has no time: {value}"
        );
        assert_eq!(
            audit["host"], value["migration"]["target_host"],
            "the audit host is not the target host: {value}"
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
    let app = TestApp::boot(
        "db-wizard",
        Options {
            mcp: false,
            ..Options::default()
        },
    );
    let admin = app.login("admin", common::ADMIN_PASSWORD).await;

    let (status, body) = app
        .call(
            "POST",
            "/api/v1/admin/database/check",
            Some(&admin),
            None,
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

    let (status, body) = app
        .call(
            "POST",
            "/api/v1/admin/database/migrate",
            Some(&admin),
            None,
            Some(json!({"id": "not-a-check"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "run_the_checks_again", "{body}");
    assert!(!body_text(&body).contains(PASSWORD), "{body}");

    let (status, body) = app.call("GET", "/api/maintenance", None, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["active"], false, "{body}");

    let (status, body) = app
        .call(
            "GET",
            "/api/admin/database/status",
            Some(&admin),
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["migration"]["running"], false, "{body}");
    assert!(body["migration"]["audit"].is_null(), "{body}");
    assert!(!body_text(&body).contains(PASSWORD), "{body}");
}

#[tokio::test]
async fn the_database_wizard_responses_have_the_documented_shape() {
    let app = TestApp::boot(
        "db-wizard-shape",
        Options {
            mcp: false,
            ..Options::default()
        },
    );
    let admin = app.login("admin", common::ADMIN_PASSWORD).await;

    let (status, body) = app
        .call(
            "GET",
            "/api/v1/admin/database/status",
            Some(&admin),
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({
            "engine": "sqlite",
            "supported": true,
            "maintenance": {"active": false, "since": null},
            "migration": {
                "running": false,
                "done": false,
                "failed": false,
                "started_at": null,
                "finished_at": null,
                "target_host": "",
                "url": "",
                "rows": 0,
                "error": null,
                "tables": [],
                "audit": null
            }
        })
    );

    let (status, body) = app
        .call(
            "POST",
            "/api/v1/admin/database/check",
            Some(&admin),
            None,
            Some(target_body("127.0.0.1", 1, PASSWORD)),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({
            "all_passed": false,
            "id": null,
            "checks": [
                {"id": "source_schema", "ok": true, "key": "database_check_source_schema_ok"},
                {"id": "reachable", "ok": false, "key": "database_check_unreachable"},
                {"id": "tls", "ok": false, "key": "database_check_not_run"},
                {"id": "server_version", "ok": false, "key": "database_check_not_run"},
                {"id": "rights", "ok": false, "key": "database_check_not_run"},
                {"id": "empty", "ok": false, "key": "database_check_not_run"}
            ]
        })
    );
    assert!(!body_text(&body).contains(PASSWORD), "{body}");
}

#[tokio::test]
async fn the_wizard_routes_need_an_administrator() {
    let app = TestApp::boot(
        "db-wizard-admin",
        Options {
            mcp: false,
            ..Options::default()
        },
    );
    let admin = app.login("admin", common::ADMIN_PASSWORD).await;
    let (status, body) = app
        .call(
            "POST",
            "/api/users",
            Some(&admin),
            None,
            Some(json!({
                "username": "viewer",
                "password": "viewer-pass",
                "display_name": "Viewer",
                "is_admin": false
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let viewer = app.login("viewer", "viewer-pass").await;

    for (method, uri, body) in [
        ("GET", "/api/admin/database/status", None),
        (
            "POST",
            "/api/v1/admin/database/check",
            Some(target_body("127.0.0.1", 1, PASSWORD)),
        ),
        (
            "POST",
            "/api/v1/admin/database/migrate",
            Some(json!({"id": "x"})),
        ),
    ] {
        let (status, body) = app.call(method, uri, Some(&viewer), None, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {body}");
    }
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
    assert!(
        script.contains("/api/v1/admin/database/check"),
        "check route"
    );
    assert!(
        script.contains("/api/v1/admin/database/migrate"),
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

#[tokio::test]
async fn the_ui_migration_copies_rows_enters_maintenance_and_returns_a_password_free_url() {
    let Some(base) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let name = common::postgres_database_name("ui_migrate");
    let target_url = create_database(&base, &name).await;
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

    let app = TestApp::boot(
        "ui-migrate-source",
        Options {
            mcp: false,
            ..Options::default()
        },
    );
    let admin = app.login("admin", common::ADMIN_PASSWORD).await;

    let body = json!({
        "host": host,
        "port": port,
        "database": name,
        "user": user,
        "password": target_password,
        "sslmode": sslmode,
        "ca_file": null
    });
    let (status, checked) = app
        .call(
            "POST",
            "/api/v1/admin/database/check",
            Some(&admin),
            None,
            Some(body),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{checked}");
    assert_eq!(checked["all_passed"], true, "{checked}");
    assert_no_password(&checked, &target_password);
    let id = checked["id"].as_str().expect("check id").to_string();

    let (status, started) = app
        .call(
            "POST",
            "/api/v1/admin/database/migrate",
            Some(&admin),
            None,
            Some(json!({"id": id})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    assert_eq!(started, json!({"started": true}));

    let mut last = Value::Null;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (status, status_body) = app
            .call(
                "GET",
                "/api/admin/database/status",
                Some(&admin),
                None,
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

    let (status, public) = app.call("GET", "/api/maintenance", None, None, None).await;
    assert_eq!(status, StatusCode::OK, "{public}");
    assert_eq!(public["active"], true, "{public}");

    let (status, refused) = app
        .call(
            "POST",
            "/api/v1/admin/database/migrate",
            Some(&admin),
            None,
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
    drop_database(&base, &name).await;
}
