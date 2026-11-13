use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::backup::Backup;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_postgres::Client;
use uuid::Uuid;

use crate::backend::Backend;
use crate::cacheinfo::{self, CacheReport};
use crate::config::{
    self, Config, DatabaseConfig, LdapConfig, LlmProvider, PostgresConfig, RunnerMode, SslMode,
    OUTPUT_LINES_MAX, OUTPUT_LINES_MIN,
};
use crate::crypto;
use crate::datalock::lock_data_directory;
use crate::db;
use crate::error::{ApiError, AppResult};
use crate::portable;
use crate::Shared;

use super::model::{
    AuditEntry, CacheGauge, CheckResult, Effective, ImportReport, JobLabels, JobMeta, LlmWrite,
    MigrationStatus, Options, ProxySettings, RestoreSummary, SettingsWrite, TableProgress,
    DATABASE_CHECK_IDS, DEFAULT_ENVIRONMENT_KEYS, REPORT_KEY,
};

const SAVED: &str = "saved";
const SECRET_MARK: &str = "********";
const LDAP_PASSWORD: &str = "ldap_bind_password";
const PROXY_PASSWORD: &str = "proxy_password";
const LLM_API_KEY: &str = "llm_api_key";
const PROXY_ENV: &[&str] = &[
    "http_proxy",
    "https_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "all_proxy",
    "ALL_PROXY",
    "no_proxy",
    "NO_PROXY",
];

impl ProxySettings {
    #[cfg(test)]
    pub fn inherit() -> Self {
        let (mut proxy, _) = proxy_from_env();
        proxy.explicit = false;
        proxy
    }

    pub fn is_set(&self) -> bool {
        !self.url.is_empty()
    }

    pub fn env_pairs(&self) -> Vec<(String, String)> {
        if self.url.is_empty() {
            return Vec::new();
        }
        let value = self.url_with_auth();
        let mut pairs = vec![
            ("http_proxy".into(), value.clone()),
            ("https_proxy".into(), value.clone()),
            ("HTTP_PROXY".into(), value.clone()),
            ("HTTPS_PROXY".into(), value),
        ];
        if !self.no_proxy.is_empty() {
            pairs.push(("no_proxy".into(), self.no_proxy.clone()));
            pairs.push(("NO_PROXY".into(), self.no_proxy.clone()));
        }
        pairs
    }

    /// Pairs for a cleaned job environment. An unsaved install copies the
    /// process proxy through. A saved empty proxy adds nothing.
    pub fn job_pairs(&self) -> Vec<(String, String)> {
        if self.explicit {
            self.env_pairs()
        } else {
            inherited_proxy_env()
        }
    }

    pub fn apply_command(&self, cmd: &mut tokio::process::Command) {
        if !self.explicit {
            return;
        }
        for key in PROXY_ENV {
            cmd.env_remove(*key);
        }
        for (key, value) in self.env_pairs() {
            cmd.env(key, value);
        }
    }

    pub fn apply_reqwest(
        &self,
        builder: reqwest::ClientBuilder,
    ) -> Result<reqwest::ClientBuilder, String> {
        if !self.explicit {
            return Ok(builder);
        }
        if self.url.is_empty() {
            return Ok(builder.no_proxy());
        }
        let mut proxy = reqwest::Proxy::all(&self.url).map_err(|err| format!("Proxy: {err}"))?;
        if !self.username.is_empty() {
            proxy = proxy.basic_auth(&self.username, &self.password);
        }
        if !self.no_proxy.is_empty() {
            proxy = proxy.no_proxy(reqwest::NoProxy::from_string(&self.no_proxy));
        }
        Ok(builder.proxy(proxy))
    }

    fn url_with_auth(&self) -> String {
        let (scheme, rest) = if let Some(rest) = self.url.strip_prefix("https://") {
            ("https://", rest)
        } else if let Some(rest) = self.url.strip_prefix("http://") {
            ("http://", rest)
        } else {
            return self.url.clone();
        };
        if self.username.is_empty() && self.password.is_empty() {
            return self.url.clone();
        }
        format!(
            "{scheme}{}:{}@{rest}",
            encode_userinfo(&self.username),
            encode_userinfo(&self.password)
        )
    }
}

impl Effective {
    /// Active directory, or nothing when the address is empty.
    pub fn directory(&self) -> Option<&LdapConfig> {
        if self.ldap.url.is_empty() {
            None
        } else {
            Some(&self.ldap)
        }
    }

    pub fn default_environment(&self, tool: &str) -> &str {
        match tool {
            "ansible" => &self.default_environment_ansible,
            "opentofu" => &self.default_environment_opentofu,
            "shell" => &self.default_environment_shell,
            _ => "",
        }
    }
}

struct Draft {
    ldap_url: String,
    ldap_mode: String,
    ldap_bind_dn: String,
    ldap_bind_password: String,
    ldap_user_base: String,
    ldap_user_filter: String,
    ldap_username_attr: String,
    ldap_group_attr: String,
    ldap_group_base: String,
    ldap_starttls: bool,
    ldap_tls_insecure: bool,
    proxy_url: String,
    proxy_username: String,
    proxy_password: String,
    proxy_no_proxy: String,
    runner: RunnerMode,
    job_timeout_secs: u64,
    job_output_lines: u64,
    default_environment_ansible: String,
    default_environment_opentofu: String,
    default_environment_shell: String,
    llm_enabled: bool,
    llm_provider: LlmProvider,
    llm_url: String,
    llm_model: String,
    llm_api_key: String,
}

/// Runner mode for `koreograf doctor`. Reads the saved value when the
/// database exists and falls back to the environment configuration.
pub fn configured_mode(cfg: &crate::Config) -> RunnerMode {
    let path = cfg.data_dir.join("koreograf.db");
    if !path.is_file() {
        return cfg.runner.in_container(cfg.container_image);
    }
    let Ok(conn) = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return cfg.runner.in_container(cfg.container_image);
    };
    conn.query_row(super::store::SETTING, ["runner"], |row| {
        row.get::<_, String>(0)
    })
    .ok()
    .map(|value| parse_runner_stored(&value, cfg.runner).in_container(cfg.container_image))
    .unwrap_or(cfg.runner.in_container(cfg.container_image))
}

pub fn load(state: &Shared) -> AppResult<Effective> {
    let rows = state.store().settings_rows()?;
    if rows.get(SAVED).map(String::as_str) == Some("1") {
        let mut effective = from_saved(state, &rows)?;
        effective.runner = effective.runner.in_container(state.cfg.container_image);
        return Ok(effective);
    }
    let mut effective = from_env(state);
    effective.default_environment_ansible = rows
        .get(DEFAULT_ENVIRONMENT_KEYS[0])
        .cloned()
        .unwrap_or_default();
    effective.default_environment_opentofu = rows
        .get(DEFAULT_ENVIRONMENT_KEYS[1])
        .cloned()
        .unwrap_or_default();
    effective.default_environment_shell = rows
        .get(DEFAULT_ENVIRONMENT_KEYS[2])
        .cloned()
        .unwrap_or_default();
    Ok(effective)
}

pub fn view(state: &Shared) -> AppResult<Effective> {
    load(state)
}

pub fn save(state: &Shared, input: &SettingsWrite) -> AppResult<Effective> {
    if input.reset {
        state.store().replace_settings(&[])?;
        let effective = from_env(state);
        crate::tls::warn_directory(&effective.ldap);
        return Ok(effective);
    }
    let current = load(state)?;
    let draft = validate(input, &current, state.store())?;
    let mut draft = draft;
    if state.cfg.container_image && draft.runner == RunnerMode::Local {
        tracing::warn!("The container image has no local runner; using docker.");
        draft.runner = RunnerMode::Docker;
    }
    let ldap_secret = encrypt_secret(state, &draft.ldap_bind_password)?;
    let proxy_secret = encrypt_secret(state, &draft.proxy_password)?;
    let llm_secret = encrypt_secret(state, &draft.llm_api_key)?;
    let mut pairs = vec![
        (SAVED, "1".to_string()),
        ("ldap_url", draft.ldap_url.clone()),
        ("ldap_mode", draft.ldap_mode.clone()),
        ("ldap_bind_dn", draft.ldap_bind_dn.clone()),
        ("ldap_user_base", draft.ldap_user_base.clone()),
        ("ldap_user_filter", draft.ldap_user_filter.clone()),
        ("ldap_username_attr", draft.ldap_username_attr.clone()),
        ("ldap_group_attr", draft.ldap_group_attr.clone()),
        ("ldap_group_base", draft.ldap_group_base.clone()),
        ("ldap_starttls", flag(draft.ldap_starttls)),
        ("ldap_tls_insecure", flag(draft.ldap_tls_insecure)),
        ("proxy_url", draft.proxy_url.clone()),
        ("proxy_username", draft.proxy_username.clone()),
        ("proxy_no_proxy", draft.proxy_no_proxy.clone()),
        ("runner", draft.runner.as_str().to_string()),
        ("job_timeout_secs", draft.job_timeout_secs.to_string()),
        ("job_output_lines", draft.job_output_lines.to_string()),
        (
            DEFAULT_ENVIRONMENT_KEYS[0],
            draft.default_environment_ansible.clone(),
        ),
        (
            DEFAULT_ENVIRONMENT_KEYS[1],
            draft.default_environment_opentofu.clone(),
        ),
        (
            DEFAULT_ENVIRONMENT_KEYS[2],
            draft.default_environment_shell.clone(),
        ),
        ("llm_enabled", flag(draft.llm_enabled)),
        ("llm_provider", draft.llm_provider.as_str().to_string()),
        ("llm_url", draft.llm_url.clone()),
        ("llm_model", draft.llm_model.clone()),
    ];
    if let Some(secret) = ldap_secret {
        pairs.push((LDAP_PASSWORD, secret));
    }
    if let Some(secret) = proxy_secret {
        pairs.push((PROXY_PASSWORD, secret));
    }
    if let Some(secret) = llm_secret {
        pairs.push((LLM_API_KEY, secret));
    }
    let pairs: Vec<(String, String)> = pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    state.store().replace_settings(&pairs)?;
    let mut effective = from_env(state);
    effective.saved = true;
    effective.unsupported_proxy = false;
    effective.ldap = draft_ldap(&draft);
    effective.proxy = ProxySettings {
        url: draft.proxy_url,
        username: draft.proxy_username,
        password: draft.proxy_password,
        no_proxy: draft.proxy_no_proxy,
        explicit: true,
    };
    effective.runner = draft.runner;
    effective.job_timeout_secs = draft.job_timeout_secs;
    effective.job_output_lines = draft.job_output_lines;
    effective.default_environment_ansible = draft.default_environment_ansible;
    effective.default_environment_opentofu = draft.default_environment_opentofu;
    effective.default_environment_shell = draft.default_environment_shell;
    effective.llm_enabled = draft.llm_enabled;
    effective.llm_provider = draft.llm_provider;
    effective.llm_url = draft.llm_url;
    effective.llm_model = draft.llm_model;
    effective.llm_api_key = draft.llm_api_key;
    crate::tls::warn_directory(&effective.ldap);
    Ok(effective)
}

fn from_env(state: &Shared) -> Effective {
    let (proxy, unsupported_proxy) = proxy_from_env();
    let ldap = state.cfg.ldap.clone().unwrap_or_else(empty_ldap);
    let llm = crate::config::llm_environment();
    let llm_api_key = match &llm.api_key_file {
        Some(path) => std::fs::read_to_string(path)
            .map(|value| value.trim().to_string())
            .unwrap_or_default(),
        None => llm.api_key,
    };
    Effective {
        ldap,
        proxy,
        runner: state.cfg.runner.in_container(state.cfg.container_image),
        job_timeout_secs: state.cfg.job_timeout_secs,
        job_output_lines: state.cfg.job_output_lines,
        default_environment_ansible: String::new(),
        default_environment_opentofu: String::new(),
        default_environment_shell: String::new(),
        llm_enabled: llm.enabled,
        llm_provider: llm.provider,
        llm_url: llm.base_url,
        llm_model: llm.model,
        llm_api_key,
        saved: false,
        unsupported_proxy,
    }
}

