use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tower::ServiceExt;

use crate::config::LlmProvider;
use crate::db::Team;
use crate::features::identity as auth;
use crate::features::identity::{AuthUser, Role};
use crate::features::operations;
use crate::Shared;

mod assistant;

pub(crate) fn routes() -> Router<Shared> {
    assistant::routes()
}

const DEFAULT_MAX_TOKENS: u32 = 800;
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MAX_MESSAGE_CHARS: usize = 8_000;
const MAX_TURN_CHARS: usize = 8_000;
const MAX_HISTORY_TURNS: usize = 20;
const MAX_HISTORY_CHARS: usize = 32_000;
const MAX_TOOL_ROUNDS: usize = 6;
const MAX_TOOL_RESULT_CHARS: usize = 16_000;
const MAX_JOB_OUTPUT_CHARS: usize = 14_000;
const RATE_LIMIT_REQUESTS: usize = 10;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);
const RATE_LIMIT_KEYS: usize = 1_024;
const PENDING_ACTIONS_MAX: usize = 512;
const PENDING_ACTION_TTL: Duration = Duration::from_secs(600);

const WRITE_TOOLS: &[&str] = &[
    "launch_template",
    "launch_workflow",
    "cancel_job",
    "relaunch_job",
    "sync_project",
    "sync_inventory",
];

static CHAT_ATTEMPTS: OnceLock<Mutex<HashMap<String, Vec<Instant>>>> = OnceLock::new();
static PENDING_ACTIONS: OnceLock<Mutex<HashMap<String, PendingAction>>> = OnceLock::new();

#[derive(Clone, PartialEq, Eq)]
pub struct LlmConfig {
    pub endpoint: String,
    pub model: String,
    pub max_tokens: u32,
    pub timeout_secs: u64,
    pub provider: LlmProvider,
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
            provider: LlmProvider::Openai,
            api_key: api_key.trim().to_string(),
        })
    }

    fn request_url(&self) -> String {
        if self.endpoint.ends_with("/chat/completions") {
            return self.endpoint.clone();
        }
        if self.provider == LlmProvider::Ollama && !self.endpoint.ends_with("/v1") {
            return format!("{}/v1/chat/completions", self.endpoint);
        }
        format!("{}/chat/completions", self.endpoint)
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
    parse_with_provider(
        enabled,
        LlmProvider::Openai,
        endpoint,
        model,
        api_key,
        api_key_file,
    )
}

fn parse_with_provider(
    enabled: bool,
    provider: LlmProvider,
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
    .map(|config| Some(config.with_provider(provider)))
}

