use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Json, Router};

use crate::crypto;
use crate::db;
use crate::error::{ApiError, ApiJson, AppResult};
use crate::features::identity::{prepare_token, AuthUser, TokenBody};
use crate::http::no_content;
use crate::Shared;

use super::model::{
    CreatedTokenResponse, ServiceAccountBody, ServiceAccountDetailResponse,
    ServiceAccountListResponse, ServiceAccountResponse, TokenListResponse, TokenResponse,
};
use super::service;

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/service-accounts", get(list).post(create))
        .route("/service-accounts/{id}", get(detail).delete(delete_account))
        .route(
            "/service-accounts/{id}/tokens",
            get(list_tokens).post(create_token),
        )
        .route(
            "/service-accounts/{id}/tokens/{token_id}",
            delete(delete_token),
        )
}

pub async fn list(
    State(state): State<Shared>,
    user: AuthUser,
) -> AppResult<Json<ServiceAccountListResponse>> {
    let accounts = service::list(&state, &user)?;
    Ok(Json(ServiceAccountListResponse(
        accounts.iter().map(ServiceAccountResponse::new).collect(),
    )))
}

pub async fn create(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<ServiceAccountBody>,
) -> AppResult<(StatusCode, Json<ServiceAccountDetailResponse>)> {
    let account = service::create(&state, &user, body)?;
    let memberships = state.store().list_user_memberships(&account.id)?;
    Ok((
        StatusCode::CREATED,
        Json(ServiceAccountDetailResponse::new(&account, memberships)),
    ))
}

pub async fn detail(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<ServiceAccountDetailResponse>> {
    let account = service::visible_account(&state, &user, &id)?;
    let memberships = state.store().list_user_memberships(&id)?;
    Ok(Json(ServiceAccountDetailResponse::new(
        &account,
        memberships,
    )))
}

pub async fn delete_account(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<axum::response::Response> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    service::delete(&state, &user, &id)?;
    Ok(no_content())
}

pub async fn list_tokens(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<TokenListResponse>> {
    service::visible_account(&state, &user, &id)?;
    let tokens = state.store().list_api_tokens(&id)?;
    Ok(Json(TokenListResponse(
        tokens.iter().map(TokenResponse::new).collect(),
    )))
}

pub async fn create_token(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<TokenBody>,
) -> AppResult<(StatusCode, Json<CreatedTokenResponse>)> {
    let account = service::visible_account(&state, &user, &id)?;
    let token = prepare_token(&body)?;
    let times = db::ApiTokenTimes {
        created_at: token.created_at.clone(),
        expires_at: token.expires_at.clone(),
        scope: token.scope.clone(),
    };
    state.store().create_api_token(
        &token.id,
        &account.id,
        &token.name,
        &token.prefix,
        &crypto::hash_api_token(&token.secret),
        &times,
    )?;
    service::note_token(&state, &user, &account, "create", &token.id, &token.name)?;
    Ok((StatusCode::CREATED, Json(CreatedTokenResponse::new(token))))
}

pub async fn delete_token(
    State(state): State<Shared>,
    user: AuthUser,
    Path((id, token_id)): Path<(String, String)>,
) -> AppResult<axum::response::Response> {
    let account = service::visible_account(&state, &user, &id)?;
    let name = state
        .store()
        .list_api_tokens(&account.id)?
        .into_iter()
        .find(|token| token.id == token_id)
        .map(|token| token.name)
        .ok_or(ApiError::NotFound)?;
    if !state.store().delete_api_token(&token_id, &account.id)? {
        return Err(ApiError::NotFound);
    }
    service::note_token(&state, &user, &account, "delete", &token_id, &name)?;
    Ok(no_content())
}
