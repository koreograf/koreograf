use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

pub type AppResult<T> = Result<T, ApiError>;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Internal(String),
}

impl ApiError {
    pub fn bad(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(err: rusqlite::Error) -> Self {
        let msg = err.to_string();
        if msg.contains("UNIQUE") {
            Self::BadRequest("Der Name ist in diesem Team schon vergeben.".into())
        } else if msg.contains("FOREIGN") || msg.contains("constraint") {
            Self::BadRequest("Der Eintrag wird noch verwendet.".into())
        } else {
            Self::Internal(msg)
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "Anmeldung erforderlich.".to_string(),
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "Keine Berechtigung für diese Aktion.".to_string(),
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "Nicht gefunden.".to_string()),
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            Self::Internal(msg) => {
                tracing::error!(error = %msg, "interner Fehler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Interner Fehler.".to_string(),
                )
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
