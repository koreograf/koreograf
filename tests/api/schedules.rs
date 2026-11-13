use axum::http::StatusCode;
use koreograf::LocalJobs;
use serde_json::{json, Value};

use crate::common::call_cookie as call;

use crate::common::*;

#[tokio::test]
async fn a_schedule_runs_the_last_miss_once_and_lists_on_its_target() {
    let harness = TestApp::boot("schedule", Options::default());
    let state = harness.state.clone();
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/main.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
    assert!(script.contains("schedule-card"));
    assert!(script.contains("id: \"schedule-form\""));
    assert!(script.contains("#/schedules"));
    assert!(script.contains("class=\"switch\""));
    let (status, style, _) = call(&app, "GET", "/app.css", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(style.as_str().unwrap_or("").contains("schedule-miss"));
    assert!(style
        .as_str()
        .unwrap_or("")
        .contains(".switch input:checked"));

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
            "name": "Scheduled",
            "tool": "shell",
            "update_on_launch": false,
            "spec": {"script": "echo scheduled"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Morning")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["next_run_at"], "2027-06-01T04:00:00Z");
    assert_eq!(created["frequency"], "daily");
    assert_eq!(created["enabled"], true);
    assert!(created["missed_at"].is_null());
    let schedule_id = created["id"].as_str().unwrap().to_string();

    let (status, list, _) = call(
        &app,
        "GET",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list.as_array().unwrap().len(), 1);

    let mut preview_body = daily_schedule("ignored");
    preview_body
        .as_object_mut()
        .expect("schedule object")
        .remove("name");
    let (status, preview, _) = call(
        &app,
        "POST",
        "/api/schedules/preview",
        Some(&cookie),
        Some(&team),
        Some(preview_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let occurrences = preview["occurrences"].as_array().unwrap();
    assert_eq!(occurrences.len(), 5);
    assert_eq!(occurrences[0], "2027-06-01T04:00:00Z");
    assert_eq!(occurrences[1], "2027-06-02T04:00:00Z");

    let (status, zone, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Unknown zone",
            "timezone": "Mars/Olympus",
            "start_date": "2027-06-01",
            "start_time": "06:00",
            "frequency": "daily",
            "end_kind": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{zone}");
    assert_eq!(zone["error"], "schedule_needs_a_known_time_zone");

    let (status, past, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Already over",
            "timezone": "Europe/Berlin",
            "start_date": "2020-01-01",
            "start_time": "06:00",
            "frequency": "once",
            "end_kind": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{past}");
    assert_eq!(past["error"], "schedule_date_is_too_far_in_the_past");

    let (status, past_preview, _) = call(
        &app,
        "POST",
        "/api/schedules/preview",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "timezone": "Europe/Berlin",
            "start_date": "2020-01-01",
            "start_time": "06:00",
            "frequency": "once",
            "end_kind": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{past_preview}");
    assert_eq!(
        past_preview["error"],
        "schedule_date_is_too_far_in_the_past"
    );

    set_start_date(&state, &schedule_id, "2020-01-01");
    let (status, kept, _) = call(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Morning kept",
            "timezone": "Europe/Berlin",
            "start_date": "2020-01-01",
            "start_time": "07:00",
            "frequency": "daily",
            "end_kind": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["name"], "Morning kept");
    assert_eq!(kept["start_date"], "2020-01-01");
    assert_eq!(kept["start_time"], "07:00");
    let (status, moved, _) = call(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Morning moved",
            "timezone": "Europe/Berlin",
            "start_date": "2019-12-31",
            "start_time": "07:00",
            "frequency": "daily",
            "end_kind": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{moved}");
    assert_eq!(moved["error"], "schedule_date_is_too_far_in_the_past");
    let (status, still, _) = call(
        &app,
        "GET",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{still}");
    let kept_row = still
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"].as_str() == Some(schedule_id.as_str()))
        .expect("aged schedule");
    assert_eq!(kept_row["name"], "Morning kept");
    assert_eq!(kept_row["start_date"], "2020-01-01");
    let (status, restored, _) = call(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Morning")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(restored["name"], "Morning");
    assert_eq!(restored["start_date"], "2027-06-01");
    assert_eq!(restored["next_run_at"], "2027-06-01T04:00:00Z");

    let yesterday = chrono::Utc::now()
        .date_naive()
        .pred_opt()
        .expect("yesterday")
        .format("%Y-%m-%d")
        .to_string();
    let (status, recent_past, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Yesterday once",
            "timezone": "UTC",
            "start_date": yesterday,
            "start_time": "00:00",
            "frequency": "once",
            "end_kind": "never"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{recent_past}");
    assert_eq!(recent_past["error"], "schedule_has_no_occurrence");

    let (status, duplicate, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Morning")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{duplicate}");
    assert_eq!(duplicate["error"], "that_name_is_already_used_in_this_team");

    mark_missed(&state, &schedule_id);
    let (status, paused, _) = call(
        &app,
        "POST",
        &format!("/api/schedules/{schedule_id}/enabled"),
        Some(&cookie),
        Some(&team),
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{paused}");
    assert_eq!(paused["enabled"], false);
    assert_eq!(paused["missed_at"], "2026-10-21T04:00:00Z");
    let (status, resumed, _) = call(
        &app,
        "POST",
        &format!("/api/schedules/{schedule_id}/enabled"),
        Some(&cookie),
        Some(&team),
        Some(json!({"enabled": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resumed}");
    assert_eq!(resumed["enabled"], true);
    assert_eq!(resumed["missed_at"], "2026-10-21T04:00:00Z");
    assert_eq!(resumed["next_run_at"], "2027-06-01T04:00:00Z");

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Morning later")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "Morning later");
    assert!(updated["missed_at"].is_null());

    mark_missed(&state, &schedule_id);
    let (status, accepted, _) = call(
        &app,
        "POST",
        &format!("/api/schedules/{schedule_id}/missed"),
        Some(&cookie),
        Some(&team),
        Some(json!({"accept": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert!(accepted["missed_at"].is_null());
    let (status, jobs, _) = call(&app, "GET", "/api/jobs", Some(&cookie), Some(&team), None).await;
    assert_eq!(status, StatusCode::OK, "{jobs}");
    let started = scheduled_jobs(&jobs, &template_id);
    assert_eq!(started.len(), 1);
    assert_eq!(started[0]["trigger"], "schedule");
    assert_eq!(started[0]["launched_by"], "admin");
    let job_id = started[0]["id"].as_str().unwrap().to_string();
    let finished = wait_for_job(&app, &cookie, &team, &job_id).await;
    assert_eq!(finished["status"], "successful", "{}", finished["events"]);
    assert!(has_line(&finished, "stdout", "scheduled"));

    let (status, again, _) = call(
        &app,
        "POST",
        &format!("/api/schedules/{schedule_id}/missed"),
        Some(&cookie),
        Some(&team),
        Some(json!({"accept": true})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{again}");
    assert_eq!(again["error"], "schedule_has_no_missed_run");

    mark_missed(&state, &schedule_id);
    let missed_path = format!("/api/schedules/{schedule_id}/missed");
    let left = call(
        &app,
        "POST",
        &missed_path,
        Some(&cookie),
        Some(&team),
        Some(json!({"accept": true})),
    );
    let right = call(
        &app,
        "POST",
        &missed_path,
        Some(&cookie),
        Some(&team),
        Some(json!({"accept": true})),
    );
    let ((left_status, left_body, _), (right_status, right_body, _)) = tokio::join!(left, right);
    let statuses = [left_status, right_status];
    assert!(
        statuses.contains(&StatusCode::OK),
        "one accept should run: {left_body} {right_body}"
    );
    assert!(
        statuses.contains(&StatusCode::BAD_REQUEST),
        "the other accept should be refused: {left_body} {right_body}"
    );
    let errors = [left_body["error"].as_str(), right_body["error"].as_str()];
    assert!(errors.contains(&Some("schedule_has_no_missed_run")));
    let (status, jobs, _) = call(&app, "GET", "/api/jobs", Some(&cookie), Some(&team), None).await;
    assert_eq!(status, StatusCode::OK, "{jobs}");
    let ids: Vec<String> = scheduled_jobs(&jobs, &template_id)
        .iter()
        .map(|job| job["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids.len(), 2);
    for id in ids {
        let finished = wait_for_job(&app, &cookie, &team, &id).await;
        assert_eq!(finished["status"], "successful", "{}", finished["events"]);
    }

    mark_missed(&state, &schedule_id);
    let (status, declined, _) = call(
        &app,
        "POST",
        &format!("/api/schedules/{schedule_id}/missed"),
        Some(&cookie),
        Some(&team),
        Some(json!({"accept": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{declined}");
    assert!(declined["missed_at"].is_null());
    let (status, jobs, _) = call(&app, "GET", "/api/jobs", Some(&cookie), Some(&team), None).await;
    assert_eq!(status, StatusCode::OK, "{jobs}");
    assert_eq!(scheduled_jobs(&jobs, &template_id).len(), 2);

    let (status, workflow, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Scheduled flow", "graph": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let workflow_id = workflow["id"].as_str().unwrap().to_string();
    let (status, weekly, _) = call(
        &app,
        "POST",
        &format!("/api/workflows/{workflow_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Weekdays",
            "timezone": "Europe/Berlin",
            "start_date": "2027-06-01",
            "start_time": "06:00",
            "frequency": "weekly",
            "weekdays": ["FR", "MO"],
            "end_kind": "after",
            "end_count": 3
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{weekly}");
    assert_eq!(weekly["weekdays"], json!(["MO", "FR"]));
    assert_eq!(weekly["end_kind"], "after");
    assert_eq!(weekly["end_count"], 3);
    assert_eq!(weekly["next_run_at"], "2027-06-04T04:00:00Z");
    let (status, flows, _) = call(
        &app,
        "GET",
        &format!("/api/workflows/{workflow_id}/schedules"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{flows}");
    assert_eq!(flows.as_array().unwrap().len(), 1);

    let (status, all, _) = call(
        &app,
        "GET",
        "/api/schedules",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{all}");
    let rows = all.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let morning = rows
        .iter()
        .find(|row| row["name"] == "Morning later")
        .unwrap();
    assert_eq!(morning["target_kind"], "template");
    assert_eq!(morning["target_id"], template_id);
    assert_eq!(morning["target_name"], "Scheduled");
    let weekdays = rows.iter().find(|row| row["name"] == "Weekdays").unwrap();
    assert_eq!(weekdays["target_kind"], "workflow");
    assert_eq!(weekdays["target_id"], workflow_id);
    assert_eq!(weekdays["target_name"], "Scheduled flow");

    let (status, gone, _) = call(
        &app,
        "DELETE",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{gone}");
    let left: i64 = {
        let db = state.db().expect("database");
        db.query_row(
            "SELECT COUNT(*) FROM schedules WHERE target_id = ?1",
            [template_id.as_str()],
            |row| row.get(0),
        )
        .expect("count")
    };
    assert_eq!(left, 0i64);
    let (status, remaining, _) = call(
        &app,
        "GET",
        "/api/schedules",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{remaining}");
    assert_eq!(remaining.as_array().unwrap().len(), 1);
    assert_eq!(remaining[0]["name"], "Weekdays");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_failed_missed_run_stays_visible() {
    let dir = temp_dir("missed-keep");
    let mut cfg = config(&dir, &Options::default());
    cfg.local_jobs = LocalJobs::Admins;
    let state = koreograf::boot(cfg).unwrap();
    let app = koreograf::router(state.clone());

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
    let (status, _, operator_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "operator", "password": "operator-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let operator_cookie = operator_cookie.expect("operator cookie");
    let (status, created, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{health_id}/schedules"),
        Some(&operator_cookie),
        Some(&team),
        Some(daily_schedule("Operator miss")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let schedule_id = created["id"].as_str().unwrap();
    mark_missed(&state, schedule_id);
    let (status, denied, _) = call(
        &app,
        "POST",
        &format!("/api/schedules/{schedule_id}/missed"),
        Some(&operator_cookie),
        Some(&team),
        Some(json!({"accept": true})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    assert_eq!(denied["error"], "local_runs_are_limited_to_administrators");
    let (status, listed, _) = call(
        &app,
        "GET",
        &format!("/api/templates/{health_id}/schedules"),
        Some(&operator_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let row = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == schedule_id)
        .expect("schedule");
    assert_eq!(row["missed_at"], "2026-10-21T04:00:00Z");
    let (status, jobs, _) = call(
        &app,
        "GET",
        "/api/jobs",
        Some(&operator_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{jobs}");
    assert!(scheduled_jobs(&jobs, &health_id).is_empty(), "{jobs}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_edit_records_the_user_who_changed_the_schedule() {
    let harness = TestApp::boot("schedule-editor", Options::default());
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
    let (status, created, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{health_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Admin clock")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["created_by"], "admin");
    assert_eq!(created["updated_by"], "admin");
    let schedule_id = created["id"].as_str().unwrap();
    let (status, _, operator_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "operator", "password": "operator-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let operator_cookie = operator_cookie.expect("operator cookie");
    let (status, edited, _) = call(
        &app,
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(&operator_cookie),
        Some(&team),
        Some(daily_schedule("Operator clock")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["name"], "Operator clock");
    assert_eq!(edited["created_by"], "admin");
    assert_eq!(edited["updated_by"], "operator");
    let (status, listed, _) = call(
        &app,
        "GET",
        &format!("/api/templates/{health_id}/schedules"),
        Some(&operator_cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let row = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == schedule_id)
        .expect("schedule");
    assert_eq!(row["created_by"], "admin");
    assert_eq!(row["updated_by"], "operator");
    let _ = std::fs::remove_dir_all(&dir);
}

#[allow(clippy::too_many_arguments)]
fn daily_shape(
    value: &Value,
    name: &str,
    target_kind: &str,
    target_id: &str,
    enabled: bool,
    next_run_at: Value,
    missed_at: Value,
    created_by: &str,
    updated_by: &str,
) -> Value {
    json!({
        "id": value["id"].clone(),
        "name": name,
        "target_kind": target_kind,
        "target_id": target_id,
        "timezone": "Europe/Berlin",
        "start_date": "2027-06-01",
        "start_time": "06:00",
        "frequency": "daily",
        "weekdays": [],
        "end_kind": "never",
        "end_count": null,
        "end_date": null,
        "enabled": enabled,
        "next_run_at": next_run_at,
        "missed_at": missed_at,
        "created_by": created_by,
        "updated_by": updated_by,
        "launch_block": null,
    })
}

#[tokio::test]
async fn schedule_endpoints_return_the_documented_shape() {
    let harness = TestApp::boot("schedule-shape", Options::default());
    let state = harness.state.clone();
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
            "name": "Scheduled shape",
            "tool": "shell",
            "update_on_launch": false,
            "spec": {"script": "echo scheduled"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        &format!("/api/v1/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Morning")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let schedule_id = created["id"].as_str().unwrap().to_string();
    let expected = daily_shape(
        &created,
        "Morning",
        "template",
        &template_id,
        true,
        json!("2027-06-01T04:00:00Z"),
        Value::Null,
        "admin",
        "admin",
    );
    assert_eq!(created, expected);

    let (status, listed, _) = call(
        &app,
        "GET",
        &format!("/api/v1/templates/{template_id}/schedules"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed, json!([expected]));

    let (status, team_list, _) = call(
        &app,
        "GET",
        "/api/v1/schedules",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{team_list}");
    let mut team_expected = expected.clone();
    team_expected["target_name"] = json!("Scheduled shape");
    assert_eq!(team_list, json!([team_expected]));

    let mut preview_body = daily_schedule("ignored");
    preview_body
        .as_object_mut()
        .expect("schedule object")
        .remove("name");
    let (status, preview, _) = call(
        &app,
        "POST",
        "/api/v1/schedules/preview",
        Some(&cookie),
        Some(&team),
        Some(preview_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(
        preview,
        json!({
            "occurrences": [
                "2027-06-01T04:00:00Z",
                "2027-06-02T04:00:00Z",
                "2027-06-03T04:00:00Z",
                "2027-06-04T04:00:00Z",
                "2027-06-05T04:00:00Z",
            ],
        })
    );

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/v1/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        Some(daily_schedule("Morning later")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    let expected = daily_shape(
        &updated,
        "Morning later",
        "template",
        &template_id,
        true,
        json!("2027-06-01T04:00:00Z"),
        Value::Null,
        "admin",
        "admin",
    );
    assert_eq!(updated, expected);

    let (status, paused, _) = call(
        &app,
        "POST",
        &format!("/api/v1/schedules/{schedule_id}/enabled"),
        Some(&cookie),
        Some(&team),
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{paused}");
    let expected = daily_shape(
        &paused,
        "Morning later",
        "template",
        &template_id,
        false,
        json!("2027-06-01T04:00:00Z"),
        Value::Null,
        "admin",
        "admin",
    );
    assert_eq!(paused, expected);

    mark_missed(&state, &schedule_id);
    let (status, declined, _) = call(
        &app,
        "POST",
        &format!("/api/v1/schedules/{schedule_id}/missed"),
        Some(&cookie),
        Some(&team),
        Some(json!({"accept": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{declined}");
    let expected = daily_shape(
        &declined,
        "Morning later",
        "template",
        &template_id,
        false,
        json!("2027-06-01T04:00:00Z"),
        Value::Null,
        "admin",
        "admin",
    );
    assert_eq!(declined, expected);

    let (status, workflow, _) = call(
        &app,
        "POST",
        "/api/v1/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Scheduled shape flow", "graph": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let workflow_id = workflow["id"].as_str().unwrap().to_string();
    let (status, weekly, _) = call(
        &app,
        "POST",
        &format!("/api/v1/workflows/{workflow_id}/schedules"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Weekdays",
            "timezone": "Europe/Berlin",
            "start_date": "2027-06-01",
            "start_time": "06:00",
            "frequency": "weekly",
            "weekdays": ["FR", "MO"],
            "end_kind": "after",
            "end_count": 3
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{weekly}");
    let expected_weekly = json!({
        "id": weekly["id"].clone(),
        "name": "Weekdays",
        "target_kind": "workflow",
        "target_id": workflow_id,
        "timezone": "Europe/Berlin",
        "start_date": "2027-06-01",
        "start_time": "06:00",
        "frequency": "weekly",
        "weekdays": ["MO", "FR"],
        "end_kind": "after",
        "end_count": 3,
        "end_date": null,
        "enabled": true,
        "next_run_at": "2027-06-04T04:00:00Z",
        "missed_at": null,
        "created_by": "admin",
        "updated_by": "admin",
        "launch_block": null,
    });
    assert_eq!(weekly, expected_weekly);

    let (status, workflow_list, _) = call(
        &app,
        "GET",
        &format!("/api/v1/workflows/{workflow_id}/schedules"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{workflow_list}");
    assert_eq!(workflow_list, json!([expected_weekly]));

    let (status, removed, _) = call(
        &app,
        "DELETE",
        &format!("/api/v1/schedules/{schedule_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{removed}");
    assert_eq!(removed, Value::Null);

    let _ = std::fs::remove_dir_all(&dir);
}
