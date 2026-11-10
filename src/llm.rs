use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::AuthUser;
use crate::db::Team;
use crate::error::ApiJson;
use crate::Shared;

const DEFAULT_MAX_TOKENS: u32 = 800;
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MAX_MESSAGE_CHARS: usize = 8_000;
const MAX_TURN_CHARS: usize = 8_000;
const MAX_HISTORY_TURNS: usize = 20;
const MAX_HISTORY_CHARS: usize = 32_000;
const RATE_LIMIT_REQUESTS: usize = 10;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);
const RATE_LIMIT_KEYS: usize = 1_024;

static CHAT_ATTEMPTS: OnceLock<Mutex<HashMap<String, Vec<Instant>>>> = OnceLock::new();

#[derive(Clone, PartialEq, Eq)]
pub struct LlmConfig {
    pub endpoint: String,
    pub model: String,
    pub max_tokens: u32,
    pub timeout_secs: u64,
    api_key: String,
}

impl std::fmt::Debug for LlmConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LlmConfig")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .field("timeout_secs", &self.timeout_secs)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LlmConfigError {
    MissingEndpoint,
    InvalidEndpoint,
    MissingModel,
    InvalidModel,
    UnreadableKeyFile,
}

impl std::fmt::Display for LlmConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::MissingEndpoint => "KOREOGRAF_LLM_URL is missing",
            Self::InvalidEndpoint => {
                "KOREOGRAF_LLM_URL must be an http or https address without credentials"
            }
            Self::MissingModel => "KOREOGRAF_LLM_MODEL is missing",
            Self::InvalidModel => "KOREOGRAF_LLM_MODEL is invalid",
            Self::UnreadableKeyFile => "KOREOGRAF_LLM_API_KEY_FILE cannot be read",
        })
    }
}

pub enum LlmSetup {
    Disabled,
    Incomplete(LlmConfigError),
    Configured(LlmConfig),
}

impl LlmConfig {
    pub fn new(endpoint: &str, model: &str, api_key: &str) -> Result<Self, LlmConfigError> {
        Self::build(
            endpoint,
            model,
            api_key,
            DEFAULT_MAX_TOKENS,
            DEFAULT_TIMEOUT_SECS,
        )
    }

    pub fn with_limits(mut self, max_tokens: u32, timeout_secs: u64) -> Self {
        self.max_tokens = max_tokens.clamp(64, 4_096);
        self.timeout_secs = timeout_secs.clamp(5, 300);
        self
    }

    fn build(
        endpoint: &str,
        model: &str,
        api_key: &str,
        max_tokens: u32,
        timeout_secs: u64,
    ) -> Result<Self, LlmConfigError> {
        let endpoint = endpoint.trim();
        if endpoint.is_empty() {
            return Err(LlmConfigError::MissingEndpoint);
        }
        let parsed = url::Url::parse(endpoint).map_err(|_| LlmConfigError::InvalidEndpoint)?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || endpoint.chars().any(char::is_control)
        {
            return Err(LlmConfigError::InvalidEndpoint);
        }
        let model = model.trim();
        if model.is_empty() {
            return Err(LlmConfigError::MissingModel);
        }
        if model.chars().count() > 200 || model.chars().any(char::is_control) {
            return Err(LlmConfigError::InvalidModel);
        }
        Ok(Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            model: model.to_string(),
            max_tokens: max_tokens.clamp(64, 4_096),
            timeout_secs: timeout_secs.clamp(5, 300),
            api_key: api_key.trim().to_string(),
        })
    }

    fn request_url(&self) -> String {
        if self.endpoint.ends_with("/chat/completions") {
            self.endpoint.clone()
        } else {
            format!("{}/chat/completions", self.endpoint)
        }
    }

    fn has_api_key(&self) -> bool {
        !self.api_key.is_empty()
    }
}