fn from_saved(state: &Shared, rows: &HashMap<String, String>) -> AppResult<Effective> {
    let ldap_password = decrypt_secret(state, rows.get(LDAP_PASSWORD))?;
    let proxy_password = decrypt_secret(state, rows.get(PROXY_PASSWORD))?;
    let llm_api_key = decrypt_secret(state, rows.get(LLM_API_KEY))?;
    let mode = rows.get("ldap_mode").map(String::as_str).unwrap_or("ldap");
    let ad = mode.eq_ignore_ascii_case("ad");
    let ldap_mode = if ad { "ad" } else { "ldap" };
    let url = rows.get("ldap_url").cloned().unwrap_or_default();
    let ldap = LdapConfig {
        url,
        mode: ldap_mode.into(),
        bind_dn: rows.get("ldap_bind_dn").cloned().unwrap_or_default(),
        bind_password: ldap_password,
        user_base: rows.get("ldap_user_base").cloned().unwrap_or_default(),
        user_filter: filled(
            rows.get("ldap_user_filter")
                .map(String::as_str)
                .unwrap_or(""),
            filter_default(ad),
        ),
        username_attr: filled(
            rows.get("ldap_username_attr")
                .map(String::as_str)
                .unwrap_or(""),
            attr_default(ad),
        ),
        group_attr: filled(
            rows.get("ldap_group_attr")
                .map(String::as_str)
                .unwrap_or(""),
            "memberOf",
        ),
        group_base: rows.get("ldap_group_base").cloned().unwrap_or_default(),
        starttls: flag_on(rows.get("ldap_starttls").map(String::as_str).unwrap_or("")),
        tls_insecure: flag_on(
            rows.get("ldap_tls_insecure")
                .map(String::as_str)
                .unwrap_or(""),
        ),
    };
    let runner = rows
        .get("runner")
        .map(|value| parse_runner_stored(value, state.cfg.runner))
        .unwrap_or(state.cfg.runner);
    let job_timeout_secs = rows
        .get("job_timeout_secs")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (30..=86_400).contains(value))
        .unwrap_or(state.cfg.job_timeout_secs);
    let job_output_lines = rows
        .get("job_output_lines")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (OUTPUT_LINES_MIN..=OUTPUT_LINES_MAX).contains(value))
        .unwrap_or(state.cfg.job_output_lines);
    Ok(Effective {
        ldap,
        proxy: ProxySettings {
            url: rows.get("proxy_url").cloned().unwrap_or_default(),
            username: rows.get("proxy_username").cloned().unwrap_or_default(),
            password: proxy_password,
            no_proxy: rows.get("proxy_no_proxy").cloned().unwrap_or_default(),
            explicit: true,
        },
        runner,
        job_timeout_secs,
        job_output_lines,
        default_environment_ansible: rows
            .get(DEFAULT_ENVIRONMENT_KEYS[0])
            .cloned()
            .unwrap_or_default(),
        default_environment_opentofu: rows
            .get(DEFAULT_ENVIRONMENT_KEYS[1])
            .cloned()
            .unwrap_or_default(),
        default_environment_shell: rows
            .get(DEFAULT_ENVIRONMENT_KEYS[2])
            .cloned()
            .unwrap_or_default(),
        llm_enabled: flag_on(rows.get("llm_enabled").map(String::as_str).unwrap_or("")),
        llm_provider: LlmProvider::parse(
            rows.get("llm_provider")
                .map(String::as_str)
                .unwrap_or("openai"),
        ),
        llm_url: rows.get("llm_url").cloned().unwrap_or_default(),
        llm_model: rows.get("llm_model").cloned().unwrap_or_default(),
        llm_api_key,
        saved: true,
        unsupported_proxy: false,
    })
}

fn draft_ldap(draft: &Draft) -> LdapConfig {
    LdapConfig {
        url: draft.ldap_url.clone(),
        mode: draft.ldap_mode.clone(),
        bind_dn: draft.ldap_bind_dn.clone(),
        bind_password: draft.ldap_bind_password.clone(),
        user_base: draft.ldap_user_base.clone(),
        user_filter: draft.ldap_user_filter.clone(),
        username_attr: draft.ldap_username_attr.clone(),
        group_attr: draft.ldap_group_attr.clone(),
        group_base: draft.ldap_group_base.clone(),
        starttls: draft.ldap_starttls,
        tls_insecure: draft.ldap_tls_insecure,
    }
}

fn empty_ldap() -> LdapConfig {
    LdapConfig {
        url: String::new(),
        mode: "ldap".into(),
        bind_dn: String::new(),
        bind_password: String::new(),
        user_base: String::new(),
        user_filter: filter_default(false).into(),
        username_attr: attr_default(false).into(),
        group_attr: "memberOf".into(),
        group_base: String::new(),
        starttls: false,
        tls_insecure: false,
    }
}

fn validate(input: &SettingsWrite, current: &Effective, store: &dyn Backend) -> AppResult<Draft> {
    let ldap_mode = match input.ldap_mode.trim().to_ascii_lowercase().as_str() {
        "ldap" => "ldap",
        "ad" => "ad",
        "" => "ldap",
        _ => return Err(ApiError::bad("the_ldap_mode_is_invalid")),
    };
    let ad = ldap_mode == "ad";
    let ldap_url = plain(&input.ldap_url, 400)?;
    if !valid_ldap_url(&ldap_url) {
        return Err(ApiError::bad("the_ldap_address_must_be_ldap_or_ldaps"));
    }
    let ldap_bind_dn = plain(&input.ldap_bind_dn, 400)?;
    let ldap_user_base = plain(&input.ldap_user_base, 400)?;
    let ldap_group_base = plain(&input.ldap_group_base, 400)?;
    let mut ldap_user_filter = plain(&input.ldap_user_filter, 400)?;
    if ldap_user_filter.is_empty() {
        ldap_user_filter = filter_default(ad).to_string();
    } else if !ldap_user_filter.contains("{username}") {
        return Err(ApiError::bad("the_ldap_filter_needs_username"));
    }
    let mut ldap_username_attr = plain(&input.ldap_username_attr, 80)?;
    if ldap_username_attr.is_empty() {
        ldap_username_attr = attr_default(ad).to_string();
    } else if !valid_attr(&ldap_username_attr) {
        return Err(ApiError::bad("the_value_contains_invalid_characters"));
    }
    let mut ldap_group_attr = plain(&input.ldap_group_attr, 80)?;
    if ldap_group_attr.is_empty() {
        ldap_group_attr = "memberOf".to_string();
    } else if !valid_attr(&ldap_group_attr) {
        return Err(ApiError::bad("the_value_contains_invalid_characters"));
    }
    if !ldap_url.is_empty() && (ldap_bind_dn.is_empty() || ldap_user_base.is_empty()) {
        return Err(ApiError::bad("ldap_is_not_fully_configured"));
    }
    let current_ldap_password = current.ldap.bind_password.as_str();
    let ldap_bind_password = resolve_secret(
        &input.ldap_bind_password,
        input.clear_ldap_password,
        current_ldap_password,
    )?;
    if !ldap_url.is_empty() && !ldap_bind_dn.is_empty() && ldap_bind_password.is_empty() {
        return Err(ApiError::bad("ldap_is_not_fully_configured"));
    }
    let proxy_url = plain(&input.proxy_url, 300)?;
    if proxy_url.contains('@') {
        return Err(ApiError::bad("put_the_credentials_in_their_own_fields_not"));
    }
    if !valid_proxy_url(&proxy_url) {
        return Err(ApiError::bad("the_proxy_address_must_be_http_or_https"));
    }
    let proxy_username = plain(&input.proxy_username, 200)?;
    let proxy_no_proxy = plain(&input.proxy_no_proxy, 500)?;
    if !valid_no_proxy(&proxy_no_proxy) {
        return Err(ApiError::bad("the_proxy_exceptions_are_invalid"));
    }
    let proxy_password = resolve_secret(
        &input.proxy_password,
        input.clear_proxy_password,
        &current.proxy.password,
    )?;
    if !proxy_url.is_empty() && proxy_username.is_empty() && !proxy_password.is_empty() {
        return Err(ApiError::bad("the_proxy_user_is_missing"));
    }
    let runner = match input.runner.trim() {
        "auto" => RunnerMode::Auto,
        "docker" => RunnerMode::Docker,
        "local" => RunnerMode::Local,
        _ => return Err(ApiError::bad("the_runner_mode_is_invalid")),
    };
    if !(30..=86_400).contains(&input.job_timeout_secs) {
        return Err(ApiError::bad("the_time_limit_must_be_between_30_and"));
    }
    let job_output_lines = if input.job_output_lines == 0 {
        current.job_output_lines
    } else if !(OUTPUT_LINES_MIN..=OUTPUT_LINES_MAX).contains(&input.job_output_lines) {
        return Err(ApiError::bad(
            "the_output_line_limit_must_be_between_100_and",
        ));
    } else {
        input.job_output_lines
    };
    let default_environment_ansible =
        validate_default_environment(store, "ansible", &input.default_environment_ansible)?;
    let default_environment_opentofu =
        validate_default_environment(store, "opentofu", &input.default_environment_opentofu)?;
    let default_environment_shell =
        validate_default_environment(store, "shell", &input.default_environment_shell)?;
    let (llm_enabled, llm_provider, llm_url, llm_model, llm_api_key) =
        validate_llm(input.llm.as_ref(), current)?;
    Ok(Draft {
        ldap_url,
        ldap_mode: ldap_mode.to_string(),
        ldap_bind_dn,
        ldap_bind_password,
        ldap_user_base,
        ldap_user_filter,
        ldap_username_attr,
        ldap_group_attr,
        ldap_group_base,
        ldap_starttls: input.ldap_starttls,
        ldap_tls_insecure: input.ldap_tls_insecure,
        proxy_url,
        proxy_username,
        proxy_password,
        proxy_no_proxy,
        runner,
        job_timeout_secs: input.job_timeout_secs,
        job_output_lines,
        default_environment_ansible,
        default_environment_opentofu,
        default_environment_shell,
        llm_enabled,
        llm_provider,
        llm_url,
        llm_model,
        llm_api_key,
    })
}

fn validate_llm(
    input: Option<&LlmWrite>,
    current: &Effective,
) -> AppResult<(bool, LlmProvider, String, String, String)> {
    let Some(input) = input else {
        return Ok((
            current.llm_enabled,
            current.llm_provider,
            current.llm_url.clone(),
            current.llm_model.clone(),
            current.llm_api_key.clone(),
        ));
    };
    let provider = LlmProvider::parse(&input.provider);
    let base_url = plain(&input.base_url, 400)?;
    if !valid_llm_url(&base_url) {
        return Err(ApiError::bad("the_assistant_address_must_be_http_or_https"));
    }
    let model = plain(&input.model, 200)?;
    if input.enabled && base_url.is_empty() {
        return Err(ApiError::bad("the_assistant_address_is_missing"));
    }
    if input.enabled && model.is_empty() {
        return Err(ApiError::bad("the_assistant_model_is_missing"));
    }
    let api_key = resolve_secret(&input.api_key, input.clear_api_key, &current.llm_api_key)?;
    Ok((input.enabled, provider, base_url, model, api_key))
}

