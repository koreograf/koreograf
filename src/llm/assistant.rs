use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::error::{ApiJson, ErrorBody};
use crate::features::identity as auth;
use crate::features::identity::{AuthUser, Role};
use crate::Shared;

use super::{
    api_text, collect_secrets, config_for, conversation, drop_pending, http_status, job_context,
    request_credential, run_tools, take_pending, within_rate_limit, write_request, ChatBody,
    LlmError, LlmSetup, PendingActionResponse, RunError,
};

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/assistant", get(status))
        .route("/assistant/chat", post(chat))
        .route("/assistant/confirm", post(confirm))
        .route("/assistant/cancel", post(cancel))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionBody {
    id: String,
}

#[derive(Serialize)]
pub struct AssistantStatusResponse {
    enabled: bool,
    model: String,
    provider: &'static str,
}

#[derive(Serialize)]
pub struct ChatResponse {
    answer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_action: Option<PendingActionResponse>,
}

#[derive(Serialize)]
pub struct ActionResponse {
    ok: bool,
}

pub async fn status(State(state): State<Shared>, _user: AuthUser) -> Response {
    match config_for(&state) {
        LlmSetup::Configured(config) => Json(AssistantStatusResponse {
            enabled: true,
            model: config.model,
            provider: config.provider.as_str(),
        })
        .into_response(),
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
    headers: HeaderMap,
    ApiJson(body): ApiJson<ChatBody>,
) -> Response {
    let config = match config_for(&state) {
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
    let credential = match request_credential(&headers) {
        Some(credential) => credential,
        None => return failure(StatusCode::UNAUTHORIZED, "sign_in_is_required"),
    };
    let teams = match state.store().list_teams_for_user(&user.id, user.is_admin) {
        Ok(teams) => teams,
        Err(error) => return error.into_response(),
    };
    let team = headers
        .get("x-team-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let secrets = collect_secrets(&state, &user, &teams, &credential);
    let job_context = match body.job_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => {
            match job_context(&state, &credential, id, team.as_deref(), &secrets).await {
                Ok(context) => Some(context),
                Err(_) => {
                    return failure(StatusCode::NOT_FOUND, "assistant_job_is_unavailable");
                }
            }
        }
        _ => None,
    };
    let messages = match conversation(&user, &teams, &body, job_context.as_deref()) {
        Ok(messages) => messages,
        Err(key) => return failure(StatusCode::BAD_REQUEST, key),
    };
    match run_tools(
        &state,
        &user,
        &credential,
        team.as_deref(),
        &config,
        messages,
        &secrets,
    )
    .await
    {
        Ok(outcome) => Json(ChatResponse {
            answer: outcome.answer,
            pending_action: outcome.pending,
        })
        .into_response(),
        Err(RunError::Llm(error)) => llm_failure(error),
        Err(RunError::Key(status, key)) => failure(status, key),
    }
}

pub async fn confirm(
    State(state): State<Shared>,
    user: AuthUser,
    headers: HeaderMap,
    ApiJson(body): ApiJson<ActionBody>,
) -> Response {
    if let Some(response) = gate(&state) {
        return response;
    }
    let Some(action) = take_pending(&body.id, &user.id) else {
        return failure(StatusCode::NOT_FOUND, "assistant_action_is_unknown");
    };
    if auth::authorize(state.store(), &user, &action.team_id, Role::Operator).is_err() {
        return failure(StatusCode::FORBIDDEN, "assistant_write_is_forbidden");
    }
    let Some(credential) = request_credential(&headers) else {
        return failure(StatusCode::UNAUTHORIZED, "sign_in_is_required");
    };
    let (method, path, payload) = write_request(&action.tool, &action.arguments);
    match api_text(
        &state,
        &credential,
        method,
        &path,
        Some(&action.team_id),
        payload.as_ref(),
    )
    .await
    {
        Ok(_) => {
            tracing::info!(
                user = %user.username,
                tool = %action.tool,
                target = %action.name,
                "assistant action executed"
            );
            Json(ActionResponse { ok: true }).into_response()
        }
        Err(key) => failure(http_status(&key), &key),
    }
}

pub async fn cancel(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<ActionBody>,
) -> Response {
    if let Some(response) = gate(&state) {
        return response;
    }
    drop_pending(&body.id, &user.id);
    Json(ActionResponse { ok: true }).into_response()
}

fn gate(state: &Shared) -> Option<Response> {
    match config_for(state) {
        LlmSetup::Configured(_) => None,
        LlmSetup::Disabled => Some(failure(StatusCode::NOT_FOUND, "assistant_is_not_enabled")),
        LlmSetup::Incomplete(_) => Some(failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "assistant_is_not_configured",
        )),
    }
}

fn llm_failure(error: LlmError) -> Response {
    match error {
        LlmError::RateLimited => failure(StatusCode::TOO_MANY_REQUESTS, "assistant_rate_limited"),
        LlmError::Upstream => failure(StatusCode::BAD_GATEWAY, "assistant_upstream_error"),
        LlmError::Unreachable => failure(StatusCode::BAD_GATEWAY, "assistant_upstream_unreachable"),
        LlmError::InvalidResponse => {
            failure(StatusCode::BAD_GATEWAY, "assistant_response_is_invalid")
        }
    }
}

fn failure(status: StatusCode, key: &str) -> Response {
    (status, Json(ErrorBody::key(key))).into_response()
}
