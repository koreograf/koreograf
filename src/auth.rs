use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::HeaderMap;

use crate::crypto::{hash_api_token, hash_password, verify_password};
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
        if let Some(token) = bearer_token(&parts.headers) {
            if token.is_empty() {
                return Err(ApiError::Unauthorized);
            }
            return api_token_user(state, &token)?.ok_or(ApiError::Unauthorized);
        }
        let token =
            read_cookie(&parts.headers, "koreograf_session").ok_or(ApiError::Unauthorized)?;
        let user = state
            .store()
            .user_by_session(&token)?
            .ok_or(ApiError::Unauthorized)?;
        if user.disabled {
            state.store().delete_session(&token)?;
            return Err(ApiError::Unauthorized);
        }
        Ok(user.into())
    }
}

pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| {
            let (scheme, token) = raw.split_once(' ').unwrap_or((raw, ""));
            scheme
                .eq_ignore_ascii_case("bearer")
                .then(|| token.trim().to_string())
        })
}

pub fn api_token_user(state: &Shared, token: &str) -> AppResult<Option<AuthUser>> {
    let Some(user) = state.store().user_by_api_token(&hash_api_token(token))? else {
        return Ok(None);
    };
    if user.disabled {
        return Ok(None);
    }
    Ok(Some(user.into()))
}