fn valid_llm_url(url: &str) -> bool {
    if url.is_empty() {
        return true;
    }
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && !url.chars().any(char::is_control)
}

fn validate_default_environment(store: &dyn Backend, tool: &str, raw: &str) -> AppResult<String> {
    let id = raw.trim();
    if id.is_empty() {
        return Ok(String::new());
    }
    let environment = store
        .environment(id)?
        .ok_or_else(|| ApiError::bad("the_default_environment_is_missing"))?;
    if environment.scope != "global" {
        return Err(ApiError::bad("the_default_environment_must_be_global"));
    }
    if !crate::runner::environment_fits_tool(tool, &environment.tool) {
        return Err(ApiError::bad("the_default_environment_is_for_another_tool"));
    }
    Ok(id.to_string())
}

fn resolve_secret(incoming: &str, clear: bool, current: &str) -> AppResult<String> {
    let incoming = incoming.trim();
    if !incoming.is_empty() && incoming != SECRET_MARK {
        if incoming.chars().count() > 1024 {
            return Err(ApiError::bad("the_value_is_too_long"));
        }
        if incoming
            .chars()
            .any(|ch| ch == '\0' || ch == '\n' || ch == '\r')
        {
            return Err(ApiError::bad("the_value_contains_invalid_characters"));
        }
        return Ok(incoming.to_string());
    }
    if incoming == SECRET_MARK && current.is_empty() {
        return Err(ApiError::bad("the_placeholder_is_not_a_password"));
    }
    if clear {
        return Ok(String::new());
    }
    Ok(current.to_string())
}

fn encrypt_secret(state: &Shared, plain: &str) -> AppResult<Option<String>> {
    if plain.is_empty() {
        Ok(None)
    } else {
        Ok(Some(state.encrypt(plain)?))
    }
}

fn decrypt_secret(state: &Shared, raw: Option<&String>) -> AppResult<String> {
    let Some(raw) = raw else {
        return Ok(String::new());
    };
    if raw.is_empty() {
        return Ok(String::new());
    }
    state
        .decrypt(raw)
        .map_err(|_| ApiError::bad("the_saved_setting_cannot_be_read"))
}

fn proxy_from_env() -> (ProxySettings, bool) {
    let raw = first_env(&["https_proxy", "HTTPS_PROXY", "http_proxy", "HTTP_PROXY"]);
    let no_proxy = first_env(&["no_proxy", "NO_PROXY"]).unwrap_or_default();
    let all_proxy = first_env(&["all_proxy", "ALL_PROXY"]);
    match raw.as_deref().and_then(parse_http_proxy) {
        Some((url, username, password)) => (
            ProxySettings {
                url,
                username,
                password,
                no_proxy,
                explicit: false,
            },
            false,
        ),
        None => (
            ProxySettings {
                url: String::new(),
                username: String::new(),
                password: String::new(),
                no_proxy,
                explicit: false,
            },
            raw.is_some() || all_proxy.is_some(),
        ),
    }
}

fn inherited_proxy_env() -> Vec<(String, String)> {
    PROXY_ENV
        .iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.is_empty())
                .map(|value| ((*key).to_string(), value))
        })
        .collect()
}

fn first_env(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn parse_http_proxy(raw: &str) -> Option<(String, String, String)> {
    let (scheme, rest) = if let Some(rest) = raw.strip_prefix("https://") {
        ("https://", rest)
    } else {
        ("http://", raw.strip_prefix("http://")?)
    };
    if rest.is_empty() || rest.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
        return None;
    }
    if let Some((userinfo, host)) = rest.rsplit_once('@') {
        if host.is_empty() {
            return None;
        }
        let (user, pass) = userinfo.split_once(':').unwrap_or((userinfo, ""));
        Some((
            format!("{scheme}{host}"),
            percent_decode(user),
            percent_decode(pass),
        ))
    } else {
        Some((raw.to_string(), String::new(), String::new()))
    }
}

fn valid_ldap_url(url: &str) -> bool {
    if url.is_empty() {
        return true;
    }
    let rest = url
        .strip_prefix("ldap://")
        .or_else(|| url.strip_prefix("ldaps://"));
    let Some(rest) = rest else {
        return false;
    };
    !rest.is_empty()
        && !rest.contains('@')
        && !url.chars().any(|ch| ch.is_whitespace() || ch.is_control())
}

fn valid_proxy_url(url: &str) -> bool {
    if url.is_empty() {
        return true;
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    let Some(rest) = rest else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !rest.is_empty()
        && !host.is_empty()
        && !host.starts_with(':')
        && !url.chars().any(|ch| ch.is_whitespace() || ch.is_control())
}

fn valid_no_proxy(value: &str) -> bool {
    value.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '.' | ':' | '-' | '_' | '*' | ',' | ' ' | '/')
    })
}

fn valid_attr(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn plain(value: &str, max: usize) -> AppResult<String> {
    let value = value.trim();
    if value.chars().count() > max {
        return Err(ApiError::bad("the_value_is_too_long"));
    }
    if value.chars().any(|ch| ch.is_control()) {
        return Err(ApiError::bad("the_value_contains_invalid_characters"));
    }
    Ok(value.to_string())
}

fn filter_default(ad: bool) -> &'static str {
    if ad {
        "(sAMAccountName={username})"
    } else {
        "(uid={username})"
    }
}

fn attr_default(ad: bool) -> &'static str {
    if ad {
        "sAMAccountName"
    } else {
        "uid"
    }
}

fn filled(value: &str, default: &str) -> String {
    if value.trim().is_empty() {
        default.to_string()
    } else {
        value.to_string()
    }
}

fn flag(value: bool) -> String {
    if value {
        "1".into()
    } else {
        "0".into()
    }
}

fn flag_on(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "ja"
    )
}

fn parse_runner_stored(value: &str, fallback: RunnerMode) -> RunnerMode {
    match value.trim() {
        "auto" => RunnerMode::Auto,
        "docker" => RunnerMode::Docker,
        "local" => RunnerMode::Local,
        _ => fallback,
    }
}

fn encode_userinfo(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Some(decoded) = hex_byte(bytes[index + 1], bytes[index + 2]) {
                out.push(decoded);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_byte(hi: u8, lo: u8) -> Option<u8> {
    Some((hex_val(hi)? << 4) | hex_val(lo)?)
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

const FORMAT: u32 = 1;
const ENGINE: &str = "sqlite";
pub(super) const MAX_BYTES: u64 = 256 * 1024 * 1024;
pub(super) const MANIFEST: &str = "manifest.json";
pub(super) const DATABASE: &str = "koreograf.db";
pub(super) const SECRET: &str = "secret.key";

const REQUIRED_TABLES: &[&str] = &[
    "users",
    "organizations",
    "teams",
    "credentials",
    "projects",
    "templates",
    "workflows",
    "hooks",
];

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Manifest {
    pub(super) format: u32,
    pub(super) engine: String,
    pub(super) created_at: String,
    pub(super) koreograf: String,
    pub(super) includes: Vec<String>,
    pub(super) omits: Vec<String>,
}

struct TempPath(PathBuf);

fn sidecars(path: &Path) -> Vec<PathBuf> {
    let mut paths = vec![path.to_path_buf()];
    for suffix in ["-wal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        paths.push(PathBuf::from(name));
    }
    paths
}

impl Drop for TempPath {
    fn drop(&mut self) {
        for path in sidecars(&self.0) {
            let _ = fs::remove_file(path);
        }
    }
}

pub(crate) fn export(state: &Shared, include_tofu_state: bool) -> AppResult<Vec<u8>> {
    let _maintenance = lock_maintenance(state)?;
    let key = read_key(&state.cfg.data_dir)?;
    let snapshot = state
        .cfg
        .data_dir
        .join(format!("backup-{}.db", Uuid::new_v4()));
    let _cleanup = TempPath(snapshot.clone());
    state.store().export_sqlite(&snapshot)?;
    snapshot_zip_from_file(&snapshot, &key, &state.cfg.data_dir, include_tofu_state)
}

pub(crate) fn restore(state: &Shared, bytes: &[u8]) -> AppResult<()> {
    let _maintenance = lock_maintenance(state)?;
    let incoming = prepare_incoming(&state.cfg.data_dir, bytes)?;
    let current_key = read_key(&state.cfg.data_dir)?;
    if state.store().busy()? {
        return Err(ApiError::bad("backup.still_running"));
    }
    let snapshot = state
        .cfg
        .data_dir
        .join(format!("backup-{}.db", Uuid::new_v4()));
    let _cleanup = TempPath(snapshot.clone());
    state.store().export_sqlite(&snapshot)?;
    let previous = snapshot_zip_from_file(&snapshot, &current_key, &state.cfg.data_dir, false)?;
    write_private(&state.cfg.data_dir.join("restore-previous.zip"), &previous)?;
    let engine = state.engine();
    if let Err(err) = state.store().restore_sqlite(&incoming.path) {
        if engine == "sqlite" {
            if let Err(rollback) = state.store().restore_sqlite(&snapshot) {
                tracing::error!(
                    error = %rollback,
                    "backup rollback failed"
                );
            }
        }
        return Err(err);
    }
    let cipher = crypto::install_key(&state.cfg.data_dir, &incoming.key)?;
    state.replace_cipher(cipher)?;
    if incoming
        .includes
        .iter()
        .any(|item| item == "opentofu_state")
    {
        clear_state_dirs(&state.cfg.data_dir)?;
        write_extra_files(&state.cfg.data_dir, &incoming.files)?;
    }
    tracing::info!("backup restored");
    Ok(())
}

pub(crate) fn restore_startup(
    data_dir: &Path,
    bytes: &[u8],
    container_image: bool,
) -> AppResult<RestoreSummary> {
    let target = data_dir.join(DATABASE);
    if target.exists() {
        return Err(ApiError::bad("the_data_directory_is_not_empty"));
    }
    fs::create_dir_all(data_dir).map_err(|err| ApiError::Internal(err.to_string()))?;
    let staging = new_staging(data_dir, ".restore");
    fs::create_dir_all(&staging).map_err(|err| ApiError::Internal(err.to_string()))?;
    let result = restore_startup_into(&staging, bytes).and_then(|summary| {
        promote(&staging, data_dir)?;
        Ok(summary)
    });
    match result {
        Ok(summary) => {
            let _ = fs::remove_dir_all(&staging);
            own_imported(data_dir, container_image);
            Ok(summary)
        }
        Err(err) => {
            let _ = fs::remove_dir_all(&staging);
            Err(err)
        }
    }
}

fn restore_startup_into(staging: &Path, bytes: &[u8]) -> AppResult<RestoreSummary> {
    let incoming = prepare_incoming(staging, bytes)?;
    let target = staging.join(DATABASE);
    if target.exists() {
        return Err(ApiError::bad("the_data_directory_is_not_empty"));
    }
    fs::rename(&incoming.path, &target).map_err(|err| ApiError::Internal(err.to_string()))?;
    crypto::install_key(staging, &incoming.key)?;
    write_extra_files(staging, &incoming.files)?;
    Ok(RestoreSummary {
        koreograf: incoming.koreograf.clone(),
        includes: incoming.includes.clone(),
    })
}

pub(crate) fn new_staging(data_dir: &Path, prefix: &str) -> PathBuf {
    data_dir.join(format!("{prefix}-{}", Uuid::new_v4()))
}

pub(crate) fn remove_stale_staging(data_dir: &Path) {
    let Ok(entries) = fs::read_dir(data_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(".import-") && !name.starts_with(".restore-") {
            continue;
        }
        let path = entry.path();
        if fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            let _ = fs::remove_dir_all(path);
        }
    }
}

pub(crate) fn promote(staging: &Path, data_dir: &Path) -> AppResult<()> {
    let mut moved = Vec::new();
    let entries = fs::read_dir(staging).map_err(|err| ApiError::Internal(err.to_string()))?;
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| {
        let name = entry.file_name();
        (name.to_str() == Some(DATABASE), name)
    });
    for entry in entries {
        let target = data_dir.join(entry.file_name());
        if fs::symlink_metadata(&target).is_ok() {
            remove_moved(&moved);
            return Err(ApiError::Internal(format!(
                "{} already exists",
                target.display()
            )));
        }
        if let Err(err) = fs::rename(entry.path(), &target) {
            remove_moved(&moved);
            return Err(ApiError::Internal(err.to_string()));
        }
        moved.push(target);
    }
    Ok(())
}

fn remove_moved(paths: &[PathBuf]) {
    for path in paths.iter().rev() {
        if fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir()) {
            let _ = fs::remove_dir_all(path);
        } else {
            let _ = fs::remove_file(path);
        }
    }
}

