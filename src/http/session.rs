use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{self, AuthUser};
use crate::crypto::{self, hash_password};
use crate::db::{self, User};
use crate::error::{ApiError, AppResult};
use crate::ldap;
use crate::Shared;

#[derive(Deserialize)]
pub struct LoginBody {
    username: String,
    password: String,
}

#[derive(Deserialize)]
pub struct UserBody {
    username: String,
    password: String,
    display_name: String,
    #[serde(default)]
    is_admin: bool,
}

pub async fn login(
    State(state): State<Shared>,
    Json(body): Json<LoginBody>,
) -> Result<Response, ApiError> {
    let username = normalize(&body.username)?;
    if body.password.is_empty() {
        return Err(ApiError::Unauthorized);
    }
    let local = {
        let db = state.db()?;
        db::user_by_username(&db, &username)?
    };
    if auth::local_password_ok(local.as_ref(), &body.password) {
        if let Some(user) = local {
            return session_response(&state, &user);
        }
    }
    if let Some(cfg) = state.cfg.ldap.clone() {
        match ldap::authenticate(&cfg, &username, &body.password).await {
            Ok(identity) => {
                let user = provision_ldap(&state, identity)?;
                return session_response(&state, &user);
            }
            Err(err) => {
                tracing::info!(error = %err, "LDAP-Anmeldung abgelehnt");
            }
        }
    }
    Err(ApiError::Unauthorized)
}

pub async fn logout(
    State(state): State<Shared>,
    headers: axum::http::HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(token) = auth::read_cookie(&headers, "koreograf_session") {
        let db = state.db()?;
        db::delete_session(&db, &token)?;
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&auth::clear_cookie())
            .map_err(|err| ApiError::Internal(err.to_string()))?,
    );
    Ok(response)
}

pub async fn me(State(state): State<Shared>, user: AuthUser) -> Result<Json<Value>, ApiError> {
    let db = state.db()?;
    let teams = db::list_teams_for_user(&db, &user.id, user.is_admin)?;
    Ok(Json(user_payload(&user, &teams)))
}

pub async fn create_user(
    State(state): State<Shared>,
    user: AuthUser,
    Json(body): Json<UserBody>,
) -> Result<Response, ApiError> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    let username = body.username.trim().to_ascii_lowercase();
    if !username
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-')
        || !(2..=64).contains(&username.len())
    {
        return Err(ApiError::bad(
            "Benutzername: 2–64 Zeichen, a-z, 0-9, Punkt, Unterstrich, Bindestrich.",
        ));
    }
    if body.password.len() < 8 {
        return Err(ApiError::bad("Passwort braucht mindestens 8 Zeichen."));
    }
    let display = body.display_name.trim();
    if display.is_empty() {
        return Err(ApiError::bad("Anzeigename fehlt."));
    }
    let db = state.db()?;
    db::insert_user(
        &db,
        &Uuid::new_v4().to_string(),
        &username,
        display,
        Some(&hash_password(&body.password)),
        "local",
        body.is_admin,
    )?;
    let created = db::user_by_username(&db, &username)?.ok_or(ApiError::NotFound)?;
    Ok(super::created(json!({
        "id": created.id,
        "username": created.username,
        "display_name": created.display_name,
        "is_admin": created.is_admin,
    })))
}

fn provision_ldap(state: &Shared, identity: ldap::LdapIdentity) -> AppResult<User> {
    let db = state.db()?;
    let user = if let Some(user) = db::user_by_username(&db, &identity.username)? {
        user
    } else {
        let id = Uuid::new_v4().to_string();
        db::insert_user(
            &db,
            &id,
            &identity.username,
            &identity.display_name,
            None,
            "ldap",
            false,
        )?;
        db::user_by_id(&db, &id)?.ok_or(ApiError::NotFound)?
    };
    auth::sync_ldap_teams(&db, &user.id, &identity.groups)?;
    Ok(user)
}

fn session_response(state: &Shared, user: &User) -> AppResult<Response> {
    let token = crypto::random_token();
    let db = state.db()?;
    db::create_session(&db, &token, &user.id)?;
    let auth_user = AuthUser::from(user.clone());
    let teams = db::list_teams_for_user(&db, &user.id, user.is_admin)?;
    drop(db);
    let mut response = (StatusCode::OK, Json(user_payload(&auth_user, &teams))).into_response();
    let cookie = HeaderValue::from_str(&auth::session_cookie(&token))
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}

fn user_payload(user: &AuthUser, teams: &[(db::Team, String)]) -> Value {
    json!({
        "id": user.id,
        "username": user.username,
        "display_name": user.display_name,
        "auth_source": user.auth_source,
        "is_admin": user.is_admin,
        "teams": teams.iter().map(|(team, role)| json!({
            "id": team.id,
            "name": team.name,
            "description": team.description,
            "ldap_group": team.ldap_group,
            "organization_id": team.organization_id,
            "organization_name": team.organization_name,
            "role": role,
        })).collect::<Vec<_>>(),
    })
}

fn normalize(username: &str) -> AppResult<String> {
    let name = username.trim().to_ascii_lowercase();
    if name.is_empty() || name.len() > 128 || name.chars().any(|ch| ch.is_control()) {
        return Err(ApiError::bad("Benutzername ist ungültig."));
    }
    Ok(name)
}
