use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use koreograf::llm::{complete, ChatMessage, LlmConfig, LlmError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

async fn stub(status: &str, body: &str) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
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

fn message(role: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: role.into(),
        content: content.into(),
    }
}

#[tokio::test]
async fn the_request_maps_to_the_openai_chat_shape() {
    let (endpoint, server) = stub(
        "200 OK",
        r#"{"choices":[{"message":{"content":"  Hello from the stub  "}}]}"#,
    )
    .await;
    let config = LlmConfig::new(&endpoint, "local-model", "sk-test-secret").unwrap();
    let answer = complete(
        &config,
        &[message("system", "be brief"), message("user", "Hi")],
    )
    .await
    .unwrap();
    assert_eq!(answer, "Hello from the stub");
    let request = server.await.unwrap();
    let lower = request.to_ascii_lowercase();
    assert!(
        lower.starts_with("post /v1/chat/completions http/1.1"),
        "{request}"
    );
    assert!(
        lower.contains("authorization: bearer sk-test-secret"),
        "{request}"
    );
    let body = request.split("\r\n\r\n").nth(1).unwrap();
    let value: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(value["model"], "local-model");
    assert_eq!(value["stream"], false);
    assert_eq!(value["max_tokens"], 800);
    assert_eq!(value["messages"][0]["role"], "system");
    assert_eq!(value["messages"][0]["content"], "be brief");
    assert_eq!(value["messages"][1]["content"], "Hi");
}

#[tokio::test]
async fn a_full_endpoint_is_not_rewritten() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0u8; 2048];
        let _ = socket.read(&mut request).await.unwrap();
        let body = r#"{"choices":[{"message":{"content":"ok"}}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
        String::from_utf8_lossy(&request).to_string()
    });
    let endpoint = format!("http://{address}/v1/chat/completions");
    let config = LlmConfig::new(&endpoint, "local-model", "").unwrap();
    assert_eq!(
        complete(&config, &[message("user", "Hi")]).await.unwrap(),
        "ok"
    );
    let request = server.await.unwrap();
    assert!(
        request.starts_with("POST /v1/chat/completions HTTP/1.1"),
        "{request}"
    );
    assert!(!request.to_ascii_lowercase().contains("authorization:"));
}

#[tokio::test]
async fn upstream_failures_map_to_keyed_errors() {
    let (endpoint, server) = stub("429 Too Many Requests", "{}").await;
    let config = LlmConfig::new(&endpoint, "local-model", "sk-test-secret").unwrap();
    assert_eq!(
        complete(&config, &[message("user", "Hi")])
            .await
            .unwrap_err(),
        LlmError::RateLimited
    );
    let _ = server.await;
    let error = LlmError::RateLimited;
    assert!(!format!("{error:?}").contains("sk-test-secret"));
    assert!(!error.to_string().contains("sk-test-secret"));

    let (endpoint, server) = stub("500 Internal Server Error", "{}").await;
    let config = LlmConfig::new(&endpoint, "local-model", "sk-test-secret").unwrap();
    assert_eq!(
        complete(&config, &[message("user", "Hi")])
            .await
            .unwrap_err(),
        LlmError::Upstream
    );
    let _ = server.await;

    let (endpoint, server) = stub("200 OK", "{}").await;
    let config = LlmConfig::new(&endpoint, "local-model", "sk-test-secret").unwrap();
    assert_eq!(
        complete(&config, &[message("user", "Hi")])
            .await
            .unwrap_err(),
        LlmError::InvalidResponse
    );
    let _ = server.await;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let config = LlmConfig::new(
        &format!("http://{address}/v1"),
        "local-model",
        "sk-test-secret",
    )
    .unwrap();
    assert_eq!(
        complete(&config, &[message("user", "Hi")])
            .await
            .unwrap_err(),
        LlmError::Unreachable
    );
}

#[derive(Clone)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl Write for Buf {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("log lock").extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn the_api_key_stays_out_of_the_logs() {
    let (endpoint, server) = stub("500 Internal Server Error", "{}").await;
    let config = LlmConfig::new(&endpoint, "local-model", "sk-test-secret").unwrap();
    let buf = Buf(Arc::new(Mutex::new(Vec::new())));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(buf.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let _ = complete(&config, &[message("user", "Hi")]).await;
    drop(guard);
    let _ = server.await;
    let text = String::from_utf8(buf.0.lock().expect("log lock").clone()).unwrap();
    assert!(text.contains("assistant upstream request failed"), "{text}");
    assert!(!text.contains("sk-test-secret"), "{text}");
    assert!(!text.contains("Bearer"), "{text}");
}
