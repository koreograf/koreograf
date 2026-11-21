use std::path::Path;

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::common::call_cookie as call;

use crate::common::*;

fn seed_dirs(dir: &Path, team: &str) -> std::path::PathBuf {
    let root = dir.join("cache").join(team);
    std::fs::create_dir_all(root.join("collections/ansible_collections/acme/tools")).unwrap();
    std::fs::write(
        root.join("collections/ansible_collections/acme/tools/MANIFEST.json"),
        br#"{"collection_info":{"version":"1.2.3"}}"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("roles/common")).unwrap();
    std::fs::create_dir_all(root.join("tofu/registry.example.com")).unwrap();
    std::fs::write(root.join("tofu/registry.example.com/provider"), [0_u8; 8]).unwrap();
    std::fs::create_dir_all(root.join("tofu-state/tpl-1")).unwrap();
    std::fs::write(root.join("tofu-state/tpl-1/state.json"), b"{}").unwrap();
    std::fs::write(root.join("unknown-entry.txt"), b"keep").unwrap();
    root
}

fn assert_cleared(root: &Path) {
    assert!(!root.join("collections").exists(), "collections stayed");
    assert!(!root.join("roles").exists(), "roles stayed");
    assert!(!root.join("tofu").exists(), "tofu stayed");
    assert!(
        root.join("tofu-state/tpl-1/state.json").exists(),
        "OpenTofu state was removed"
    );
    assert!(
        root.join("unknown-entry.txt").exists(),
        "an unknown entry was removed"
    );
}

async fn create_member(
    app: &axum::Router,
    admin: &str,
    admin_team: &str,
    member_team: &str,
    username: &str,
    role: &str,
) -> String {
    let (status, created, _) = call(
        app,
        "POST",
        "/api/users",
        Some(admin),
        Some(admin_team),
        Some(json!({
            "username": username,
            "display_name": username,
            "password": format!("{username}-pass"),
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let (status, member, _) = call(
        app,
        "POST",
        &format!("/api/teams/{member_team}/members"),
        Some(admin),
        Some(admin_team),
        Some(json!({"username": username, "role": role})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{member}");
    let (status, _, cookie) = call(
        app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": username, "password": format!("{username}-pass")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    cookie.expect("member cookie")
}

async fn activity(app: &axum::Router, cookie: &str, team: &str) -> Value {
    let (status, body, _) = call(
        app,
        "GET",
        "/api/v1/activity?object_type=cache",
        Some(cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_clears_only_the_disposable_cache_and_records_activity() {
    let harness = TestApp::boot("cache-owner", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();
    let (cookie, team) = harness.sign_in().await;
    let root = seed_dirs(&dir, &team);
    let outside = dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), b"keep").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("roles/outside-link")).unwrap();
    }

    let (status, cleared, _) = call(
        &app,
        "DELETE",
        "/api/v1/cache",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{cleared}");
    assert_cleared(&root);
    assert!(outside.join("keep.txt").exists(), "a symlink was followed");

    let (status, report, _) = call(
        &app,
        "GET",
        "/api/v1/cache",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["collections"], json!([]));
    assert_eq!(report["roles"], json!([]));
    assert_eq!(report["collections_bytes"], 0);
    assert_eq!(report["roles_bytes"], 0);
    assert_eq!(report["tofu_bytes"], 0);
    assert_eq!(
        report["note"],
        "the_cache_holds_dependencies_and_keeps_the_opentofu_state"
    );

    let rows = activity(&app, &cookie, &team).await;
    assert_eq!(rows["total"], 1, "{rows}");
    let item = &rows["items"][0];
    assert_eq!(item["action"], "delete");
    assert_eq!(item["object_type"], "cache");
    assert_eq!(item["result"], "success");
    assert_eq!(item["actor"], "admin");
    assert_eq!(item["team_id"], team);

    let (status, again, _) = call(
        &app,
        "DELETE",
        "/api/v1/cache",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{again}");
    assert_eq!(activity(&app, &cookie, &team).await["total"], 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn viewers_and_operators_cannot_clear_the_cache() {
    let harness = TestApp::boot("cache-roles", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();
    let (cookie, team) = harness.sign_in().await;
    let root = seed_dirs(&dir, &team);
    let viewer = create_member(&app, &cookie, &team, &team, "cache-viewer", "viewer").await;
    let operator = create_member(&app, &cookie, &team, &team, "cache-operator", "operator").await;

    for member in [&viewer, &operator] {
        let (status, denied, _) = call(
            &app,
            "DELETE",
            "/api/v1/cache",
            Some(member),
            Some(&team),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
        assert!(root.join("collections").exists());
        assert!(root.join("roles").exists());
        assert!(root.join("tofu").exists());
    }
    assert_eq!(activity(&app, &cookie, &team).await["total"], 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owner_of_another_team_cannot_clear_this_cache() {
    let harness = TestApp::boot("cache-foreign-owner", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();
    let (cookie, team) = harness.sign_in().await;
    let root = seed_dirs(&dir, &team);

    let (status, other, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Cache other", "description": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_team = other["id"].as_str().unwrap();
    let owner = create_member(
        &app,
        &cookie,
        &team,
        other_team,
        "cache-other-owner",
        "owner",
    )
    .await;

    let (status, denied, _) = call(
        &app,
        "DELETE",
        "/api/v1/cache",
        Some(&owner),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    assert!(root.join("collections").exists());
    assert_eq!(activity(&app, &cookie, &team).await["total"], 0);

    let (status, allowed, _) = call(
        &app,
        "DELETE",
        "/api/v1/cache",
        Some(&owner),
        Some(other_team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{allowed}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pending_job_blocks_the_clear_until_it_finishes() {
    let harness = TestApp::boot("cache-busy", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();
    let (cookie, team) = harness.sign_in().await;
    let root = seed_dirs(&dir, &team);
    let (status, template, _) = call(
        &app,
        "POST",
        "/api/v1/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Cache busy",
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": "#!/bin/sh\nsleep 10\n"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();
    let (status, job, _) = call(
        &app,
        "POST",
        &format!("/api/v1/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    let job_id = job["id"].as_str().unwrap();

    let (status, refused, _) = call(
        &app,
        "DELETE",
        "/api/v1/cache",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"], "a_job_is_running");
    assert!(root.join("collections").exists());
    assert!(root.join("roles").exists());
    assert!(root.join("tofu").exists());
    assert_eq!(activity(&app, &cookie, &team).await["total"], 0);

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
    let finished = wait_for_job(&app, &cookie, &team, job_id).await;
    assert_eq!(finished["status"], "canceled");

    let (status, cleared, _) = call(
        &app,
        "DELETE",
        "/api/v1/cache",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{cleared}");
    assert_cleared(&root);
    assert_eq!(activity(&app, &cookie, &team).await["total"], 1);
    let _ = std::fs::remove_dir_all(&dir);
}