pub fn from_env() -> LlmSetup {
    let env = crate::config::llm_environment();
    match parse_with_provider(
        env.enabled,
        env.provider,
        &env.base_url,
        &env.model,
        &env.api_key,
        env.api_key_file.as_deref(),
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

pub fn config_for(state: &Shared) -> LlmSetup {
    let effective = match operations::load(state) {
        Ok(effective) => effective,
        Err(_) => return LlmSetup::Incomplete(LlmConfigError::UnreadableSettings),
    };
    if !effective.saved {
        return from_env();
    }
    if !effective.llm_enabled {
        return LlmSetup::Disabled;
    }
    match parse_with_provider(
        true,
        effective.llm_provider,
        &effective.llm_url,
        &effective.llm_model,
        &effective.llm_api_key,
        None,
    ) {
        Ok(Some(config)) => LlmSetup::Configured(config),
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
    let wire = messages
        .iter()
        .map(|message| json!({"role": message.role, "content": message.content}))
        .collect::<Vec<_>>();
    let reply = request_chat(config, &wire, None).await?;
    if reply.content.is_empty() {
        return Err(LlmError::InvalidResponse);
    }
    Ok(reply.content)
}

struct ChatReply {
    message: Value,
    content: String,
    calls: Vec<ToolCall>,
}

struct ToolCall {
    id: String,
    name: String,
    arguments: Value,
}

async fn request_chat(
    config: &LlmConfig,
    messages: &[Value],
    tools: Option<&Value>,
) -> Result<ChatReply, LlmError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.timeout_secs))
        .build()
        .map_err(|_| LlmError::Unreachable)?;
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "max_tokens": config.max_tokens,
        "stream": false,
    });
    if let Some(tools) = tools {
        body["tools"] = tools.clone();
        body["tool_choice"] = json!("auto");
    }
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
    let message = payload["choices"][0]["message"]
        .as_object()
        .map(|object| Value::Object(object.clone()))
        .ok_or(LlmError::InvalidResponse)?;
    let content = message["content"].as_str().unwrap_or("").trim().to_string();
    let calls = message["tool_calls"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let name = item["function"]["name"].as_str()?.to_string();
                    let id = item["id"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("call_{name}"));
                    let arguments = item["function"]["arguments"]
                        .as_str()
                        .and_then(|raw| serde_json::from_str(raw).ok())
                        .unwrap_or_else(|| json!({}));
                    Some(ToolCall {
                        id,
                        name,
                        arguments,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ChatReply {
        message,
        content,
        calls,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatBody {
    message: String,
    #[serde(default)]
    history: Vec<ChatMessage>,
    #[serde(default)]
    job_id: Option<String>,
}

#[derive(Serialize)]
pub struct PendingActionResponse {
    id: String,
    tool: String,
    target: String,
}

enum RunError {
    Llm(LlmError),
    Key(StatusCode, &'static str),
}

struct ToolOutcome {
    answer: String,
    pending: Option<PendingActionResponse>,
}

async fn run_tools(
    state: &Shared,
    user: &AuthUser,
    credential: &Credential,
    team: Option<&str>,
    config: &LlmConfig,
    messages: Vec<ChatMessage>,
    secrets: &SecretSet,
) -> Result<ToolOutcome, RunError> {
    let mut wire = messages
        .iter()
        .map(|message| json!({"role": message.role, "content": message.content}))
        .collect::<Vec<_>>();
    let tools = tool_specs();
    let mut pending: Option<PendingActionResponse> = None;
    for _ in 0..MAX_TOOL_ROUNDS {
        let reply = request_chat(config, &wire, Some(&tools))
            .await
            .map_err(RunError::Llm)?;
        wire.push(reply.message);
        if reply.calls.is_empty() {
            return Ok(ToolOutcome {
                answer: reply.content,
                pending,
            });
        }
        for call in reply.calls {
            if is_write(&call.name) {
                if pending.is_some() {
                    return Err(RunError::Key(
                        StatusCode::BAD_REQUEST,
                        "assistant_action_is_ambiguous",
                    ));
                }
                let action_team = tool_team(&call.arguments, team).ok_or(RunError::Key(
                    StatusCode::BAD_REQUEST,
                    "assistant_write_needs_a_team",
                ))?;
                if auth::authorize(state.store(), user, &action_team, Role::Operator).is_err() {
                    return Err(RunError::Key(
                        StatusCode::FORBIDDEN,
                        "assistant_write_is_forbidden",
                    ));
                }
                let name =
                    action_target(state, credential, &call.name, &call.arguments, &action_team)
                        .await;
                let id = store_pending(user, &action_team, &call.name, &call.arguments, &name);
                tracing::info!(
                    user = %user.username,
                    tool = %call.name,
                    target = %name,
                    "assistant write needs confirmation"
                );
                pending = Some(PendingActionResponse {
                    id,
                    tool: call.name,
                    target: name,
                });
                return Ok(ToolOutcome {
                    answer: reply.content,
                    pending,
                });
            }
            let result = read_tool(
                state,
                credential,
                &call.name,
                &call.arguments,
                team,
                secrets,
            )
            .await
            .map_err(|key| RunError::Key(StatusCode::BAD_GATEWAY, key))?;
            wire.push(json!({
                "role": "tool",
                "tool_call_id": call.id,
                "content": result,
            }));
        }
    }
    Err(RunError::Key(
        StatusCode::BAD_GATEWAY,
        "assistant_tool_limit_reached",
    ))
}

async fn read_tool(
    state: &Shared,
    credential: &Credential,
    name: &str,
    arguments: &Value,
    team: Option<&str>,
    secrets: &SecretSet,
) -> Result<String, &'static str> {
    let call_team = tool_team(arguments, team);
    let id = arguments.get("id").and_then(Value::as_str).unwrap_or("");
    let result = match name {
        "whoami" => {
            api_json(
                state,
                credential,
                Method::GET,
                "/api/me",
                call_team.as_deref(),
            )
            .await
        }
        "version" => {
            api_json(
                state,
                credential,
                Method::GET,
                "/api/version",
                call_team.as_deref(),
            )
            .await
        }
        "list_projects" => {
            api_json(
                state,
                credential,
                Method::GET,
                "/api/projects",
                call_team.as_deref(),
            )
            .await
        }
        "list_inventories" => {
            api_json(
                state,
                credential,
                Method::GET,
                "/api/inventories",
                call_team.as_deref(),
            )
            .await
        }
        "list_credentials" => {
            api_json(
                state,
                credential,
                Method::GET,
                "/api/credentials",
                call_team.as_deref(),
            )
            .await
        }
        "list_templates" => {
            api_json(
                state,
                credential,
                Method::GET,
                "/api/templates",
                call_team.as_deref(),
            )
            .await
        }
        "list_workflows" => {
            api_json(
                state,
                credential,
                Method::GET,
                "/api/workflows",
                call_team.as_deref(),
            )
            .await
        }
        "list_schedules" => {
            api_json(
                state,
                credential,
                Method::GET,
                "/api/schedules",
                call_team.as_deref(),
            )
            .await
        }
        "list_jobs" => {
            let path = jobs_path(arguments);
            api_json(state, credential, Method::GET, &path, call_team.as_deref()).await
        }
        "get_job" => {
            let path = format!("/api/jobs/{}", encode_segment(id));
            api_json(state, credential, Method::GET, &path, call_team.as_deref()).await
        }
        "job_output" => {
            let path = format!("/api/jobs/{}/output", encode_segment(id));
            api_text(
                state,
                credential,
                Method::GET,
                &path,
                call_team.as_deref(),
                None,
            )
            .await
            .map(Value::String)
        }
        "get_workflow_job" => {
            let path = format!("/api/workflow-jobs/{}", encode_segment(id));
            api_json(state, credential, Method::GET, &path, call_team.as_deref()).await
        }
        _ => return Err("assistant_tool_is_unknown"),
    };
    result
        .map(|value| {
            let text = match value {
                Value::String(text) => redact_text(&text, secrets),
                other => redact_value(&other, secrets).to_string(),
            };
            truncate_chars(&text, MAX_TOOL_RESULT_CHARS)
        })
        .map_err(|_| "assistant_tool_failed")
}

fn jobs_path(arguments: &Value) -> String {
    let mut parts = Vec::new();
    for key in ["limit", "offset", "q", "status", "tool", "template_id"] {
        let Some(value) = arguments.get(key) else {
            continue;
        };
        let text = match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            _ => continue,
        };
        if text.is_empty() {
            continue;
        }
        parts.push(format!("{}={}", key, encode_segment(&text)));
    }
    if parts.is_empty() {
        "/api/jobs".to_string()
    } else {
        format!("/api/jobs?{}", parts.join("&"))
    }
}

async fn action_target(
    state: &Shared,
    credential: &Credential,
    tool: &str,
    arguments: &Value,
    team: &str,
) -> String {
    let id = arguments.get("id").and_then(Value::as_str).unwrap_or("");
    let list = match tool {
        "launch_template" => Some("/api/templates"),
        "launch_workflow" => Some("/api/workflows"),
        "sync_project" => Some("/api/projects"),
        "sync_inventory" => Some("/api/inventories"),
        _ => None,
    };
    if let Some(path) = list {
        if let Ok(Value::Array(items)) =
            api_json(state, credential, Method::GET, path, Some(team)).await
        {
            if let Some(name) = items
                .iter()
                .find(|item| item["id"].as_str() == Some(id))
                .and_then(|item| item["name"].as_str())
            {
                return name.to_string();
            }
        }
        return id.to_string();
    }
    let path = format!("/api/jobs/{}", encode_segment(id));
    if let Ok(job) = api_json(state, credential, Method::GET, &path, Some(team)).await {
        if let Some(name) = job["name"].as_str() {
            return name.to_string();
        }
    }
    id.to_string()
}

async fn job_context(
    state: &Shared,
    credential: &Credential,
    id: &str,
    team: Option<&str>,
    secrets: &SecretSet,
) -> Result<String, ()> {
    let path = format!("/api/jobs/{}", encode_segment(id));
    let job = api_json(state, credential, Method::GET, &path, team)
        .await
        .map_err(|_| ())?;
    let output_path = format!("/api/jobs/{}/output", encode_segment(id));
    let output = api_text(state, credential, Method::GET, &output_path, team, None)
        .await
        .unwrap_or_default();
    let metadata = redact_value(&job, secrets);
    let output = truncate_chars(&redact_text(&output, secrets), MAX_JOB_OUTPUT_CHARS);
    Ok(format!(
        "Job metadata from the API (already redacted):\n{metadata}\nJob output (already redacted):\n{output}"
    ))
}

fn write_request(tool: &str, arguments: &Value) -> (Method, String, Option<Value>) {
    let id = arguments.get("id").and_then(Value::as_str).unwrap_or("");
    let segment = encode_segment(id);
    let extra_vars = || {
        arguments
            .get("extra_vars")
            .cloned()
            .unwrap_or_else(|| json!({}))
    };
    match tool {
        "launch_template" => (
            Method::POST,
            format!("/api/templates/{segment}/launch"),
            Some(json!({"extra_vars": extra_vars()})),
        ),
        "launch_workflow" => (
            Method::POST,
            format!("/api/workflows/{segment}/launch"),
            Some(json!({"extra_vars": extra_vars()})),
        ),
        "cancel_job" => (Method::POST, format!("/api/jobs/{segment}/cancel"), None),
        "relaunch_job" => (
            Method::POST,
            format!("/api/jobs/{segment}/relaunch"),
            Some(json!({})),
        ),
        "sync_project" => (Method::POST, format!("/api/projects/{segment}/sync"), None),
        _ => (
            Method::POST,
            format!("/api/inventories/{segment}/sync"),
            None,
        ),
    }
}

fn tool_team(arguments: &Value, header: Option<&str>) -> Option<String> {
    arguments
        .get("team_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| header.map(str::to_string))
}

fn is_write(name: &str) -> bool {
    WRITE_TOOLS.contains(&name)
}

fn conversation(
    user: &AuthUser,
    teams: &[(Team, String)],
    body: &ChatBody,
    job_context: Option<&str>,
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
    let mut system = system_prompt(user, teams);
    if let Some(context) = job_context {
        system.push_str("\n\n");
        system.push_str(context);
    }
    let mut messages = vec![ChatMessage {
        role: "system".into(),
        content: system,
    }];
    let mut kept = Vec::new();
    let mut used = 0;
    for turn in body.history.iter().rev() {
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
            role: turn.role.clone(),
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
        "Use the read tools to ground answers about jobs, templates, projects, and inventories."
            .to_string(),
        "Write tools never run on their own. A write tool call only produces a confirmation card for the user."
            .to_string(),
        "Never invent credentials, secrets, extra variables, or job output."
            .to_string(),
        "Never repeat redacted values. Values shown as [redacted] stay hidden.".to_string(),
        "Answer in the language of the user.".to_string(),
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

#[derive(Clone)]
struct PendingAction {
    user_id: String,
    team_id: String,
    tool: String,
    arguments: Value,
    name: String,
    created: Instant,
}

fn store_pending(user: &AuthUser, team: &str, tool: &str, arguments: &Value, name: &str) -> String {
    let id = crate::crypto::random_token();
    let now = Instant::now();
    if let Ok(mut actions) = PENDING_ACTIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        actions
            .retain(|_, action| now.saturating_duration_since(action.created) < PENDING_ACTION_TTL);
        if actions.len() >= PENDING_ACTIONS_MAX {
            actions.clear();
        }
        actions.insert(
            id.clone(),
            PendingAction {
                user_id: user.id.clone(),
                team_id: team.to_string(),
                tool: tool.to_string(),
                arguments: arguments.clone(),
                name: name.to_string(),
                created: now,
            },
        );
    }
    id
}

fn take_pending(id: &str, user_id: &str) -> Option<PendingAction> {
    let mut actions = PENDING_ACTIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()?;
    let action = actions.remove(id)?;
    if action.user_id != user_id
        || Instant::now().saturating_duration_since(action.created) >= PENDING_ACTION_TTL
    {
        return None;
    }
    Some(action)
}

fn drop_pending(id: &str, user_id: &str) {
    if let Ok(mut actions) = PENDING_ACTIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        if actions
            .get(id)
            .is_some_and(|action| action.user_id == user_id)
        {
            actions.remove(id);
        }
    }
}

#[derive(Clone)]
enum Credential {
    Cookie(String),
    Bearer(String),
}

fn request_credential(headers: &HeaderMap) -> Option<Credential> {
    if let Some(token) = auth::bearer_token(headers) {
        return Some(Credential::Bearer(token));
    }
    auth::read_cookie(headers, "koreograf_session").map(Credential::Cookie)
}

fn credential_secret(credential: &Credential) -> &str {
    match credential {
        Credential::Cookie(token) | Credential::Bearer(token) => token,
    }
}

async fn api_text(
    state: &Shared,
    credential: &Credential,
    method: Method,
    path: &str,
    team: Option<&str>,
    body: Option<&Value>,
) -> Result<String, String> {
    let mut builder = axum::http::Request::builder().method(method).uri(path);
    builder = match credential {
        Credential::Cookie(token) => {
            builder.header(header::COOKIE, format!("koreograf_session={token}"))
        }
        Credential::Bearer(token) => {
            builder.header(header::AUTHORIZATION, format!("Bearer {token}"))
        }
    };
    if let Some(team) = team {
        builder = builder.header("x-team-id", team);
    }
    let request = if let Some(body) = body {
        builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .map_err(|_| "request_failed".to_string())?
    } else {
        builder
            .body(Body::empty())
            .map_err(|_| "request_failed".to_string())?
    };
    let response = crate::http::api_router(state.clone())
        .oneshot(request)
        .await
        .map_err(|_| "request_failed".to_string())?;
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .map_err(|_| "request_failed".to_string())?
        .to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    if status.is_success() {
        Ok(text)
    } else {
        Err(error_key(&text))
    }
}

async fn api_json(
    state: &Shared,
    credential: &Credential,
    method: Method,
    path: &str,
    team: Option<&str>,
) -> Result<Value, String> {
    let text = api_text(state, credential, method, path, team, None).await?;
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|_| "invalid_response".to_string())
}

fn error_key(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value["error"].as_str().map(str::to_string))
        .unwrap_or_else(|| "request_failed".to_string())
}

fn http_status(key: &str) -> StatusCode {
    match key {
        "sign_in_is_required" | "unauthorized" => StatusCode::UNAUTHORIZED,
        "forbidden" => StatusCode::FORBIDDEN,
        "not_found" => StatusCode::NOT_FOUND,
        _ => StatusCode::BAD_REQUEST,
    }
}

#[derive(Default)]
struct SecretSet {
    values: Vec<String>,
}

impl SecretSet {
    fn add(&mut self, value: &str) {
        let value = value.trim();
        if value.chars().count() < 4 {
            return;
        }
        if !self.values.iter().any(|item| item == value) {
            self.values.push(value.to_string());
            self.values
                .sort_by_key(|left| std::cmp::Reverse(left.len()));
        }
    }

    fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        for value in &self.values {
            if out.contains(value) {
                out = out.replace(value, "[redacted]");
            }
        }
        out
    }
}

fn collect_secrets(
    state: &Shared,
    _user: &AuthUser,
    teams: &[(Team, String)],
    credential: &Credential,
) -> SecretSet {
    let mut secrets = SecretSet::default();
    secrets.add(credential_secret(credential));
    if let Ok(effective) = operations::load(state) {
        secrets.add(&effective.ldap.bind_password);
        secrets.add(&effective.proxy.password);
        secrets.add(&effective.llm_api_key);
    }
    for (team, _) in teams {
        let Ok(credentials) = state.store().list_credentials(&team.id) else {
            continue;
        };
        for item in credentials {
            let Ok(plain) = state.decrypt(&item.secret_blob) else {
                continue;
            };
            let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(&plain) else {
                continue;
            };
            for (key, value) in fields {
                if crate::features::credentials::is_secret_field(&item.kind, &key) {
                    collect_strings(&value, &mut secrets);
                }
            }
        }
    }
    secrets
}

fn collect_strings(value: &Value, secrets: &mut SecretSet) {
    match value {
        Value::String(text) => secrets.add(text),
        Value::Array(items) => {
            for item in items {
                collect_strings(item, secrets);
            }
        }
        Value::Object(fields) => {
            for value in fields.values() {
                collect_strings(value, secrets);
            }
        }
        _ => {}
    }
}

fn redact_text(text: &str, secrets: &SecretSet) -> String {
    let text = secrets.redact(text);
    let text = redact_private_keys(&text);
    let text = crate::runner::redact_secrets(&text);
    let text = redact_assignments(&text);
    let text = redact_prefixed_tokens(&text);
    redact_vault_blocks(&text)
}

fn redact_assignments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find(['=', ':']) {
        if rest[index..].starts_with("://") || rest[index..].starts_with(": //") {
            out.push_str(&rest[..=index]);
            rest = &rest[index + 1..];
            continue;
        }
        let key_start = rest[..index]
            .rfind(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
            .map(|position| position + 1)
            .unwrap_or(0);
        let key = rest[key_start..index].trim();
        if key.is_empty() || !secret_field(key) {
            out.push_str(&rest[..=index]);
            rest = &rest[index + 1..];
            continue;
        }
        let mut value_start = index + 1;
        while rest[value_start..]
            .chars()
            .next()
            .is_some_and(|ch| ch == ' ' || ch == '\t' || ch == '"' || ch == '\'')
        {
            value_start += 1;
        }
        let value_end = rest[value_start..]
            .find(|ch: char| {
                ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | '&' | '}' | ']' | ';')
            })
            .map(|offset| value_start + offset)
            .unwrap_or(rest.len());
        out.push_str(&rest[..value_start]);
        if rest[value_start..].starts_with("[redacted]") {
            rest = &rest[value_start..];
            continue;
        }
        out.push_str("[redacted]");
        rest = &rest[value_end..];
    }
    out.push_str(rest);
    out
}

