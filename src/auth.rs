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
    pub email: Option<String>,
    pub is_admin: bool,
}

impl From<User> for AuthUser {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            auth_source: user.auth_source,
            email: user.email,
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

/// `launched_by` is an administrator username, or `workflow:<id>` whose
/// own launcher resolves to one. Hooks do not count.
pub fn launcher_is_admin(c: &rusqlite::Connection, launched_by: &str) -> AppResult<bool> {
    principal_is_admin(c, launched_by, 0)
}

fn principal_is_admin(c: &rusqlite::Connection, launched_by: &str, depth: u8) -> AppResult<bool> {
    if depth > 8 {
        return Ok(false);
    }
    if let Some(id) = launched_by.strip_prefix("workflow:") {
        let Some(job) = db::workflow_job(c, id)? else {
            return Ok(false);
        };
        return principal_is_admin(c, &job.launched_by, depth + 1);
    }
    if launched_by.is_empty() || launched_by.starts_with("hook:") {
        return Ok(false);
    }
    let Some(user) = db::user_by_username(c, launched_by)? else {
        return Ok(false);
    };
    Ok(user.is_admin && !user.disabled)
}

/// A local run needs the template, and its project when it has one, to have
/// been saved by an administrator.
pub fn content_saved_by_admin(
    c: &rusqlite::Connection,
    template: &db::Template,
) -> AppResult<bool> {
    if !template.admin_saved {
        return Ok(false);
    }
    let Some(project_id) = template.project_id.as_deref().filter(|id| !id.is_empty()) else {
        return Ok(true);
    };
    Ok(db::project(c, project_id)?.is_some_and(|project| project.admin_saved))
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
        if ldap::group_matches(&keys, group) {
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
