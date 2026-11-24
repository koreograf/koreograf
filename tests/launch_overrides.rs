#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use common::{call, config, temp_dir, Options, TestApp};
use koreograf::LocalJobs;
use serde_json::json;

async fn shell_template(
    app: &TestApp,
    cookie: &str,
    team: &str,
    name: &str,
    prompt_on_launch: bool,
) -> String {
    let (status, template) = app
        .call(
            "POST",
            "/api/templates",
            Some(cookie),
            Some(team),
            Some(json!({
                "name": name,
                "tool": "shell",
                "entrypoint": "",
                "prompt_on_launch": prompt_on_launch,
                "extra_vars": {"keep": "template"},
                "spec": {"script": "true"}
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    template["id"].as_str().unwrap().to_string()
}

async fn cancel(app: &TestApp, cookie: &str, team: &str, job_id: &str) {
    let (status, body) = app
        .call(
            "POST",
            &format!("/api/jobs/{job_id}/cancel"),
            Some(cookie),
            Some(team),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn launch_extra_vars_follow_the_prompt_gate() {
    let app = TestApp::boot("launch-overrides", Options::default());
    let (cookie, team) = app.sign_in().await;
    let quiet = shell_template(&app, &cookie, &team, "Quiet", false).await;

    let (status, blocked) = app
        .call(
            "POST",
            &format!("/api/templates/{quiet}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({"extra_vars": {"from_launch": "yes"}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{blocked}");
    assert_eq!(blocked["error"], "prompt_on_launch_is_off");

    let (status, plain) = app
        .call(
            "POST",
            &format!("/api/templates/{quiet}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{plain}");
    assert_eq!(plain["parameters"]["extra_vars"]["keep"], "template");
    cancel(&app, &cookie, &team, plain["id"].as_str().unwrap()).await;

    let (status, empty) = app
        .call(
            "POST",
            &format!("/api/templates/{quiet}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({"extra_vars": {}})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{empty}");
    cancel(&app, &cookie, &team, empty["id"].as_str().unwrap()).await;

    let (status, options) = app
        .call(
            "POST",
            &format!("/api/templates/{quiet}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({
                "extra_vars": {"from_launch": "yes"},
                "launch_with_options": true
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{options}");
    assert_eq!(options["parameters"]["extra_vars"]["keep"], "template");
    assert_eq!(options["parameters"]["extra_vars"]["from_launch"], "yes");
    assert_eq!(options["parameters"]["launch_with_options"], true);
    cancel(&app, &cookie, &team, options["id"].as_str().unwrap()).await;

    let prompted = shell_template(&app, &cookie, &team, "Prompted", true).await;
    let (status, prompted_job) = app
        .call(
            "POST",
            &format!("/api/templates/{prompted}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({"extra_vars": {"from_launch": "yes"}})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{prompted_job}");
    assert_eq!(prompted_job["parameters"]["extra_vars"]["keep"], "template");
    assert_eq!(
        prompted_job["parameters"]["extra_vars"]["from_launch"],
        "yes"
    );
    cancel(&app, &cookie, &team, prompted_job["id"].as_str().unwrap()).await;

    let (status, stored) = app
        .call("GET", "/api/templates", Some(&cookie), Some(&team), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    let row = stored
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == quiet)
        .unwrap();
    assert_eq!(row["extra_vars"]["keep"], "template");
    assert!(row["extra_vars"].get("from_launch").is_none());
}

async fn tofu_template(
    app: &TestApp,
    cookie: &str,
    team: &str,
    name: &str,
    prompt_on_launch: bool,
    action: &str,
) -> String {
    let (status, template) = app
        .call(
            "POST",
            "/api/templates",
            Some(cookie),
            Some(team),
            Some(json!({
                "name": name,
                "tool": "opentofu",
                "entrypoint": "infra",
                "prompt_on_launch": prompt_on_launch,
                "extra_vars": {},
                "spec": {"action": action}
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    template["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn launch_can_pick_plan_or_apply_for_opentofu_only() {
    let app = TestApp::boot("launch-action", Options::default());
    let (cookie, team) = app.sign_in().await;
    let prompted = tofu_template(&app, &cookie, &team, "Prompted tofu", true, "plan").await;

    let (status, applied) = app
        .call(
            "POST",
            &format!("/api/templates/{prompted}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({"extra_vars": {}, "spec": {"action": "apply"}})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{applied}");
    assert_eq!(applied["parameters"]["spec"]["action"], "apply");
    cancel(&app, &cookie, &team, applied["id"].as_str().unwrap()).await;

    let (status, replanned) = app
        .call(
            "POST",
            &format!("/api/templates/{prompted}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({"extra_vars": {}, "spec": {"action": "plan"}})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{replanned}");
    assert_eq!(replanned["parameters"]["spec"]["action"], "plan");
    cancel(&app, &cookie, &team, replanned["id"].as_str().unwrap()).await;

    let (status, invalid) = app
        .call(
            "POST",
            &format!("/api/templates/{prompted}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({"extra_vars": {}, "spec": {"action": "destroy"}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid}");
    assert_eq!(invalid["error"], "action_is_plan_or_apply");

    let quiet = tofu_template(&app, &cookie, &team, "Quiet tofu", false, "plan").await;
    let (status, blocked) = app
        .call(
            "POST",
            &format!("/api/templates/{quiet}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({"extra_vars": {}, "spec": {"action": "apply"}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{blocked}");
    assert_eq!(blocked["error"], "prompt_on_launch_is_off");

    let (status, options) = app
        .call(
            "POST",
            &format!("/api/templates/{quiet}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({
                "extra_vars": {},
                "spec": {"action": "apply"},
                "launch_with_options": true
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{options}");
    assert_eq!(options["parameters"]["spec"]["action"], "apply");
    cancel(&app, &cookie, &team, options["id"].as_str().unwrap()).await;

    let (status, mixed) = app
        .call(
            "POST",
            &format!("/api/templates/{prompted}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({"extra_vars": {}, "spec": {"action": "apply", "limit": "web"}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{mixed}");
    assert_eq!(mixed["error"], "launch_fields_are_only_for_ansible");

    let shell = shell_template(&app, &cookie, &team, "Prompted shell", true).await;
    for spec in [json!({"action": "apply"}), json!({"action": "plan"})] {
        let (status, denied) = app
            .call(
                "POST",
                &format!("/api/templates/{shell}/launch"),
                Some(&cookie),
                Some(&team),
                Some(json!({"extra_vars": {}, "spec": spec})),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
        assert_eq!(denied["error"], "launch_fields_are_only_for_ansible");
    }

    let (status, ansible) = app
        .call(
            "POST",
            "/api/templates",
            Some(&cookie),
            Some(&team),
            Some(json!({
                "name": "Prompted ansible",
                "tool": "ansible",
                "entrypoint": "site.yml",
                "prompt_on_launch": true,
                "extra_vars": {},
                "spec": {"job_type": "run"}
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{ansible}");
    let ansible_id = ansible["id"].as_str().unwrap();
    let (status, denied) = app
        .call(
            "POST",
            &format!("/api/templates/{ansible_id}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({"extra_vars": {}, "spec": {"action": "apply"}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    assert_eq!(denied["error"], "launch_fields_are_only_for_ansible");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompted_opentofu_action_stays_a_template_launch_for_local_admins() {
    let dir = temp_dir("tofu-action-local");
    let mut cfg = config(&dir, &Options::default());
    cfg.local_jobs = LocalJobs::Admins;
    let state = koreograf::boot(cfg).expect("boot");
    let app = koreograf::router(state);
    let (cookie, team) = common::sign_in(&app).await;
    let team = team.as_str();

    let (status, template) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Local tofu",
            "tool": "opentofu",
            "entrypoint": "infra",
            "no_environment": true,
            "prompt_on_launch": true,
            "extra_vars": {},
            "spec": {"action": "plan"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();

    let (status, launched) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(team),
        Some(json!({"extra_vars": {}, "spec": {"action": "apply"}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");
    assert_eq!(launched["parameters"]["spec"]["action"], "apply");
    let (status, canceled) = call(
        &app,
        "POST",
        &format!("/api/jobs/{}/cancel", launched["id"].as_str().unwrap()),
        Some(&cookie),
        Some(team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    let _ = std::fs::remove_dir_all(&dir);
}

async fn ansible_template(
    app: &TestApp,
    cookie: &str,
    team: &str,
    name: &str,
    prompt_on_launch: bool,
    prompt_fields: &[&str],
    spec: serde_json::Value,
) -> serde_json::Value {
    let (status, inventory) = app
        .call(
            "POST",
            "/api/inventories",
            Some(cookie),
            Some(team),
            Some(json!({
                "name": format!("{name} hosts"),
                "body": "[web]\nweb-1\n"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap();
    let (status, template) = app
        .call(
            "POST",
            "/api/templates",
            Some(cookie),
            Some(team),
            Some(json!({
                "name": name,
                "tool": "ansible",
                "entrypoint": "site.yml",
                "inventory_id": inventory_id,
                "prompt_on_launch": prompt_on_launch,
                "prompt_fields": prompt_fields,
                "extra_vars": {"keep": "template"},
                "spec": spec
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    template
}

#[tokio::test]
async fn a_partial_prompt_accepts_only_the_listed_field() {
    let app = TestApp::boot("partial-prompt", Options::default());
    let (cookie, team) = app.sign_in().await;
    let template = ansible_template(
        &app,
        &cookie,
        &team,
        "Partial prompt",
        false,
        &["limit"],
        json!({"job_type": "run", "tags": "ping", "limit": "web"}),
    )
    .await;
    let template_id = template["id"].as_str().unwrap().to_string();
    assert_eq!(template["prompt_fields"], json!(["limit"]));
    assert_eq!(template["prompt_on_launch"], false);

    let (status, launched) = app
        .call(
            "POST",
            &format!("/api/templates/{template_id}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({"spec": {"limit": "db"}})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");
    assert_eq!(launched["parameters"]["spec"]["limit"], "db");
    assert_eq!(launched["parameters"]["spec"]["tags"], "ping");
    assert_eq!(launched["parameters"]["spec"]["job_type"], "run");
    assert_eq!(launched["parameters"]["extra_vars"]["keep"], "template");
    cancel(&app, &cookie, &team, launched["id"].as_str().unwrap()).await;

    for body in [
        json!({"inventory_id": "missing"}),
        json!({"credential_ids": ["missing"]}),
        json!({"extra_vars": {"from_launch": "yes"}}),
        json!({"spec": {"tags": "db"}}),
        json!({"spec": {"skip_tags": "slow"}}),
        json!({"spec": {"job_type": "check"}}),
        json!({"spec": {"verbosity": 2}}),
        json!({"spec": {"limit": "db", "tags": "db"}}),
    ] {
        let (status, denied) = app
            .call(
                "POST",
                &format!("/api/templates/{template_id}/launch"),
                Some(&cookie),
                Some(&team),
                Some(body.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {denied}");
        assert_eq!(denied["error"], "field_is_not_prompted", "{body}");
    }

    for body in [json!({}), json!({"extra_vars": {}})] {
        let (status, plain) = app
            .call(
                "POST",
                &format!("/api/templates/{template_id}/launch"),
                Some(&cookie),
                Some(&team),
                Some(body.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}: {plain}");
        assert_eq!(plain["parameters"]["spec"]["limit"], "web", "{body}");
        assert_eq!(plain["parameters"]["spec"]["tags"], "ping", "{body}");
        cancel(&app, &cookie, &team, plain["id"].as_str().unwrap()).await;
    }

    let (status, stored) = app
        .call("GET", "/api/templates", Some(&cookie), Some(&team), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    let row = stored
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == template_id)
        .unwrap();
    assert_eq!(row["prompt_on_launch"], false);
    assert_eq!(row["prompt_fields"], json!(["limit"]));
    assert_eq!(row["spec"]["limit"], "web");
    assert_eq!(row["spec"]["tags"], "ping");
    assert!(row["extra_vars"].get("from_launch").is_none());
}

#[tokio::test]
async fn prompt_field_lists_drop_unknown_and_mismatched_keys() {
    let app = TestApp::boot("prompt-field-fit", Options::default());
    let (cookie, team) = app.sign_in().await;

    let ansible = ansible_template(
        &app,
        &cookie,
        &team,
        "Fit ansible",
        false,
        &["action", "forks", "bogus", "tags", "limit", "limit"],
        json!({}),
    )
    .await;
    assert_eq!(ansible["prompt_fields"], json!(["limit", "tags"]));

    let (status, shell) = app
        .call(
            "POST",
            "/api/templates",
            Some(&cookie),
            Some(&team),
            Some(json!({
                "name": "Fit shell",
                "tool": "shell",
                "prompt_fields": ["limit", "inventory", "extra_vars", "credentials"],
                "spec": {"script": "true"}
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{shell}");
    assert_eq!(shell["prompt_fields"], json!(["credentials", "extra_vars"]));

    let (status, tofu) = app
        .call(
            "POST",
            "/api/templates",
            Some(&cookie),
            Some(&team),
            Some(json!({
                "name": "Fit tofu",
                "tool": "opentofu",
                "entrypoint": "infra",
                "prompt_fields": ["action", "inventory", "tags", "credentials"],
                "spec": {"action": "plan"}
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{tofu}");
    assert_eq!(tofu["prompt_fields"], json!(["credentials", "action"]));
}

#[tokio::test]
async fn prompt_on_launch_and_options_still_accept_every_field() {
    let app = TestApp::boot("prompt-all-fields", Options::default());
    let (cookie, team) = app.sign_in().await;

    let (status, inventory) = app
        .call(
            "POST",
            "/api/inventories",
            Some(&cookie),
            Some(&team),
            Some(json!({"name": "Prompt hosts", "body": "[web]\nweb-1\n"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap().to_string();
    let (status, credential) = app
        .call(
            "POST",
            "/api/credentials",
            Some(&cookie),
            Some(&team),
            Some(json!({
                "name": "Prompt key",
                "kind": "ssh",
                "secret": {"username": "deploy", "password": "prompt-pass"}
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"].as_str().unwrap().to_string();

    let prompted = ansible_template(
        &app,
        &cookie,
        &team,
        "Prompt all",
        true,
        &[],
        json!({"job_type": "run", "limit": "web"}),
    )
    .await;
    let prompted_id = prompted["id"].as_str().unwrap();
    let (status, launched) = app
        .call(
            "POST",
            &format!("/api/templates/{prompted_id}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({
                "inventory_id": inventory_id,
                "credential_ids": [credential_id],
                "extra_vars": {"from_launch": "yes"},
                "spec": {
                    "job_type": "check",
                    "tags": "heal",
                    "skip_tags": "slow",
                    "limit": "db",
                    "verbosity": 2
                }
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");
    assert_eq!(launched["parameters"]["inventory_id"], inventory_id);
    assert_eq!(
        launched["parameters"]["credentials"][0]["id"],
        credential_id
    );
    assert_eq!(launched["parameters"]["extra_vars"]["from_launch"], "yes");
    assert_eq!(launched["parameters"]["spec"]["limit"], "db");
    assert_eq!(launched["parameters"]["spec"]["tags"], "heal");
    assert_eq!(launched["parameters"]["spec"]["job_type"], "check");
    assert_eq!(launched["parameters"]["spec"]["verbosity"], 2);
    cancel(&app, &cookie, &team, launched["id"].as_str().unwrap()).await;

    let partial = ansible_template(
        &app,
        &cookie,
        &team,
        "Options partial",
        false,
        &["limit"],
        json!({"job_type": "run", "limit": "web"}),
    )
    .await;
    let partial_id = partial["id"].as_str().unwrap();
    let (status, options) = app
        .call(
            "POST",
            &format!("/api/templates/{partial_id}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({
                "inventory_id": inventory_id,
                "extra_vars": {"from_launch": "yes"},
                "spec": {"tags": "heal", "limit": "db"},
                "launch_with_options": true
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{options}");
    assert_eq!(options["parameters"]["launch_with_options"], true);
    assert_eq!(options["parameters"]["inventory_id"], inventory_id);
    assert_eq!(options["parameters"]["spec"]["tags"], "heal");
    assert_eq!(options["parameters"]["spec"]["limit"], "db");
    cancel(&app, &cookie, &team, options["id"].as_str().unwrap()).await;
}

#[tokio::test]
async fn a_hook_still_merges_extra_vars_without_a_prompt_flag() {
    let app = TestApp::boot("hook-extra-vars-prompt", Options::default());
    let (cookie, team) = app.sign_in().await;
    let template = ansible_template(
        &app,
        &cookie,
        &team,
        "Hook prompt",
        false,
        &["limit"],
        json!({"job_type": "run", "limit": "web"}),
    )
    .await;
    let template_id = template["id"].as_str().unwrap().to_string();

    let (status, hook) = app
        .call(
            "POST",
            "/api/hooks",
            Some(&cookie),
            Some(&team),
            Some(json!({
                "name": "Prompt hook",
                "match_alertname": "DiskSpaceLow",
                "template_id": template_id,
                "extra_vars": {"from_hook": "yes"}
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");
    let hook_id = hook["id"].as_str().unwrap();
    let token = hook["token"].as_str().unwrap();

    let (status, accepted) = app
        .call(
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
    let job_id = accepted["job_id"].as_str().expect("hook job");
    let (status, job) = app
        .call(
            "GET",
            &format!("/api/jobs/{job_id}"),
            Some(&cookie),
            Some(&team),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{job}");
    assert_eq!(job["parameters"]["extra_vars"]["keep"], "template");
    assert_eq!(job["parameters"]["extra_vars"]["from_hook"], "yes");
    assert_eq!(job["parameters"]["spec"]["limit"], "web");
    cancel(&app, &cookie, &team, job_id).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_partial_prompt_stays_a_template_launch_for_local_admins() {
    let dir = temp_dir("partial-prompt-local");
    let mut cfg = config(&dir, &Options::default());
    cfg.local_jobs = LocalJobs::Admins;
    let state = koreograf::boot(cfg).expect("boot");
    let app = koreograf::router(state);
    let (cookie, team) = common::sign_in(&app).await;
    let team = team.as_str();

    let (status, inventory) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(team),
        Some(json!({"name": "Local hosts", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap();

    let (status, template) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Local partial",
            "tool": "ansible",
            "entrypoint": "site.yml",
            "inventory_id": inventory_id,
            "no_environment": true,
            "prompt_fields": ["tags"],
            "spec": {"job_type": "run", "tags": "ping"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();

    let (status, launched) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(team),
        Some(json!({"spec": {"tags": "other"}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");
    assert_eq!(launched["parameters"]["spec"]["tags"], "other");
    let (status, canceled) = call(
        &app,
        "POST",
        &format!("/api/jobs/{}/cancel", launched["id"].as_str().unwrap()),
        Some(&cookie),
        Some(team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    let _ = std::fs::remove_dir_all(&dir);
}
