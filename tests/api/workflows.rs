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

#[tokio::test(flavor = "multi_thread")]
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

    let (status, cycle, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Cyclic run",
            "description": "",
            "graph": {
                "nodes": [
                    {"id": "a", "template_id": template_id, "name": "A"},
                    {"id": "b", "template_id": template_id, "name": "B"}
                ],
                "edges": [
                    {"from": "a", "to": "b", "when": "success"},
                    {"from": "b", "to": "a", "when": "success"}
                ]
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(cycle["error"], "the_workflow_contains_a_cycle");

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

#[tokio::test(flavor = "multi_thread")]
async fn workflow_node_can_override_template_parameters() {
    let harness = TestApp::boot("node-params", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/main.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);

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

async fn shell_workflow(
    app: &axum::Router,
    cookie: &str,
    team: &str,
    name: &str,
    script: &str,
) -> String {
    let (status, template, _) = call(
        app,
        "POST",
        "/api/templates",
        Some(cookie),
        Some(team),
        Some(json!({
            "name": format!("{name} template"),
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": script}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();
    let (status, workflow, _) = call(
        app,
        "POST",
        "/api/workflows",
        Some(cookie),
        Some(team),
        Some(json!({
            "name": name,
            "description": "",
            "graph": {
                "nodes": [{"id": "one", "template_id": template_id, "name": "one"}],
                "edges": []
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    workflow["id"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_workflow_run_can_be_canceled() {
    let harness = TestApp::boot("workflow-cancel", Options::default());
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
    let workflow_id = shell_workflow(&app, &cookie, &team, "Workflow sleeper", "sleep 30\n").await;

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
    let run_id = run["id"].as_str().unwrap().to_string();

    let mut job_id = None;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(100)).await;
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
        job_id = live["nodes"][0]["job_id"].as_str().map(str::to_string);
        if job_id.is_some() {
            break;
        }
    }
    let job_id = job_id.expect("the workflow did not start its node job");

    let (status, denied, _) = call(
        &app,
        "POST",
        &format!("/api/workflow-jobs/{run_id}/relaunch"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    assert_eq!(
        denied["error"],
        "only_a_finished_workflow_run_can_be_relaunched"
    );

    let (status, canceled, _) = call(
        &app,
        "POST",
        &format!("/api/workflow-jobs/{run_id}/cancel"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    assert_eq!(canceled["status"], "canceled");

    let (status, child, _) = call(
        &app,
        "GET",
        &format!("/api/jobs/{job_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{child}");
    assert_eq!(child["status"], "canceled", "{child}");

    tokio::time::sleep(Duration::from_millis(300)).await;
    let (status, settled, _) = call(
        &app,
        "GET",
        &format!("/api/workflow-jobs/{run_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{settled}");
    assert_eq!(settled["status"], "canceled", "{settled}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_workflow_run_can_be_listed_and_relaunched() {
    let harness = TestApp::boot("workflow-relaunch", Options::default());
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
    let workflow_id =
        shell_workflow(&app, &cookie, &team, "Workflow echo", "echo workflow-ok\n").await;

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
    let run_id = run["id"].as_str().unwrap().to_string();
    let finished = wait_for_workflow(&app, &cookie, &team, &run_id).await;
    assert_eq!(finished["status"], "successful", "{finished}");

    let (status, listed, _) = call(
        &app,
        "GET",
        &format!("/api/workflow-jobs?workflow_id={workflow_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let items = listed.as_array().expect("run list");
    assert!(items.iter().any(|item| item["id"] == run_id), "{listed}");
    assert_eq!(items[0]["trigger"], "manual", "{listed}");

    let (status, copy, _) = call(
        &app,
        "POST",
        &format!("/api/workflow-jobs/{run_id}/relaunch"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{copy}");
    assert_ne!(copy["id"], finished["id"]);
    assert_eq!(copy["trigger"], "relaunch");
    let copy_id = copy["id"].as_str().unwrap().to_string();

    let (status, listed, _) = call(
        &app,
        "GET",
        &format!("/api/workflow-jobs?workflow_id={workflow_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed.as_array().map(Vec::len), Some(2), "{listed}");

    let copy = wait_for_workflow(&app, &cookie, &team, &copy_id).await;
    assert_eq!(copy["status"], "successful", "{copy}");

    let (status, workflow, _) = call(
        &app,
        "GET",
        &format!("/api/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{workflow}");
    assert_eq!(workflow["last_run_status"], "successful", "{workflow}");
    assert!(workflow["last_run_id"].is_string(), "{workflow}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_workflow_save_is_rejected_and_keeps_the_first_writer() {
    let harness = TestApp::boot("workflow-conflict", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/v1/login",
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
        "/api/v1/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Conflict shell",
            "description": "",
            "tool": "shell",
            "no_environment": true,
            "spec": {"script": ":"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    let graph = |name: &str| {
        json!({
            "nodes": [{"id": "one", "template_id": template_id, "name": name}],
            "edges": []
        })
    };
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/v1/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Conflict workflow", "description": "", "graph": graph("First")})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let workflow_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["version"], 1, "{created}");

    let (status, first, _) = call(
        &app,
        "PUT",
        &format!("/api/v1/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Conflict workflow",
            "description": "",
            "graph": graph("First"),
            "version": 1
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["version"], 2, "{first}");

    let (status, stale, _) = call(
        &app,
        "PUT",
        &format!("/api/v1/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Conflict workflow 2",
            "description": "",
            "graph": graph("Second"),
            "version": 1
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["error"], "workflow_changed");
    assert_eq!(stale["vars"]["version"], "2", "{stale}");

    let (status, kept, _) = call(
        &app,
        "GET",
        &format!("/api/v1/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["version"], 2, "{kept}");
    assert_eq!(kept["name"], "Conflict workflow", "{kept}");
    assert_eq!(kept["graph"]["nodes"][0]["name"], "First", "{kept}");

    let (status, second, _) = call(
        &app,
        "PUT",
        &format!("/api/v1/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Conflict workflow 2",
            "description": "",
            "graph": graph("Second"),
            "version": 2
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["version"], 3, "{second}");
    assert_eq!(second["graph"]["nodes"][0]["name"], "Second", "{second}");

    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/v1/workflows",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let item = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == workflow_id)
        .expect("workflow in list");
    assert_eq!(item["version"], 3, "{listed}");

    let (status, legacy, _) = call(
        &app,
        "PUT",
        &format!("/api/v1/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Conflict workflow 3",
            "description": "",
            "graph": graph("Third")
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{legacy}");
    assert_eq!(legacy["version"], 4, "{legacy}");

    let _ = std::fs::remove_dir_all(&dir);
}