fn lock_maintenance(state: &Shared) -> AppResult<std::sync::MutexGuard<'_, ()>> {
    state
        .maintenance
        .lock()
        .map_err(|_| ApiError::Internal("backup lock is damaged".into()))
}

fn read_key(data_dir: &Path) -> AppResult<Vec<u8>> {
    let key = fs::read(data_dir.join(SECRET))
        .map_err(|_| ApiError::Internal("secret.key is missing".into()))?;
    crypto::parse_key(&key)?;
    Ok(key)
}

fn prepare_incoming(data_dir: &Path, bytes: &[u8]) -> AppResult<Incoming> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
        return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
    }
    let entries = unpack(bytes)?;
    let manifest = entry(&entries, MANIFEST, "the_backup_cannot_be_read")?;
    let manifest: Manifest = serde_json::from_slice(manifest)
        .map_err(|_| ApiError::bad("the_backup_has_an_unknown_format"))?;
    if manifest.format != FORMAT || manifest.engine != ENGINE {
        return Err(ApiError::bad("the_backup_has_an_unknown_format"));
    }
    let database = entry(
        &entries,
        DATABASE,
        "the_backup_does_not_contain_a_koreograf_database",
    )?
    .to_vec();
    let key = entry(&entries, SECRET, "the_backup_does_not_contain_a_key")?.to_vec();
    crypto::parse_key(&key)?;
    let mut files = Vec::new();
    for (name, bytes) in &entries {
        if name == MANIFEST || name == DATABASE || name == SECRET {
            continue;
        }
        if state_file_parts(name).is_none() {
            return Err(ApiError::bad("the_backup_cannot_be_read"));
        }
        files.push((name.clone(), bytes.clone()));
    }
    let path = data_dir.join(format!("restore-{}.db", Uuid::new_v4()));
    fs::write(&path, database).map_err(|err| ApiError::Internal(err.to_string()))?;
    let incoming = Incoming {
        path,
        key,
        files,
        koreograf: manifest.koreograf,
        includes: manifest.includes,
    };
    let conn = Connection::open(&incoming.path)?;
    checked_database(&conn)?;
    let version = db::schema_version(&conn)?;
    if version > db::SCHEMA_VERSION {
        return Err(ApiError::bad("the_backup_is_from_a_newer_koreograf"));
    }
    drop(conn);
    let conn = db::open(&incoming.path)?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(incoming)
}

struct Incoming {
    path: PathBuf,
    key: Vec<u8>,
    files: Vec<(String, Vec<u8>)>,
    koreograf: String,
    includes: Vec<String>,
}

impl Drop for Incoming {
    fn drop(&mut self) {
        for path in sidecars(&self.path) {
            let _ = fs::remove_file(path);
        }
    }
}

fn entry<'a>(entries: &'a [(String, Vec<u8>)], name: &str, missing: &str) -> AppResult<&'a [u8]> {
    entries
        .iter()
        .find(|(item, _)| item == name)
        .map(|(_, bytes)| bytes.as_slice())
        .ok_or_else(|| ApiError::bad(missing))
}

fn snapshot_zip_from_file(
    path: &Path,
    key: &[u8],
    data_dir: &Path,
    include_tofu_state: bool,
) -> AppResult<Vec<u8>> {
    let database = fs::read(path).map_err(|err| ApiError::Internal(err.to_string()))?;
    if database.is_empty() || database.len() as u64 > MAX_BYTES {
        return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
    }
    let mut includes = vec!["database".into(), "secret_key".into()];
    let mut omits = vec!["checkouts".into(), "job_workdirs".into()];
    let extras = if include_tofu_state {
        includes.push("opentofu_state".into());
        collect_tofu_state(data_dir)?
    } else {
        omits.push("cache".into());
        Vec::new()
    };
    let manifest = Manifest {
        format: FORMAT,
        engine: ENGINE.into(),
        created_at: db::now(),
        koreograf: env!("CARGO_PKG_VERSION").into(),
        includes,
        omits,
    };
    let manifest =
        serde_json::to_vec_pretty(&manifest).map_err(|err| ApiError::Internal(err.to_string()))?;
    let mut files: Vec<(&str, &[u8])> = vec![
        (MANIFEST, manifest.as_slice()),
        (DATABASE, database.as_slice()),
        (SECRET, key),
    ];
    for (name, bytes) in &extras {
        files.push((name.as_str(), bytes.as_slice()));
    }
    let total: u64 = files.iter().map(|(_, bytes)| bytes.len() as u64).sum();
    if total > MAX_BYTES {
        return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
    }
    let packed = pack(&files)?;
    if packed.len() as u64 > MAX_BYTES {
        return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
    }
    Ok(packed)
}

fn collect_tofu_state(data_dir: &Path) -> AppResult<Vec<(String, Vec<u8>)>> {
    let cache = data_dir.join("cache");
    let mut out = Vec::new();
    let mut total = 0u64;
    let Ok(teams) = fs::read_dir(&cache) else {
        return Ok(out);
    };
    let mut teams: Vec<_> = teams.filter_map(Result::ok).collect();
    teams.sort_by_key(|entry| entry.file_name());
    for team in teams {
        let name = team.file_name().to_string_lossy().to_string();
        if !safe_component(&name) {
            continue;
        }
        let state = team.path().join("tofu-state");
        if !fs::symlink_metadata(&state).is_ok_and(|meta| meta.is_dir()) {
            continue;
        }
        collect_files(
            &state,
            &format!("cache/{name}/tofu-state"),
            &mut out,
            &mut total,
        )?;
    }
    Ok(out)
}

fn collect_files(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<(String, Vec<u8>)>,
    total: &mut u64,
) -> AppResult<()> {
    let entries = fs::read_dir(dir).map_err(|err| ApiError::Internal(err.to_string()))?;
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if !safe_component(&name) {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|err| ApiError::Internal(err.to_string()))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let child = entry.path();
        if metadata.is_dir() {
            collect_files(&child, &format!("{prefix}/{name}"), out, total)?;
        } else if metadata.is_file() {
            if metadata.len() > MAX_BYTES {
                return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
            }
            let bytes = fs::read(&child).map_err(|err| ApiError::Internal(err.to_string()))?;
            *total = total.saturating_add(bytes.len() as u64);
            if *total > MAX_BYTES {
                return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
            }
            out.push((format!("{prefix}/{name}"), bytes));
        }
    }
    Ok(())
}

fn safe_component(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

fn safe_relative(name: &str) -> bool {
    let path = Path::new(name);
    !name.contains(['\\', '\0'])
        && !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

pub(super) fn write_extra_files(data_dir: &Path, files: &[(String, Vec<u8>)]) -> AppResult<()> {
    for (name, bytes) in files {
        let parts =
            state_file_parts(name).ok_or_else(|| ApiError::bad("the_backup_cannot_be_read"))?;
        let mut dir = ensure_dir(data_dir, "cache")?;
        dir = ensure_dir(&dir, parts[1])?;
        dir = ensure_dir(&dir, "tofu-state")?;
        for part in &parts[3..parts.len() - 1] {
            dir = ensure_dir(&dir, part)?;
        }
        let target = dir.join(parts[parts.len() - 1]);
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => return Err(ApiError::bad("the_backup_cannot_be_read")),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(ApiError::Internal(err.to_string())),
        }
        write_private(&target, bytes)?;
    }
    Ok(())
}

fn clear_state_dirs(data_dir: &Path) -> AppResult<()> {
    let cache = data_dir.join("cache");
    match fs::symlink_metadata(&cache) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(ApiError::Internal(err.to_string())),
    }
    let teams = fs::read_dir(&cache).map_err(|err| ApiError::Internal(err.to_string()))?;
    for team in teams.filter_map(Result::ok) {
        let state = team.path().join("tofu-state");
        match fs::symlink_metadata(&state) {
            Ok(meta) if meta.is_dir() => {
                fs::remove_dir_all(&state).map_err(|err| ApiError::Internal(err.to_string()))?;
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(ApiError::Internal(err.to_string())),
        }
    }
    Ok(())
}

fn state_file_parts(name: &str) -> Option<Vec<&str>> {
    if !safe_relative(name) {
        return None;
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() < 4
        || parts[0] != "cache"
        || parts[2] != "tofu-state"
        || !parts.iter().all(|part| safe_component(part))
    {
        return None;
    }
    Some(parts)
}

fn ensure_dir(parent: &Path, name: &str) -> AppResult<PathBuf> {
    let path = parent.join(name);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_dir() => Ok(path),
        Ok(_) => Err(ApiError::bad("the_backup_cannot_be_read")),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&path).map_err(|err| ApiError::Internal(err.to_string()))?;
            Ok(path)
        }
        Err(err) => Err(ApiError::Internal(err.to_string())),
    }
}

pub(crate) fn own_imported(data_dir: &Path, container_image: bool) {
    #[cfg(unix)]
    {
        use nix::unistd::Uid;
        if !container_image || !Uid::effective().is_root() {
            return;
        }
        let _ = crate::entrypoint::own_tree(data_dir, 10001, 10001);
    }
    #[cfg(not(unix))]
    let _ = (data_dir, container_image);
}

pub(crate) fn busy(c: &Connection) -> AppResult<bool> {
    let jobs: i64 = c.query_row(
        "SELECT COUNT(*) FROM jobs WHERE status IN ('pending', 'running')",
        [],
        |row| row.get(0),
    )?;
    let flows: i64 = c.query_row(
        "SELECT COUNT(*) FROM workflow_jobs WHERE status IN ('pending', 'running')",
        [],
        |row| row.get(0),
    )?;
    Ok(jobs > 0 || flows > 0)
}

fn checked_database(c: &Connection) -> AppResult<()> {
    let status: String = c.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if status != "ok" {
        return Err(ApiError::bad("the_database_in_the_backup_is_damaged"));
    }
    for table in REQUIRED_TABLES {
        if !has_table(c, table)? {
            return Err(ApiError::bad(
                "the_backup_does_not_contain_a_koreograf_database",
            ));
        }
    }
    Ok(())
}

