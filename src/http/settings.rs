use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ApiError, ApiJson, AppResult};
use crate::features::identity::AuthUser;
use crate::settings::{self, Effective, LlmWrite, SettingsWrite};
use crate::Shared;

pub(super) fn routes() -> Router<Shared> {
    Router::new().route("/settings", get(get_settings).put(put_settings))
}

#[derive(Serialize)]
pub(super) struct SettingsResponse {
    source: String,
    ldap: LdapSettingsResponse,
    proxy: ProxySettingsResponse,
    runner: String,
    job_timeout_secs: u64,
    job_output_lines: u64,
    default_environment_ansible: String,
    default_environment_opentofu: String,
    default_environment_shell: String,
    llm: LlmSettingsResponse,
}

#[derive(Serialize)]
pub(super) struct LdapSettingsResponse {
    url: String,
    mode: String,
    bind_dn: String,
    bind_password_set: bool,
    user_base: String,
    user_filter: String,
    username_attr: String,
    group_attr: String,
    group_base: String,
    starttls: bool,
    tls_insecure: bool,
}

#[derive(Serialize)]
pub(super) struct ProxySettingsResponse {
    url: String,
    username: String,
    password_set: bool,
    no_proxy: String,
    unsupported_env: bool,
}

#[derive(Serialize)]
pub(super) struct LlmSettingsResponse {
    enabled: bool,
    provider: String,
    base_url: String,
    model: String,
    api_key_set: bool,
}

impl From<&Effective> for SettingsResponse {
    fn from(effective: &Effective) -> Self {
        Self {
            source: if effective.saved { "saved" } else { "env" }.to_string(),
            ldap: LdapSettingsResponse {
                url: effective.ldap.url.clone(),
                mode: effective.ldap.mode.clone(),
                bind_dn: effective.ldap.bind_dn.clone(),
                bind_password_set: !effective.ldap.bind_password.is_empty(),
                user_base: effective.ldap.user_base.clone(),
                user_filter: effective.ldap.user_filter.clone(),
                username_attr: effective.ldap.username_attr.clone(),
                group_attr: effective.ldap.group_attr.clone(),
                group_base: effective.ldap.group_base.clone(),
                starttls: effective.ldap.starttls,
                tls_insecure: effective.ldap.tls_insecure,
            },
            proxy: ProxySettingsResponse {
                url: effective.proxy.url.clone(),
                username: effective.proxy.username.clone(),
                password_set: !effective.proxy.password.is_empty(),
                no_proxy: effective.proxy.no_proxy.clone(),
                unsupported_env: effective.unsupported_proxy,
            },
            runner: effective.runner.as_str().to_string(),
            job_timeout_secs: effective.job_timeout_secs,
            job_output_lines: effective.job_output_lines,
            default_environment_ansible: effective.default_environment_ansible.clone(),
            default_environment_opentofu: effective.default_environment_opentofu.clone(),
            default_environment_shell: effective.default_environment_shell.clone(),
            llm: LlmSettingsResponse {
                enabled: effective.llm_enabled,
                provider: effective.llm_provider.as_str().to_string(),
                base_url: effective.llm_url.clone(),
                model: effective.llm_model.clone(),
                api_key_set: !effective.llm_api_key.is_empty(),
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SettingsBody {
    #[serde(default, rename = "source")]
    _source: Option<Value>,
    #[serde(default)]
    reset: bool,
    ldap: Option<LdapBody>,
    proxy: Option<ProxyBody>,
    runner: Option<String>,
    job_timeout_secs: Option<u64>,
    #[serde(default)]
    job_output_lines: u64,
    #[serde(default)]
    default_environment_ansible: String,
    #[serde(default)]
    default_environment_opentofu: String,
    #[serde(default)]
    default_environment_shell: String,
    #[serde(default)]
    clear: Vec<String>,
    llm: Option<LlmBody>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct LlmBody {
    #[serde(default, rename = "api_key_set")]
    _api_key_set: Option<Value>,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    api_key: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct LdapBody {
    #[serde(default, rename = "bind_password_set")]
    _bind_password_set: Option<Value>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    bind_dn: String,
    #[serde(default)]
    bind_password: String,
    #[serde(default)]
    user_base: String,
    #[serde(default)]
    user_filter: String,
    #[serde(default)]
    username_attr: String,
    #[serde(default)]
    group_attr: String,
    #[serde(default)]
    group_base: String,
    #[serde(default)]
    starttls: bool,
    #[serde(default)]
    tls_insecure: bool,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct ProxyBody {
    #[serde(default, rename = "password_set")]
    _password_set: Option<Value>,
    #[serde(default, rename = "unsupported_env")]
    _unsupported_env: Option<Value>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    no_proxy: String,
}

pub async fn get_settings(
    State(state): State<Shared>,
    user: AuthUser,
) -> AppResult<Json<SettingsResponse>> {
    admin(&user)?;
    Ok(Json(SettingsResponse::from(&settings::view(&state)?)))
}

pub async fn put_settings(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<SettingsBody>,
) -> AppResult<Json<SettingsResponse>> {
    admin(&user)?;
    if !body.reset {
        check_complete(&body)?;
    }
    let ldap = body.ldap.unwrap_or_default();
    let proxy = body.proxy.unwrap_or_default();
    let clear_ldap_password = body.clear.iter().any(|item| item == "ldap_bind_password");
    let clear_proxy_password = body.clear.iter().any(|item| item == "proxy_password");
    let clear_api_key = body.clear.iter().any(|item| item == "llm_api_key");
    let llm = body.llm.map(|llm| LlmWrite {
        enabled: llm.enabled,
        provider: llm.provider,
        base_url: llm.base_url,
        model: llm.model,
        api_key: llm.api_key,
        clear_api_key,
    });
    let write = SettingsWrite {
        ldap_url: ldap.url,
        ldap_mode: ldap.mode,
        ldap_bind_dn: ldap.bind_dn,
        ldap_bind_password: ldap.bind_password,
        ldap_user_base: ldap.user_base,
        ldap_user_filter: ldap.user_filter,
        ldap_username_attr: ldap.username_attr,
        ldap_group_attr: ldap.group_attr,
        ldap_group_base: ldap.group_base,
        ldap_starttls: ldap.starttls,
        ldap_tls_insecure: ldap.tls_insecure,
        proxy_url: proxy.url,
        proxy_username: proxy.username,
        proxy_password: proxy.password,
        proxy_no_proxy: proxy.no_proxy,
        runner: body.runner.unwrap_or_default(),
        job_timeout_secs: body.job_timeout_secs.unwrap_or_default(),
        job_output_lines: body.job_output_lines,
        default_environment_ansible: body.default_environment_ansible,
        default_environment_opentofu: body.default_environment_opentofu,
        default_environment_shell: body.default_environment_shell,
        clear_ldap_password,
        clear_proxy_password,
        llm,
        reset: body.reset,
    };
    Ok(Json(SettingsResponse::from(&settings::save(
        &state, &write,
    )?)))
}

fn check_complete(body: &SettingsBody) -> AppResult<()> {
    let mut missing = Vec::new();
    if body.ldap.is_none() {
        missing.push("ldap");
    }
    if body.proxy.is_none() {
        missing.push("proxy");
    }
    if body.runner.as_deref().unwrap_or("").trim().is_empty() {
        missing.push("runner");
    }
    if body.job_timeout_secs.is_none() {
        missing.push("job_timeout_secs");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ApiError::bad_vars(
            "the_settings_body_is_incomplete",
            [("fields", missing.join(", "))],
        ))
    }
}

fn admin(user: &AuthUser) -> AppResult<()> {
    if user.is_admin {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}
