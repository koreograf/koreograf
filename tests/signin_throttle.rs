use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use http_body_util::BodyExt;
use koreograf::{Config, CookieSecure, LdapConfig, LocalJobs, RunnerMode};
use serde_json::{json, Value};
use tower::ServiceExt;

fn temp_dir(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "koreograf-signin-throttle-{label}-{nanos}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn config(dir: &Path, trusted_proxy: bool, ldap: Option<LdapConfig>) -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        data_dir: dir.to_path_buf(),
        web_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
        examples_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
        admin_user: "admin".into(),
        admin_password: Some("test-admin-pass".into()),
        runner: RunnerMode::Local,
        container_image: false,
        job_timeout_secs: 60,
        job_output_lines: 8_000,
        local_jobs: LocalJobs::All,
        project_roots: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")],
        hsts: false,
        cookie_secure: CookieSecure::Auto,
        trusted_proxy,
        mcp: false,
        public_url: Vec::new(),
        data_host: None,
        ldap,
    }
}

struct Reply {
    status: StatusCode,
    body: Value,
    headers: HeaderMap,
}

async fn login(
    app: &axum::Router,
    forwarded_for: Option<&str>,
    username: &str,
    password: &str,
) -> Reply {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/login")
        .header("content-type", "application/json");
    if let Some(source) = forwarded_for {
        builder = builder.header("x-forwarded-for", source);
    }
    let request = builder
        .body(Body::from(
            json!({ "username": username, "password": password }).to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Reply {
        status,
        body,
        headers,
    }
}

fn retry_after(reply: &Reply) -> u64 {
    reply
        .headers
        .get("retry-after")
        .expect("retry-after")
        .to_str()
        .unwrap()
        .parse()
        .unwrap()
}

async fn directory_probe() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let counter = connections.clone();
    let handle = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            drop(socket);
        }
    });
    (format!("ldap://{address}"), connections, handle)
}

fn directory_config(url: String) -> LdapConfig {
    LdapConfig {
        url,
        mode: "ldap".into(),
        bind_dn: "cn=reader,dc=example,dc=com".into(),
        bind_password: "secret".into(),
        user_base: "ou=people,dc=example,dc=com".into(),
        user_filter: "(uid={username})".into(),
        username_attr: "uid".into(),
        group_attr: "memberOf".into(),
        group_base: String::new(),
        starttls: false,
        tls_insecure: false,
    }
}

#[tokio::test]
async fn a_burst_of_bad_passwords_ends_in_a_retry_after() {
    let dir = temp_dir("burst");
    let state = koreograf::boot(config(&dir, false, None)).unwrap();
    let app = koreograf::router(state);

    for _ in 0..5 {
        let reply = login(&app, None, "admin", "wrong-password").await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert_eq!(reply.body["error"], "invalid_username_or_password");
    }

    let blocked = login(&app, None, "admin", "wrong-password").await;
    assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(blocked.body["error"], "invalid_username_or_password");
    let retry = retry_after(&blocked);
    assert!((1..=60).contains(&retry), "{retry}");

    let refused = login(&app, None, "admin", "test-admin-pass").await;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(retry_after(&refused) >= 1);

    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn another_source_is_not_locked_out() {
    let dir = temp_dir("source");
    let state = koreograf::boot(config(&dir, true, None)).unwrap();
    let app = koreograf::router(state);

    for _ in 0..5 {
        let reply = login(&app, Some("203.0.113.10"), "admin", "wrong-password").await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
    let blocked = login(&app, Some("203.0.113.10"), "admin", "wrong-password").await;
    assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);

    let other_bad = login(&app, Some("203.0.113.11"), "admin", "wrong-password").await;
    assert_eq!(other_bad.status, StatusCode::UNAUTHORIZED);

    let other_ok = login(&app, Some("203.0.113.11"), "admin", "test-admin-pass").await;
    assert_eq!(other_ok.status, StatusCode::OK);

    let still_blocked = login(&app, Some("203.0.113.10"), "admin", "test-admin-pass").await;
    assert_eq!(still_blocked.status, StatusCode::TOO_MANY_REQUESTS);

    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_refused_sign_in_does_not_contact_the_directory() {
    let dir = temp_dir("directory");
    let (url, connections, handle) = directory_probe().await;
    let state = koreograf::boot(config(&dir, true, Some(directory_config(url)))).unwrap();
    let app = koreograf::router(state);

    for _ in 0..5 {
        let reply = login(&app, Some("203.0.113.20"), "ada", "wrong-password").await;
        assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let before = connections.load(Ordering::SeqCst);
    assert!(before > 0, "the directory probe was never contacted");

    let blocked = login(&app, Some("203.0.113.20"), "ada", "wrong-password").await;
    assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(connections.load(Ordering::SeqCst), before);

    handle.abort();
    let _ = std::fs::remove_dir_all(dir);
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn capture_logs() -> Arc<Mutex<Vec<u8>>> {
    static BUFFER: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    static INIT: Once = Once::new();
    let buffer = BUFFER
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone();
    INIT.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(Capture(buffer.clone()))
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
    buffer
}

#[tokio::test]
async fn the_failure_log_names_the_username_and_source_without_the_password() {
    let dir = temp_dir("log");
    let state = koreograf::boot(config(&dir, true, None)).unwrap();
    let app = koreograf::router(state);

    let buffer = capture_logs();
    buffer.lock().unwrap().clear();
    let reply = login(
        &app,
        Some("198.51.100.20"),
        "  ADMIN  ",
        "TopSecret-ShouldNotAppear",
    )
    .await;

    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    let text = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert!(
        text.lines()
            .any(|line| line.contains("username=admin") && line.contains("198.51.100.20")),
        "{text}"
    );
    assert!(!text.contains("TopSecret-ShouldNotAppear"), "{text}");

    let _ = std::fs::remove_dir_all(dir);
}