pub(super) fn has_table(c: &Connection, name: &str) -> AppResult<bool> {
    let found: Option<i64> = c
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

pub(crate) fn copy_database(from: &Connection, to: &mut Connection) -> AppResult<()> {
    let backup = Backup::new(from, to)?;
    for _ in 0..20_000 {
        match backup.step(64)? {
            rusqlite::backup::StepResult::Done => return Ok(()),
            rusqlite::backup::StepResult::More => {}
            rusqlite::backup::StepResult::Busy | rusqlite::backup::StepResult::Locked => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    Err(ApiError::Internal("database copy did not finish".into()))
}

pub(super) fn pack(files: &[(&str, &[u8])]) -> AppResult<Vec<u8>> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in files {
            writer
                .start_file(*name, options)
                .map_err(|_| ApiError::Internal("backup cannot be written".into()))?;
            writer
                .write_all(bytes)
                .map_err(|_| ApiError::Internal("backup cannot be written".into()))?;
        }
        writer
            .finish()
            .map_err(|_| ApiError::Internal("backup cannot be written".into()))?;
    }
    Ok(cursor.into_inner())
}

pub(super) fn unpack(bytes: &[u8]) -> AppResult<Vec<(String, Vec<u8>)>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| ApiError::bad("the_backup_cannot_be_read"))?;
    if archive.is_empty() || archive.len() > 10_000 {
        return Err(ApiError::bad("the_backup_cannot_be_read"));
    }
    let mut out = Vec::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|_| ApiError::bad("the_backup_cannot_be_read"))?;
        if file.is_symlink() {
            return Err(ApiError::bad("the_backup_cannot_be_read"));
        }
        if !file.is_file() {
            continue;
        }
        let Some(name) = file.enclosed_name() else {
            return Err(ApiError::bad("the_backup_cannot_be_read"));
        };
        let name = name.to_string_lossy().replace('\\', "/");
        if name.contains("..") || file.size() > MAX_BYTES {
            return Err(ApiError::bad("the_backup_cannot_be_read"));
        }
        let mut buf = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut buf)
            .map_err(|_| ApiError::bad("the_backup_cannot_be_read"))?;
        if buf.len() as u64 > MAX_BYTES {
            return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
        }
        total = total.saturating_add(buf.len() as u64);
        if total > MAX_BYTES {
            return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
        }
        out.push((name, buf));
    }
    Ok(out)
}

fn write_private(path: &Path, bytes: &[u8]) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| ApiError::Internal(err.to_string()))?;
    }
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts
        .open(path)
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    file.write_all(bytes)
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(())
}

const PATH_MAP_KEY: &str = "import_path_map";

impl Options {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            restore_from: env_path("KOREOGRAF_RESTORE_FROM"),
            import_from: env_path("KOREOGRAF_IMPORT_FROM"),
            path_map: parse_path_map(&env_string("KOREOGRAF_PATH_MAP"))?,
            copy_cache: env_flag("KOREOGRAF_IMPORT_CACHE"),
        })
    }
}

pub(crate) fn prepare(cfg: &Config) -> anyhow::Result<()> {
    let options = Options::from_env()?;
    prepare_with(cfg, &options)
}

pub(crate) fn prepare_with(cfg: &Config, options: &Options) -> anyhow::Result<()> {
    fs::create_dir_all(&cfg.data_dir)?;
    let data_dir = std::path::absolute(&cfg.data_dir)?;
    remove_stale_staging(&data_dir);
    if let Some(file) = &options.restore_from {
        if !is_empty(&data_dir) {
            tracing::warn!(
                file = %file.display(),
                "KOREOGRAF_RESTORE_FROM is ignored because the data directory already holds data"
            );
        } else {
            let bytes = fs::read(file).map_err(|err| {
                anyhow::anyhow!("The restore file {} cannot be read: {err}", file.display())
            })?;
            let summary =
                restore_startup(&data_dir, &bytes, cfg.container_image).map_err(|err| {
                    anyhow::anyhow!("Restore from {} failed: {}", file.display(), describe(&err))
                })?;
            tracing::info!(
                file = %file.display(),
                koreograf = %summary.koreograf,
                includes = %summary.includes.join(","),
                "restored the installation from the startup backup"
            );
            mark_applied(file);
        }
    }
    if let Some(source) = &options.import_from {
        if !is_empty(&data_dir) {
            tracing::warn!(
                directory = %source.display(),
                "KOREOGRAF_IMPORT_FROM is ignored because the data directory already holds data"
            );
        } else {
            let report = import_directory(cfg, source, options)?;
            for path in &report.missing_paths {
                tracing::warn!(
                    path = %path,
                    "an imported path does not exist in this container; it needs a mount or a change"
                );
            }
            tracing::info!(
                directory = %source.display(),
                missing_paths = report.missing_paths.len(),
                tofu_files = report.tofu_files,
                path_map = report.path_map.len(),
                "imported the existing data directory"
            );
        }
    }
    Ok(())
}

pub(crate) fn import_directory(
    cfg: &Config,
    source: &Path,
    options: &Options,
) -> anyhow::Result<ImportReport> {
    let metadata = fs::symlink_metadata(source).map_err(|err| {
        anyhow::anyhow!(
            "The import directory {} cannot be read: {err}",
            source.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(anyhow::anyhow!(
            "KOREOGRAF_IMPORT_FROM must be a directory, but {} is not",
            source.display()
        ));
    }
    let source_db = source.join(DATABASE);
    required_file(&source_db, "database")?;
    let source_key = source.join(SECRET);
    required_file(&source_key, "secret.key")?;
    let key = fs::read(&source_key)?;
    crypto::parse_key(&key).map_err(|_| {
        anyhow::anyhow!(
            "The key in {} is not a valid Koreograf key",
            source_key.display()
        )
    })?;

    fs::create_dir_all(&cfg.data_dir)?;
    let dest_db = cfg.data_dir.join(DATABASE);
    if dest_db.exists() {
        return Err(anyhow::anyhow!(
            "The data directory already contains a database; nothing was imported"
        ));
    }
    let source_conn = open_source_database(&source_db)?;
    source_conn.busy_timeout(Duration::from_secs(5))?;
    let version = db::schema_version(&source_conn)?;
    if version > db::SCHEMA_VERSION {
        return Err(anyhow::anyhow!(
            "The database {} uses schema {version}, but this build supports {}. Upgrade Koreograf before importing.",
            source_db.display(),
            db::SCHEMA_VERSION
        ));
    }
    let staging = new_staging(&cfg.data_dir, ".import");
    fs::create_dir_all(&staging)?;
    let result = import_into(&staging, &source_conn, source, cfg, &key, options);
    match result {
        Ok(report) => {
            if let Err(err) = promote(&staging, &cfg.data_dir) {
                let _ = fs::remove_dir_all(&staging);
                return Err(err.into());
            }
            let _ = fs::remove_dir_all(&staging);
            own_imported(&cfg.data_dir, cfg.container_image);
            Ok(report)
        }
        Err(err) => {
            let _ = fs::remove_dir_all(&staging);
            Err(err)
        }
    }
}

fn import_into(
    staging: &Path,
    source_conn: &Connection,
    source: &Path,
    cfg: &Config,
    key: &[u8],
    options: &Options,
) -> anyhow::Result<ImportReport> {
    let dest_db = staging.join(DATABASE);
    let mut dest_conn = Connection::open(&dest_db)?;
    dest_conn.busy_timeout(Duration::from_secs(5))?;
    copy_database(source_conn, &mut dest_conn)?;
    db::ensure_schema(&dest_conn)?;
    crypto::install_key(staging, key)?;
    let mapped = apply_path_map(&dest_conn, &options.path_map)?;
    if mapped == 0 && !options.path_map.is_empty() {
        tracing::warn!("KOREOGRAF_PATH_MAP is already recorded and was not applied again");
    }
    let tofu_files = copy_state(source, staging, options.copy_cache)?;
    let missing_paths = missing_paths(&dest_conn, cfg)?;
    let report = json!({
        "imported_at": db::now(),
        "source": source.display().to_string(),
        "koreograf": env!("CARGO_PKG_VERSION"),
        "schema": db::SCHEMA_VERSION,
        "path_map": options.path_map.iter().map(pair_text).collect::<Vec<_>>(),
        "copy_cache": options.copy_cache,
        "tofu_files": tofu_files,
        "missing_paths": missing_paths,
    });
    set_setting(
        &dest_conn,
        REPORT_KEY,
        &serde_json::to_string(&report).map_err(|err| ApiError::Internal(err.to_string()))?,
    )?;
    drop(dest_conn);
    Ok(ImportReport {
        missing_paths,
        tofu_files,
        path_map: options.path_map.iter().map(pair_text).collect(),
    })
}

fn open_source_database(source_db: &Path) -> anyhow::Result<Connection> {
    let mut wal = source_db.as_os_str().to_os_string();
    wal.push("-wal");
    let wal = PathBuf::from(wal);
    let conn = if wal.is_file() {
        Connection::open_with_flags(
            source_db,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
    } else {
        Connection::open_with_flags(
            read_only_uri(source_db),
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI,
        )
    };
    conn.map_err(|err| {
        anyhow::anyhow!(
            "The database {} cannot be opened read-only: {err}",
            source_db.display()
        )
    })
}

fn read_only_uri(path: &Path) -> String {
    let mut uri = String::from("file:");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                uri.push(byte as char);
            }
            _ => {
                uri.push('%');
                uri.push_str(&format!("{byte:02X}"));
            }
        }
    }
    uri.push_str("?mode=ro&immutable=1");
    uri
}

pub(crate) fn report(db: &Connection) -> AppResult<Option<Value>> {
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [REPORT_KEY],
            |row| row.get(0),
        )
        .optional()?;
    Ok(raw.and_then(|text| serde_json::from_str(&text).ok()))
}

fn required_file(path: &Path, label: &str) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|err| {
        anyhow::anyhow!(
            "The import directory is missing {label} ({}): {err}",
            path.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(anyhow::anyhow!(
            "The import directory has no {label} file at {}",
            path.display()
        ));
    }
    Ok(())
}

fn is_empty(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(true)
}

fn mark_applied(path: &Path) {
    let marker = PathBuf::from(format!("{}.applied", path.display()));
    match fs::rename(path, &marker) {
        Ok(()) => tracing::info!(
            marker = %marker.display(),
            "the startup restore marker was renamed"
        ),
        Err(err) => tracing::warn!(
            marker = %marker.display(),
            error = %err,
            "the startup restore marker could not be renamed"
        ),
    }
}

fn copy_state(source: &Path, data_dir: &Path, copy_cache: bool) -> anyhow::Result<usize> {
    let cache = source.join("cache");
    if !cache.is_dir() {
        return Ok(0);
    }
    if copy_cache {
        return copy_tree(&cache, &data_dir.join("cache"));
    }
    let mut copied = 0;
    for team in fs::read_dir(&cache)? {
        let team = team?;
        let name = team.file_name().to_string_lossy().to_string();
        if name.is_empty() || name.contains(['/', '\\', '\0']) {
            continue;
        }
        let state = team.path().join("tofu-state");
        if !fs::symlink_metadata(&state).is_ok_and(|meta| meta.is_dir()) {
            continue;
        }
        let dest = data_dir.join("cache").join(&name).join("tofu-state");
        copied += copy_tree(&state, &dest)?;
    }
    Ok(copied)
}

fn copy_tree(source: &Path, dest: &Path) -> anyhow::Result<usize> {
    let mut copied = 0;
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let target = dest.join(entry.file_name());
        if metadata.is_dir() {
            copied += copy_tree(&entry.path(), &target)?;
        } else if metadata.is_file() {
            fs::copy(entry.path(), &target)?;
            copied += 1;
        }
    }
    Ok(copied)
}

pub(super) fn apply_path_map(conn: &Connection, map: &[(PathBuf, PathBuf)]) -> AppResult<usize> {
    if map.is_empty() {
        return Ok(0);
    }
    let existing: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [PATH_MAP_KEY],
            |row| row.get(0),
        )
        .optional()?;
    if existing.is_some() {
        return Ok(0);
    }
    let rows = {
        let mut stmt =
            conn.prepare("SELECT id, source FROM projects WHERE source_type = 'path'")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut changed = 0;
    for (id, source) in rows {
        let Some(mapped) = map_one(&source, map) else {
            continue;
        };
        conn.execute(
            "UPDATE projects SET source = ?2 WHERE id = ?1",
            params![id, mapped],
        )?;
        changed += 1;
    }
    set_setting(
        conn,
        PATH_MAP_KEY,
        &serde_json::to_string(&map.iter().map(pair_text).collect::<Vec<_>>())
            .map_err(|err| ApiError::Internal(err.to_string()))?,
    )?;
    Ok(changed)
}

