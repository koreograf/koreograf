use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use koreograf::LocalJobs;
use serde_json::json;

use crate::common::call_cookie as call;

use crate::common::*;

#[tokio::test]
async fn a_hook_delivery_is_rejected_when_local_runs_are_admin_only() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-hook-local-admin-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
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
    let state = koreograf::boot(config(&dir, &Options::default())).unwrap();
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

#[tokio::test]
async fn a_hook_can_be_edited_and_keeps_its_token() {
    let harness = TestApp::boot("hook-edit", Options::default());
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
