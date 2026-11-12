#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use chrono::Months;
use common::{Options, TestApp};
use serde_json::{json, Value};

fn schedule(start_date: &str) -> Value {
    json!({
        "name": "Bound",
        "timezone": "Europe/Berlin",
        "start_date": start_date,
        "start_time": "06:00",
        "frequency": "once",
        "end_kind": "never"
    })
}

#[tokio::test]
async fn a_start_more_than_ten_years_ahead_is_rejected() {
    let app = TestApp::boot("schedule-bound", Options::default());
    let (cookie, team) = app.sign_in().await;

    let (status, template) = app
        .call(
            "POST",
            "/api/templates",
            Some(&cookie),
            Some(&team),
            Some(json!({
                "name": "Scheduled",
                "tool": "shell",
                "update_on_launch": false,
                "spec": {"script": "echo scheduled"}
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();
    let schedules = format!("/api/templates/{template_id}/schedules");

    let today = chrono::Utc::now()
        .with_timezone(&chrono_tz::Europe::Berlin)
        .date_naive();
    let limit = today
        .checked_add_months(Months::new(120))
        .expect("ten years");
    let exact = limit.format("%Y-%m-%d").to_string();
    let too_far = limit
        .succ_opt()
        .expect("next day")
        .format("%Y-%m-%d")
        .to_string();

    let (status, far) = app
        .call(
            "POST",
            &schedules,
            Some(&cookie),
            Some(&team),
            Some(schedule("9999-12-31")),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{far}");
    assert_eq!(far["error"], "schedule_date_is_too_far_in_the_future");

    let (status, edge) = app
        .call(
            "POST",
            &schedules,
            Some(&cookie),
            Some(&team),
            Some(schedule(&too_far)),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{edge}");
    assert_eq!(edge["error"], "schedule_date_is_too_far_in_the_future");

    let (status, accepted) = app
        .call(
            "POST",
            &schedules,
            Some(&cookie),
            Some(&team),
            Some(schedule(&exact)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{accepted}");
    let schedule_id = accepted["id"].as_str().unwrap().to_string();

    let (status, moved) = app
        .call(
            "PUT",
            &format!("/api/schedules/{schedule_id}"),
            Some(&cookie),
            Some(&team),
            Some(schedule("9999-12-31")),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{moved}");
    assert_eq!(moved["error"], "schedule_date_is_too_far_in_the_future");

    let mut preview = schedule("9999-12-31");
    preview.as_object_mut().unwrap().remove("name");
    let (status, preview) = app
        .call(
            "POST",
            "/api/schedules/preview",
            Some(&cookie),
            Some(&team),
            Some(preview),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{preview}");
    assert_eq!(preview["error"], "schedule_date_is_too_far_in_the_future");
}