fn map_one(source: &str, map: &[(PathBuf, PathBuf)]) -> Option<String> {
    let path = Path::new(source);
    for (old, new) in map {
        if let Ok(rest) = path.strip_prefix(old) {
            return Some(new.join(rest).display().to_string());
        }
    }
    None
}

fn missing_paths(conn: &Connection, cfg: &Config) -> AppResult<Vec<String>> {
    let mut missing = BTreeSet::new();
    let sources = {
        let mut stmt = conn.prepare("SELECT source FROM projects WHERE source_type = 'path'")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for source in sources {
        let path = Path::new(&source);
        if !path.is_dir() {
            missing.insert(source);
        }
    }
    for root in &cfg.project_roots {
        if !root.is_dir() {
            missing.insert(root.display().to_string());
        }
    }
    Ok(missing.into_iter().collect())
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn pair_text((old, new): &(PathBuf, PathBuf)) -> String {
    format!("{}={}", old.display(), new.display())
}

pub(super) fn parse_path_map(raw: &str) -> anyhow::Result<Vec<(PathBuf, PathBuf)>> {
    let mut map = Vec::new();
    for item in raw
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let Some((old, new)) = item.split_once('=') else {
            return Err(anyhow::anyhow!(
                "KOREOGRAF_PATH_MAP must use OLD=NEW with absolute paths"
            ));
        };
        let old = PathBuf::from(old.trim());
        let new = PathBuf::from(new.trim());
        if !old.is_absolute() || !new.is_absolute() {
            return Err(anyhow::anyhow!(
                "KOREOGRAF_PATH_MAP must use OLD=NEW with absolute paths"
            ));
        }
        map.push((old, new));
    }
    Ok(map)
}

fn describe(err: &ApiError) -> String {
    let key = err.to_string();
    let prose = match key.as_str() {
        "the_backup_is_empty_or_too_large" => "The backup is empty or too large.",
        "the_backup_cannot_be_read" => "The backup cannot be read. It is truncated or not a zip.",
        "the_backup_has_an_unknown_format" => "The backup has an unknown manifest format.",
        "the_backup_does_not_contain_a_koreograf_database" => {
            "The backup does not contain a Koreograf database."
        }
        "the_backup_does_not_contain_a_key" => "The backup does not contain secret.key.",
        "the_key_in_the_backup_is_invalid" => "The key in the backup is invalid.",
        "the_database_in_the_backup_is_damaged" => "The database in the backup is damaged.",
        "the_data_directory_is_not_empty" => "The data directory is not empty.",
        _ => return key,
    };
    prose.to_string()
}

fn env_path(key: &str) -> Option<PathBuf> {
    let value = env_string(key);
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

fn env_string(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

fn env_flag(key: &str) -> bool {
    matches!(
        env_string(key).to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "ja"
    )
}

const MIN_SERVER_VERSION: u32 = 130_000;

impl MigrationStatus {
    pub(crate) fn empty_tables() -> Vec<TableProgress> {
        portable::TABLES
            .iter()
            .map(|table| TableProgress {
                table,
                rows: 0,
                done: false,
            })
            .collect()
    }
}

pub(crate) fn parse_form_target(
    host: &str,
    port: u16,
    database: &str,
    user: &str,
    password: &str,
    sslmode: &str,
    ca_file: Option<&str>,
) -> Result<PostgresConfig, ApiError> {
    let host = host.trim();
    let database = database.trim();
    let user = user.trim();
    let sslmode = sslmode.trim();
    let sslmode = if sslmode.is_empty() {
        "require"
    } else {
        sslmode
    };
    if host.is_empty() || database.is_empty() || user.is_empty() || port == 0 {
        return Err(ApiError::bad("database_target_is_invalid"));
    }
    let mode =
        SslMode::parse(sslmode).ok_or_else(|| ApiError::bad("database_target_is_invalid"))?;
    let mut url = url::Url::parse("postgresql://localhost")
        .map_err(|_| ApiError::bad("database_target_is_invalid"))?;
    url.set_username(user)
        .map_err(|_| ApiError::bad("database_target_is_invalid"))?;
    url.set_password((!password.is_empty()).then_some(password))
        .map_err(|_| ApiError::bad("database_target_is_invalid"))?;
    url.set_host(Some(host))
        .map_err(|_| ApiError::bad("database_target_is_invalid"))?;
    url.set_port(Some(port))
        .map_err(|_| ApiError::bad("database_target_is_invalid"))?;
    url.set_path(&format!("/{database}"));
    url.query_pairs_mut().append_pair("sslmode", mode.as_str());
    let ca_file = ca_file
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    match config::parse_database_config(url.as_str(), None, ca_file) {
        Ok(DatabaseConfig::Postgres(target)) => Ok(target),
        _ => Err(ApiError::bad("database_target_is_invalid")),
    }
}

pub(crate) async fn run_checks(target: &PostgresConfig, source: &Path) -> Vec<CheckResult> {
    let mut checks = Vec::with_capacity(6);
    match check_source(source) {
        Ok(()) => checks.push(check(
            "source_schema",
            true,
            "database_check_source_schema_ok",
        )),
        Err(_) => checks.push(check(
            "source_schema",
            false,
            "database_check_source_schema_failed",
        )),
    }
    match crate::backend::connect_client(target).await {
        Err(err) => {
            tracing::warn!(error = %redact(&err.to_string(), target), "database target is unreachable");
            for id in DATABASE_CHECK_IDS {
                let key = if id == "reachable" {
                    "database_check_unreachable"
                } else {
                    "database_check_not_run"
                };
                checks.push(check(id, false, key));
            }
            checks.push(check("empty", false, "database_check_not_run"));
        }
        Ok(client) => {
            checks.push(check("reachable", true, "database_check_reachable_ok"));
            checks.push(tls_check(&client, target.sslmode).await);
            checks.push(server_version_check(&client).await);
            checks.push(rights_check(&client).await);
            checks.push(empty_check(&client).await);
        }
    }
    checks
}

fn check(id: &'static str, ok: bool, key: &'static str) -> CheckResult {
    CheckResult { id, ok, key }
}

pub(crate) fn all_passed(checks: &[CheckResult]) -> bool {
    checks.iter().all(|item| item.ok)
}

pub(crate) fn audit_entry(user: &str, target: &PostgresConfig) -> AuditEntry {
    AuditEntry {
        user: user.to_string(),
        at: db::now(),
        host: target.target(),
    }
}

pub(crate) fn audit_line(entry: &AuditEntry) -> String {
    format!(
        "database migration: user={} at={} target={}",
        entry.user, entry.at, entry.host
    )
}

fn redact(message: &str, target: &PostgresConfig) -> String {
    let password = target.password();
    if password.is_empty() {
        message.to_string()
    } else {
        message.replace(password, "***")
    }
}

async fn tls_check(client: &Client, mode: SslMode) -> CheckResult {
    if mode == SslMode::Disable {
        return check("tls", true, "database_check_tls_disabled");
    }
    let encrypted = client
        .query_one(
            "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            &[],
        )
        .await
        .ok()
        .and_then(|row| row.get::<_, Option<bool>>(0));
    match encrypted {
        Some(true) => check("tls", true, "database_check_tls_ok"),
        Some(false) if mode == SslMode::Prefer => check("tls", true, "database_check_tls_disabled"),
        Some(false) => check("tls", false, "database_check_tls_failed"),
        None => check("tls", true, "database_check_tls_ok"),
    }
}

async fn server_version_check(client: &Client) -> CheckResult {
    let version = client
        .query_one("SHOW server_version_num", &[])
        .await
        .ok()
        .and_then(|row| row.get::<_, String>(0).parse::<u32>().ok())
        .unwrap_or(0);
    if version >= MIN_SERVER_VERSION {
        check("server_version", true, "database_check_server_version_ok")
    } else {
        check(
            "server_version",
            false,
            "database_check_server_version_failed",
        )
    }
}

async fn rights_check(client: &Client) -> CheckResult {
    let can_create = client
        .query_one(
            "SELECT has_schema_privilege(current_schema(), 'CREATE')",
            &[],
        )
        .await
        .ok()
        .and_then(|row| row.get::<_, Option<bool>>(0));
    if can_create == Some(true) {
        check("rights", true, "database_check_rights_ok")
    } else {
        check("rights", false, "database_check_rights_failed")
    }
}

async fn empty_check(client: &Client) -> CheckResult {
    let tables = match client
        .query(
            "SELECT table_name FROM information_schema.tables
             WHERE table_schema = current_schema() AND table_type = 'BASE TABLE'",
            &[],
        )
        .await
    {
        Ok(tables) => tables,
        Err(_) => return check("empty", false, "database_check_not_empty_failed"),
    };
    for row in tables {
        let table: String = row.get(0);
        if table == "schema_meta" {
            continue;
        }
        let sql = format!("SELECT EXISTS (SELECT 1 FROM {} LIMIT 1)", quote(&table));
        match client.query_one(&sql, &[]).await {
            Ok(row) => {
                let exists: bool = row.get(0);
                if exists {
                    return check("empty", false, "database_check_not_empty_failed");
                }
            }
            Err(_) => return check("empty", false, "database_check_not_empty_failed"),
        }
    }
    check("empty", true, "database_check_empty_ok")
}

pub fn db_migrate_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter().skip(1);
    args.next().is_some_and(|arg| arg.as_ref() == "db")
        && args.next().is_some_and(|arg| arg.as_ref() == "migrate")
}

pub async fn db_migrate(args: &[String]) -> anyhow::Result<i32> {
    let Some(url) = target_argument(args)? else {
        eprintln!("Usage: koreograf db migrate --to postgres://user@host/database");
        return Ok(2);
    };
    match run(&url).await {
        Ok(()) => Ok(0),
        Err(err) => {
            eprintln!("Database migration failed: {err}");
            Ok(1)
        }
    }
}

pub(super) fn target_argument(args: &[String]) -> anyhow::Result<Option<String>> {
    let mut target = None;
    let mut args = args.iter().skip(3);
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--to=") {
            if target.is_some() || value.is_empty() {
                return Ok(None);
            }
            target = Some(value.to_string());
        } else if arg == "--to" {
            let Some(value) = args.next() else {
                return Ok(None);
            };
            if target.is_some() || value.is_empty() {
                return Ok(None);
            }
            target = Some(value.clone());
        } else {
            return Ok(None);
        }
    }
    Ok(target)
}

async fn run(url: &str) -> anyhow::Result<()> {
    let target = parse_target(url)?;
    let cfg = Config::from_env();
    let data_dir = std::path::absolute(&cfg.data_dir)?;
    let _lock = lock_data_directory(&data_dir)?;
    let source = data_dir.join("koreograf.db");
    if !source.is_file() {
        anyhow::bail!(
            "{} does not contain a SQLite database to copy.",
            data_dir.display()
        );
    }
    check_source(&source)?;
    let client = crate::backend::connect_client(&target).await?;
    precheck(&client).await?;
    let rows = match copy_source(&client, &source, |_, _| {}).await {
        Ok(rows) => rows,
        Err(err) => {
            return Err(anyhow::anyhow!(
                "{err}. The target was rolled back and can be reused."
            ));
        }
    };
    println!("Copied {rows} rows into {} tables.", portable::TABLES.len());
    println!("Set this value before the next start and restart Koreograf:");
    println!("KOREOGRAF_DATABASE_URL='{}'", target_url(&target));
    if !target.password().is_empty() {
        println!("The password is not printed. Keep it in KOREOGRAF_DATABASE_PASSWORD_FILE.");
    }
    Ok(())
}