pub fn parse(
    enabled: bool,
    endpoint: &str,
    model: &str,
    api_key: &str,
    api_key_file: Option<&Path>,
) -> Result<Option<LlmConfig>, LlmConfigError> {
    if !enabled {
        return Ok(None);
    }
    let key = match api_key_file {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|_| LlmConfigError::UnreadableKeyFile)?
            .trim()
            .to_string(),
        None => api_key.trim().to_string(),
    };
    LlmConfig::build(
        endpoint,
        model,
        &key,
        DEFAULT_MAX_TOKENS,
        DEFAULT_TIMEOUT_SECS,
    )
    .map(Some)
}

pub fn from_env() -> LlmSetup {
    let enabled = matches!(
        env_string("KOREOGRAF_LLM").to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "ja"
    );
    let key_file = env_string("KOREOGRAF_LLM_API_KEY_FILE");
    let key_file = if key_file.is_empty() {
        None
    } else {
        Some(Path::new(&key_file))
    };
    match parse(
        enabled,
        &env_string("KOREOGRAF_LLM_URL"),
        &env_string("KOREOGRAF_LLM_MODEL"),
        &env_string("KOREOGRAF_LLM_API_KEY"),
        key_file,
    ) {
        Ok(Some(config)) => {
            let max_tokens = env_string("KOREOGRAF_LLM_MAX_TOKENS")
                .parse()
                .unwrap_or(DEFAULT_MAX_TOKENS);
            let timeout_secs = env_string("KOREOGRAF_LLM_TIMEOUT_SECS")
                .parse()
                .unwrap_or(DEFAULT_TIMEOUT_SECS);
            LlmSetup::Configured(config.with_limits(max_tokens, timeout_secs))
        }
        Ok(None) => LlmSetup::Disabled,
        Err(error) => LlmSetup::Incomplete(error),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmError {
    RateLimited,
    Upstream,
    Unreachable,
    InvalidResponse,
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::RateLimited => "the assistant is rate limited",
            Self::Upstream => "the assistant endpoint returned an error",
            Self::Unreachable => "the assistant endpoint cannot be reached",
            Self::InvalidResponse => "the assistant returned an invalid answer",
        })
    }
}

pub async fn complete(config: &LlmConfig, messages: &[ChatMessage]) -> Result<String, LlmError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.timeout_secs))
        .build()
        .map_err(|_| LlmError::Unreachable)?;
    let body = json!({
        "model": config.model,
        "messages": messages,
        "max_tokens": config.max_tokens,
        "stream": false,
    });
    let mut request = client.post(config.request_url()).json(&body);
    if config.has_api_key() {
        request = request.bearer_auth(&config.api_key);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(_) => {
            tracing::warn!("assistant upstream request failed");
            return Err(LlmError::Unreachable);
        }
    };
    let status = response.status();
    if status == StatusCode::TOO_MANY_REQUESTS {
        tracing::warn!(status = status.as_u16(), "assistant upstream rate limited");
        return Err(LlmError::RateLimited);
    }
    if !status.is_success() {
        tracing::warn!(
            status = status.as_u16(),
            "assistant upstream request failed"
        );
        return Err(LlmError::Upstream);
    }
    let payload: Value = response
        .json()
        .await
        .map_err(|_| LlmError::InvalidResponse)?;
    let answer = payload["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim();
    if answer.is_empty() {
        return Err(LlmError::InvalidResponse);
    }
    Ok(answer.to_string())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatBody {
    message: String,
    #[serde(default)]
    history: Vec<ChatMessage>,
}

pub async fn status(_user: AuthUser) -> Response {
    match from_env() {
        LlmSetup::Configured(config) => {
            Json(json!({"enabled": true, "model": config.model})).into_response()
        }
        LlmSetup::Disabled => failure(StatusCode::NOT_FOUND, "assistant_is_not_enabled"),
        LlmSetup::Incomplete(_) => failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "assistant_is_not_configured",
        ),
    }
}

