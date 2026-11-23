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