pub(crate) async fn copy_source<F>(
    client: &Client,
    source: &Path,
    progress: F,
) -> anyhow::Result<i64>
where
    F: FnMut(&'static str, i64),
{
    db::ensure_postgres_schema(client).await?;
    client.batch_execute("BEGIN").await?;
    let copied = async {
        portable::copy_from_sqlite_with(client, source, false, progress).await?;
        let rows = portable::verify_counts(client, source).await?;
        portable::set_schema_version(client).await?;
        Ok::<i64, ApiError>(rows)
    }
    .await;
    match copied {
        Ok(rows) => {
            client.batch_execute("COMMIT").await?;
            Ok(rows)
        }
        Err(err) => {
            let _ = client.batch_execute("ROLLBACK").await;
            Err(anyhow::anyhow!("{err}"))
        }
    }
}

pub(super) fn parse_target(url: &str) -> anyhow::Result<PostgresConfig> {
    let password_file = migrate_env_path("KOREOGRAF_DATABASE_PASSWORD_FILE");
    let ca_file = migrate_env_path("KOREOGRAF_DATABASE_CA_FILE");
    match config::parse_database_config(url, password_file, ca_file) {
        Ok(DatabaseConfig::Postgres(target)) => Ok(target),
        Ok(DatabaseConfig::Sqlite) => {
            anyhow::bail!("db migrate needs a postgres:// or postgresql:// target.")
        }
        Err(err) => anyhow::bail!("{err}"),
    }
}

fn migrate_env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub(crate) fn check_source(source: &Path) -> anyhow::Result<()> {
    let conn = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.pragma_update(None, "query_only", true)?;
    let version = db::schema_version(&conn)?;
    if version != db::SCHEMA_VERSION {
        anyhow::bail!(
            "The SQLite database uses schema {version}, but this build copies schema {}. Start the current build once against the SQLite database before copying it.",
            db::SCHEMA_VERSION
        );
    }
    Ok(())
}

pub(crate) async fn precheck(client: &Client) -> anyhow::Result<()> {
    let version: String = client
        .query_one("SHOW server_version_num", &[])
        .await?
        .get(0);
    let version: u32 = version.parse().unwrap_or(0);
    if version < MIN_SERVER_VERSION {
        anyhow::bail!("PostgreSQL {version} is not supported. Use PostgreSQL 13 or newer.");
    }
    let can_create: Option<bool> = client
        .query_one(
            "SELECT has_schema_privilege(current_schema(), 'CREATE')",
            &[],
        )
        .await?
        .get(0);
    if can_create != Some(true) {
        anyhow::bail!("The PostgreSQL user cannot create tables in the current schema.");
    }
    let tables = client
        .query(
            "SELECT table_name FROM information_schema.tables
             WHERE table_schema = current_schema() AND table_type = 'BASE TABLE'",
            &[],
        )
        .await?;
    for row in tables {
        let table: String = row.get(0);
        if table == "schema_meta" {
            continue;
        }
        let sql = format!("SELECT EXISTS (SELECT 1 FROM {} LIMIT 1)", quote(&table));
        let exists: bool = client.query_one(&sql, &[]).await?.get(0);
        if exists {
            anyhow::bail!("The PostgreSQL target is not empty: table {table} already holds rows.");
        }
    }
    Ok(())
}

pub(crate) fn target_url(target: &PostgresConfig) -> String {
    let mut url = match url::Url::parse("postgresql://localhost") {
        Ok(url) => url,
        Err(_) => {
            return format!(
                "postgresql://{}:{}/{}",
                target.host, target.port, target.database
            )
        }
    };
    let _ = url.set_username(&target.user);
    let _ = url.set_password(None);
    let _ = url.set_host(Some(&target.host));
    let _ = url.set_port(Some(target.port));
    url.set_path(&format!("/{}", target.database));
    url.query_pairs_mut()
        .append_pair("sslmode", target.sslmode.as_str());
    url.to_string()
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

const LABEL_PLAYBOOK: usize = 120;
const LABEL_ROLE: usize = 80;
const LABEL_TEAM: usize = 80;
const LABEL_TEMPLATE: usize = 80;
const LABEL_TOOL: usize = 32;
/// The two closes of one job land before this many other jobs finish.
/// Older ids are dropped so the set does not grow for the process lifetime.
pub(super) const CLOSED_LIMIT: usize = 1024;

/// Tool, entry path, team, and template stored on the job.
pub(crate) fn job_labels(job: &db::Job) -> JobLabels {
    let params = db::json_object(&job.parameters);
    let tool = params
        .get("tool")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(job.tool.as_str());
    let playbook = params
        .get("entrypoint")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let template = params
        .get("template_name")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(job.name.as_str());
    JobLabels {
        tool: tool.to_string(),
        playbook: playbook.to_string(),
        team_id: job.team_id.clone(),
        template: template.to_string(),
    }
}

struct OpenJob {
    tool: String,
    playbook: String,
    team_id: String,
    template: String,
    started: Instant,
    observer: PlayObserver,
}

struct Book {
    started: BTreeMap<(String, String), u64>,
    finished: BTreeMap<(String, String, String), u64>,
    roles: BTreeMap<(String, String, &'static str), u64>,
    open: HashMap<String, OpenJob>,
    closed: HashSet<String>,
    closed_order: VecDeque<String>,
}

pub(crate) struct Metrics {
    inner: Mutex<Book>,
}

struct RoleSummary {
    role: String,
    ok: u64,
    changed: u64,
    failed: u64,
    unreachable: u64,
    skipped: u64,
}

enum Step {
    Host { role: String, result: &'static str },
    Done(RoleSummary),
}

enum LogEv {
    Host {
        playbook: String,
        role: String,
        result: &'static str,
    },
    Done {
        playbook: String,
        summary: RoleSummary,
    },
}

struct ClosedJob {
    tool: String,
    playbook: String,
    team_id: String,
    template: String,
    status: String,
    duration_ms: u64,
    summary: Option<RoleSummary>,
}

#[derive(Default)]
struct PlayObserver {
    role: Option<String>,
    ok: u64,
    changed: u64,
    failed: u64,
    unreachable: u64,
    skipped: u64,
    saw_play: bool,
    saw_host: bool,
    saw_work: bool,
}

impl Metrics {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(Book {
                started: BTreeMap::new(),
                finished: BTreeMap::new(),
                roles: BTreeMap::new(),
                open: HashMap::new(),
                closed: HashSet::new(),
                closed_order: VecDeque::new(),
            }),
        }
    }

    pub(crate) fn begin(&self, meta: &JobMeta<'_>) {
        let Some(open) = self.record_begin(meta) else {
            return;
        };
        tracing::info!(
            target: "koreograf::job",
            job_id = meta.job_id,
            team_id = %open.team_id,
            tool = %open.tool,
            playbook = %open.playbook,
            template = %open.template,
            "job started"
        );
    }

    pub(crate) fn observe(&self, job_id: &str, line: &str) {
        let Some(events) = self.record_line(job_id, line) else {
            return;
        };
        emit(job_id, events);
    }

    /// A tag filter reached hosts and never ran a task other than Gathering Facts.
    pub(crate) fn tag_filter_matched_nothing(&self, job_id: &str) -> bool {
        self.inner
            .lock()
            .ok()
            .and_then(|book| {
                let open = book.open.get(job_id)?;
                (open.tool == "ansible").then_some(open.observer.matched_nothing())
            })
            .unwrap_or(false)
    }

    pub(crate) fn close(&self, job_id: &str, status: &str, fallback: Option<&JobMeta<'_>>) {
        let Some(closed) = self.record_close(job_id, status, false, fallback) else {
            return;
        };
        if let Some(summary) = closed.summary {
            log_role_finished(job_id, &closed.playbook, &summary);
        }
        tracing::info!(
            target: "koreograf::job",
            job_id,
            team_id = %closed.team_id,
            tool = %closed.tool,
            playbook = %closed.playbook,
            template = %closed.template,
            status = %closed.status,
            duration_ms = closed.duration_ms,
            "job finished"
        );
    }

    /// Counts a terminal job that never reached `begin`. A later `close` is a no-op.
    pub(crate) fn finish_unstarted(&self, meta: &JobMeta<'_>, status: &str) {
        let Some(closed) = self.record_close(meta.job_id, status, true, Some(meta)) else {
            return;
        };
        tracing::info!(
            target: "koreograf::job",
            job_id = meta.job_id,
            team_id = %closed.team_id,
            tool = %closed.tool,
            playbook = %closed.playbook,
            template = %closed.template,
            status = %closed.status,
            duration_ms = closed.duration_ms,
            "job finished"
        );
    }

    pub(crate) fn render(&self, caches: &[CacheGauge]) -> String {
        let Ok(book) = self.inner.lock() else {
            return String::new();
        };
        let mut out = String::new();
        family(
            &mut out,
            "koreograf_jobs_started_total",
            "counter",
            "Jobs that began execution.",
        );
        for ((tool, playbook), value) in &book.started {
            sample(
                &mut out,
                "koreograf_jobs_started_total",
                &[("tool", tool), ("playbook", playbook)],
                *value,
            );
        }
        family(
            &mut out,
            "koreograf_jobs_finished_total",
            "counter",
            "Jobs that reached a terminal status.",
        );
        for ((tool, playbook, status), value) in &book.finished {
            sample(
                &mut out,
                "koreograf_jobs_finished_total",
                &[("tool", tool), ("playbook", playbook), ("status", status)],
                *value,
            );
        }
        family(
            &mut out,
            "koreograf_role_results_total",
            "counter",
            "Ansible role task results seen in playbook output.",
        );
        for ((playbook, role, result), value) in &book.roles {
            sample(
                &mut out,
                "koreograf_role_results_total",
                &[("playbook", playbook), ("role", role), ("result", result)],
                *value,
            );
        }
        family(
            &mut out,
            "koreograf_cache_collections",
            "gauge",
            "Ansible collections stored in the team cache.",
        );
        for gauge in caches {
            sample(
                &mut out,
                "koreograf_cache_collections",
                &[("team_id", &gauge.team_id), ("team", &gauge.team)],
                gauge.collections,
            );
        }
        family(
            &mut out,
            "koreograf_cache_roles",
            "gauge",
            "Ansible roles stored in the team cache.",
        );
        for gauge in caches {
            sample(
                &mut out,
                "koreograf_cache_roles",
                &[("team_id", &gauge.team_id), ("team", &gauge.team)],
                gauge.roles,
            );
        }
        family(
            &mut out,
            "koreograf_cache_bytes",
            "gauge",
            "Cache size in bytes.",
        );
        for gauge in caches {
            let labels = [
                ("team_id", gauge.team_id.as_str()),
                ("team", gauge.team.as_str()),
            ];
            sample(
                &mut out,
                "koreograf_cache_bytes",
                &[labels[0], labels[1], ("kind", "collections")],
                gauge.collections_bytes,
            );
            sample(
                &mut out,
                "koreograf_cache_bytes",
                &[labels[0], labels[1], ("kind", "roles")],
                gauge.roles_bytes,
            );
            sample(
                &mut out,
                "koreograf_cache_bytes",
                &[labels[0], labels[1], ("kind", "tofu")],
                gauge.tofu_bytes,
            );
        }
        out
    }

    fn record_begin(&self, meta: &JobMeta<'_>) -> Option<OpenJob> {
        let mut book = self.inner.lock().ok()?;
        if book.open.contains_key(meta.job_id) || book.closed.contains(meta.job_id) {
            return None;
        }
        let open = OpenJob {
            tool: sanitize_label(meta.tool, LABEL_TOOL),
            playbook: sanitize_label(meta.playbook, LABEL_PLAYBOOK),
            team_id: sanitize_label(meta.team_id, LABEL_TEAM),
            template: sanitize_label(meta.template, LABEL_TEMPLATE),
            started: Instant::now(),
            observer: PlayObserver::default(),
        };
        *book
            .started
            .entry((open.tool.clone(), open.playbook.clone()))
            .or_default() += 1;
        book.open.insert(meta.job_id.to_string(), clone_open(&open));
        Some(open)
    }

    fn record_line(&self, job_id: &str, line: &str) -> Option<Vec<LogEv>> {
        let mut book = self.inner.lock().ok()?;
        let open = book.open.get_mut(job_id)?;
        if open.tool != "ansible" {
            return None;
        }
        let playbook = open.playbook.clone();
        let steps = open.observer.push(line);
        let mut events = Vec::new();
        for step in steps {
            match step {
                Step::Host { role, result } => {
                    *book
                        .roles
                        .entry((playbook.clone(), role.clone(), result))
                        .or_default() += 1;
                    events.push(LogEv::Host {
                        playbook: playbook.clone(),
                        role,
                        result,
                    });
                }
                Step::Done(summary) => events.push(LogEv::Done {
                    playbook: playbook.clone(),
                    summary,
                }),
            }
        }
        Some(events)
    }

    fn record_close(
        &self,
        job_id: &str,
        status: &str,
        only_unstarted: bool,
        fallback: Option<&JobMeta<'_>>,
    ) -> Option<ClosedJob> {
        let mut book = self.inner.lock().ok()?;
        if book.closed.contains(job_id) {
            return None;
        }
        let status = terminal_status(status).to_string();
        let closed = if let Some(mut open) = book.open.remove(job_id) {
            if only_unstarted {
                book.open.insert(job_id.to_string(), open);
                return None;
            }
            let summary = open.observer.finish();
            let duration_ms = u64::try_from(open.started.elapsed().as_millis()).unwrap_or(u64::MAX);
            ClosedJob {
                tool: open.tool,
                playbook: open.playbook,
                team_id: open.team_id,
                template: open.template,
                status,
                duration_ms,
                summary,
            }
        } else {
            let (tool, playbook, team_id, template) = match fallback {
                Some(meta) => (
                    sanitize_label(meta.tool, LABEL_TOOL),
                    sanitize_label(meta.playbook, LABEL_PLAYBOOK),
                    sanitize_label(meta.team_id, LABEL_TEAM),
                    sanitize_label(meta.template, LABEL_TEMPLATE),
                ),
                None => (
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                ),
            };
            ClosedJob {
                tool,
                playbook,
                team_id,
                template,
                status,
                duration_ms: 0,
                summary: None,
            }
        };
        *book
            .finished
            .entry((
                closed.tool.clone(),
                closed.playbook.clone(),
                closed.status.clone(),
            ))
            .or_default() += 1;
        remember_closed(&mut book, job_id);
        Some(closed)
    }
}

fn remember_closed(book: &mut Book, job_id: &str) {
    if book.closed.contains(job_id) {
        return;
    }
    let id = job_id.to_string();
    book.closed.insert(id.clone());
    book.closed_order.push_back(id);
    if book.closed_order.len() > CLOSED_LIMIT {
        if let Some(old) = book.closed_order.pop_front() {
            book.closed.remove(&old);
        }
    }
}

fn clone_open(open: &OpenJob) -> OpenJob {
    OpenJob {
        tool: open.tool.clone(),
        playbook: open.playbook.clone(),
        team_id: open.team_id.clone(),
        template: open.template.clone(),
        started: open.started,
        observer: PlayObserver::default(),
    }
}

fn emit(job_id: &str, events: Vec<LogEv>) {
    for event in events {
        match event {
            LogEv::Host {
                playbook,
                role,
                result,
            } if result == "failed" || result == "unreachable" => {
                tracing::warn!(
                    target: "koreograf::role",
                    job_id,
                    playbook,
                    role,
                    result,
                    "Rollenergebnis"
                );
            }
            LogEv::Host { .. } => {}
            LogEv::Done { playbook, summary } => log_role_finished(job_id, &playbook, &summary),
        }
    }
}

fn log_role_finished(job_id: &str, playbook: &str, summary: &RoleSummary) {
    tracing::info!(
        target: "koreograf::role",
        job_id,
        playbook,
        role = %summary.role,
        ok = summary.ok,
        changed = summary.changed,
        failed = summary.failed,
        unreachable = summary.unreachable,
        skipped = summary.skipped,
        "Rolle beendet"
    );
}

pub(crate) fn log_cache(data_dir: &Path, job_id: &str, team_id: &str) {
    let report = cacheinfo::team_cache(data_dir, team_id);
    log_cache_report(job_id, team_id, &report);
}

fn log_cache_report(job_id: &str, team_id: &str, report: &CacheReport) {
    tracing::info!(
        target: "koreograf::cache",
        job_id,
        team_id,
        collections = report.collections.len(),
        roles = report.roles.len(),
        collections_bytes = report.collections_bytes,
        roles_bytes = report.roles_bytes,
        tofu_bytes = report.tofu_bytes,
        "Cache-Stand"
    );
}

pub(crate) fn scrape(state: &Shared) -> String {
    let ids = cache_team_ids(&state.cfg.data_dir);
    let names = team_names(state, &ids);
    let gauges = gauges_from_dir(&state.cfg.data_dir, &names);
    state.metrics.render(&gauges)
}

pub(crate) fn gauges_from_dir(data_dir: &Path, names: &HashMap<String, String>) -> Vec<CacheGauge> {
    cache_team_ids(data_dir)
        .into_iter()
        .map(|id| {
            let report = cacheinfo::team_cache(data_dir, &id);
            let team = names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| sanitize_label(&id, LABEL_TEAM));
            CacheGauge {
                team_id: sanitize_label(&id, LABEL_TEAM),
                team: sanitize_label(&team, LABEL_TEAM),
                collections: as_u64(report.collections.len()),
                roles: as_u64(report.roles.len()),
                collections_bytes: report.collections_bytes,
                roles_bytes: report.roles_bytes,
                tofu_bytes: report.tofu_bytes,
            }
        })
        .collect()
}

fn team_names(state: &Shared, ids: &[String]) -> HashMap<String, String> {
    let db = state.store();
    let mut names = HashMap::new();
    for id in ids {
        if let Ok(Some(team)) = db.team_by_id(id) {
            names.insert(id.clone(), team.name);
        }
    }
    names
}

fn cache_team_ids(data_dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(data_dir.join("cache")) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for entry in entries.flatten() {
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.is_empty() && !name.contains(['/', '\\']) {
                ids.push(name);
            }
        }
    }
    ids.sort();
    ids
}

