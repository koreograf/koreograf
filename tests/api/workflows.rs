use std::time::Duration;

use axum::http::StatusCode;
use serde_json::json;

use crate::common::call_cookie as call;

use crate::common::*;

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

#[tokio::test]
async fn a_workflow_node_without_an_inventory_fails_before_launching() {
    let harness = TestApp::boot("wf-inventory", Options::default());
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
async fn workflow_node_can_override_template_parameters() {
    let harness = TestApp::boot("node-params", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/features/legacy.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
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
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    assert_eq!(missing["error"], "not_found");

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
