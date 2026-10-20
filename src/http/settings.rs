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
    #[serde(default)]
    ldap: LdapBody,
    #[serde(default)]
    proxy: ProxyBody,
    #[serde(default)]
    runner: String,
    #[serde(default)]
    job_timeout_secs: u64,
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
    let clear_ldap_password = body.clear.iter().any(|item| item == "ldap_bind_password");
    let clear_proxy_password = body.clear.iter().any(|item| item == "proxy_password");
    let write = SettingsWrite {
        ldap_url: body.ldap.url,
        ldap_mode: body.ldap.mode,
        ldap_bind_dn: body.ldap.bind_dn,
        ldap_bind_password: body.ldap.bind_password,
        ldap_user_base: body.ldap.user_base,
        ldap_user_filter: body.ldap.user_filter,
        ldap_username_attr: body.ldap.username_attr,
        ldap_group_attr: body.ldap.group_attr,
        ldap_group_base: body.ldap.group_base,
        ldap_starttls: body.ldap.starttls,
        ldap_tls_insecure: body.ldap.tls_insecure,
        proxy_url: body.proxy.url,
        proxy_username: body.proxy.username,
        proxy_password: body.proxy.password,
        proxy_no_proxy: body.proxy.no_proxy,
        runner: body.runner,
        job_timeout_secs: body.job_timeout_secs,
        clear_ldap_password,
        clear_proxy_password,
        reset: body.reset,
    };
    Ok(Json(settings::save(&state, &write)?))
}

fn admin(user: &AuthUser) -> AppResult<()> {
    if user.is_admin {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}
