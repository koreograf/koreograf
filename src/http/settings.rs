use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiJson, AppResult};
use crate::settings::{self, SettingsWrite};
use crate::Shared;

#[derive(Deserialize)]
pub(super) struct SettingsBody {
    #[serde(default)]
    reset: bool,
    ldap: Option<LdapBody>,
    proxy: Option<ProxyBody>,
    runner: Option<String>,
    job_timeout_secs: Option<u64>,
    #[serde(default)]
    job_output_lines: u64,
    #[serde(default)]
    clear: Vec<String>,
}

#[derive(Deserialize, Default)]
pub(super) struct LdapBody {
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
    #[serde(default)]
    ca_file: String,
}

#[derive(Deserialize, Default)]
pub(super) struct ProxyBody {
    #[serde(default)]
    url: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    no_proxy: String,
}

pub async fn get_settings(State(state): State<Shared>, user: AuthUser) -> AppResult<Json<Value>> {
    admin(&user)?;
    Ok(Json(settings::view(&state)?))
}

pub async fn put_settings(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<SettingsBody>,
) -> AppResult<Json<Value>> {
    admin(&user)?;
    if !body.reset {
        check_complete(&body)?;
    }
    let ldap = body.ldap.unwrap_or_default();
    let proxy = body.proxy.unwrap_or_default();
    let clear_ldap_password = body.clear.iter().any(|item| item == "ldap_bind_password");
    let clear_proxy_password = body.clear.iter().any(|item| item == "proxy_password");
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
        ldap_ca_file: ldap.ca_file,
        proxy_url: proxy.url,
        proxy_username: proxy.username,
        proxy_password: proxy.password,
        proxy_no_proxy: proxy.no_proxy,
        runner: body.runner.unwrap_or_default(),
        job_timeout_secs: body.job_timeout_secs.unwrap_or_default(),
        job_output_lines: body.job_output_lines,
        clear_ldap_password,
        clear_proxy_password,
        reset: body.reset,
    };
    Ok(Json(settings::save(&state, &write)?))
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
