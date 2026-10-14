//! Saved admin settings. Environment variables stay in effect until an
//! administrator saves. After that, the database wins without a restart.

use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::config::{LdapConfig, RunnerMode};
use crate::error::{ApiError, AppResult};
use crate::Shared;

const SAVED: &str = "saved";
const SECRET_MARK: &str = "********";
const LDAP_PASSWORD: &str = "ldap_bind_password";
const PROXY_PASSWORD: &str = "proxy_password";
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

#[derive(Clone)]
pub struct ProxySettings {
    pub url: String,
    pub username: String,
    pub password: String,
    pub no_proxy: String,
    /// `true` after an administrator has saved. Callers then replace the
    /// process environment instead of inheriting it.
    pub explicit: bool,
}

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

pub struct Effective {
    /// Directory fields for the form. Login uses [`Effective::directory`].
    pub ldap: LdapConfig,
    pub proxy: ProxySettings,
    pub runner: RunnerMode,
    pub job_timeout_secs: u64,
    pub saved: bool,
    pub unsupported_proxy: bool,
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
}

pub struct SettingsWrite {
    pub ldap_url: String,
    pub ldap_mode: String,
    pub ldap_bind_dn: String,
    pub ldap_bind_password: String,
    pub ldap_user_base: String,
    pub ldap_user_filter: String,
    pub ldap_username_attr: String,
    pub ldap_group_attr: String,
    pub ldap_group_base: String,
    pub ldap_starttls: bool,
    pub ldap_tls_insecure: bool,
    pub proxy_url: String,
    pub proxy_username: String,
    pub proxy_password: String,
    pub proxy_no_proxy: String,
    pub runner: String,
    pub job_timeout_secs: u64,
    pub clear_ldap_password: bool,
    pub clear_proxy_password: bool,
    pub reset: bool,
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
}

pub fn load(state: &Shared) -> AppResult<Effective> {
    let rows = {
        let db = state.db()?;
        read_rows(&db)?
    };
    if rows.get(SAVED).map(String::as_str) == Some("1") {
        return from_saved(state, &rows);
    }
    Ok(from_env(state))
}

pub fn view(state: &Shared) -> AppResult<Value> {
    Ok(public_json(&load(state)?))
}

pub fn save(state: &Shared, input: &SettingsWrite) -> AppResult<Value> {
    if input.reset {
        let db = state.db()?;
        db.execute("DELETE FROM settings", [])?;
        drop(db);
        return Ok(public_json(&from_env(state)));
    }
    let current = load(state)?;
    let draft = validate(input, &current)?;
    let ldap_secret = encrypt_secret(state, &draft.ldap_bind_password)?;
    let proxy_secret = encrypt_secret(state, &draft.proxy_password)?;
    let mut db = state.db()?;
    let tx = db.transaction()?;
    tx.execute("DELETE FROM settings", [])?;
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
    ];
    if let Some(secret) = ldap_secret {
        pairs.push((LDAP_PASSWORD, secret));
    }
    if let Some(secret) = proxy_secret {
        pairs.push((PROXY_PASSWORD, secret));
    }
    for (key, value) in &pairs {
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
    }
    tx.commit()?;
    drop(db);
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
    Ok(public_json(&effective))
}

fn public_json(effective: &Effective) -> Value {
    let ldap = &effective.ldap;
    json!({
        "source": if effective.saved { "saved" } else { "env" },
        "ldap": {
            "url": ldap.url,
            "mode": ldap.mode,
            "bind_dn": ldap.bind_dn,
            "bind_password_set": !ldap.bind_password.is_empty(),
            "user_base": ldap.user_base,
            "user_filter": ldap.user_filter,
            "username_attr": ldap.username_attr,
            "group_attr": ldap.group_attr,
            "group_base": ldap.group_base,
            "starttls": ldap.starttls,
            "tls_insecure": ldap.tls_insecure,
        },
        "proxy": {
            "url": effective.proxy.url,
            "username": effective.proxy.username,
            "password_set": !effective.proxy.password.is_empty(),
            "no_proxy": effective.proxy.no_proxy,
            "unsupported_env": effective.unsupported_proxy,
        },
        "runner": effective.runner.as_str(),
        "job_timeout_secs": effective.job_timeout_secs,
    })
}

fn from_env(state: &Shared) -> Effective {
    let (proxy, unsupported_proxy) = proxy_from_env();
    let ldap = state.cfg.ldap.clone().unwrap_or_else(empty_ldap);
    Effective {
        ldap,
        proxy,
        runner: state.cfg.runner,
        job_timeout_secs: state.cfg.job_timeout_secs,
        saved: false,
        unsupported_proxy,
    }
}

