#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use common::{Options, TestApp};
use serde_json::json;

struct EnvGuard;

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for key in [
            "KOREOGRAF_LLM",
            "KOREOGRAF_LLM_URL",
            "KOREOGRAF_LLM_MODEL",
            "KOREOGRAF_LLM_API_KEY",
            "KOREOGRAF_LLM_API_KEY_FILE",
            "KOREOGRAF_LLM_MAX_TOKENS",
            "KOREOGRAF_LLM_TIMEOUT_SECS",
        ] {
            std::env::remove_var(key);
        }
    }
}

#[tokio::test]
async fn the_assistant_route_is_off_until_the_environment_enables_it() {
    let _guard = EnvGuard;
    for key in [
        "KOREOGRAF_LLM",
        "KOREOGRAF_LLM_URL",
        "KOREOGRAF_LLM_MODEL",
        "KOREOGRAF_LLM_API_KEY",
        "KOREOGRAF_LLM_API_KEY_FILE",
    ] {
        std::env::remove_var(key);
    }
    let app = TestApp::boot("llm-gate", Options::default());
    let (cookie, _) = app.sign_in().await;

    let (status, body) = app.call("GET", "/api/assistant", None, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "sign_in_is_required");

    let (status, body) = app
        .call("GET", "/api/assistant", Some(&cookie), None, None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "assistant_is_not_enabled");

    let (status, body) = app
        .call(
            "POST",
            "/api/assistant/chat",
            Some(&cookie),
            None,
            Some(json!({"message": "hello"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "assistant_is_not_enabled");

    std::env::set_var("KOREOGRAF_LLM", "1");
    let (status, body) = app
        .call("GET", "/api/assistant", Some(&cookie), None, None)
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "assistant_is_not_configured");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    std::env::set_var("KOREOGRAF_LLM_URL", format!("http://{address}/v1"));
    std::env::set_var("KOREOGRAF_LLM_MODEL", "local-model");
    std::env::set_var("KOREOGRAF_LLM_API_KEY", "sk-test-secret");
    let (status, body) = app
        .call("GET", "/api/assistant", Some(&cookie), None, None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], true);
    assert_eq!(body["model"], "local-model");
    assert!(!body.to_string().contains("sk-test-secret"));

    let (status, body) = app
        .call(
            "POST",
            "/api/assistant/chat",
            Some(&cookie),
            None,
            Some(json!({"message": "hello", "history": [{"role": "system", "content": "x"}]})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "assistant_history_is_invalid");
    assert!(!body.to_string().contains("sk-test-secret"));

    let (status, body) = app
        .call(
            "POST",
            "/api/assistant/chat",
            Some(&cookie),
            None,
            Some(json!({"message": "hello"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"], "assistant_upstream_unreachable");
    assert!(!body.to_string().contains("sk-test-secret"));

    for _ in 0..8 {
        let (status, _) = app
            .call(
                "POST",
                "/api/assistant/chat",
                Some(&cookie),
                None,
                Some(json!({"message": "hello", "history": [{"role": "system", "content": "x"}]})),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, body) = app
        .call(
            "POST",
            "/api/assistant/chat",
            Some(&cookie),
            None,
            Some(json!({"message": "hello"})),
        )
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"], "assistant_rate_limited");
}
