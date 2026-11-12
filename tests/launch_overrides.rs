#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use common::{Options, TestApp};
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
