use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use uuid::Uuid;

use crate::crypto::{self, hash_password};
use crate::db;
use crate::error::{ApiError, ApiJson, AppResult};
use crate::http::{no_content, require_name};
use crate::Shared;

use super::model::{
    AccountDetailResponse, AccountListResponse, AccountResponse, AuthUser, ClientPeer,
    CreatedTokenResponse, LoginBody, MeUpdate, ProfileResponse, TokenBody, TokenListResponse,
    TokenResponse, UserBody, UserUpdate,
};
use super::service::{
    authenticate, clear_cookie, client_source, directory_unreachable, disabled_sign_in,
    ensure_last_admin, ldap_failure_hint, local_password_ok, new_password, normalize,
    profile_email, profile_json, provision_ldap, read_cookie, request_cookie_secure, require_admin,
    session_response, signin_failed, signin_remaining, signin_succeeded, throttled,
    trimmed_display,
};

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me).put(update_me))
        .route("/me/tokens", get(list_tokens).post(create_token))
        .route("/me/tokens/{id}", delete(delete_token))
        .route("/users", get(list_users).post(create_user))
        .route(
            "/users/{id}",
            get(get_user).put(update_user).delete(delete_user),
        )
}

pub async fn login(
    State(state): State<Shared>,
    headers: HeaderMap,
    peer: ClientPeer,
    ApiJson(body): ApiJson<LoginBody>,
) -> Result<Response, ApiError> {
    let username = normalize(&body.username)?;
    let source = client_source(&state.cfg, &headers, peer.0);
    if let Some(retry) = signin_remaining(&state.cfg.data_dir, &username, &source) {
        tracing::warn!(
            username = %username,
            client_ip = %source,
            retry_after = retry.as_secs().max(1),
            "sign-in refused after repeated failures"
        );
        return Ok(throttled(retry));
    }
    if body.password.is_empty() {
        signin_failed(&state.cfg.data_dir, &username, &source);
        tracing::warn!(
            username = %username,
            client_ip = %source,
            "sign-in failed"
        );
        return Err(ApiError::SignInFailed);
    }
    let local = state.store().user_by_username(&username)?;
    if local_password_ok(local.as_ref(), &body.password) {
        if let Some(user) = local {
            if user.disabled {
                return Err(disabled_sign_in(&state, &username, &source));
            }
            signin_succeeded(&state.cfg.data_dir, &username, &source);
            return session_response(&state, &headers, &user);
        }
    }
    let directory = crate::settings::load(&state)?.directory().cloned();
    if let Some(cfg) = directory {
        match authenticate(&cfg, &username, &body.password).await {
            Ok(identity) => {
                let user = provision_ldap(&state, identity)?;
                if user.disabled {
                    return Err(disabled_sign_in(&state, &username, &source));
                }
                signin_succeeded(&state.cfg.data_dir, &username, &source);
                return session_response(&state, &headers, &user);
            }
            Err(err) => {
                tracing::warn!(
                    username = %username,
                    client_ip = %source,
                    error = %err,
                    hint = ldap_failure_hint(&err),
                    "LDAP sign-in failed"
                );
                if directory_unreachable(&err) {
                    return Err(ApiError::DirectoryUnavailable);
                }
                signin_failed(&state.cfg.data_dir, &username, &source);
                return Err(ApiError::SignInFailed);
            }
        }
    }
    signin_failed(&state.cfg.data_dir, &username, &source);
    tracing::warn!(
        username = %username,
        client_ip = %source,
        "sign-in failed"
    );
    Err(ApiError::SignInFailed)
}

pub async fn logout(
    State(state): State<Shared>,
    headers: axum::http::HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(token) = read_cookie(&headers, "koreograf_session") {
        state.store().delete_session(&token)?;
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&clear_cookie(request_cookie_secure(&state.cfg, &headers)))
            .map_err(|err| ApiError::Internal(err.to_string()))?,
    );
    Ok(response)
}

pub async fn me(
    State(state): State<Shared>,
    user: AuthUser,
) -> Result<Json<ProfileResponse>, ApiError> {
    Ok(Json(profile_json(&state, &user)?))
}

pub async fn update_me(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<MeUpdate>,
) -> AppResult<Json<ProfileResponse>> {
    let display = trimmed_display(&body.display_name)?;
    let existing = state
        .store()
        .user_by_id(&user.id)?
        .ok_or(ApiError::NotFound)?;
    if existing.auth_source != "local" && !body.password.is_empty() {
        return Err(ApiError::bad(
            "the_password_comes_from_ldap_or_active_directory",
        ));
    }
    let email = profile_email(
        &existing.auth_source,
        existing.email.as_deref(),
        body.email.as_deref(),
    )?;
    let password_hash = new_password(&existing, &body.password)?;
    state.store().update_profile(
        &user.id,
        display,
        email.as_deref(),
        password_hash.as_deref(),
    )?;
    let updated = state
        .store()
        .user_by_id(&user.id)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(profile_json(&state, &updated.into())?))
}

