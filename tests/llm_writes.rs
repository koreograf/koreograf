use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use koreograf::{Config, CookieSecure, LocalJobs, RunnerMode};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tower::ServiceExt;

fn config(dir: &std::path::Path) -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        data_dir: dir.to_path_buf(),
        web_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
        examples_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
        admin_user: "admin".into(),
        admin_password: Some("test-admin-pass".into()),
        runner: RunnerMode::Local,
        container_image: false,
        job_timeout_secs: 60,
        job_output_lines: 8_000,
        local_jobs: LocalJobs::All,
        project_roots: vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")],
        hsts: false,
        cookie_secure: CookieSecure::Auto,
        trusted_proxy: false,
        mcp: true,
        public_url: Vec::new(),
        data_host: None,
        ldap: None,
    }
}

struct Harness {
    dir: std::path::PathBuf,
    app: axum::Router,
}

impl Harness {
    fn boot(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-llm-{name}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let state = koreograf::boot(config(&dir)).unwrap();
        Self {
            dir,
            app: koreograf::router(state),
        }
    }

    async fn call(
        &self,
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
        let response = self.app.clone().oneshot(request).await.unwrap();
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

    async fn login(&self, username: &str, password: &str) -> String {
        let (status, body, cookie) = self
            .call(
                "POST",
                "/api/login",
                None,
                None,
                Some(json!({"username": username, "password": password})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        cookie.expect("session cookie")
    }

    async fn save_assistant(&self, cookie: &str, endpoint: &str, enabled: bool) {
        let body = json!({
            "ldap": {},
            "proxy": {},
            "runner": "local",
            "job_timeout_secs": 60,
            "llm": {
                "enabled": enabled,
                "provider": "ollama",
                "base_url": endpoint,
                "model": "test-model",
                "api_key": "sk-test-secret"
            }
        });
        let (status, body, _) = self
            .call("PUT", "/api/settings", Some(cookie), None, Some(body))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(!body.to_string().contains("sk-test-secret"));
        assert_eq!(body["llm"]["api_key_set"], true);
    }

    async fn team(&self, cookie: &str) -> (String, String) {
        let (status, body, _) = self
            .call("GET", "/api/teams", Some(cookie), None, None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let team = body
            .as_array()
            .and_then(|items| items.first())
            .expect("seeded team");
        (
            team["id"].as_str().unwrap().to_string(),
            team["name"].as_str().unwrap().to_string(),
        )
    }

    async fn demo_project(&self, cookie: &str, team: &str) -> String {
        let (status, body, _) = self
            .call("GET", "/api/projects", Some(cookie), Some(team), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body.as_array()
            .and_then(|items| items.iter().find(|item| item["name"] == "Demo"))
            .and_then(|item| item["id"].as_str())
            .map(str::to_string)
            .expect("seeded Demo project")
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn stub(body: Value) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let body = body.to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let header_end = loop {
            let mut chunk = [0u8; 1024];
            let read = socket.read(&mut chunk).await.unwrap();
            assert!(read > 0, "client closed before the headers");
            request.extend_from_slice(&chunk[..read]);
            if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
        let length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while request.len() < header_end + length {
            let mut chunk = [0u8; 1024];
            let read = socket.read(&mut chunk).await.unwrap();
            assert!(read > 0, "client closed before the body");
            request.extend_from_slice(&chunk[..read]);
        }
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
        String::from_utf8_lossy(&request).to_string()
    });
    (format!("http://{address}/v1"), handle)
}

fn wire_body(request: &str) -> &str {
    request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or("")
}

fn tool_call(name: &str, arguments: Value) -> Value {
    json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": name, "arguments": arguments.to_string()}
                }]
            }
        }]
    })
}

