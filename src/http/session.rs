use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{self, AuthUser};
use crate::crypto::{self, hash_password};
use crate::db::{self, User};
use crate::error::{ApiError, ApiJson, AppResult};
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
    headers: HeaderMap,
    ApiJson(body): ApiJson<LoginBody>,
) -> Result<Response, ApiError> {
    let username = normalize(&body.username)?;
    if body.password.is_empty() {
        return Err(ApiError::SignInFailed);
    }
    let local = {
        let db = state.db()?;
        db::user_by_username(&db, &username)?
    };
    if auth::local_password_ok(local.as_ref(), &body.password) {
        if let Some(user) = local {
            return session_response(&state, &headers, &user);
        }
    }
    let directory = crate::settings::load(&state)?.directory().cloned();
    if let Some(cfg) = directory {
        match ldap::authenticate(&cfg, &username, &body.password).await {
            Ok(identity) => {
                let user = provision_ldap(&state, identity)?;
                return session_response(&state, &headers, &user);
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    hint = ldap_failure_hint(&err),
                    "LDAP sign-in failed"
                );
                if directory_unreachable(&err) {
                    return Err(ApiError::DirectoryUnavailable);
                }
            }
        }
    }
    Err(ApiError::SignInFailed)
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
        HeaderValue::from_str(&auth::clear_cookie(auth::request_cookie_secure(
            &state.cfg, &headers,
        )))
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
    ApiJson(body): ApiJson<MeUpdate>,
) -> AppResult<Json<Value>> {
    let display = trimmed_display(&body.display_name)?;
    let db = state.db()?;
    let existing = db::user_by_id(&db, &user.id)?.ok_or(ApiError::NotFound)?;
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
    ApiJson(body): ApiJson<UserBody>,
) -> Result<Response, ApiError> {
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
    let db = state.db()?;
    if db::user_by_username(&db, &username)?.is_some() {
        return Err(ApiError::bad("that_username_is_already_used"));
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
    ApiJson(body): ApiJson<UserUpdate>,
) -> AppResult<Json<Value>> {
    require_admin(&user)?;
    let display = trimmed_display(&body.display_name)?;
    let db = state.db()?;
    let existing = db::user_by_id(&db, &id)?.ok_or(ApiError::NotFound)?;
    if body.disabled && id == user.id {
        return Err(ApiError::bad("you_cannot_disable_your_own_account"));
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
        return Err(ApiError::bad("you_cannot_delete_your_own_account"));
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
        if user.auth_source != "ldap" {
            tracing::warn!(
                username = %identity.username,
                "LDAP sign-in matches a local account"
            );
            return Err(ApiError::SignInFailed);
        }
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

fn session_response(state: &Shared, headers: &HeaderMap, user: &User) -> AppResult<Response> {
    if user.disabled {
        return Err(ApiError::bad("the_account_is_disabled"));
    }
    let token = crypto::random_token();
    let db = state.db()?;
    db::create_session(&db, &token, &user.id)?;
    let auth_user = AuthUser::from(user.clone());
    let body = profile_json(&db, &auth_user)?;
    drop(db);
    let mut response = (StatusCode::OK, Json(body)).into_response();
    let cookie = HeaderValue::from_str(&auth::session_cookie(
        &token,
        auth::request_cookie_secure(&state.cfg, headers),
    ))
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
        Err(ApiError::bad("display_name_is_missing"))
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
        Err(ApiError::bad("the_last_administrator_stays"))
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
            "the_password_comes_from_ldap_or_active_directory",
        ));
    }
    if password.len() < 8 {
        return Err(ApiError::bad("password_needs_at_least_8_characters"));
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
                "the_email_comes_from_ldap_or_active_directory",
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
        return Err(ApiError::bad("email_is_invalid"));
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
        return Err(ApiError::bad("email_is_invalid"));
    }
    Ok(Some(value.to_string()))
}