pub async fn list_tokens(
    State(state): State<Shared>,
    user: AuthUser,
) -> AppResult<Json<TokenListResponse>> {
    let tokens = state.store().list_api_tokens(&user.id)?;
    Ok(Json(TokenListResponse(
        tokens.iter().map(TokenResponse::new).collect(),
    )))
}

pub async fn create_token(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<TokenBody>,
) -> Result<(StatusCode, Json<CreatedTokenResponse>), ApiError> {
    let name = require_name(&body.name)?;
    let secret = crypto::api_token();
    let prefix = crypto::token_prefix(&secret);
    let created_at = db::now();
    let id = Uuid::new_v4().to_string();
    state.store().create_api_token(
        &id,
        &user.id,
        &name,
        &prefix,
        &crypto::hash_api_token(&secret),
        &created_at,
    )?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedTokenResponse {
            id,
            name,
            prefix,
            secret,
            created_at,
            last_used_at: None,
        }),
    ))
}

pub async fn delete_token(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    if !state.store().delete_api_token(&id, &user.id)? {
        return Err(ApiError::NotFound);
    }
    Ok(no_content())
}

pub async fn list_users(
    State(state): State<Shared>,
    user: AuthUser,
) -> AppResult<Json<AccountListResponse>> {
    require_admin(&user)?;
    let users = state.store().list_users()?;
    Ok(Json(AccountListResponse(
        users
            .iter()
            .map(|(item, team_count)| AccountResponse::new(item, *team_count))
            .collect(),
    )))
}

pub async fn get_user(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<AccountDetailResponse>> {
    require_admin(&user)?;
    let found = state.store().user_by_id(&id)?.ok_or(ApiError::NotFound)?;
    let memberships = state.store().list_user_memberships(&id)?;
    Ok(Json(AccountDetailResponse::new(&found, &memberships)))
}

pub async fn create_user(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<UserBody>,
) -> Result<(StatusCode, Json<AccountResponse>), ApiError> {
    require_admin(&user)?;
    let username = body.username.trim().to_ascii_lowercase();
    if !username
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-')
        || !(2..=64).contains(&username.len())
    {
        return Err(ApiError::bad("username_2_64_characters_a_z_0_9"));
    }
    if body.password.len() < 8 {
        return Err(ApiError::bad("password_needs_at_least_8_characters"));
    }
    let display = trimmed_display(&body.display_name)?;
    if state.store().user_by_username(&username)?.is_some() {
        return Err(ApiError::bad("that_username_is_already_used"));
    }
    state.store().insert_user(
        &Uuid::new_v4().to_string(),
        &username,
        display,
        Some(&hash_password(&body.password)),
        "local",
        body.is_admin,
    )?;
    let created = state
        .store()
        .user_by_username(&username)?
        .ok_or(ApiError::NotFound)?;
    Ok((StatusCode::CREATED, Json(AccountResponse::new(&created, 0))))
}

pub async fn update_user(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UserUpdate>,
) -> AppResult<Json<AccountDetailResponse>> {
    require_admin(&user)?;
    let display = trimmed_display(&body.display_name)?;
    let existing = state.store().user_by_id(&id)?.ok_or(ApiError::NotFound)?;
    if body.disabled && id == user.id {
        return Err(ApiError::bad("you_cannot_disable_your_own_account"));
    }
    ensure_last_admin(
        &existing,
        body.is_admin,
        body.disabled,
        state.store().other_active_admins(&id)?,
    )?;
    let password_hash = new_password(&existing, &body.password)?;
    state.store().update_user(
        &id,
        display,
        body.is_admin,
        body.disabled,
        password_hash.as_deref(),
    )?;
    if id != user.id && (password_hash.is_some() || (body.disabled && !existing.disabled)) {
        state.store().delete_user_sessions(&id)?;
    }
    let updated = state.store().user_by_id(&id)?.ok_or(ApiError::NotFound)?;
    let memberships = state.store().list_user_memberships(&id)?;
    Ok(Json(AccountDetailResponse::new(&updated, &memberships)))
}

pub async fn delete_user(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    require_admin(&user)?;
    if id == user.id {
        return Err(ApiError::bad("you_cannot_delete_your_own_account"));
    }
    let existing = state.store().user_by_id(&id)?.ok_or(ApiError::NotFound)?;
    ensure_last_admin(
        &existing,
        false,
        true,
        state.store().other_active_admins(&id)?,
    )?;
    if !state.store().delete_user(&id)? {
        return Err(ApiError::NotFound);
    }
    Ok(no_content())
}
