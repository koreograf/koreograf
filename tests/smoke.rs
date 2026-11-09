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
        container_image: false,
        job_timeout_secs: 60,
        job_output_lines: 8_000,
        local_jobs: LocalJobs::All,
        project_roots: vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")],
        hsts: false,
        cookie_secure: CookieSecure::Auto,
        trusted_proxy: false,
        mcp: true,
        data_host: None,
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

async fn call_bearer(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    team: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let bearer = format!("Bearer {token}");
    let has_body = body.is_some();
    let mut extra = vec![("authorization", bearer.as_str())];
    if has_body {
        extra.push(("content-type", "application/json"));
    }
    if let Some(team) = team {
        extra.push(("x-team-id", team));
    }
    let body = body.map(|value| value.to_string().into_bytes());
    let (status, bytes, _) = call_raw(app, method, uri, None, &extra, body).await;
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).to_string()))
    };
    (status, parsed)
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
    assert!(style.contains(".log-summary"));
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
async fn launch_with_options_overrides_a_template_without_prompt_on_launch() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-launch-options-{nanos}"));
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

    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Options hosts", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap();

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Options",
            "tool": "ansible",
            "entrypoint": "site.yml",
            "inventory_id": inventory_id,
            "extra_vars": {"keep": "template"},
            "spec": {"job_type": "run", "tags": "ping", "limit": "web"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    assert_eq!(template["prompt_on_launch"], false);
    let template_id = template["id"].as_str().unwrap();

    let (status, blocked, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}, "spec": {"limit": "db", "tags": "heal"}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{blocked}");
    assert_eq!(blocked["error"], "prompt_on_launch_is_off");

    let (status, launched, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "extra_vars": {"from_launch": "yes"},
            "spec": {"limit": "db", "tags": "heal", "job_type": "check"},
            "launch_with_options": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");
    let job_id = launched["id"].as_str().unwrap();
    assert_eq!(launched["parameters"]["spec"]["limit"], "db");
    assert_eq!(launched["parameters"]["spec"]["tags"], "heal");
    assert_eq!(launched["parameters"]["spec"]["job_type"], "check");
    assert_eq!(launched["parameters"]["extra_vars"]["keep"], "template");
    assert_eq!(launched["parameters"]["extra_vars"]["from_launch"], "yes");
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
    assert_eq!(row["prompt_on_launch"], false);
    assert_eq!(row["spec"]["limit"], "web");
    assert_eq!(row["spec"]["tags"], "ping");
    assert_eq!(row["spec"]["job_type"], "run");
    assert_eq!(row["extra_vars"]["keep"], "template");
    assert!(row["extra_vars"].get("from_launch").is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

async fn launch_options_admin_app(dir: &Path) -> (axum::Router, String, String, String) {
    let mut cfg = config(dir);
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

    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Gated hosts", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap().to_string();

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Gated",
            "tool": "ansible",
            "entrypoint": "site.yml",
            "inventory_id": inventory_id,
            "spec": {"job_type": "run", "tags": "ping", "limit": "web"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();
    (app, cookie, team, template_id)
}

#[tokio::test]
async fn options_launch_passes_the_local_runner_gate() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-options-gate-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let (app, cookie, team, template_id) = launch_options_admin_app(&dir).await;

    let (status, launched, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "spec": {"limit": "db", "tags": "heal", "job_type": "check"},
            "launch_with_options": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");
    let job_id = launched["id"].as_str().unwrap();
    assert_eq!(launched["parameters"]["launch_with_options"], true);
    assert_eq!(launched["parameters"]["spec"]["limit"], "db");
    assert_eq!(launched["parameters"]["spec"]["tags"], "heal");
    assert_eq!(launched["parameters"]["spec"]["job_type"], "check");
    let _ = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/cancel"),
        Some(&cookie),
        Some(&team),
        None,
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
    let stored = stored
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == template_id.as_str())
        .unwrap();
    assert_eq!(stored["spec"]["limit"], "web");
    assert_eq!(stored["spec"]["tags"], "ping");
    assert_eq!(stored["spec"]["job_type"], "run");
    assert_eq!(stored["prompt_on_launch"], false);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn options_launch_relaunch_passes_the_local_runner_gate() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-options-relaunch-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let (app, cookie, team, template_id) = launch_options_admin_app(&dir).await;

    let (status, launched, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "spec": {"limit": "db", "tags": "heal"},
            "launch_with_options": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");
    let job_id = launched["id"].as_str().unwrap();
    let _ = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/cancel"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;

    let (status, again, _) = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/relaunch"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{again}");
    assert_ne!(again["id"], job_id);
    assert_eq!(again["trigger"], "relaunch");
    assert_eq!(again["parameters"]["launch_with_options"], true);
    assert_eq!(again["parameters"]["spec"]["limit"], "db");
    assert_eq!(again["parameters"]["spec"]["tags"], "heal");
    let _ = call(
        &app,
        "POST",
        &format!("/api/jobs/{}/cancel", again["id"].as_str().unwrap()),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn ansible_launch_requires_an_inventory() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-inventory-required-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = config(&dir);
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

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Needs inventory",
            "tool": "ansible",
            "entrypoint": "site.yml"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();

    let (status, rejected, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["error"], "inventory_is_required");

    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Required hosts", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap();

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Needs inventory",
            "tool": "ansible",
            "entrypoint": "site.yml",
            "inventory_id": inventory_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");

    let (status, launched, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");
    let job_id = launched["id"].as_str().unwrap();
    assert_eq!(launched["parameters"]["inventory_id"], inventory_id);
    let _ = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/cancel"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;

    let (status, shell, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "No inventory shell",
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": "true"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{shell}");
    let shell_id = shell["id"].as_str().unwrap();
    let (status, shell_launched, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{shell_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{shell_launched}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_workflow_node_without_an_inventory_fails_before_launching() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-wf-inventory-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = config(&dir);
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

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Workflow needs inventory",
            "tool": "ansible",
            "entrypoint": "site.yml"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();

    let (status, workflow, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Needs inventory run",
            "description": "",
            "graph": {
                "nodes": [
                    {"id": "one", "template_id": template_id, "name": "one"}
                ],
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
    let run_id = run["id"].as_str().unwrap();

    let mut finished = None;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let (status, body, _) = call(
            &app,
            "GET",
            &format!("/api/workflow-jobs/{run_id}"),
            Some(&cookie),
            Some(&team),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if matches!(
            body["status"].as_str(),
            Some("successful" | "failed" | "canceled")
        ) {
            finished = Some(body);
            break;
        }
    }
    let finished = finished.expect("workflow run did not finish");
    assert_eq!(finished["status"], "failed", "{finished}");
    let node = &finished["nodes"][0];
    assert_eq!(node["phase"], "failed", "{finished}");
    assert!(node["job_id"].is_null(), "{finished}");

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
async fn a_project_can_be_edited_and_syncs_a_new_revision() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-edit-{nanos}"));
    let origin = std::env::temp_dir().join(format!("koreograf-edit-origin-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&origin).unwrap();
    let mut cfg = config(&dir);
    cfg.project_roots.push(origin.clone());
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

    git(&origin, &["init", "-b", "main"]);
    git(&origin, &["config", "user.email", "test@example.com"]);
    git(&origin, &["config", "user.name", "Koreograf Test"]);
    std::fs::write(origin.join("main.txt"), "main\n").unwrap();
    git(&origin, &["add", "main.txt"]);
    git(&origin, &["commit", "-m", "main"]);
    git(&origin, &["checkout", "-b", "dev"]);
    std::fs::write(origin.join("dev.txt"), "dev\n").unwrap();
    git(&origin, &["add", "dev.txt"]);
    git(&origin, &["commit", "-m", "dev"]);
    git(&origin, &["checkout", "main"]);
    let url = format!("file://{}", origin.display());

    let (status, project, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Editable",
            "source_type": "git",
            "source": url,
            "revision": "main"
        })),
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
    let checkout = std::path::PathBuf::from(synced["path"].as_str().unwrap());
    assert!(checkout.join("main.txt").exists());

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Renamed", "revision": "dev"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "Renamed");
    assert_eq!(updated["revision"], "dev");
    assert_eq!(updated["sync"]["ok"], true);
    assert_eq!(updated["last_sync_status"], "success");
    assert!(!updated["last_sync_at"].as_str().unwrap().is_empty());
    assert!(!updated["last_sync_commit"].as_str().unwrap().is_empty());
    assert!(checkout.join("dev.txt").exists());

    let (status, stored, _) = call(
        &app,
        "GET",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(stored["name"], "Renamed");
    assert_eq!(stored["revision"], "dev");

    let (status, invalid, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"source": "ftp://example.com/repo.git"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid}");
    assert_eq!(invalid["error"], "the_git_address_must_be_http_s_ssh");
    let (status, created_invalid, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Broken",
            "source_type": "git",
            "source": "ftp://example.com/repo.git"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{created_invalid}");
    assert_eq!(created_invalid["error"], invalid["error"]);

    let (status, _viewer, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "viewer",
            "display_name": "viewer",
            "password": "viewer-pass",
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
        Some(json!({"username": "viewer", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, viewer_me, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "viewer", "password": "viewer-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer_me}");
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    let (status, denied, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&viewer_cookie),
        Some(&team),
        Some(json!({"name": "Nope"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");

    let _ = std::fs::remove_dir_all(&origin);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn projects_use_a_source_control_credential() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-project-credential-{nanos}"));
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

    let (status, scm, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Git access",
            "kind": "scm",
            "secret": {"username": "deploy", "password": "glpat-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{scm}");
    let scm_id = scm["id"].as_str().unwrap().to_string();
    let (status, second, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Git access two",
            "kind": "scm",
            "secret": {"password": "second-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let second_id = second["id"].as_str().unwrap().to_string();

    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/demo")
        .display()
        .to_string();
    let (status, project, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Private project",
            "source_type": "path",
            "source": source,
            "credential_id": scm_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap().to_string();
    assert_eq!(project["credential_id"], scm_id);
    assert!(!project.to_string().contains("glpat-token"));

    let (status, stored, _) = call(
        &app,
        "GET",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(stored["credential_id"], scm_id);
    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/projects",
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
        .any(|item| item["id"] == project_id && item["credential_id"] == scm_id));

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"credential_id": second_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["credential_id"], second_id);

    let (status, ssh, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "SSH access",
            "kind": "ssh",
            "secret": {"username": "deploy", "password": "ssh-pass"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ssh}");
    let ssh_id = ssh["id"].as_str().unwrap();
    let (status, wrong_kind, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Wrong kind",
            "source_type": "path",
            "source": source,
            "credential_id": ssh_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{wrong_kind}");
    assert_eq!(
        wrong_kind["error"],
        "the_credential_is_not_a_source_control_credential"
    );

    let (status, other_team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Other Git team"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other_team}");
    let other = other_team["id"].as_str().unwrap();
    let (status, foreign, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(other),
        Some(json!({
            "name": "Foreign Git access",
            "kind": "scm",
            "secret": {"password": "foreign-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{foreign}");
    let foreign_id = foreign["id"].as_str().unwrap();
    let (status, wrong_team, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Wrong team",
            "source_type": "path",
            "source": source,
            "credential_id": foreign_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{wrong_team}");
    assert_eq!(
        wrong_team["error"],
        "the_credential_belongs_to_another_team"
    );

    let (status, embedded, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Embedded",
            "source_type": "git",
            "source": "https://user:secret@example.com/repo.git"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{embedded}");
    assert_eq!(embedded["error"], "the_git_address_contains_credentials");
    assert_eq!(embedded["vars"]["field"], "credential_id");
    let (status, embedded_update, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"source_type": "git", "source": "https://user:secret@example.com/repo.git"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{embedded_update}");
    assert_eq!(
        embedded_update["error"],
        "the_git_address_contains_credentials"
    );

    let (status, credentials, _) = call(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{credentials}");
    let row = credentials
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == second_id)
        .expect("used credential");
    assert!(row["usages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|usage| usage["project_id"] == project_id));

    let (status, refused, _) = call(
        &app,
        "DELETE",
        &format!("/api/credentials/{second_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"], "credential_is_in_use");
    assert!(
        refused["vars"]["names"]
            .as_str()
            .unwrap_or("")
            .contains("Private project"),
        "{refused}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn project_credentials_stay_with_the_project_team() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-project-credential-team-{nanos}"));
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
    let admin_team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, org, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        Some(&admin_team),
        Some(json!({"name": "Credential projects"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org}");
    let org_id = org["id"].as_str().unwrap();
    let (status, owners, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&admin_team),
        Some(json!({"name": "Credential owners", "organization_id": org_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{owners}");
    let owner_team = owners["id"].as_str().unwrap().to_string();
    let (status, guests, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&admin_team),
        Some(json!({"name": "Credential guests", "organization_id": org_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{guests}");
    let guest_team = guests["id"].as_str().unwrap().to_string();

    for (username, team) in [
        ("credential-owner", &owner_team),
        ("credential-guest", &guest_team),
    ] {
        let (status, _user, _) = call(
            &app,
            "POST",
            "/api/users",
            Some(&cookie),
            None,
            Some(json!({
                "username": username,
                "display_name": username,
                "password": "operator-pass"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _, _) = call(
            &app,
            "POST",
            &format!("/api/teams/{team}/members"),
            Some(&cookie),
            Some(team),
            Some(json!({"username": username, "role": "operator"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let (status, owner_me, owner_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "credential-owner", "password": "operator-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{owner_me}");
    let owner_cookie = owner_cookie.expect("owner cookie");
    let (status, guest_me, guest_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "credential-guest", "password": "operator-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{guest_me}");
    let guest_cookie = guest_cookie.expect("guest cookie");

    let (status, owner_credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&owner_team),
        Some(json!({
            "name": "Owner Git",
            "kind": "scm",
            "secret": {"username": "deploy", "password": "owner-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{owner_credential}");
    let owner_credential = owner_credential["id"].as_str().unwrap().to_string();
    let (status, guest_credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&guest_team),
        Some(json!({
            "name": "Guest Git",
            "kind": "scm",
            "secret": {"password": "guest-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{guest_credential}");
    let guest_credential = guest_credential["id"].as_str().unwrap().to_string();

    let (status, foreign, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&owner_cookie),
        Some(&owner_team),
        Some(json!({
            "name": "Foreign credential project",
            "source_type": "git",
            "source": "https://example.com/owner.git",
            "team_ids": [guest_team],
            "credential_id": guest_credential
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{foreign}");
    assert_eq!(foreign["error"], "the_credential_belongs_to_another_team");

    let demo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/demo")
        .display()
        .to_string();
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&owner_cookie),
        Some(&owner_team),
        Some(json!({
            "name": "Owner credential project",
            "source_type": "path",
            "source": demo,
            "team_ids": [owner_team, guest_team],
            "credential_id": owner_credential
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let project_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["credential_id"], owner_credential);

    let (status, guest_view, _) = call(
        &app,
        "GET",
        &format!("/api/projects/{project_id}"),
        Some(&guest_cookie),
        Some(&guest_team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{guest_view}");
    assert!(guest_view["credential_id"].is_null(), "{guest_view}");
    let (status, guest_list, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&guest_cookie),
        Some(&guest_team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{guest_list}");
    assert!(guest_list
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["id"] == project_id && item["credential_id"].is_null() }));

    let inventory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/demo/inventory")
        .display()
        .to_string();
    let (status, changed, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&guest_cookie),
        Some(&guest_team),
        Some(json!({"source": inventory})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert!(changed["credential_id"].is_null(), "{changed}");
    let (status, owner_view, _) = call(
        &app,
        "GET",
        &format!("/api/projects/{project_id}"),
        Some(&owner_cookie),
        Some(&owner_team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{owner_view}");
    assert!(owner_view["credential_id"].is_null(), "{owner_view}");

    let (status, kept, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&owner_cookie),
        Some(&owner_team),
        Some(json!({"source": demo, "credential_id": owner_credential})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["credential_id"], owner_credential);
    let (status, synced_owner, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&owner_cookie),
        Some(&owner_team),
        Some(json!({"source": inventory})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced_owner}");
    assert_eq!(synced_owner["credential_id"], owner_credential);

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
    assert!(script.contains("[\"netbox\", \"netbox\", \"netbox_credential_address_hint\"]"));
    assert!(script.contains("data-netbox-address"));
    assert!(script.contains("data-new-netbox-credential"));
    assert!(!script.contains("data-token-box"));
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
    let (status, credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Source NetBox",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": format!("http://{addr}")}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"].as_str().unwrap().to_string();

    let (status, preview, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/preview",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "credential_id": credential_id,
            "group_by": ["tags"],
            "group_names_raw": true,
            "load_config_context": true,
            "query_filters": ["status=active"],
            "virtual_machines": false,
            "max_hosts": 100
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["host_count"], 1);
    assert_eq!(preview["groups"][0]["name"], "webserver");
    assert_eq!(preview["groups"][0]["host_count"], 1);
    assert_eq!(preview["sample_host"]["name"], "web-1");
    assert_eq!(preview["sample_host"]["vars"]["http_port"], "8080");
    assert!(preview["collisions"].as_array().unwrap().is_empty());
    assert!(!preview.to_string().contains("secret-token"));

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Aus NetBox",
            "credential_id": credential_id,
            "group_by": ["tags"],
            "group_names_raw": true,
            "load_config_context": true,
            "query_filters": ["status=active"],
            "virtual_machines": false,
            "max_hosts": 100
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
    let body = created["body"].as_str().unwrap_or("");
    assert!(body.starts_with("all:\n  hosts:\n"), "{body}");
    assert!(body.contains("http_port: 8080"), "{body}");
    assert!(body.contains("    webserver:\n"), "{body}");
    assert!(!body.contains("secret-token"));
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

    let (status, wrong_credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Wrong NetBox token",
            "kind": "netbox",
            "secret": {"token": "wrong-token", "url": format!("http://{addr}")}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{wrong_credential}");
    let wrong_credential_id = wrong_credential["id"].as_str().unwrap().to_string();
    let (status, bad, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Kaputtes NetBox",
            "credential_id": wrong_credential_id,
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    assert_eq!(bad["error"], "netbox.token_rejected");
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

#[tokio::test]
async fn inventory_hosts_page_search_and_filter() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-host-page-{nanos}"));
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

    let body = "[web]\n\
web-01 ansible_host=10.0.0.1 role=frontend\n\
web-02 ansible_host=10.0.0.2 role=frontend\n\
web-03 ansible_host=10.0.0.3 role=frontend\n\
\n\
[db]\n\
db-01 ansible_host=10.0.0.11 role=database\n\
db-02 ansible_host=10.0.0.12 role=database\n\
\n\
[cache]\n\
cache-01 ansible_host=10.0.0.42 role=cache\n";
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Paged hosts", "body": body})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["host_count"], 6);

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
    let summary = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .unwrap();
    assert!(summary.get("hosts").is_none(), "{summary}");
    assert!(summary.get("body").is_none(), "{summary}");
    assert_eq!(summary["host_count"], 6);
    assert_eq!(summary["group_count"], 3);
    assert_eq!(summary["source_count"], 1);
    assert!(summary["sources"]
        .as_array()
        .unwrap()
        .iter()
        .all(|source| source.get("body").is_none()));

    let (status, detail, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert!(detail.get("hosts").is_none(), "{detail}");
    assert!(detail.get("body").is_none(), "{detail}");
    assert!(detail["sources"]
        .as_array()
        .unwrap()
        .iter()
        .all(|source| source.get("body").is_none()));

    let source_id = detail["sources"][0]["id"].as_str().unwrap();
    let (status, source, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{source}");
    assert!(source["body"].as_str().unwrap_or("").contains("web-01"));
    assert_eq!(source["host_count"], 6);

    let hosts_url = |query: &str| format!("/api/inventories/{id}/hosts{query}");
    let (status, first, _) = call(
        &app,
        "GET",
        &hosts_url(""),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["total"], 6);
    assert_eq!(first["page"], 1);
    assert_eq!(first["page_size"], 50);
    assert_eq!(first["hosts"].as_array().unwrap().len(), 6);
    assert_eq!(first["groups"], json!(["cache", "db", "web"]));

    let (status, second, _) = call(
        &app,
        "GET",
        &hosts_url("?page=2&page_size=2"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["total"], 6);
    assert_eq!(second["page"], 2);
    assert_eq!(second["hosts"][0]["name"], "db-02");
    assert_eq!(second["hosts"][1]["name"], "web-01");
    assert_eq!(second["hosts"].as_array().unwrap().len(), 2);

    let (status, beyond, _) = call(
        &app,
        "GET",
        &hosts_url("?page=4&page_size=2"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{beyond}");
    assert_eq!(beyond["total"], 6);
    assert_eq!(beyond["page"], 4);
    assert_eq!(beyond["hosts"], json!([]));

    let (status, huge, _) = call(
        &app,
        "GET",
        &hosts_url("?page=18446744073709551615&page_size=2"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{huge}");
    assert_eq!(huge["total"], 6);
    assert_eq!(huge["hosts"], json!([]));

    let (status, zero, _) = call(
        &app,
        "GET",
        &hosts_url("?page=0"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{zero}");
    assert_eq!(zero["page"], 1);
    assert_eq!(zero["hosts"].as_array().unwrap().len(), 6);

    let (status, capped, _) = call(
        &app,
        "GET",
        &hosts_url("?limit=5000"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{capped}");
    assert_eq!(capped["page_size"], 200);

    let (status, offset, _) = call(
        &app,
        "GET",
        &hosts_url("?offset=4&limit=2"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{offset}");
    assert_eq!(offset["page"], 3);
    assert_eq!(offset["hosts"][0]["name"], "web-02");
    assert_eq!(offset["hosts"].as_array().unwrap().len(), 2);

    let (status, search, _) = call(
        &app,
        "GET",
        &hosts_url("?search=web"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{search}");
    assert_eq!(search["total"], 3);
    assert!(search["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|host| host["name"].as_str().unwrap().starts_with("web-")));

    let (status, address, _) = call(
        &app,
        "GET",
        &hosts_url("?search=10.0.0.42"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{address}");
    assert_eq!(address["total"], 1);
    assert_eq!(address["hosts"][0]["name"], "cache-01");

    let (status, variable, _) = call(
        &app,
        "GET",
        &hosts_url("?search=database"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{variable}");
    assert_eq!(variable["total"], 2);

    let (status, grouped, _) = call(
        &app,
        "GET",
        &hosts_url("?group=db"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{grouped}");
    assert_eq!(grouped["total"], 2);
    assert!(grouped["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|host| host["groups"] == json!(["db"])));

    let (status, combined, _) = call(
        &app,
        "GET",
        &hosts_url("?group=web&search=web-02"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{combined}");
    assert_eq!(combined["total"], 1);

    let (status, empty, _) = call(
        &app,
        "GET",
        &hosts_url("?search=missing"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert_eq!(empty["total"], 0);
    assert_eq!(empty["hosts"], json!([]));

    let (status, reversed, _) = call(
        &app,
        "GET",
        &hosts_url("?sort=-name&page_size=1"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reversed}");
    assert_eq!(reversed["hosts"][0]["name"], "web-03");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn inventory_host_and_group_details_include_effective_variables() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-host-details-{nanos}"));
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

    let body = "\
[all:vars]\n\
ansible_user=ansible\n\
\n\
[app:children]\n\
web\n\
\n\
[app:vars]\n\
app_port=1\n\
\n\
[web:vars]\n\
http_port=8080\n\
\n\
[web]\n\
web-01 ansible_host=10.0.0.1\n\
web-02 ansible_host=10.0.0.2\n";
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Host details", "body": body})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    let source_id = created["sources"][0]["id"].as_str().unwrap().to_string();

    let (status, groups, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{id}/groups"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{groups}");
    assert_eq!(groups["total"], 3);
    assert_eq!(groups["groups"][0]["name"], "all");
    assert_eq!(groups["groups"][0]["host_count"], 2);
    let app_group = groups["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["name"] == "app")
        .unwrap();
    assert_eq!(app_group["children"], json!(["web"]));
    assert_eq!(app_group["vars"]["app_port"], "1");
    assert_eq!(app_group["host_count"], 2);
    let web_group = groups["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["name"] == "web")
        .unwrap();
    assert_eq!(web_group["vars"]["http_port"], "8080");
    assert_eq!(web_group["host_count"], 2);

    let (status, group, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{id}/groups/web?page=2&page_size=1"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{group}");
    assert_eq!(group["name"], "web");
    assert_eq!(group["parents"], json!(["app"]));
    assert_eq!(group["total"], 2);
    assert_eq!(group["page"], 2);
    assert_eq!(group["hosts"].as_array().unwrap().len(), 1);
    assert_eq!(group["hosts"][0]["name"], "web-02");

    let (status, host, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{id}/hosts/web-01"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{host}");
    assert_eq!(host["name"], "web-01");
    assert_eq!(host["groups"], json!(["all", "app", "web"]));
    assert_eq!(host["source"]["id"], source_id);
    assert_eq!(host["source"]["kind"], "ini");
    let vars = host["vars"].as_array().unwrap();
    let find = |name: &str| vars.iter().find(|item| item["name"] == name).unwrap();
    assert_eq!(find("ansible_host")["value"], "10.0.0.1");
    assert_eq!(find("ansible_host")["origin"], "host");
    assert_eq!(find("http_port")["origin"], "web");
    assert_eq!(find("app_port")["origin"], "app");
    assert_eq!(find("ansible_user")["value"], "ansible");
    assert_eq!(find("ansible_user")["origin"], "all");
    assert!(host["netbox_id"].is_null());
    assert!(host["netbox_url"].is_null());

    let (status, missing_host, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{id}/hosts/missing"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing_host}");
    let (status, missing_group, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{id}/groups/missing"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing_group}");

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        None,
        Some(json!({
            "username": "detail-viewer",
            "password": "viewer-pass",
            "display_name": "Detail viewer"
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
        Some(json!({"username": "detail-viewer", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "detail-viewer", "password": "viewer-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    for path in [
        format!("/api/inventories/{id}/hosts/web-01"),
        format!("/api/inventories/{id}/groups"),
        format!("/api/inventories/{id}/groups/web"),
    ] {
        let (status, body, _) =
            call(&app, "GET", &path, Some(&viewer_cookie), Some(&team), None).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn inventory_sources_can_be_edited_and_inventories_renamed() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-source-edit-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("data-edit-source"));
    assert!(script.contains("hosts_before_after"));

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

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        None,
        Some(json!({
            "username": "source-viewer",
            "password": "viewer-pass",
            "display_name": "Source viewer"
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
        Some(json!({"username": "source-viewer", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/dcim/devices/", axum::routing::get(nb_editable));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });
    let (status, credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Edit NetBox",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": format!("http://{addr}")}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "NetBox edit",
            "credential_id": credential_id,
            "group_by": ["sites"],
            "query_filters": ["status=active"],
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let inventory_id = created["id"].as_str().unwrap().to_string();
    let source_id = created["sources"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(created["host_count"], 1);
    assert!(created["sources"][0]["body"]
        .as_str()
        .unwrap_or("")
        .contains("sites_berlin"));

    let (status, edited, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Edited NetBox",
            "credential_id": credential_id,
            "group_by": ["site"],
            "query_filters": ["status=planned"],
            "validate_certs": true,
            "ca_file": "/no/such/koreograf-ca.pem",
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["sources"][0]["id"], source_id);
    assert_eq!(edited["sources"][0]["name"], "Edited NetBox");
    assert_eq!(edited["sources"][0]["source"]["group_by"], json!(["site"]));
    assert!(edited["sources"][0]["source"].get("ca_file").is_none());
    assert_eq!(
        edited["sources"][0]["source"]["query"],
        json!([["status", "planned"]])
    );
    assert!(edited["sources"][0]["body"]
        .as_str()
        .unwrap_or("")
        .contains("site_berlin"));
    assert!(edited["sources"][0]["body"]
        .as_str()
        .unwrap_or("")
        .contains("web-2"));
    assert_eq!(edited["host_count"], 2);
    assert_eq!(edited["sync"]["ok"], true);
    assert_eq!(edited["sync"]["before"], 1);
    assert_eq!(edited["sync"]["after"], 2);
    assert!(!edited.to_string().contains("secret-token"));

    let (status, stored, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(stored["sources"][0]["id"], source_id);
    assert_eq!(stored["sources"][0]["name"], "Edited NetBox");
    assert_eq!(stored["sources"][0]["source"]["group_by"], json!(["site"]));

    let (status, foreign, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"path": "inventory/hosts.ini"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{foreign}");
    assert_eq!(foreign["error"], "json_field_is_not_supported");
    assert_eq!(foreign["vars"]["field"], "path");

    let (status, renamed, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Renamed inventory"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["name"], "Renamed inventory");
    assert_eq!(renamed["sources"][0]["id"], source_id);

    let (status, _, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "source-viewer", "password": "viewer-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    let (status, forbidden, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&viewer_cookie),
        Some(&team),
        Some(json!({"name": "Not allowed"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{forbidden}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn netbox_sources_follow_the_credential_address() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-netbox-credential-url-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state.clone());

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

    let (first, first_hits) = netbox_mock().await;
    let (second, second_hits) = netbox_mock().await;

    let (status, credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "NetBox address",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": first}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Credential address",
            "credential_id": credential_id,
            "query_filters": ["status=active"],
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["sources"][0]["credential_id"], credential_id);
    assert!(created["sources"][0]["source"]
        .get("api_endpoint")
        .is_none());
    assert!(!created.to_string().contains("secret-token"));
    assert!(!created.to_string().contains("secret_blob"));
    assert_eq!(
        first_hits.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "{created}"
    );
    let inventory_id = created["id"].as_str().unwrap().to_string();
    let source_id = created["sources"][0]["id"].as_str().unwrap().to_string();

    let (status, rotated, _) = call(
        &app,
        "PUT",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "NetBox address", "secret": {"url": second}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    assert_eq!(rotated["fields"]["url"], second);
    assert!(!rotated.to_string().contains("secret-token"));

    let (status, synced, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}/sync"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    assert_eq!(
        second_hits.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "{synced}"
    );
    assert_eq!(
        first_hits.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "{synced}"
    );

    {
        let db = state.db().unwrap();
        let source_json: String = db
            .query_row(
                "SELECT source_json FROM inventory_sources WHERE id = ?1",
                [&source_id],
                |row| row.get(0),
            )
            .unwrap();
        let mut value: Value = serde_json::from_str(&source_json).unwrap();
        value["api_endpoint"] = json!(first);
        db.execute(
            "UPDATE inventory_sources SET source_json = ?2 WHERE id = ?1",
            rusqlite::params![source_id, value.to_string()],
        )
        .unwrap();
    }
    let (status, legacy, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}/sync"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{legacy}");
    assert_eq!(
        second_hits.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "{legacy}"
    );
    assert_eq!(
        first_hits.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "{legacy}"
    );

    let (status, mismatch, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Wrong address",
            "credential_id": credential_id,
            "api_endpoint": first,
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{mismatch}");
    assert_eq!(mismatch["error"], "netbox_endpoint_must_match_credential");
    assert_eq!(
        second_hits.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "{mismatch}"
    );

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
    let before = listed.as_array().unwrap().len();
    let (status, token_only, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Token shortcut",
            "api_endpoint": second,
            "token": "secret-token",
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{token_only}");
    assert_eq!(token_only["error"], "netbox_token_needs_a_credential");
    let (status, missing, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "No credential", "virtual_machines": false})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{missing}");
    assert_eq!(missing["error"], "netbox_credential_is_required");
    let (status, listed_again, _) = call(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed_again}");
    assert_eq!(listed_again.as_array().unwrap().len(), before);
    assert!(!listed_again.to_string().contains("secret-token"));
    assert!(!listed_again.to_string().contains("secret_blob"));

    let (status, stored, _) = call(
        &app,
        "GET",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert!(!stored.to_string().contains("secret-token"));
    assert!(!stored.to_string().contains("secret_blob"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn netbox_credentials_without_a_url_cannot_reach_another_endpoint() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-netbox-no-url-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state.clone());

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

    let (first, first_hits) = netbox_mock().await;
    let (second, second_hits) = netbox_mock().await;

    let (status, legacy, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Legacy token",
            "kind": "github",
            "secret": {"token": "legacy-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{legacy}");
    let legacy_id = legacy["id"].as_str().unwrap().to_string();
    {
        let db = state.db().unwrap();
        let changed = db
            .execute(
                "UPDATE credentials SET kind = 'netbox' WHERE id = ?1",
                [&legacy_id],
            )
            .unwrap();
        assert_eq!(changed, 1);
    }

    let (status, missing, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "No address",
            "kind": "netbox",
            "secret": {"token": "secret-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{missing}");
    assert_eq!(missing["error"], "netbox_needs_an_address");

    let (status, valid, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Valid NetBox",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": first}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{valid}");
    let valid_id = valid["id"].as_str().unwrap().to_string();

    let (status, cleared, _) = call(
        &app,
        "PUT",
        &format!("/api/credentials/{valid_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Valid NetBox", "secret": {"url": ""}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{cleared}");
    assert_eq!(cleared["error"], "netbox_needs_an_address");

    let (status, denied_create, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&operator_cookie),
        Some(&team),
        Some(json!({
            "name": "Foreign NetBox",
            "sources": [{
                "kind": "netbox",
                "credential_id": legacy_id,
                "api_endpoint": second
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied_create}");
    assert_eq!(denied_create["error"], "netbox_needs_an_address");

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&operator_cookie),
        Some(&team),
        Some(json!({
            "name": "Bound NetBox",
            "sources": [{
                "kind": "netbox",
                "credential_id": valid_id,
                "api_endpoint": first,
                "virtual_machines": false
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(
        first_hits.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "{created}"
    );
    let inventory_id = created["id"].as_str().unwrap().to_string();
    let source_id = created["sources"][0]["id"].as_str().unwrap().to_string();

    let (status, denied_add, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources"),
        Some(&operator_cookie),
        Some(&team),
        Some(json!({
            "kind": "netbox",
            "credential_id": legacy_id,
            "api_endpoint": second
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied_add}");
    assert_eq!(denied_add["error"], "netbox_needs_an_address");

    let (status, denied_update, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&operator_cookie),
        Some(&team),
        Some(json!({"credential_id": legacy_id, "api_endpoint": second})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied_update}");
    assert_eq!(denied_update["error"], "netbox_needs_an_address");

    let (status, denied_test, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/test",
        Some(&operator_cookie),
        Some(&team),
        Some(json!({"credential_id": legacy_id, "api_endpoint": second})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied_test}");
    assert_eq!(denied_test["error"], "netbox_needs_an_address");
    assert_eq!(
        second_hits.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "{denied_test}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn netbox_credentials_without_a_source_are_disabled() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-netbox-disabled-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state.clone());

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

    let (status, legacy, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Orphan token",
            "kind": "github",
            "secret": {"token": "orphan-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{legacy}");
    let legacy_id = legacy["id"].as_str().unwrap().to_string();
    {
        let db = state.db().unwrap();
        db.execute(
            "UPDATE credentials SET kind = 'netbox' WHERE id = ?1",
            [&legacy_id],
        )
        .unwrap();
    }
    drop(app);
    drop(state);

    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state.clone());
    {
        let db = state.db().unwrap();
        let disabled: i64 = db
            .query_row(
                "SELECT disabled FROM credentials WHERE id = ?1",
                [&legacy_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(disabled, 1);
    }

    let (status, _, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("session cookie");
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
    assert!(!listed
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == legacy_id));

    let (status, denied, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/test",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "credential_id": legacy_id,
            "api_endpoint": "http://127.0.0.1:9"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    assert_eq!(denied["error"], "netbox_credential_is_disabled");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn netbox_source_update_can_switch_the_credential() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-netbox-switch-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state.clone());

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

    let (first, first_seen) = netbox_mock_recording().await;
    let (second, second_seen) = netbox_mock_recording().await;

    let (status, shared, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Shared NetBox",
            "kind": "netbox",
            "secret": {"token": "shared-token", "url": first}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{shared}");
    let shared_id = shared["id"].as_str().unwrap().to_string();

    let (status, private, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Private NetBox",
            "kind": "netbox",
            "secret": {"token": "private-token", "url": second}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{private}");
    let private_id = private["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Switch source",
            "sources": [{
                "kind": "netbox",
                "credential_id": shared_id,
                "virtual_machines": false
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(
        first_seen.lock().unwrap().last().map(String::as_str),
        Some("Token shared-token")
    );
    let inventory_id = created["id"].as_str().unwrap().to_string();
    let source_id = created["sources"][0]["id"].as_str().unwrap().to_string();

    let (status, edited, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&operator_cookie),
        Some(&team),
        Some(json!({"credential_id": private_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["sources"][0]["credential_id"], private_id);
    assert_eq!(
        second_seen.lock().unwrap().last().map(String::as_str),
        Some("Token private-token")
    );
    assert!(!edited.to_string().contains("private-token"));
    assert!(!edited.to_string().contains("secret_blob"));

    let (status, stored, _) = call(
        &app,
        "GET",
        &format!("/api/credentials/{shared_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(stored["fields"]["url"], first);
    assert!(!stored.to_string().contains("shared-token"));

    let (status, denied, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&operator_cookie),
        Some(&team),
        Some(json!({"validate_certs": false})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    assert_eq!(denied["error"], "you_are_not_allowed_to_do_this");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn netbox_sources_share_a_normal_credential() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-netbox-shared-{nanos}"));
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

    let (addr, seen) = netbox_mock_recording().await;
    let (status, credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "NetBox Prod",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": addr}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"].as_str().unwrap().to_string();

    let (status, first, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "NetBox shared",
            "sources": [{
                "kind": "netbox",
                "credential_id": credential_id,
                "query_filters": ["status=active"],
                "virtual_machines": false
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["sources"][0]["credential_id"], credential_id);
    let first_id = first["id"].as_str().unwrap().to_string();
    let first_source = first["sources"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        seen.lock().unwrap().last().map(String::as_str),
        Some("Token secret-token")
    );

    let (status, second, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "NetBox second",
            "sources": [{
                "kind": "netbox",
                "credential_id": credential_id,
                "api_endpoint": addr,
                "query_filters": ["status=active"],
                "virtual_machines": false
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let second_id = second["id"].as_str().unwrap().to_string();
    assert_eq!(
        seen.lock().unwrap().last().map(String::as_str),
        Some("Token secret-token")
    );

    let (status, rotated, _) = call(
        &app,
        "PUT",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "NetBox Prod", "secret": {"token": "rotated-token"}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    assert!(!rotated.to_string().contains("rotated-token"));
    for id in [&first_id, &second_id] {
        let (status, synced, _) = call(
            &app,
            "POST",
            &format!("/api/inventories/{id}/sync"),
            Some(&cookie),
            Some(&team),
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{synced}");
        assert!(synced["hosts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "web-1"));
        assert_eq!(
            seen.lock().unwrap().last().map(String::as_str),
            Some("Token rotated-token")
        );
    }

    let (status, ssh, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Not NetBox",
            "kind": "ssh",
            "secret": {"username": "deploy", "password": "ssh-pass"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ssh}");
    let ssh_id = ssh["id"].as_str().unwrap();
    let (status, wrong_kind, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Wrong kind",
            "sources": [{
                "kind": "netbox",
                "credential_id": ssh_id,
                "api_endpoint": addr
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{wrong_kind}");
    assert_eq!(wrong_kind["error"], "netbox_credential_has_the_wrong_kind");

    let (status, other_team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Other NetBox team"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other_team}");
    let other = other_team["id"].as_str().unwrap();
    let (status, foreign, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(other),
        Some(json!({
            "name": "Foreign NetBox",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": addr}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{foreign}");
    let foreign_id = foreign["id"].as_str().unwrap();
    let (status, wrong_team, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Wrong team",
            "sources": [{
                "kind": "netbox",
                "credential_id": foreign_id,
                "api_endpoint": addr
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{wrong_team}");
    assert_eq!(
        wrong_team["error"],
        "the_credential_belongs_to_another_team"
    );

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
    let row = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == credential_id)
        .expect("shared credential");
    assert_eq!(row["usages"].as_array().unwrap().len(), 2, "{row}");
    assert!(row["usages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|usage| usage["inventory_name"] == "NetBox shared"));

    let (status, refused, _) = call(
        &app,
        "DELETE",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"], "credential_is_in_use");
    let names = refused["vars"]["names"].as_str().unwrap_or("");
    assert!(names.contains("NetBox shared / netbox"), "{names}");
    assert!(names.contains("NetBox second / netbox"), "{names}");

    let (status, synced, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{first_id}/sources/{first_source}/sync"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    assert_eq!(
        seen.lock().unwrap().last().map(String::as_str),
        Some("Token rotated-token")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn netbox_automatic_credentials_become_normal_and_get_their_url() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-netbox-migrate-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state.clone());

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

    let (addr, _) = netbox_mock_recording().await;
    let (status, credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Legacy NetBox Token",
            "kind": "github",
            "secret": {"token": "secret-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"].as_str().unwrap().to_string();
    let inventory_id = format!("legacy-inventory-{nanos}");
    let source_id = format!("legacy-source-{nanos}");
    let source_json = json!({
        "api_endpoint": addr,
        "validate_certs": true,
        "ca_file": "/no/such/koreograf-ca.pem",
        "group_by": [],
        "query": [["status", "active"]],
        "virtual_machines": false
    })
    .to_string();
    let created_at = "2024-01-01T00:00:00Z";

    {
        let db = state.db().unwrap();
        let changed = db
            .execute(
                "UPDATE credentials SET kind = 'netbox', auto = 1 WHERE id = ?1",
                [&credential_id],
            )
            .unwrap();
        assert_eq!(changed, 1);
        db.execute(
            "INSERT INTO inventories (id, team_id, name, body, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            [
                inventory_id.as_str(),
                team.as_str(),
                "Legacy NetBox",
                "",
                created_at,
            ],
        )
        .unwrap();
        db.execute(
            "INSERT INTO inventory_sources (
                id, inventory_id, name, kind, source_json, credential_id, body, position, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            [
                source_id.as_str(),
                inventory_id.as_str(),
                "Legacy NetBox",
                "netbox",
                source_json.as_str(),
                credential_id.as_str(),
                "",
                "0",
                created_at,
            ],
        )
        .unwrap();
    }
    drop(app);
    drop(state);

    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state.clone());

    {
        let db = state.db().unwrap();
        let automatic: i64 = db
            .query_row(
                "SELECT auto FROM credentials WHERE id = ?1",
                [&credential_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(automatic, 0);
    }

    let (status, _, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("session cookie");
    let (status, stored, _) = call(
        &app,
        "GET",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(stored["name"], "Legacy NetBox Token");
    assert_eq!(stored["fields"]["url"], addr);
    assert_eq!(stored["usages"][0]["inventory_id"], inventory_id);

    let (status, synced, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}/sync"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    assert!(synced["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "web-1"));

    let _ = std::fs::remove_dir_all(&dir);
}

async fn netbox_mock_recording() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = seen.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route(
        "/api/dcim/devices/",
        axum::routing::get(move |headers: axum::http::HeaderMap| {
            let seen = recorded.clone();
            async move {
                let token = headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                seen.lock().unwrap().push(token);
                axum::Json(json!({
                    "next": null,
                    "results": [{
                        "id": 1,
                        "name": "web-1",
                        "primary_ip4": {"address": "10.0.0.8/24"},
                        "site": {"slug": "berlin"}
                    }]
                }))
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });
    (format!("http://{addr}"), seen)
}

async fn netbox_mock() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = hits.clone();
    let status_count = hits.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new()
        .route(
            "/api/dcim/devices/",
            axum::routing::get(move || {
                let count = count.clone();
                async move {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    axum::Json(json!({
                        "next": null,
                        "results": [{
                            "id": 1,
                            "name": "web-1",
                            "primary_ip4": {"address": "10.0.0.8/24"},
                            "site": {"slug": "berlin"}
                        }]
                    }))
                }
            }),
        )
        .route(
            "/api/status/",
            axum::routing::get(move || {
                let count = status_count.clone();
                async move {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    axum::Json(json!({"netbox-version": "4.2.1", "plugins": {}}))
                }
            }),
        );
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });
    (format!("http://{addr}"), hits)
}

async fn nb_editable(
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
    let results = if query.contains("status=planned") {
        json!([
            {
                "id": 1,
                "name": "web-1",
                "primary_ip4": {"address": "10.0.0.8/24"},
                "site": {"slug": "berlin"}
            },
            {
                "id": 2,
                "name": "web-2",
                "primary_ip4": {"address": "10.0.0.9/24"},
                "site": {"slug": "munich"}
            }
        ])
    } else {
        json!([{
            "id": 1,
            "name": "web-1",
            "primary_ip4": {"address": "10.0.0.8/24"},
            "site": {"slug": "berlin"}
        }])
    };
    axum::Json(json!({"next": null, "results": results})).into_response()
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
            "site": {"slug": "berlin"},
            "tags": [{"slug": "webserver"}],
            "config_context": {"http_port": 8080, "db_password": "secret"}
        }]
    }))
    .into_response()
}

#[tokio::test]
async fn netbox_test_connection_reports_the_version_or_a_mapped_error() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-netbox-test-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/status/", axum::routing::get(nb_status));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });

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

    let good = create_credential(
        &app,
        &cookie,
        &team,
        "Test NetBox",
        "netbox",
        json!({"token": "secret-token", "url": format!("http://{addr}")}),
    )
    .await;
    let (status, ok, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/test",
        Some(&cookie),
        Some(&team),
        Some(json!({"credential_id": good})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ok}");
    assert_eq!(ok["version"], "4.2.1", "{ok}");

    let denied = create_credential(
        &app,
        &cookie,
        &team,
        "Wrong NetBox token",
        "netbox",
        json!({"token": "wrong-token", "url": format!("http://{addr}")}),
    )
    .await;
    let (status, denied, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/test",
        Some(&cookie),
        Some(&team),
        Some(json!({"credential_id": denied})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    assert_eq!(denied["error"], "netbox.token_rejected", "{denied}");
    assert_eq!(denied["vars"]["status"], "403", "{denied}");

    let missing = create_credential(
        &app,
        &cookie,
        &team,
        "Wrong NetBox path",
        "netbox",
        json!({"token": "secret-token", "url": format!("http://{addr}/wrong")}),
    )
    .await;
    let (status, missing, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/test",
        Some(&cookie),
        Some(&team),
        Some(json!({"credential_id": missing})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{missing}");
    assert_eq!(missing["error"], "netbox.address_not_found", "{missing}");
    assert_eq!(missing["vars"]["status"], "404", "{missing}");

    let unreachable = create_credential(
        &app,
        &cookie,
        &team,
        "Unreachable NetBox",
        "netbox",
        json!({"token": "secret-token", "url": "http://127.0.0.1:9"}),
    )
    .await;
    let (status, unreachable, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/test",
        Some(&cookie),
        Some(&team),
        Some(json!({"credential_id": unreachable})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{unreachable}");
    assert_eq!(
        unreachable["error"], "netbox_is_not_reachable_err",
        "{unreachable}"
    );

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        None,
        Some(json!({
            "username": "watcher",
            "password": "watcher-pass",
            "display_name": "Watcher"
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
        Some(json!({"username": "watcher", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "watcher", "password": "watcher-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    let (status, forbidden, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/test",
        Some(&viewer_cookie),
        Some(&team),
        Some(json!({"credential_id": good})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{forbidden}");

    let _ = std::fs::remove_dir_all(&dir);
}

async fn nb_status(headers: axum::http::HeaderMap) -> axum::response::Response {
    use axum::response::IntoResponse;
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some("Token secret-token")
    {
        return (StatusCode::FORBIDDEN, "no").into_response();
    }
    axum::Json(json!({"netbox-version": "4.2.1", "plugins": {}})).into_response()
}

#[tokio::test]
async fn deleting_an_inventory_keeps_its_shortcut_netbox_credential() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-netbox-cred-{nanos}"));
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

    let (status, manual, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Unrelated NetBox Token",
            "kind": "netbox",
            "secret": {"token": "keep-me", "url": "http://127.0.0.1:9"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{manual}");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/dcim/devices/", axum::routing::get(nb_devices));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });

    let (status, prod, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prod NB Token",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": format!("http://{addr}")}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{prod}");
    let prod_id = prod["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prod NB",
            "credential_id": prod_id,
            "query_filters": ["status=active"],
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let netbox_id = created["id"].as_str().unwrap();

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
    let names = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"Prod NB Token"), "{names:?}");
    assert!(names.contains(&"Unrelated NetBox Token"), "{names:?}");

    let (status, deleted, _) = call(
        &app,
        "DELETE",
        &format!("/api/inventories/{netbox_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{deleted}");

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
    let names = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"Prod NB Token"), "{names:?}");
    assert!(names.contains(&"Unrelated NetBox Token"), "{names:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn deleting_a_used_inventory_is_refused_with_the_template_names() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-inv-in-use-{nanos}"));
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

    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Prod", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap();

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Ping",
            "tool": "ansible",
            "entrypoint": "ping.yml",
            "inventory_id": inventory_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();

    let (status, refused, _) = call(
        &app,
        "DELETE",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"], "inventory_is_in_use");
    assert!(
        refused["vars"]["names"].as_str().unwrap().contains("Ping"),
        "{refused}"
    );

    let (status, still_there, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{still_there}");

    let (status, detached, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Ping",
            "tool": "ansible",
            "entrypoint": "ping.yml",
            "inventory_id": ""
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detached}");

    let (status, deleted, _) = call(
        &app,
        "DELETE",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{deleted}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn deleting_an_inventory_keeps_shortcut_credentials_from_sources() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-src-cred-{nanos}"));
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

    let (status, manual, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Unrelated NetBox Token",
            "kind": "netbox",
            "secret": {"token": "keep-me", "url": "http://127.0.0.1:9"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{manual}");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/dcim/devices/", axum::routing::get(nb_devices));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });

    let (status, prod, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prod NB Token",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": format!("http://{addr}")}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{prod}");
    let prod_id = prod["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prod NB",
            "sources": [{
                "kind": "netbox",
                "name": "Prod NB",
                "credential_id": prod_id,
                "query_filters": ["status=active"],
                "virtual_machines": false
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let netbox_id = created["id"].as_str().unwrap();

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
    let names = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"Prod NB Token"), "{names:?}");
    assert!(names.contains(&"Unrelated NetBox Token"), "{names:?}");

    let (status, deleted, _) = call(
        &app,
        "DELETE",
        &format!("/api/inventories/{netbox_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{deleted}");

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
    let names = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"Prod NB Token"), "{names:?}");
    assert!(names.contains(&"Unrelated NetBox Token"), "{names:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_broken_ini_inventory_is_rejected_with_the_line_number() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-bad-ini-{nanos}"));
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

    let (status, broken, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Broken INI", "body": "[all\nh1 ansible_host=\n[[["})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{broken}");
    assert_eq!(broken["error"], "inventory_ini_is_invalid");
    assert_eq!(broken["vars"]["line"], "1");

    let (status, valid, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Good INI", "body": "[web]\nweb-1 ansible_host=10.0.0.5\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{valid}");
    let valid_id = valid["id"].as_str().unwrap();

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{valid_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Good INI", "body": "[web]\n[db\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{updated}");
    assert_eq!(updated["error"], "inventory_ini_is_invalid");
    assert_eq!(updated["vars"]["line"], "2");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn broken_ini_bodies_are_rejected_on_the_source_endpoints() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-bad-ini-sources-{nanos}"));
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

    let broken = "[all\nh1 ansible_host=\n[[[";

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Broken sources", "sources": [{"kind": "ini", "body": broken}]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{created}");
    assert_eq!(created["error"], "inventory_ini_is_invalid");
    assert_eq!(created["vars"]["line"], "1");

    let (status, good, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Good sources", "sources": [{"kind": "ini", "body": "[web]\nweb-1 ansible_host=10.0.0.5\n"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{good}");
    let inventory_id = good["id"].as_str().unwrap();
    let source_id = good["sources"][0]["id"].as_str().unwrap();

    let (status, added, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources"),
        Some(&cookie),
        Some(&team),
        Some(json!({"kind": "ini", "body": broken})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{added}");
    assert_eq!(added["error"], "inventory_ini_is_invalid");
    assert_eq!(added["vars"]["line"], "1");

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"body": broken})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{updated}");
    assert_eq!(updated["error"], "inventory_ini_is_invalid");
    assert_eq!(updated["vars"]["line"], "1");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn deleting_an_inventory_keeps_shortcut_credentials_referenced_in_workflows() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-cred-guard-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state.clone());

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

    let (status, manual, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Unrelated NetBox Token",
            "kind": "netbox",
            "secret": {"token": "keep-me", "url": "http://127.0.0.1:9"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{manual}");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/dcim/devices/", axum::routing::get(nb_devices));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });

    let (status, prod, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prod NB Token",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": format!("http://{addr}")}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{prod}");
    let prod_id = prod["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prod NB",
            "sources": [{
                "kind": "netbox",
                "name": "Prod NB",
                "credential_id": prod_id.clone(),
                "query_filters": ["status=active"],
                "virtual_machines": false
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let netbox_id = created["id"].as_str().unwrap();
    {
        let db = state.db().unwrap();
        db.execute("UPDATE credentials SET auto = 1 WHERE id = ?1", [&prod_id])
            .unwrap();
    }

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
    let auto_credential = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Prod NB Token")
        .expect("auto credential")["id"]
        .as_str()
        .unwrap()
        .to_string();

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
        .expect("Demo project")["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, plain, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Testsysteme", "body": "test-1 ansible_connection=local\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{plain}");
    let plain_id = plain["id"].as_str().unwrap().to_string();

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
            "inventory_id": plain_id,
            "entrypoint": "site.yml",
            "spec": {"job_type": "run", "limit": "test", "forks": 5}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    let (status, workflow, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Guard",
            "graph": {
                "nodes": [{
                    "id": "test",
                    "template_id": template_id,
                    "name": "test",
                    "x": 40,
                    "y": 40,
                    "convergence": "any",
                    "overrides": {"credential_id": auto_credential}
                }],
                "edges": []
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");

    let (status, deleted, _) = call(
        &app,
        "DELETE",
        &format!("/api/inventories/{netbox_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{deleted}");

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
    let names = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"Prod NB Token"), "{names:?}");
    assert!(names.contains(&"Unrelated NetBox Token"), "{names:?}");

    let _ = std::fs::remove_dir_all(&dir);
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
    let netbox_credential = create_credential(
        &app,
        &cookie,
        &team,
        "Mixed NetBox",
        "netbox",
        json!({"token": "secret-token", "url": format!("http://{addr}")}),
    )
    .await;
    let (status, with_netbox, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "kind": "netbox",
            "name": "DC",
            "credential_id": netbox_credential,
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
    assert!(script.contains("id: \"node-output\""));
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
        for job in jobs["items"].as_array().unwrap() {
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

#[test]
fn backup_page_carries_the_state_flag_and_the_overview_shows_missing_paths() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    assert!(script.contains("id=\"backup-tofu-state\""));
    assert!(script.contains("include_opentofu_state"));
    assert!(script.contains("opentofu_state_can_contain_secrets"));
    assert!(script.contains("importNoticeHtml(data.import)"));
    let functions = format!(
        "{}\n{}",
        extract_braced_function(&script, "function backupDownloadUrl"),
        extract_braced_function(&script, "function importNoticeHtml")
    );
    let probe = functions
        + r##"
function esc(value) {
  return String(value ?? "").replace(/[&<>"']/g, (ch) => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[ch]));
}
function h(text) { return esc(text); }
if (backupDownloadUrl(true) !== "/api/admin/backup?tofu_state=true") {
  console.error(backupDownloadUrl(true));
  process.exit(1);
}
if (backupDownloadUrl(false) !== "/api/admin/backup") {
  console.error(backupDownloadUrl(false));
  process.exit(1);
}
const notice = importNoticeHtml({missing_paths: ["/home/user/projects/demo"]});
if (!notice.includes("imported_paths_need_a_mount_or_a_change") || !notice.includes("/home/user/projects/demo")) {
  console.error(notice);
  process.exit(1);
}
if (importNoticeHtml({missing_paths: []}) !== "") {
  console.error("empty import notice");
  process.exit(1);
}
"##;
    run_node_probe("backup-import-ui", &probe);
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
        .split_once("function openEnvironmentDialog")
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
            "tool": "ansible",
            "pull_policy": "always",
            "network": "koreograf_default",
            "description": "angepasst"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "EE neu");
    assert_eq!(updated["image"], image);
    assert_eq!(updated["pull_policy"], "always");
    assert_eq!(updated["network"], "koreograf_default");
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
    assert_eq!(again["network"], "koreograf_default");
    let (status, rejected, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "EE neu",
            "image": "bad image",
            "tool": "ansible",
            "pull_policy": "always"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["error"], "the_image_name_is_invalid");
    let (status, rejected, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "EE neu",
            "image": image,
            "tool": "ansible",
            "pull_policy": "always",
            "network": "--net=host"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["error"], "the_network_name_is_invalid");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_environment_tool_limits_the_templates_that_can_use_it() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-env-tool-{nanos}"));
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

    let (status, environment, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Tofu",
            "image": "ghcr.io/opentofu/opentofu:latest",
            "tool": "opentofu"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{environment}");
    assert_eq!(environment["tool"], "opentofu");
    let environment_id = environment["id"].as_str().unwrap();

    let (status, rejected, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Ansible",
            "tool": "ansible",
            "entrypoint": "site.yml",
            "environment_id": environment_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(
        rejected["error"],
        "the_execution_environment_is_for_another_tool"
    );

    let (status, accepted, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Tofu",
            "tool": "opentofu",
            "entrypoint": "module",
            "environment_id": environment_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{accepted}");
    assert_eq!(accepted["environment_id"], environment_id);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn editing_an_environment_clears_a_tool_default_it_no_longer_fits() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-env-default-edit-{nanos}"));
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

    let (status, environment, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Default EE",
            "image": "quay.io/ansible/awx-ee:latest",
            "tool": "ansible",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{environment}");
    let id = environment["id"].as_str().unwrap();

    let settings_body = |ansible: &str, opentofu: &str| {
        json!({
            "ldap": {"url": "", "mode": "ldap"},
            "proxy": {"url": ""},
            "runner": "local",
            "job_timeout_secs": 60,
            "default_environment_ansible": ansible,
            "default_environment_opentofu": opentofu
        })
    };

    let (status, settings, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        Some(team),
        Some(settings_body(id, "")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{settings}");
    assert_eq!(settings["default_environment_ansible"], id);

    let (status, renamed, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Default EE renamed",
            "image": "quay.io/ansible/awx-ee:latest",
            "tool": "ansible",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["name"], "Default EE renamed");
    let (status, kept, _) = call(
        &app,
        "GET",
        "/api/settings",
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["default_environment_ansible"], id);

    let (status, changed, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Default EE renamed",
            "image": "ghcr.io/opentofu/opentofu:latest",
            "tool": "opentofu",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert_eq!(changed["tool"], "opentofu");
    let (status, cleared, _) = call(
        &app,
        "GET",
        "/api/settings",
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["default_environment_ansible"], "");

    let (status, again, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        Some(team),
        Some(settings_body("", id)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["default_environment_opentofu"], id);

    let (status, moved, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{id}"),
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Default EE renamed",
            "image": "ghcr.io/opentofu/opentofu:latest",
            "tool": "opentofu",
            "scope": "team"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["scope"], "team");
    let (status, cleared, _) = call(
        &app,
        "GET",
        "/api/settings",
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["default_environment_opentofu"], "");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_opentofu_template_without_an_environment_uses_the_tool_default() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-env-default-{nanos}"));
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

    let (status, environment, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Tofu default",
            "image": "ghcr.io/opentofu/opentofu:latest",
            "tool": "opentofu",
            "scope": "global"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{environment}");
    let environment_id = environment["id"].as_str().unwrap();

    let (status, settings, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        Some(team),
        Some(json!({
            "ldap": {"url": "", "mode": "ldap"},
            "proxy": {"url": ""},
            "runner": "local",
            "job_timeout_secs": 60,
            "default_environment_opentofu": environment_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{settings}");
    assert_eq!(settings["default_environment_opentofu"], environment_id);

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Tofu ohne Umgebung",
            "tool": "opentofu",
            "entrypoint": "module"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    assert!(template["environment_id"].is_null());
    assert_eq!(template["no_environment"], false);
    let template_id = template["id"].as_str().unwrap();

    let job = launch_and_wait(&app, &cookie, team, template_id).await;
    assert_eq!(job["parameters"]["environment_id"], environment_id);
    assert_eq!(job["parameters"]["environment_name"], "Tofu default");
    assert_eq!(
        job["parameters"]["environment_image"],
        "ghcr.io/opentofu/opentofu:latest"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_fresh_install_seeds_a_default_environment_per_tool() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-seeded-ee-{nanos}"));
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

    let (status, environments, _) = call(
        &app,
        "GET",
        "/api/environments",
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{environments}");
    let global = environments
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["scope"] == "global")
        .collect::<Vec<_>>();
    assert_eq!(global.len(), 3, "{environments}");
    for tool in ["ansible", "opentofu", "shell"] {
        assert!(
            global.iter().any(|row| row["tool"] == tool),
            "missing {tool}: {environments}"
        );
    }

    let (status, settings, _) = call(
        &app,
        "GET",
        "/api/settings",
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{settings}");
    for tool in ["ansible", "opentofu", "shell"] {
        let id = settings
            .get(format!("default_environment_{tool}"))
            .and_then(Value::as_str)
            .unwrap_or("");
        assert!(!id.is_empty(), "missing default for {tool}: {settings}");
        assert!(
            global
                .iter()
                .any(|row| row["id"] == id && row["tool"] == tool),
            "default for {tool} is not the seeded environment: {settings}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_job_without_a_default_environment_fails_before_it_starts() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-no-default-ee-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = config(&dir);
    cfg.container_image = true;
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
    let team = me["teams"][0]["id"].as_str().unwrap();

    let (status, cleared, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        Some(team),
        Some(json!({
            "ldap": {"url": "", "mode": "ldap"},
            "proxy": {"url": ""},
            "runner": "auto",
            "job_timeout_secs": 60,
            "default_environment_ansible": "",
            "default_environment_opentofu": "",
            "default_environment_shell": ""
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Shell ohne Standard",
            "tool": "shell",
            "entrypoint": "run.sh"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();

    let (status, rejected, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["error"], "no_default_execution_environment");
    assert_eq!(rejected["vars"]["tool"], "Shell");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_changed_environment_tool_fails_the_job_before_the_container_starts() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-env-tool-job-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = config(&dir);
    cfg.runner = RunnerMode::Docker;
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
    let team = me["teams"][0]["id"].as_str().unwrap();

    let (status, environment, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Any",
            "image": "quay.io/example/any:1",
            "tool": "shell"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{environment}");
    let environment_id = environment["id"].as_str().unwrap();

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Shell",
            "tool": "shell",
            "entrypoint": "",
            "environment_id": environment_id,
            "spec": {"script": "true"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();

    let (status, changed, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{environment_id}"),
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Any",
            "image": "quay.io/example/any:1",
            "tool": "ansible"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert_eq!(changed["tool"], "ansible");

    let job = launch_and_wait(&app, &cookie, team, template_id).await;
    assert_eq!(job["status"], "failed", "{}", job["events"]);
    assert!(
        has_line(
            &job,
            "system",
            "the_execution_environment_is_for_another_tool"
        ),
        "{}",
        job["events"]
    );
    assert!(
        !job["events"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["line"]
                .as_str()
                .is_some_and(|line| line.starts_with("Execution Environment ")))),
        "{}",
        job["events"]
    );

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
            "tool": "ansible",
            "pull_policy": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{private_env}");
    assert_eq!(private_env["scope"], "team");
    assert_eq!(private_env["team_id"].as_str(), Some(team.as_str()));
    assert!(private_env["organization_id"].is_null());
    let private_id = private_env["id"].as_str().unwrap().to_string();

    for network in ["host", "container:koreograf-koreograf-1"] {
        let (status, denied_network, _) = call(
            &app,
            "POST",
            "/api/environments",
            Some(&ada_cookie),
            Some(&team),
            Some(json!({
                "name": "Privilegiert",
                "image": "quay.io/example/private:1",
                "tool": "ansible",
                "network": network
            })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{denied_network}");
        assert_eq!(denied_network["error"], "you_are_not_allowed_to_do_this");
    }
    let (status, denied_update, _) = call(
        &app,
        "PUT",
        &format!("/api/environments/{private_id}"),
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Privat",
            "image": "quay.io/example/private:1",
            "tool": "ansible",
            "network": "host"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied_update}");
    assert_eq!(denied_update["error"], "you_are_not_allowed_to_do_this");
    let (status, admin_network, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Hostnetz",
            "image": "quay.io/example/host:1",
            "tool": "ansible",
            "network": "host"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{admin_network}");
    assert_eq!(admin_network["network"], "host");

    let (status, org_env, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&ada_cookie),
        Some(&team),
        Some(json!({
            "name": "Orga",
            "image": "quay.io/example/orga:1",
            "tool": "shell",
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
            "tool": "ansible",
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
            "tool": "ansible",
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
            "tool": "shell",
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
            "tool": "shell",
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
            "tool": "shell",
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
            "tool": "shell",
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
            "tool": "shell",
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
async fn a_privileged_environment_network_is_refused_for_a_non_admin_launch() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-env-network-admin-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = config(&dir);
    cfg.local_jobs = LocalJobs::Admins;
    cfg.runner = RunnerMode::Docker;
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

    let (status, _user, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "owner",
            "display_name": "Owner",
            "password": "owner-pass",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, joined, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(&team),
        Some(json!({"username": "owner", "role": "owner"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{joined}");
    let (status, owner_me, owner_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "owner", "password": "owner-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{owner_me}");
    let owner_cookie = owner_cookie.expect("owner cookie");

    let (status, environment, _) = call(
        &app,
        "POST",
        "/api/environments",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Hostnetz",
            "image": "quay.io/example/host:1",
            "tool": "shell",
            "network": "host"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{environment}");
    let environment_id = environment["id"].as_str().unwrap();
    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Mit Hostnetz",
            "tool": "shell",
            "entrypoint": "",
            "environment_id": environment_id,
            "spec": {"script": "true"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();

    let job = launch_and_wait(&app, &owner_cookie, &team, template_id).await;
    assert_eq!(job["status"], "failed", "{}", job["events"]);
    assert!(
        has_line(
            &job,
            "system",
            "The execution environment network needs an administrator."
        ),
        "{}",
        job["events"]
    );

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
    assert!(script.contains("source_control_credential_hint"));
    assert!(script.contains("leave_empty_to_keep_the_stored_value"));
    assert!(script.contains("data-stored-mask=\"1\""));
    assert!(script.contains("class=\"secret-mask\""));
    assert!(!script.contains("*** bedeutet, dass ein Wert gespeichert ist."));
    assert!(script.contains("the_type_cannot_be_changed"));
    assert!(script.contains("#/credentials?edit=${esc(item.id)}"));

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

fn ansible_with_credentials(name: &str, credential_ids: Value, inventory_id: &str) -> Value {
    json!({
        "name": name,
        "tool": "ansible",
        "entrypoint": "site.yml",
        "inventory_id": inventory_id,
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
    assert!(!script.contains("name=\"vault_ids\""));
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

    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Vault hosts", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "Vault-Zuordnung",
            json!([ssh, vault_prod, vault_stage]),
            &inventory_id,
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
        Some(ansible_with_credentials(
            "Vault-Zuordnung",
            json!([ssh, ssh_other]),
            &inventory_id,
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
        Some(ansible_with_credentials(
            "Vault-Zuordnung",
            json!([vault_prod, vault_prod_again]),
            &inventory_id,
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
        Some(ansible_with_credentials(
            "Vault-Zuordnung",
            json!([vault_plain, vault_plain_two]),
            &inventory_id,
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
        if let Some(found) = jobs["items"]
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
async fn template_credential_kinds_follow_the_tool() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-tool-credentials-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let db_state = state.clone();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("const TOOL_CREDENTIAL_KINDS = {"));
    assert!(script.contains("ansible: [\"ssh\", \"vault\"]"));
    assert!(script.contains("opentofu: [\"aws\", \"azure\", \"gce\", \"vmware\", \"openstack\", \"kubernetes\", \"env\"]"));
    assert!(script.contains("shell: [\"ssh\", \"env\"]"));
    assert!(script.contains("data-credential-field"));
    assert!(script.contains("template-legacy-credentials"));
    assert!(script.contains("legacy-pills"));
    assert!(script.contains("remove_credential_name"));

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
        "SSH",
        "ssh",
        json!({"username": "deploy", "password": "tool-ssh-geheim"}),
    )
    .await;
    let vault = create_credential(
        &app,
        &cookie,
        &team,
        "Vault",
        "vault",
        json!({"password": "tool-vault-geheim"}),
    )
    .await;
    let netbox = create_credential(
        &app,
        &cookie,
        &team,
        "NetBox",
        "netbox",
        json!({"token": "tool-netbox-geheim", "url": "https://netbox.example.com"}),
    )
    .await;
    let aws = create_credential(
        &app,
        &cookie,
        &team,
        "AWS",
        "aws",
        json!({"access_key": "AKIAEXAMPLE", "secret_key": "tool-aws-geheim"}),
    )
    .await;
    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Tool hosts", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap().to_string();

    let (status, refused, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "NetBox-Ansible",
            json!([netbox]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(
        refused["error"],
        "this_credential_kind_does_not_fit_the_tool"
    );

    let (status, refused, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "SSH-OpenTofu",
            "tool": "opentofu",
            "entrypoint": ".",
            "credential_ids": [ssh]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(
        refused["error"],
        "this_credential_kind_does_not_fit_the_tool"
    );

    let (status, refused, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(shell_with_credentials("Vault-Shell", json!([vault]))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(
        refused["error"],
        "this_credential_kind_does_not_fit_the_tool"
    );

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "SSH-Ansible",
            json!([ssh, vault]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let template_id = created["id"].as_str().unwrap().to_string();

    let (status, unchanged, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "SSH-Ansible",
            json!([ssh, vault]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{unchanged}");
    assert_eq!(unchanged["credential_ids"], json!([ssh, vault]));

    let (status, tofu, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "AWS-OpenTofu",
            "tool": "opentofu",
            "entrypoint": ".",
            "credential_ids": [aws]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{tofu}");
    assert_eq!(tofu["credential_ids"], json!([aws]));

    let (status, plain, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "Alt-NetBox",
            json!([]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{plain}");
    let legacy_id = plain["id"].as_str().unwrap().to_string();
    {
        let db = db_state.db().unwrap();
        db.execute(
            "INSERT INTO template_credentials (template_id, credential_id, position) VALUES (?1, ?2, 0)",
            rusqlite::params![legacy_id, netbox],
        )
        .unwrap();
    }
    let (status, legacy, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{legacy_id}"),
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "Alt-NetBox",
            json!([netbox]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{legacy}");
    assert_eq!(legacy["credential_ids"], json!([netbox]));

    let (status, refused, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{legacy_id}"),
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "Alt-NetBox",
            json!([netbox, aws]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(
        refused["error"],
        "this_credential_kind_does_not_fit_the_tool"
    );

    let (status, removed, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{legacy_id}"),
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "Alt-NetBox",
            json!([]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{removed}");
    assert!(removed["credential_ids"].as_array().unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn launch_overrides_follow_the_tool_credential_kinds() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-launch-kinds-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let db_state = state.clone();
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

    let ssh = create_credential(
        &app,
        &cookie,
        &team,
        "Launch SSH",
        "ssh",
        json!({"username": "deploy", "password": "launch-ssh-geheim"}),
    )
    .await;
    let aws = create_credential(
        &app,
        &cookie,
        &team,
        "Launch AWS",
        "aws",
        json!({"access_key": "AKIAEXAMPLE", "secret_key": "launch-aws-geheim"}),
    )
    .await;
    let netbox = create_credential(
        &app,
        &cookie,
        &team,
        "Launch NetBox",
        "netbox",
        json!({"token": "launch-netbox-geheim", "url": "https://netbox.example.com"}),
    )
    .await;
    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Launch hosts", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap().to_string();

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Launch kinds",
            "tool": "ansible",
            "entrypoint": "site.yml",
            "inventory_id": inventory_id,
            "prompt_on_launch": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();
    {
        let db = db_state.db().unwrap();
        db.execute(
            "INSERT INTO template_credentials (template_id, credential_id, position) VALUES (?1, ?2, 0)",
            rusqlite::params![template_id, netbox],
        )
        .unwrap();
    }

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
    assert_eq!(
        launched["parameters"]["credentials"][0]["id"],
        Value::String(netbox.clone())
    );
    let job_id = launched["id"].as_str().unwrap();
    let _ = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/cancel"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;

    let (status, refused, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}, "credential_ids": [aws]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(
        refused["error"],
        "this_credential_kind_does_not_fit_the_tool"
    );

    let (status, overridden, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}, "credential_ids": [ssh]})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{overridden}");
    assert_eq!(overridden["parameters"]["credentials"][0]["id"], ssh);
    let job_id = overridden["id"].as_str().unwrap();
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
    assert_eq!(row["credential_ids"], json!([netbox]));

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
    assert!(!script.contains("private_ca_file"));
    assert!(!script.contains("ldap_ca_file"));
    assert!(!script.contains("name=ca_file"));
    assert!(!script.contains("an_absolute_pem_path"));
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
    {
        let mut round_trip = current.clone();
        if let Some(object) = round_trip.as_object_mut() {
            object.remove("clear");
        }
        let (status, echoed, _) = call(
            &app,
            "PUT",
            "/api/settings",
            Some(&cookie),
            None,
            Some(round_trip),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "read-only fields rejected: {echoed}"
        );
    }
    assert_eq!(current["runner"], "local");
    assert_eq!(current["ldap"]["url"], "");
    assert!(current["ldap"].get("ca_file").is_none());
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
    assert!(saved["ldap"].get("ca_file").is_none());
    assert_eq!(saved["ldap"]["user_filter"], "(sAMAccountName={username})");
    assert_eq!(saved["ldap"]["bind_password_set"], true);
    assert_eq!(saved["proxy"]["password_set"], true);
    assert_eq!(saved["proxy"]["url"], "http://proxy.example:3128");
    assert_eq!(saved["runner"], "docker");
    assert_eq!(saved["job_timeout_secs"], 90);
    assert_eq!(saved["job_output_lines"], 4000);
    assert!(saved["ldap"].get("bind_password").is_none());
    assert!(saved["proxy"].get("password").is_none());

    let (status, partial, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(json!({"job_timeout_secs": 60})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{partial}");
    assert_eq!(partial["error"], "the_settings_body_is_incomplete");
    assert_eq!(partial["vars"]["fields"], "ldap, proxy, runner");

    let (status, bad_runner, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(json!({
            "ldap": {
                "url": "",
                "mode": "ldap"
            },
            "proxy": {
                "url": ""
            },
            "runner": "bogus",
            "job_timeout_secs": 60
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_runner}");
    assert_eq!(bad_runner["error"], "the_runner_mode_is_invalid");

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
    assert!(
        overview["runtime"].is_null()
            || overview["runtime"] == "docker"
            || overview["runtime"] == "podman",
        "{overview}"
    );
    assert_same_runner(&app, &cookie, &team, &overview).await;

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
    assert!(
        overview["runtime"].is_null()
            || overview["runtime"] == "docker"
            || overview["runtime"] == "podman",
        "{overview}"
    );
    assert_same_runner(&app, &cookie, &team, &overview).await;

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
    let kept_id = kept["id"].as_str().unwrap().to_string();
    let kept_commit = kept["parameters"]["project_commit"]
        .as_str()
        .expect("project commit")
        .to_string();
    let kept_full = kept["parameters"]["project_commit_full"]
        .as_str()
        .expect("full project commit")
        .to_string();
    assert_eq!(kept_commit.chars().count(), 7);
    assert_eq!(kept_full.chars().count(), 40);
    assert!(kept_full.starts_with(&kept_commit));
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
    let fresh_full = updated["parameters"]["project_commit_full"]
        .as_str()
        .expect("fresh full commit");
    assert_ne!(fresh_full, kept_full.as_str());

    let (status, synced_again, _) = call(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced_again}");
    let (status, old, _) = call(
        &app,
        "GET",
        &format!("/api/jobs/{kept_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{old}");
    assert_eq!(old["parameters"]["project_commit"], kept_commit);
    assert_eq!(old["parameters"]["project_commit_full"], kept_full);

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
        Some(template_write_body(&health)),
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
        Some(template_write_body(&health)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");

    let finished = launch_and_wait(&app, &cookie, &team, &health_id).await;
    assert_eq!(finished["status"], "successful", "{}", finished["events"]);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&outside);
}

#[tokio::test]
async fn a_hook_delivery_is_rejected_when_local_runs_are_admin_only() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-hook-local-admin-{nanos}"));
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

    let (status, hook, _) = call(
        &app,
        "POST",
        "/api/hooks",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "LocalRunsProbe",
            "match_alertname": "DiskSpaceLow",
            "template_id": health_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");
    let hook_id = hook["id"].as_str().unwrap();
    let token = hook["token"].as_str().unwrap();

    let (status, denied, _) = call(
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
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    assert_eq!(denied["error"], "local_runs_are_limited_to_administrators");

    let (status, deliveries, _) = call(
        &app,
        "GET",
        &format!("/api/hooks/{hook_id}/deliveries"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deliveries}");
    let rejected = deliveries
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["hook_id"] == hook_id)
        .unwrap();
    assert_eq!(rejected["status"], "rejected");
    assert!(
        rejected["summary"]
            .as_str()
            .unwrap()
            .contains("KOREOGRAF_LOCAL_RUNNER_FOR=all"),
        "{}",
        rejected["summary"]
    );

    let (status, tested, _) = call(
        &app,
        "POST",
        &format!("/api/hooks/{hook_id}/test"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{tested}");
    assert_eq!(
        tested["summary"],
        "Test, run as admin: 1 alert(s) started, first DiskSpaceLow."
    );
    let finished = wait_for_job(&app, &cookie, &team, tested["job_id"].as_str().unwrap()).await;
    assert_eq!(finished["status"], "successful", "{}", finished["events"]);

    let (status, workflow, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "LocalRunsWorkflowProbe",
            "description": "",
            "graph": {
                "nodes": [
                    {"id": "check", "template_id": health_id, "name": "Healthcheck"}
                ],
                "edges": []
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let workflow_id = workflow["id"].as_str().unwrap();

    let (status, wf_hook, _) = call(
        &app,
        "POST",
        "/api/hooks",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "LocalRunsWorkflowProbe",
            "match_alertname": "DiskSpaceLow",
            "workflow_id": workflow_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{wf_hook}");
    let wf_hook_id = wf_hook["id"].as_str().unwrap();
    let wf_token = wf_hook["token"].as_str().unwrap();

    let (status, denied_wf, _) = call(
        &app,
        "POST",
        &format!("/hooks/{wf_hook_id}/{wf_token}"),
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
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied_wf}");
    assert_eq!(
        denied_wf["error"],
        "local_runs_are_limited_to_administrators"
    );

    let (status, wf_deliveries, _) = call(
        &app,
        "GET",
        &format!("/api/hooks/{wf_hook_id}/deliveries"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{wf_deliveries}");
    let wf_rejected = wf_deliveries
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["hook_id"] == wf_hook_id)
        .unwrap();
    assert_eq!(wf_rejected["status"], "rejected");
    assert!(
        wf_rejected["summary"]
            .as_str()
            .unwrap()
            .contains("KOREOGRAF_LOCAL_RUNNER_FOR=all"),
        "{}",
        wf_rejected["summary"]
    );

    let (status, tested_wf, _) = call(
        &app,
        "POST",
        &format!("/api/hooks/{wf_hook_id}/test"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{tested_wf}");
    assert_eq!(
        tested_wf["summary"],
        "Test, run as admin: 1 alert(s) started, first DiskSpaceLow."
    );
    let wf_finished = wait_for_workflow(
        &app,
        &cookie,
        &team,
        tested_wf["workflow_job_id"].as_str().unwrap(),
    )
    .await;
    assert_eq!(wf_finished["status"], "successful", "{}", wf_finished);
    let _ = std::fs::remove_dir_all(&dir);

    let dir = std::env::temp_dir().join(format!("koreograf-hook-local-all-{nanos}"));
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

    let (status, hook, _) = call(
        &app,
        "POST",
        "/api/hooks",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "LocalRunsProbe",
            "match_alertname": "DiskSpaceLow",
            "template_id": health_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");
    let hook_id = hook["id"].as_str().unwrap();
    let token = hook["token"].as_str().unwrap();

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
    let finished = wait_for_job(&app, &cookie, &team, accepted["job_id"].as_str().unwrap()).await;
    assert_eq!(finished["status"], "successful", "{}", finished["events"]);

    let (status, workflow, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "LocalRunsWorkflowProbe",
            "description": "",
            "graph": {
                "nodes": [
                    {"id": "check", "template_id": health_id, "name": "Healthcheck"}
                ],
                "edges": []
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let workflow_id = workflow["id"].as_str().unwrap();

    let (status, wf_hook, _) = call(
        &app,
        "POST",
        "/api/hooks",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "LocalRunsWorkflowProbe",
            "match_alertname": "DiskSpaceLow",
            "workflow_id": workflow_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{wf_hook}");
    let wf_hook_id = wf_hook["id"].as_str().unwrap();
    let wf_token = wf_hook["token"].as_str().unwrap();

    let (status, accepted_wf, _) = call(
        &app,
        "POST",
        &format!("/hooks/{wf_hook_id}/{wf_token}"),
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
    assert_eq!(status, StatusCode::OK, "{accepted_wf}");
    assert_eq!(accepted_wf["status"], "accepted");
    let wf_finished = wait_for_workflow(
        &app,
        &cookie,
        &team,
        accepted_wf["workflow_job_id"].as_str().unwrap(),
    )
    .await;
    assert_eq!(wf_finished["status"], "successful", "{}", wf_finished);
    let _ = std::fs::remove_dir_all(&dir);
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
async fn a_hook_can_be_edited_and_keeps_its_token() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-hook-edit-{nanos}"));
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
    let template_id = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Healthcheck")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/hooks",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Before",
            "match_alertname": "DiskSpaceLow",
            "template_id": template_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap();
    let token = created["token"].as_str().unwrap();

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/hooks/{id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "After",
            "match_alertname": "CpuHigh",
            "match_status": "resolved",
            "template_id": template_id,
            "extra_vars": {"limit": "web-1"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "After");
    assert_eq!(updated["match_alertname"], "CpuHigh");
    assert_eq!(updated["match_status"], "resolved");
    assert_eq!(updated["token"], token);
    assert_eq!(updated["extra_vars"]["limit"], "web-1");

    let (status, hooks, _) =
        call(&app, "GET", "/api/hooks", Some(&cookie), Some(&team), None).await;
    assert_eq!(status, StatusCode::OK, "{hooks}");
    let stored = hooks
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .unwrap();
    assert_eq!(stored["name"], "After");
    assert_eq!(stored["token"], token);
    assert_eq!(stored["created_at"], created["created_at"]);
    let _ = std::fs::remove_dir_all(&dir);
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
async fn json_bodies_are_rejected_by_kind_and_objects_reject_arrays() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-json-kind-{nanos}"));
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
    let launch = format!("/api/templates/{health_id}/launch");

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Sleeper",
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": "sleep 30\necho should-not-run\n"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let sleeper_id = created["id"].as_str().unwrap();
    let sleeper_launch = format!("/api/templates/{sleeper_id}/launch");

    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Manual",
            "sources": [{"kind": "ini", "name": "Hosts", "body": "web-1\n"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let source_id = inventory["sources"][0]["id"].as_str().unwrap();
    let source = format!(
        "/api/inventories/{}/sources/{source_id}",
        inventory["id"].as_str().unwrap()
    );

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
    let workflow_id = workflows.as_array().unwrap()[0]["id"].as_str().unwrap();
    let workflow_launch = format!("/api/workflows/{workflow_id}/launch");

    let before = job_count(&app, &cookie, &team).await;
    let invalid = (StatusCode::BAD_REQUEST, "json_body_invalid");
    let wrong_type = json!({"inventory_id": 1}).to_string();
    expect_json_error(
        &app,
        "POST",
        &launch,
        Some(&cookie),
        Some(&team),
        br#"{"limit":"#,
        invalid,
    )
    .await;
    expect_json_error(
        &app,
        "POST",
        &launch,
        Some(&cookie),
        Some(&team),
        b"",
        invalid,
    )
    .await;
    expect_json_error(
        &app,
        "POST",
        &launch,
        Some(&cookie),
        Some(&team),
        wrong_type.as_bytes(),
        invalid,
    )
    .await;
    expect_json_error(
        &app,
        "POST",
        &launch,
        Some(&cookie),
        Some(&team),
        b"[]",
        invalid,
    )
    .await;
    expect_json_error(
        &app,
        "POST",
        &launch,
        Some(&cookie),
        Some(&team),
        b"null",
        invalid,
    )
    .await;
    expect_json_error(&app, "POST", "/api/login", None, None, b"{}", invalid).await;
    expect_json_error(
        &app,
        "POST",
        &workflow_launch,
        Some(&cookie),
        Some(&team),
        b"[]",
        invalid,
    )
    .await;
    expect_json_error(
        &app,
        "PUT",
        &source,
        Some(&cookie),
        Some(&team),
        b"[]",
        invalid,
    )
    .await;
    let (status, patched, _) = call(
        &app,
        "PUT",
        &source,
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(patched["sources"][0]["name"], "Hosts");

    let mut huge = vec![b' '; 2_097_152 + 64];
    huge[..1].copy_from_slice(b"{");
    expect_json_error(
        &app,
        "POST",
        &launch,
        Some(&cookie),
        Some(&team),
        &huge,
        (StatusCode::PAYLOAD_TOO_LARGE, "request_too_large"),
    )
    .await;
    let signed_out = (StatusCode::UNAUTHORIZED, "sign_in_is_required");
    expect_json_error(&app, "POST", &launch, None, Some(&team), &huge, signed_out).await;
    expect_json_error(&app, "POST", &launch, None, Some(&team), b"[]", signed_out).await;
    assert_eq!(job_count(&app, &cookie, &team).await, before);

    let (status, started, _) = call(
        &app,
        "POST",
        &sleeper_launch,
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    let job_id = started["id"].as_str().unwrap();
    assert_eq!(job_count(&app, &cookie, &team).await, before + 1);
    let relaunch = format!("/api/jobs/{job_id}/relaunch");
    expect_json_error(
        &app,
        "POST",
        &relaunch,
        Some(&cookie),
        Some(&team),
        b"[]",
        invalid,
    )
    .await;
    assert_eq!(job_count(&app, &cookie, &team).await, before + 1);

    let (status, canceled, _) = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/cancel"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    assert_eq!(canceled["status"], "canceled");
    let (status, again, _) = call(
        &app,
        "POST",
        &relaunch,
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{again}");
    let again_id = again["id"].as_str().unwrap();
    assert_ne!(again_id, job_id);
    let (status, stopped, _) = call(
        &app,
        "POST",
        &format!("/api/jobs/{again_id}/cancel"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stopped}");
    assert_eq!(stopped["status"], "canceled");

    let (status, hooks, _) =
        call(&app, "GET", "/api/hooks", Some(&cookie), Some(&team), None).await;
    assert_eq!(status, StatusCode::OK, "{hooks}");
    let hook = &hooks.as_array().unwrap()[0];
    let (status, ignored, _) = call(
        &app,
        "POST",
        &format!(
            "/hooks/{}/{}",
            hook["id"].as_str().unwrap(),
            hook["token"].as_str().unwrap()
        ),
        None,
        None,
        Some(json!([])),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ignored}");
    assert_eq!(ignored["status"], "ignored");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn unknown_fields_are_rejected_on_write_endpoints() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-unknown-{nanos}"));
    let origin = std::env::temp_dir().join(format!("koreograf-unknown-origin-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&origin).unwrap();
    let mut cfg = config(&dir);
    cfg.project_roots.push(origin.clone());
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
    let organization = me["teams"][0]["organization_id"]
        .as_str()
        .unwrap()
        .to_string();

    let project = json!({
        "name": "Unknown project",
        "source_type": "git",
        "source": format!("file://{}", origin.display())
    });
    let mut rejected = project.clone();
    rejected["credential_name"] = json!("ignored");
    expect_unknown_field(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        &rejected,
        "credential_name",
    )
    .await;
    let (status, project, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(project),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap().to_string();

    let assignment = json!({"organization_id": organization, "team_ids": []});
    let mut rejected = assignment.clone();
    rejected["credential_name"] = json!("ignored");
    expect_unknown_field(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "credential_name",
    )
    .await;
    let (status, assigned, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        Some(assignment),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{assigned}");

    let inventory = json!({"name": "Unknown inventory", "body": "[all]\nbox\n"});
    let mut rejected = inventory.clone();
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(inventory),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap().to_string();

    let update = json!({"name": "Unknown inventory", "body": ""});
    let mut rejected = update.clone();
    rejected["sources"] = json!([]);
    expect_unknown_field(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "sources",
    )
    .await;
    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        Some(update),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");

    let source = json!({"kind": "ini", "name": "Extra source", "body": "web-2\n"});
    let mut rejected = source.clone();
    rejected["group_name_prefix"] = json!(true);
    expect_unknown_field(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "group_name_prefix",
    )
    .await;
    let (status, inventory, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources"),
        Some(&cookie),
        Some(&team),
        Some(source),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{inventory}");
    let source_id = inventory["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Extra source")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let source_update = json!({"body": "web-3\n"});
    let mut rejected = source_update.clone();
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, inventory, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team),
        Some(source_update),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{inventory}");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/dcim/devices/", axum::routing::get(nb_devices));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });
    let netbox_credential = create_credential(
        &app,
        &cookie,
        &team,
        "Unknown NetBox",
        "netbox",
        json!({"token": "secret-token", "url": format!("http://{addr}")}),
    )
    .await;
    let netbox = json!({
        "name": "Unknown NetBox",
        "credential_id": netbox_credential,
        "query_filters": ["status=active"],
        "virtual_machines": false
    });
    let mut rejected = netbox.clone();
    rejected["group_name_prefix"] = json!(true);
    expect_unknown_field(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        &rejected,
        "group_name_prefix",
    )
    .await;
    let (status, netbox, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(netbox),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{netbox}");

    let template = json!({
        "name": "Unknown template",
        "tool": "shell",
        "spec": {"script": "true"}
    });
    let mut rejected = template.clone();
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(template),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    let template_update = json!({
        "name": "Unknown template changed",
        "tool": "shell",
        "spec": {"script": "true"}
    });
    let mut rejected = template_update.clone();
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, changed, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        Some(template_update),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");

    let mut rejected = daily_schedule("Unknown schedule");
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, schedule, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Unknown schedule")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{schedule}");
    let schedule_id = schedule["id"].as_str().unwrap().to_string();

    let mut rejected = daily_schedule("Unknown schedule changed");
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, schedule, _) = call(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Unknown schedule changed")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{schedule}");

    let credential = json!({
        "name": "Unknown credential",
        "kind": "ssh",
        "secret": {"username": "deploy", "password": "secret"}
    });
    let mut rejected = credential.clone();
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(credential),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"].as_str().unwrap().to_string();

    let credential_update = json!({
        "name": "Unknown credential changed",
        "secret": {"username": "deploy", "password": "secret"}
    });
    let mut rejected = credential_update.clone();
    rejected["kind"] = json!("ssh");
    expect_unknown_field(
        &app,
        "PUT",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "kind",
    )
    .await;
    let (status, credential, _) = call(
        &app,
        "PUT",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team),
        Some(credential_update),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{credential}");

    let mut rejected = json!({"reset": true});
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, settings, _) = call(
        &app,
        "PUT",
        "/api/settings",
        Some(&cookie),
        None,
        Some(json!({"reset": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{settings}");

    let hook = json!({
        "name": "Unknown hook",
        "match_alertname": "Alert",
        "match_status": "firing",
        "template_id": template_id
    });
    let mut rejected = hook.clone();
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "POST",
        "/api/hooks",
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, hook, _) = call(
        &app,
        "POST",
        "/api/hooks",
        Some(&cookie),
        Some(&team),
        Some(hook),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");

    let (status, workflow, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Unknown workflow",
            "graph": {"nodes": [], "edges": []}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let workflow_id = workflow["id"].as_str().unwrap().to_string();
    let mut rejected = json!({
        "name": "Unknown workflow",
        "graph": {"nodes": [], "edges": []}
    });
    rejected["api_endpoint"] = json!("ignored");
    expect_unknown_field(
        &app,
        "PUT",
        &format!("/api/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        &rejected,
        "api_endpoint",
    )
    .await;
    let (status, changed, _) = call(
        &app,
        "PUT",
        &format!("/api/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Unknown workflow changed",
            "graph": {"nodes": [], "edges": []}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&origin);
}

async fn expect_unknown_field(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    team: Option<&str>,
    body: &Value,
    field: &str,
) {
    let mut headers = vec![("content-type", "application/json")];
    if let Some(team) = team {
        headers.push(("x-team-id", team));
    }
    let (status, bytes, _) = call_raw(
        app,
        method,
        uri,
        cookie,
        &headers,
        Some(body.to_string().into_bytes()),
    )
    .await;
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(parsed["error"], "json_field_is_not_supported", "{text}");
    assert_eq!(parsed["vars"]["field"], field, "{text}");
}

#[tokio::test]
async fn job_list_pages_searches_filters_and_reloads_the_hash() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-job-list-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state.clone());
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
    {
        let db = state.db().expect("database");
        for index in 0..60 {
            let id = format!("job-{index:02}");
            let name = format!("Job {index:02}");
            let tool = ["shell", "ansible", "opentofu"][index % 3];
            let job_status = if index % 5 == 0 {
                "failed"
            } else {
                "successful"
            };
            let trigger = if index % 2 == 0 { "manual" } else { "schedule" };
            let created = format!("2026-02-01T00:{index:02}:00Z");
            db.execute(
                "INSERT INTO jobs (
                    id, team_id, name, tool, status, extra_vars, launched_by, trigger, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, '{}', 'admin', ?6, ?7)",
                (
                    id.as_str(),
                    team.as_str(),
                    name.as_str(),
                    tool,
                    job_status,
                    trigger,
                    created.as_str(),
                ),
            )
            .unwrap();
        }
    }

    let (status, page, _) = call(
        &app,
        "GET",
        "/api/jobs?limit=25&offset=25",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["total"], 60);
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 25);
    assert_eq!(items[0]["id"], "job-34");

    let (status, found, _) = call(
        &app,
        "GET",
        "/api/jobs?q=Job%2005&limit=25",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["total"], 1);
    assert_eq!(found["items"][0]["id"], "job-05");

    let (status, failed, _) = call(
        &app,
        "GET",
        "/api/jobs?status=failed&limit=100",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{failed}");
    assert_eq!(failed["total"], 12);
    assert!(failed["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|job| job["status"] == "failed"));

    let (status, combined, _) = call(
        &app,
        "GET",
        "/api/jobs?status=failed&tool=shell&limit=100",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{combined}");
    assert_eq!(combined["total"], 4);

    let (status, bad, _) = call(
        &app,
        "GET",
        "/api/jobs?since=not-a-date",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    assert_eq!(bad["error"], "invalid_query_parameter");
    assert_eq!(bad["vars"]["field"], "since");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn job_list_hash_round_trips_filters_and_pages() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let functions = format!(
        "{}\n{}",
        extract_braced_function(&script, "function jobsState"),
        extract_braced_function(&script, "function jobsHash")
    );
    let probe = format!(
        r##"{functions}
let location = {{ hash: "#/jobs?status=failed&page=2&size=50&q=Job%2005&tool=shell&trigger=schedule&template=tpl-a" }};
const view = jobsState();
if (view.page !== 2 || view.size !== 50 || view.q !== "Job 05" || view.status !== "failed" || view.tool !== "shell" || view.trigger !== "schedule" || view.template !== "tpl-a") {{
  console.error(JSON.stringify(view));
  process.exit(1);
}}
if (jobsHash(view) !== "#/jobs?page=2&size=50&q=Job+05&status=failed&tool=shell&trigger=schedule&template=tpl-a") {{
  console.error(jobsHash(view));
  process.exit(1);
}}
location.hash = "#/jobs";
const plain = jobsState();
if (plain.page !== 1 || plain.size !== 25 || jobsHash(plain) !== "#/jobs") {{
  console.error(jobsHash(plain));
  process.exit(1);
}}
"##
    );
    run_node_probe("job-list-hash", &probe);
}

#[test]
fn job_log_summary_search_and_first_failure() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let functions = format!(
        "{}\n{}\n{}",
        extract_braced_function(&script, "function parseRecapLine"),
        extract_braced_function(&script, "function recapFailed"),
        extract_braced_function(&script, "function searchSegments")
    );
    let probe = format!(
        r##"{functions}
const fixture = [
  "PLAY RECAP *********************************************************************",
  "web-1                      : ok=3    changed=1    unreachable=0    failed=0    skipped=1    rescued=0    ignored=0",
  "web-2                      : ok=1    changed=0    unreachable=1    failed=1    skipped=0    rescued=0    ignored=0"
];
const rows = fixture.map(parseRecapLine).filter(Boolean);
if (rows.length !== 2) {{
  console.error(JSON.stringify(rows));
  process.exit(1);
}}
if (rows[0].host !== "web-1" || rows[0].ok !== 3 || rows[0].changed !== 1 || recapFailed(rows[0])) {{
  console.error(JSON.stringify(rows[0]));
  process.exit(1);
}}
if (rows[1].host !== "web-2" || rows[1].unreachable !== 1 || rows[1].failed !== 1 || !recapFailed(rows[1])) {{
  console.error(JSON.stringify(rows[1]));
  process.exit(1);
}}
const raw = '<script>alert("&lt;")</script> &lt; <b>x</b>';
for (const query of ["script", "&lt;", "<b>"]) {{
  const segments = searchSegments(raw, query);
  if (segments.map((item) => item.text).join("") !== raw) {{
    console.error(JSON.stringify(segments));
    process.exit(1);
  }}
  if (!segments.some((item) => item.hit)) {{
    console.error(query);
    process.exit(1);
  }}
}}
"##
    );
    run_node_probe("job-log-summary", &probe);
    let paint = extract_braced_function(&script, "function paintLogLine");
    assert!(paint.contains("createTextNode"));
    assert!(paint.contains("textContent"));
    assert!(!paint.contains("innerHTML"));
    assert!(script.contains("id=\"log-first-failure\""));
    assert!(script.contains("id=\"log-jump-end\""));
    assert!(script.contains("id=\"log-search\""));
    assert!(script.contains("id=\"log-wrap\""));
    assert!(script.contains("id=\"log-result\""));
    assert!(script.contains("state.firstFailure.scrollIntoView"));
    assert!(script.contains("state.atBottom"));
    let parameters = extract_braced_function(&script, "function jobParameters");
    assert!(parameters.contains("paramRow(\"credentials\""));
    assert!(!parameters.contains("paramRow(\"nav.access\""));
    assert!(parameters.contains("project_commit_full"));
}

async fn job_count(app: &axum::Router, cookie: &str, team: &str) -> usize {
    let (status, jobs, _) = call(app, "GET", "/api/jobs", Some(cookie), Some(team), None).await;
    assert_eq!(status, StatusCode::OK, "{jobs}");
    jobs["items"].as_array().expect("job list");
    jobs["total"].as_u64().expect("job total") as usize
}

async fn expect_json_error(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    team: Option<&str>,
    body: &[u8],
    expected: (StatusCode, &str),
) {
    let mut headers = vec![("content-type", "application/json")];
    if let Some(team) = team {
        headers.push(("x-team-id", team));
    }
    let (status, bytes, _) =
        call_raw(app, method, uri, cookie, &headers, Some(body.to_vec())).await;
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(status, expected.0, "{text}");
    let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(parsed["error"], expected.1, "{text}");
    assert!(!text.contains("Expected request"), "{text}");
    assert!(!text.contains("Failed to parse"), "{text}");
    assert!(!text.contains("Failed to deserialize"), "{text}");
    assert!(!text.contains("Failed to buffer"), "{text}");
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
async fn jobs_that_end_before_execution_are_still_counted() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-finished-early-{nanos}"));
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

    let missing = format!("file:///tmp/koreograf-missing-{nanos}");
    let (status, project, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(team),
        Some(json!({"name": "Unsynced", "source_type": "git", "source": missing})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap();
    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(team),
        Some(json!({"name": "Unsynced hosts", "body": "[all]\nlocalhost\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap();
    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Not synced",
            "tool": "ansible",
            "project_id": project_id,
            "inventory_id": inventory_id,
            "entrypoint": "site.yml",
            "update_on_launch": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();
    let failed = launch_and_wait(&app, &cookie, team, template_id).await;
    assert_eq!(failed["status"], "failed", "{}", failed["events"]);
    assert!(has_line(&failed, "system", "Sync the Git project first."));
    let metrics = metrics_text(&app).await;
    assert!(
        metrics.contains(
            "koreograf_jobs_finished_total{tool=\"ansible\",playbook=\"site.yml\",status=\"failed\"} 1"
        ),
        "{metrics}"
    );
    assert!(
        !metrics.contains("koreograf_jobs_started_total{tool=\"ansible\",playbook=\"site.yml\""),
        "{metrics}"
    );

    let (status, sleeper, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Pending sleeper",
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": "sleep 30\necho should-not-run\n"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{sleeper}");
    let sleeper_id = sleeper["id"].as_str().unwrap();
    let (status, job, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{sleeper_id}/launch"),
        Some(&cookie),
        Some(team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    let job_id = job["id"].as_str().unwrap();
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

    let mut metrics = String::new();
    for _ in 0..20 {
        metrics = metrics_text(&app).await;
        if finished_total(&metrics, "canceled") >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(finished_total(&metrics, "canceled"), 1, "{metrics}");

    tokio::time::sleep(Duration::from_millis(400)).await;
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
    assert!(!events.contains("should-not-run"), "{events}");
    if let Some(pid) = body["events"].as_array().and_then(|events| {
        events.iter().find_map(|event| {
            event["line"]
                .as_str()
                .and_then(|line| line.strip_prefix("Process group "))
                .and_then(|line| line.strip_suffix('.'))
                .map(str::to_string)
        })
    }) {
        let proc = std::path::PathBuf::from(format!("/proc/{pid}"));
        for _ in 0..40 {
            if !proc.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(!proc.exists(), "process {pid} is still running");
    }
    let metrics = metrics_text(&app).await;
    assert_eq!(finished_total(&metrics, "canceled"), 1, "{metrics}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn finished_total(metrics: &str, status: &str) -> u64 {
    let needle = format!("status=\"{status}\"");
    metrics
        .lines()
        .filter(|line| line.starts_with("koreograf_jobs_finished_total{") && line.contains(&needle))
        .map(|line| {
            line.rsplit(' ')
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0)
        })
        .sum()
}

async fn metrics_text(app: &axum::Router) -> String {
    let (status, body, _) = call(app, "GET", "/metrics", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    body.as_str().unwrap_or("").to_string()
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
    assert!(script.contains("id: \"schedule-form\""));
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

    let mut preview_body = daily_schedule("ignored");
    preview_body
        .as_object_mut()
        .expect("schedule object")
        .remove("name");
    let (status, preview, _) = call(
        &app,
        "POST",
        "/api/schedules/preview",
        Some(&cookie),
        Some(&team),
        Some(preview_body),
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

#[tokio::test]
async fn a_failed_missed_run_stays_visible() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-missed-keep-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = config(&dir);
    cfg.local_jobs = LocalJobs::Admins;
    let state = koreograf::boot(cfg).unwrap();
    let app = koreograf::router(state.clone());

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
    let (status, _, operator_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "operator", "password": "operator-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let operator_cookie = operator_cookie.expect("operator cookie");
    let (status, created, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{health_id}/schedules"),
        Some(&operator_cookie),
        Some(&team),
        Some(daily_schedule("Operator miss")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let schedule_id = created["id"].as_str().unwrap();
    mark_missed(&state, schedule_id);
    let (status, denied, _) = call(
        &app,
        "POST",
        &format!("/api/schedules/{schedule_id}/missed"),
        Some(&operator_cookie),
        Some(&team),
        Some(json!({"accept": true})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    assert_eq!(denied["error"], "local_runs_are_limited_to_administrators");
    let (status, listed, _) = call(
        &app,
        "GET",
        &format!("/api/templates/{health_id}/schedules"),
        Some(&operator_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let row = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == schedule_id)
        .expect("schedule");
    assert_eq!(row["missed_at"], "2026-10-21T04:00:00Z");
    let (status, jobs, _) = call(
        &app,
        "GET",
        "/api/jobs",
        Some(&operator_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{jobs}");
    assert!(scheduled_jobs(&jobs, &health_id).is_empty(), "{jobs}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_edit_records_the_user_who_changed_the_schedule() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-schedule-editor-{nanos}"));
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
    let (status, created, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{health_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Admin clock")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["created_by"], "admin");
    assert_eq!(created["updated_by"], "admin");
    let schedule_id = created["id"].as_str().unwrap();
    let (status, _, operator_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "operator", "password": "operator-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let operator_cookie = operator_cookie.expect("operator cookie");
    let (status, edited, _) = call(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&operator_cookie),
        Some(&team),
        Some(daily_schedule("Operator clock")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["name"], "Operator clock");
    assert_eq!(edited["created_by"], "admin");
    assert_eq!(edited["updated_by"], "operator");
    let (status, listed, _) = call(
        &app,
        "GET",
        &format!("/api/templates/{health_id}/schedules"),
        Some(&operator_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let row = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == schedule_id)
        .expect("schedule");
    assert_eq!(row["created_by"], "admin");
    assert_eq!(row["updated_by"], "operator");
    let _ = std::fs::remove_dir_all(&dir);
}

fn template_write_body(item: &Value) -> Value {
    let mut body = item.clone();
    let map = body.as_object_mut().expect("template object");
    for key in ["id", "team_id", "created_at"] {
        map.remove(key);
    }
    body
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
    jobs["items"]
        .as_array()
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

async fn wait_for_workflow(app: &axum::Router, cookie: &str, team: &str, run_id: &str) -> Value {
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let (status, body, _) = call(
            app,
            "GET",
            &format!("/api/workflow-jobs/{run_id}"),
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
    panic!("workflow run {run_id} did not finish");
}

#[test]
fn the_image_starts_the_binary_entrypoint_behind_a_static_init() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dockerfile = std::fs::read_to_string(root.join("Dockerfile")).expect("Dockerfile");
    assert!(!dockerfile.contains("entrypoint.sh"));
    assert!(dockerfile.contains("ARG TINI_VERSION=0.19.0"));
    assert!(dockerfile.contains("ARG TINI_SHA256_AMD64="));
    assert!(dockerfile.contains("ARG TINI_SHA256_ARM64="));
    assert!(dockerfile.contains(
        "https://github.com/krallin/tini/releases/download/v$TINI_VERSION/tini-static-$tini_arch"
    ));
    assert!(dockerfile.contains("install -D -m 755 /tmp/tini-static /out/sbin/tini"));
    assert!(dockerfile.contains("USER 10001:10001"));
    assert!(dockerfile.contains(
        "ENTRYPOINT [\"/sbin/tini\", \"--\", \"/usr/local/bin/koreograf\", \"entrypoint\"]"
    ));
    assert!(dockerfile.contains("CMD [\"koreograf\"]"));
    assert!(dockerfile.contains("CMD [\"/usr/local/bin/koreograf\", \"healthcheck\"]"));
    let docs = std::fs::read_to_string(root.join("docs/installation.md")).expect("installation.md");
    assert!(docs.contains("`/sbin/tini` is PID 1"));
    assert!(docs.contains("`user: \"10001:10001\"`"));
    assert!(docs.contains("koreograf entrypoint"));
    assert!(docs.contains("cargo run"));
}

#[test]
fn the_image_labels_name_the_version_and_the_source() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dockerfile = std::fs::read_to_string(root.join("Dockerfile")).expect("Dockerfile");
    assert!(dockerfile.contains("org.opencontainers.image.version=\"${VERSION}\""));
    assert!(dockerfile.contains("org.opencontainers.image.source=\"${SOURCE}\""));
    assert!(dockerfile.contains("org.opencontainers.image.revision=\"${GIT_SHA}\""));
    assert!(dockerfile.contains("KOREOGRAF_GIT_SHA=${GIT_SHA}"));
    assert!(dockerfile.contains("KOREOGRAF_BUILT_AT=${BUILT_AT}"));
    let version = env!("CARGO_PKG_VERSION");
    assert!(dockerfile.contains("# x-release-please-start-version"));
    assert!(dockerfile.contains(&format!("ARG VERSION={version}\n")));
    assert!(dockerfile.contains("# x-release-please-end"));
    assert!(!dockerfile.contains(&format!("ARG VERSION={version} #")));
    let release =
        std::fs::read_to_string(root.join(".github/workflows/release.yml")).expect("release.yml");
    assert!(release.contains("VERSION=${{ steps.tags.outputs.version }}"));
    assert!(release.contains("SOURCE=https://github.com/koreograf/koreograf"));
    assert!(release.contains("GIT_SHA=${{ steps.build-info.outputs.sha }}"));
    assert!(release.contains("BUILT_AT=${{ steps.build-info.outputs.date }}"));
}

#[test]
fn the_packages_ship_a_systemd_unit_and_a_pinned_nfpm_config() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let nfpm = std::fs::read_to_string(root.join("packaging/nfpm.yaml")).expect("nfpm.yaml");
    assert!(nfpm.contains("version_schema: none"));
    assert!(nfpm.contains("openssh-client"));
    assert!(nfpm.contains("openssh-clients"));
    assert!(nfpm.contains("ca-certificates"));
    assert!(nfpm.contains("type: config|noreplace"));
    assert!(!nfpm.contains("type: ghost"));
    assert!(nfpm.contains("/lib/systemd/system/koreograf.service"));
    assert!(nfpm.contains("/etc/koreograf/koreograf.env"));
    let unit =
        std::fs::read_to_string(root.join("packaging/koreograf.service")).expect("unit file");
    assert!(unit.contains("User=koreograf"));
    assert!(unit.contains("Group=koreograf"));
    assert!(unit.contains("EnvironmentFile=/etc/koreograf/koreograf.env"));
    assert!(unit.contains("ExecStart=/usr/bin/koreograf"));
    assert!(unit.contains("Restart=on-failure"));
    assert!(!unit.contains("entrypoint"));
    let env = std::fs::read_to_string(root.join("packaging/koreograf.env")).expect("env file");
    assert!(env.contains("KOREOGRAF_DATA=/var/lib/koreograf"));
    assert!(env.contains("KOREOGRAF_BIND=127.0.0.1:8080"));
    let release =
        std::fs::read_to_string(root.join(".github/workflows/release.yml")).expect("release.yml");
    assert!(release.contains("NFPM_VERSION: 2.48.0"));
    assert!(release.contains("dist/koreograf_${version}_${PACKAGE_ARCH}.deb"));
    assert!(release.contains("dist/koreograf_${version}_${PACKAGE_ARCH}.rpm"));
    assert!(release.contains("gh release upload \"$TAG\" dist/*.deb dist/*.rpm"));
    assert!(release.contains("gh release view \"$TAG\" >/dev/null"));
    let ci = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("ci.yml");
    assert!(ci.contains("--packager deb"));
    assert!(!ci.contains("--packager rpm"));
    let docs = std::fs::read_to_string(root.join("docs/installation.md")).expect("installation.md");
    assert!(docs.contains("sudo apt install ./koreograf_<version>_amd64.deb"));
    assert!(docs.contains("sudo dnf install ./koreograf_<version>_amd64.rpm"));
}

#[tokio::test]
async fn version_is_available_without_signing_in() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-version-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);
    let (status, body, _) = call(&app, "GET", "/api/version", None, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ui_reads_version_and_links_the_repository() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let login = extract_braced_function(&script, "function renderLogin");
    let shell = extract_braced_function(&script, "function shell");
    let version_slot = extract_braced_function(&script, "function versionSlot");
    let load = extract_braced_function(&script, "async function loadVersion");
    for body in [&login, &shell] {
        assert!(
            body.contains("versionSlot()"),
            "a page does not render the build line"
        );
        assert!(
            body.contains("loadVersion()"),
            "a page does not request the version"
        );
    }
    assert!(load.contains("/api/version"));
    assert!(load.contains("data.commit"));
    assert!(version_slot.contains("https://github.com/koreograf/koreograf"));
    assert!(version_slot.contains("target=\"_blank\""));
    assert!(version_slot.contains("rel=\"noopener noreferrer\""));
}

#[test]
fn auto_mode_names_a_missing_runtime() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let runner = std::fs::read_to_string(root.join("src/runner.rs")).expect("runner.rs");
    assert!(runner
        .contains("Mount the Docker socket, or set the runner mode to docker to fail instead."));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    assert!(script.contains("id=\"environment-runtime-warning\""));
    assert!(script.contains("this_execution_environment_is_ignored_until_docker_or"));
    assert!(script.contains("no_container_runtime"));
    assert!(script.contains("api(\"GET\", \"/api/runner\")"));
    assert_environment_warning_cases(&script);
    let en = std::fs::read_to_string(root.join("web/i18n/en.json")).expect("en");
    let de = std::fs::read_to_string(root.join("web/i18n/de.json")).expect("de");
    assert!(
        en.contains("Mount the Docker socket, or set the runner mode to docker to fail instead.")
    );
    assert!(en.contains("In automatic mode a set image is also ignored"));
    assert!(de.contains("Docker-Socket"));
    assert!(de.contains("Im automatischen Modus"));
    let docs = std::fs::read_to_string(root.join("docs/configuration.md")).expect("docs");
    assert!(docs.contains("mount the Docker socket"));
    assert!(docs.contains("stops after ten seconds"));
    assert!(docs.contains("GET /api/runner"));
}

#[test]
fn schedule_overlay_uses_the_shared_wide_form() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    assert!(script.contains("wide: true"));
    assert!(script.contains("bodyClass: \"schedule-body\""));
    assert!(script.contains("class=\"schedule-layout\""));
    assert!(script.contains("class=\"schedule-fields\""));
    assert!(script.contains("class=\"schedule-preview\""));
    assert!(script.contains("id=\"schedule-pin\""));
    assert!(script.contains("fields_marked_with_a_star_are_required"));
    assert!(script.contains("class=\"field required\""));
    assert!(script.contains("classList.toggle(\"required\", after)"));
    assert!(script.contains("classList.toggle(\"required\", onDate)"));
    assert!(script.contains("!form.name.value.trim()"));
    assert!(!script.contains("select class=\"fit\" name=\"timezone\""));
    assert!(!script.contains("select class=\"fit\" name=\"frequency\""));
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    assert!(css.contains(".schedule-layout { display: grid; grid-template-columns: minmax(0, 1fr) minmax(15rem, 19rem);"));
    assert!(css.contains(".schedule-fields { overflow-y: auto;"));
    assert!(css.contains(".schedule-preview { overflow-y: auto;"));
    assert!(css.contains("@media (max-width: 900px)"));
    assert!(css.contains(".schedule-layout { grid-template-columns: 1fr; height: auto; }"));
    assert!(css.contains(".dialog-footer { display: block; }"));
    assert!(css.contains(".schedule-pin { display: block;"));
    assert!(
        css.contains(".overlay-card.dialog input:not([type=\"checkbox\"]):not([type=\"radio\"])")
    );
    assert!(!css.contains("#schedule-form input"));
}

#[test]
fn schedule_overlay_address_opens_and_closes_the_dialog() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    assert!(script.contains("if (request.edit)"));
    assert!(script.contains(
        "openScheduleForm(item.target_kind || kind, item.target_id || target, item, draw);"
    ));
    assert!(script.contains("openScheduleForm(openKind, openTarget, null, draw);"));
    assert!(!script.contains("fromHash"));
    assert!(script.contains("rememberScheduleHash({ edit: item.id })"));
    assert!(script.contains("rememberScheduleHash({ new: \"1\" })"));
    assert!(script.contains("onClose: () => {"));
    assert!(script.contains("document.querySelector(\"#schedule-form\")?.remove();"));
    let functions = format!(
        "{}\n{}\n{}",
        extract_braced_function(&script, "function scheduleQuery"),
        extract_braced_function(&script, "function scheduleAddress"),
        extract_braced_function(&script, "function scheduleNewTarget")
    );
    let probe = format!(
        r##"{functions}
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
const edit = scheduleQuery("#/schedules?edit=s1");
check(edit.edit === "s1" && edit.create === null, "edit was not read");
const create = scheduleQuery("#/schedules?new=1");
check(create.create === "1", "new was not read");
const template = scheduleNewTarget({{ create: "template:t1", kind: "", target: "" }});
check(template && template.kind === "template" && template.target === "t1", "template target was not read");
const workflow = scheduleNewTarget({{ create: "1", kind: "workflow", target: "w1" }});
check(workflow && workflow.kind === "workflow" && workflow.target === "w1", "workflow target was not read");
check(scheduleAddress({{ edit: "s1" }}, "#/schedules?page=2") === "#/schedules?page=2&edit=s1", "edit address was not built");
check(scheduleAddress({{ new: "1" }}, "#/templates/t1") === "#/templates/t1?new=1", "new address was not built");
check(scheduleAddress({{}}, "#/schedules?edit=s1&page=2") === "#/schedules?page=2", "address was not cleared");
"##
    );
    run_node_probe("schedule-address", &probe);
}

#[test]
fn schedule_overlay_guards_unsaved_edits() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    assert!(script.contains("dirty: () => dirty"));
    let form = extract_braced_function(&script, "function openScheduleForm");
    assert!(form.contains("leaveGuard = () => dialog.dirty();"));
    let submit = form
        .find("onSubmit: async")
        .expect("schedule dialog submit");
    let close = form
        .find("onClose: () => {")
        .expect("schedule dialog close");
    assert!(submit < close);
    assert!(form[submit..close].contains("leaveGuard = null;"));
    assert!(form[close..].contains("leaveGuard = null;"));
}

#[test]
fn edit_overlays_share_one_dialog_helper() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let helper = extract_function_with_destructured_params(&script, "function openEditDialog");
    assert!(helper.contains("openDialog({"));
    assert!(helper.contains("fields_marked_with_a_star_are_required"));
    assert!(helper.contains("role=\"tablist\""));
    assert!(helper.contains("data-dialog-tab="));
    assert!(helper.contains("data-dialog-panel="));
    assert!(helper.contains("ArrowRight"));
    assert!(helper.contains("ArrowLeft"));
    assert!(helper.contains("Home"));
    assert!(helper.contains("End"));
    assert!(helper.contains("has-error"));
    assert!(helper.contains("dialog.form.addEventListener(\"invalid\""));
    assert!(helper.contains("choose(panel.dataset.dialogPanel, false)"));
    assert!(helper.contains("leaveGuard = () => dialog.dirty();"));
    assert!(helper.contains("clearEditHash();"));
    let dialog = extract_function_with_destructured_params(&script, "function openDialog");
    assert!(dialog.contains("if (dirty && !confirm(t(\"discard_unsaved_changes\"))) return false;"));
    assert!(dialog.contains("handle(error);"));
    assert!(dialog.contains("if (onError) onError(error);"));
    let handle = extract_braced_function(&script, "function handle");
    assert!(handle.contains("const card = topmostCard();"));
    assert!(handle.contains("card.querySelector(\".dialog-body\")"));
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    assert!(css.contains(".dialog-tabs { display: flex;"));
    assert!(css.contains(".dialog-tab[aria-selected=\"true\"]"));
    assert!(css.contains(".dialog-tab.has-error::after"));
    assert!(css.contains(".dialog-panel[hidden] { display: none; }"));
    assert!(css.contains(
        ".overlay-card, .overlay-card.dialog, .overlay-card.dialog.tabbed { width: 100%;"
    ));
}

#[test]
fn edit_overlay_addresses_round_trip_and_old_pages_redirect() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    for redirect in [
        "location.replace(\"#/credentials?new=1\")",
        "location.replace(`#/credentials?edit=${encodeURIComponent(parts[1])}`)",
        "location.replace(\"#/organizations?new=1\")",
        "location.replace(`#/organizations?edit=${encodeURIComponent(parts[1])}`)",
        "location.replace(\"#/team?new=1\")",
        "location.replace(`#/team?edit=${encodeURIComponent(parts[1])}`)",
        "location.replace(\"#/users?new=1\")",
        "location.replace(`#/users?edit=${encodeURIComponent(parts[1])}`)",
        "location.replace(\"#/projects?new=1\")",
        "location.replace(\"#/inventories?new=1\")",
        "location.replace(\"#/hooks?new=1\")",
        "location.replace(`#/hooks?edit=${encodeURIComponent(parts[1])}`)",
        "location.replace(\"#/runtime?new=1\")",
        "location.replace(\"#/team?new=member\")",
    ] {
        assert!(script.contains(redirect), "missing redirect {redirect}");
    }
    for old in [
        "href=\"#/credentials/${",
        "href=\"#/organizations/${",
        "href=\"#/users/${",
        "addLink(\"#/credentials/new\"",
        "addLink(\"#/organizations/new\"",
        "addLink(\"#/team/new\"",
        "addLink(\"#/users/new\"",
        "addLink(\"#/projects/new\"",
        "addLink(\"#/inventories/new\"",
        "addLink(\"#/hooks/new\"",
        "addLink(\"#/environments/new\"",
        "addLink(\"#/team/members/new\"",
        "href=\"#/projects/new\"",
        "href=\"#/inventories/new\"",
        "href=\"#/hooks/new\"",
        "href=\"#/environments/new\"",
        "id=\"hook-form\"",
        "id=\"env-form\"",
        "id=\"member-form\"",
    ] {
        assert!(
            !script.contains(old),
            "old page link {old} is still in the UI"
        );
    }
    let functions = format!(
        "{}\n{}\n{}\n{}",
        extract_braced_function(&script, "function editQuery"),
        extract_braced_function(&script, "function editAddress"),
        extract_braced_function(&script, "function rememberEditHash"),
        extract_braced_function(&script, "function clearEditHash")
    );
    let probe = format!(
        r##"{functions}
let location = {{ hash: "#/credentials" }};
let shownHash = null;
let pushed = 0;
let replaced = 0;
const history = {{
  pushState(_state, _title, url) {{ pushed += 1; location.hash = url; }},
  replaceState(_state, _title, url) {{ replaced += 1; location.hash = url; }},
}};
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
const edit = editQuery("#/credentials?edit=c1&tab=memberships");
check(edit.edit === "c1" && edit.tab === "memberships" && edit.create === null, "edit and tab were not read");
const create = editQuery("#/organizations?new=1");
check(create.create === "1" && create.edit === "" && create.tab === "", "new was not read");
const project = editQuery("#/projects?edit=p1&tab=source");
check(project.edit === "p1" && project.tab === "source", "project edit and tab were not read");
const hook = editQuery("#/hooks?edit=h1");
check(hook.edit === "h1" && hook.create === null, "hook edit was not read");
const member = editQuery("#/team?new=member");
check(member.create === "member" && member.edit === "", "member create was not read");
check(editAddress({{ create: "1" }}, "#/credentials") === "#/credentials?new=1", "new address was not built");
check(editAddress({{ create: "1" }}, "#/runtime") === "#/runtime?new=1", "runtime new address was not built");
check(editAddress({{ edit: "e1" }}, "#/runtime") === "#/runtime?edit=e1", "runtime edit address was not built");
check(editAddress({{ create: "member" }}, "#/team") === "#/team?new=member", "member address was not built");
check(editAddress({{ create: "1" }}, "#/inventories") === "#/inventories?new=1", "inventory new address was not built");
check(editAddress({{ edit: "p1", tab: "source" }}, "#/projects") === "#/projects?edit=p1&tab=source", "project address was not built");
check(editAddress({{ edit: "u1", tab: "account" }}, "#/users?page=2") === "#/users?page=2&edit=u1&tab=account", "edit address was not built");
check(editAddress({{ edit: "t1" }}, "#/team?edit=t1&tab=general") === "#/team?edit=t1", "old tab was not cleared");
check(editAddress({{}}, "#/team?edit=t1&tab=general") === "#/team", "address was not cleared");
rememberEditHash({{ create: "1" }});
check(pushed === 1 && replaced === 0 && location.hash === "#/credentials?new=1" && shownHash === location.hash, "open did not push the address");
rememberEditHash({{ edit: "c1", tab: "memberships" }}, false);
check(pushed === 1 && replaced === 1 && location.hash === "#/credentials?edit=c1&tab=memberships", "tab did not replace the address");
clearEditHash();
check(replaced === 2 && location.hash === "#/credentials" && shownHash === "#/credentials", "close did not clear the address");
"##
    );
    run_node_probe("edit-address", &probe);
}

#[test]
fn credentials_and_admin_objects_use_the_edit_overlay() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let credentials = extract_braced_function(&script, "async function pageCredentials");
    assert!(credentials.contains("openCredentialDialog(item, !can(\"owner\"), request.tab)"));
    assert!(credentials.contains("openCredentialDialog(null)"));
    assert!(credentials.contains("#/credentials?edit="));
    assert!(credentials.contains("#/credentials?new=1"));
    assert!(credentials.contains("aria-label=\"${h(\"search_credentials\")}\""));
    assert!(credentials.contains(
        "confirm(t(\"delete_this_credential\", { name: button.dataset.credentialName }))"
    ));
    let credential = extract_braced_function(&script, "function openCredentialDialog");
    assert!(credential.contains("openEditDialog({"));
    assert!(credential.contains("api(\"POST\", \"/api/credentials\""));
    assert!(credential.contains("api(\"PUT\", `/api/credentials/"));
    assert!(credential.contains("api(\"DELETE\", `/api/credentials/"));
    assert!(credential.contains("data-credential-delete"));
    assert!(credential.contains("hint: \"fields_marked_with_a_star_are_required\""));
    assert!(credential.contains("hint: \"\","));
    assert!(credential.contains("confirm(t(\"delete_this_credential\", { name: item.name }))"));
    assert!(credential.contains("markSaved();"));
    assert!(credential.contains("close();"));
    assert!(credential.contains("await route();"));
    let organizations = extract_braced_function(&script, "async function pageOrganizations");
    assert!(organizations.contains("openOrganizationDialog(item, !me.is_admin, request.tab)"));
    assert!(organizations.contains("openOrganizationDialog(null)"));
    let organization = extract_braced_function(&script, "function openOrganizationDialog");
    assert!(organization.contains("openEditDialog({"));
    assert!(organization.contains("api(\"POST\", \"/api/organizations\""));
    assert!(organization.contains("api(\"PUT\", `/api/organizations/"));
    assert!(organization.contains("{ id: \"general\", label: t(\"general\")"));
    assert!(organization.contains("{ id: \"teams\", label: t(\"teams\")"));
    assert!(organization.contains("onTab: editing ? (tab) => rememberEditHash("));
    let team = extract_braced_function(&script, "function openTeamDialog");
    assert!(team.contains("openEditDialog({"));
    assert!(team.contains("api(\"POST\", \"/api/teams\""));
    assert!(team.contains("api(\"PUT\", `/api/teams/"));
    assert!(!script.contains("id=\"team-form\""));
    let users = extract_braced_function(&script, "async function pageUsers");
    assert!(users.contains("openUserDialog(item, teams, request.tab)"));
    assert!(users.contains("openUserCreateDialog()"));
    let user = extract_braced_function(&script, "function openUserDialog");
    assert!(user.contains("openEditDialog({"));
    assert!(user.contains("{ id: \"account\", label: t(\"account\")"));
    assert!(user.contains("{ id: \"memberships\", label: t(\"memberships\")"));
    assert!(user.contains("api(\"PUT\", `/api/users/"));
    assert!(user.contains("api(\"DELETE\", `/api/users/"));
    assert!(user.contains("onTab: (tab) => rememberEditHash("));
    let create = extract_braced_function(&script, "function openUserCreateDialog");
    assert!(create.contains("api(\"POST\", \"/api/users\""));
    assert!(create.contains("class=\"field required\""));
    assert!(!script.contains("id=\"create-user\""));
    for function in [&credential, &organization, &team, &create] {
        assert!(function.contains("markSaved();"));
        assert!(function.contains("await route();"));
    }
}

#[test]
fn projects_and_inventories_use_the_edit_overlay() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let projects = extract_braced_function(&script, "async function pageProjects");
    assert!(projects.contains("openProjectDialog(item, request.tab)"));
    assert!(projects.contains("openProjectDialog(null)"));
    assert!(projects.contains("#/projects?edit="));
    assert!(projects.contains("#/projects?new=1"));
    assert!(projects.contains("clearEditHash();"));
    let project = extract_braced_function(&script, "async function openProjectDialog");
    assert!(project.contains("openEditDialog({"));
    assert!(project.contains("api(\"POST\", \"/api/projects\""));
    assert!(project.contains("api(\"PUT\", `/api/projects/"));
    assert!(project.contains("{ id: \"general\", label: t(\"general\")"));
    assert!(project.contains("{ id: \"source\", label: t(\"source_2\")"));
    assert!(project.contains("{ id: \"assignment\", label: t(\"assignment\")"));
    assert!(project.contains("onTab: editing ? (tab) => rememberEditHash("));
    assert!(project.contains("saved.sync"));
    assert!(project.contains("markSaved();"));
    assert!(project.contains("close();"));
    assert!(project.contains("await route();"));
    let detail = extract_braced_function(&script, "async function pageProject(id)");
    assert!(detail.contains("#/projects?edit="));
    assert!(detail.contains("delete-project"));
    assert!(!detail.contains("id=\"project-form\""));
    let inventories = extract_braced_function(&script, "async function pageInventories");
    assert!(inventories.contains("openInventoryDialog(item)"));
    assert!(inventories.contains("openInventoryDialog(null)"));
    assert!(inventories.contains("#/inventories?edit="));
    assert!(inventories.contains("#/inventories?new=1"));
    let inventory = extract_braced_function(&script, "async function openInventoryDialog");
    assert!(inventory.contains("openEditDialog({"));
    assert!(inventory.contains("api(\"POST\", \"/api/inventories\""));
    assert!(inventory.contains("api(\"PUT\", `/api/inventories/"));
    assert!(inventory.contains("bindSourcePicks(projects, credentials)"));
    assert!(inventory.contains("markSaved();"));
    assert!(inventory.contains("close();"));
    assert!(inventory.contains("await route();"));
    assert!(!inventory.contains("onTab:"));
    let inventory_detail = extract_braced_function(&script, "async function pageInventory(id)");
    assert!(inventory_detail.contains("#/inventories?edit="));
    assert!(!inventory_detail.contains("id=\"inventory-name-form\""));
    assert!(!script.contains("id=\"project-form\""));
    assert!(!script.contains("id=\"inventory-create\""));
    assert!(!script.contains("id=\"inventory-name-form\""));
    let close = extract_braced_function(&script, "function closeLive");
    assert!(close.contains("[data-edit-overlay]"));
}

#[test]
fn hooks_environments_and_team_members_use_the_edit_overlay() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let hooks = extract_braced_function(&script, "async function pageHooks");
    assert!(hooks.contains("openHookDialog(item)"));
    assert!(hooks.contains("openHookDialog(null)"));
    assert!(hooks.contains("#/hooks?edit="));
    assert!(hooks.contains("#/hooks?new=1"));
    assert!(hooks.contains("clearEditHash();"));
    let hook = extract_braced_function(&script, "async function openHookDialog");
    assert!(hook.contains("openEditDialog({"));
    assert!(hook.contains("api(\"POST\", \"/api/hooks\""));
    assert!(hook.contains("api(\"PUT\", `/api/hooks/"));
    assert!(hook.contains("jsonObject(form.extra_vars.value"));
    assert!(hook.contains("markSaved();"));
    assert!(hook.contains("close();"));
    assert!(hook.contains("await route();"));
    assert!(!hook.contains("handle(error)"));
    let runtime = extract_braced_function(&script, "async function pageRuntime");
    assert!(runtime.contains("openEnvironmentDialog(item)"));
    assert!(runtime.contains("openEnvironmentDialog(null)"));
    assert!(runtime.contains("#/runtime?edit="));
    assert!(runtime.contains("#/runtime?new=1"));
    assert!(runtime.contains("clearEditHash();"));
    let environment = extract_braced_function(&script, "function openEnvironmentDialog");
    assert!(environment.contains("openEditDialog({"));
    assert!(environment.contains("api(\"POST\", \"/api/environments\""));
    assert!(environment.contains("api(\"PUT\", `/api/environments/"));
    assert!(environment.contains("environmentImagePlaceholder(dialog.form.tool.value)"));
    assert!(environment.contains("markSaved();"));
    assert!(environment.contains("close();"));
    assert!(environment.contains("await route();"));
    assert!(!environment.contains("handle(error)"));
    let detail = extract_braced_function(&script, "async function pageEnvironment");
    assert!(detail.contains("#/runtime?edit="));
    assert!(detail.contains("delete-env"));
    assert!(!detail.contains("environmentForm"));
    let team = extract_braced_function(&script, "async function pageTeam");
    assert!(team.contains("openMemberDialog()"));
    assert!(team.contains("#/team?new=member"));
    let member = extract_braced_function(&script, "function openMemberDialog");
    assert!(member.contains("openEditDialog({"));
    assert!(member.contains("api(\"POST\", `/api/teams/${encodeURIComponent(teamId)}/members`"));
    assert!(member.contains("markSaved();"));
    assert!(member.contains("close();"));
    assert!(member.contains("await route();"));
    assert!(!member.contains("handle(error)"));
    let close = extract_braced_function(&script, "function closeLive");
    assert!(close.contains("[data-edit-overlay]"));
}

#[test]
fn template_create_and_edit_use_the_tabbed_overlay() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let templates = extract_braced_function(&script, "async function pageTemplates");
    assert!(templates.contains("openTemplateDialog(item, request.tab)"));
    assert!(templates.contains("openTemplateDialog(null, request.tab)"));
    assert!(templates.contains("#/templates?edit="));
    assert!(templates.contains("#/templates?new=1"));
    assert!(templates.contains("clearEditHash();"));
    assert!(script.contains("addLink(\"#/templates?new=1\", \"template_2\")"));
    assert!(!script.contains("addLink(\"#/templates/new\""));
    assert!(!script.contains("href=\"#/templates/${"));
    let dialog = extract_braced_function(&script, "async function openTemplateDialog");
    assert!(dialog.contains("openEditDialog({"));
    assert!(dialog.contains("wide: true"));
    assert!(dialog.contains("{ id: \"general\", label: t(\"general\")"));
    assert!(dialog.contains("{ id: \"target\", label: t(\"target\")"));
    assert!(dialog.contains("{ id: \"options\", label: t(\"options\")"));
    assert!(dialog.contains("{ id: \"variables\", label: t(\"variables\")"));
    assert!(dialog.contains("vaultPicker.ids()"));
    assert!(dialog
        .contains("bindVaultPicker(form, credentials, selectedCredentialIds(item), readOnly)"));
    assert!(!dialog.contains("input[name=vault_ids]"));
    assert!(!script.contains("function bindVaultChoices"));
    assert!(!script.contains("openVaultPicker("));
    assert!(dialog.contains("showToolSections"));
    assert!(dialog.contains("#ansible-options"));
    assert!(dialog.contains("#tofu-options"));
    assert!(dialog.contains("#shell-options"));
    assert!(dialog.contains("environmentOptions(tool, selected)"));
    assert!(dialog.contains("api(\"POST\", \"/api/templates\""));
    assert!(dialog.contains("api(\"PUT\", `/api/templates/"));
    assert!(dialog.contains("markSaved();"));
    assert!(dialog.contains("close();"));
    assert!(dialog.contains("await route();"));
    assert!(dialog.contains("onTab: (tab) => rememberEditHash("));
    assert!(!dialog.contains("handle(error)"));
    let submit = dialog
        .find("onSubmit: readOnly ? undefined : async")
        .expect("template submit");
    let tab = dialog[submit..].find("onTab:").expect("template tab") + submit;
    assert!(dialog[submit..tab].contains("throw error"));
    assert!(!dialog[submit..tab].contains("handle(error)"));
    let remove = extract_braced_function(&script, "async function deleteTemplate");
    assert!(remove.contains("api(\"DELETE\", `/api/templates/"));
    assert!(remove.contains("markSaved();"));
    assert!(remove.contains("dialog.close();"));
    assert!(remove.contains("await route();"));
    let fields = extract_braced_function(&script, "function templateCredentialFields");
    assert!(!fields.contains("inlineVault"));
    assert!(!fields.contains("name=\"vault_ids\""));
    assert!(fields.contains("id=\"vault-pills\""));
    assert!(fields.contains("id=\"vault-search\""));
    assert!(fields.contains("role=\"combobox\""));
    let bind = extract_braced_function(&script, "function bindVaultPicker");
    assert!(bind.contains("data-vault-pick"));
    assert!(bind.contains("event.key === \"Enter\""));
    assert!(!bind.contains("Backspace"));
    let helper = extract_function_with_destructured_params(&script, "function openEditDialog");
    assert!(helper.contains("panel.hidden = panel.dataset.dialogPanel !== current;"));
    assert!(!helper.contains("panel.innerHTML ="));
}

#[test]
fn template_vault_pills_add_by_search_remove_and_save() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let functions = format!(
        "{}\n{}\n{}",
        extract_braced_function(&script, "function vaultSuggestions"),
        extract_braced_function(&script, "function renderVaultPills"),
        extract_braced_function(&script, "function bindVaultPicker")
    );
    let probe = format!(
        r##"{functions}
function esc(value) {{ return String(value); }}
function h(key, vars) {{ return vars ? `${{key}} ${{vars.name}}` : key; }}
function element() {{
  return {{
    innerHTML: "",
    hidden: false,
    value: "",
    handlers: {{}},
    attributes: {{}},
    addEventListener(type, handler) {{ this.handlers[type] = handler; }},
    setAttribute(name, value) {{ this.attributes[name] = value; }},
    focus() {{}},
    fire(type, event) {{ if (this.handlers[type]) this.handlers[type](event || {{}}); }},
  }};
}}
const box = element();
const search = element();
const options = element();
const form = {{ querySelector(selector) {{
  if (selector === "#vault-pills") return box;
  if (selector === "#vault-search") return search;
  if (selector === "#vault-options") return options;
  return null;
}} }};
const credentials = Array.from({{ length: 30 }}, (_item, index) => ({{
  id: `vault-${{index}}`,
  kind: "vault",
  name: index === 3 ? "Alpha vault" : index === 17 ? "Beta vault" : `Vault ${{index}}`,
}}));
const picker = bindVaultPicker(form, credentials, [], false);
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
check(picker.ids().length === 0, "pills did not start empty");
search.value = "alpha";
search.fire("input");
search.fire("keydown", {{ key: "Enter", preventDefault() {{}} }});
check(picker.ids().join(",") === "vault-3", "search did not add the highlighted vault");
search.value = "beta";
search.fire("input");
search.fire("keydown", {{ key: "Enter", preventDefault() {{}} }});
check(picker.ids().join(",") === "vault-3,vault-17", "second search did not append");
const target = {{
  dataset: {{ vaultId: "vault-3" }},
  closest(selector) {{
    if (selector === ".chip-remove") return target;
    if (selector === ".chip") return {{ dataset: {{ vaultId: "vault-3" }} }};
    return null;
  }},
}};
box.fire("click", {{ target }});
check(picker.ids().join(",") === "vault-17", "remove did not drop the first pill");
search.value = "";
const before = picker.ids().join(",");
search.fire("keydown", {{ key: "Backspace" }});
check(picker.ids().join(",") === before, "Backspace removed a pill");
const body = {{ credential_ids: picker.ids() }};
check(body.credential_ids.join(",") === "vault-17", "saved credential_ids changed");
const readonlyBox = element();
const readonly = bindVaultPicker({{ querySelector(selector) {{ return selector === "#vault-pills" ? readonlyBox : null; }} }}, credentials, ["vault-3"], true);
check(readonly.ids().join(",") === "vault-3", "read-only picker lost the selected vault");
check(readonlyBox.innerHTML.includes("Alpha vault"), "read-only pills are missing");
check(!readonlyBox.innerHTML.includes("chip-remove"), "read-only pills have a remove button");
"##
    );
    run_node_probe("template-vault-pills", &probe);
}

#[test]
fn viewers_open_templates_in_a_read_only_overlay() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let templates = extract_braced_function(&script, "async function pageTemplates");
    assert!(templates.contains("await openTemplateDialog(item, request.tab);"));
    assert!(
        !templates.contains("if (can(\"operator\")) await openTemplateDialog(item, request.tab);")
    );
    assert!(templates.contains("<a href=\"#/templates?edit=${esc(item.id)}\">"));
    let dialog = extract_braced_function(&script, "async function openTemplateDialog");
    assert!(dialog.contains("const readOnly = !can(\"operator\");"));
    assert!(dialog.contains("title: readOnly ? t(\"read_only_template\", { name: item.name })"));
    assert!(dialog.contains("hint: readOnly ? \"read_only_template_hint\" : undefined"));
    assert!(dialog.contains("form: true"));
    assert!(dialog.contains("submit: readOnly ? undefined : editing ? t(\"save\") : t(\"create\")"));
    assert!(dialog.contains("onSubmit: readOnly ? undefined : async"));
    assert!(dialog.contains("!readOnly && editing"));
    assert!(dialog.contains("if (readOnly) {"));
    assert!(dialog.contains("control.disabled = true;"));
    assert!(dialog.contains("button.hasAttribute(\"data-dialog-cancel\")"));
    assert!(dialog.contains("button.classList.contains(\"dialog-close\")"));
    let helper = extract_function_with_destructured_params(&script, "function openEditDialog");
    assert!(helper.contains("form = Boolean(onSubmit)"));
    assert!(helper.contains("    form,\n    actions,"));
    let probe = format!(
        r#"{helper}
let editSerial = 0;
let leaveGuard = null;
function clearEditHash() {{}}
function h(value) {{ return value; }}
let captured = null;
function openDialog(options) {{
  captured = options;
  return {{
    overlay: {{ setAttribute() {{}}, querySelectorAll() {{ return []; }} }},
    form: {{ addEventListener() {{}} }},
    dirty() {{ return false; }},
  }};
}}
openEditDialog({{ title: "read only", body: "" }});
if (captured.form !== false) {{ console.error("missing submit dialog defaulted to a form"); process.exit(1); }}
openEditDialog({{ title: "read only", body: "", form: true }});
if (captured.form !== true) {{ console.error("explicit form was not forwarded"); process.exit(1); }}
openEditDialog({{ title: "edit", body: "", onSubmit: async () => {{}} }});
if (captured.form !== true) {{ console.error("submit dialog did not default to a form"); process.exit(1); }}
"#
    );
    run_node_probe("edit-dialog-form", &probe);
}

#[test]
fn tabbed_dialogs_keep_the_tab_row_in_the_chrome() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    assert!(script.contains("footer = \"\", tabs = \"\""));
    assert!(script.contains("overlay.className = tabs ? \"overlay tabbed\" : \"overlay\";"));
    assert!(script.contains("${tabs ? \" tabbed\" : \"\"}"));
    let head_at = script
        .find("</div>\n      ${tabs}")
        .expect("tabs after the head");
    let form_at = script
        .find("<${shellTag} class=\"dialog-form\">")
        .expect("form");
    assert!(
        head_at < form_at,
        "tabs must be rendered before the scrolling body"
    );
    let helper = extract_function_with_destructured_params(&script, "function openEditDialog");
    assert!(helper.contains("tabs: tabBar,"));
    assert!(
        !helper.contains("${tabBar}${panelMarkup}"),
        "tab bar still sits in the scrolling body"
    );
    assert!(css.contains(".overlay.tabbed { align-items: flex-start; }"));
    let narrow = css
        .split("@media (max-width: 600px)")
        .nth(1)
        .and_then(|rest| rest.get(..rest.len().min(400)))
        .expect("narrow overlay rule");
    assert!(
        narrow.contains(".overlay.tabbed"),
        "tabbed overlays must stay full screen"
    );
    assert!(
        narrow.contains(".overlay-card.dialog.tabbed"),
        "the narrow full-screen rule must win over the tabbed max height"
    );
    let tabs_rule = css
        .split(".dialog-tabs {")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect("dialog-tabs rule");
    assert!(
        tabs_rule.contains("flex: none;"),
        "the tab row must not scroll with the body"
    );
    let tab_rule = css
        .split(".dialog-tab {")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect("dialog-tab rule");
    assert!(
        tab_rule.contains("font-weight: 600;"),
        "all tabs keep one width"
    );
    let selected = css
        .split(".dialog-tab[aria-selected=\"true\"] {")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect("selected tab rule");
    assert!(
        !selected.contains("font-weight"),
        "selection must not change the label width"
    );
}

#[test]
fn template_overlay_addresses_round_trip_and_old_pages_redirect() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    for redirect in [
        "location.replace(\"#/templates?new=1\")",
        "location.replace(`#/templates?edit=${encodeURIComponent(parts[1])}`)",
    ] {
        assert!(script.contains(redirect), "missing redirect {redirect}");
    }
    let functions = format!(
        "{}\n{}\n{}\n{}",
        extract_braced_function(&script, "function editQuery"),
        extract_braced_function(&script, "function editAddress"),
        extract_braced_function(&script, "function rememberEditHash"),
        extract_braced_function(&script, "function clearEditHash")
    );
    let probe = format!(
        r##"{functions}
let location = {{ hash: "#/templates" }};
let shownHash = null;
let pushed = 0;
let replaced = 0;
const history = {{
  pushState(_state, _title, url) {{ pushed += 1; location.hash = url; }},
  replaceState(_state, _title, url) {{ replaced += 1; location.hash = url; }},
}};
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
const edit = editQuery("#/templates?edit=t1&tab=target");
check(edit.edit === "t1" && edit.tab === "target" && edit.create === null, "template edit and tab were not read");
const create = editQuery("#/templates?new=1&tab=options");
check(create.create === "1" && create.edit === "" && create.tab === "options", "template new and tab were not read");
check(editAddress({{ edit: "t1", tab: "target" }}, "#/templates") === "#/templates?edit=t1&tab=target", "template edit address was not built");
check(editAddress({{ create: "1", tab: "variables" }}, "#/templates") === "#/templates?new=1&tab=variables", "template new address was not built");
check(editAddress({{ edit: "t1" }}, "#/templates?tab=target&page=2") === "#/templates?page=2&edit=t1", "old tab was not cleared");
rememberEditHash({{ create: "1", tab: "general" }});
check(pushed === 1 && location.hash === "#/templates?new=1&tab=general" && shownHash === location.hash, "template open did not push the address");
rememberEditHash({{ edit: "t1", tab: "target" }}, false);
check(pushed === 1 && replaced === 1 && location.hash === "#/templates?edit=t1&tab=target", "template tab did not replace the address");
clearEditHash();
check(replaced === 2 && location.hash === "#/templates" && shownHash === "#/templates", "template close did not clear the address");
"##
    );
    run_node_probe("template-address", &probe);
}

#[test]
fn workflow_create_uses_the_edit_overlay() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let workflows = extract_braced_function(&script, "async function pageWorkflows");
    assert!(workflows.contains("#/workflows?new=1"));
    assert!(workflows.contains("const request = editQuery();"));
    assert!(workflows.contains("if (request.create !== null) {"));
    assert!(workflows.contains("if (can(\"operator\")) openWorkflowDialog();"));
    assert!(script.contains("addLink(\"#/workflows?new=1\", \"workflow_2\")"));
    assert!(!script.contains("addLink(\"#/workflows/new\""));
    assert!(!script.contains("async function pageWorkflowNew"));
    assert!(!script.contains("id=\"workflow-new\""));
    let dialog = extract_braced_function(&script, "function openWorkflowDialog");
    assert!(dialog.contains("openEditDialog({"));
    assert!(dialog.contains("title: t(\"new_workflow\")"));
    assert!(dialog.contains("class=\"field required\""));
    assert!(dialog.contains("name=\"name\" required"));
    assert!(dialog.contains("maxlength=\"1000\""));
    assert!(dialog.contains("submit: t(\"create_and_open\")"));
    assert!(dialog.contains("api(\"POST\", \"/api/workflows\""));
    assert!(dialog.contains("graph: { nodes: [], edges: [] }"));
    assert!(dialog.contains("markSaved();"));
    assert!(dialog.contains("close();"));
    assert!(dialog.contains("go(`#/workflows/${created.id}`);"));
    assert!(!dialog.contains("handle(error)"));
}

#[test]
fn workflow_create_overlay_submits_and_keeps_errors() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let dialog = extract_braced_function(&script, "function openWorkflowDialog");
    let probe = format!(
        r##"{dialog}
function t(key) {{ return key; }}
function h(key) {{ return key; }}
let captured = null;
let posted = null;
let saved = 0;
let closed = 0;
let navigated = "";
let fail = false;
function markSaved() {{ saved += 1; }}
function go(hash) {{ navigated = hash; }}
function openEditDialog(options) {{ captured = options; }}
function api(method, path, body) {{
  posted = {{ method, path, body }};
  if (fail) return Promise.reject(new Error("workflow_error"));
  return Promise.resolve({{ id: "w1" }});
}}
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
function submit() {{
  return captured.onSubmit({{
    form: {{ name: {{ value: "Heal" }}, description: {{ value: "restart" }} }},
    close: () => {{ closed += 1; }},
  }});
}}
openWorkflowDialog();
check(captured.title === "new_workflow", "workflow overlay title is missing");
submit().then(async () => {{
  check(posted.method === "POST" && posted.path === "/api/workflows", "workflow create did not post");
  check(posted.body.name === "Heal" && posted.body.description === "restart", "workflow body is wrong");
  check(posted.body.graph.nodes.length === 0 && posted.body.graph.edges.length === 0, "workflow graph is not empty");
  check(saved === 1 && closed === 1 && navigated === "#/workflows/w1", "workflow create did not open the editor");
  fail = true;
  try {{
    await submit();
    console.error("workflow error was swallowed");
    process.exit(1);
  }} catch (error) {{
    check(error.message === "workflow_error", "workflow error changed");
  }}
  check(closed === 1 && navigated === "#/workflows/w1", "workflow error closed the overlay or navigated");
}}).catch((error) => {{ console.error(error); process.exit(1); }});
"##
    );
    run_node_probe("workflow-create", &probe);
}

#[test]
fn workflow_overlay_addresses_round_trip_and_old_pages_redirect() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    assert!(script.contains("location.replace(\"#/workflows?new=1\")"));
    assert!(!script.contains("await pageWorkflowNew();"));
    let functions = format!(
        "{}\n{}\n{}\n{}",
        extract_braced_function(&script, "function editQuery"),
        extract_braced_function(&script, "function editAddress"),
        extract_braced_function(&script, "function rememberEditHash"),
        extract_braced_function(&script, "function clearEditHash")
    );
    let probe = format!(
        r##"{functions}
let location = {{ hash: "#/workflows" }};
let shownHash = null;
let pushed = 0;
let replaced = 0;
const history = {{
  pushState(_state, _title, url) {{ pushed += 1; location.hash = url; }},
  replaceState(_state, _title, url) {{ replaced += 1; location.hash = url; }},
}};
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
const create = editQuery("#/workflows?new=1");
check(create.create === "1" && create.edit === "" && create.tab === "", "workflow new was not read");
check(editQuery("#/workflows").create === null, "workflow list was read as new");
check(editAddress({{ create: "1" }}, "#/workflows") === "#/workflows?new=1", "workflow new address was not built");
check(editAddress({{ create: "1" }}, "#/workflows?page=2") === "#/workflows?page=2&new=1", "workflow new address dropped the query");
rememberEditHash({{ create: "1" }});
check(pushed === 1 && location.hash === "#/workflows?new=1" && shownHash === location.hash, "workflow open did not push the address");
clearEditHash();
check(replaced === 1 && location.hash === "#/workflows" && shownHash === "#/workflows", "workflow close did not clear the address");
"##
    );
    run_node_probe("workflow-address", &probe);
}

#[test]
fn template_tool_switch_changes_options_and_environments() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let dialog = extract_braced_function(&script, "async function openTemplateDialog");
    assert!(dialog.contains("form.tool.addEventListener(\"change\""));
    assert!(dialog.contains("showToolSections();"));
    assert!(dialog.contains("loadPlaybooks();"));
    assert!(dialog.contains(
        "dialog.overlay.querySelector(\"#ansible-options\").hidden = tool !== \"ansible\""
    ));
    assert!(dialog.contains(
        "dialog.overlay.querySelector(\"#tofu-options\").hidden = tool !== \"opentofu\""
    ));
    assert!(dialog
        .contains("dialog.overlay.querySelector(\"#shell-options\").hidden = tool !== \"shell\""));
    assert!(dialog.contains(
        "dialog.overlay.querySelector(\"#inventory-field\").hidden = tool !== \"ansible\""
    ));
    assert!(dialog.contains("form.environment_id.innerHTML = environmentOptions(tool, selected)"));
    assert!(dialog.contains("default_environment_${tool}"));
    assert!(dialog.contains("noEnvironmentAvailable"));
    assert!(dialog.contains("runner.local_tools"));
    let environments_for = extract_braced_function(&script, "const environmentsFor = (tool) =>");
    let probe = format!(
        r#"{environments_for}
const environments = [
  {{ id: "a1", tool: "ansible" }},
  {{ id: "t1", tool: "opentofu" }},
  {{ id: "s1", tool: "shell" }},
  {{ id: "any1", tool: "any" }},
  {{ id: "legacy" }}
];
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
check(environmentsFor("ansible").map((row) => row.id).join(",") === "a1,any1,legacy", "ansible environments do not match");
check(environmentsFor("opentofu").map((row) => row.id).join(",") === "t1,any1,legacy", "opentofu environments do not match");
check(environmentsFor("shell").map((row) => row.id).join(",") === "s1,any1,legacy", "shell environments do not match");
"#
    );
    run_node_probe("template-environments", &probe);
}

#[test]
fn template_validation_errors_open_the_matching_tab() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let helper = extract_function_with_destructured_params(&script, "function openEditDialog");
    assert!(helper.contains("mark(panel.dataset.dialogPanel);"));
    assert!(helper.contains("choose(panel.dataset.dialogPanel, false);"));
    let mapping = extract_braced_function(&script, "const ERROR_FIELD =");
    let probe = format!(
        r#"{mapping}
function check(key, want) {{
  if (ERROR_FIELD[key] !== want) {{
    console.error(key, ERROR_FIELD[key], want);
    process.exit(1);
  }}
}}
check("tool_is_ansible_opentofu_or_shell", "tool");
check("project_is_missing", "project_id");
check("inventory_is_missing", "inventory_id");
check("execution_environment_is_missing", "environment_id");
check("the_execution_environment_is_for_another_tool", "environment_id");
check("action_is_plan_or_apply", "action");
check("ansible_needs_a_playbook_as_the_entry", "entrypoint");
check("opentofu_needs_a_module_directory", "entrypoint");
check("shell_needs_a_path_or_spec_script", "entrypoint");
check("the_entry_path_must_be_relative_and_must", "entrypoint");
check("extra_variables_must_be_a_json_object", "extra_vars");
check("template.vars_not_objects", "extra_vars");
"#
    );
    run_node_probe("template-error-tabs", &probe);
}

#[test]
fn template_form_saves_the_visible_playbook_path() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    assert!(script.contains("fields_marked_with_a_star_are_required"));
    assert!(script.contains("class=\"field required\""));
    assert!(script.contains("classList.toggle(\"required\", tool !== \"shell\")"));
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    assert!(css.contains(".field.required"));
    assert_chosen_entrypoint_cases(&script);
    assert_playbook_choice_cases(&script);
}

#[test]
fn inventory_tables_drop_variables_and_details_keep_them() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let host_table = extract_braced_function(&script, "function inventoryHostTable");
    let group_table = extract_braced_function(&script, "function inventoryGroupTable");
    for table in [&host_table, &group_table] {
        assert!(
            !table.contains("h(\"variables\")"),
            "inventory table still renders a variables header"
        );
        assert!(
            !table.contains("formatVars"),
            "inventory table still renders variables"
        );
    }
    assert!(
        !script.contains("function formatVars"),
        "formatVars should be removed with the column"
    );
    assert!(host_table.contains("head(\"address\", \"address\")"));
    assert!(host_table.contains("head(\"groups\", \"groups\")"));
    assert!(group_table.contains("h(\"hosts\")"));
    assert!(group_table.contains("h(\"groups\")"));

    let host_detail = extract_braced_function(&script, "async function pageInventoryHost");
    let group_detail = extract_braced_function(&script, "async function pageInventoryGroup");
    assert!(host_detail.contains("variablesViewer(host.vars || [])"));
    assert!(group_detail.contains("variablesViewer(vars)"));
    assert!(host_detail.contains("bindVariablesViews();"));
    assert!(group_detail.contains("bindVariablesViews();"));
    assert!(script.contains(
        "variablesViewer(Object.entries(result.sample_host.vars || {}).map(([name, value]) => ({ name, value, origin: \"host\" })))"
    ));
    assert!(script.contains("bindVariablesViews(resultBox);"));

    let functions = [
        "function looksSecretVar",
        "function variableText",
        "function variableOrigin",
        "function yamlPlain",
        "function variableScalar",
        "function variableKey",
        "function effectiveVariablesYaml",
        "function jsonVariableMarkup",
        "function effectiveVariablesJson",
        "function variablesViewer",
    ]
    .iter()
    .map(|needle| extract_braced_function(&script, needle))
    .collect::<Vec<_>>()
    .join("\n");
    let probe = format!(
        r#"{functions}
let variablesViewSerial = 0;
function t(key) {{ return key; }}
function esc(value) {{
  return String(value ?? "").replace(/[&<>"']/g, (ch) => ({{
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  }}[ch]));
}}
function h(key) {{ return esc(t(key)); }}
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
const vars = [
  {{ name: "ansible_host", value: "10.0.0.1", origin: "host" }},
  {{ name: "message", value: "hello world", origin: "web" }},
  {{ name: "ansible_become_password", value: "s3cret", origin: "host" }},
  {{ name: "evil", value: "<script>x &lt; y</script>", origin: "group" }},
];
const yaml = effectiveVariablesYaml(vars);
for (const want of [
  '<span class="vars-key">ansible_host</span>: <span class="vars-value">10.0.0.1</span>',
  '<span class="vars-value">&quot;hello world&quot;</span>',
  '<span class="vars-value">&quot;••••••&quot;</span>',
  '<span class="vars-comment"># host</span>',
  '<span class="vars-comment"># web</span>',
]) {{
  check(yaml.includes(want), "yaml missing " + want + "\n" + yaml);
}}
check(yaml.includes("&lt;script&gt;x &amp;lt; y&lt;/script&gt;"), "yaml value was not escaped\n" + yaml);
check(!yaml.includes("<script>"), "yaml kept raw markup\n" + yaml);

const json = effectiveVariablesJson(vars);
for (const want of [
  '<span class="vars-key">&quot;name&quot;</span>',
  '<span class="vars-key">&quot;value&quot;</span>',
  '<span class="vars-key">&quot;origin&quot;</span>',
  '<span class="vars-value">&quot;ansible_host&quot;</span>',
  '<span class="vars-value">&quot;host&quot;</span>',
  '<span class="vars-value">&quot;••••••&quot;</span>',
  '&lt;script&gt;x &amp;lt; y&lt;/script&gt;',
]) {{
  check(json.includes(want), "json missing " + want + "\n" + json);
}}
check(!json.includes("<script>"), "json kept raw markup\n" + json);
check(json.includes("\n  {{\n"), "json is not pretty printed\n" + json);

const view = variablesViewer(vars);
check(view.includes('<div class="spread tab-bar">'), "missing page tab bar\n" + view);
check(view.includes('<div class="tabs" role="tablist"'), "missing page tabs\n" + view);
check(view.includes('data-vars-tab="yaml" aria-selected="true"'), "yaml is not selected by default\n" + view);
check(view.includes('data-vars-tab="json" aria-selected="false"'), "json tab is not unselected\n" + view);
check(/<div id="vars-\d+-yaml" role="tabpanel" data-vars-panel="yaml"><pre class="vars">/.test(view), "yaml panel is missing\n" + view);
check(/<div id="vars-\d+-json" role="tabpanel" data-vars-panel="json" hidden>/.test(view), "json panel is missing\n" + view);

const weird = effectiveVariablesYaml([
  {{ name: "a", value: "1", origin: "team\n  injected: true" }},
  {{ name: "b", value: "2", origin: "team\u0085  injected: true" }},
]);
check(!weird.includes("\n  injected"), "origin line break added a mapping line\n" + weird);
check(/# team injected: true/.test(weird.replace(/\s+/g, " ")), "origin was not flattened\n" + weird);
check(!/[\u0085\u2028\u2029]/.test(weird), "origin kept a Unicode line break\n" + weird);

const empty = variablesViewer([]);
check((empty.match(/no_variables/g) || []).length === 2, "empty state is not in both formats\n" + empty);
check(!empty.includes('<pre class="vars">'), "empty state rendered a code block\n" + empty);
"#
    );
    run_node_probe("inventory-vars-yaml-json", &probe);
}

#[test]
fn route_leave_guard_only_blocks_changed_forms() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let route = extract_braced_function(&script, "async function route");
    let probe = format!(
        r##"let leaveGuard = null;
let shownHash = null;
let navToken = null;
let me = null;
let location = {{ hash: "#/" }};
let replaced = 0;
const history = {{ replaceState(_state, _title, url) {{ replaced += 1; location.hash = url; }} }};
let prompts = 0;
let promptedKey = "";
let confirmAnswer = true;
function confirm() {{ prompts += 1; return confirmAnswer; }}
function t(key) {{ promptedKey = key; return key; }}
function closeLive() {{}}
let rendered = 0;
function renderLogin() {{ rendered += 1; }}
{route}
function reset(hash) {{
  location.hash = hash;
  shownHash = hash;
  replaced = 0;
  prompts = 0;
  promptedKey = "";
  rendered = 0;
  confirmAnswer = true;
}}
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
(async () => {{
  reset("#/projects/p1");
  leaveGuard = () => false;
  location.hash = "#/jobs";
  await route();
  check(rendered === 1 && shownHash === "#/jobs" && replaced === 0 && prompts === 0, "unchanged project was blocked");

  reset("#/inventories/i1");
  leaveGuard = () => false;
  location.hash = "#/jobs";
  await route();
  check(rendered === 1 && shownHash === "#/jobs" && replaced === 0 && prompts === 0, "unchanged inventory was blocked");

  reset("#/projects/p1");
  leaveGuard = () => true;
  confirmAnswer = false;
  location.hash = "#/jobs";
  await route();
  check(prompts === 1 && promptedKey === "discard_unsaved_changes" && replaced === 1 && location.hash === "#/projects/p1" && shownHash === "#/projects/p1" && rendered === 0, "cancel left the form");

  reset("#/inventories/i1");
  leaveGuard = () => true;
  location.hash = "#/jobs";
  await route();
  check(prompts === 1 && replaced === 0 && shownHash === "#/jobs" && rendered === 1, "confirm did not navigate");

  reset("#/projects/p1");
  leaveGuard = () => false;
  location.hash = "#/projects";
  await route();
  check(rendered === 1 && shownHash === "#/projects" && replaced === 0, "browser back was blocked");
}})();
"##
    );
    run_node_probe("route-leave-guard", &probe);
}

#[tokio::test]
async fn runner_preflight_is_admin_only_and_leaks_no_secrets() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-preflight-smoke-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, denied, _) = call(&app, "GET", "/api/runner/preflight", None, None, None).await;
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
    let cookie = cookie.expect("admin cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, report, _) = call(
        &app,
        "GET",
        "/api/runner/preflight",
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["mode"], "local");
    assert!(report["tools"]
        .as_array()
        .is_some_and(|tools| tools.len() == 5));
    assert!(report["runtime"].is_object());
    assert!(report["missing"].is_array());
    let body = report.to_string();
    assert!(!body.contains("KOREOGRAF_"), "{body}");
    assert!(!body.contains("\"PATH\""), "{body}");
    if let Ok(path) = std::env::var("PATH") {
        assert!(!body.contains(&path), "{body}");
    }

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
    assert!(overview["preflight"].is_object(), "{overview}");
    assert_eq!(overview["preflight"]["ok"], report["ok"]);

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "preflight-viewer",
            "display_name": "Preflight Viewer",
            "password": "viewer-pass",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let (status, member, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(&team),
        Some(json!({"username": "preflight-viewer", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{member}");
    let (status, viewer_me, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "preflight-viewer", "password": "viewer-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer_me}");
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    let (status, forbidden, _) = call(
        &app,
        "GET",
        "/api/runner/preflight",
        Some(&viewer_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{forbidden}");
    assert_eq!(forbidden["error"], "you_are_not_allowed_to_do_this");
    let (status, viewer_overview, _) = call(
        &app,
        "GET",
        "/api/overview",
        Some(&viewer_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer_overview}");
    assert!(viewer_overview["preflight"].is_null(), "{viewer_overview}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_preflight_notice_uses_the_missing_tool_and_runtime() {
    let script = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("web/app.js"))
        .expect("app.js");
    assert!(script.contains("id=\"preflight-check\""));
    assert!(script.contains("api(\"GET\", \"/api/runner/preflight\")"));
    let function = extract_braced_function(&script, "function preflightNotices(report)");
    let probe = format!(
        r#"{function}
const cases = [
  [null, []],
  [{{ok: true, missing: []}}, []],
  [{{ok: false, missing: [{{kind: "tool", tool: "ansible-playbook", template_tool: "ansible", reason: "missing"}}]}}, ["preflight_tool_missing"]],
  [{{ok: false, missing: [{{kind: "runtime", tool: "docker/podman", template_tool: "", reason: "missing"}}, {{kind: "tool", tool: "ansible-playbook", template_tool: "ansible", reason: "missing"}}]}}, ["preflight_runtime_missing", "preflight_no_runtime_and_tool_missing"]],
  [{{ok: false, missing: [{{kind: "tool", tool: "tofu", template_tool: "opentofu", reason: "version", version: "1.5.0", minimum: "1.6"}}]}}, ["preflight_tool_too_old"]],
  [{{ok: false, missing: [{{kind: "tool", tool: "sh", template_tool: "shell", reason: "timeout"}}]}}, ["preflight_tool_unusable"]],
  [{{ok: false, missing: [{{kind: "project", tool: "git", template_tool: "", reason: "missing"}}]}}, ["preflight_project_missing"]],
  [{{ok: false, missing: [{{kind: "project", tool: "git", template_tool: "", reason: "failed"}}]}}, ["preflight_project_unusable"]],
  [{{ok: false, missing: [{{kind: "tool", tool: "ssh", template_tool: "ansible", reason: "missing"}}]}}, ["preflight_tool_missing"]]
];
for (const [report, want] of cases) {{
  const got = preflightNotices(report).map((item) => item.key);
  if (JSON.stringify(got) !== JSON.stringify(want)) {{
    console.error(JSON.stringify({{report, want, got}}));
    process.exit(1);
  }}
}}
"#
    );
    run_node_probe("preflight-notice", &probe);
}

#[tokio::test]
async fn dashboard_series_comes_from_stored_jobs_and_is_team_scoped() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-dashboard-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state.clone());

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

    let (status, other, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        None,
        Some(json!({"name": "Dashboard Other", "description": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_team = other["id"].as_str().unwrap().to_string();

    let (status, empty_team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        None,
        Some(json!({"name": "Dashboard Empty", "description": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{empty_team}");
    let empty_team = empty_team["id"].as_str().unwrap().to_string();

    let now = chrono::Utc::now();
    let stamp = |ago: chrono::Duration| (now - ago).format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let today = stamp(chrono::Duration::zero());
    let yesterday = stamp(chrono::Duration::days(1) + chrono::Duration::minutes(1));
    let older = stamp(chrono::Duration::days(3) + chrono::Duration::minutes(1));
    {
        let db = state.db().expect("database");
        let add = |id: &str,
                   team_id: &str,
                   status: &str,
                   started: Option<&str>,
                   finished: Option<&str>| {
            db.execute(
                "INSERT INTO jobs (
                    id, team_id, name, tool, status, extra_vars, launched_by, trigger,
                    started_at, finished_at, created_at
                 ) VALUES (?1, ?2, ?1, 'shell', ?3, '{}', 'admin', 'manual', ?4, ?5, ?6)",
                (id, team_id, status, started, finished, today.as_str()),
            )
            .unwrap();
        };
        add(
            "today-successful",
            &team,
            "successful",
            Some(&today),
            Some(&today),
        );
        add("today-failed", &team, "failed", Some(&today), Some(&today));
        add(
            "today-canceled",
            &team,
            "canceled",
            Some(&today),
            Some(&today),
        );
        add("today-running", &team, "running", Some(&today), None);
        add(
            "yesterday-successful",
            &team,
            "successful",
            Some(&yesterday),
            Some(&yesterday),
        );
        add(
            "older-successful",
            &team,
            "successful",
            Some(&older),
            Some(&older),
        );
        add(
            "other-team-successful",
            &other_team,
            "successful",
            Some(&today),
            Some(&today),
        );
        add("pending-without-start", &team, "pending", None, None);
    }

    let cache_root = dir.join("cache").join(&team);
    std::fs::create_dir_all(cache_root.join("collections/ansible_collections/acme/tools")).unwrap();
    std::fs::create_dir_all(cache_root.join("roles/common")).unwrap();
    std::fs::create_dir_all(cache_root.join("tofu")).unwrap();
    std::fs::write(cache_root.join("tofu/provider.bin"), [0_u8; 123]).unwrap();

    let total = |series: &Value, key: &str| {
        series
            .as_array()
            .unwrap()
            .iter()
            .map(|day| day[key].as_u64().unwrap())
            .sum::<u64>()
    };

    let (status, day, _) = call(
        &app,
        "GET",
        "/api/dashboard?days=1&tz=UTC",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{day}");
    assert_eq!(day["range_days"], 1);
    assert_eq!(day["series"].as_array().unwrap().len(), 1);
    assert_eq!(day["series"][0]["date"], day["to"].as_str().unwrap()[..10]);
    assert_eq!(day["series"][0]["successful"], 1);
    assert_eq!(day["series"][0]["failed"], 1);
    assert_eq!(day["series"][0]["canceled"], 1);
    assert_eq!(day["series"][0]["started"], 4);
    assert_eq!(day["cache"]["collections"], 1);
    assert_eq!(day["cache"]["roles"], 1);
    assert_eq!(day["cache"]["tofu_bytes"], 123);

    let (status, week, _) = call(
        &app,
        "GET",
        "/api/dashboard?days=7&tz=UTC",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{week}");
    assert_eq!(week["range_days"], 7);
    assert_eq!(week["series"].as_array().unwrap().len(), 7);
    assert_eq!(total(&week["series"], "successful"), 3);
    assert_eq!(total(&week["series"], "failed"), 1);
    assert_eq!(total(&week["series"], "canceled"), 1);
    assert_eq!(total(&week["series"], "started"), 6);

    let (status, capped, _) = call(
        &app,
        "GET",
        "/api/dashboard?days=30&tz=UTC",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{capped}");
    assert_eq!(capped["range_days"], 14);
    assert_eq!(capped["series"].as_array().unwrap().len(), 14);
    assert_eq!(total(&capped["series"], "successful"), 3);

    let (status, defaulted, _) = call(
        &app,
        "GET",
        "/api/dashboard?tz=UTC",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{defaulted}");
    assert_eq!(defaulted["range_days"], 1);
    assert_eq!(defaulted["series"].as_array().unwrap().len(), 1);

    let (status, isolated, _) = call(
        &app,
        "GET",
        "/api/dashboard?days=7&tz=UTC",
        Some(&cookie),
        Some(&other_team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{isolated}");
    assert_eq!(total(&isolated["series"], "successful"), 1);
    assert_eq!(total(&isolated["series"], "started"), 1);
    assert_eq!(isolated["cache"]["tofu_bytes"], 0);

    let (status, empty, _) = call(
        &app,
        "GET",
        "/api/dashboard?days=14&tz=UTC",
        Some(&cookie),
        Some(&empty_team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert_eq!(empty["range_days"], 14);
    assert_eq!(total(&empty["series"], "started"), 0);
    assert_eq!(total(&empty["series"], "successful"), 0);
    assert_eq!(empty["cache"]["collections"], 0);

    let (status, _viewer, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "dashboard-viewer",
            "display_name": "Dashboard Viewer",
            "password": "viewer-pass",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _member, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(&team),
        Some(json!({"username": "dashboard-viewer", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, viewer_me, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "dashboard-viewer", "password": "viewer-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer_me}");
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    let (status, viewer, _) = call(
        &app,
        "GET",
        "/api/dashboard?days=7&tz=UTC",
        Some(&viewer_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer}");
    assert_eq!(total(&viewer["series"], "successful"), 3);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dashboard_page_has_tabs_default_range_empty_state_and_cache_numbers() {
    let script = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("web/app.js"))
        .expect("app.js");
    let tabs = extract_braced_function(&script, "function dashboardTabs(tab)");
    let chart = extract_braced_function(
        &script,
        "function dashboardChart(series, values, heading, legend)",
    );
    let body = extract_braced_function(&script, "function dashboardBody(data, range)");
    let short = extract_braced_function(&script, "function shortDay(value)");
    let range = extract_braced_function(&script, "function dashboardRange()");
    let address = extract_braced_function(&script, "function dashboardAddress(tab, range)");
    let probe = format!(
        r##"{tabs}
{chart}
{body}
{short}
{range}
{address}
let lang = "en";
let location = {{ hash: "#/" }};
function esc(value) {{ return String(value ?? ""); }}
function t(key) {{ return key; }}
function h(key) {{ return esc(t(key)); }}
function pill(status) {{ return `<span class="pill ${{status}}">${{status}}</span>`; }}
function bytes(value) {{ return `${{Number(value) || 0}} B`; }}
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
check(dashboardRange() === "1", "default range is not today");
location.hash = "#/?tab=dashboard&days=7";
check(dashboardRange() === "7", "range did not come from the address");
check(dashboardAddress("dashboard", "1") === "#/?tab=dashboard", "dashboard tab is not kept");
check(dashboardAddress("overview", "7") === "#/", "overview address carries dashboard state");
const tabsView = dashboardTabs("overview");
check(tabsView.includes('role="tablist"'), "missing tablist");
check(tabsView.includes('id="tab-overview" aria-selected="true"'), "overview is not selected");
check(tabsView.includes('id="tab-dashboard" aria-selected="false"'), "dashboard is not unselected");
const empty = dashboardBody({{ series: [{{ date: "2026-11-05", started: 0, successful: 0, failed: 0, canceled: 0 }}], cache: {{ collections: 0, roles: 0, tofu_bytes: 0 }} }}, "1");
check(empty.includes("no_jobs_in_this_range"), "empty range has no empty state");
check(empty.includes('data-dashboard-range="1" aria-pressed="true"'), "today is not the default range");
check(empty.includes('data-dashboard-range="7" aria-pressed="false"'), "7 days is not offered");
check(empty.includes('data-dashboard-range="14" aria-pressed="false"'), "14 days is not offered");
const filled = dashboardBody({{ series: [{{ date: "2026-11-05", started: 3, successful: 1, failed: 1, canceled: 1 }}], cache: {{ collections: 2, roles: 4, tofu_bytes: 2048 }} }}, "7");
check(filled.includes("jobs_finished_per_day"), "missing finished chart");
check(filled.includes("jobs_started_per_day"), "missing started chart");
check(filled.includes('<span class="small muted">collections</span><strong>2</strong>'), "collections number is missing");
check(filled.includes('<span class="small muted">roles</span><strong>4</strong>'), "roles number is missing");
check(filled.includes('<span class="small muted">opentofu</span><strong>2048 B</strong>'), "tofu bytes are missing");
const cachePart = filled.slice(filled.indexOf("dependency_cache"));
check(!cachePart.includes('class="chart"'), "cache drew a fake chart line");
"##
    );
    run_node_probe("dashboard-page", &probe);
}

fn extract_braced_function(script: &str, needle: &str) -> String {
    let start = script
        .find(needle)
        .unwrap_or_else(|| panic!("missing {needle} in the UI script"));
    let mut depth = 0;
    let mut seen = false;
    for (offset, ch) in script[start..].char_indices() {
        if ch == '{' {
            depth += 1;
            seen = true;
        } else if ch == '}' {
            depth -= 1;
            if seen && depth == 0 {
                return script[start..start + offset + 1].to_string();
            }
        }
    }
    panic!("function after {needle} has no end");
}

fn extract_function_with_destructured_params(script: &str, needle: &str) -> String {
    let start = script
        .find(needle)
        .unwrap_or_else(|| panic!("missing {needle} in the UI script"));
    let body = script[start..]
        .find("\n}) {")
        .map(|offset| start + offset + 4)
        .unwrap_or_else(|| panic!("function after {needle} has no body"));
    let mut depth = 0;
    for (offset, ch) in script[body..].char_indices() {
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
            if depth == 0 {
                return script[start..body + offset + 1].to_string();
            }
        }
    }
    panic!("function after {needle} has no end");
}

fn assert_environment_warning_cases(script: &str) {
    let function = extract_braced_function(script, "function environmentRuntimeWarning");
    let probe = format!(
        r#"{function}
const cases = [
  ["auto", null, "ee", "this_execution_environment_is_ignored_until_docker_or"],
  ["auto", "", "ee", "this_execution_environment_is_ignored_until_docker_or"],
  ["auto", "docker", "ee", ""],
  ["auto", "podman", "ee", ""],
  ["auto", null, "", ""],
  ["docker", null, "ee", "an_execution_environment_is_set_but_neither_docker"],
  ["docker", "docker", "ee", ""],
  ["local", null, "ee", ""],
  ["", null, "ee", "this_execution_environment_is_ignored_until_docker_or"],
  [null, null, "ee", "this_execution_environment_is_ignored_until_docker_or"]
];
for (const [mode, runtime, environmentId, want] of cases) {{
  const got = environmentRuntimeWarning(mode, runtime, environmentId);
  if (got !== want) {{
    console.error(JSON.stringify({{mode, runtime, environmentId, want, got}}));
    process.exit(1);
  }}
}}
"#
    );
    run_node_probe("runtime-warning", &probe);
}

fn run_node_probe(name: &str, probe: &str) {
    let path = std::env::temp_dir().join(format!("koreograf-{}-{}.js", name, std::process::id()));
    std::fs::write(&path, probe).expect("probe");
    let output = std::process::Command::new("node")
        .arg(&path)
        .output()
        .expect("node");
    let _ = std::fs::remove_file(&path);
    assert!(
        output.status.success(),
        "probe {name} failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_chosen_entrypoint_cases(script: &str) {
    let function = extract_braced_function(script, "const chosenEntrypoint = () =>");
    let probe = format!(
        r#"{function}
let form;
const cases = [
  ["ansible", "baseline.yml", "other/path.yml", "other/path.yml"],
  ["ansible", "", "", ""],
  ["opentofu", "ignored", "modules/", "modules/"],
  ["shell", "", " scripts/run.sh ", "scripts/run.sh"]
];
for (const [tool, playbook, entrypoint, want] of cases) {{
  form = {{ tool: {{ value: tool }}, playbook: {{ value: playbook }}, entrypoint: {{ value: entrypoint }} }};
  const got = chosenEntrypoint();
  if (got !== want) {{
    console.error(JSON.stringify({{tool, playbook, entrypoint, want, got}}));
    process.exit(1);
  }}
}}
"#
    );
    run_node_probe("chosen-entrypoint", &probe);
}

fn assert_playbook_choice_cases(script: &str) {
    let function = extract_braced_function(script, "function playbookChoice");
    let probe = format!(
        r#"{function}
const cases = [
  [["a.yml", "b.yml"], "b.yml", "b.yml"],
  [["a.yml", "b.yml"], "", "a.yml"],
  [["a.yml"], "custom/path.yml", ""],
  [[], "", ""],
  [[], "x.yml", ""]
];
for (const [books, entrypoint, want] of cases) {{
  const got = playbookChoice(books, entrypoint);
  if (got !== want) {{
    console.error(JSON.stringify({{books, entrypoint, want, got}}));
    process.exit(1);
  }}
}}
"#
    );
    run_node_probe("playbook-choice", &probe);
}

async fn assert_same_runner(app: &axum::Router, cookie: &str, team: &str, overview: &Value) {
    let (status, denied, _) = call(app, "GET", "/api/runner", None, Some(team), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{denied}");
    let (status, runner, _) = call(app, "GET", "/api/runner", Some(cookie), Some(team), None).await;
    assert_eq!(status, StatusCode::OK, "{runner}");
    assert_eq!(runner["runner_mode"], overview["runner_mode"]);
    assert_eq!(runner["runtime"], overview["runtime"]);
    assert!(runner["local_tools"].is_object(), "{runner}");
    assert!(runner["container_image"].is_boolean(), "{runner}");
    assert_eq!(runner.as_object().map(|item| item.len()), Some(8));
}

#[test]
fn message_catalogs_match_and_cover_the_ui() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let de_path = root.join("web/i18n/de.json");
    let en_path = root.join("web/i18n/en.json");
    assert_catalog_sorted(&de_path);
    assert_catalog_sorted(&en_path);
    let de = load_catalog(&de_path);
    let en = load_catalog(&en_path);
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

#[test]
fn source_control_token_hints_are_in_the_forms_and_catalogs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    assert!(script.contains("[\"scm\", \"source_control\", \"source_control_credential_hint\"]"));
    assert!(script.contains("credHint(\"source_control_username_hint\")"));
    assert!(script.contains("h(\"source_control_project_hint\")"));
    assert!(script.contains("h(\"source_control_key_hint\")"));
    let en = std::fs::read_to_string(root.join("web/i18n/en.json")).expect("en");
    let de = std::fs::read_to_string(root.join("web/i18n/de.json")).expect("de");
    for catalog in [&en, &de] {
        assert!(catalog.contains("x-access-token"));
        assert!(catalog.contains("oauth2"));
        assert!(catalog.contains("source_control_credential_hint"));
        assert!(catalog.contains("source_control_username_hint"));
        assert!(catalog.contains("source_control_project_hint"));
    }
    let docs = std::fs::read_to_string(root.join("docs/jobs.md")).expect("docs");
    assert!(docs.contains("https://github.com/example/project.git"));
    assert!(docs.contains("x-access-token"));
    assert!(docs.contains("https://gitlab.com/example/project.git"));
    assert!(docs.contains("oauth2"));
}

fn assert_catalog_sorted(path: &Path) {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    let keys = top_level_json_keys(&text);
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "{} keys are not alphabetical", path.display());
}

fn top_level_json_keys(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut keys = Vec::new();
    let mut depth = 0;
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '"' {
            let mut key = String::new();
            index += 1;
            while index < chars.len() {
                let item = chars[index];
                if item == '\\' {
                    index += 1;
                    if index < chars.len() {
                        key.push(chars[index]);
                    }
                } else if item == '"' {
                    break;
                } else {
                    key.push(item);
                }
                index += 1;
            }
            let mut next = index + 1;
            while chars.get(next).is_some_and(|item| item.is_whitespace()) {
                next += 1;
            }
            if depth == 1 && chars.get(next) == Some(&':') {
                keys.push(key);
            }
            index += 1;
            continue;
        }
        if ch == '{' || ch == '[' {
            depth += 1;
        } else if ch == '}' || ch == ']' {
            depth -= 1;
        }
        index += 1;
    }
    keys
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
        push_call_key(&text, "ApiError::too_large(", keys);
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

#[tokio::test]
async fn api_tokens_authenticate_and_revoke() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-tokens-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state.clone());

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
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-token-local-{nanos}"));
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

#[tokio::test]
async fn mcp_tools_call_the_http_api() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-mcp-{nanos}"));
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
    let project_id = projects
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Demo")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
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
    let template_id = templates
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
        None,
        Some(json!({
            "username": "mcp-viewer",
            "display_name": "MCP viewer",
            "password": "viewer-pass",
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
        Some(json!({"username": "mcp-viewer", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, viewer_me, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "mcp-viewer", "password": "viewer-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer_me}");
    let viewer_cookie = viewer_cookie.expect("viewer cookie");

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        None,
        Some(json!({
            "username": "mcp-operator",
            "display_name": "MCP operator",
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
        Some(json!({"username": "mcp-operator", "role": "operator"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, operator_me, operator_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "mcp-operator", "password": "operator-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{operator_me}");
    let operator_cookie = operator_cookie.expect("operator cookie");

    let (status, admin_token, _) = call(
        &app,
        "POST",
        "/api/me/tokens",
        Some(&cookie),
        None,
        Some(json!({"name": "mcp-admin"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let admin_secret = admin_token["secret"].as_str().unwrap().to_string();
    let (status, viewer_token, _) = call(
        &app,
        "POST",
        "/api/me/tokens",
        Some(&viewer_cookie),
        None,
        Some(json!({"name": "mcp-viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let viewer_secret = viewer_token["secret"].as_str().unwrap().to_string();
    let (status, operator_token, _) = call(
        &app,
        "POST",
        "/api/me/tokens",
        Some(&operator_cookie),
        None,
        Some(json!({"name": "mcp-operator"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let operator_secret = operator_token["secret"].as_str().unwrap().to_string();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let served = app.clone();
    let server = tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });
    let url = format!("http://{address}");

    let client =
        koreograf::mcp::Client::new(url.clone(), admin_secret, Some(team.clone())).unwrap();
    assert_eq!(client.whoami(None).await.unwrap()["username"], "admin");
    assert_eq!(
        client.version(None).await.unwrap()["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert!(client
        .list_projects(None)
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "Demo"));
    assert!(client.list_inventories(None).await.unwrap().is_array());
    assert!(client.list_credentials(None).await.unwrap().is_array());
    assert!(client
        .list_templates(None)
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "Healthcheck"));
    assert!(client.list_workflows(None).await.unwrap().is_array());
    assert!(client.list_schedules(None).await.unwrap().is_array());
    let jobs = client
        .list_jobs(koreograf::mcp::JobQuery {
            limit: Some(5),
            status: Some("successful"),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(jobs["items"].is_array());

    let launched = client
        .launch_template(&template_id, serde_json::Map::new(), None)
        .await
        .unwrap();
    let job_id = launched["id"].as_str().unwrap().to_string();
    let mut final_job = Value::Null;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let job = client.get_job(&job_id, None).await.unwrap();
        if matches!(
            job["status"].as_str(),
            Some("successful" | "failed" | "canceled")
        ) {
            final_job = job;
            break;
        }
    }
    assert_eq!(final_job["status"], "successful", "job: {final_job}");
    let output = client.job_output(&job_id, None).await.unwrap();
    assert!(output.contains("healthcheck ok"), "{output}");
    let synced = client.sync_project(&project_id, None).await.unwrap();
    assert!(synced["path"].is_string(), "{synced}");

    let viewer =
        koreograf::mcp::Client::new(url, viewer_secret.clone(), Some(team.clone())).unwrap();
    let denied = viewer
        .launch_template(&template_id, serde_json::Map::new(), None)
        .await
        .unwrap_err();
    let (route_status, route_body) = call_bearer(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        &viewer_secret,
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(denied.status, Some(route_status.as_u16()));
    assert_eq!(denied.key, route_body["error"].as_str().unwrap());

    let session = mcp_session(&app, &operator_secret).await;
    let (status, bytes, _) = mcp_request(
        &app,
        Some(&operator_secret),
        Some(&session),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "launch_template",
                "arguments": {
                    "id": template_id,
                    "extra_vars": {"from_mcp_http": "yes"},
                    "team_id": team
                }
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
    let message = sse_message(&bytes);
    assert_eq!(message["result"]["isError"], false, "{message}");
    let launched: Value =
        serde_json::from_str(message["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(launched["template_id"], template_id);
    assert_eq!(launched["extra_vars"]["from_mcp_http"], "yes");
    assert!(launched["id"].is_string(), "{launched}");

    let viewer_session = mcp_session(&app, &viewer_secret).await;
    let (status, bytes, _) = mcp_request(
        &app,
        Some(&viewer_secret),
        Some(&viewer_session),
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
    assert!(
        sse_message(&bytes)
            .to_string()
            .contains("you_are_not_allowed_to_do_this"),
        "{}",
        String::from_utf8_lossy(&bytes)
    );

    let (status, bytes, _) = mcp_request(&app, None, None, mcp_initialize_body()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let refused: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(refused["error"], "sign_in_is_required");

    let (status, bytes, _) =
        mcp_request(&app, Some("koro_unknown"), None, mcp_initialize_body()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let refused: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(refused["error"], "sign_in_is_required");

    server.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

fn mcp_initialize_body() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "smoke", "version": "0.1"}
        }
    })
}

async fn mcp_request(
    app: &axum::Router,
    token: Option<&str>,
    session: Option<&str>,
    body: Value,
) -> (StatusCode, Vec<u8>, axum::http::HeaderMap) {
    let mut extra = vec![
        ("host", "localhost".to_string()),
        ("content-type", "application/json".to_string()),
        ("accept", "application/json, text/event-stream".to_string()),
    ];
    if let Some(token) = token {
        extra.push(("authorization", format!("Bearer {token}")));
    }
    if let Some(session) = session {
        extra.push(("mcp-session-id", session.to_string()));
    }
    let extra: Vec<(&str, &str)> = extra
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    call_raw(
        app,
        "POST",
        "/mcp",
        None,
        &extra,
        Some(body.to_string().into_bytes()),
    )
    .await
}

async fn mcp_session(app: &axum::Router, token: &str) -> String {
    let (status, bytes, headers) = mcp_request(app, Some(token), None, mcp_initialize_body()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let session = headers["mcp-session-id"].to_str().unwrap().to_string();
    let (status, bytes, _) = mcp_request(
        app,
        Some(token),
        Some(&session),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    session
}

fn sse_message(bytes: &[u8]) -> Value {
    for line in String::from_utf8_lossy(bytes).lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<Value>(data.trim()) {
            if value.get("jsonrpc").is_some() {
                return value;
            }
        }
    }
    panic!("no SSE JSON message in {}", String::from_utf8_lossy(bytes));
}

#[tokio::test]
async fn mcp_http_routes_can_be_turned_off() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-mcp-off-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = config(&dir);
    cfg.mcp = false;
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
    let (status, me, _) = call(&app, "GET", "/api/me", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");

    let (status, _, _) = mcp_request(&app, None, None, mcp_initialize_body()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let _ = std::fs::remove_dir_all(&dir);
}