fn directory_unreachable(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    [
        "ldap connection:",
        "ldap tls setup:",
        "service bind:",
        "user search:",
        "ldap bind failed:",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

fn ldap_failure_hint(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("timed out") || lower.contains("timeout") {
        "timeout"
    } else if lower.contains("connection")
        || lower.contains("tls")
        || lower.contains("name resolution")
    {
        "unreachable"
    } else {
        "reject"
    }
}

fn normalize(username: &str) -> AppResult<String> {
    let name = username.trim().to_ascii_lowercase();
    if name.is_empty() || name.len() > 128 || name.chars().any(|ch| ch.is_control()) {
        return Err(ApiError::bad("username_is_invalid"));
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::{directory_unreachable, ldap_failure_hint, normalize_email, profile_email};

    #[test]
    fn ldap_failure_hint_splits_timeout_from_a_rejected_bind() {
        assert_eq!(ldap_failure_hint("LDAP connection: timed out"), "timeout");
        assert_eq!(ldap_failure_hint("operation timeout"), "timeout");
        assert_eq!(
            ldap_failure_hint("LDAP connection: connection refused"),
            "unreachable"
        );
        assert_eq!(ldap_failure_hint("LDAP TLS setup failed"), "unreachable");
        assert_eq!(ldap_failure_hint("ldap_rejected_the_password"), "reject");
        assert_eq!(
            ldap_failure_hint("the_user_was_not_found_uniquely"),
            "reject"
        );
    }

    #[test]
    fn directory_unreachable_covers_transport_failures_but_not_rejections() {
        assert!(directory_unreachable("LDAP connection: connection refused"));
        assert!(directory_unreachable("ldap tls setup: invalid CA file"));
        assert!(directory_unreachable("service bind: timed out"));
        assert!(directory_unreachable("user search: connection reset"));
        assert!(directory_unreachable("LDAP bind failed: timed out"));
        assert!(!directory_unreachable(
            "service bind rejected: invalid credentials"
        ));
        assert!(!directory_unreachable(
            "user search rejected: insufficient access rights"
        ));
        assert!(!directory_unreachable("ldap_rejected_the_password"));
        assert!(!directory_unreachable("the_user_was_not_found_uniquely"));
        assert!(!directory_unreachable("empty_password"));
        assert!(!directory_unreachable("ldap_is_not_fully_configured"));
    }

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
        assert_eq!(
            err.to_string(),
            "the_email_comes_from_ldap_or_active_directory"
        );
        assert_eq!(
            profile_email("ldap", Some("Ada@Example.com"), Some("ada@example.com"))
                .unwrap()
                .as_deref(),
            Some("ada@example.com")
        );
        assert_eq!(profile_email("local", None, Some("")).unwrap(), None);
    }

    #[test]
    fn ldap_sign_in_does_not_take_over_a_local_account() {
        use crate::config::{Config, CookieSecure, LocalJobs, RunnerMode};
        use crate::db;
        use crate::error::ApiError;
        use std::path::PathBuf;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-ldap-local-{nanos}"));
        let state = crate::boot(Config {
            bind: "127.0.0.1:0".into(),
            data_dir: dir.clone(),
            web_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
            examples_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
            admin_user: "admin".into(),
            admin_password: Some("test-admin-pass".into()),
            runner: RunnerMode::Local,
            job_timeout_secs: 60,
            job_output_lines: 8_000,
            local_jobs: LocalJobs::All,
            project_roots: Vec::new(),
            hsts: false,
            cookie_secure: CookieSecure::Auto,
            trusted_proxy: false,
            data_host: None,
            ldap: None,
        })
        .expect("boot");
        let err = super::provision_ldap(
            &state,
            crate::ldap::LdapIdentity {
                username: "admin".into(),
                display_name: "Taken".into(),
                email: Some("taken@example.com".into()),
                groups: vec!["cn=kg-admins,ou=groups,dc=example,dc=com".into()],
            },
        )
        .expect_err("local account");
        assert!(matches!(err, ApiError::SignInFailed));
        let db = state.db().expect("db");
        let user = db::user_by_username(&db, "admin")
            .expect("read")
            .expect("admin");
        assert_eq!(user.auth_source, "local");
        assert!(user.is_admin);
        assert_eq!(user.display_name, "Administrator");
        assert!(user.email.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn boot_with_directory(url: String) -> (crate::Shared, std::path::PathBuf) {
        use crate::config::{Config, CookieSecure, LdapConfig, LocalJobs, RunnerMode};
        use std::path::PathBuf;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-ldap-outage-{nanos}"));
        let state = crate::boot(Config {
            bind: "127.0.0.1:0".into(),
            data_dir: dir.clone(),
            web_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
            examples_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
            admin_user: "admin".into(),
            admin_password: Some("test-admin-pass".into()),
            runner: RunnerMode::Local,
            job_timeout_secs: 60,
            job_output_lines: 8_000,
            local_jobs: LocalJobs::All,
            project_roots: Vec::new(),
            hsts: false,
            cookie_secure: CookieSecure::Auto,
            trusted_proxy: false,
            data_host: None,
            ldap: Some(LdapConfig {
                url,
                mode: "ldap".into(),
                bind_dn: "cn=reader,dc=example,dc=com".into(),
                bind_password: "secret".into(),
                user_base: "ou=people,dc=example,dc=com".into(),
                user_filter: "(uid={username})".into(),
                username_attr: "uid".into(),
                group_attr: "memberOf".into(),
                group_base: String::new(),
                starttls: false,
                tls_insecure: false,
                ca_file: String::new(),
            }),
        })
        .expect("boot");
        (state, dir)
    }

    async fn closed_directory_url() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("ldap://{addr}")
    }

    #[tokio::test]
    async fn ldap_outage_reports_the_directory_as_unreachable() {
        use axum::extract::State;
        use axum::http::{HeaderMap, StatusCode};
        use axum::response::IntoResponse;
        use http_body_util::BodyExt;

        let (state, dir) = boot_with_directory(closed_directory_url().await);
        let error = super::login(
            State(state),
            HeaderMap::new(),
            super::ApiJson(super::LoginBody {
                username: "ada".into(),
                password: "secret".into(),
            }),
        )
        .await
        .expect_err("sign-in must fail while the directory is down");
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("directory_is_unreachable"), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn local_sign_in_still_works_while_the_directory_is_down() {
        use axum::extract::State;
        use axum::http::{HeaderMap, StatusCode};

        let (state, dir) = boot_with_directory(closed_directory_url().await);
        let response = super::login(
            State(state),
            HeaderMap::new(),
            super::ApiJson(super::LoginBody {
                username: "admin".into(),
                password: "test-admin-pass".into(),
            }),
        )
        .await
        .expect("local sign-in");
        assert_eq!(response.status(), StatusCode::OK);
        let _ = std::fs::remove_dir_all(dir);
    }
}