fn redact_private_keys(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("-----BEGIN ") {
        let Some(end) = rest[start..].find("-----END ") else {
            break;
        };
        let end = start + end;
        let Some(after) = rest[end + 5..].find("-----") else {
            break;
        };
        let end = end + 5 + after + 5;
        out.push_str(&rest[..start]);
        out.push_str("[redacted private key]");
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn redact_vault_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_vault = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("$ANSIBLE_VAULT") {
            if !in_vault {
                out.push_str("[redacted vault]\n");
            }
            in_vault = true;
            continue;
        }
        if in_vault {
            if trimmed.trim().is_empty() {
                in_vault = false;
            }
            continue;
        }
        out.push_str(line);
    }
    out
}

const TOKEN_PREFIXES: &[&str] = &[
    "sk-",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "AKIA",
    "ASIA",
    "eyJ",
];

fn redact_prefixed_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let Some((index, prefix)) = TOKEN_PREFIXES
            .iter()
            .filter_map(|prefix| rest.find(prefix).map(|index| (index, *prefix)))
            .min_by_key(|(index, _)| *index)
        else {
            out.push_str(rest);
            break;
        };
        let boundary = index == 0
            || !rest[..index]
                .chars()
                .next_back()
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
        if !boundary {
            out.push_str(&rest[..index + prefix.len()]);
            rest = &rest[index + prefix.len()..];
            continue;
        }
        out.push_str(&rest[..index]);
        let end = rest[index..]
            .find(|ch: char| {
                ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | '}' | ']' | ')' | ';')
            })
            .unwrap_or(rest.len() - index);
        let token = &rest[index..index + end];
        if token.len() >= 12 {
            out.push_str("[redacted]");
        } else {
            out.push_str(token);
        }
        rest = &rest[index + end..];
    }
    out
}

