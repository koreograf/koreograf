use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use serde_json::json;

use crate::common::call_cookie as call;

use crate::common::*;

#[tokio::test]
async fn an_empty_template_entry_path_is_rejected_and_marked_in_the_form() {
    let harness = TestApp::boot("entry", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/features/legacy.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
    assert!(script.contains("for_example_site_yml"));
    assert!(script.contains("id=\"entrypoint-error\""));
    assert!(script.contains("form.entrypoint.value = books[0]"));
    assert!(script.contains("ansible_needs_a_playbook_as_the_entry"));
    assert!(script.contains("opentofu_needs_a_module_directory"));
    assert!(!script.contains("placeholder=\"site.yml\""));
    let (status, style, _) = call(&app, "GET", "/app.css", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(style.as_str().unwrap_or("").contains("field-error"));

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

    let (status, ansible, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Empty playbook",
            "tool": "ansible",
            "entrypoint": "  ",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{ansible}");
    assert_eq!(ansible["error"], "ansible_needs_a_playbook_as_the_entry");

    let (status, tofu, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Empty module",
            "tool": "opentofu",
            "entrypoint": "",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{tofu}");
    assert_eq!(tofu["error"], "opentofu_needs_a_module_directory");

    let (status, outside, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Outside",
            "tool": "ansible",
            "entrypoint": "../secret",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{outside}");
    assert_eq!(outside["error"], "the_entry_path_must_be_relative_and_must");

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(team),
        Some(json!({
            "name": "Site",
            "tool": "ansible",
            "entrypoint": "site.yml",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["entrypoint"], "site.yml");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_template_can_run_the_last_sync_offline() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = temp_dir("offline");
    let origin = temp_dir("offline-origin");
    let mut cfg = config(&dir, &Options::default());
    cfg.project_roots.push(origin.clone());
    let state = koreograf::boot(cfg).unwrap();
    let app = koreograf::router(state);

    let (status, _, _) = call(&app, "GET", "/js/features/legacy.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
    assert!(script.contains("id=\"update-on-launch\""));
    assert!(script.contains("update_the_project_on_launch"));

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

    git(&origin, &["init", "-b", "main"]);
    git(&origin, &["config", "user.email", "test@example.com"]);
    git(&origin, &["config", "user.name", "Koreograf Test"]);
    std::fs::write(origin.join("marker.txt"), "synced\n").unwrap();
    git(&origin, &["add", "marker.txt"]);
    git(&origin, &["commit", "-m", "synced"]);
    let url = format!("file://{}", origin.display());

    let (status, project, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Offline", "source_type": "git", "source": url})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap();
    let (status, synced, _) = call(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");

    std::fs::write(origin.join("marker.txt"), "newer\n").unwrap();
    git(&origin, &["add", "marker.txt"]);
    git(&origin, &["commit", "-m", "newer"]);

    let (status, defaults, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Default",
            "tool": "shell",
            "project_id": project_id,
            "spec": {"script": "echo ok"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{defaults}");
    assert_eq!(defaults["update_on_launch"], true);

    let (status, offline, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Keep",
            "tool": "shell",
            "project_id": project_id,
            "update_on_launch": false,
            "spec": {"script": "cat marker.txt"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{offline}");
    assert_eq!(offline["update_on_launch"], false);
    let offline_id = offline["id"].as_str().unwrap();
    let kept = launch_and_wait(&app, &cookie, &team, offline_id).await;
    assert_eq!(kept["status"], "successful", "{}", kept["events"]);
    assert_eq!(kept["parameters"]["update_on_launch"], false);
    assert_eq!(kept["parameters"]["project_name"], "Offline");
    let kept_id = kept["id"].as_str().unwrap().to_string();
    let kept_commit = kept["parameters"]["project_commit"]
        .as_str()
        .expect("project commit")
        .to_string();
    let kept_full = kept["parameters"]["project_commit_full"]
        .as_str()
        .expect("full project commit")
        .to_string();
    assert_eq!(kept_commit.chars().count(), 7);
    assert_eq!(kept_full.chars().count(), 40);
    assert!(kept_full.starts_with(&kept_commit));
    assert!(has_line(
        &kept,
        "system",
        "Using the checkout from the last sync."
    ));
    assert!(has_line(&kept, "stdout", "synced"));
    assert!(!has_line(&kept, "stdout", "newer"));

    let (status, fresh, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Fresh",
            "tool": "shell",
            "project_id": project_id,
            "update_on_launch": true,
            "spec": {"script": "cat marker.txt"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{fresh}");
    let fresh_id = fresh["id"].as_str().unwrap();
    let updated = launch_and_wait(&app, &cookie, &team, fresh_id).await;
    assert_eq!(updated["status"], "successful", "{}", updated["events"]);
    assert_eq!(updated["parameters"]["update_on_launch"], true);
    assert!(!has_line(
        &updated,
        "system",
        "Using the checkout from the last sync."
    ));
    assert!(has_line(&updated, "stdout", "newer"));
    let fresh_full = updated["parameters"]["project_commit_full"]
        .as_str()
        .expect("fresh full commit");
    assert_ne!(fresh_full, kept_full.as_str());

    let (status, synced_again, _) = call(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced_again}");
    let (status, old, _) = call(
        &app,
        "GET",
        &format!("/api/jobs/{kept_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{old}");
    assert_eq!(old["parameters"]["project_commit"], kept_commit);
    assert_eq!(old["parameters"]["project_commit_full"], kept_full);

    let missing = format!(
        "file://{}/koreograf-missing-{}",
        std::env::temp_dir().display(),
        nanos
    );
    let (status, absent, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Absent", "source_type": "git", "source": missing})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{absent}");
    let absent_id = absent["id"].as_str().unwrap();
    let (status, blocked, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Blocked",
            "tool": "shell",
            "project_id": absent_id,
            "update_on_launch": false,
            "spec": {"script": "echo should-not-run"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{blocked}");
    let blocked_id = blocked["id"].as_str().unwrap();
    let failed = launch_and_wait(&app, &cookie, &team, blocked_id).await;
    assert_eq!(failed["status"], "failed", "{}", failed["events"]);
    assert!(has_line(&failed, "system", "Sync the Git project first."));
    assert!(!has_line(&failed, "stdout", "should-not-run"));

    let _ = std::fs::remove_dir_all(&origin);
    let _ = std::fs::remove_dir_all(&dir);
}
