use std::collections::BTreeMap;

use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::config::LlmProvider;

pub(super) const DEFAULT_MAX_TOKENS: u32 = 800;
pub(super) const DEFAULT_TIMEOUT_SECS: u64 = 60;

#[derive(Clone, PartialEq, Eq)]
pub struct LlmConfig {
    pub endpoint: String,
    pub model: String,
    pub max_tokens: u32,
    pub timeout_secs: u64,
    pub provider: LlmProvider,
    pub(super) api_key: String,
}

impl std::fmt::Debug for LlmConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LlmConfig")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .field("timeout_secs", &self.timeout_secs)
            .field("provider", &self.provider)
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
    UnreadableSettings,
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
            Self::UnreadableSettings => "the saved assistant settings cannot be read",
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

    pub fn with_provider(mut self, provider: LlmProvider) -> Self {
        self.provider = provider;
        self
    }

    pub(super) fn build(
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
            provider: LlmProvider::Openai,
            api_key: api_key.trim().to_string(),
        })
    }

    pub(super) fn request_url(&self) -> String {
        if self.endpoint.ends_with("/chat/completions") {
            return self.endpoint.clone();
        }
        if self.provider == LlmProvider::Ollama && !self.endpoint.ends_with("/v1") {
            return format!("{}/v1/chat/completions", self.endpoint);
        }
        format!("{}/chat/completions", self.endpoint)
    }

    pub(super) fn has_api_key(&self) -> bool {
        !self.api_key.is_empty()
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatBody {
    pub(super) message: String,
    #[serde(default)]
    pub(super) history: Vec<ChatMessage>,
    #[serde(default)]
    pub(super) job_id: Option<String>,
}

#[derive(Serialize)]
pub struct PendingActionResponse {
    pub(super) id: String,
    pub(super) tool: String,
    pub(super) target: String,
}

#[derive(Debug, Clone)]
pub struct Error {
    pub status: Option<u16>,
    pub key: String,
    pub vars: BTreeMap<String, String>,
}

impl Error {
    pub(super) fn new(key: impl Into<String>) -> Self {
        Self {
            status: None,
            key: key.into(),
            vars: BTreeMap::new(),
        }
    }

    pub fn text(&self) -> String {
        json!({"error": self.key, "vars": self.vars}).to_string()
    }
}

#[derive(Default)]
pub struct JobQuery<'a> {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub q: Option<&'a str>,
    pub status: Option<&'a str>,
    pub tool: Option<&'a str>,
    pub template_id: Option<&'a str>,
    pub team_id: Option<&'a str>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct TeamArgs {
    #[serde(default)]
    pub(super) team_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct IdArgs {
    pub(super) id: String,
    #[serde(default)]
    pub(super) team_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct LaunchArgs {
    pub(super) id: String,
    #[serde(default)]
    pub(super) extra_vars: Map<String, Value>,
    #[serde(default)]
    pub(super) team_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ListJobsArgs {
    #[serde(default)]
    pub(super) limit: Option<i64>,
    #[serde(default)]
    pub(super) offset: Option<i64>,
    #[serde(default)]
    pub(super) q: Option<String>,
    #[serde(default)]
    pub(super) status: Option<String>,
    #[serde(default)]
    pub(super) tool: Option<String>,
    #[serde(default)]
    pub(super) template_id: Option<String>,
    #[serde(default)]
    pub(super) team_id: Option<String>,
}
