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
    assert!(script.contains("[\"netbox\", \"NetBox\", \"\"]"));
    let inspector = script
        .split_once("function renderInspector")
        .expect("workflow inspector")
        .1;
    let node_at = inspector
        .find("Dieser Knoten")
        .expect("selected node panel");
    let add_at = inspector[..node_at]
        .rfind("addNodeControls()")
        .expect("add stays above the selected node");
    assert!(
        node_at - add_at < 800,
        "add node drifted away from the selected node"
    );
    let node_panel = &inspector[node_at..inspector.find("node-delete").expect("delete node")];
    assert!(node_panel.contains("Erfolg führt zu"));
    assert!(node_panel.contains("Fehler führt zu"));
    assert!(node_panel.contains("Immer führt zu"));
    assert!(node_panel.contains("nodeParams("));
    assert!(script.contains("Gilt nur für diesen Knoten"));
    assert!(script.contains("node-inventory"));
    assert!(script.contains("Ohne Inventar"));

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
    assert!(script.contains("Ein Klick zeigt die Ausgabe"));
    assert!(script.contains("Zurück zum Workflow"));
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
                "name": "Test",
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
    assert_eq!(missing["error"], "Inventar fehlt.");

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
    assert_eq!(
        bad_extra["error"],
        "Extra-Variablen muss ein JSON-Objekt sein."
    );

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
        .find("Execution Environments")
        .expect("environment heading");
    let collections = runtime.find("Collections im Cache").expect("cache heading");
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
    assert_eq!(rejected["error"], "Image-Name ist ungültig.");

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
    assert!(script.contains("Maschine (SSH)"));
    assert!(script.contains("Amazon Web Services"));
    assert!(script.contains("Schlüssel oder Passwort, auch beides."));
    assert!(script.contains("Gilt im Auftrag. Der Projekt-Sync nutzt diesen Zugang noch nicht."));
    assert!(script.contains("Leer lassen, um den Wert zu behalten."));
    assert!(script.contains("data-stored-mask=\"1\""));
    assert!(script.contains("class=\"secret-mask\""));
    assert!(!script.contains("*** bedeutet, dass ein Wert gespeichert ist."));
    assert!(script.contains("Die Art lässt sich nicht ändern."));
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
    assert_eq!(
        cleared["error"],
        "SSH braucht einen Schlüssel oder ein Passwort."
    );
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
    assert_eq!(
        rejected["error"],
        "SSH braucht einen Schlüssel oder ein Passwort."
    );
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
    assert_eq!(unknown["error"], "Unbekannte Zugangsart.");
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
    assert!(script.contains(
        "Pro Art ein Zugang. Vault-Zugänge wählst du über Auswählen. Sie stehen hier als Pillen."
    ));
    assert!(script.contains(
        "Gilt nur für diesen Knoten. Die Vorlage bleibt unverändert. Ein anderer Zugang ersetzt nur dieselbe Art."
    ));
    assert!(script.contains("id=\"vault-pills\""));
    assert!(script.contains("Keine Vault-Zugänge im Team."));
    assert!(script.contains("Vault-Zugang {name} entfernen"));

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
    assert_eq!(second_ssh["error"], "Pro Art ist nur ein Zugang erlaubt.");

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
    assert_eq!(same_label["error"], "Diese Vault-Id ist schon gewählt.");

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
        "Nur ein Vault-Zugang darf ohne Vault-Id sein."
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
    assert!(script.contains("Einstellungen"));
    assert!(script.contains("Bind-Passwort"));
    assert!(script.contains("Proxy-Adresse"));
    assert!(script.contains("Umgebungsvariablen verwenden"));
    assert!(script.contains("data-stored-mask=\"1\""));
    assert!(!script.contains("bind-secret-value"));
    assert!(!script.contains("proxy-secret-value"));

    let (status, denied, _) = call(&app, "GET", "/api/settings", None, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{denied}");
    assert_eq!(denied["error"], "Anmeldung erforderlich.");

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
    assert_eq!(forbidden["error"], "Keine Berechtigung für diese Aktion.");

    let (status, current, _) = call(&app, "GET", "/api/settings", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{current}");
    assert_eq!(current["source"], "env");
    assert_eq!(current["runner"], "local");
    assert_eq!(current["ldap"]["url"], "");
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
        "job_timeout_secs": 90
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
    assert_eq!(saved["ldap"]["user_filter"], "(sAMAccountName={username})");
    assert_eq!(saved["ldap"]["bind_password_set"], true);
    assert_eq!(saved["proxy"]["password_set"], true);
    assert_eq!(saved["proxy"]["url"], "http://proxy.example:3128");
    assert_eq!(saved["runner"], "docker");
    assert_eq!(saved["job_timeout_secs"], 90);
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
        "Zugangsdaten gehören in die eigenen Felder, nicht in die Proxy-Adresse."
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

    let _ = std::fs::remove_dir_all(&dir);
}