pub async fn chat(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<ChatBody>,
) -> Response {
    let config = match from_env() {
        LlmSetup::Configured(config) => config,
        LlmSetup::Disabled => return failure(StatusCode::NOT_FOUND, "assistant_is_not_enabled"),
        LlmSetup::Incomplete(_) => {
            return failure(
                StatusCode::SERVICE_UNAVAILABLE,
                "assistant_is_not_configured",
            );
        }
    };
    if !within_rate_limit(&user.id) {
        return failure(StatusCode::TOO_MANY_REQUESTS, "assistant_rate_limited");
    }
    let teams = match state.store().list_teams_for_user(&user.id, user.is_admin) {
        Ok(teams) => teams,
        Err(error) => return error.into_response(),
    };
    let messages = match conversation(&user, &teams, body) {
        Ok(messages) => messages,
        Err(key) => return failure(StatusCode::BAD_REQUEST, key),
    };
    match complete(&config, &messages).await {
        Ok(answer) => Json(json!({"answer": answer})).into_response(),
        Err(LlmError::RateLimited) => {
            failure(StatusCode::TOO_MANY_REQUESTS, "assistant_rate_limited")
        }
        Err(LlmError::Upstream) => failure(StatusCode::BAD_GATEWAY, "assistant_upstream_error"),
        Err(LlmError::Unreachable) => {
            failure(StatusCode::BAD_GATEWAY, "assistant_upstream_unreachable")
        }
        Err(LlmError::InvalidResponse) => {
            failure(StatusCode::BAD_GATEWAY, "assistant_response_is_invalid")
        }
    }
}

fn conversation(
    user: &AuthUser,
    teams: &[(Team, String)],
    body: ChatBody,
) -> Result<Vec<ChatMessage>, &'static str> {
    let message = body.message.trim();
    if message.is_empty() {
        return Err("assistant_message_is_missing");
    }
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return Err("assistant_message_is_too_long");
    }
    if body.history.len() > MAX_HISTORY_TURNS {
        return Err("assistant_history_is_invalid");
    }
    let mut messages = vec![ChatMessage {
        role: "system".into(),
        content: system_prompt(user, teams),
    }];
    let mut kept = Vec::new();
    let mut used = 0;
    for turn in body.history.into_iter().rev() {
        if turn.role != "user" && turn.role != "assistant" {
            return Err("assistant_history_is_invalid");
        }
        let content = turn.content.trim();
        if content.is_empty() || content.chars().count() > MAX_TURN_CHARS {
            return Err("assistant_history_is_invalid");
        }
        let length = content.chars().count();
        if used + length > MAX_HISTORY_CHARS {
            break;
        }
        used += length;
        kept.push(ChatMessage {
            role: turn.role,
            content: content.to_string(),
        });
    }
    messages.extend(kept.into_iter().rev());
    messages.push(ChatMessage {
        role: "user".into(),
        content: message.to_string(),
    });
    Ok(messages)
}

fn system_prompt(user: &AuthUser, teams: &[(Team, String)]) -> String {
    let mut lines = vec![
        "You are the assistant inside Koreograf, a team-scoped automation tool.".to_string(),
        "Answer only with the access of the signed-in account below.".to_string(),
        "Never invent credentials, secrets, extra variables, or job output.".to_string(),
        format!("Account: {}", user.username),
        format!(
            "Administrator: {}",
            if user.is_admin { "yes" } else { "no" }
        ),
        "Teams and roles:".to_string(),
    ];
    if teams.is_empty() {
        lines.push("- none".to_string());
    }
    for (team, role) in teams {
        lines.push(format!("- {}: {}", team.name, role));
    }
    lines.join("\n")
}

fn within_rate_limit(user_id: &str) -> bool {
    let now = Instant::now();
    let Ok(mut attempts) = CHAT_ATTEMPTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    else {
        return true;
    };
    if attempts.len() > RATE_LIMIT_KEYS {
        attempts.retain(|_, times| {
            times
                .iter()
                .any(|instant| now.saturating_duration_since(*instant) < RATE_LIMIT_WINDOW)
        });
    }
    let times = attempts.entry(user_id.to_string()).or_default();
    times.retain(|instant| now.saturating_duration_since(*instant) < RATE_LIMIT_WINDOW);
    if times.len() >= RATE_LIMIT_REQUESTS {
        return false;
    }
    times.push(now);
    true
}

fn failure(status: StatusCode, key: &str) -> Response {
    (status, Json(json!({"error": key}))).into_response()
}

fn env_string(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}