fn from_saved(state: &Shared, rows: &HashMap<String, String>) -> AppResult<Effective> {
    let ldap_password = decrypt_secret(state, rows.get(LDAP_PASSWORD))?;
    let proxy_password = decrypt_secret(state, rows.get(PROXY_PASSWORD))?;
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

fn validate(input: &SettingsWrite, current: &Effective) -> AppResult<Draft> {
    let ldap_mode = match input.ldap_mode.trim().to_ascii_lowercase().as_str() {
        "ldap" => "ldap",
        "ad" => "ad",
        "" => "ldap",
        _ => return Err(ApiError::bad("LDAP-Modus ist ungültig.")),
    };
    let ad = ldap_mode == "ad";
    let ldap_url = plain(&input.ldap_url, 400)?;
    if !valid_ldap_url(&ldap_url) {
        return Err(ApiError::bad(
            "LDAP-Adresse muss ldap:// oder ldaps:// sein.",
        ));
    }
    let ldap_bind_dn = plain(&input.ldap_bind_dn, 400)?;
    let ldap_user_base = plain(&input.ldap_user_base, 400)?;
    let ldap_group_base = plain(&input.ldap_group_base, 400)?;
    let mut ldap_user_filter = plain(&input.ldap_user_filter, 400)?;
    if ldap_user_filter.is_empty() {
        ldap_user_filter = filter_default(ad).to_string();
    } else if !ldap_user_filter.contains("{username}") {
        return Err(ApiError::bad("LDAP-Filter braucht {username}."));
    }
    let mut ldap_username_attr = plain(&input.ldap_username_attr, 80)?;
    if ldap_username_attr.is_empty() {
        ldap_username_attr = attr_default(ad).to_string();
    } else if !valid_attr(&ldap_username_attr) {
        return Err(ApiError::bad("Der Wert enthält ungültige Zeichen."));
    }
    let mut ldap_group_attr = plain(&input.ldap_group_attr, 80)?;
    if ldap_group_attr.is_empty() {
        ldap_group_attr = "memberOf".to_string();
    } else if !valid_attr(&ldap_group_attr) {
        return Err(ApiError::bad("Der Wert enthält ungültige Zeichen."));
    }
    if !ldap_url.is_empty() && (ldap_bind_dn.is_empty() || ldap_user_base.is_empty()) {
        return Err(ApiError::bad("LDAP ist unvollständig konfiguriert."));
    }
    let current_ldap_password = current.ldap.bind_password.as_str();
    let ldap_bind_password = resolve_secret(
        &input.ldap_bind_password,
        input.clear_ldap_password,
        current_ldap_password,
    )?;
    let proxy_url = plain(&input.proxy_url, 300)?;
    if proxy_url.contains('@') {
        return Err(ApiError::bad(
            "Zugangsdaten gehören in die eigenen Felder, nicht in die Proxy-Adresse.",
        ));
    }
    if !valid_proxy_url(&proxy_url) {
        return Err(ApiError::bad(
            "Proxy-Adresse muss http:// oder https:// sein.",
        ));
    }
    let proxy_username = plain(&input.proxy_username, 200)?;
    let proxy_no_proxy = plain(&input.proxy_no_proxy, 500)?;
    if !valid_no_proxy(&proxy_no_proxy) {
        return Err(ApiError::bad("Proxy-Ausnahmen sind ungültig."));
    }
    let proxy_password = resolve_secret(
        &input.proxy_password,
        input.clear_proxy_password,
        &current.proxy.password,
    )?;
    if !proxy_url.is_empty() && proxy_username.is_empty() && !proxy_password.is_empty() {
        return Err(ApiError::bad("Proxy-Benutzer fehlt."));
    }
    let runner = match input.runner.trim() {
        "auto" => RunnerMode::Auto,
        "docker" => RunnerMode::Docker,
        "local" => RunnerMode::Local,
        _ => return Err(ApiError::bad("Runner-Modus ist ungültig.")),
    };
    if !(30..=86_400).contains(&input.job_timeout_secs) {
        return Err(ApiError::bad(
            "Zeitlimit muss zwischen 30 und 86400 Sekunden liegen.",
        ));
    }
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
    })
}

