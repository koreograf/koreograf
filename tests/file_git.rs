#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;

use axum::http::StatusCode;
use common::{temp_dir, Options, TestApp};
use serde_json::json;

fn git(repo: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn file_git_project_syncs_with_the_container_image_flag() {
    let dir = temp_dir("file-git-sync");
    let root = dir.join("projects");
    let origin = root.join("repo");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-b", "main"]);
    git(&origin, &["config", "user.email", "test@example.com"]);
    git(&origin, &["config", "user.name", "Koreograf Test"]);
    std::fs::write(origin.join("marker.txt"), "one\n").unwrap();
    git(&origin, &["add", "marker.txt"]);
    git(&origin, &["commit", "-m", "one"]);

    let app = TestApp::boot(
        "file-git-sync",
        Options {
            container_image: true,
            project_roots: vec![root.clone()],
            ..Options::default()
        },
    );
    let (cookie, team) = app.sign_in().await;
    let url = format!("file://{}", origin.display());
    let (status, project) = app
        .call(
            "POST",
            "/api/projects",
            Some(&cookie),
            Some(&team),
            Some(json!({"name": "Local", "source_type": "git", "source": url})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap();
    let (status, synced) = app
        .call(
            "POST",
            &format!("/api/projects/{project_id}/sync"),
            Some(&cookie),
            Some(&team),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    let checkout = Path::new(synced["path"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(checkout.join("marker.txt")).unwrap(),
        "one\n"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_git_url_validation_maps_errors_to_api_keys() {
    let dir = temp_dir("file-git-urls");
    let root = dir.join("projects");
    std::fs::create_dir_all(&root).unwrap();

    let app = TestApp::boot(
        "file-git-urls",
        Options {
            container_image: true,
            project_roots: vec![root.clone()],
            ..Options::default()
        },
    );
    let (cookie, team) = app.sign_in().await;

    for (source, key) in [
        ("file://evil.example/tmp", "the_project_path_is_not_allowed"),
        ("file:///etc", "the_project_path_is_not_allowed"),
        ("file://relative", "the_project_path_is_not_allowed"),
        (
            "file:///tmp/demo.git extra",
            "the_git_address_must_be_http_s_ssh",
        ),
        ("file:///tmp/../etc", "the_git_address_must_be_http_s_ssh"),
    ] {
        let (status, body) = app
            .call(
                "POST",
                "/api/projects",
                Some(&cookie),
                Some(&team),
                Some(json!({"name": "Bad", "source_type": "git", "source": source})),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{source}: {body}");
        assert_eq!(body["error"], key, "{source}: {body}");
    }

    let (status, body) = app
        .call(
            "POST",
            "/api/projects",
            Some(&cookie),
            Some(&team),
            Some(json!({
                "name": "Remote",
                "source_type": "git",
                "source": "https://example.com/repo.git"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, body) = app
        .call(
            "POST",
            "/api/projects",
            Some(&cookie),
            Some(&team),
            Some(json!({
                "name": "Path",
                "source_type": "path",
                "source": root.display().to_string()
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let _ = std::fs::remove_dir_all(&dir);
}
