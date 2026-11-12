#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock};
use std::time::Duration;

use axum::extract::ConnectInfo;
use axum::http::StatusCode;
use axum::Extension;
use common::{login_reply, Options, Reply, TestApp};
use koreograf::LdapConfig;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

async fn rejecting_directory() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let counter = connections.clone();
    let handle = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            let mut buffer = [0_u8; 4096];
            loop {
                match socket.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if let Some(id) = ldap_message_id(&buffer[..read]) {
                            let _ = socket.write_all(&ldap_rejection(&id)).await;
                        }
                    }
                }
            }
        }
    });
    (format!("ldap://{address}"), connections, handle)
}

fn ldap_message_id(message: &[u8]) -> Option<Vec<u8>> {
    if message.first() != Some(&0x30) {
        return None;
    }
    let (_, mut index) = ldap_length(message, 1)?;
    if message.get(index) != Some(&0x02) {
        return None;
    }
    let (length, start) = ldap_length(message, index + 1)?;
    index = start;
    message.get(index..index + length).map(<[u8]>::to_vec)
}

fn ldap_length(bytes: &[u8], index: usize) -> Option<(usize, usize)> {
    let first = *bytes.get(index)?;
    if first & 0x80 == 0 {
        return Some((usize::from(first), index + 1));
    }
    let count = usize::from(first & 0x7f);
    if count == 0 || count > 4 {
        return None;
    }
    let mut length = 0;
    for offset in 0..count {
        length = (length << 8) | usize::from(*bytes.get(index + 1 + offset)?);
    }
    Some((length, index + 1 + count))
}

fn ldap_rejection(id: &[u8]) -> Vec<u8> {
    let mut body = vec![0x02, id.len() as u8];
    body.extend_from_slice(id);
    body.extend_from_slice(&[0x61, 0x07, 0x0a, 0x01, 0x31, 0x04, 0x00, 0x04, 0x00]);
    let mut response = vec![0x30, body.len() as u8];
    response.extend(body);
    response
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
    let harness = TestApp::boot(
        "signin-burst",
        Options {
            mcp: false,
            ..Options::default()
        },
    );
    let app = harness
        .app
        .clone()
        .layer(Extension(ConnectInfo(SocketAddr::from((
            [127, 0, 0, 1],
            40000,
        )))));

    for _ in 0..5 {
        let reply = login_reply(&app, None, "admin", "wrong-password").await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert_eq!(reply.body["error"], "invalid_username_or_password");
    }

    let blocked = login_reply(&app, None, "admin", "wrong-password").await;
    assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(blocked.body["error"], "invalid_username_or_password");
    let retry = retry_after(&blocked);
    assert!((1..=60).contains(&retry), "{retry}");

    let refused = login_reply(&app, None, "admin", "test-admin-pass").await;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(retry_after(&refused) >= 1);
}

#[tokio::test]
async fn another_source_is_not_locked_out() {
    let harness = TestApp::boot(
        "signin-source",
        Options {
            mcp: false,
            trusted_proxy: true,
            ..Options::default()
        },
    );
    let app = harness.app.clone();

    for _ in 0..5 {
        let reply = login_reply(&app, Some("203.0.113.10"), "admin", "wrong-password").await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
    let blocked = login_reply(&app, Some("203.0.113.10"), "admin", "wrong-password").await;
    assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);

    let other_bad = login_reply(&app, Some("203.0.113.11"), "admin", "wrong-password").await;
    assert_eq!(other_bad.status, StatusCode::UNAUTHORIZED);

    let other_ok = login_reply(&app, Some("203.0.113.11"), "admin", "test-admin-pass").await;
    assert_eq!(other_ok.status, StatusCode::OK);

    let still_blocked = login_reply(&app, Some("203.0.113.10"), "admin", "test-admin-pass").await;
    assert_eq!(still_blocked.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn an_unknown_source_is_not_throttled() {
    let harness = TestApp::boot(
        "signin-unknown",
        Options {
            mcp: false,
            ..Options::default()
        },
    );
    let app = harness.app.clone();

    for _ in 0..6 {
        let reply = login_reply(&app, None, "admin", "wrong-password").await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
}

async fn direct_login(client: &reqwest::Client, url: &str, password: &str) -> StatusCode {
    client
        .post(url)
        .json(&json!({ "username": "admin", "password": password }))
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn a_busy_direct_peer_does_not_lock_out_another_loopback_address() {
    let harness = TestApp::boot(
        "signin-peer",
        Options {
            mcp: false,
            ..Options::default()
        },
    );
    let app = harness.app.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let url = format!("http://{address}/api/login");
    let first = reqwest::Client::builder()
        .local_address(IpAddr::from([127, 0, 0, 2]))
        .build()
        .unwrap();
    let second = reqwest::Client::builder()
        .local_address(IpAddr::from([127, 0, 0, 3]))
        .build()
        .unwrap();

    for _ in 0..5 {
        assert_eq!(
            direct_login(&first, &url, "wrong-password").await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        direct_login(&first, &url, "wrong-password").await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        direct_login(&second, &url, "wrong-password").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        direct_login(&second, &url, "test-admin-pass").await,
        StatusCode::OK
    );
    assert_eq!(
        direct_login(&first, &url, "test-admin-pass").await,
        StatusCode::TOO_MANY_REQUESTS
    );

    server.abort();
}

#[tokio::test]
async fn a_refused_sign_in_does_not_contact_the_directory() {
    let (url, connections, handle) = rejecting_directory().await;
    let harness = TestApp::boot(
        "signin-directory",
        Options {
            mcp: false,
            trusted_proxy: true,
            ldap: Some(directory_config(url)),
            ..Options::default()
        },
    );
    let app = harness.app.clone();

    for _ in 0..5 {
        let reply = login_reply(&app, Some("203.0.113.20"), "ada", "wrong-password").await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let before = connections.load(Ordering::SeqCst);
    assert!(before > 0, "the directory probe was never contacted");

    let blocked = login_reply(&app, Some("203.0.113.20"), "ada", "wrong-password").await;
    assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(connections.load(Ordering::SeqCst), before);

    handle.abort();
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
    let harness = TestApp::boot(
        "signin-log",
        Options {
            mcp: false,
            trusted_proxy: true,
            ..Options::default()
        },
    );
    let app = harness.app.clone();

    let buffer = capture_logs();
    buffer.lock().unwrap().clear();
    let reply = login_reply(
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
}
