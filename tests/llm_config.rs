use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use koreograf::llm::{parse, LlmConfig, LlmConfigError};

fn temp_file(name: &str, body: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("koreograf-llm-{name}-{nanos}"));
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn the_assistant_is_off_by_default() {
    assert_eq!(parse(false, "", "", "", None), Ok(None));
    assert_eq!(
        parse(false, "http://localhost:11434/v1", "llama3", "secret", None),
        Ok(None)
    );
}

#[test]
fn an_enabled_assistant_needs_an_address_and_model() {
    assert_eq!(
        parse(true, "", "llama3", "", None).unwrap_err(),
        LlmConfigError::MissingEndpoint
    );
    assert_eq!(
        parse(true, "http://localhost:11434/v1", "", "", None).unwrap_err(),
        LlmConfigError::MissingModel
    );
    assert_eq!(
        parse(true, "ftp://localhost/v1", "llama3", "", None).unwrap_err(),
        LlmConfigError::InvalidEndpoint
    );
    assert_eq!(
        parse(true, "http://user:secret@localhost/v1", "llama3", "", None).unwrap_err(),
        LlmConfigError::InvalidEndpoint
    );
    assert_eq!(
        parse(true, "http://localhost/v1", "bad\nmodel", "", None).unwrap_err(),
        LlmConfigError::InvalidModel
    );
}

#[test]
fn a_base_address_and_a_full_endpoint_are_both_accepted() {
    let base = parse(true, "http://localhost:11434/v1/", "llama3", "", None)
        .unwrap()
        .unwrap();
    assert_eq!(base.endpoint, "http://localhost:11434/v1");
    assert_eq!(base.model, "llama3");
    assert_eq!(base.max_tokens, 800);
    let full = parse(
        true,
        "https://api.example.com/v1/chat/completions",
        "gpt-test",
        "",
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(full.endpoint, "https://api.example.com/v1/chat/completions");
}

#[test]
fn the_key_file_wins_and_never_appears_in_debug_or_errors() {
    let path = temp_file("key", "file-secret\n");
    let config = parse(
        true,
        "https://api.example.com/v1",
        "gpt-test",
        "inline-secret",
        Some(&path),
    )
    .unwrap()
    .unwrap();
    let debug = format!("{config:?}");
    assert!(!debug.contains("file-secret"), "{debug}");
    assert!(!debug.contains("inline-secret"), "{debug}");
    assert!(debug.contains("<redacted>"), "{debug}");
    assert!(!format!("{:?}", LlmConfigError::UnreadableKeyFile).contains("file-secret"));
    assert!(!LlmConfigError::UnreadableKeyFile
        .to_string()
        .contains("file-secret"));
    let _ = std::fs::remove_file(&path);
    let missing = parse(
        true,
        "https://api.example.com/v1",
        "gpt-test",
        "",
        Some(&path),
    );
    assert_eq!(missing.unwrap_err(), LlmConfigError::UnreadableKeyFile);
}

#[test]
fn limits_are_clamped() {
    let config = LlmConfig::new("http://localhost/v1", "llama3", "secret")
        .unwrap()
        .with_limits(1, 1);
    assert_eq!(config.max_tokens, 64);
    assert_eq!(config.timeout_secs, 5);
    let wide = LlmConfig::new("http://localhost/v1", "llama3", "secret")
        .unwrap()
        .with_limits(99_999, 99_999);
    assert_eq!(wide.max_tokens, 4_096);
    assert_eq!(wide.timeout_secs, 300);
}