fn resolve_secret(incoming: &str, clear: bool, current: &str) -> AppResult<String> {
    let incoming = incoming.trim();
    if !incoming.is_empty() && incoming != SECRET_MARK {
        if incoming.chars().count() > 1024 {
            return Err(ApiError::bad("Der Wert ist zu lang."));
        }
        if incoming
            .chars()
            .any(|ch| ch == '\0' || ch == '\n' || ch == '\r')
        {
            return Err(ApiError::bad("Der Wert enthält ungültige Zeichen."));
        }
        return Ok(incoming.to_string());
    }
    if incoming == SECRET_MARK && current.is_empty() {
        return Err(ApiError::bad("Das Platzhalterzeichen ist kein Passwort."));
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
        .map_err(|_| ApiError::bad("Gespeicherte Einstellung lässt sich nicht lesen."))
}

fn read_rows(db: &Connection) -> AppResult<HashMap<String, String>> {
    let mut stmt = db.prepare("SELECT key, value FROM settings")?;
    let mapped = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut rows = HashMap::new();
    for item in mapped {
        let (key, value) = item?;
        rows.insert(key, value);
    }
    Ok(rows)
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
    } else if let Some(rest) = raw.strip_prefix("http://") {
        ("http://", rest)
    } else {
        return None;
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
        return Err(ApiError::bad("Der Wert ist zu lang."));
    }
    if value.chars().any(|ch| ch.is_control()) {
        return Err(ApiError::bad("Der Wert enthält ungültige Zeichen."));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> Shared {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-settings-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        crate::boot(crate::Config {
            bind: "127.0.0.1:0".into(),
            data_dir: dir,
            web_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
            examples_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
            admin_user: "admin".into(),
            admin_password: Some("test-admin-pass".into()),
            runner: RunnerMode::Local,
            job_timeout_secs: 60,
            ldap: None,
        })
        .unwrap()
    }

    fn sample(password: &str) -> SettingsWrite {
        SettingsWrite {
            ldap_url: "ldap://ldap.example.com".into(),
            ldap_mode: "ldap".into(),
            ldap_bind_dn: "cn=svc,dc=example,dc=com".into(),
            ldap_bind_password: password.into(),
            ldap_user_base: "ou=users,dc=example,dc=com".into(),
            ldap_user_filter: "".into(),
            ldap_username_attr: "".into(),
            ldap_group_attr: "".into(),
            ldap_group_base: "".into(),
            ldap_starttls: true,
            ldap_tls_insecure: false,
            proxy_url: "http://proxy.example:3128".into(),
            proxy_username: "a b".into(),
            proxy_password: "p@ss:word".into(),
            proxy_no_proxy: "localhost, 127.0.0.1".into(),
            runner: "docker".into(),
            job_timeout_secs: 120,
            clear_ldap_password: false,
            clear_proxy_password: false,
            reset: false,
        }
    }

    #[test]
    fn proxy_env_encodes_userinfo() {
        let proxy = ProxySettings {
            url: "http://proxy.example:3128".into(),
            username: "a b".into(),
            password: "p@ss:word".into(),
            no_proxy: "localhost".into(),
            explicit: true,
        };
        let value = proxy
            .env_pairs()
            .into_iter()
            .find(|(key, _)| key == "http_proxy")
            .unwrap()
            .1;
        assert_eq!(value, "http://a%20b:p%40ss%3Aword@proxy.example:3128");
        assert!(!value.contains("p@ss:word"));
    }

    #[test]
    fn saved_secret_stays_out_of_the_public_view() {
        let state = state();
        let saved = save(&state, &sample("bind-secret")).unwrap();
        let body = saved.to_string();
        assert!(!body.contains("bind-secret"));
        assert!(!body.contains("p@ss:word"));
        assert_eq!(saved["source"], "saved");
        assert_eq!(saved["ldap"]["bind_password_set"], true);
        assert_eq!(saved["proxy"]["password_set"], true);
        assert_eq!(saved["ldap"]["user_filter"], "(uid={username})");
        assert_eq!(saved["runner"], "docker");
        let loaded = load(&state).unwrap();
        assert!(loaded.directory().is_some());
        assert_eq!(loaded.ldap.bind_password, "bind-secret");
        assert!(loaded.ldap.starttls);
        assert_eq!(loaded.proxy.password, "p@ss:word");
        assert!(loaded.proxy.explicit);
        let raw: String = {
            let db = state.db().unwrap();
            db.query_row(
                "SELECT value FROM settings WHERE key = 'proxy_password'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert!(!raw.contains("p@ss"));
        let kept = save(&state, &sample("")).unwrap();
        assert_eq!(kept["ldap"]["bind_password_set"], true);
        assert_eq!(load(&state).unwrap().ldap.bind_password, "bind-secret");
        let mut cleared = sample("");
        cleared.clear_ldap_password = true;
        cleared.clear_proxy_password = true;
        cleared.proxy_password.clear();
        let cleared = save(&state, &cleared).unwrap();
        assert_eq!(cleared["ldap"]["bind_password_set"], false);
        assert_eq!(cleared["proxy"]["password_set"], false);
        let reset = save(
            &state,
            &SettingsWrite {
                reset: true,
                ..sample("")
            },
        )
        .unwrap();
        assert_eq!(reset["source"], "env");
        assert_eq!(reset["ldap"]["url"], "");
        assert_eq!(reset["runner"], "local");
    }

    #[test]
    fn invalid_proxy_does_not_replace_a_saved_value() {
        let state = state();
        save(&state, &sample("bind-secret")).unwrap();
        let mut bad = sample("bind-secret");
        bad.proxy_url = "http://user:secret@proxy.example".into();
        let err = save(&state, &bad).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Zugangsdaten gehören in die eigenen Felder, nicht in die Proxy-Adresse."
        );
        assert_eq!(load(&state).unwrap().proxy.url, "http://proxy.example:3128");
    }
}
