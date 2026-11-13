use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use ldap3::{LdapConnAsync, LdapConnSettings, Scope, SearchEntry};
use uuid::Uuid;

use crate::config::LdapConfig;
use crate::crypto::{self, hash_api_token, hash_password, verify_password};
use crate::db::{self, User};
use crate::error::{ApiError, AppResult, ErrorBody};
use crate::Shared;

use super::model::{AuthUser, LdapIdentity, ProfileResponse, Role};

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

pub(super) const SIGNIN_FAILURE_LIMIT: u32 = 5;
pub(super) const SIGNIN_FAILURE_WINDOW: Duration = Duration::from_secs(60);
pub(super) const SIGNIN_LOCKOUT: Duration = Duration::from_secs(60);
pub(super) const SIGNIN_TRACKED_KEYS: usize = 1024;
const UNKNOWN_SOURCE: &str = "unknown";

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct SignInKey {
    pub(super) data_dir: std::path::PathBuf,
    pub(super) username: String,
    pub(super) source: String,
}

pub(super) struct SignInAttempt {
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

pub(super) fn remaining_at(
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

pub(super) fn record_failure_at(
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

pub(super) fn client_source_from(
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
        if crate::features::inventories::loads_config_context(&inventory.source_json)
            || store
                .list_inventory_sources(inventory_id)?
                .iter()
                .any(|source| {
                    source.kind == "netbox"
                        && crate::features::inventories::loads_config_context(&source.source_json)
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
    let keys = group_keys(groups);
    for team in store.teams_with_ldap()? {
        let Some(group) = team.ldap_group.as_deref() else {
            continue;
        };
        if group_matches(&keys, group) {
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

pub fn escape_filter(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '*' => out.push_str(r"\2a"),
            '(' => out.push_str(r"\28"),
            ')' => out.push_str(r"\29"),
            '\\' => out.push_str(r"\5c"),
            '\0' => out.push_str(r"\00"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn cn_from_dn(dn: &str) -> Option<String> {
    let rdns = parse_rdns(dn);
    let first = rdns.first()?;
    if first.len() != 1 {
        return None;
    }
    let ava = &first[0];
    if ava.type_name.eq_ignore_ascii_case("cn") {
        Some(ava.value.clone())
    } else {
        None
    }
}

struct Ava {
    type_name: String,
    value: String,
}

/// RDNs in order. Each RDN is the set of its attribute-value pairs.
fn parse_rdns(dn: &str) -> Vec<Vec<Ava>> {
    let mut rdns = Vec::new();
    for raw_rdn in split_unescaped(dn, b',') {
        let raw_rdn = trim_insignificant(raw_rdn);
        if raw_rdn.is_empty() {
            continue;
        }
        let mut avas = Vec::new();
        for raw_ava in split_unescaped(raw_rdn, b'+') {
            let raw_ava = trim_insignificant(raw_ava);
            let Some((raw_type, raw_value)) = split_once_unescaped(raw_ava, b'=') else {
                continue;
            };
            let type_name = unescape_dn(trim_insignificant(raw_type));
            if type_name.is_empty() {
                continue;
            }
            avas.push(Ava {
                type_name,
                value: unescape_dn(trim_insignificant(raw_value)),
            });
        }
        if !avas.is_empty() {
            rdns.push(avas);
        }
    }
    rdns
}

fn split_unescaped(input: &str, separator: u8) -> Vec<&str> {
    let bytes = input.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += escape_span(&bytes[index..]);
            continue;
        }
        if bytes[index] == separator {
            parts.push(&input[start..index]);
            index += 1;
            start = index;
            continue;
        }
        index += 1;
    }
    parts.push(&input[start..]);
    parts
}

fn split_once_unescaped(input: &str, separator: u8) -> Option<(&str, &str)> {
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += escape_span(&bytes[index..]);
            continue;
        }
        if bytes[index] == separator {
            return Some((&input[..index], &input[index + 1..]));
        }
        index += 1;
    }
    None
}

fn escape_span(bytes: &[u8]) -> usize {
    if bytes.len() >= 3 && bytes[1].is_ascii_hexdigit() && bytes[2].is_ascii_hexdigit() {
        3
    } else if bytes.len() >= 2 {
        2
    } else {
        1
    }
}

/// Spaces next to a separator are not part of the value unless they are escaped.
fn trim_insignificant(input: &str) -> &str {
    let bytes = input.as_bytes();
    let mut start = 0;
    while start < bytes.len() && bytes[start] == b' ' {
        start += 1;
    }
    let mut end = bytes.len();
    while end > start && bytes[end - 1] == b' ' {
        let mut slashes = 0;
        let mut index = end - 1;
        while index > start && bytes[index - 1] == b'\\' {
            slashes += 1;
            index -= 1;
        }
        if slashes % 2 == 1 {
            break;
        }
        end -= 1;
    }
    &input[start..end]
}

fn unescape_dn(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            let rest = &bytes[index..];
            if rest.len() >= 3 && rest[1].is_ascii_hexdigit() && rest[2].is_ascii_hexdigit() {
                let hex = &value[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                    continue;
                }
            }
            if rest.len() >= 2 {
                out.push(rest[1]);
                index += 2;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn escape_rdn_value(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    for (index, ch) in chars.iter().copied().enumerate() {
        let first = index == 0;
        let last = index + 1 == chars.len();
        match ch {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            '+' => out.push_str("\\+"),
            '"' => out.push_str("\\\""),
            '<' => out.push_str("\\<"),
            '>' => out.push_str("\\>"),
            ';' => out.push_str("\\;"),
            '=' => out.push_str("\\="),
            ' ' if first || last => out.push_str("\\ "),
            '#' if first => out.push_str("\\#"),
            other if other.is_control() => {
                let mut buf = [0; 4];
                for byte in other.encode_utf8(&mut buf).bytes() {
                    out.push('\\');
                    out.push(hex_digit(byte >> 4));
                    out.push(hex_digit(byte & 0x0f));
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn hex_digit(nibble: u8) -> char {
    char::from(b"0123456789ABCDEF"[usize::from(nibble)])
}

/// Directory DNs and the CN of each DN, kept apart so a CN cannot match a DN.
#[derive(Debug, Default)]
pub struct GroupKeys {
    dns: HashSet<String>,
    cns: HashSet<String>,
}

/// Accepts memberOf values and entry DNs. Other strings are ignored.
pub fn group_keys(groups: &[String]) -> GroupKeys {
    let mut keys = GroupKeys::default();
    for group in groups {
        if !group.contains('=') {
            continue;
        }
        let normalized = normalize_dn(group);
        if normalized.is_empty() {
            continue;
        }
        keys.dns.insert(normalized);
        if let Some(cn) = cn_from_dn(group) {
            keys.cns.insert(cn.to_ascii_lowercase());
        }
    }
    keys
}

/// A configured value that contains `=` is a DN. Anything else is a CN.
pub fn group_matches(keys: &GroupKeys, ldap_group: &str) -> bool {
    let configured = ldap_group.trim();
    if configured.contains('=') {
        keys.dns.contains(&normalize_dn(configured))
    } else {
        keys.cns.contains(&configured.to_ascii_lowercase())
    }
}

pub(super) fn normalize_dn(dn: &str) -> String {
    let mut rdns = Vec::new();
    for rdn in parse_rdns(dn) {
        let mut avas = rdn
            .iter()
            .map(|ava| {
                let type_name = escape_rdn_value(&ava.type_name.to_ascii_lowercase());
                let value = escape_rdn_value(&ava.value.to_ascii_lowercase());
                format!("{type_name}={value}")
            })
            .collect::<Vec<_>>();
        avas.sort();
        rdns.push(avas.join("+"));
    }
    rdns.join(",")
}

pub async fn authenticate(
    cfg: &LdapConfig,
    username: &str,
    password: &str,
) -> Result<LdapIdentity, String> {
    if password.is_empty() {
        return Err("empty_password".into());
    }
    // An empty service password is an unauthenticated bind. Some directories
    // accept it and then search as anonymous.
    if cfg.bind_dn.is_empty() || cfg.user_base.is_empty() || cfg.bind_password.is_empty() {
        return Err("ldap_is_not_fully_configured".into());
    }
    let settings = match connection_settings(cfg) {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!(url = %cfg.url, %error, "LDAP TLS setup failed");
            return Err(format!("ldap tls setup: {error}"));
        }
    };
    let (conn, mut ldap) = LdapConnAsync::with_settings(settings, &cfg.url)
        .await
        .map_err(|e| format!("LDAP connection: {e}"))?;
    ldap3::drive!(conn);

    ldap.simple_bind(&cfg.bind_dn, &cfg.bind_password)
        .await
        .map_err(|e| format!("service bind: {e}"))?
        .success()
        .map_err(|e| format!("service bind rejected: {e}"))?;

    let filter = cfg
        .user_filter
        .replace("{username}", &escape_filter(username));
    let attrs = vec![
        "cn",
        "displayName",
        "mail",
        cfg.username_attr.as_str(),
        cfg.group_attr.as_str(),
        "memberOf",
    ];
    let (entries, _) = ldap
        .search(&cfg.user_base, Scope::Subtree, &filter, attrs)
        .await
        .map_err(|e| format!("user search: {e}"))?
        .success()
        .map_err(|e| format!("user search rejected: {e}"))?;
    if entries.len() != 1 {
        let _ = ldap.unbind().await;
        return Err("the_user_was_not_found_uniquely".into());
    }
    let Some(raw_entry) = entries.into_iter().next() else {
        let _ = ldap.unbind().await;
        return Err("the_user_was_not_found_uniquely".into());
    };
    let entry = SearchEntry::construct(raw_entry);
    let user_dn = entry.dn.clone();
    let mut groups = attr_values(&entry, &cfg.group_attr);
    if groups.is_empty() && !cfg.group_attr.eq_ignore_ascii_case("memberOf") {
        groups = attr_values(&entry, "memberOf");
    }
    let (account, from_directory) = directory_username(&entry, &cfg.username_attr, username);
    let display = first_attr(&entry, "displayName")
        .or_else(|| first_attr(&entry, "cn"))
        .unwrap_or_else(|| account.clone());

    ldap.simple_bind(&user_dn, password)
        .await
        .map_err(|e| format!("LDAP bind failed: {e}"))?
        .success()
        .map_err(|_| "ldap_rejected_the_password".to_string())?;
    if !from_directory {
        tracing::warn!(
            attribute = %cfg.username_attr,
            "LDAP username attribute is missing, so the entered login is the account name"
        );
    }

    // The user bind only checks the password. Group entries are often
    // readable by the service account alone, so search as that account.
    if groups.is_empty() {
        groups = groups_as_service(&mut ldap, cfg, &user_dn, &account).await;
    }
    let _ = ldap.unbind().await;
    Ok(LdapIdentity {
        username: account,
        display_name: display,
        email: first_attr(&entry, "mail"),
        groups,
    })
}

fn connection_settings(cfg: &LdapConfig) -> Result<LdapConnSettings, String> {
    let mut settings = LdapConnSettings::new()
        .set_starttls(cfg.starttls)
        .set_conn_timeout(Duration::from_secs(8));
    if cfg.tls_insecure {
        settings = settings.set_no_tls_verify(true);
    } else if let Some(config) = crate::tls::ldap_client_config()? {
        settings = settings.set_config(config);
    }
    Ok(settings)
}

fn attr_values(entry: &SearchEntry, name: &str) -> Vec<String> {
    entry
        .attrs
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, values)| values.clone())
        .unwrap_or_default()
}

fn first_attr(entry: &SearchEntry, name: &str) -> Option<String> {
    attr_values(entry, name).into_iter().next()
}

/// Account name from the directory. The bool is false when the attribute is missing.
pub(super) fn directory_username(entry: &SearchEntry, attr: &str, entered: &str) -> (String, bool) {
    let found = first_attr(entry, attr)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    match found {
        Some(value) => (value, true),
        None => (entered.trim().to_ascii_lowercase(), false),
    }
}

async fn groups_as_service(
    ldap: &mut ldap3::Ldap,
    cfg: &LdapConfig,
    user_dn: &str,
    username: &str,
) -> Vec<String> {
    if let Err(error) = rebind_service(ldap, cfg).await {
        let error = redact_secret(&error, &cfg.bind_password);
        tracing::warn!(
            user = username,
            error = %error,
            "LDAP service re-bind failed, so the group search was skipped"
        );
        return Vec::new();
    }
    let base = if cfg.group_base.is_empty() {
        cfg.user_base.as_str()
    } else {
        cfg.group_base.as_str()
    };
    match search_groups(ldap, base, user_dn, username).await {
        Ok(found) => {
            if found.is_empty() && !cfg.group_base.is_empty() {
                tracing::warn!(user = username, "LDAP group search returned no groups");
            }
            found
        }
        Err(error) => {
            let error = redact_secret(&error, &cfg.bind_password);
            tracing::warn!(user = username, error = %error, "LDAP group search failed");
            Vec::new()
        }
    }
}

async fn rebind_service(ldap: &mut ldap3::Ldap, cfg: &LdapConfig) -> Result<(), String> {
    ldap.simple_bind(&cfg.bind_dn, &cfg.bind_password)
        .await
        .map_err(|error| format!("service re-bind: {error}"))?
        .success()
        .map_err(|error| format!("service re-bind rejected: {error}"))?;
    Ok(())
}

pub(super) fn redact_secret(message: &str, secret: &str) -> String {
    if secret.is_empty() {
        message.to_string()
    } else {
        message.replace(secret, "[redacted]")
    }
}

pub(super) fn group_search_filters(user_dn: &str, username: &str) -> Vec<String> {
    let user_dn = escape_filter(user_dn);
    let username = escape_filter(username);
    vec![
        format!("(&(objectClass=groupOfNames)(member={user_dn}))"),
        format!("(&(objectClass=posixGroup)(memberUid={username}))"),
        format!("(&(objectClass=group)(member={user_dn}))"),
        format!("(&(objectClass=groupOfUniqueNames)(uniqueMember={user_dn}))"),
    ]
}

fn push_unique(found: &mut Vec<String>, value: String) {
    if !found.iter().any(|existing| existing == &value) {
        found.push(value);
    }
}

async fn search_groups(
    ldap: &mut ldap3::Ldap,
    base: &str,
    user_dn: &str,
    username: &str,
) -> Result<Vec<String>, String> {
    let mut found = Vec::new();
    let mut errors = Vec::new();
    for filter in group_search_filters(user_dn, username) {
        let searched = ldap
            // 1.1 requests no attributes. Matching uses the entry DN.
            .search(base, Scope::Subtree, &filter, vec!["1.1"])
            .await;
        let entries = match searched {
            Ok(result) => match result.success() {
                Ok((entries, _)) => entries,
                Err(error) => {
                    push_unique(&mut errors, format!("group search rejected: {error}"));
                    continue;
                }
            },
            Err(error) => {
                push_unique(&mut errors, format!("group search: {error}"));
                continue;
            }
        };
        for entry in entries {
            let parsed = SearchEntry::construct(entry);
            if !parsed.dn.is_empty() {
                found.push(parsed.dn);
            }
        }
    }
    found.sort_by_key(|left| left.to_ascii_lowercase());
    found.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    if found.is_empty() && !errors.is_empty() {
        Err(errors.join("; "))
    } else {
        Ok(found)
    }
}

pub(super) fn disabled_sign_in(state: &Shared, username: &str, source: &str) -> ApiError {
    signin_failed(&state.cfg.data_dir, username, source);
    tracing::warn!(
        username = %username,
        client_ip = %source,
        reason = "disabled",
        "sign-in failed"
    );
    ApiError::SignInFailed
}

pub(super) fn throttled(retry: Duration) -> Response {
    let seconds = retry.as_secs().max(1);
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        Json(ErrorBody::key("invalid_username_or_password")),
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

pub(super) fn provision_ldap(state: &Shared, identity: LdapIdentity) -> AppResult<User> {
    let store = state.store();
    let user = if let Some(user) = store.user_by_username(&identity.username)? {
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
        store.insert_user(
            &id,
            &identity.username,
            &identity.display_name,
            None,
            "ldap",
            false,
        )?;
        store.user_by_id(&id)?.ok_or(ApiError::NotFound)?
    };
    sync_ldap_teams(store, &user.id, &identity.groups)?;
    if let Some(raw) = identity.email.as_deref() {
        if let Ok(Some(email)) = normalize_email(raw) {
            store.set_user_email(&user.id, Some(&email))?;
        }
    }
    store.user_by_id(&user.id)?.ok_or(ApiError::NotFound)
}

pub(super) fn session_response(
    state: &Shared,
    headers: &HeaderMap,
    user: &User,
) -> AppResult<Response> {
    if user.disabled {
        tracing::warn!(
            username = %user.username,
            reason = "disabled",
            "sign-in failed"
        );
        return Err(ApiError::SignInFailed);
    }
    let token = crypto::random_token();
    state.store().create_session(&token, &user.id)?;
    let auth_user = AuthUser::from(user.clone());
    let body = profile_json(state, &auth_user)?;
    let mut response = (StatusCode::OK, Json(body)).into_response();
    let cookie = HeaderValue::from_str(&session_cookie(
        &token,
        request_cookie_secure(&state.cfg, headers),
    ))
    .map_err(|err| ApiError::Internal(err.to_string()))?;
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}

pub(super) fn profile_json(state: &Shared, user: &AuthUser) -> AppResult<ProfileResponse> {
    let teams = state.store().list_teams_for_user(&user.id, user.is_admin)?;
    let memberships = state.store().list_user_memberships(&user.id)?;
    Ok(ProfileResponse::new(user, &teams, &memberships))
}

pub(super) fn require_admin(user: &AuthUser) -> AppResult<()> {
    if user.is_admin {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

pub(super) fn trimmed_display(value: &str) -> AppResult<&str> {
    let display = value.trim();
    if display.is_empty() {
        Err(ApiError::bad("display_name_is_missing"))
    } else {
        Ok(display)
    }
}

pub(super) fn ensure_last_admin(
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

pub(super) fn new_password(existing: &User, password: &str) -> AppResult<Option<String>> {
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

pub(super) fn profile_email(
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

pub(super) fn normalize_email(value: &str) -> AppResult<Option<String>> {
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

pub(super) fn directory_unreachable(error: &str) -> bool {
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

pub(super) fn ldap_failure_hint(error: &str) -> &'static str {
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

pub(super) fn normalize(username: &str) -> AppResult<String> {
    let name = username.trim().to_ascii_lowercase();
    if name.is_empty() || name.len() > 128 || name.chars().any(|ch| ch.is_control()) {
        return Err(ApiError::bad("username_is_invalid"));
    }
    Ok(name)
}