#[tokio::test]
async fn a_viewer_cannot_launch_through_chat() {
    let harness = Harness::boot("viewer-launch");
    let admin = harness.login("admin", "test-admin-pass").await;
    let (team, _) = harness.team(&admin).await;
    let (status, body, _) = harness
        .call(
            "POST",
            "/api/users",
            Some(&admin),
            None,
            Some(json!({
                "username": "viewer",
                "password": "viewer-pass",
                "display_name": "Viewer",
                "is_admin": false
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, body, _) = harness
        .call(
            "POST",
            &format!("/api/teams/{team}/members"),
            Some(&admin),
            None,
            Some(json!({"username": "viewer", "role": "viewer"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (endpoint, server) = stub(tool_call(
        "launch_template",
        json!({"id": "template-1", "team_id": team}),
    ))
    .await;
    harness.save_assistant(&admin, &endpoint, true).await;

    let viewer = harness.login("viewer", "viewer-pass").await;
    let (status, body, _) = harness
        .call(
            "POST",
            "/api/assistant/chat",
            Some(&viewer),
            Some(&team),
            Some(json!({"message": "launch the template"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "assistant_write_is_forbidden");
    let request = server.await.unwrap();
    assert!(request.contains("launch_template"), "{request}");
    assert!(!wire_body(&request).contains("sk-test-secret"), "{request}");
}

#[tokio::test]
async fn the_saved_flag_turns_the_chat_routes_and_the_entry_off() {
    let harness = Harness::boot("gate");
    let admin = harness.login("admin", "test-admin-pass").await;
    harness
        .save_assistant(&admin, "http://127.0.0.1:9/v1", false)
        .await;

    let (status, body, _) = harness
        .call("GET", "/api/assistant", Some(&admin), None, None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "assistant_is_not_enabled");

    let (status, body, _) = harness
        .call(
            "POST",
            "/api/assistant/chat",
            Some(&admin),
            None,
            Some(json!({"message": "hello"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "assistant_is_not_enabled");

    let (status, body, _) = harness
        .call("GET", "/api/settings", Some(&admin), None, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["llm"]["enabled"], false);
    assert!(!body.to_string().contains("sk-test-secret"), "{body}");
}

#[tokio::test]
async fn a_write_round_trip_waits_for_confirmation_and_cancel_changes_nothing() {
    let harness = Harness::boot("confirm");
    let admin = harness.login("admin", "test-admin-pass").await;
    let (team, _) = harness.team(&admin).await;
    let project = harness.demo_project(&admin, &team).await;

    let (endpoint, server) = stub(tool_call(
        "sync_project",
        json!({"id": project, "team_id": team}),
    ))
    .await;
    harness.save_assistant(&admin, &endpoint, true).await;

    let (status, body, _) = harness
        .call(
            "POST",
            "/api/assistant/chat",
            Some(&admin),
            Some(&team),
            Some(json!({"message": "sync the demo project"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["pending_action"]["tool"], "sync_project");
    assert_eq!(body["pending_action"]["target"], "Demo");
    let action = body["pending_action"]["id"].as_str().unwrap().to_string();
    let request = server.await.unwrap();
    assert!(request.contains("sync_project"), "{request}");
    assert!(!wire_body(&request).contains("sk-test-secret"), "{request}");

    let (status, body, _) = harness
        .call(
            "POST",
            "/api/assistant/confirm",
            Some(&admin),
            Some(&team),
            Some(json!({"id": action})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);

    let (status, body, _) = harness
        .call(
            "POST",
            "/api/assistant/confirm",
            Some(&admin),
            Some(&team),
            Some(json!({"id": action})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "assistant_action_is_unknown");

    let (endpoint, server) = stub(tool_call(
        "sync_project",
        json!({"id": project, "team_id": team}),
    ))
    .await;
    harness.save_assistant(&admin, &endpoint, true).await;
    let (status, body, _) = harness
        .call(
            "POST",
            "/api/assistant/chat",
            Some(&admin),
            Some(&team),
            Some(json!({"message": "sync the demo project again"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let canceled = body["pending_action"]["id"].as_str().unwrap().to_string();
    let _ = server.await;

    let (status, body, _) = harness
        .call(
            "POST",
            "/api/assistant/cancel",
            Some(&admin),
            Some(&team),
            Some(json!({"id": canceled})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body, _) = harness
        .call(
            "POST",
            "/api/assistant/confirm",
            Some(&admin),
            Some(&team),
            Some(json!({"id": canceled})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "assistant_action_is_unknown");
}
