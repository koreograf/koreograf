use axum::extract::{Path, State};
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

#[derive(Deserialize)]
pub(super) struct MeUpdate {
    display_name: String,
    email: Option<String>,
    #[serde(default)]
    password: String,
}

#[derive(Deserialize)]
pub(super) struct UserUpdate {
    display_name: String,
    is_admin: bool,
    disabled: bool,
    #[serde(default)]
    password: String,
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
    let directory = crate::settings::load(&state)?.directory().cloned();
    if let Some(cfg) = directory {
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
    Ok(Json(profile_json(&db, &user)?))
}

pub async fn update_me(
    State(state): State<Shared>,
    user: AuthUser,
    Json(body): Json<MeUpdate>,
) -> AppResult<Json<Value>> {
    let display = trimmed_display(&body.display_name)?;
    let db = state.db()?;
    let existing = db::user_by_id(&db, &user.id)?.ok_or(ApiError::NotFound)?;
    if existing.auth_source != "local" && !body.password.is_empty() {
        return Err(ApiError::bad(
            "Das Passwort kommt aus LDAP oder Active Directory.",
        ));
    }
    let email = profile_email(
        &existing.auth_source,
        existing.email.as_deref(),
        body.email.as_deref(),
    )?;
    let password_hash = new_password(&existing, &body.password)?;
    db::update_profile(
        &db,
        &user.id,
        display,
        email.as_deref(),
        password_hash.as_deref(),
    )?;
    let updated = db::user_by_id(&db, &user.id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(profile_json(&db, &updated.into())?))
}

pub async fn list_users(State(state): State<Shared>, user: AuthUser) -> AppResult<Json<Value>> {
    require_admin(&user)?;
    let db = state.db()?;
    let users = db::list_users(&db)?;
    Ok(Json(Value::Array(
        users
            .iter()
            .map(|(item, team_count)| account_json(item, *team_count))
            .collect(),
    )))
}

pub async fn get_user(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    require_admin(&user)?;
    let db = state.db()?;
    let found = db::user_by_id(&db, &id)?.ok_or(ApiError::NotFound)?;
    let memberships = db::list_user_memberships(&db, &id)?;
    Ok(Json(account_detail(&found, &memberships)))
}

pub async fn create_user(
    State(state): State<Shared>,
    user: AuthUser,
    Json(body): Json<UserBody>,
) -> Result<Response, ApiError> {
    require_admin(&user)?;
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
    let display = trimmed_display(&body.display_name)?;
    let db = state.db()?;
    if db::user_by_username(&db, &username)?.is_some() {
        return Err(ApiError::bad("Der Benutzername ist schon vergeben."));
    }
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
    Ok(super::created(account_json(&created, 0)))
}

pub async fn update_user(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UserUpdate>,
) -> AppResult<Json<Value>> {
    require_admin(&user)?;
    let display = trimmed_display(&body.display_name)?;
    let db = state.db()?;
    let existing = db::user_by_id(&db, &id)?.ok_or(ApiError::NotFound)?;
    if body.disabled && id == user.id {
        return Err(ApiError::bad(
            "Du kannst dein eigenes Konto nicht deaktivieren.",
        ));
    }
    ensure_last_admin(
        &existing,
        body.is_admin,
        body.disabled,
        db::other_active_admins(&db, &id)?,
    )?;
    let password_hash = new_password(&existing, &body.password)?;
    db::update_user(
        &db,
        &id,
        display,
        body.is_admin,
        body.disabled,
        password_hash.as_deref(),
    )?;
    if id != user.id && (password_hash.is_some() || (body.disabled && !existing.disabled)) {
        db::delete_user_sessions(&db, &id)?;
    }
    let updated = db::user_by_id(&db, &id)?.ok_or(ApiError::NotFound)?;
    let memberships = db::list_user_memberships(&db, &id)?;
    Ok(Json(account_detail(&updated, &memberships)))
}

pub async fn delete_user(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    require_admin(&user)?;
    if id == user.id {
        return Err(ApiError::bad("Du kannst dein eigenes Konto nicht löschen."));
    }
    let db = state.db()?;
    let existing = db::user_by_id(&db, &id)?.ok_or(ApiError::NotFound)?;
    ensure_last_admin(&existing, false, true, db::other_active_admins(&db, &id)?)?;
    if !db::delete_user(&db, &id)? {
        return Err(ApiError::NotFound);
    }
    Ok(super::no_content())
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
    if let Some(raw) = identity.email.as_deref() {
        if let Ok(Some(email)) = normalize_email(raw) {
            db::set_user_email(&db, &user.id, Some(&email))?;
        }
    }
    db::user_by_id(&db, &user.id)?.ok_or(ApiError::NotFound)
}

fn session_response(state: &Shared, user: &User) -> AppResult<Response> {
    if user.disabled {
        return Err(ApiError::bad("Das Konto ist deaktiviert."));
    }
    let token = crypto::random_token();
    let db = state.db()?;
    db::create_session(&db, &token, &user.id)?;
    let auth_user = AuthUser::from(user.clone());
    let body = profile_json(&db, &auth_user)?;
    drop(db);
    let mut response = (StatusCode::OK, Json(body)).into_response();
    let cookie = HeaderValue::from_str(&auth::session_cookie(&token))
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}

fn profile_json(db: &rusqlite::Connection, user: &AuthUser) -> AppResult<Value> {
    let teams = db::list_teams_for_user(db, &user.id, user.is_admin)?;
    let memberships = db::list_user_memberships(db, &user.id)?;
    Ok(user_payload(user, &teams, &memberships))
}

fn user_payload(
    user: &AuthUser,
    teams: &[(db::Team, String)],
    memberships: &[db::UserMembership],
) -> Value {
    json!({
        "id": user.id,
        "username": user.username,
        "display_name": user.display_name,
        "auth_source": user.auth_source,
        "email": user.email,
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
        "memberships": memberships,
    })
}

fn require_admin(user: &AuthUser) -> AppResult<()> {
    if user.is_admin {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn trimmed_display(value: &str) -> AppResult<&str> {
    let display = value.trim();
    if display.is_empty() {
        Err(ApiError::bad("Anzeigename fehlt."))
    } else {
        Ok(display)
    }
}

fn ensure_last_admin(
    existing: &User,
    is_admin: bool,
    disabled: bool,
    others: i64,
) -> AppResult<()> {
    let was_active = existing.is_admin && !existing.disabled;
    let stays_active = is_admin && !disabled;
    if was_active && !stays_active && others == 0 {
        Err(ApiError::bad("Der letzte Administrator bleibt erhalten."))
    } else {
        Ok(())
    }
}

fn new_password(existing: &User, password: &str) -> AppResult<Option<String>> {
    if password.is_empty() {
        return Ok(None);
    }
    if existing.auth_source != "local" {
        return Err(ApiError::bad(
            "Das Passwort kommt aus LDAP oder Active Directory.",
        ));
    }
    if password.len() < 8 {
        return Err(ApiError::bad("Passwort braucht mindestens 8 Zeichen."));
    }
    Ok(Some(hash_password(password)))
}

fn account_json(user: &User, team_count: i64) -> Value {
    json!({
        "id": user.id,
        "username": user.username,
        "display_name": user.display_name,
        "auth_source": user.auth_source,
        "email": user.email,
        "is_admin": user.is_admin,
        "disabled": user.disabled,
        "created_at": user.created_at,
        "team_count": team_count,
    })
}

fn account_detail(user: &User, memberships: &[db::UserMembership]) -> Value {
    let mut value = account_json(user, i64::try_from(memberships.len()).unwrap_or(i64::MAX));
    value["memberships"] = json!(memberships);
    value
}

fn profile_email(
    auth_source: &str,
    current: Option<&str>,
    requested: Option<&str>,
) -> AppResult<Option<String>> {
    let Some(requested) = requested else {
        return Ok(current.map(str::to_string));
    };
    let next = normalize_email(requested)?;
    if auth_source != "local" {
        let same = next
            .as_deref()
            .unwrap_or("")
            .eq_ignore_ascii_case(current.unwrap_or(""));
        if !same {
            return Err(ApiError::bad(
                "Die E-Mail kommt aus LDAP oder Active Directory.",
            ));
        }
    }
    Ok(next)
}

fn normalize_email(value: &str) -> AppResult<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let Some((local, domain)) = value.split_once('@') else {
        return Err(ApiError::bad("E-Mail ist ungültig."));
    };
    if local.is_empty()
        || domain.is_empty()
        || domain.starts_with('.')
        || domain.ends_with('.')
        || value.len() > 254
        || value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(ApiError::bad("E-Mail ist ungültig."));
    }
    Ok(Some(value.to_string()))
}

fn normalize(username: &str) -> AppResult<String> {
    let name = username.trim().to_ascii_lowercase();
    if name.is_empty() || name.len() > 128 || name.chars().any(|ch| ch.is_control()) {
        return Err(ApiError::bad("Benutzername ist ungültig."));
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::{normalize_email, profile_email};

    #[test]
    fn email_trims_and_rejects_a_broken_address() {
        assert_eq!(
            normalize_email("  ada@example.com ").unwrap().as_deref(),
            Some("ada@example.com")
        );
        assert_eq!(normalize_email("  ").unwrap(), None);
        assert!(normalize_email("ada").is_err());
        assert!(normalize_email("ada @example.com").is_err());
    }

    #[test]
    fn ldap_email_stays_with_the_directory() {
        let err =
            profile_email("ldap", Some("ada@example.com"), Some("other@example.com")).unwrap_err();
        assert!(err.to_string().contains("LDAP"));
        assert_eq!(
            profile_email("ldap", Some("Ada@Example.com"), Some("ada@example.com"))
                .unwrap()
                .as_deref(),
            Some("ada@example.com")
        );
        assert_eq!(profile_email("local", None, Some("")).unwrap(), None);
    }
}