fn as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn terminal_status(status: &str) -> &str {
    match status {
        "successful" | "failed" | "canceled" => status,
        _ => "failed",
    }
}

impl PlayObserver {
    fn push(&mut self, line: &str) -> Vec<Step> {
        let cleaned = strip_ansi(line);
        let line = cleaned.trim();
        if line.is_empty() || line.starts_with("KOREOGRAF") {
            return Vec::new();
        }
        if line.starts_with("PLAY ") {
            self.saw_play = true;
            return self.end_role().into_iter().map(Step::Done).collect();
        }
        if host_result(line).is_some() || play_recap_host(line) {
            self.saw_host = true;
        }
        if let Some(task) = task_name(line) {
            if task != "Gathering Facts" {
                self.saw_work = true;
            }
            let mut steps = Vec::new();
            let next = role_of_task(&task);
            if self.role.as_deref() != next.as_deref() {
                if let Some(done) = self.end_role() {
                    steps.push(Step::Done(done));
                }
                self.role = next;
            }
            return steps;
        }
        let Some(result) = host_result(line) else {
            return Vec::new();
        };
        let Some(role) = self.role.clone() else {
            return Vec::new();
        };
        match result {
            "ok" => self.ok += 1,
            "changed" => self.changed += 1,
            "failed" => self.failed += 1,
            "unreachable" => self.unreachable += 1,
            "skipped" => self.skipped += 1,
            _ => {}
        }
        vec![Step::Host { role, result }]
    }

    fn finish(&mut self) -> Option<RoleSummary> {
        self.end_role()
    }

    fn matched_nothing(&self) -> bool {
        self.saw_play && self.saw_host && !self.saw_work
    }

    fn end_role(&mut self) -> Option<RoleSummary> {
        let role = self.role.take()?;
        let summary = RoleSummary {
            role,
            ok: self.ok,
            changed: self.changed,
            failed: self.failed,
            unreachable: self.unreachable,
            skipped: self.skipped,
        };
        self.ok = 0;
        self.changed = 0;
        self.failed = 0;
        self.unreachable = 0;
        self.skipped = 0;
        if summary.ok + summary.changed + summary.failed + summary.unreachable + summary.skipped
            == 0
        {
            None
        } else {
            Some(summary)
        }
    }
}

fn play_recap_host(line: &str) -> bool {
    let Some(marker) = line.find(" : ok=").or_else(|| line.find(": ok=")) else {
        return false;
    };
    let host = line[..marker].trim();
    !host.is_empty() && !host.contains(' ') && line.contains("changed=")
}

fn task_name(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("TASK ")
        .or_else(|| line.strip_prefix("RUNNING HANDLER "))?;
    let rest = rest.trim_start().strip_prefix('[')?;
    let end = rest.rfind(']')?;
    Some(rest[..end].trim().to_string())
}

fn role_of_task(task: &str) -> Option<String> {
    let (role, _) = task.split_once(" : ")?;
    let role = sanitize_label(role.trim(), LABEL_ROLE);
    if role == "-" {
        None
    } else {
        Some(role)
    }
}

fn host_result(line: &str) -> Option<&'static str> {
    let (kind, rest) = line.split_once(':')?;
    if !rest.trim_start().starts_with('[') {
        return None;
    }
    match kind.trim() {
        "ok" => Some("ok"),
        "changed" => Some("changed"),
        "fatal" | "failed" => Some("failed"),
        "unreachable" => Some("unreachable"),
        "skipping" => Some("skipped"),
        _ => None,
    }
}

fn strip_ansi(line: &str) -> Cow<'_, str> {
    if !line.contains('\u{1b}') {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.next() == Some('[') {
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }
    Cow::Owned(out)
}

fn sanitize_label(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            ch if ch.is_control() => {}
            ch => out.push(ch),
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        "-".to_string()
    } else {
        trimmed.to_string()
    }
}

fn prom_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '"' => out.push_str("\\\""),
            ch => out.push(ch),
        }
    }
    out
}

fn family(out: &mut String, name: &str, kind: &str, help: &str) {
    out.push_str("# HELP ");
    out.push_str(name);
    out.push(' ');
    out.push_str(help);
    out.push('\n');
    out.push_str("# TYPE ");
    out.push_str(name);
    out.push(' ');
    out.push_str(kind);
    out.push('\n');
}

fn sample(out: &mut String, name: &str, labels: &[(&str, &str)], value: u64) {
    out.push_str(name);
    if !labels.is_empty() {
        out.push('{');
        for (index, (key, value)) in labels.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(key);
            out.push_str("=\"");
            out.push_str(&prom_escape(value));
            out.push('"');
        }
        out.push('}');
    }
    out.push(' ');
    out.push_str(&value.to_string());
    out.push('\n');
}
