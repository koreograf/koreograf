use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use koreograf::{Config, CookieSecure, LocalJobs, RunnerMode};
use serde_json::{json, Value};
use tower::ServiceExt;

fn config(dir: &Path, root: &Path) -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        data_dir: dir.to_path_buf(),
        web_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("web"),
        examples_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("examples"),
        admin_user: "admin".into(),
        admin_password: Some("test-admin-pass".into()),
        runner: RunnerMode::Local,
        container_image: true,
        job_timeout_secs: 60,
        job_output_lines: 8_000,
        local_jobs: LocalJobs::All,
        project_roots: vec![root.to_path_buf()],
        hsts: false,
        cookie_secure: CookieSecure::Auto,
        trusted_proxy: false,
        mcp: true,
        public_url: Vec::new(),
        data_host: None,
        ldap: None,
    }
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    team: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, Option<String>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    if let Some(team) = team {
        builder = builder.header("x-team-id", team);
    }
    let request = if let Some(body) = body {
        builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::to_string);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).to_string()))
    };
    (status, parsed, cookie)
}

async fn sign_in(app: &axum::Router) -> (String, String) {
    let (status, me, cookie) = call(
        app,
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
    (cookie, team)
}

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

fn temp_dir(label: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("koreograf-file-git-{label}-{nanos}"))
}

#[tokio::test]
async fn file_git_project_syncs_with_the_container_image_flag() {
    let dir = temp_dir("sync");
    let data = dir.join("data");
    let root = dir.join("projects");
    let origin = root.join("repo");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-b", "main"]);
    git(&origin, &["config", "user.email", "test@example.com"]);
    git(&origin, &["config", "user.name", "Koreograf Test"]);
    std::fs::write(origin.join("marker.txt"), "one\n").unwrap();
    git(&origin, &["add", "marker.txt"]);
    git(&origin, &["commit", "-m", "one"]);

    let state = koreograf::boot(config(&data, &root)).unwrap();
    let app = koreograf::router(state);
    let (cookie, team) = sign_in(&app).await;
    let url = format!("file://{}", origin.display());
    let (status, project, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Local", "source_type": "git", "source": url})),
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
    let checkout = Path::new(synced["path"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(checkout.join("marker.txt")).unwrap(),
        "one\n"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_git_url_validation_maps_errors_to_api_keys() {
    let dir = temp_dir("urls");
    let data = dir.join("data");
    let root = dir.join("projects");
    std::fs::create_dir_all(&root).unwrap();

    let state = koreograf::boot(config(&data, &root)).unwrap();
    let app = koreograf::router(state);
    let (cookie, team) = sign_in(&app).await;

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
        let (status, body, _) = call(
            &app,
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

    let (status, body, _) = call(
        &app,
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

    let (status, body, _) = call(
        &app,
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
