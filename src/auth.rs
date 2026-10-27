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

/// A local run needs every object that supplies the command to have been saved
/// by an administrator. `template` carries the project and inventory that will
/// actually run. When `requires_workflow` is set, `workflow_id` must name a
/// workflow an administrator saved. That covers a node override and a stored
/// snapshot that is not the current template.
pub fn content_saved_by_admin(
    c: &rusqlite::Connection,
    template: &db::Template,
    workflow_id: Option<&str>,
    requires_workflow: bool,
) -> AppResult<bool> {
    if !template.admin_saved {
        return Ok(false);
    }
    if let Some(project_id) = template.project_id.as_deref().filter(|id| !id.is_empty()) {
        if !db::project(c, project_id)?.is_some_and(|project| project.admin_saved) {
            return Ok(false);
        }
    }
    if let Some(inventory_id) = template.inventory_id.as_deref().filter(|id| !id.is_empty()) {
        if !db::inventory(c, inventory_id)?.is_some_and(|item| item.admin_saved) {
            return Ok(false);
        }
    }
    if !requires_workflow {
        return Ok(true);
    }
    let Some(workflow_id) = workflow_id.filter(|id| !id.is_empty()) else {
        return Ok(false);
    };
    Ok(db::workflow(c, workflow_id)?.is_some_and(|item| item.admin_saved))
}

/// True when `params` would run the same command as `template`. A missing
/// `tool` means the runner keeps the template. `spec.limit` is ignored because
/// relaunch of failed hosts adds it.
pub(crate) fn command_matches_template(
    template: &db::Template,
    params: &serde_json::Value,
) -> bool {
    let Some(tool) = params.get("tool").and_then(serde_json::Value::as_str) else {
        return true;
    };
    if tool != template.tool {
        return false;
    }
    if let Some(entrypoint) = params.get("entrypoint").and_then(serde_json::Value::as_str) {
        if entrypoint != template.entrypoint {
            return false;
        }
    }
    if saved_target(params, "project_id") != template.project_id {
        return false;
    }
    if saved_target(params, "inventory_id") != template.inventory_id {
        return false;
    }
    specs_match(&template.spec, params)
}

fn saved_target(params: &serde_json::Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn specs_match(template_spec: &str, params: &serde_json::Value) -> bool {
    let Some(saved) = params.get("spec").filter(|value| value.is_object()) else {
        return true;
    };
    spec_map(saved) == spec_map(&db::json_object(template_spec))
}

fn spec_map(value: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    let mut map = value.as_object().cloned().unwrap_or_default();
    map.remove("limit");
    map
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

pub fn session_cookie(token: &str, secure: bool) -> String {
    let mut cookie =
        format!("koreograf_session={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

pub fn clear_cookie(secure: bool) -> String {
    let mut cookie = String::from("koreograf_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

pub fn request_cookie_secure(cfg: &crate::config::Config, headers: &HeaderMap) -> bool {
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok());
    cfg.cookie_secure.enabled(cfg.trusted_proxy, proto)
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

#[cfg(test)]
mod tests {
    use super::{clear_cookie, command_matches_template, session_cookie};
    use crate::db::Template;
    use serde_json::json;

    #[test]
    fn cookies_carry_secure_only_when_asked() {
        let plain = session_cookie("tok", false);
        assert_eq!(
            plain,
            "koreograf_session=tok; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800"
        );
        let secure = session_cookie("tok", true);
        assert_eq!(format!("{plain}; Secure"), secure);
        assert_eq!(
            clear_cookie(false),
            "koreograf_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0"
        );
        assert_eq!(
            clear_cookie(true),
            "koreograf_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0; Secure"
        );
    }

    fn template(spec: &str) -> Template {
        Template {
            id: "t".into(),
            team_id: "team".into(),
            name: "Heal".into(),
            description: String::new(),
            tool: "ansible".into(),
            project_id: Some("project".into()),
            inventory_id: Some("inventory".into()),
            credential_id: None,
            environment_id: None,
            entrypoint: "site.yml".into(),
            extra_vars: "{}".into(),
            spec: spec.into(),
            created_at: String::new(),
            update_on_launch: true,
            admin_saved: true,
        }
    }

    #[test]
    fn a_failed_host_limit_does_not_change_the_command() {
        let template = template(r#"{"tags":"heal"}"#);
        let limited = json!({
            "tool": "ansible",
            "entrypoint": "site.yml",
            "project_id": "project",
            "inventory_id": "inventory",
            "spec": {"tags": "heal", "limit": "web-1,db-1"}
        });
        assert!(command_matches_template(&template, &limited));
        let replaced = json!({
            "tool": "shell",
            "entrypoint": "",
            "project_id": "project",
            "inventory_id": "inventory",
            "spec": {"script": "true"}
        });
        assert!(!command_matches_template(&template, &replaced));
    }
}
