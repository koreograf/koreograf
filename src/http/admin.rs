use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::auth::{self, AuthUser};
use crate::backup;
use crate::error::{ApiError, AppResult};
use crate::Shared;

pub async fn download(State(state): State<Shared>, user: AuthUser) -> AppResult<Response> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    let bytes = backup::export(&state)?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/zip"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"koreograf-backup.zip\""),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        bytes,
    )
        .into_response())
}

pub async fn restore(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    body: Bytes,
) -> AppResult<Response> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    if headers
        .get("x-restore-confirm")
        .and_then(|value| value.to_str().ok())
        != Some("ERSETZEN")
    {
        return Err(ApiError::bad("confirm_the_replacement_with_ersetzen"));
    }
    backup::restore(&state, &body)?;
    let mut response = (
        StatusCode::OK,
        Json(json!({
            "restored": true,
            "engine": "sqlite",
            "previous": "restore-previous.zip"
        })),
    )
        .into_response();
    let cookie = HeaderValue::from_str(&auth::clear_cookie(auth::request_cookie_secure(
        &state.cfg, &headers,
    )))
    .map_err(|err| ApiError::Internal(err.to_string()))?;
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}
