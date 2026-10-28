use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use koreograf::{Config, CookieSecure, LocalJobs, RunnerMode};
use serde_json::{json, Value};
use tower::ServiceExt;

fn config(dir: &std::path::Path) -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        data_dir: dir.to_path_buf(),
        web_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
        examples_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
        admin_user: "admin".into(),
        admin_password: Some("test-admin-pass".into()),
        runner: RunnerMode::Local,
        job_timeout_secs: 60,
        job_output_lines: 8_000,
        local_jobs: LocalJobs::All,
        project_roots: vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")],
        hsts: false,
        cookie_secure: CookieSecure::Auto,
        trusted_proxy: false,
        ldap: None,
    }
}

async fn call(
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

#[tokio::test]
async fn responses_carry_standard_headers() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-headers-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    assert!(!state.cfg.hsts);
    let app = koreograf::router(state);
    for path in ["/", "/app.js", "/api/me", "/healthz", "/metrics"] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let headers = response.headers();
        assert_eq!(headers["x-content-type-options"], "nosniff", "{path}");
        assert_eq!(headers["x-frame-options"], "DENY", "{path}");
        let policy = headers["content-security-policy"].to_str().unwrap();
        assert!(policy.contains("frame-ancestors 'none'"), "{path} {policy}");
        assert!(
            policy.contains("style-src-attr 'unsafe-inline'"),
            "{path} {policy}"
        );
        assert_eq!(headers["referrer-policy"], "no-referrer", "{path}");
        assert_eq!(
            headers["permissions-policy"], "camera=(), microphone=(), geolocation=()",
            "{path}"
        );
        assert!(headers.get("strict-transport-security").is_none(), "{path}");
    }
    drop(app);

    let mut enabled = config(&dir);
    enabled.hsts = true;
    let state = koreograf::boot(enabled).unwrap();
    let app = koreograf::router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.headers()["strict-transport-security"],
        "max-age=31536000"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn session_cookie_secure_follows_the_setting_and_the_proxy() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-cookie-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let login = br#"{"username":"admin","password":"test-admin-pass"}"#.to_vec();

    let state = koreograf::boot(config(&dir)).unwrap();
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

    let mut forced = config(&dir);
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

    let mut automatic = config(&dir);
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
async fn a_team_description_stops_at_one_thousand_characters() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-desc-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
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

