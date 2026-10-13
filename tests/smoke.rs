use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use koreograf::{Config, RunnerMode};
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
    assert!(script.contains("Ausgabe"));
    assert!(script.contains("Ausgabe herunterladen"));
    assert!(script.contains("data-relaunch"));
    assert!(script.contains("Erneut starten"));
    assert!(script.contains("Nur fehlgeschlagene Hosts"));
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
    assert_eq!(
        denied["error"],
        "Nur Ansible-Aufträge haben fehlgeschlagene Hosts."
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
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir)).unwrap();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("Inventare"), "{script}");
    assert!(script.contains("Organisationen"));
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

    let origin = dir.join("origin");
    std::fs::create_dir_all(&origin).unwrap();
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
            lower.contains("fatal") || lower.contains("repository") || lower.contains("exist"),
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
            "display_name": "Operator",
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
    assert!(script.contains("Playbook aus dem Projekt"), "{script}");
    assert!(script.contains("data-add-kind=\"project\""));
    assert!(script.contains("data-add-kind=\"netbox\""));
    assert!(script.contains("Aktualisieren"));
    assert!(script.contains("value=\"netbox\""));

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
        .contains("keine externe Quelle"));

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
        .contains("synchronisieren"));

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
    assert!(bad["error"].as_str().unwrap_or("").contains("403"), "{bad}");
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
    assert!(script.contains("+ Inventar"));
    assert!(script.contains("+ Projekt"));
    assert!(script.contains("+ Zugang"));
    assert!(script.contains("+ Umgebung"));
    assert!(script.contains("data-lang-select"));
    assert!(script.contains("FLAG_SVG"));
    assert!(script.contains("country: \"gb\""));
    assert!(script.contains("data-add-kind"));
    assert!(script.contains("Check (Dry-Run)"));
    assert!(script.contains("Tags überspringen"));
    assert!(script.contains("0 (Normal)"));
    assert!(script.contains("1 (Ausführlich)"));
    assert!(script.contains("2 (Ausführlicher)"));
    assert!(script.contains("4 (Verbindungs-Debug)"));
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
    assert!(script.contains("Sicherung herunterladen"));
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
    assert!(rejected.contains("ERSETZEN"), "{rejected}");

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
    assert!(script.contains("Benutzer suchen"));
    assert!(script.contains("#/users"));
    assert!(script.contains("+ Benutzer"));

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
    assert_eq!(duplicate["error"], "Der Benutzername ist schon vergeben.");

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
    assert_eq!(blocked["error"], "Das Konto ist deaktiviert.");
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
    assert_eq!(
        self_off["error"],
        "Du kannst dein eigenes Konto nicht deaktivieren."
    );
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
    assert_eq!(demote["error"], "Der letzte Administrator bleibt erhalten.");

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
    assert_eq!(
        self_delete["error"],
        "Du kannst dein eigenes Konto nicht löschen."
    );

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
    assert!(script.contains("Mein Konto"));
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
    assert_eq!(broken["error"], "E-Mail ist ungültig.");

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