pub fn authorize(
    store: &dyn crate::backend::Backend,
    user: &AuthUser,
    team_id: &str,
    min: Role,
) -> AppResult<()> {
    if user.is_admin {
        return Ok(());
    }
    let Some(role) = store.role_of(team_id, &user.id)? else {
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

const SIGNIN_FAILURE_LIMIT: u32 = 5;
const SIGNIN_FAILURE_WINDOW: Duration = Duration::from_secs(60);
const SIGNIN_LOCKOUT: Duration = Duration::from_secs(60);
const SIGNIN_TRACKED_KEYS: usize = 1024;
const UNKNOWN_SOURCE: &str = "unknown";

#[derive(Clone, PartialEq, Eq, Hash)]
struct SignInKey {
    data_dir: std::path::PathBuf,
    username: String,
    source: String,
}

struct SignInAttempt {
    failures: u32,
    first: Instant,
    locked_until: Option<Instant>,
}

static SIGNIN_ATTEMPTS: OnceLock<Mutex<HashMap<SignInKey, SignInAttempt>>> = OnceLock::new();

fn signin_attempts() -> &'static Mutex<HashMap<SignInKey, SignInAttempt>> {
    SIGNIN_ATTEMPTS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn signin_remaining(data_dir: &Path, username: &str, source: &str) -> Option<Duration> {
    if source == UNKNOWN_SOURCE {
        return None;
    }
    let key = signin_key(data_dir, username, source);
    let mut attempts = signin_attempts().lock().ok()?;
    remaining_at(&mut attempts, &key, Instant::now())
}

pub(crate) fn signin_failed(data_dir: &Path, username: &str, source: &str) {
    if source == UNKNOWN_SOURCE {
        return;
    }
    let key = signin_key(data_dir, username, source);
    if let Ok(mut attempts) = signin_attempts().lock() {
        record_failure_at(&mut attempts, key, Instant::now());
    }
}

pub(crate) fn signin_succeeded(data_dir: &Path, username: &str, source: &str) {
    if source == UNKNOWN_SOURCE {
        return;
    }
    let key = signin_key(data_dir, username, source);
    if let Ok(mut attempts) = signin_attempts().lock() {
        attempts.remove(&key);
    }
}

fn signin_key(data_dir: &Path, username: &str, source: &str) -> SignInKey {
    SignInKey {
        data_dir: data_dir.to_path_buf(),
        username: username.to_string(),
        source: source.to_string(),
    }
}

fn remaining_at(
    attempts: &mut HashMap<SignInKey, SignInAttempt>,
    key: &SignInKey,
    now: Instant,
) -> Option<Duration> {
    prune_attempts(attempts, now);
    attempts.get(key).and_then(|attempt| {
        attempt
            .locked_until
            .and_then(|until| until.checked_duration_since(now))
            .filter(|remaining| !remaining.is_zero())
    })
}

fn record_failure_at(
    attempts: &mut HashMap<SignInKey, SignInAttempt>,
    key: SignInKey,
    now: Instant,
) {
    prune_attempts(attempts, now);
    let current = key.clone();
    let attempt = attempts.entry(key).or_insert_with(|| SignInAttempt {
        failures: 0,
        first: now,
        locked_until: None,
    });
    if now.saturating_duration_since(attempt.first) >= SIGNIN_FAILURE_WINDOW {
        attempt.failures = 0;
        attempt.first = now;
        attempt.locked_until = None;
    }
    attempt.failures += 1;
    if attempt.failures >= SIGNIN_FAILURE_LIMIT {
        attempt.locked_until = Some(now + SIGNIN_LOCKOUT);
    }
    if attempts.len() > SIGNIN_TRACKED_KEYS {
        evict_attempt(attempts, &current);
    }
}

fn evict_attempt(attempts: &mut HashMap<SignInKey, SignInAttempt>, current: &SignInKey) {
    let oldest_unlocked = attempts
        .iter()
        .filter(|(_, attempt)| attempt.locked_until.is_none())
        .min_by_key(|(_, attempt)| attempt.first)
        .map(|(key, _)| key.clone());
    match oldest_unlocked {
        Some(key) => {
            attempts.remove(&key);
        }
        None => {
            attempts.remove(current);
        }
    }
}

fn prune_attempts(attempts: &mut HashMap<SignInKey, SignInAttempt>, now: Instant) {
    attempts.retain(|_, attempt| match attempt.locked_until {
        Some(until) => now < until,
        None => now.saturating_duration_since(attempt.first) < SIGNIN_FAILURE_WINDOW,
    });
}

pub(crate) fn client_source(
    cfg: &crate::config::Config,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> String {
    client_source_from(cfg.trusted_proxy, headers, peer)
}

fn client_source_from(
    trusted_proxy: bool,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> String {
    if trusted_proxy {
        if let Some(source) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
        {
            return source.to_string();
        }
    }
    match peer {
        Some(address) => address.ip().to_string(),
        None => "unknown".to_string(),
    }
}

/// `launched_by` is an administrator username, or `workflow:<id>` whose
/// own launcher resolves to one. Hooks do not count.
pub fn launcher_is_admin(
    store: &dyn crate::backend::Backend,
    launched_by: &str,
) -> AppResult<bool> {
    principal_is_admin(store, launched_by, 0)
}

fn principal_is_admin(
    store: &dyn crate::backend::Backend,
    launched_by: &str,
    depth: u8,
) -> AppResult<bool> {
    if depth > 8 {
        return Ok(false);
    }
    if let Some(id) = launched_by.strip_prefix("workflow:") {
        let Some(job) = store.workflow_job(id)? else {
            return Ok(false);
        };
        return principal_is_admin(store, &job.launched_by, depth + 1);
    }
    if launched_by.is_empty() || launched_by.starts_with("hook:") {
        return Ok(false);
    }
    let Some(user) = store.user_by_username(launched_by)? else {
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
    store: &dyn crate::backend::Backend,
    template: &db::Template,
    workflow_id: Option<&str>,
    requires_workflow: bool,
) -> AppResult<bool> {
    if !template.admin_saved {
        return Ok(false);
    }
    if let Some(project_id) = template.project_id.as_deref().filter(|id| !id.is_empty()) {
        if !store
            .project(project_id)?
            .is_some_and(|project| project.admin_saved)
        {
            return Ok(false);
        }
    }
    if let Some(inventory_id) = template.inventory_id.as_deref().filter(|id| !id.is_empty()) {
        let Some(inventory) = store.inventory(inventory_id)? else {
            return Ok(false);
        };
        if !inventory.admin_saved {
            return Ok(false);
        }
        if crate::netbox::loads_config_context(&inventory.source_json)
            || store
                .list_inventory_sources(inventory_id)?
                .iter()
                .any(|source| {
                    source.kind == "netbox"
                        && crate::netbox::loads_config_context(&source.source_json)
                })
        {
            return Ok(false);
        }
    }
    if !requires_workflow {
        return Ok(true);
    }
    let Some(workflow_id) = workflow_id.filter(|id| !id.is_empty()) else {
        return Ok(false);
    };
    Ok(store
        .workflow(workflow_id)?
        .is_some_and(|item| item.admin_saved))
}

/// True when `params` would run the same command as `template`. A missing
/// `tool` means the runner keeps the template. `spec.limit` is ignored because
/// relaunch of failed hosts adds it. A template that asks at launch also
/// ignores inventory, tags, skip tags, job type, and verbosity.
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
    let prompted = template.prompt_on_launch
        || params
            .get("launch_with_options")
            .and_then(serde_json::Value::as_bool)
            == Some(true);
    if !prompted && saved_target(params, "inventory_id") != template.inventory_id {
        return false;
    }
    specs_match(&template.spec, params, prompted)
}

fn saved_target(params: &serde_json::Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn specs_match(template_spec: &str, params: &serde_json::Value, prompt: bool) -> bool {
    let Some(saved) = params.get("spec").filter(|value| value.is_object()) else {
        return true;
    };
    spec_map(saved, prompt) == spec_map(&db::json_object(template_spec), prompt)
}

fn spec_map(value: &serde_json::Value, prompt: bool) -> serde_json::Map<String, serde_json::Value> {
    let mut map = value.as_object().cloned().unwrap_or_default();
    map.remove("limit");
    if prompt {
        for key in ["tags", "skip_tags", "job_type", "verbosity"] {
            map.remove(key);
        }
    }
    map
}

pub fn sync_ldap_teams(
    store: &dyn crate::backend::Backend,
    user_id: &str,
    groups: &[String],
) -> AppResult<()> {
    let keys = ldap::group_keys(groups);
    for team in store.teams_with_ldap()? {
        let Some(group) = team.ldap_group.as_deref() else {
            continue;
        };
        if ldap::group_matches(&keys, group) {
            store.upsert_membership(&team.id, user_id, Role::Operator.as_str(), "ldap")?;
        } else {
            store.delete_ldap_membership(&team.id, user_id)?;
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
    use super::{
        clear_cookie, client_source_from, command_matches_template, record_failure_at,
        remaining_at, session_cookie, SignInKey, SIGNIN_FAILURE_LIMIT, SIGNIN_FAILURE_WINDOW,
        SIGNIN_LOCKOUT, SIGNIN_TRACKED_KEYS,
    };
    use crate::db::Template;
    use axum::http::HeaderMap;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Instant;

    fn signin_key(username: &str, source: &str) -> SignInKey {
        SignInKey {
            data_dir: PathBuf::from("/tmp/koreograf-throttle-test"),
            username: username.into(),
            source: source.into(),
        }
    }

    #[test]
    fn repeated_failures_lock_one_account_and_source_until_the_lockout_ends() {
        let mut attempts = HashMap::new();
        let key = signin_key("ada", "203.0.113.10");
        let start = Instant::now();
        for _ in 0..SIGNIN_FAILURE_LIMIT - 1 {
            record_failure_at(&mut attempts, key.clone(), start);
        }
        assert_eq!(remaining_at(&mut attempts, &key, start), None);
        record_failure_at(&mut attempts, key.clone(), start);
        assert!(remaining_at(&mut attempts, &key, start).is_some());
        assert!(remaining_at(&mut attempts, &key, start + SIGNIN_LOCKOUT / 2).is_some());
        assert_eq!(
            remaining_at(&mut attempts, &key, start + SIGNIN_LOCKOUT),
            None
        );
        assert!(!attempts.contains_key(&key));
    }

    #[test]
    fn the_lockout_is_scoped_to_the_username_and_source() {
        let mut attempts = HashMap::new();
        let key = signin_key("ada", "203.0.113.10");
        let start = Instant::now();
        for _ in 0..SIGNIN_FAILURE_LIMIT {
            record_failure_at(&mut attempts, key.clone(), start);
        }
        assert!(remaining_at(&mut attempts, &key, start).is_some());
        assert_eq!(
            remaining_at(&mut attempts, &signin_key("ada", "203.0.113.11"), start),
            None
        );
        assert_eq!(
            remaining_at(&mut attempts, &signin_key("grace", "203.0.113.10"), start),
            None
        );
    }

    #[test]
    fn spraying_usernames_does_not_evict_a_lock() {
        let mut attempts = HashMap::new();
        let start = Instant::now();
        let locked = signin_key("admin", "203.0.113.10");
        for _ in 0..SIGNIN_FAILURE_LIMIT {
            record_failure_at(&mut attempts, locked.clone(), start);
        }
        for index in 0..SIGNIN_TRACKED_KEYS {
            record_failure_at(
                &mut attempts,
                signin_key(&format!("user{index}"), "203.0.113.99"),
                start,
            );
        }
        assert!(remaining_at(&mut attempts, &locked, start).is_some());
    }

    #[test]
    fn new_keys_are_not_tracked_once_every_slot_holds_a_lock() {
        let mut attempts = HashMap::new();
        let start = Instant::now();
        for index in 0..SIGNIN_TRACKED_KEYS {
            let key = signin_key(&format!("locked{index}"), "203.0.113.99");
            for _ in 0..SIGNIN_FAILURE_LIMIT {
                record_failure_at(&mut attempts, key.clone(), start);
            }
        }
        let extra = signin_key("extra", "203.0.113.99");
        record_failure_at(&mut attempts, extra.clone(), start);
        assert!(!attempts.contains_key(&extra));
        assert_eq!(attempts.len(), SIGNIN_TRACKED_KEYS);
    }

    #[test]
    fn failures_older_than_the_window_do_not_add_up() {
        let mut attempts = HashMap::new();
        let key = signin_key("ada", "203.0.113.10");
        let start = Instant::now();
        for _ in 0..SIGNIN_FAILURE_LIMIT - 1 {
            record_failure_at(&mut attempts, key.clone(), start);
        }
        for _ in 0..SIGNIN_FAILURE_LIMIT - 1 {
            record_failure_at(
                &mut attempts,
                key.clone(),
                start + SIGNIN_FAILURE_WINDOW + SIGNIN_FAILURE_WINDOW / 2,
            );
        }
        assert_eq!(
            remaining_at(
                &mut attempts,
                &key,
                start + SIGNIN_FAILURE_WINDOW + SIGNIN_FAILURE_WINDOW / 2
            ),
            None
        );
    }

    #[test]
    fn the_forwarded_address_is_only_used_when_the_proxy_is_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.7, 10.0.0.1".parse().unwrap());
        let peer = "192.0.2.5:4321".parse().ok();
        assert_eq!(client_source_from(true, &headers, peer), "198.51.100.7");
        assert_eq!(client_source_from(false, &headers, peer), "192.0.2.5");
        assert_eq!(client_source_from(false, &headers, None), "unknown");
        headers.insert("x-forwarded-for", "not-an-address".parse().unwrap());
        assert_eq!(client_source_from(true, &headers, peer), "192.0.2.5");
    }

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
            no_environment: true,
            entrypoint: "site.yml".into(),
            extra_vars: "{}".into(),
            spec: spec.into(),
            created_at: String::new(),
            update_on_launch: true,
            admin_saved: true,
            prompt_on_launch: false,
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
        let mut prompted = template.clone();
        prompted.prompt_on_launch = true;
        let varied = json!({
            "tool": "ansible",
            "entrypoint": "site.yml",
            "project_id": "project",
            "inventory_id": "other",
            "spec": {"tags": "other", "skip_tags": "slow", "job_type": "check", "verbosity": 2, "limit": "db"}
        });
        assert!(command_matches_template(&prompted, &varied));
        assert!(!command_matches_template(&template, &varied));
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
