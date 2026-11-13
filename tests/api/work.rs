use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::common::call_cookie as call;

use crate::common::*;

async fn admin_team(app: &axum::Router, name: &str) -> (String, String) {
    let (status, me, cookie) = call(
        app,
        "POST",
        "/api/v1/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let (status, team, _) = call(
        app,
        "POST",
        "/api/v1/teams",
        Some(&cookie),
        None,
        Some(json!({"name": name, "description": "Shape team"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{team}");
    let team_id = team["id"].as_str().expect("team id").to_string();
    (cookie, team_id)
}

async fn shell_template(
    app: &axum::Router,
    cookie: &str,
    team: &str,
    name: &str,
    script: &str,
) -> String {
    let (status, template, _) = call(
        app,
        "POST",
        "/api/v1/templates",
        Some(cookie),
        Some(team),
        Some(json!({
            "name": name,
            "description": "",
            "tool": "shell",
            "no_environment": true,
            "spec": {"script": script},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    template["id"].as_str().expect("template id").to_string()
}

fn job_parameters(template_id: &str, name: &str, script: &str) -> Value {
    json!({
        "template_id": template_id,
        "template_name": name,
        "tool": "shell",
        "entrypoint": "",
        "project_id": null,
        "project_name": null,
        "inventory_id": null,
        "inventory_name": null,
        "credential_id": null,
        "credential_name": null,
        "credential_kind": null,
        "credentials": [],
        "environment_id": null,
        "no_environment": true,
        "environment_name": null,
        "environment_image": null,
        "extra_vars": {},
        "spec": {"script": script},
        "update_on_launch": true,
    })
}

fn job_shape(
    value: &Value,
    team: &str,
    template_id: &str,
    name: &str,
    trigger: &str,
    parameters: &Value,
) -> Value {
    json!({
        "id": value["id"].clone(),
        "team_id": team,
        "template_id": template_id,
        "workflow_job_id": null,
        "workflow_node_id": null,
        "name": name,
        "tool": "shell",
        "status": value["status"].clone(),
        "extra_vars": {},
        "launched_by": "admin",
        "trigger": trigger,
        "started_at": value["started_at"].clone(),
        "finished_at": value["finished_at"].clone(),
        "created_at": value["created_at"].clone(),
        "parameters": parameters,
        "failed_hosts": [],
    })
}

fn event_shape(value: &Value) -> Value {
    let event = value.as_object().expect("event object");
    assert_eq!(event.len(), 4, "{value}");
    assert!(value["id"].is_i64(), "{value}");
    assert!(value["ts"].is_string(), "{value}");
    assert!(value["stream"].is_string(), "{value}");
    assert!(value["line"].is_string(), "{value}");
    json!({
        "id": value["id"].clone(),
        "ts": value["ts"].clone(),
        "stream": value["stream"].clone(),
        "line": value["line"].clone(),
    })
}

fn job_events(value: &Value) -> Vec<Value> {
    let events = value["events"].as_array().expect("events array");
    assert!(!events.is_empty());
    events.iter().map(event_shape).collect()
}

async fn wait_for_job(app: &axum::Router, cookie: &str, team: &str, id: &str) -> Value {
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let (status, body, _) = call(
            app,
            "GET",
            &format!("/api/v1/jobs/{id}"),
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
    panic!("job {id} did not finish");
}

#[tokio::test]
async fn templates_return_the_documented_shape() {
    let harness = TestApp::boot("work-templates", Options::default());
    let app = harness.app.clone();
    let (cookie, team) = admin_team(&app, "Shape Templates").await;

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/v1/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Shape Template",
            "description": "Shape description",
            "tool": "shell",
            "no_environment": true,
            "spec": {"script": "echo shape"},
            "extra_vars": {"answer": 1},
            "update_on_launch": false,
            "prompt_on_launch": true,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let template_id = created["id"].as_str().expect("template id");
    let mut expected = json!({
        "id": created["id"].clone(),
        "team_id": team,
        "name": "Shape Template",
        "description": "Shape description",
        "tool": "shell",
        "project_id": null,
        "inventory_id": null,
        "credential_id": null,
        "credential_ids": [],
        "environment_id": null,
        "no_environment": true,
        "entrypoint": "",
        "extra_vars": {"answer": 1},
        "spec": {"script": "echo shape"},
        "created_at": created["created_at"].clone(),
        "update_on_launch": false,
        "prompt_on_launch": true,
    });
    assert_eq!(created, expected);

    let (status, list, _) = call(
        &app,
        "GET",
        "/api/v1/templates",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list, json!([expected.clone()]));

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/v1/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Shape Template 2",
            "description": "Shape description 2",
            "tool": "shell",
            "no_environment": true,
            "spec": {"script": "echo shape"},
            "extra_vars": {"answer": 2},
            "update_on_launch": true,
            "prompt_on_launch": true,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    expected["name"] = json!("Shape Template 2");
    expected["description"] = json!("Shape description 2");
    expected["extra_vars"] = json!({"answer": 2});
    expected["update_on_launch"] = json!(true);
    assert_eq!(updated, expected);

    let (status, deleted, _) = call(
        &app,
        "DELETE",
        &format!("/api/v1/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{deleted}");
    assert_eq!(deleted, Value::Null);

    let (status, list, _) = call(
        &app,
        "GET",
        "/api/v1/templates",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list, json!([]));
    let _ = std::fs::remove_dir_all(&harness.dir);
}

#[tokio::test]
async fn jobs_return_the_documented_shape() {
    let harness = TestApp::boot("work-jobs", Options::default());
    let app = harness.app.clone();
    let (cookie, team) = admin_team(&app, "Shape Jobs").await;
    let template_id =
        shell_template(&app, &cookie, &team, "Shape Job", "printf 'shape-job\\n'").await;
    let parameters = job_parameters(&template_id, "Shape Job", "printf 'shape-job\\n'");

    let (status, launched, _) = call(
        &app,
        "POST",
        &format!("/api/v1/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");
    let job_id = launched["id"].as_str().expect("job id").to_string();
    assert_eq!(
        launched,
        job_shape(
            &launched,
            &team,
            &template_id,
            "Shape Job",
            "manual",
            &parameters
        )
    );
    assert!(
        matches!(
            launched["status"].as_str(),
            Some("pending" | "running" | "successful")
        ),
        "{launched}"
    );

    let finished = wait_for_job(&app, &cookie, &team, &job_id).await;
    assert_eq!(finished["status"], "successful", "{finished}");
    let mut expected = job_shape(
        &finished,
        &team,
        &template_id,
        "Shape Job",
        "manual",
        &parameters,
    );
    expected["events"] = json!(job_events(&finished));
    expected["output_capped"] = json!(false);
    assert_eq!(finished, expected);

    let (status, list, _) = call(
        &app,
        "GET",
        "/api/v1/jobs",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(
        list,
        json!({
            "items": [job_shape(&finished, &team, &template_id, "Shape Job", "manual", &parameters)],
            "total": 1,
        })
    );

    let mut canceled_expected = finished.clone();
    canceled_expected
        .as_object_mut()
        .expect("job object")
        .remove("events");
    canceled_expected
        .as_object_mut()
        .expect("job object")
        .remove("output_capped");
    let (status, canceled, _) = call(
        &app,
        "POST",
        &format!("/api/v1/jobs/{job_id}/cancel"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    assert_eq!(canceled, canceled_expected);

    let (status, output, _) = call(
        &app,
        "GET",
        &format!("/api/v1/jobs/{job_id}/output"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{output}");
    assert!(
        output.as_str().unwrap_or("").contains("shape-job"),
        "{output}"
    );

    let (status, stream, _) = call(
        &app,
        "GET",
        &format!("/api/v1/jobs/{job_id}/events?after=0"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stream}");
    let stream = stream.as_str().expect("event stream");
    let data = stream
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|line| serde_json::from_str::<Value>(line).expect("event data"))
        .collect::<Vec<_>>();
    assert!(!data.is_empty(), "{stream}");
    let end = data.last().expect("end event");
    assert_eq!(
        end,
        &json!({
            "status": "successful",
            "finished_at": finished["finished_at"].clone(),
        })
    );
    for event in &data[..data.len() - 1] {
        assert_eq!(event, &event_shape(event));
    }

    let fail_template = shell_template(
        &app,
        &cookie,
        &team,
        "Shape Fail",
        "printf 'shape-fail\\n' >&2; exit 1",
    )
    .await;
    let (status, failed, _) = call(
        &app,
        "POST",
        &format!("/api/v1/templates/{fail_template}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{failed}");
    let failed_id = failed["id"].as_str().expect("failed job id");
    let failed = wait_for_job(&app, &cookie, &team, failed_id).await;
    assert_eq!(failed["status"], "failed", "{failed}");

    let (status, relaunched, _) = call(
        &app,
        "POST",
        &format!("/api/v1/jobs/{failed_id}/relaunch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"only_failed_hosts": false})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{relaunched}");
    let fail_parameters = job_parameters(
        &fail_template,
        "Shape Fail",
        "printf 'shape-fail\\n' >&2; exit 1",
    );
    assert_eq!(
        relaunched,
        job_shape(
            &relaunched,
            &team,
            &fail_template,
            "Shape Fail",
            "relaunch",
            &fail_parameters
        )
    );
    let _ = std::fs::remove_dir_all(&harness.dir);
}

fn workflow_graph(template_id: &str) -> Value {
    json!({
        "nodes": [{
            "id": "one",
            "template_id": template_id,
            "name": "One",
            "x": 10.0,
            "y": 20.0,
            "convergence": "any",
        }],
        "edges": [],
    })
}

fn workflow_shape(value: &Value, team: &str, name: &str, description: &str) -> Value {
    json!({
        "id": value["id"].clone(),
        "team_id": team,
        "name": name,
        "description": description,
        "graph": value["graph"].clone(),
        "created_at": value["created_at"].clone(),
    })
}

fn workflow_node_shape(value: &Value) -> Value {
    let node = value.as_object().expect("node object");
    assert_eq!(node.len(), 8, "{value}");
    assert!(value["node_id"].is_string(), "{value}");
    assert!(value["phase"].is_string(), "{value}");
    for key in ["job_id", "job_name", "tool", "started_at", "finished_at"] {
        assert!(
            value[key].is_null() || value[key].is_string(),
            "{key}: {value}"
        );
    }
    assert!(value["job_created_at"].is_null() || value["job_created_at"].is_string());
    json!({
        "node_id": value["node_id"].clone(),
        "phase": value["phase"].clone(),
        "job_id": value["job_id"].clone(),
        "job_name": value["job_name"].clone(),
        "tool": value["tool"].clone(),
        "started_at": value["started_at"].clone(),
        "finished_at": value["finished_at"].clone(),
        "job_created_at": value["job_created_at"].clone(),
    })
}

fn workflow_run_shape(
    value: &Value,
    team: &str,
    workflow_id: &str,
    name: &str,
    graph: &Value,
) -> Value {
    json!({
        "id": value["id"].clone(),
        "workflow_id": workflow_id,
        "team_id": team,
        "name": name,
        "status": value["status"].clone(),
        "extra_vars": {},
        "launched_by": "admin",
        "trigger": "manual",
        "created_at": value["created_at"].clone(),
        "finished_at": value["finished_at"].clone(),
        "graph": graph,
        "nodes": value["nodes"]
            .as_array()
            .expect("nodes array")
            .iter()
            .map(workflow_node_shape)
            .collect::<Vec<_>>(),
    })
}

async fn wait_for_workflow(app: &axum::Router, cookie: &str, team: &str, id: &str) -> Value {
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let (status, body, _) = call(
            app,
            "GET",
            &format!("/api/v1/workflow-jobs/{id}"),
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
    panic!("workflow job {id} did not finish");
}

#[tokio::test]
async fn workflows_and_workflow_jobs_return_the_documented_shape() {
    let harness = TestApp::boot("work-workflows", Options::default());
    let app = harness.app.clone();
    let (cookie, team) = admin_team(&app, "Shape Workflows").await;
    let template_id =
        shell_template(&app, &cookie, &team, "Shape Workflow Node", "echo workflow").await;
    let graph = workflow_graph(&template_id);

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/v1/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Shape Workflow",
            "description": "Shape workflow",
            "graph": graph,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let workflow_id = created["id"].as_str().expect("workflow id").to_string();
    let mut expected = workflow_shape(&created, &team, "Shape Workflow", "Shape workflow");
    assert_eq!(created, expected);
    assert_eq!(created["graph"], graph);

    let (status, list, _) = call(
        &app,
        "GET",
        "/api/v1/workflows",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list, json!([expected.clone()]));

    let (status, fetched, _) = call(
        &app,
        "GET",
        &format!("/api/v1/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched, expected);

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/v1/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Shape Workflow 2",
            "description": "Shape workflow 2",
            "graph": graph,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    expected = workflow_shape(&updated, &team, "Shape Workflow 2", "Shape workflow 2");
    assert_eq!(updated, expected);

    let (status, run, _) = call(
        &app,
        "POST",
        &format!("/api/v1/workflows/{workflow_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{run}");
    let run_id = run["id"].as_str().expect("workflow job id").to_string();
    assert_eq!(run["status"], "running");
    assert_eq!(
        run,
        workflow_run_shape(&run, &team, &workflow_id, "Shape Workflow 2", &graph)
    );

    let finished = wait_for_workflow(&app, &cookie, &team, &run_id).await;
    assert_eq!(finished["status"], "successful", "{finished}");
    let node = &finished["nodes"][0];
    assert_eq!(node["phase"], "successful");
    assert!(node["job_id"].is_string(), "{finished}");
    assert_eq!(node["job_name"], "Shape Workflow Node");
    assert_eq!(node["tool"], "shell");
    assert!(node["started_at"].is_string(), "{finished}");
    assert!(node["finished_at"].is_string(), "{finished}");
    assert!(node["job_created_at"].is_string(), "{finished}");
    assert_eq!(
        finished,
        workflow_run_shape(&finished, &team, &workflow_id, "Shape Workflow 2", &graph)
    );

    let (status, deleted, _) = call(
        &app,
        "DELETE",
        &format!("/api/v1/workflows/{workflow_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{deleted}");
    assert_eq!(deleted, Value::Null);
    let _ = std::fs::remove_dir_all(&harness.dir);
}

fn hook_shape(
    value: &Value,
    team: &str,
    name: &str,
    match_status: &str,
    template_id: &str,
) -> Value {
    json!({
        "id": value["id"].clone(),
        "team_id": team,
        "name": name,
        "token": value["token"].clone(),
        "match_alertname": "ShapeAlert",
        "match_status": match_status,
        "template_id": template_id,
        "workflow_id": null,
        "extra_vars": {"severity": "warning"},
        "enabled": true,
        "created_at": value["created_at"].clone(),
    })
}

fn dispatch_shape(value: &Value, summary: &str, workflow: bool) -> Value {
    let target = if workflow {
        let target = value["workflow_job_id"].clone();
        assert!(target.is_string(), "{value}");
        Value::Null
    } else {
        let target = value["job_id"].clone();
        assert!(target.is_string(), "{value}");
        target
    };
    json!({
        "status": "accepted",
        "summary": summary,
        "job_id": target,
        "workflow_job_id": if workflow { value["workflow_job_id"].clone() } else { Value::Null },
    })
}

#[tokio::test]
async fn hooks_and_deliveries_return_the_documented_shape() {
    let harness = TestApp::boot("work-hooks", Options::default());
    let app = harness.app.clone();
    let (cookie, team) = admin_team(&app, "Shape Hooks").await;
    let template_id = shell_template(&app, &cookie, &team, "Shape Hook", "echo hook").await;

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/v1/hooks",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Shape Hook",
            "match_alertname": "ShapeAlert",
            "match_status": "firing",
            "template_id": template_id,
            "extra_vars": {"severity": "warning"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let hook_id = created["id"].as_str().expect("hook id").to_string();
    let token = created["token"].as_str().expect("hook token").to_string();
    assert!(!token.is_empty());
    let mut expected = hook_shape(&created, &team, "Shape Hook", "firing", &template_id);
    assert_eq!(created, expected);

    let (status, list, _) = call(
        &app,
        "GET",
        "/api/v1/hooks",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list, json!([expected.clone()]));

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/v1/hooks/{hook_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Shape Hook 2",
            "match_alertname": "ShapeAlert",
            "match_status": "any",
            "template_id": template_id,
            "extra_vars": {"severity": "warning"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    expected = hook_shape(&updated, &team, "Shape Hook 2", "any", &template_id);
    assert_eq!(updated, expected);
    assert_eq!(updated["token"], token);

    let (status, tested, _) = call(
        &app,
        "POST",
        &format!("/api/v1/hooks/{hook_id}/test"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{tested}");
    assert_eq!(
        tested,
        dispatch_shape(
            &tested,
            "Test, run as admin: 1 alert(s) started, first ShapeAlert.",
            false
        )
    );

    let (status, deliveries, _) = call(
        &app,
        "GET",
        &format!("/api/v1/hooks/{hook_id}/deliveries"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deliveries}");
    let delivery = &deliveries[0];
    assert_eq!(
        deliveries,
        json!([{
            "id": delivery["id"].clone(),
            "hook_id": hook_id,
            "hook_name": "Shape Hook 2",
            "status": "accepted",
            "summary": "Test, run as admin: 1 alert(s) started, first ShapeAlert.",
            "job_id": tested["job_id"].clone(),
            "workflow_job_id": null,
            "created_at": delivery["created_at"].clone(),
        }])
    );

    let (status, ingested, _) = call(
        &app,
        "POST",
        &format!("/hooks/{hook_id}/{token}"),
        None,
        None,
        Some(json!({
            "status": "firing",
            "alerts": [{
                "status": "firing",
                "labels": {"alertname": "ShapeAlert"},
                "annotations": {},
            }],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ingested}");
    assert_eq!(
        ingested,
        dispatch_shape(&ingested, "1 alert(s) started, first ShapeAlert.", false)
    );

    let workflow_id = {
        let (status, workflow, _) = call(
            &app,
            "POST",
            "/api/v1/workflows",
            Some(&cookie),
            Some(&team),
            Some(json!({
                "name": "Shape Hook Workflow",
                "description": "",
                "graph": workflow_graph(&template_id),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{workflow}");
        workflow["id"].as_str().expect("workflow id").to_string()
    };
    let (status, workflow_hook, _) = call(
        &app,
        "POST",
        "/api/v1/hooks",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Shape Workflow Hook",
            "match_alertname": "ShapeAlert",
            "workflow_id": workflow_id,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow_hook}");
    let workflow_hook_id = workflow_hook["id"].as_str().expect("hook id");
    let (status, workflow_dispatch, _) = call(
        &app,
        "POST",
        &format!("/api/v1/hooks/{workflow_hook_id}/test"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{workflow_dispatch}");
    assert_eq!(
        workflow_dispatch,
        dispatch_shape(
            &workflow_dispatch,
            "Test, run as admin: 1 alert(s) started, first ShapeAlert.",
            true
        )
    );
    assert!(workflow_dispatch["job_id"].is_null(), "{workflow_dispatch}");
    assert!(
        workflow_dispatch["workflow_job_id"].is_string(),
        "{workflow_dispatch}"
    );

    let (status, deleted, _) = call(
        &app,
        "DELETE",
        &format!("/api/v1/hooks/{hook_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{deleted}");
    assert_eq!(deleted, Value::Null);
    let _ = std::fs::remove_dir_all(&harness.dir);
}
