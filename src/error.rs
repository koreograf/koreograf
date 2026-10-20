use std::collections::BTreeMap;

use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

pub type AppResult<T> = Result<T, ApiError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserMessage {
    pub key: String,
    pub vars: BTreeMap<String, String>,
}

impl std::fmt::Display for UserMessage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.key)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    BadRequest(UserMessage),
    #[error("{0}")]
    Internal(String),
}

impl ApiError {
    pub fn bad(message: impl Into<String>) -> Self {
        let message = message.into();
        if let Some((key, raw)) = message.split_once('\u{1f}') {
            if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(raw) {
                let vars = map
                    .into_iter()
                    .filter_map(|(name, value)| value.as_str().map(|item| (name, item.to_string())))
                    .collect();
                return Self::BadRequest(UserMessage {
                    key: key.to_string(),
                    vars,
                });
            }
        }
        Self::BadRequest(UserMessage {
            key: message,
            vars: BTreeMap::new(),
        })
    }

    pub fn bad_vars(
        key: impl Into<String>,
        vars: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        Self::BadRequest(UserMessage {
            key: key.into(),
            vars: vars
                .into_iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
        })
    }
}

pub fn message(key: &str, vars: &[(&str, &str)]) -> String {
    let map: BTreeMap<&str, &str> = vars.iter().copied().collect();
    match serde_json::to_string(&map) {
        Ok(encoded) => format!("{key}\u{1f}{encoded}"),
        Err(_) => key.to_string(),
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(err: rusqlite::Error) -> Self {
        let msg = err.to_string();
        if msg.contains("UNIQUE") {
            Self::bad("that_name_is_already_used_in_this_team")
        } else if msg.contains("FOREIGN") || msg.contains("constraint") {
            Self::bad("the_entry_is_still_in_use")
        } else {
            Self::Internal(msg)
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, key, vars) = match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "sign_in_is_required".to_string(),
                BTreeMap::new(),
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "you_are_not_allowed_to_do_this".to_string(),
                BTreeMap::new(),
            ),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found".to_string(),
                BTreeMap::new(),
            ),
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message.key, message.vars),
            Self::Internal(detail) => {
                tracing::error!(error = %detail, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "error.internal".to_string(),
                    BTreeMap::new(),
                )
            }
        };
        let body = if vars.is_empty() {
            json!({ "error": key })
        } else {
            json!({ "error": key, "vars": vars })
        };
        (status, Json(body)).into_response()
    }
}

/// JSON body that rejects a missing or broken payload as a catalog error.
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(_) => Err(ApiError::bad("json_body_required")),
        }
    }
}