fn redact_value(value: &Value, secrets: &SecretSet) -> Value {
    match value {
        Value::String(text) => Value::String(redact_text(text, secrets)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_value(item, secrets))
                .collect(),
        ),
        Value::Object(fields) => {
            let mut out = Map::new();
            for (key, value) in fields {
                if key == "extra_vars" || secret_field(key) {
                    out.insert(key.clone(), Value::String("[redacted]".into()));
                } else {
                    out.insert(key.clone(), redact_value(value, secrets));
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn secret_field(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    if key.ends_with("_id") || key.ends_with("_ids") || key.ends_with("_name") {
        return false;
    }
    [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "private_key",
        "passphrase",
        "authorization",
    ]
    .iter()
    .any(|marker| key.contains(marker))
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("\n[truncated]");
    out
}

fn encode_segment(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'~' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn tool_specs() -> Value {
    let team = json!({"type": "object", "properties": {"team_id": {"type": "string"}}});
    let id = json!({
        "type": "object",
        "properties": {"id": {"type": "string"}, "team_id": {"type": "string"}},
        "required": ["id"]
    });
    let launch = json!({
        "type": "object",
        "properties": {
            "id": {"type": "string"},
            "extra_vars": {"type": "object"},
            "team_id": {"type": "string"}
        },
        "required": ["id"]
    });
    let jobs = json!({
        "type": "object",
        "properties": {
            "limit": {"type": "integer"},
            "offset": {"type": "integer"},
            "q": {"type": "string"},
            "status": {"type": "string"},
            "tool": {"type": "string"},
            "template_id": {"type": "string"},
            "team_id": {"type": "string"}
        }
    });
    let tool = |name: &str, description: &str, parameters: Value| {
        json!({"type": "function", "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }})
    };
    json!([
        tool("whoami", "Return the signed-in account.", team.clone()),
        tool("version", "Return the server version.", team.clone()),
        tool(
            "list_projects",
            "List the projects of the selected team.",
            team.clone()
        ),
        tool(
            "list_inventories",
            "List the inventories of the selected team.",
            team.clone()
        ),
        tool(
            "list_credentials",
            "List credentials. Secrets stay hidden by the API.",
            team.clone()
        ),
        tool(
            "list_templates",
            "List the templates of the selected team.",
            team.clone()
        ),
        tool(
            "list_workflows",
            "List the workflows of the selected team.",
            team.clone()
        ),
        tool(
            "list_schedules",
            "List the schedules of the selected team.",
            team.clone()
        ),
        tool("list_jobs", "List jobs with optional filters.", jobs),
        tool("get_job", "Return one job by id.", id.clone()),
        tool(
            "job_output",
            "Return the redacted output of one job.",
            id.clone()
        ),
        tool(
            "get_workflow_job",
            "Return one workflow job by id.",
            id.clone()
        ),
        tool(
            "launch_template",
            "Ask to launch a template. The user confirms first.",
            launch.clone()
        ),
        tool(
            "launch_workflow",
            "Ask to launch a workflow. The user confirms first.",
            launch
        ),
        tool(
            "cancel_job",
            "Ask to cancel a job. The user confirms first.",
            id.clone()
        ),
        tool(
            "relaunch_job",
            "Ask to relaunch a job. The user confirms first.",
            id.clone()
        ),
        tool(
            "sync_project",
            "Ask to sync a project. The user confirms first.",
            id.clone()
        ),
        tool(
            "sync_inventory",
            "Ask to sync an inventory. The user confirms first.",
            id
        ),
    ])
}

fn env_string(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets(values: &[&str]) -> SecretSet {
        let mut set = SecretSet::default();
        for value in values {
            set.add(value);
        }
        set
    }

    #[test]
    fn known_secret_values_never_survive_redaction() {
        let set = secrets(&["bind-secret-value", "proxy-pass"]);
        let text = "ldap rejected bind-secret-value; proxy-pass works";
        let redacted = redact_text(text, &set);
        assert!(!redacted.contains("bind-secret-value"), "{redacted}");
        assert!(!redacted.contains("proxy-pass"), "{redacted}");
        assert!(redacted.contains("[redacted]"), "{redacted}");
    }

    #[test]
    fn secret_assignments_bearer_tokens_and_urls_are_masked() {
        let text = "DB_PASSWORD=hunter2 Authorization: Bearer abc123token git https://user:pw@example/repo.git";
        let redacted = redact_text(text, &SecretSet::default());
        assert!(!redacted.contains("hunter2"), "{redacted}");
        assert!(!redacted.contains("abc123token"), "{redacted}");
        assert!(!redacted.contains("user:pw"), "{redacted}");
        assert!(
            redacted.contains("https://[redacted]@example/repo.git"),
            "{redacted}"
        );
    }

    #[test]
    fn secret_shaped_tokens_and_private_keys_are_masked() {
        let openai = ["sk-", "live-abcdefghijklmnopqrstuvwxyz"].concat();
        let github = ["ghp_", "0123456789abcdef"].concat();
        let begin = ["-----BEGIN ", "OPENSSH", " PRIVATE KEY-----"].concat();
        let end = ["-----END ", "OPENSSH", " PRIVATE KEY-----"].concat();
        let text = format!("token {openai} and {github}\n{begin}\nbody\n{end}");
        let redacted = redact_text(&text, &SecretSet::default());
        assert!(
            !redacted.contains("live-abcdefghijklmnopqrstuvwxyz"),
            "{redacted}"
        );
        assert!(!redacted.contains("0123456789abcdef"), "{redacted}");
        assert!(!redacted.contains("body"), "{redacted}");
        assert!(redacted.contains("[redacted private key]"), "{redacted}");
    }

    #[test]
    fn extra_vars_values_never_reach_the_model() {
        let job = json!({
            "id": "job-1",
            "name": "Deploy",
            "extra_vars": {"api_password": "hunter2", "plain": "visible"},
            "parameters": {"script": "echo ok"},
            "template": {"token": "ghp_secretvalue", "credential_id": "cred-1"}
        });
        let redacted = redact_value(&job, &SecretSet::default());
        assert_eq!(redacted["extra_vars"], "[redacted]");
        assert_eq!(redacted["template"]["token"], "[redacted]");
        assert_eq!(redacted["template"]["credential_id"], "cred-1");
        assert_eq!(redacted["parameters"]["script"], "echo ok");
        let text = redacted.to_string();
        assert!(!text.contains("hunter2"), "{text}");
        assert!(!text.contains("ghp_secretvalue"), "{text}");
    }

    #[test]
    fn vault_blocks_are_masked() {
        let text = "$ANSIBLE_VAULT;1.1;AES256\n63386436\nmore\n\nplain tail";
        let redacted = redact_text(text, &SecretSet::default());
        assert!(!redacted.contains("63386436"), "{redacted}");
        assert!(redacted.contains("plain tail"), "{redacted}");
    }

    #[test]
    fn a_long_output_is_truncated_after_redaction() {
        let text = format!("{} secret-in-the-tail", "x".repeat(60));
        let redacted = truncate_chars(&redact_text(&text, &secrets(&["secret-in-the-tail"])), 20);
        assert!(redacted.ends_with("[truncated]"), "{redacted}");
        assert!(!redacted.contains("secret-in-the-tail"), "{redacted}");
    }

    #[test]
    fn only_write_tools_need_a_confirmation() {
        for name in WRITE_TOOLS {
            assert!(is_write(name), "{name}");
        }
        assert!(!is_write("job_output"));
        assert!(!is_write("launch_everything"));
    }
}