#[tokio::test]
async fn an_empty_template_entry_path_is_rejected_and_marked_in_the_form() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-entry-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("for_example_site_yml"));
    assert!(script.contains("id=\"entrypoint-error\""));
    assert!(script.contains("form.entrypoint.value = books[0]"));
    assert!(script.contains("ansible_needs_a_playbook_as_the_entry"));
    assert!(script.contains("opentofu_needs_a_module_directory"));
    assert!(!script.contains("placeholder=\"site.yml\""));
    let (status, style, _) = call(&app, "GET", "/app.css", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(style.as_str().unwrap_or("").contains("field-error"));

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

    let (status, ansible, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Empty playbook",
            "tool": "ansible",
            "entrypoint": "  ",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{ansible}");
    assert_eq!(ansible["error"], "ansible_needs_a_playbook_as_the_entry");

    let (status, tofu, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Empty module",
            "tool": "opentofu",
            "entrypoint": "",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{tofu}");
    assert_eq!(tofu["error"], "opentofu_needs_a_module_directory");

    let (status, outside, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Outside",
            "tool": "ansible",
            "entrypoint": "../secret",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{outside}");
    assert_eq!(outside["error"], "the_entry_path_must_be_relative_and_must");

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Site",
            "tool": "ansible",
            "entrypoint": "site.yml",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["entrypoint"], "site.yml");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_job_log_marks_a_fatal_line_and_keeps_galaxy_as_a_warning() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-log-style-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("function classifyLogLine"));
    assert!(script.contains("fatal:\\s*\\["));
    assert!(script.contains("UNREACHABLE!"));
    assert!(script.contains("ERROR!"));
    assert!(script.contains("failed=[1-9]"));
    assert!(script.contains("unreachable=[1-9]"));
    assert!(script.contains("PLAY RECAP"));
    assert!(script.contains("Galaxy install failed. The existing cache is used anyway."));
    assert!(script.contains("Role install failed. The existing cache is used anyway."));
    assert!(script.contains("ansible-galaxy collection install"));
    assert!(script.contains("first_failure"));
    assert!(script.contains(
        "row.stream === \"system\" && plainLogLine(row.line).includes(\"Tag filter matched no tasks.\")"
    ));
    assert!(script.contains("tag_filter_matched_no_tasks"));
    assert!(script.contains("function when("));
    assert!(script.contains("timeZoneName: \"short\""));
    assert!(script.contains("let flashKey"));
    assert!(script.contains("this_field_needs_a_valid_value"));
    assert!(script.contains("class=\"btn pill danger\""));
    assert!(script.contains("log-summary"));
    let (status, style, _) = call(&app, "GET", "/app.css", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let style = style.as_str().unwrap_or("");
    assert!(style.contains(".card.danger-zone"));
    assert!(style.contains(".table th { text-align: left; font-size: 13px;"));
    assert!(style.contains(".log div.fail"));
    assert!(style.contains(".log div.warn"));
    assert!(style.contains(".log .log-summary"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn login_shell_job_and_webhook() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, index, _) = call(&app, "GET", "/", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(index.as_str().unwrap_or("").contains("app.js"));
    let (status, icon, _) = call(&app, "GET", "/favicon.svg", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(icon.as_str().unwrap_or("").contains("<svg"));
    let (status, metrics, _) = call(&app, "GET", "/metrics", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let metrics = metrics.as_str().unwrap_or("");
    assert!(
        metrics.contains("# TYPE koreograf_jobs_started_total counter"),
        "{metrics}"
    );
    assert!(
        metrics.contains("# TYPE koreograf_cache_collections gauge"),
        "{metrics}"
    );
    assert!(!metrics.contains("test-admin-pass"));

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

    let (status, templates, _) = call(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{templates}");
    let health = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Healthcheck")
        .unwrap();
    let health_id = health["id"].as_str().unwrap();

    let (status, job, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{health_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    let job_id = job["id"].as_str().unwrap().to_string();

    let mut final_job = Value::Null;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let (status, body, _) = call(
            &app,
            "GET",
            &format!("/api/jobs/{job_id}"),
            Some(&cookie),
            Some(&team),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        if matches!(
            body["status"].as_str(),
            Some("successful" | "failed" | "canceled")
        ) {
            final_job = body;
            break;
        }
    }
    assert_eq!(
        final_job["status"], "successful",
        "log: {}",
        final_job["events"]
    );

    let mut metrics = String::new();
    for _ in 0..20 {
        let (status, body, _) = call(&app, "GET", "/metrics", None, None, None).await;
        assert_eq!(status, StatusCode::OK);
        metrics = body.as_str().unwrap_or("").to_string();
        if metrics.contains(
            "koreograf_jobs_finished_total{tool=\"shell\",playbook=\"health.sh\",status=\"successful\"} 1",
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        metrics.contains("koreograf_jobs_started_total{tool=\"shell\",playbook=\"health.sh\"} 1"),
        "{metrics}"
    );
    assert!(
        metrics.contains(
            "koreograf_jobs_finished_total{tool=\"shell\",playbook=\"health.sh\",status=\"successful\"} 1"
        ),
        "{metrics}"
    );
    assert!(metrics.contains("team=\"Plattform\""), "{metrics}");
    assert!(
        metrics.contains("koreograf_cache_collections{"),
        "{metrics}"
    );
    assert!(metrics.contains("koreograf_cache_roles{"), "{metrics}");
    assert!(!metrics.contains("test-admin-pass"), "{metrics}");

    let (status, hooks, _) =
        call(&app, "GET", "/api/hooks", Some(&cookie), Some(&team), None).await;
    assert_eq!(status, StatusCode::OK, "{hooks}");
    let hook = &hooks.as_array().unwrap()[0];
    let hook_id = hook["id"].as_str().unwrap();
    let token = hook["token"].as_str().unwrap();

    let (status, ignored, _) = call(
        &app,
        "POST",
        &format!("/hooks/{hook_id}/{token}"),
        None,
        None,
        Some(json!({"alertname": "Other", "status": "firing"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ignored}");
    assert_eq!(ignored["status"], "ignored");

    let (status, accepted, _) = call(
        &app,
        "POST",
        &format!("/hooks/{hook_id}/{token}"),
        None,
        None,
        Some(json!({
            "status": "firing",
            "alerts": [{
                "status": "firing",
                "labels": {"alertname": "DiskSpaceLow", "instance": "web-1"}
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(accepted["status"], "accepted");
    assert!(accepted["workflow_job_id"].is_string());

    let (status, denied, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Zyklus",
            "graph": {
                "nodes": [
                    {"id": "a", "template_id": health_id, "name": "A", "x": 0, "y": 0},
                    {"id": "b", "template_id": health_id, "name": "B", "x": 10, "y": 10}
                ],
                "edges": [
                    {"from": "a", "to": "b", "when": "success"},
                    {"from": "b", "to": "a", "when": "success"}
                ]
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The launch handler used to lock the database and then lock it again while
/// building the response. `std::sync::Mutex` waits forever in that case, and
/// every later request stops. A timeout on another thread is the failure signal.
#[test]
fn launching_a_workflow_answers() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("runtime");
        let result = runtime.block_on(launch_demo_workflow());
        let _ = tx.send(result);
    });
    let (status, body, health) = rx.recv_timeout(Duration::from_secs(8)).unwrap_or_else(|_| {
        panic!("POST /api/workflows/{{id}}/launch held the database lock and stopped answering")
    });
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["name"], "Self-Heal Ablauf");
    assert_eq!(body["status"], "running");
    assert_eq!(body["nodes"].as_array().map(Vec::len), Some(3), "{body}");
    assert_eq!(health, StatusCode::OK);
}

async fn launch_demo_workflow() -> (StatusCode, Value, StatusCode) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-wf-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
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
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();
    let (status, workflows, _) = call(
        &app,
        "GET",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{workflows}");
    let workflow_id = workflows
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Self-Heal Ablauf")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, body, _) = call(
        &app,
        "POST",
        &format!("/api/workflows/{workflow_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    let (health, _, _) = call(&app, "GET", "/healthz", None, None, None).await;
    let _ = std::fs::remove_dir_all(&dir);
    (status, body, health)
}

#[tokio::test]
async fn failed_job_shows_parameters_and_can_be_relaunched() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-relaunch-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("output"));
    assert!(script.contains("download_output"));
    assert!(script.contains("data-relaunch"));
    assert!(script.contains("relaunch"));
    assert!(script.contains("failed_hosts_only"));
    let (status, style, _) = call(&app, "GET", "/app.css", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let style = style.as_str().unwrap_or("");
    assert!(style.contains(".tabs"));
    assert!(style.contains(".tab-bar"));
    assert!(script.contains("download-output"));

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

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Kaputt",
            "tool": "shell",
            "entrypoint": "",
            "extra_vars": {"keep": "template"},
            "spec": {"script": "echo nope; exit 1"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();

    let (status, launched, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {"from_launch": "yes"}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");
    let job_id = launched["id"].as_str().unwrap().to_string();

    let mut failed = Value::Null;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let (status, body, _) = call(
            &app,
            "GET",
            &format!("/api/jobs/{job_id}"),
            Some(&cookie),
            Some(&team),
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
    let output_request = Request::builder()
        .method("GET")
        .uri(format!("/api/jobs/{job_id}/output"))
        .header("cookie", &cookie)
        .header("x-team-id", &team)
        .body(Body::empty())
        .unwrap();
    let output = app.clone().oneshot(output_request).await.unwrap();
    assert_eq!(output.status(), StatusCode::OK);
    let disposition = output
        .headers()
        .get("content-disposition")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(disposition.contains("attachment"));
    assert!(disposition.contains("filename=\"kaputt-"));
    let output_body = output.into_body().collect().await.unwrap().to_bytes();
    let output_text = String::from_utf8_lossy(&output_body);
    assert!(output_text.contains("nope"), "{output_text}");
    assert_eq!(failed["parameters"]["tool"], "shell");
    assert_eq!(failed["parameters"]["extra_vars"]["keep"], "template");
    assert_eq!(failed["parameters"]["extra_vars"]["from_launch"], "yes");
    assert!(failed["parameters"]["spec"]["script"]
        .as_str()
        .unwrap_or("")
        .contains("exit 1"));
    assert!(failed["failed_hosts"].as_array().unwrap().is_empty());

    let (status, copy, _) = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/relaunch"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{copy}");
    assert_ne!(copy["id"], failed["id"]);
    assert_eq!(copy["trigger"], "relaunch");
    assert_eq!(copy["parameters"]["extra_vars"]["keep"], "template");
    assert_eq!(copy["parameters"]["extra_vars"]["from_launch"], "yes");

    let (status, denied, _) = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/relaunch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"only_failed_hosts": true})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    assert_eq!(denied["error"], "only_ansible_jobs_have_failed_hosts");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn prompt_on_launch_overrides_the_job_and_keeps_the_template() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-prompt-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = config(&dir);
    cfg.local_jobs = LocalJobs::Admins;
    let state = koreograf::boot(cfg).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("prompt_on_launch"));
    assert!(script.contains("launch-prompt"));

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

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prompted",
            "tool": "ansible",
            "entrypoint": "site.yml",
            "prompt_on_launch": true,
            "extra_vars": {"keep": "template"},
            "spec": {"job_type": "run", "tags": "ping", "limit": "web", "forks": 5, "diff": true}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    assert_eq!(template["prompt_on_launch"], true);
    let template_id = template["id"].as_str().unwrap();

    let (status, denied, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Quiet",
            "tool": "ansible",
            "entrypoint": "site.yml",
            "spec": {"limit": "web"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{denied}");
    assert_eq!(denied["prompt_on_launch"], false);
    let quiet_id = denied["id"].as_str().unwrap();
    let (status, blocked, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{quiet_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}, "spec": {"limit": "db"}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{blocked}");
    assert_eq!(blocked["error"], "prompt_on_launch_is_off");

    let (status, shell, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prompt shell",
            "tool": "shell",
            "entrypoint": "",
            "prompt_on_launch": true,
            "spec": {"script": "true"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{shell}");
    let shell_id = shell["id"].as_str().unwrap();
    let (status, shell_denied, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{shell_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}, "spec": {"limit": "db"}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{shell_denied}");
    assert_eq!(shell_denied["error"], "launch_fields_are_only_for_ansible");

    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Prompt hosts", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap();
    let (status, credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prompt SSH",
            "kind": "ssh",
            "secret": {"username": "deploy", "password": "prompt-ssh-pass"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    assert!(
        !credential.to_string().contains("prompt-ssh-pass"),
        "response echoed the secret"
    );
    let credential_id = credential["id"].as_str().unwrap();
    let (status, other_team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Other prompt team"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other_team}");
    let other = other_team["id"].as_str().unwrap();
    let (status, foreign_inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(other),
        Some(json!({"name": "Foreign hosts", "body": "[db]\ndb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{foreign_inventory}");
    let foreign_inventory_id = foreign_inventory["id"].as_str().unwrap();
    let (status, foreign_credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(other),
        Some(json!({
            "name": "Foreign SSH",
            "kind": "ssh",
            "secret": {"username": "other", "password": "foreign-ssh-pass"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{foreign_credential}");
    let foreign_credential_id = foreign_credential["id"].as_str().unwrap();
    let (status, wrong_inventory, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}, "inventory_id": foreign_inventory_id})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{wrong_inventory}");
    assert_eq!(
        wrong_inventory["error"],
        "the_inventory_belongs_to_another_team"
    );
    let (status, wrong_credential, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}, "credential_ids": [foreign_credential_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{wrong_credential}");
    assert_eq!(
        wrong_credential["error"],
        "the_credential_belongs_to_another_team"
    );

    let (status, launched, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "extra_vars": {"from_launch": "yes"},
            "inventory_id": inventory_id,
            "credential_ids": [credential_id],
            "spec": {
                "limit": "db",
                "tags": "heal",
                "skip_tags": "",
                "job_type": "check",
                "verbosity": 1
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");
    let job_id = launched["id"].as_str().unwrap();
    assert_eq!(launched["parameters"]["spec"]["limit"], "db");
    assert_eq!(launched["parameters"]["spec"]["tags"], "heal");
    assert_eq!(launched["parameters"]["spec"]["job_type"], "check");
    assert_eq!(launched["parameters"]["spec"]["verbosity"], 1);
    assert_eq!(launched["parameters"]["spec"]["forks"], 5);
    assert_eq!(launched["parameters"]["spec"]["diff"], true);
    assert!(launched["parameters"]["spec"].get("skip_tags").is_none());
    assert_eq!(launched["parameters"]["extra_vars"]["keep"], "template");
    assert_eq!(launched["parameters"]["extra_vars"]["from_launch"], "yes");
    assert_eq!(launched["parameters"]["inventory_id"], inventory_id);
    assert_eq!(launched["parameters"]["inventory_name"], "Prompt hosts");
    assert_eq!(launched["parameters"]["credential_id"], credential_id);
    assert_eq!(
        launched["parameters"]["credentials"][0]["id"],
        credential_id
    );
    assert!(
        !launched.to_string().contains("prompt-ssh-pass"),
        "job echoed the secret"
    );
    let _ = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/cancel"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;

    let (status, stored, _) = call(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    let row = stored
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == template_id)
        .unwrap();
    assert_eq!(row["prompt_on_launch"], true);
    assert_eq!(row["spec"]["limit"], "web");
    assert_eq!(row["spec"]["tags"], "ping");
    assert_eq!(row["spec"]["job_type"], "run");
    assert_eq!(row["extra_vars"]["keep"], "template");
    assert!(row["extra_vars"].get("from_launch").is_none());
    assert!(row["inventory_id"].is_null());
    assert!(row.get("credential_id").is_none_or(Value::is_null));
    assert!(row["credential_ids"].as_array().unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn oversized_job_output_keeps_the_recap_in_the_download() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-output-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = config(&dir);
    cfg.job_output_lines = 100;
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
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();
    let script = "i=1\nwhile [ \"$i\" -le 140 ]; do\n  printf 'bulk-%s\\n' \"$i\"\n  i=$((i + 1))\ndone\nprintf '%s\\n' 'fatal: [ghost]: FAILED! => {}'\nprintf '%s\\n' 'PLAY RECAP'\nprintf '%s\\n' 'web-9 : ok=1 changed=0 unreachable=0 failed=1 skipped=0 rescued=0 ignored=0'\nprintf '%s\\n' 'MARKER-TAIL-19'\nexit 1\n";
    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Overflow",
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": script}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();
    let (status, launched, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");
    let job_id = launched["id"].as_str().unwrap();
    let mut failed = Value::Null;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let (status, body, _) = call(
            &app,
            "GET",
            &format!("/api/jobs/{job_id}"),
            Some(&cookie),
            Some(&team),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if body["status"] == "failed" || body["status"] == "successful" {
            failed = body;
            break;
        }
    }
    assert_eq!(failed["status"], "failed", "{failed}");
    assert_eq!(failed["failed_hosts"], json!(["web-9"]));
    let events = failed["events"].as_array().unwrap();
    assert!(events.iter().any(|event| event["line"]
        .as_str()
        .unwrap_or("")
        .contains("MARKER-TAIL-19")));
    let output_request = Request::builder()
        .method("GET")
        .uri(format!("/api/jobs/{job_id}/output"))
        .header("cookie", &cookie)
        .header("x-team-id", &team)
        .body(Body::empty())
        .unwrap();
    let output = app.clone().oneshot(output_request).await.unwrap();
    assert_eq!(output.status(), StatusCode::OK);
    let output_body = output.into_body().collect().await.unwrap().to_bytes();
    let output_text = String::from_utf8_lossy(&output_body);
    assert!(output_text.contains("MARKER-TAIL-19"), "{output_text}");
    assert!(output_text.contains("bulk-1"), "{output_text}");
    assert!(
        output_text.contains("limit 100; kept the first 75 and the last 25"),
        "{output_text}"
    );
    assert!(!output_text.contains("bulk-80"), "{output_text}");
    assert!(
        !output_text.contains("Further output is discarded (limit"),
        "{output_text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("git ist nicht installiert");
    assert!(status.success(), "git {args:?} in {}", dir.display());
}

fn project_names(list: &Value) -> Vec<String> {
    list.as_array()
        .expect("Projektliste")
        .iter()
        .map(|item| item["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn organizations_inventories_and_git_sync() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-org-{nanos}"));
    let origin = std::env::temp_dir().join(format!("koreograf-origin-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&origin).unwrap();
    let mut cfg = config(&dir);
    cfg.project_roots.push(origin.clone());
    let state = koreograf::boot(cfg).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("inventories"), "{script}");
    assert!(script.contains("organizations"));
    assert!(script.contains("nav-group"));
    let (status, style, _) = call(&app, "GET", "/app.css", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(style.as_str().unwrap_or("").contains("nav-label"));

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
    let org_on_team = me["teams"][0]["organization_id"].as_str().unwrap();
    assert!(!org_on_team.is_empty());

    let (status, orgs, _) = call(
        &app,
        "GET",
        "/api/organizations",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{orgs}");
    let org_id = orgs
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Plattform")
        .expect("Organisation Plattform")["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(org_on_team, org_id);

    let inventory_body = "[web]\nweb-1 ansible_host=10.0.0.8 ansible_connection=ssh\n\n[db]\ndb-1\n\n[all:children]\nweb\ndb\n";
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Web", "body": inventory_body})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["host_count"], 2);
    let inventory_id = created["id"].as_str().unwrap();
    let web = created["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|host| host["name"] == "web-1")
        .unwrap();
    assert_eq!(web["groups"], json!(["web"]));
    assert_eq!(web["vars"]["ansible_host"], "10.0.0.8");
    assert_eq!(web["vars"]["ansible_connection"], "ssh");
    assert!(created["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|host| host["name"] != "web" && host["name"] != "db"));

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Web", "body": format!("{inventory_body}[web]\nweb-2\n")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["host_count"], 3);

    git(&origin, &["init", "-b", "main"]);
    git(&origin, &["config", "user.email", "test@example.com"]);
    git(&origin, &["config", "user.name", "Koreograf Test"]);
    std::fs::write(origin.join("marker.txt"), "one\n").unwrap();
    git(&origin, &["add", "marker.txt"]);
    git(&origin, &["commit", "-m", "one"]);
    let url = format!("file://{}", origin.display());

    let (status, project, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Syncprobe", "source_type": "git", "source": url})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    assert_eq!(project["organization_id"], org_id);
    let project_id = project["id"].as_str().unwrap();

    let (status, synced, _) = call(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    let checkout = std::path::PathBuf::from(synced["path"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(checkout.join("marker.txt")).unwrap(),
        "one\n"
    );

    std::fs::write(origin.join("two.txt"), "two\n").unwrap();
    git(&origin, &["add", "two.txt"]);
    git(&origin, &["commit", "-m", "two"]);
    let (status, synced, _) = call(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    assert_eq!(
        std::fs::read_to_string(checkout.join("two.txt")).unwrap(),
        "two\n"
    );

    let missing = format!("file:///tmp/koreograf-missing-{nanos}");
    let (status, broken, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Kaputt", "source_type": "git", "source": missing})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{broken}");
    let broken_id = broken["id"].as_str().unwrap();
    for _ in 0..2 {
        let (status, failed, _) = call(
            &app,
            "POST",
            &format!("/api/projects/{broken_id}/sync"),
            Some(&cookie),
            Some(&team),
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{failed}");
        let message = failed["error"].as_str().unwrap_or("");
        let lower = message.to_ascii_lowercase();
        assert!(
            lower.contains("fatal")
                || lower.contains("repository")
                || lower.contains("exist")
                || message == "the_project_path_is_not_allowed",
            "{failed}"
        );
        assert!(!message.contains("git clone ist fehlgeschlagen"));
    }

    let (status, other_org, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Andere", "description": "Zweite Organisation"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other_org}");
    let other_org_id = other_org["id"].as_str().unwrap();

    let (status, _user, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "operator",
            "display_name": "operator",
            "password": "operator-pass",
            "is_admin": false
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
    let (status, operator_me, operator_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "operator", "password": "operator-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{operator_me}");
    let operator_cookie = operator_cookie.expect("operator cookie");
    let (status, denied, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&operator_cookie),
        Some(&team),
        Some(json!({"organization_id": other_org_id, "team_ids": []})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");

    let (status, second, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Betrieb",
            "description": "Zweites Team",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let betrieb = second["id"].as_str().unwrap();
    assert_eq!(second["organization_id"], org_id);

    let (status, visible, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{visible}");
    assert!(project_names(&visible).contains(&"Syncprobe".to_string()));
    let (status, visible_b, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(betrieb),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{visible_b}");
    assert!(
        project_names(&visible_b).contains(&"Syncprobe".to_string()),
        "{visible_b}"
    );

    let (status, assigned, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"organization_id": org_id, "team_ids": [betrieb]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{assigned}");
    let (status, hidden, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{hidden}");
    assert!(
        !project_names(&hidden)
            .iter()
            .any(|name| name == "Syncprobe"),
        "{hidden}"
    );
    let (status, only_b, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(betrieb),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{only_b}");
    assert!(
        project_names(&only_b).contains(&"Syncprobe".to_string()),
        "{only_b}"
    );

    let (status, nacht, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Nacht",
            "description": "Team der anderen Organisation",
            "organization_id": other_org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{nacht}");
    let nacht = nacht["id"].as_str().unwrap();
    let (status, moved, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(betrieb),
        Some(json!({"organization_id": other_org_id, "team_ids": []})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["organization_id"], other_org_id);
    let (status, gone, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(betrieb),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{gone}");
    assert!(
        !project_names(&gone).iter().any(|name| name == "Syncprobe"),
        "{gone}"
    );
    let (status, there, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(nacht),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{there}");
    assert!(
        project_names(&there).contains(&"Syncprobe".to_string()),
        "{there}"
    );

    let _ = std::fs::remove_dir_all(&origin);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn playbooks_project_inventory_and_netbox() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-src-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("playbook_from_the_project"), "{script}");
    assert!(script.contains("data-add-kind=\"project\""));
    assert!(script.contains("data-add-kind=\"netbox\""));
    assert!(script.contains("refresh"));
    assert!(script.contains("netbox: t(\"netbox\")"));
    assert!(script.contains("[\"netbox\", \"netbox\", \"\"]"));
    let inspector = script
        .split_once("function renderInspector")
        .expect("workflow inspector")
        .1;
    let node_at = inspector.find("this_node").expect("selected node panel");
    let add_at = inspector[..node_at]
        .rfind("addNodeControls()")
        .expect("add stays above the selected node");
    assert!(
        node_at - add_at < 800,
        "add node drifted away from the selected node"
    );
    let node_panel = &inspector[node_at..inspector.find("node-delete").expect("delete node")];
    assert!(node_panel.contains("on_success_go_to"));
    assert!(node_panel.contains("on_failure_go_to"));
    assert!(node_panel.contains("always_go_to"));
    assert!(node_panel.contains("nodeParams("));
    assert!(script.contains("applies_only_to_this_node_the_template_stays"));
    assert!(script.contains("node-inventory"));
    assert!(script.contains("no_inventory"));

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

    let (status, projects, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{projects}");
    let demo = projects
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Demo")
        .expect("Demo-Projekt")["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, contents, _) = call(
        &app,
        "GET",
        &format!("/api/projects/{demo}/contents"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{contents}");
    assert!(contents["playbooks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == "site.yml"));
    assert!(contents["inventories"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["path"] == "inventory/hosts.ini" && item["format"] == "ini" }));

    let (status, imported, _) = call(
        &app,
        "POST",
        "/api/inventories/import",
        Some(&cookie),
        Some(&team),
        Some(json!({"project_id": demo, "path": "inventory/hosts.ini"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{imported}");
    assert_eq!(imported["source_kind"], "project");
    assert_eq!(imported["source"]["path"], "inventory/hosts.ini");
    let host = imported["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "localhost")
        .expect("localhost");
    assert_eq!(host["vars"]["ansible_connection"], "local");
    let imported_id = imported["id"].as_str().unwrap();

    let (status, refreshed, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{imported_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{refreshed}");
    assert!(refreshed["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "localhost"));

    let (status, plain, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Hand", "body": "[all]\nbox ansible_connection=local\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{plain}");
    let plain_id = plain["id"].as_str().unwrap();
    let (status, denied, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{plain_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    assert!(denied["error"]
        .as_str()
        .unwrap_or("")
        .contains("this_inventory_has_no_external_source"));

    let (status, unsynced, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Noch nicht synchron",
            "source_type": "git",
            "source": format!("file:///tmp/koreograf-empty-{nanos}")
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{unsynced}");
    let unsynced_id = unsynced["id"].as_str().unwrap();
    let (status, missing, _) = call(
        &app,
        "GET",
        &format!("/api/projects/{unsynced_id}/contents"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{missing}");
    assert!(missing["error"]
        .as_str()
        .unwrap_or("")
        .contains("sync_the_git_project_first"));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/dcim/devices/", axum::routing::get(nb_devices));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Aus NetBox",
            "api_endpoint": format!("http://{addr}"),
            "token": "secret-token",
            "group_by": ["sites"],
            "query_filters": ["status=active"],
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["source_kind"], "netbox");
    assert!(created["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "web-1"));
    assert!(!created["body"]
        .as_str()
        .unwrap_or("")
        .contains("secret-token"));
    let netbox_id = created["id"].as_str().unwrap();
    let (status, again, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{netbox_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert!(again["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "web-1"));

    let (status, bad, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Kaputtes NetBox",
            "api_endpoint": format!("http://{addr}"),
            "token": "wrong-token",
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    assert_eq!(bad["error"], "netbox.bad_status");
    assert_eq!(bad["vars"]["status"], "403", "{bad}");
    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(listed
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["name"] != "Kaputtes NetBox"));

    let _ = std::fs::remove_dir_all(&dir);
}

async fn nb_devices(
    headers: axum::http::HeaderMap,
    request: axum::extract::Request,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some("Token secret-token")
    {
        return (StatusCode::FORBIDDEN, "no").into_response();
    }
    let query = request.uri().query().unwrap_or("");
    if !query.contains("status=active") {
        return (StatusCode::BAD_REQUEST, "filter").into_response();
    }
    axum::Json(json!({
        "next": null,
        "results": [{
            "id": 1,
            "name": "web-1",
            "primary_ip4": {"address": "10.0.0.8/24"},
            "site": {"slug": "berlin"}
        }]
    }))
    .into_response()
}

#[tokio::test]
async fn inventory_sources_and_ansible_check_options() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-mix-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("id=\"inventory-search\""));
    assert!(script.contains("inventory_2"));
    assert!(script.contains("project_2"));
    assert!(script.contains("credential"));
    assert!(script.contains("environment"));
    assert!(script.contains("data-lang-select"));
    assert!(script.contains("FLAG_SVG"));
    assert!(script.contains("country: \"gb\""));
    assert!(script.contains("data-add-kind"));
    assert!(script.contains("check_dry_run"));
    assert!(script.contains("skip_tags"));
    assert!(script.contains("0_normal"));
    assert!(script.contains("1_verbose"));
    assert!(script.contains("2_more_verbose"));
    assert!(script.contains("3_debug"));
    assert!(script.contains("4_connection_debug"));
    assert!(script.contains("data-chips"));
    assert!(script.contains("chip-entry"));
    let (status, style, _) = call(&app, "GET", "/app.css", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let style = style.as_str().unwrap_or("");
    assert!(style.contains(".chips"));
    assert!(style.contains(".chip-entry"));

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
    let (status, projects, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{projects}");
    let demo = projects
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Demo")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Gemischt",
            "sources": [
                {"kind": "ini", "name": "Extra", "body": "[web]\nweb-1 ansible_host=10.0.0.8\n"},
                {"kind": "project", "project_id": demo, "path": "inventory/hosts.ini"}
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["source_kind"], "mixed");
    assert_eq!(created["source_count"], 2);
    let names: Vec<&str> = created["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|host| host["name"].as_str())
        .collect();
    assert!(names.contains(&"web-1"), "{created}");
    assert!(names.contains(&"localhost"), "{created}");
    let inventory_id = created["id"].as_str().unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/dcim/devices/", axum::routing::get(nb_devices));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });
    let (status, with_netbox, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "kind": "netbox",
            "name": "DC",
            "api_endpoint": format!("http://{addr}"),
            "token": "secret-token",
            "group_by": ["sites"],
            "query_filters": ["status=active"],
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{with_netbox}");
    assert_eq!(with_netbox["source_count"], 3);
    assert!(with_netbox["body"].as_str().unwrap_or("").contains("web-1"));
    assert!(!with_netbox["body"]
        .as_str()
        .unwrap_or("")
        .contains("secret-token"));
    let ini_source = with_netbox["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["kind"] == "ini")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, dropped, _) = call(
        &app,
        "DELETE",
        &format!("/api/inventories/{inventory_id}/sources/{ini_source}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dropped}");
    assert_eq!(dropped["source_count"], 2);
    assert!(dropped["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|host| host["name"] == "localhost"));

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Dry-Run",
            "tool": "ansible",
            "project_id": demo,
            "inventory_id": inventory_id,
            "entrypoint": "site.yml",
            "spec": {
                "job_type": "check",
                "tags": "ping",
                "skip_tags": "slow",
                "limit": "web:&db",
                "diff": true,
                "verbosity": 2,
                "forks": 5
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    assert_eq!(template["spec"]["job_type"], "check");
    assert_eq!(template["spec"]["tags"], "ping");
    assert_eq!(template["spec"]["limit"], "web:&db");
    let (status, rejected, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Kaputte Tags",
            "tool": "ansible",
            "project_id": demo,
            "entrypoint": "site.yml",
            "spec": {"tags": "-evil"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn workflow_node_can_override_template_parameters() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-node-params-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("colors_show_the_run_a_click_shows_the"));
    assert!(script.contains("back_to_the_workflow"));
    assert!(script.contains("id=\"run-log\""));
    assert!(script.contains("id=\"node-output\""));
    assert!(!script.contains("Ein Klick öffnet den Auftrag"));
    assert!(!script.contains("#/jobs/${phase.job_id}"));

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

    let (status, projects, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{projects}");
    let demo = projects
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Demo")
        .expect("Demo-Projekt")["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, test_inv, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Testsysteme", "body": "test-1 ansible_connection=local\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{test_inv}");
    let test_id = test_inv["id"].as_str().unwrap().to_string();

    let (status, prod_inv, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Produktion", "body": "prod-1 ansible_connection=local\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{prod_inv}");
    let prod_id = prod_inv["id"].as_str().unwrap().to_string();

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "patch-server",
            "tool": "ansible",
            "project_id": demo,
            "inventory_id": test_id,
            "entrypoint": "site.yml",
            "extra_vars": {"stage": "template"},
            "spec": {"job_type": "run", "limit": "test", "forks": 5}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    let graph = json!({
        "nodes": [
            {
                "id": "test",
                "template_id": template_id,
                "name": "test",
                "x": 40,
                "y": 40,
                "convergence": "any"
            },
            {
                "id": "prod",
                "template_id": template_id,
                "name": "Produktion",
                "x": 280,
                "y": 40,
                "convergence": "any",
                "overrides": {
                    "inventory_id": prod_id,
                    "extra_vars": {"stage": "node"},
                    "spec": {"limit": "prod", "forks": null}
                }
            }
        ],
        "edges": [{"from": "test", "to": "prod", "when": "success"}]
    });
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Patch erst Test, dann Prod",
            "graph": graph
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let workflow_id = created["id"].as_str().unwrap().to_string();
    let prod = created["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "prod")
        .unwrap();
    assert_eq!(prod["overrides"]["inventory_id"], prod_id);
    assert_eq!(prod["overrides"]["spec"]["limit"], "prod");
    assert!(prod["overrides"]["spec"]["forks"].is_null());
    let test_node = created["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "test")
        .unwrap();
    assert!(test_node.get("overrides").is_none(), "{test_node}");

    let (status, missing, _) = call(
        &app,
        "PUT",
        &format!("/api/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Patch erst Test, dann Prod",
            "graph": {
                "nodes": [{
                    "id": "prod",
                    "template_id": template_id,
                    "name": "Produktion",
                    "overrides": {"inventory_id": "missing"}
                }],
                "edges": []
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{missing}");
    assert_eq!(missing["error"], "inventory_is_missing");

    let (status, bad_extra, _) = call(
        &app,
        "PUT",
        &format!("/api/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Patch erst Test, dann Prod",
            "graph": {
                "nodes": [{
                    "id": "prod",
                    "template_id": template_id,
                    "name": "Produktion",
                    "overrides": {"extra_vars": [1]}
                }],
                "edges": []
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_extra}");
    assert_eq!(bad_extra["error"], "extra_variables_must_be_a_json_object");

    let (status, kept, _) = call(
        &app,
        "GET",
        &format!("/api/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["graph"]["edges"][0]["when"], "success");

    // Both nodes are roots so the launch does not wait for Ansible to finish.
    let mut roots = graph.clone();
    roots["edges"] = json!([]);
    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Patch erst Test, dann Prod",
            "graph": roots
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");

    let (status, run, _) = call(
        &app,
        "POST",
        &format!("/api/workflows/{workflow_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {"from": "run"}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{run}");

    let mut test_job = None;
    let mut prod_job = None;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (status, jobs, _) =
            call(&app, "GET", "/api/jobs", Some(&cookie), Some(&team), None).await;
        assert_eq!(status, StatusCode::OK, "{jobs}");
        for job in jobs.as_array().unwrap() {
            if job["workflow_node_id"] == "test" {
                test_job = Some(job.clone());
            }
            if job["workflow_node_id"] == "prod" {
                prod_job = Some(job.clone());
            }
        }
        if test_job.is_some() && prod_job.is_some() {
            break;
        }
    }
    let test_job = test_job.expect("test node job");
    let prod_job = prod_job.expect("prod node job");
    assert_eq!(test_job["parameters"]["inventory_id"], test_id);
    assert_eq!(test_job["parameters"]["inventory_name"], "Testsysteme");
    assert_eq!(test_job["parameters"]["spec"]["limit"], "test");
    assert_eq!(test_job["parameters"]["spec"]["forks"], 5);
    assert_eq!(test_job["parameters"]["extra_vars"]["stage"], "template");
    assert_eq!(test_job["parameters"]["extra_vars"]["from"], "run");
    assert_eq!(prod_job["parameters"]["inventory_id"], prod_id);
    assert_eq!(prod_job["parameters"]["inventory_name"], "Produktion");
    assert_eq!(prod_job["parameters"]["spec"]["limit"], "prod");
    assert_eq!(prod_job["parameters"]["spec"]["job_type"], "run");
    assert!(prod_job["parameters"]["spec"].get("forks").is_none());
    assert_eq!(prod_job["parameters"]["extra_vars"]["stage"], "node");
    assert_eq!(prod_job["parameters"]["extra_vars"]["from"], "run");

    let (status, templates, _) = call(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{templates}");
    let unchanged = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == template_id)
        .expect("Vorlage bleibt erhalten");
    assert_eq!(unchanged["inventory_id"], test_id);
    assert_eq!(unchanged["spec"]["limit"], "test");
    assert_eq!(unchanged["spec"]["forks"], 5);
    assert_eq!(unchanged["extra_vars"]["stage"], "template");

    let run_id = run["id"].as_str().unwrap();
    let mut prod_node = None;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (status, live, _) = call(
            &app,
            "GET",
            &format!("/api/workflow-jobs/{run_id}"),
            Some(&cookie),
            Some(&team),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{live}");
        let node = live["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["node_id"] == "prod")
            .cloned();
        if node.as_ref().is_some_and(|item| item["job_id"].is_string()) {
            prod_node = node;
            break;
        }
    }
    let prod_node = prod_node.expect("prod node on the run");
    assert!(prod_node["job_id"].is_string(), "{prod_node}");
    assert!(prod_node.get("started_at").is_some(), "{prod_node}");
    assert!(prod_node.get("finished_at").is_some(), "{prod_node}");
    assert_eq!(prod_node["tool"], "ansible");
    assert_eq!(prod_node["job_name"], "patch-server");

    let _ = std::fs::remove_dir_all(&dir);
}

async fn call_raw(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    extra: &[(&str, &str)],
    body: Option<Vec<u8>>,
) -> (StatusCode, Vec<u8>, axum::http::HeaderMap) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    for (key, value) in extra {
        builder = builder.header(*key, *value);
    }
    let request = match body {
        Some(body) => builder.body(Body::from(body)).unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec(), headers)
}

#[tokio::test]
async fn admin_backup_roundtrip_replaces_the_installation() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-backup-http-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("download_backup"));
    assert!(script.contains("#/backup"));
    assert!(script.contains("X-Restore-Confirm"));

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

    let (status, archive, headers) =
        call_raw(&app, "GET", "/api/admin/backup", Some(&cookie), &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/zip")
    );
    assert!(archive.starts_with(b"PK"), "zip magic");

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        None,
        Some(json!({"name": "NurKurz"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        None,
        Some(json!({
            "username": "viewer",
            "password": "viewer-pass",
            "display_name": "Viewer"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, viewer, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "viewer", "password": "viewer-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer}");
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    let (status, denied, _) = call_raw(
        &app,
        "GET",
        "/api/admin/backup",
        Some(&viewer_cookie),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!denied
        .windows(b"secret.key".len())
        .any(|window| window == b"secret.key"));

    let (status, rejected, _) = call_raw(
        &app,
        "POST",
        "/api/admin/backup",
        Some(&cookie),
        &[],
        Some(archive.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let rejected = String::from_utf8_lossy(&rejected);
    assert!(
        rejected.contains("confirm_the_replacement_with_ersetzen"),
        "{rejected}"
    );

    let (status, orgs, _) =
        call(&app, "GET", "/api/organizations", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{orgs}");
    assert!(orgs
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "NurKurz"));

    let (status, restored, headers) = call_raw(
        &app,
        "POST",
        "/api/admin/backup",
        Some(&cookie),
        &[("x-restore-confirm", "ERSETZEN")],
        Some(archive),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&restored)
    );
    assert!(String::from_utf8_lossy(&restored).contains("\"restored\":true"));
    let set_cookie = headers
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(set_cookie.contains("Max-Age=0"), "{set_cookie}");

    let (status, orgs, _) =
        call(&app, "GET", "/api/organizations", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{orgs}");
    assert!(orgs
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["name"] != "NurKurz"));
    assert!(std::path::Path::new(&dir)
        .join("restore-previous.zip")
        .is_file());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn admin_can_list_and_update_users() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-users-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
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
    assert_eq!(status, StatusCode::BAD_REQUEST, "{blocked}");
    assert_eq!(blocked["error"], "the_account_is_disabled");
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
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-account-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
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

    let (status, saved, _) = call(
        &app,
        "PUT",
        "/api/me",
        Some(&nia_cookie),
        None,
        Some(json!({
            "display_name": "Nia Klein",
            "email": "nia@example.com",
            "is_admin": true
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
async fn owner_can_edit_an_execution_environment() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-env-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    let runtime = script
        .split_once("async function pageRuntime")
        .expect("runtime page")
        .1
        .split_once("function environmentForm")
        .expect("environment form")
        .0;
    let environments = runtime
        .find("execution_environments")
        .expect("environment heading");
    let collections = runtime.find("cached_collections").expect("cache heading");
    assert!(environments < collections);
    assert!(runtime.contains("#/environments/"));

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
    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/environments",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let id = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "AWX-EE")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let image = "quay.io/ansible/awx-ee@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "EE neu",
            "image": image,
            "pull_policy": "always",
            "description": "angepasst"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "EE neu");
    assert_eq!(updated["image"], image);
    assert_eq!(updated["pull_policy"], "always");
    assert_eq!(updated["scope"], "team");
    assert_eq!(updated["team_id"].as_str(), Some(team.as_str()));
    assert!(updated["organization_id"].is_null());
    let (status, again, _) = call(
        &app,
        "GET",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["description"], "angepasst");
    let (status, rejected, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "EE neu",
            "image": "bad image",
            "pull_policy": "always"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["error"], "the_image_name_is_invalid");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn environment_scopes_follow_the_selected_team() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-env-scope-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("applies_to"));
    assert!(script.contains("this_team"));
    assert!(script.contains("this_organization"));
    assert!(script.contains("organization_name"));
    assert!(script.contains("team_name"));
    assert!(script.contains("global_applies_to_every_team_the_organization_applies"));
    assert!(script.contains("canManageEnvironment"));
    assert!(script.contains("scopeLabel(row)"));

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
    let org_id = me["teams"][0]["organization_id"]
        .as_str()
        .unwrap()
        .to_string();
    let org_name = me["teams"][0]["organization_name"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, other_org, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Andere", "description": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other_org}");
    let other_org_id = other_org["id"].as_str().unwrap();
    let (status, sibling, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Betrieb",
            "description": "Gleiches Organisation",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{sibling}");
    let sibling = sibling["id"].as_str().unwrap().to_string();
    let (status, foreign, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Nacht",
            "description": "Andere Organisation",
            "organization_id": other_org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{foreign}");
    let foreign = foreign["id"].as_str().unwrap().to_string();

    let (status, _user, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "ada",
            "display_name": "Ada",
            "password": "ada-secret-1",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    for (target, role) in [(&team, "owner"), (&sibling, "owner")] {
        let (status, joined, _) = call(
            &app,
            "POST",
            &format!("/api/teams/{target}/members"),
            Some(&cookie),
            Some(&team),
            Some(json!({"username": "ada", "role": role})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{joined}");
    }
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
    let ada_cookie = ada_cookie.expect("ada cookie");

    let (status, private_env, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Privat",
            "image": "quay.io/example/private:1",
            "pull_policy": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{private_env}");
    assert_eq!(private_env["scope"], "team");
    assert_eq!(private_env["team_id"].as_str(), Some(team.as_str()));
    assert!(private_env["organization_id"].is_null());
    let private_id = private_env["id"].as_str().unwrap().to_string();

    let (status, org_env, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Orga",
            "image": "quay.io/example/orga:1",
            "scope": "organization"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org_env}");
    assert_eq!(org_env["scope"], "organization");
    assert_eq!(org_env["organization_id"], org_id);
    assert_eq!(org_env["organization_name"], org_name);
    assert!(org_env["team_id"].is_null());
    let org_env_id = org_env["id"].as_str().unwrap().to_string();

    let (status, denied_global, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Nicht global",
            "image": "quay.io/example/nope:1",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied_global}");
    assert_eq!(denied_global["error"], "you_are_not_allowed_to_do_this");
    let (status, bad_scope, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Kaputt",
            "image": "quay.io/example/bad:1",
            "scope": "world"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_scope}");
    assert_eq!(bad_scope["error"], "scope_is_team_organization_or_global");

    let (status, global_env, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Geteilt",
            "image": "quay.io/example/global:1",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{global_env}");
    assert_eq!(global_env["scope"], "global");
    assert!(global_env["team_id"].is_null());
    assert!(global_env["organization_id"].is_null());
    let global_id = global_env["id"].as_str().unwrap().to_string();

    let names = |list: &Value| {
        list.as_array()
            .unwrap()
            .iter()
            .map(|item| item["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let (status, home, _) = call(
        &app,
        "GET",
        "/api/environments",
        Some(&ada_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{home}");
    let home_names = names(&home);
    assert!(home_names.contains(&"AWX-EE".to_string()), "{home}");
    assert!(home_names.contains(&"Privat".to_string()));
    assert!(home_names.contains(&"Orga".to_string()));
    assert!(home_names.contains(&"Geteilt".to_string()));
    let rank = |scope: &str| match scope {
        "global" => 0,
        "organization" => 1,
        "team" => 2,
        _ => 9,
    };
    let scopes = home
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["scope"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(scopes.windows(2).all(|pair| rank(pair[0]) <= rank(pair[1])));

    let (status, same_org, _) = call(
        &app,
        "GET",
        "/api/environments",
        Some(&ada_cookie),
        Some(&sibling),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{same_org}");
    let same_names = names(&same_org);
    assert!(same_names.contains(&"Geteilt".to_string()), "{same_org}");
    assert!(same_names.contains(&"Orga".to_string()), "{same_org}");
    assert!(!same_names.iter().any(|name| name == "Privat"));
    assert!(!same_names.iter().any(|name| name == "AWX-EE"));

    let (status, other, _) = call(
        &app,
        "GET",
        "/api/environments",
        Some(&cookie),
        Some(&foreign),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{other}");
    let other_names = names(&other);
    assert!(other_names.contains(&"Geteilt".to_string()), "{other}");
    assert!(!other_names.iter().any(|name| name == "Orga"));
    assert!(!other_names.iter().any(|name| name == "Privat"));
    assert!(!other_names.iter().any(|name| name == "AWX-EE"));

    let (status, seen, _) = call(
        &app,
        "GET",
        &format!("/api/environments/{global_id}"),
        Some(&ada_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{seen}");
    assert_eq!(seen["scope"], "global");
    let (status, forbidden, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{global_id}"),
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Geteilt",
            "image": "quay.io/example/global:1",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{forbidden}");
    assert_eq!(forbidden["error"], "you_are_not_allowed_to_do_this");
    let (status, _, _) = call(
        &app,
        "DELETE",
        &format!("/api/environments/{global_id}"),
        Some(&ada_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, hidden, _) = call(
        &app,
        "GET",
        &format!("/api/environments/{private_id}"),
        Some(&cookie),
        Some(&foreign),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");

    let (status, kept, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{org_env_id}"),
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Orga",
            "image": "quay.io/example/orga:2",
            "description": "bleibt"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["scope"], "organization");
    assert_eq!(kept["image"], "quay.io/example/orga:2");
    let (status, promote, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{org_env_id}"),
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Orga",
            "image": "quay.io/example/orga:2",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{promote}");
    let (status, still_org, _) = call(
        &app,
        "GET",
        &format!("/api/environments/{org_env_id}"),
        Some(&ada_cookie),
        Some(&sibling),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{still_org}");
    assert_eq!(still_org["scope"], "organization");
    let (status, sibling_edit, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{org_env_id}"),
        Some(&ada_cookie),
        Some(&sibling),
        Some(json!({
            "name": "Orga",
            "image": "quay.io/example/orga:3",
            "description": "geschwister"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sibling_edit}");
    assert_eq!(sibling_edit["scope"], "organization");
    assert_eq!(sibling_edit["organization_id"], org_id);

    let shell = |name: &str, environment_id: &str| {
        json!({
            "name": name,
            "tool": "shell",
            "entrypoint": "",
            "environment_id": environment_id,
            "spec": {"script": "true"}
        })
    };
    let (status, used, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&foreign),
        Some(shell("Mit Global", &global_id)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{used}");
    assert_eq!(used["environment_id"], global_id);
    let (status, blocked, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&foreign),
        Some(shell("Mit Orga", &org_env_id)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{blocked}");
    assert_eq!(
        blocked["error"],
        "the_execution_environment_is_not_visible_to_this"
    );
    let (status, allowed, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&ada_cookie),
        Some(&sibling),
        Some(shell("Mit Orga erlaubt", &org_env_id)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{allowed}");
    assert_eq!(allowed["environment_id"], org_env_id);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn owner_can_store_an_ssh_password_without_a_key() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-cred-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("machine_ssh"));
    assert!(script.contains("amazon_web_services"));
    assert!(script.contains("a_key_or_a_password_or_both"));
    assert!(script.contains("applies_to_the_job_project_sync_does_not"));
    assert!(script.contains("leave_empty_to_keep_the_stored_value"));
    assert!(script.contains("data-stored-mask=\"1\""));
    assert!(script.contains("class=\"secret-mask\""));
    assert!(!script.contains("*** bedeutet, dass ein Wert gespeichert ist."));
    assert!(script.contains("the_type_cannot_be_changed"));
    assert!(script.contains("#/credentials/${esc(item.id)}"));

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
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "SSH Passwort",
            "kind": "ssh",
            "secret": {"username": "deploy", "password": "ssh-only-pass"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["kind"], "ssh");
    assert!(
        !created.to_string().contains("ssh-only-pass"),
        "response echoed the secret"
    );
    let id = created["id"].as_str().unwrap();
    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(
        !listed.to_string().contains("ssh-only-pass"),
        "list echoed the secret"
    );
    assert!(listed
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == id && item["kind"] == "ssh"));
    let (status, one, _) = call(
        &app,
        "GET",
        &format!("/api/credentials/{id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{one}");
    assert_eq!(one["fields"]["username"], "deploy");
    assert_eq!(one["set"], json!(["password"]));
    assert!(
        !one.to_string().contains("ssh-only-pass"),
        "credential view echoed the secret"
    );
    let (status, kept, _) = call(
        &app,
        "PUT",
        &format!("/api/credentials/{id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "SSH Passwort neu",
            "secret": {"username": "deploy", "password": ""}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["name"], "SSH Passwort neu");
    assert_eq!(kept["set"], json!(["password"]));
    assert!(!kept.to_string().contains("ssh-only-pass"));
    let (status, replaced, _) = call(
        &app,
        "PUT",
        &format!("/api/credentials/{id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "SSH Passwort neu",
            "secret": {
                "username": "deploy",
                "password": "ssh-new-pass",
                "private_key": "KEYMATERIAL"
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replaced}");
    assert_eq!(replaced["set"], json!(["password", "private_key"]));
    let replaced_text = replaced.to_string();
    assert!(!replaced_text.contains("ssh-new-pass"));
    assert!(!replaced_text.contains("ssh-only-pass"));
    assert!(!replaced_text.contains("KEYMATERIAL"));
    let (status, cleared, _) = call(
        &app,
        "PUT",
        &format!("/api/credentials/{id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "SSH Passwort neu",
            "secret": {"username": "deploy"},
            "clear": ["password", "private_key"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{cleared}");
    assert_eq!(cleared["error"], "ssh_needs_a_key_or_a_password");
    let (status, still, _) = call(
        &app,
        "GET",
        &format!("/api/credentials/{id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{still}");
    assert_eq!(still["set"], json!(["password", "private_key"]));
    assert!(!still.to_string().contains("KEYMATERIAL"));
    let (status, rejected, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Leer", "kind": "ssh", "secret": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["error"], "ssh_needs_a_key_or_a_password");
    let (status, unknown, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Fremd", "kind": "gpg", "secret": {"token": "x"}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{unknown}");
    assert_eq!(unknown["error"], "unknown_credential_type");
    let (status, gone, _) = call(
        &app,
        "DELETE",
        &format!("/api/credentials/{id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{gone}");

    let _ = std::fs::remove_dir_all(&dir);
}

fn shell_with_credentials(name: &str, credential_ids: Value) -> Value {
    json!({
        "name": name,
        "tool": "shell",
        "spec": {"script": "echo ok"},
        "credential_ids": credential_ids
    })
}

fn assert_no_secrets(body: &Value) {
    let rendered = body.to_string();
    for secret in [
        "ssh-eins-geheim",
        "ssh-zwei-geheim",
        "vault-prod-geheim",
        "vault-stage-geheim",
        "vault-plain-geheim",
    ] {
        assert!(
            !rendered.contains(secret),
            "response echoed a credential secret"
        );
    }
}

async fn create_credential(
    app: &axum::Router,
    cookie: &str,
    team: &str,
    name: &str,
    kind: &str,
    secret: Value,
) -> String {
    let (status, created, _) = call(
        app,
        "POST",
        "/api/credentials",
        Some(cookie),
        Some(team),
        Some(json!({"name": name, "kind": kind, "secret": secret})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        created.get("error").unwrap_or(&Value::Null)
    );
    assert_no_secrets(&created);
    created["id"].as_str().expect("credential id").to_string()
}

#[tokio::test]
async fn template_accepts_one_credential_per_kind_and_many_vaults() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-vault-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("one_credential_of_each_kind_choose_vault_credentials"));
    assert!(script.contains("applies_only_to_this_node_the_template_stays"));
    assert!(script.contains("id=\"vault-pills\""));
    assert!(script.contains("no_vault_credentials_in_this_team"));
    assert!(script.contains("remove_vault_credential_name"));

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

    let ssh = create_credential(
        &app,
        &cookie,
        &team,
        "SSH eins",
        "ssh",
        json!({"username": "deploy", "password": "ssh-eins-geheim"}),
    )
    .await;
    let ssh_other = create_credential(
        &app,
        &cookie,
        &team,
        "SSH zwei",
        "ssh",
        json!({"username": "deploy", "password": "ssh-zwei-geheim"}),
    )
    .await;
    let vault_prod = create_credential(
        &app,
        &cookie,
        &team,
        "Vault prod",
        "vault",
        json!({"password": "vault-prod-geheim", "vault_id": "prod"}),
    )
    .await;
    let vault_stage = create_credential(
        &app,
        &cookie,
        &team,
        "Vault stage",
        "vault",
        json!({"password": "vault-stage-geheim", "vault_id": "stage"}),
    )
    .await;
    let vault_prod_again = create_credential(
        &app,
        &cookie,
        &team,
        "Vault prod nochmal",
        "vault",
        json!({"password": "vault-prod-geheim", "vault_id": "prod"}),
    )
    .await;
    let vault_plain = create_credential(
        &app,
        &cookie,
        &team,
        "Vault ohne Id",
        "vault",
        json!({"password": "vault-plain-geheim"}),
    )
    .await;
    let vault_plain_two = create_credential(
        &app,
        &cookie,
        &team,
        "Vault ohne Id zwei",
        "vault",
        json!({"password": "vault-plain-geheim"}),
    )
    .await;

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(shell_with_credentials(
            "Vault-Zuordnung",
            json!([ssh, vault_prod, vault_stage]),
        )),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        created.get("error").unwrap_or(&Value::Null)
    );
    assert_no_secrets(&created);
    assert_eq!(created["credential_id"], ssh);
    let ids = created["credential_ids"]
        .as_array()
        .expect("credential_ids");
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[0], ssh);
    assert_eq!(ids[1], vault_prod);
    assert_eq!(ids[2], vault_stage);
    let template_id = created["id"].as_str().unwrap().to_string();

    let (status, single, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Alter Zugang",
            "tool": "shell",
            "spec": {"script": "echo ok"},
            "credential_id": ssh
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{single}");
    assert_eq!(single["credential_id"], ssh);
    assert_eq!(single["credential_ids"], json!([ssh]));

    let (status, second_ssh, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        Some(shell_with_credentials(
            "Vault-Zuordnung",
            json!([ssh, ssh_other]),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{second_ssh}");
    assert_eq!(
        second_ssh["error"],
        "only_one_credential_of_each_kind_is_allowed"
    );

    let (status, same_label, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        Some(shell_with_credentials(
            "Vault-Zuordnung",
            json!([vault_prod, vault_prod_again]),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{same_label}");
    assert_eq!(same_label["error"], "this_vault_id_is_already_selected");

    let (status, two_plain, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        Some(shell_with_credentials(
            "Vault-Zuordnung",
            json!([vault_plain, vault_plain_two]),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{two_plain}");
    assert_eq!(
        two_plain["error"],
        "only_one_vault_credential_may_omit_the_vault"
    );

    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let saved = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == template_id)
        .expect("template");
    assert_eq!(saved["credential_id"], ssh);
    assert_eq!(saved["credential_ids"].as_array().unwrap().len(), 3);

    let (status, job, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    assert_no_secrets(&job);
    let launched = job["parameters"]["credentials"]
        .as_array()
        .expect("credentials");
    assert_eq!(launched.len(), 3);
    assert_eq!(launched[0]["id"], ssh);
    assert_eq!(launched[0]["name"], "SSH eins");
    assert_eq!(launched[0]["kind"], "ssh");
    assert_eq!(launched[1]["id"], vault_prod);
    assert_eq!(launched[1]["name"], "Vault prod");
    assert_eq!(launched[1]["kind"], "vault");
    assert_eq!(launched[2]["id"], vault_stage);
    assert_eq!(launched[2]["kind"], "vault");
    let job_id = job["id"].as_str().unwrap();
    let (status, stored, _) = call(
        &app,
        "GET",
        &format!("/api/jobs/{job_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_no_secrets(&stored);
    assert_eq!(
        stored["parameters"]["credentials"]
            .as_array()
            .unwrap()
            .len(),
        3
    );

    let (status, workflow, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Anderer SSH-Zugang",
            "graph": {
                "nodes": [{
                    "id": "step",
                    "template_id": template_id,
                    "name": "Mit anderem SSH",
                    "overrides": {"credential_id": ssh_other}
                }],
                "edges": []
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let workflow_id = workflow["id"].as_str().unwrap();
    let (status, run, _) = call(
        &app,
        "POST",
        &format!("/api/workflows/{workflow_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{run}");

    let mut node_job = None;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (status, jobs, _) =
            call(&app, "GET", "/api/jobs", Some(&cookie), Some(&team), None).await;
        assert_eq!(status, StatusCode::OK, "{jobs}");
        if let Some(found) = jobs
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["workflow_node_id"] == "step")
        {
            node_job = Some(found.clone());
            break;
        }
    }
    let node_job = node_job.expect("workflow node job");
    assert_no_secrets(&node_job);
    let overridden = node_job["parameters"]["credentials"]
        .as_array()
        .expect("credentials");
    assert_eq!(overridden.len(), 3);
    let overridden_ids: Vec<&str> = overridden
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert!(overridden_ids.contains(&ssh_other.as_str()));
    assert!(overridden_ids.contains(&vault_prod.as_str()));
    assert!(overridden_ids.contains(&vault_stage.as_str()));
    assert!(!overridden_ids.contains(&ssh.as_str()));
    assert!(overridden
        .iter()
        .any(|item| item["id"] == ssh_other && item["kind"] == "ssh"));
    assert_eq!(
        overridden
            .iter()
            .filter(|item| item["kind"] == "vault")
            .count(),
        2
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn admin_settings_hide_secrets_and_apply_without_restart() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-settings-smoke-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("#/settings"));
    assert!(script.contains("settings"));
    assert!(script.contains("bind_password"));
    assert!(script.contains("proxy_address"));
    assert!(script.contains("use_environment_variables"));
    assert!(script.contains("data-stored-mask=\"1\""));
    assert!(!script.contains("bind-secret-value"));
    assert!(!script.contains("proxy-secret-value"));

    let (status, denied, _) = call(&app, "GET", "/api/settings", None, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{denied}");
    assert_eq!(denied["error"], "sign_in_is_required");

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
    let (status, forbidden, _) =
        call(&app, "GET", "/api/settings", Some(&ada_cookie), None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{forbidden}");
    assert_eq!(forbidden["error"], "you_are_not_allowed_to_do_this");

    let (status, current, _) = call(&app, "GET", "/api/settings", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{current}");
    assert_eq!(current["source"], "env");
    assert_eq!(current["runner"], "local");
    assert_eq!(current["ldap"]["url"], "");
    assert_eq!(current["ldap"]["ca_file"], "");
    assert_eq!(current["ldap"]["bind_password_set"], false);
    assert_eq!(current["proxy"]["password_set"], false);
    assert!(!current.to_string().contains("password\":"));

    let (status, blocked, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&ada_cookie),
        None,
        Some(json!({"reset": true})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{blocked}");

    let body = json!({
        "ldap": {
            "url": "ldap://ldap.example.com",
            "mode": "ad",
            "bind_dn": "cn=svc,dc=example,dc=com",
            "bind_password": "bind-secret-value",
            "user_base": "ou=users,dc=example,dc=com",
            "starttls": true,
            "tls_insecure": false
        },
        "proxy": {
            "url": "http://proxy.example:3128",
            "username": "proxy-user",
            "password": "proxy-secret-value",
            "no_proxy": "localhost,127.0.0.1"
        },
        "runner": "docker",
        "job_timeout_secs": 90,
        "job_output_lines": 4000
    });
    let (status, saved, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let saved_text = saved.to_string();
    assert!(!saved_text.contains("bind-secret-value"), "{saved_text}");
    assert!(!saved_text.contains("proxy-secret-value"), "{saved_text}");
    assert_eq!(saved["source"], "saved");
    assert_eq!(saved["ldap"]["mode"], "ad");
    assert_eq!(saved["ldap"]["ca_file"], "");
    assert_eq!(saved["ldap"]["user_filter"], "(sAMAccountName={username})");
    assert_eq!(saved["ldap"]["bind_password_set"], true);
    assert_eq!(saved["proxy"]["password_set"], true);
    assert_eq!(saved["proxy"]["url"], "http://proxy.example:3128");
    assert_eq!(saved["runner"], "docker");
    assert_eq!(saved["job_timeout_secs"], 90);
    assert_eq!(saved["job_output_lines"], 4000);
    assert!(saved["ldap"].get("bind_password").is_none());
    assert!(saved["proxy"].get("password").is_none());

    let (status, overview, _) = call(
        &app,
        "GET",
        "/api/overview",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert_eq!(overview["ldap"], true);
    assert_eq!(overview["runner_mode"], "docker");

    let (status, kept, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(json!({
            "ldap": {
                "url": "ldap://ldap.example.com",
                "mode": "ad",
                "bind_dn": "cn=svc,dc=example,dc=com",
                "bind_password": "",
                "user_base": "ou=users,dc=example,dc=com"
            },
            "proxy": {
                "url": "http://proxy.example:3128",
                "username": "proxy-user",
                "password": "",
                "no_proxy": "localhost,127.0.0.1"
            },
            "runner": "docker",
            "job_timeout_secs": 90
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["ldap"]["bind_password_set"], true);
    assert_eq!(kept["proxy"]["password_set"], true);

    let (status, rejected, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(json!({
            "ldap": {
                "url": "ldap://ldap.example.com",
                "mode": "ad",
                "bind_dn": "cn=svc,dc=example,dc=com",
                "user_base": "ou=users,dc=example,dc=com"
            },
            "proxy": {
                "url": "http://user:secret@proxy.example:3128",
                "username": "proxy-user"
            },
            "runner": "local",
            "job_timeout_secs": 45
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(
        rejected["error"],
        "put_the_credentials_in_their_own_fields_not"
    );
    let (status, still, _) = call(&app, "GET", "/api/settings", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{still}");
    assert_eq!(still["runner"], "docker");
    assert_eq!(still["proxy"]["password_set"], true);
    assert!(!still.to_string().contains("proxy-secret-value"));

    let (status, cleared, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(json!({
            "ldap": {
                "url": "",
                "mode": "ldap",
                "bind_dn": "cn=svc,dc=example,dc=com",
                "user_base": "ou=users,dc=example,dc=com"
            },
            "proxy": {
                "url": "http://proxy.example:3128",
                "username": "proxy-user",
                "no_proxy": "localhost"
            },
            "runner": "auto",
            "job_timeout_secs": 30,
            "clear": ["ldap_bind_password", "proxy_password"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["ldap"]["url"], "");
    assert_eq!(cleared["ldap"]["bind_password_set"], false);
    assert_eq!(cleared["proxy"]["password_set"], false);
    assert_eq!(cleared["runner"], "auto");

    let (status, overview, _) = call(
        &app,
        "GET",
        "/api/overview",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert_eq!(overview["ldap"], false);
    assert_eq!(overview["runner_mode"], "auto");

    let (status, reset, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(json!({"reset": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reset}");
    assert_eq!(reset["source"], "env");
    assert_eq!(reset["runner"], "local");
    assert_eq!(reset["job_timeout_secs"], 60);
    assert_eq!(reset["job_output_lines"], 8000);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_template_can_run_the_last_sync_offline() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-offline-{nanos}"));
    let origin = std::env::temp_dir().join(format!("koreograf-offline-origin-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&origin).unwrap();
    let mut cfg = config(&dir);
    cfg.project_roots.push(origin.clone());
    let state = koreograf::boot(cfg).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("id=\"update-on-launch\""));
    assert!(script.contains("update_the_project_on_launch"));

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

    git(&origin, &["init", "-b", "main"]);
    git(&origin, &["config", "user.email", "test@example.com"]);
    git(&origin, &["config", "user.name", "Koreograf Test"]);
    std::fs::write(origin.join("marker.txt"), "synced\n").unwrap();
    git(&origin, &["add", "marker.txt"]);
    git(&origin, &["commit", "-m", "synced"]);
    let url = format!("file://{}", origin.display());

    let (status, project, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Offline", "source_type": "git", "source": url})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap();
    let (status, synced, _) = call(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");

    std::fs::write(origin.join("marker.txt"), "newer\n").unwrap();
    git(&origin, &["add", "marker.txt"]);
    git(&origin, &["commit", "-m", "newer"]);

    let (status, defaults, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Default",
            "tool": "shell",
            "project_id": project_id,
            "spec": {"script": "echo ok"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{defaults}");
    assert_eq!(defaults["update_on_launch"], true);

    let (status, offline, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Keep",
            "tool": "shell",
            "project_id": project_id,
            "update_on_launch": false,
            "spec": {"script": "cat marker.txt"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{offline}");
    assert_eq!(offline["update_on_launch"], false);
    let offline_id = offline["id"].as_str().unwrap();
    let kept = launch_and_wait(&app, &cookie, &team, offline_id).await;
    assert_eq!(kept["status"], "successful", "{}", kept["events"]);
    assert_eq!(kept["parameters"]["update_on_launch"], false);
    assert_eq!(kept["parameters"]["project_name"], "Offline");
    assert!(has_line(
        &kept,
        "system",
        "Using the checkout from the last sync."
    ));
    assert!(has_line(&kept, "stdout", "synced"));
    assert!(!has_line(&kept, "stdout", "newer"));

    let (status, fresh, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Fresh",
            "tool": "shell",
            "project_id": project_id,
            "update_on_launch": true,
            "spec": {"script": "cat marker.txt"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{fresh}");
    let fresh_id = fresh["id"].as_str().unwrap();
    let updated = launch_and_wait(&app, &cookie, &team, fresh_id).await;
    assert_eq!(updated["status"], "successful", "{}", updated["events"]);
    assert_eq!(updated["parameters"]["update_on_launch"], true);
    assert!(!has_line(
        &updated,
        "system",
        "Using the checkout from the last sync."
    ));
    assert!(has_line(&updated, "stdout", "newer"));

    let missing = format!(
        "file://{}/koreograf-missing-{}",
        std::env::temp_dir().display(),
        nanos
    );
    let (status, absent, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Absent", "source_type": "git", "source": missing})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{absent}");
    let absent_id = absent["id"].as_str().unwrap();
    let (status, blocked, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Blocked",
            "tool": "shell",
            "project_id": absent_id,
            "update_on_launch": false,
            "spec": {"script": "echo should-not-run"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{blocked}");
    let blocked_id = blocked["id"].as_str().unwrap();
    let failed = launch_and_wait(&app, &cookie, &team, blocked_id).await;
    assert_eq!(failed["status"], "failed", "{}", failed["events"]);
    assert!(has_line(&failed, "system", "Sync the Git project first."));
    assert!(!has_line(&failed, "stdout", "should-not-run"));

    let _ = std::fs::remove_dir_all(&origin);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_operator_cannot_start_a_local_job_or_an_arbitrary_path() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-local-admin-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = config(&dir);
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
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();
    let (status, templates, _) = call(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{templates}");
    let health_id = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Healthcheck")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "operator",
            "display_name": "operator",
            "password": "operator-pass",
            "is_admin": false
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
    let (status, operator_me, operator_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "operator", "password": "operator-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{operator_me}");
    let operator_cookie = operator_cookie.expect("operator cookie");

    let (status, denied, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{health_id}/launch"),
        Some(&operator_cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    assert_eq!(denied["error"], "local_runs_are_limited_to_administrators");

    let outside = std::env::temp_dir().join(format!("koreograf-outside-{nanos}"));
    std::fs::create_dir_all(&outside).unwrap();
    let (status, path_denied, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&operator_cookie),
        Some(&team),
        Some(json!({
            "name": "Operator path",
            "source_type": "path",
            "source": outside.display().to_string()
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{path_denied}");
    assert_eq!(
        path_denied["error"],
        "path_projects_are_limited_to_administrators"
    );
    let (status, etc_denied, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Etc",
            "source_type": "path",
            "source": "/etc"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{etc_denied}");
    assert_eq!(etc_denied["error"], "the_project_path_is_not_allowed");

    let health = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Healthcheck")
        .unwrap()
        .clone();
    let (status, edited, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{health_id}"),
        Some(&operator_cookie),
        Some(&team),
        Some(health.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    let (status, blocked, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{health_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{blocked}");
    assert_eq!(blocked["error"], "local_runs_are_limited_to_administrators");
    let (status, restored, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{health_id}"),
        Some(&cookie),
        Some(&team),
        Some(health),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");

    let finished = launch_and_wait(&app, &cookie, &team, &health_id).await;
    assert_eq!(finished["status"], "successful", "{}", finished["events"]);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&outside);
}

async fn launch_and_wait(app: &axum::Router, cookie: &str, team: &str, template_id: &str) -> Value {
    let (status, job, _) = call(
        app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(cookie),
        Some(team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    let job_id = job["id"].as_str().unwrap();
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let (status, body, _) = call(
            app,
            "GET",
            &format!("/api/jobs/{job_id}"),
            Some(cookie),
            Some(team),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if matches!(
            body["status"].as_str(),
            Some("successful" | "failed" | "canceled")
        ) {
            return body;
        }
    }
    panic!("job {job_id} did not finish");
}

fn has_line(job: &Value, stream: &str, line: &str) -> bool {
    job["events"].as_array().is_some_and(|events| {
        events
            .iter()
            .any(|event| event["stream"] == stream && event["line"] == line)
    })
}

#[tokio::test]
async fn a_rejected_sign_in_uses_one_message_and_keeps_the_username() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-signin-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
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
async fn a_request_without_json_returns_a_catalog_error() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-json-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, bytes, _) = call_raw(&app, "POST", "/api/login", None, &[], None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let missing: Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(missing["error"], "json_body_required");
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("Expected request"), "{text}");

    let (status, bytes, _) = call_raw(
        &app,
        "POST",
        "/api/login",
        None,
        &[("content-type", "text/plain")],
        Some(b"{}".to_vec()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let wrong_type: Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(wrong_type["error"], "json_body_required");

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
    let (status, templates, _) = call(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{templates}");
    let health_id = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Healthcheck")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let (status, bytes, _) = call_raw(
        &app,
        "POST",
        &format!("/api/templates/{health_id}/launch"),
        Some(&cookie),
        &[("x-team-id", team)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let launch: Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(launch["error"], "json_body_required");

    let finished = launch_and_wait(&app, &cookie, team, health_id).await;
    assert_eq!(finished["status"], "successful", "{}", finished["events"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_relative_data_dir_can_run_a_local_shell_job() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let relative = std::path::PathBuf::from(format!("target/koreograf-rel-{nanos}"));
    assert!(relative.is_relative());
    std::fs::create_dir_all(&relative).unwrap();
    let state = koreograf::boot(config(&relative)).unwrap();
    let data_dir = state.cfg.data_dir.clone();
    assert!(data_dir.is_absolute(), "{}", data_dir.display());
    assert!(
        data_dir.ends_with(format!("koreograf-rel-{nanos}")),
        "{}",
        data_dir.display()
    );
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
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap();
    let (status, templates, _) = call(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{templates}");
    let health_id = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Healthcheck")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let job = launch_and_wait(&app, &cookie, team, health_id).await;
    assert_eq!(job["status"], "successful", "{}", job["events"]);
    assert!(
        has_line(&job, "stdout", "healthcheck ok"),
        "{}",
        job["events"]
    );
    assert!(has_line(
        &job,
        "stdout",
        "KOREOGRAF Shell script: health.sh"
    ));
    assert!(has_line(&job, "system", "Local run on the controller."));
    let events = job["events"].to_string();
    assert!(!events.contains("Shell-Skript"), "{events}");
    assert!(!events.contains("cannot open"), "{events}");
    let _ = std::fs::remove_dir_all(&data_dir);
}

#[tokio::test]
async fn cancel_stops_the_local_process_group() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-cancel-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
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
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap();
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Sleeper",
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": "sleep 30\necho should-not-run\n"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let template_id = created["id"].as_str().unwrap();
    let (status, job, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    let job_id = job["id"].as_str().unwrap();
    let mut pid = String::new();
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (status, body, _) = call(
            &app,
            "GET",
            &format!("/api/jobs/{job_id}"),
            Some(&cookie),
            Some(team),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if let Some(found) = body["events"].as_array().and_then(|events| {
            events.iter().find_map(|event| {
                event["line"]
                    .as_str()
                    .and_then(|line| line.strip_prefix("Process group "))
                    .and_then(|line| line.strip_suffix('.'))
                    .map(str::to_string)
            })
        }) {
            pid = found;
            break;
        }
    }
    assert!(!pid.is_empty(), "process group was not logged");
    let (status, canceled, _) = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/cancel"),
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    assert_eq!(canceled["status"], "canceled");
    let proc = std::path::PathBuf::from(format!("/proc/{pid}"));
    let mut gone = false;
    for _ in 0..40 {
        if !proc.exists() {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(gone, "process {pid} is still running");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (status, body, _) = call(
        &app,
        "GET",
        &format!("/api/jobs/{job_id}"),
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "canceled");
    let events = body["events"].to_string();
    assert!(events.contains("Cancel requested."), "{events}");
    assert!(!events.contains("should-not-run"), "{events}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn cancel_kills_a_child_that_ignores_term() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-cancel-trap-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
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
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap();
    let script = "( trap '' TERM; sleep 30 & echo $! > \"$KOREOGRAF_WORK/hold.pid\"; wait; echo ran > \"$KOREOGRAF_WORK/marker\" ) &\nwait\n";
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Trap",
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": script}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let template_id = created["id"].as_str().unwrap();
    let (status, job, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    let job_id = job["id"].as_str().unwrap();
    let hold = dir.join("jobs").join(job_id).join("hold.pid");
    let marker = dir.join("jobs").join(job_id).join("marker");
    let mut holder = String::new();
    for _ in 0..50 {
        if let Ok(text) = std::fs::read_to_string(&hold) {
            holder = text.trim().to_string();
            if !holder.is_empty() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        !holder.is_empty(),
        "the child that ignores TERM did not start"
    );
    let (status, canceled, _) = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/cancel"),
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    assert_eq!(canceled["status"], "canceled");
    let mut gone = false;
    for _ in 0..40 {
        let cmdline = std::fs::read_to_string(format!("/proc/{holder}/cmdline")).ok();
        if cmdline
            .as_deref()
            .is_none_or(|text| text.is_empty() || !text.contains("sleep"))
        {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(gone, "child {holder} was still running 4s after cancel");
    assert!(!marker.exists(), "the child kept running after cancel");
    let (status, body, _) = call(
        &app,
        "GET",
        &format!("/api/jobs/{job_id}"),
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "canceled");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn cancel_kills_a_detached_child_that_ignores_term() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-cancel-detached-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
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
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap();
    // /bin/true is external, so a shell that buffers stdout flushes the
    // start line before it waits. Output after cancel is discarded.
    let script = "( trap '' TERM; sleep 45 & echo $! > \"$KOREOGRAF_WORK/hold.pid\"; wait; echo detached-after > \"$KOREOGRAF_WORK/marker\" ) >/dev/null 2>&1 &\necho detached-start\n/bin/true\nwait\n";
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Detached",
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": script}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let template_id = created["id"].as_str().unwrap();
    let (status, job, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    let job_id = job["id"].as_str().unwrap();
    let hold = dir.join("jobs").join(job_id).join("hold.pid");
    let marker = dir.join("jobs").join(job_id).join("marker");
    let mut holder = String::new();
    let mut started = false;
    for _ in 0..50 {
        if holder.is_empty() {
            if let Ok(text) = std::fs::read_to_string(&hold) {
                let text = text.trim().to_string();
                if !text.is_empty() {
                    holder = text;
                }
            }
        }
        if !started {
            let (status, body, _) = call(
                &app,
                "GET",
                &format!("/api/jobs/{job_id}"),
                Some(&cookie),
                Some(team),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            if body["events"].to_string().contains("detached-start") {
                started = true;
            }
        }
        if !holder.is_empty() && started {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        !holder.is_empty(),
        "the detached child that ignores TERM did not start"
    );
    assert!(started, "the job did not log detached-start before cancel");
    let (status, canceled, _) = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/cancel"),
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    assert_eq!(canceled["status"], "canceled");
    let mut gone = false;
    for _ in 0..50 {
        let cmdline = std::fs::read_to_string(format!("/proc/{holder}/cmdline")).ok();
        if cmdline
            .as_deref()
            .is_none_or(|text| text.is_empty() || !text.contains("sleep"))
        {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        gone,
        "detached child {holder} was still running 5s after cancel"
    );
    assert!(
        !marker.exists(),
        "the detached child kept running after cancel"
    );
    let (status, body, _) = call(
        &app,
        "GET",
        &format!("/api/jobs/{job_id}"),
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "canceled");
    let events = body["events"].to_string();
    assert!(events.contains("detached-start"), "{events}");
    assert!(!events.contains("detached-after"), "{events}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_schedule_runs_the_last_miss_once_and_lists_on_its_target() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-schedule-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state.clone());

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("schedule-card"));
    assert!(script.contains("id=\"schedule-form\""));
    assert!(script.contains("#/schedules"));
    assert!(script.contains("class=\"switch\""));
    let (status, style, _) = call(&app, "GET", "/app.css", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(style.as_str().unwrap_or("").contains("schedule-miss"));
    assert!(style
        .as_str()
        .unwrap_or("")
        .contains(".switch input:checked"));

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

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Scheduled",
            "tool": "shell",
            "update_on_launch": false,
            "spec": {"script": "echo scheduled"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Morning")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["next_run_at"], "2027-06-01T04:00:00Z");
    assert_eq!(created["frequency"], "daily");
    assert_eq!(created["enabled"], true);
    assert!(created["missed_at"].is_null());
    let schedule_id = created["id"].as_str().unwrap().to_string();

    let (status, list, _) = call(
        &app,
        "GET",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list.as_array().unwrap().len(), 1);

    let (status, preview, _) = call(
        &app,
        "POST",
        "/api/schedules/preview",
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("ignored")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let occurrences = preview["occurrences"].as_array().unwrap();
    assert_eq!(occurrences.len(), 5);
    assert_eq!(occurrences[0], "2027-06-01T04:00:00Z");
    assert_eq!(occurrences[1], "2027-06-02T04:00:00Z");

    let (status, zone, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Unknown zone",
            "timezone": "Mars/Olympus",
            "start_date": "2027-06-01",
            "start_time": "06:00",
            "frequency": "daily",
            "end_kind": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{zone}");
    assert_eq!(zone["error"], "schedule_needs_a_known_time_zone");

    let (status, past, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Already over",
            "timezone": "Europe/Berlin",
            "start_date": "2020-01-01",
            "start_time": "06:00",
            "frequency": "once",
            "end_kind": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{past}");
    assert_eq!(past["error"], "schedule_date_is_too_far_in_the_past");

    let (status, past_preview, _) = call(
        &app,
        "POST",
        "/api/schedules/preview",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "timezone": "Europe/Berlin",
            "start_date": "2020-01-01",
            "start_time": "06:00",
            "frequency": "once",
            "end_kind": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{past_preview}");
    assert_eq!(
        past_preview["error"],
        "schedule_date_is_too_far_in_the_past"
    );

    set_start_date(&state, &schedule_id, "2020-01-01");
    let (status, kept, _) = call(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Morning kept",
            "timezone": "Europe/Berlin",
            "start_date": "2020-01-01",
            "start_time": "07:00",
            "frequency": "daily",
            "end_kind": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["name"], "Morning kept");
    assert_eq!(kept["start_date"], "2020-01-01");
    assert_eq!(kept["start_time"], "07:00");
    let (status, moved, _) = call(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Morning moved",
            "timezone": "Europe/Berlin",
            "start_date": "2019-12-31",
            "start_time": "07:00",
            "frequency": "daily",
            "end_kind": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{moved}");
    assert_eq!(moved["error"], "schedule_date_is_too_far_in_the_past");
    let (status, still, _) = call(
        &app,
        "GET",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{still}");
    let kept_row = still
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"].as_str() == Some(schedule_id.as_str()))
        .expect("aged schedule");
    assert_eq!(kept_row["name"], "Morning kept");
    assert_eq!(kept_row["start_date"], "2020-01-01");
    let (status, restored, _) = call(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Morning")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(restored["name"], "Morning");
    assert_eq!(restored["start_date"], "2027-06-01");
    assert_eq!(restored["next_run_at"], "2027-06-01T04:00:00Z");

    let yesterday = chrono::Utc::now()
        .date_naive()
        .pred_opt()
        .expect("yesterday")
        .format("%Y-%m-%d")
        .to_string();
    let (status, recent_past, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Yesterday once",
            "timezone": "UTC",
            "start_date": yesterday,
            "start_time": "00:00",
            "frequency": "once",
            "end_kind": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{recent_past}");
    assert_eq!(recent_past["error"], "schedule_has_no_occurrence");

    let (status, duplicate, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Morning")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{duplicate}");
    assert_eq!(duplicate["error"], "that_name_is_already_used_in_this_team");

    mark_missed(&state, &schedule_id);
    let (status, paused, _) = call(
        &app,
        "POST",
        &format!("/api/schedules/{schedule_id}/enabled"),
        Some(&cookie),
        Some(&team),
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{paused}");
    assert_eq!(paused["enabled"], false);
    assert_eq!(paused["missed_at"], "2026-10-21T04:00:00Z");
    let (status, resumed, _) = call(
        &app,
        "POST",
        &format!("/api/schedules/{schedule_id}/enabled"),
        Some(&cookie),
        Some(&team),
        Some(json!({"enabled": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resumed}");
    assert_eq!(resumed["enabled"], true);
    assert_eq!(resumed["missed_at"], "2026-10-21T04:00:00Z");
    assert_eq!(resumed["next_run_at"], "2027-06-01T04:00:00Z");

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Morning later")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "Morning later");
    assert!(updated["missed_at"].is_null());

    mark_missed(&state, &schedule_id);
    let (status, accepted, _) = call(
        &app,
        "POST",
        &format!("/api/schedules/{schedule_id}/missed"),
        Some(&cookie),
        Some(&team),
        Some(json!({"accept": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert!(accepted["missed_at"].is_null());
    let (status, jobs, _) = call(&app, "GET", "/api/jobs", Some(&cookie), Some(&team), None).await;
    assert_eq!(status, StatusCode::OK, "{jobs}");
    let started = scheduled_jobs(&jobs, &template_id);
    assert_eq!(started.len(), 1);
    assert_eq!(started[0]["trigger"], "schedule");
    assert_eq!(started[0]["launched_by"], "admin");
    let job_id = started[0]["id"].as_str().unwrap().to_string();
    let finished = wait_for_job(&app, &cookie, &team, &job_id).await;
    assert_eq!(finished["status"], "successful", "{}", finished["events"]);
    assert!(has_line(&finished, "stdout", "scheduled"));

    let (status, again, _) = call(
        &app,
        "POST",
        &format!("/api/schedules/{schedule_id}/missed"),
        Some(&cookie),
        Some(&team),
        Some(json!({"accept": true})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{again}");
    assert_eq!(again["error"], "schedule_has_no_missed_run");

    mark_missed(&state, &schedule_id);
    let missed_path = format!("/api/schedules/{schedule_id}/missed");
    let left = call(
        &app,
        "POST",
        &missed_path,
        Some(&cookie),
        Some(&team),
        Some(json!({"accept": true})),
    );
    let right = call(
        &app,
        "POST",
        &missed_path,
        Some(&cookie),
        Some(&team),
        Some(json!({"accept": true})),
    );
    let ((left_status, left_body, _), (right_status, right_body, _)) = tokio::join!(left, right);
    let statuses = [left_status, right_status];
    assert!(
        statuses.contains(&StatusCode::OK),
        "one accept should run: {left_body} {right_body}"
    );
    assert!(
        statuses.contains(&StatusCode::BAD_REQUEST),
        "the other accept should be refused: {left_body} {right_body}"
    );
    let errors = [left_body["error"].as_str(), right_body["error"].as_str()];
    assert!(errors.contains(&Some("schedule_has_no_missed_run")));
    let (status, jobs, _) = call(&app, "GET", "/api/jobs", Some(&cookie), Some(&team), None).await;
    assert_eq!(status, StatusCode::OK, "{jobs}");
    let ids: Vec<String> = scheduled_jobs(&jobs, &template_id)
        .iter()
        .map(|job| job["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids.len(), 2);
    for id in ids {
        let finished = wait_for_job(&app, &cookie, &team, &id).await;
        assert_eq!(finished["status"], "successful", "{}", finished["events"]);
    }

    mark_missed(&state, &schedule_id);
    let (status, declined, _) = call(
        &app,
        "POST",
        &format!("/api/schedules/{schedule_id}/missed"),
        Some(&cookie),
        Some(&team),
        Some(json!({"accept": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{declined}");
    assert!(declined["missed_at"].is_null());
    let (status, jobs, _) = call(&app, "GET", "/api/jobs", Some(&cookie), Some(&team), None).await;
    assert_eq!(status, StatusCode::OK, "{jobs}");
    assert_eq!(scheduled_jobs(&jobs, &template_id).len(), 2);

    let (status, workflow, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Scheduled flow", "graph": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let workflow_id = workflow["id"].as_str().unwrap().to_string();
    let (status, weekly, _) = call(
        &app,
        "POST",
        &format!("/api/workflows/{workflow_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Weekdays",
            "timezone": "Europe/Berlin",
            "start_date": "2027-06-01",
            "start_time": "06:00",
            "frequency": "weekly",
            "weekdays": ["FR", "MO"],
            "end_kind": "after",
            "end_count": 3
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{weekly}");
    assert_eq!(weekly["weekdays"], json!(["MO", "FR"]));
    assert_eq!(weekly["end_kind"], "after");
    assert_eq!(weekly["end_count"], 3);
    assert_eq!(weekly["next_run_at"], "2027-06-04T04:00:00Z");
    let (status, flows, _) = call(
        &app,
        "GET",
        &format!("/api/workflows/{workflow_id}/schedules"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{flows}");
    assert_eq!(flows.as_array().unwrap().len(), 1);

    let (status, all, _) = call(
        &app,
        "GET",
        "/api/schedules",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{all}");
    let rows = all.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let morning = rows
        .iter()
        .find(|row| row["name"] == "Morning later")
        .unwrap();
    assert_eq!(morning["target_kind"], "template");
    assert_eq!(morning["target_id"], template_id);
    assert_eq!(morning["target_name"], "Scheduled");
    let weekdays = rows.iter().find(|row| row["name"] == "Weekdays").unwrap();
    assert_eq!(weekdays["target_kind"], "workflow");
    assert_eq!(weekdays["target_id"], workflow_id);
    assert_eq!(weekdays["target_name"], "Scheduled flow");

    let (status, gone, _) = call(
        &app,
        "DELETE",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{gone}");
    let left: i64 = {
        let db = state.db().expect("database");
        db.query_row(
            "SELECT COUNT(*) FROM schedules WHERE target_id = ?1",
            [template_id.as_str()],
            |row| row.get(0),
        )
        .expect("count")
    };
    assert_eq!(left, 0i64);
    let (status, remaining, _) = call(
        &app,
        "GET",
        "/api/schedules",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{remaining}");
    assert_eq!(remaining.as_array().unwrap().len(), 1);
    assert_eq!(remaining[0]["name"], "Weekdays");

    let _ = std::fs::remove_dir_all(&dir);
}

fn daily_schedule(name: &str) -> Value {
    json!({
        "name": name,
        "timezone": "Europe/Berlin",
        "start_date": "2027-06-01",
        "start_time": "06:00",
        "frequency": "daily",
        "end_kind": "never"
    })
}

fn set_start_date(state: &koreograf::Shared, id: &str, start_date: &str) {
    let db = state.db().expect("database");
    let changed = db
        .execute(
            "UPDATE schedules SET start_date = ?1 WHERE id = ?2",
            (start_date, id),
        )
        .expect("set start date");
    assert_eq!(changed, 1);
}

fn mark_missed(state: &koreograf::Shared, id: &str) {
    let db = state.db().expect("database");
    let changed = db
        .execute(
            "UPDATE schedules SET missed_at = ?1 WHERE id = ?2",
            ("2026-10-21T04:00:00Z", id),
        )
        .expect("mark missed");
    assert_eq!(changed, 1);
}

fn scheduled_jobs<'a>(jobs: &'a Value, template_id: &str) -> Vec<&'a Value> {
    jobs.as_array()
        .unwrap()
        .iter()
        .filter(|job| {
            job["template_id"].as_str() == Some(template_id) && job["trigger"] == "schedule"
        })
        .collect()
}

async fn wait_for_job(app: &axum::Router, cookie: &str, team: &str, job_id: &str) -> Value {
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let (status, body, _) = call(
            app,
            "GET",
            &format!("/api/jobs/{job_id}"),
            Some(cookie),
            Some(team),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if matches!(
            body["status"].as_str(),
            Some("successful" | "failed" | "canceled")
        ) {
            return body;
        }
    }
    panic!("job {job_id} did not finish");
}

#[test]
fn message_catalogs_match_and_cover_the_ui() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let de = load_catalog(&root.join("web/i18n/de.json"));
    let en = load_catalog(&root.join("web/i18n/en.json"));
    assert_eq!(
        de.keys().collect::<Vec<_>>(),
        en.keys().collect::<Vec<_>>(),
        "catalog keys differ"
    );

    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let mut missing = BTreeSet::new();
    for key in ui_message_keys(&script) {
        if !de.contains_key(&key) {
            missing.insert(key);
        }
    }
    assert!(
        missing.is_empty(),
        "app.js keys missing from the catalogs: {missing:?}"
    );

    let german_ui = umlaut_lines("web/app.js", &script);
    assert!(german_ui.is_empty(), "German left in app.js: {german_ui:?}");
    let mut german_src = Vec::new();
    leftover_german(&root.join("src"), &mut german_src);
    assert!(german_src.is_empty(), "German left in src: {german_src:?}");

    let mut api_missing = BTreeSet::new();
    for key in rust_message_keys(&root.join("src")) {
        if !de.contains_key(&key) {
            api_missing.insert(key);
        }
    }
    assert!(
        api_missing.is_empty(),
        "API keys missing from the catalogs: {api_missing:?}"
    );
    for catalog in [&de, &en] {
        let hint = catalog
            .get("confirm_the_replacement_with_ersetzen")
            .map(String::as_str)
            .unwrap_or("");
        assert!(
            hint.contains("ERSETZEN"),
            "restore hint must name the confirm token"
        );
    }

    let runner = std::fs::read_to_string(root.join("src/runner.rs")).expect("runner.rs");
    assert!(
        runner.contains("nicht gefunden"),
        "git still reports a missing revision in German"
    );
    assert!(
        runner.contains("konnte den remote-verweis"),
        "git still reports a missing remote ref in German"
    );
}

fn load_catalog(path: &Path) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    let value: Value =
        serde_json::from_str(&text).unwrap_or_else(|err| panic!("json {}: {err}", path.display()));
    let Some(object) = value.as_object() else {
        panic!("{} is not an object", path.display());
    };
    let mut catalog = BTreeMap::new();
    for (key, item) in object {
        let Some(text) = item.as_str() else {
            panic!("{key} in {} is not a string", path.display());
        };
        assert!(!text.trim().is_empty(), "{key} is empty");
        catalog.insert(key.clone(), text.to_string());
    }
    catalog
}

fn ui_message_keys(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut keys = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if let Some(open) = call_open(&chars, index) {
            keys.extend(first_arg_strings(&chars, open));
        }
        index += 1;
    }
    keys
}

fn is_ident(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'
}

fn call_open(chars: &[char], index: usize) -> Option<usize> {
    if index > 0 && is_ident(chars[index - 1]) {
        return None;
    }
    for name in ["renderLogin", "Error", "t", "h"] {
        if !at_name(chars, index, name) {
            continue;
        }
        let mut next = index + name.chars().count();
        while chars.get(next).is_some_and(|ch| ch.is_whitespace()) {
            next += 1;
        }
        if chars.get(next) == Some(&'(') {
            return Some(next);
        }
    }
    None
}

fn at_name(chars: &[char], index: usize, name: &str) -> bool {
    let mut offset = index;
    for ch in name.chars() {
        if chars.get(offset) != Some(&ch) {
            return false;
        }
        offset += 1;
    }
    !chars.get(offset).is_some_and(|ch| is_ident(*ch))
}

fn first_arg_strings(chars: &[char], open: usize) -> Vec<String> {
    let mut keys = Vec::new();
    let mut index = open + 1;
    let mut paren = 0i32;
    let mut brace = 0i32;
    let mut bracket = 0i32;
    let mut seen_comma = false;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '"' || ch == '\'' {
            let (next, text) = read_quoted(chars, index);
            if ch == '"'
                && !seen_comma
                && paren == 0
                && brace == 0
                && bracket == 0
                && !text.is_empty()
            {
                keys.push(text);
            }
            index = next;
            continue;
        }
        if ch == '`' {
            index = skip_template(chars, index);
            continue;
        }
        if ch == '/' && chars.get(index + 1) == Some(&'/') {
            index += 2;
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if ch == '/' && chars.get(index + 1) == Some(&'*') {
            index += 2;
            while index + 1 < chars.len() && !(chars[index] == '*' && chars[index + 1] == '/') {
                index += 1;
            }
            index = (index + 2).min(chars.len());
            continue;
        }
        match ch {
            '(' => paren += 1,
            ')' if paren > 0 => paren -= 1,
            ')' => break,
            '{' => brace += 1,
            '}' if brace > 0 => brace -= 1,
            '[' => bracket += 1,
            ']' if bracket > 0 => bracket -= 1,
            ',' if paren == 0 && brace == 0 && bracket == 0 => seen_comma = true,
            _ => {}
        }
        index += 1;
    }
    keys
}

fn read_quoted(chars: &[char], start: usize) -> (usize, String) {
    let quote = chars[start];
    let mut out = String::new();
    let mut index = start + 1;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\\' {
            if let Some(next) = chars.get(index + 1) {
                out.push(*next);
                index += 2;
                continue;
            }
        }
        if ch == quote {
            return (index + 1, out);
        }
        out.push(ch);
        index += 1;
    }
    (index, out)
}

fn skip_template(chars: &[char], start: usize) -> usize {
    let mut index = start + 1;
    while index < chars.len() {
        if chars[index] == '\\' {
            index = (index + 2).min(chars.len());
            continue;
        }
        if chars[index] == '`' {
            return index + 1;
        }
        if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
            index = skip_braces(chars, index + 1);
            continue;
        }
        index += 1;
    }
    index
}

fn skip_braces(chars: &[char], open: usize) -> usize {
    let mut index = open + 1;
    let mut depth = 1i32;
    while index < chars.len() && depth > 0 {
        if chars[index] == '"' || chars[index] == '\'' {
            index = read_quoted(chars, index).0;
            continue;
        }
        if chars[index] == '`' {
            index = skip_template(chars, index);
            continue;
        }
        if chars[index] == '{' {
            depth += 1;
        } else if chars[index] == '}' {
            depth -= 1;
            if depth == 0 {
                return index + 1;
            }
        }
        index += 1;
    }
    index
}

fn umlaut_lines(label: &str, text: &str) -> Vec<String> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| has_umlaut(line))
        .map(|(number, line)| format!("{label}:{}: {}", number + 1, preview(line)))
        .collect()
}

fn has_umlaut(line: &str) -> bool {
    line.chars()
        .any(|ch| matches!(ch, 'ä' | 'ö' | 'ü' | 'Ä' | 'Ö' | 'Ü' | 'ß'))
}

fn preview(line: &str) -> String {
    let mut short: String = line.chars().take(160).collect();
    if line.chars().count() > 160 {
        short.push_str("...");
    }
    short
}

fn rust_message_keys(dir: &Path) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    collect_rust_keys(dir, &mut keys);
    keys
}

fn collect_rust_keys(dir: &Path, keys: &mut BTreeSet<String>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("{err}"));
        let path = entry.path();
        if path.is_dir() {
            collect_rust_keys(&path, keys);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        push_call_key(&text, "ApiError::bad(", keys);
        push_call_key(&text, "ApiError::bad_vars(", keys);
        push_call_key(&text, "::message(", keys);
        push_err_key(&text, keys);
    }
}

fn push_call_key(text: &str, needle: &str, keys: &mut BTreeSet<String>) {
    let mut from = 0;
    while let Some(rel) = text[from..].find(needle) {
        let at = from + rel + needle.len();
        let tail = text[at..].trim_start();
        if let Some((key, _)) = take_quoted(tail) {
            keys.insert(key);
        }
        from = at;
    }
}

fn push_err_key(text: &str, keys: &mut BTreeSet<String>) {
    let mut from = 0;
    while let Some(rel) = text[from..].find("Err(") {
        let at = from + rel + 4;
        let tail = text[at..].trim_start();
        if let Some((key, used)) = take_quoted(tail) {
            let after = tail[used..].trim_start();
            if (after.starts_with(".into()") || after.starts_with(".to_string()"))
                && is_message_key(&key)
            {
                keys.insert(key);
            }
        }
        from = at;
    }
}

fn is_message_key(key: &str) -> bool {
    let mut chars = key.chars();
    if !chars.next().is_some_and(|ch| ch.is_ascii_lowercase()) {
        return false;
    }
    let mut marked = false;
    for ch in key.chars() {
        if ch == '_' || ch == '.' {
            marked = true;
            continue;
        }
        if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() {
            return false;
        }
    }
    marked
}

fn take_quoted(text: &str) -> Option<(String, usize)> {
    let mut chars = text.char_indices();
    if chars.next().is_none_or(|(_, ch)| ch != '"') {
        return None;
    }
    let mut key = String::new();
    let mut escaped = false;
    for (index, ch) in chars {
        if escaped {
            key.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return Some((key, index + ch.len_utf8()));
        }
        key.push(ch);
    }
    None
}

fn leftover_german(dir: &Path, hits: &mut Vec<String>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("{err}"));
        let path = entry.path();
        if path.is_dir() {
            leftover_german(&path, hits);
            continue;
        }
        let ext = path.extension().and_then(|ext| ext.to_str());
        if ext != Some("rs") && ext != Some("sh") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        for (number, line) in text.lines().enumerate() {
            if line.contains("nicht gefunden") || line.contains("konnte den remote-verweis") {
                continue;
            }
            if has_umlaut(line) {
                hits.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    number + 1,
                    preview(line)
                ));
            }
        }
    }
}
