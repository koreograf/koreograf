use std::sync::OnceLock;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::HeaderMap;

use crate::crypto::{hash_password, verify_password};
use crate::db::{self, User};
use crate::error::{ApiError, AppResult};
use crate::ldap;
use crate::Shared;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer = 1,
    Operator = 2,
    Owner = 3,
}

impl Role {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "viewer" => Some(Self::Viewer),
            "operator" => Some(Self::Operator),
            "owner" => Some(Self::Owner),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Operator => "operator",
            Self::Owner => "owner",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuthUser {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub auth_source: String,
    pub is_admin: bool,
}

impl From<User> for AuthUser {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            auth_source: user.auth_source,
            username: user.username,
            display_name: user.display_name,
            is_admin: user.is_admin,
        }
    }
}

impl FromRequestParts<Shared> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Shared,
    ) -> Result<Self, Self::Rejection> {
        let token =
            read_cookie(&parts.headers, "koreograf_session").ok_or(ApiError::Unauthorized)?;
        let db = state.db()?;
        let user = db::user_by_session(&db, &token)?.ok_or(ApiError::Unauthorized)?;
        if user.disabled {
            db::delete_session(&db, &token)?;
            return Err(ApiError::Unauthorized);
        }
        Ok(user.into())
    }
}

pub fn authorize(
    c: &rusqlite::Connection,
    user: &AuthUser,
    team_id: &str,
    min: Role,
) -> AppResult<()> {
    if user.is_admin {
        return Ok(());
    }
    let Some(role) = db::role_of(c, team_id, &user.id)? else {
        return Err(ApiError::Forbidden);
    };
    let Some(role) = Role::parse(&role) else {
        return Err(ApiError::Forbidden);
    };
    if role >= min {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

pub fn local_password_ok(user: Option<&User>, password: &str) -> bool {
    let stored = user
        .and_then(|item| item.password_hash.as_deref())
        .unwrap_or_else(|| dummy_hash());
    let matches = verify_password(stored, password);
    matches && user.is_some_and(|item| item.password_hash.is_some())
}

fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| hash_password("not-a-real-password"))
}

pub fn sync_ldap_teams(
    c: &rusqlite::Connection,
    user_id: &str,
    groups: &[String],
) -> AppResult<()> {
    let keys = ldap::group_keys(groups);
    for team in db::teams_with_ldap(c)? {
        let Some(group) = team.ldap_group.as_deref() else {
            continue;
        };
        if keys.contains(&group.to_ascii_lowercase()) {
            db::upsert_membership(c, &team.id, user_id, Role::Operator.as_str(), "ldap")?;
        } else {
            db::delete_ldap_membership(c, &team.id, user_id)?;
        }
    }
    Ok(())
}

pub fn session_cookie(token: &str) -> String {
    format!("koreograf_session={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800")
}

pub fn clear_cookie() -> String {
    "koreograf_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0".into()
}

pub fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    let prefix = format!("{name}=");
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(&prefix) {
            return Some(value.to_string());
        }
    }
    None
}
