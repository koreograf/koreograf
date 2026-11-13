use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use koreograf::LocalJobs;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::common::call_cookie as call;

use crate::common::*;

#[tokio::test]
async fn the_job_log_marks_a_fatal_line_and_keeps_galaxy_as_a_warning() {
    let harness = TestApp::boot("log-style", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/main.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
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
    let state = koreograf::boot(config(&dir, &Options::default())).unwrap();
    let app = koreograf::router(state);

    let (status, index, _) = call(&app, "GET", "/", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(index.as_str().unwrap_or("").contains("/js/main.js"));
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

#[tokio::test]
async fn failed_job_shows_parameters_and_can_be_relaunched() {
    let harness = TestApp::boot("relaunch", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/main.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
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

    let (status, blocked, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {"from_launch": "yes"}})),
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
            "launch_with_options": true
        })),
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
    let dir = temp_dir("prompt");
    let mut cfg = config(&dir, &Options::default());
    cfg.local_jobs = LocalJobs::Admins;
    let state = koreograf::boot(cfg).unwrap();
    let app = koreograf::router(state);

    let (status, _, _) = call(&app, "GET", "/js/main.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
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
    assert_eq!(status, StatusCode::NOT_FOUND, "{wrong_inventory}");
    assert_eq!(wrong_inventory["error"], "not_found");
    let (status, wrong_credential, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}, "credential_ids": [foreign_credential_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{wrong_credential}");
    assert_eq!(wrong_credential["error"], "not_found");

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
    let harness = TestApp::boot("launch-options", Options::default());
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
    let harness = TestApp::boot("inventory-required", Options::default());
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
async fn oversized_job_output_keeps_the_recap_in_the_download() {
    let dir = temp_dir("output");
    let mut cfg = config(&dir, &Options::default());
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

#[tokio::test]
async fn an_operator_cannot_start_a_local_job_or_an_arbitrary_path() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = temp_dir("local-admin");
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
async fn job_list_pages_searches_filters_and_reloads_the_hash() {
    let harness = TestApp::boot("job-list", Options::default());
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

#[tokio::test]
async fn a_relative_data_dir_can_run_a_local_shell_job() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let relative = std::path::PathBuf::from(format!("target/koreograf-rel-{nanos}"));
    assert!(relative.is_relative());
    std::fs::create_dir_all(&relative).unwrap();
    let state = koreograf::boot(config(&relative, &Options::default())).unwrap();
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
    let harness = TestApp::boot("cancel", Options::default());
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
    let harness = TestApp::boot("finished-early", Options::default());
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

#[tokio::test]
async fn cancel_kills_a_child_that_ignores_term() {
    let harness = TestApp::boot("cancel-trap", Options::default());
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
    let harness = TestApp::boot("cancel-detached", Options::default());
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
