use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value};

use crate::error::{ApiError, AppResult};

use super::model::{AssignedCredential, AssignedSet, PreparedCredential, KINDS};

const BECOME_METHODS: &[&str] = &[
    "sudo", "su", "pbrun", "pfexec", "doas", "dzdo", "ksu", "runas",
];

pub fn known_kind(kind: &str) -> bool {
    KINDS.contains(&kind)
}

pub fn kinds_for_tool(tool: &str) -> &'static [&'static str] {
    match tool {
        "ansible" => &["ssh", "vault"],
        "opentofu" => &[
            "aws",
            "azure",
            "gce",
            "vmware",
            "openstack",
            "kubernetes",
            "env",
        ],
        "shell" => &["ssh", "env"],
        _ => &[],
    }
}

pub fn kind_fits_tool(tool: &str, kind: &str) -> bool {
    kinds_for_tool(tool).contains(&kind)
}

/// One credential of each kind. Vault may repeat when each vault id is unique.
pub fn check_assignment(items: &[AssignedCredential<'_>]) -> Result<(), String> {
    if items.len() > 32 {
        return Err("too_many_credentials".into());
    }
    let mut seen_ids = HashSet::new();
    let mut seen_kinds = HashSet::new();
    let mut seen_vault_ids = HashSet::new();
    let mut default_vaults = 0usize;
    for item in items {
        if item.id.is_empty() {
            return Err("credential_is_missing".into());
        }
        if !seen_ids.insert(item.id) {
            return Err("credential_is_listed_twice".into());
        }
        if item.kind == "vault" {
            if item.vault_id.is_empty() {
                default_vaults += 1;
                if default_vaults > 1 {
                    return Err("only_one_vault_credential_may_omit_the_vault".into());
                }
            } else if !seen_vault_ids.insert(item.vault_id) {
                return Err("this_vault_id_is_already_selected".into());
            }
        } else if !seen_kinds.insert(item.kind) {
            return Err("only_one_credential_of_each_kind_is_allowed".into());
        }
    }
    Ok(())
}

/// Team, vault id, and kind rules for credentials that are already loaded.
pub fn assign_loaded(
    state: &crate::Shared,
    team: &str,
    loaded: &[crate::db::Credential],
) -> AppResult<AssignedSet> {
    let mut ids = Vec::with_capacity(loaded.len());
    let mut vault_labels = Vec::with_capacity(loaded.len());
    for credential in loaded {
        if credential.team_id != team {
            return Err(ApiError::NotFound);
        }
        let vault_id = if credential.kind == "vault" {
            let plain = state
                .decrypt(&credential.secret_blob)
                .map_err(|_| ApiError::bad("credentials_cannot_be_read"))?;
            let secret: Value = serde_json::from_str(&plain)
                .map_err(|_| ApiError::bad("credentials_are_not_json"))?;
            check("vault", &secret).map_err(ApiError::bad)?;
            secret
                .get("vault_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string()
        } else {
            String::new()
        };
        vault_labels.push(vault_id);
        ids.push(credential.id.clone());
    }
    let view = ids
        .iter()
        .zip(loaded.iter().zip(vault_labels.iter()))
        .map(|(id, (row, vault_id))| AssignedCredential {
            id,
            kind: &row.kind,
            vault_id,
        })
        .collect::<Vec<_>>();
    check_assignment(&view).map_err(ApiError::bad)?;
    let primary_id = loaded
        .iter()
        .find(|row| row.kind != "vault")
        .or_else(|| loaded.first())
        .map(|row| row.id.clone());
    Ok(AssignedSet { ids, primary_id })
}

/// Values the edit form must not display again. Every `env` key is secret.
pub fn is_secret_field(kind: &str, key: &str) -> bool {
    match kind {
        "ssh" => matches!(
            key,
            "password" | "private_key" | "passphrase" | "become_password"
        ),
        "scm" => matches!(key, "password" | "private_key" | "passphrase"),
        "vault" => key == "password",
        "net" => matches!(key, "password" | "private_key" | "authorize_password"),
        "aws" => matches!(key, "secret_key" | "security_token"),
        "azure" => key == "secret",
        "gce" => key == "service_account",
        "vmware" | "openstack" | "registry" | "insights" => key == "password",
        "github" | "gitlab" | "galaxy" | "kubernetes" | "netbox" => key == "token",
        "env" => true,
        _ => true,
    }
}

/// Public fields and the names of stored secrets. Secret values stay out.
pub fn describe(kind: &str, secret: &Value) -> (Value, Vec<String>) {
    let mut fields = Map::new();
    let mut set = Vec::new();
    let Some(object) = secret.as_object() else {
        return (Value::Object(fields), set);
    };
    let mut keys: Vec<&String> = object.keys().collect();
    keys.sort();
    for key in keys {
        let value = &object[key];
        if is_secret_field(kind, key) {
            let present = match value {
                Value::String(text) => !text.trim().is_empty(),
                Value::Bool(flag) => *flag,
                Value::Null => false,
                _ => true,
            };
            if present {
                set.push(key.clone());
            }
            continue;
        }
        if let Some(text) = value.as_str() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                fields.insert(key.clone(), Value::String(trimmed.to_string()));
            }
        } else if let Some(flag) = value.as_bool() {
            fields.insert(key.clone(), Value::Bool(flag));
        }
    }
    (Value::Object(fields), set)
}

/// Overlay an edit onto the stored secret.
///
/// An empty secret field keeps the stored value, because the form cannot show it.
/// `clear` removes secret fields. A non-empty secret value wins over `clear`.
/// Public fields, including an empty string, replace what was stored.
pub fn merge_secret(
    kind: &str,
    previous: &Value,
    incoming: &Value,
    clear: &[String],
) -> Result<Value, String> {
    if !incoming.is_null() && !incoming.is_object() {
        return Err("the_secret_must_be_a_json_object".into());
    }
    let mut base = previous.as_object().cloned().unwrap_or_default();
    for key in clear {
        if is_secret_field(kind, key) {
            base.remove(key);
        }
    }
    let Some(object) = incoming.as_object() else {
        return Ok(Value::Object(base));
    };
    for (key, value) in object {
        if is_secret_field(kind, key) {
            if let Some(text) = value.as_str() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    base.insert(key.clone(), Value::String(trimmed.to_string()));
                }
            }
            continue;
        }
        match value {
            Value::String(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    base.remove(key);
                } else {
                    base.insert(key.clone(), Value::String(trimmed.to_string()));
                }
            }
            Value::Bool(flag) => {
                base.insert(key.clone(), Value::Bool(*flag));
            }
            Value::Null => {
                base.remove(key);
            }
            _ => {}
        }
    }
    Ok(Value::Object(base))
}

pub fn check(kind: &str, secret: &Value) -> Result<(), String> {
    if !known_kind(kind) {
        return Err("unknown_credential_type".into());
    }
    if !secret.is_object() {
        return Err("the_secret_must_be_a_json_object".into());
    }
    match kind {
        "ssh" => check_ssh(secret),
        "vault" => check_vault(secret),
        "scm" => check_scm(secret),
        "net" => check_net(secret),
        "aws" => check_aws(secret),
        "azure" => check_azure(secret),
        "gce" => check_gce(secret),
        "vmware" => check_vmware(secret),
        "openstack" => check_openstack(secret),
        "registry" => check_registry(secret),
        "github" | "gitlab" => check_token(secret),
        "galaxy" => check_galaxy(secret),
        "kubernetes" => check_kubernetes(secret),
        "insights" => check_login(secret),
        "env" => Ok(()),
        "netbox" => check_netbox(secret),
        _ => Err("unknown_credential_type".into()),
    }
}

pub fn prepare(
    kind: &str,
    secret: &Value,
    container: bool,
    work: &Path,
) -> Result<PreparedCredential, String> {
    check(kind, secret)?;
    let mut env = Vec::new();
    let mut files = Vec::new();
    match kind {
        "ssh" => ssh_material(secret, container, work, &mut env, &mut files),
        "net" => {
            ssh_material(secret, container, work, &mut env, &mut files);
            push_env(&mut env, "ANSIBLE_NET_USERNAME", text(secret, "username"));
            push_env(&mut env, "ANSIBLE_NET_PASSWORD", text(secret, "password"));
            if flag(secret, "authorize") {
                env.push(("ANSIBLE_NET_AUTHORIZE".into(), "true".into()));
            }
            push_env(
                &mut env,
                "ANSIBLE_NET_AUTH_PASS",
                text(secret, "authorize_password"),
            );
        }
        "vault" => {
            let password = text(secret, "password");
            files.push(("vault.pass".into(), password.to_string()));
            env.push((
                "KOREOGRAF_VAULT_FILE".into(),
                guest(container, work, "vault.pass"),
            ));
            let vault_id = text(secret, "vault_id");
            if !vault_id.is_empty() {
                env.push(("KOREOGRAF_VAULT_ID".into(), vault_id.to_string()));
            }
        }
        "scm" => scm_material(secret, container, work, &mut env, &mut files),
        "aws" => {
            push_env(&mut env, "AWS_ACCESS_KEY_ID", text(secret, "access_key"));
            push_env(
                &mut env,
                "AWS_SECRET_ACCESS_KEY",
                text(secret, "secret_key"),
            );
            push_env(
                &mut env,
                "AWS_SESSION_TOKEN",
                text(secret, "security_token"),
            );
        }
        "azure" => {
            push_env(
                &mut env,
                "AZURE_SUBSCRIPTION_ID",
                text(secret, "subscription_id"),
            );
            push_env(&mut env, "AZURE_CLIENT_ID", text(secret, "client_id"));
            let secret_value = text(secret, "secret");
            push_env(&mut env, "AZURE_SECRET", secret_value);
            push_env(&mut env, "AZURE_CLIENT_SECRET", secret_value);
            push_env(&mut env, "AZURE_TENANT", text(secret, "tenant"));
        }
        "gce" => {
            let body = text(secret, "service_account");
            files.push(("gce.json".into(), body.to_string()));
            let path = guest(container, work, "gce.json");
            env.push(("GOOGLE_APPLICATION_CREDENTIALS".into(), path.clone()));
            env.push(("GCE_CREDENTIALS_FILE_PATH".into(), path));
        }
        "vmware" => {
            push_env(&mut env, "VMWARE_HOST", text(secret, "host"));
            push_env(&mut env, "VMWARE_USER", text(secret, "username"));
            push_env(&mut env, "VMWARE_PASSWORD", text(secret, "password"));
        }
        "openstack" => {
            push_env(&mut env, "OS_AUTH_URL", text(secret, "auth_url"));
            push_env(&mut env, "OS_USERNAME", text(secret, "username"));
            push_env(&mut env, "OS_PASSWORD", text(secret, "password"));
            push_env(&mut env, "OS_PROJECT_NAME", text(secret, "project"));
            let domain = text(secret, "domain");
            let domain = if domain.is_empty() { "Default" } else { domain };
            env.push(("OS_USER_DOMAIN_NAME".into(), domain.to_string()));
            env.push(("OS_IDENTITY_API_VERSION".into(), "3".into()));
        }
        "registry" => {
            push_env(&mut env, "KOREOGRAF_REGISTRY_HOST", text(secret, "host"));
            push_env(
                &mut env,
                "KOREOGRAF_REGISTRY_USERNAME",
                text(secret, "username"),
            );
            push_env(
                &mut env,
                "KOREOGRAF_REGISTRY_PASSWORD",
                text(secret, "password"),
            );
        }
        "github" => {
            let token = text(secret, "token");
            push_env(&mut env, "GH_TOKEN", token);
            push_env(&mut env, "GITHUB_TOKEN", token);
        }
        "gitlab" => {
            let token = text(secret, "token");
            push_env(&mut env, "GITLAB_TOKEN", token);
            push_env(&mut env, "GITLAB_PRIVATE_TOKEN", token);
        }
        "galaxy" => {
            let url = text(secret, "url");
            let url = if url.is_empty() {
                "https://galaxy.ansible.com"
            } else {
                url
            };
            env.push(("ANSIBLE_GALAXY_SERVER_LIST".into(), "published".into()));
            env.push(("ANSIBLE_GALAXY_SERVER_PUBLISHED_URL".into(), url.into()));
            push_env(
                &mut env,
                "ANSIBLE_GALAXY_SERVER_PUBLISHED_TOKEN",
                text(secret, "token"),
            );
        }
        "kubernetes" => {
            push_env(&mut env, "K8S_AUTH_HOST", text(secret, "host"));
            push_env(&mut env, "K8S_AUTH_API_KEY", text(secret, "token"));
            let verify = if flag_default(secret, "verify_ssl", true) {
                "true"
            } else {
                "false"
            };
            env.push(("K8S_AUTH_VERIFY_SSL".into(), verify.into()));
        }
        "insights" => {
            push_env(&mut env, "INSIGHTS_USER", text(secret, "username"));
            push_env(&mut env, "INSIGHTS_PASSWORD", text(secret, "password"));
        }
        "env" => env_material(secret, &mut env)?,
        "netbox" => {
            push_env(&mut env, "NETBOX_TOKEN", text(secret, "token"));
            push_env(&mut env, "NETBOX_API", text(secret, "url"));
            push_env(&mut env, "NETBOX_URL", text(secret, "url"));
        }
        _ => return Err("unknown_credential_type".into()),
    }
    Ok(PreparedCredential { env, files })
}

fn check_ssh(secret: &Value) -> Result<(), String> {
    let key = text(secret, "private_key");
    let password = text(secret, "password");
    if key.is_empty() && password.is_empty() {
        return Err("ssh_needs_a_key_or_a_password".into());
    }
    one_line(text(secret, "username"), "user")?;
    one_line(password, "password")?;
    limited(key, 65_536, "private_key")?;
    one_line(text(secret, "passphrase"), "passphrase")?;
    if !text(secret, "passphrase").is_empty() && key.is_empty() {
        return Err("a_passphrase_needs_a_private_key".into());
    }
    let method = text(secret, "become_method");
    if !method.is_empty() && !BECOME_METHODS.contains(&method) {
        return Err("the_become_method_is_invalid".into());
    }
    one_line(text(secret, "become_username"), "become_user")?;
    one_line(text(secret, "become_password"), "become_password")?;
    Ok(())
}

fn check_vault(secret: &Value) -> Result<(), String> {
    let password = text(secret, "password");
    require(password, "vault_needs_a_password")?;
    one_line(password, "password")?;
    let vault_id = text(secret, "vault_id");
    if vault_id.is_empty() {
        return Ok(());
    }
    let ok = vault_id.len() <= 64
        && vault_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'));
    if ok {
        Ok(())
    } else {
        Err("the_vault_id_is_invalid".into())
    }
}

fn check_scm(secret: &Value) -> Result<(), String> {
    let key = text(secret, "private_key");
    let password = text(secret, "password");
    if key.is_empty() && password.is_empty() {
        return Err("source_control_needs_a_key_or_a_password".into());
    }
    one_line(text(secret, "username"), "user")?;
    one_line(password, "password")?;
    limited(key, 65_536, "private_key")?;
    one_line(text(secret, "passphrase"), "passphrase")?;
    if !text(secret, "passphrase").is_empty() && key.is_empty() {
        return Err("a_passphrase_needs_a_private_key".into());
    }
    Ok(())
}

fn check_net(secret: &Value) -> Result<(), String> {
    if text(secret, "username").is_empty() && text(secret, "password").is_empty() {
        return Err("network_needs_a_username_or_a_password".into());
    }
    one_line(text(secret, "username"), "user")?;
    one_line(text(secret, "password"), "password")?;
    limited(text(secret, "private_key"), 65_536, "private_key")?;
    one_line(text(secret, "authorize_password"), "authorize_password")?;
    Ok(())
}

fn check_aws(secret: &Value) -> Result<(), String> {
    require(text(secret, "access_key"), "aws_needs_access_key")?;
    require(text(secret, "secret_key"), "aws_needs_secret_key")?;
    one_line(text(secret, "access_key"), "access_key")?;
    one_line(text(secret, "secret_key"), "secret_key")?;
    one_line(text(secret, "security_token"), "session_token")?;
    Ok(())
}

fn check_azure(secret: &Value) -> Result<(), String> {
    require(
        text(secret, "subscription_id"),
        "azure_needs_subscription_id",
    )?;
    require(text(secret, "client_id"), "azure_needs_client_id")?;
    require(text(secret, "secret"), "azure_needs_secret")?;
    require(text(secret, "tenant"), "azure_needs_tenant")?;
    for (key, label) in [
        ("subscription_id", "subscription"),
        ("client_id", "client"),
        ("secret", "secret"),
        ("tenant", "tenant"),
    ] {
        one_line(text(secret, key), label)?;
    }
    Ok(())
}

fn check_gce(secret: &Value) -> Result<(), String> {
    let body = text(secret, "service_account");
    require(body, "the_service_account_is_missing")?;
    limited(body, 65_536, "service_account")?;
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Object(_)) => Ok(()),
        _ => Err("the_service_account_is_not_a_json_object".into()),
    }
}

fn check_vmware(secret: &Value) -> Result<(), String> {
    require(text(secret, "host"), "vmware_needs_a_host")?;
    check_login(secret)?;
    one_line(text(secret, "host"), "host")?;
    Ok(())
}

fn check_openstack(secret: &Value) -> Result<(), String> {
    require(text(secret, "auth_url"), "openstack_needs_auth_url")?;
    https_url(text(secret, "auth_url"))?;
    require(text(secret, "project"), "openstack_needs_project")?;
    check_login(secret)?;
    one_line(text(secret, "project"), "project")?;
    one_line(text(secret, "domain"), "domain")?;
    Ok(())
}

fn check_registry(secret: &Value) -> Result<(), String> {
    require(text(secret, "host"), "the_registry_needs_a_host")?;
    check_login(secret)?;
    one_line(text(secret, "host"), "host")?;
    Ok(())
}

fn check_kubernetes(secret: &Value) -> Result<(), String> {
    require(text(secret, "host"), "kubernetes_needs_a_host")?;
    https_url(text(secret, "host"))?;
    require(text(secret, "token"), "token_is_missing")?;
    one_line(text(secret, "token"), "token")?;
    Ok(())
}

fn check_token(secret: &Value) -> Result<(), String> {
    let token = text(secret, "token");
    require(token, "token_is_missing")?;
    one_line(token, "token")
}

fn check_galaxy(secret: &Value) -> Result<(), String> {
    check_token(secret)?;
    let url = text(secret, "url");
    if url.is_empty() {
        Ok(())
    } else {
        one_line(url, "address")
    }
}

fn check_netbox(secret: &Value) -> Result<(), String> {
    let token = text(secret, "token");
    require(token, "netbox_needs_a_token")?;
    one_line(token, "token")?;
    let url = text(secret, "url");
    require(url, "netbox_needs_an_address")?;
    if !crate::features::inventories::valid_endpoint(url) {
        return Err("the_netbox_address_must_be_http_or_https".into());
    }
    Ok(())
}

pub(crate) fn backfill_netbox_urls(state: &crate::Shared) -> Result<(), crate::error::ApiError> {
    let (credentials, sources) = {
        let db = state.db()?;
        (
            super::store::netbox_credentials(&db)?,
            super::store::netbox_credential_endpoints(&db)?,
        )
    };
    let mut endpoints = HashMap::new();
    for (credential_id, source_json) in sources {
        let endpoint = serde_json::from_str::<Value>(&source_json)
            .ok()
            .and_then(|value| {
                value
                    .get("api_endpoint")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            });
        if let Some(endpoint) = endpoint {
            endpoints.entry(credential_id).or_insert(endpoint);
        }
    }
    let mut updates = Vec::new();
    for credential in credentials {
        let Ok(plain) = state.decrypt(&credential.secret_blob) else {
            continue;
        };
        let Ok(mut secret) = serde_json::from_str::<Value>(&plain) else {
            continue;
        };
        let has_url = secret
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|url| !url.trim().is_empty());
        if has_url {
            continue;
        }
        if let Some(endpoint) = endpoints.get(&credential.id) {
            let Some(object) = secret.as_object_mut() else {
                continue;
            };
            object.insert("url".into(), Value::String(endpoint.clone()));
            if let Ok(blob) = state.encrypt(&secret.to_string()) {
                updates.push((credential.id, Some(blob), false));
            }
        } else {
            updates.push((credential.id, None, true));
        }
    }
    if updates.is_empty() {
        return Ok(());
    }
    let db = state.db()?;
    for (id, blob, disabled) in updates {
        if let Some(blob) = blob {
            db.execute(
                "UPDATE credentials SET secret_blob = ?2, disabled = ?3 WHERE id = ?1",
                rusqlite::params![id, blob, disabled],
            )?;
        } else {
            db.execute(
                "UPDATE credentials SET disabled = ?2 WHERE id = ?1",
                rusqlite::params![id, disabled],
            )?;
        }
    }
    Ok(())
}

fn check_login(secret: &Value) -> Result<(), String> {
    require(text(secret, "username"), "username_is_missing")?;
    require(text(secret, "password"), "password_is_missing")?;
    one_line(text(secret, "username"), "user")?;
    one_line(text(secret, "password"), "password")?;
    Ok(())
}

fn ssh_material(
    secret: &Value,
    container: bool,
    work: &Path,
    env: &mut Vec<(String, String)>,
    files: &mut Vec<(String, String)>,
) {
    let username = text(secret, "username");
    let password = text(secret, "password");
    let key = text(secret, "private_key");
    let passphrase = text(secret, "passphrase");
    let become_method = text(secret, "become_method");
    let become_username = text(secret, "become_username");
    let become_password = text(secret, "become_password");
    if !key.is_empty() {
        files.push(("ssh_key".into(), key.to_string()));
        env.push((
            "KOREOGRAF_PRIVATE_KEY".into(),
            guest(container, work, "ssh_key"),
        ));
    }
    if !username.is_empty() {
        env.push(("KOREOGRAF_REMOTE_USER".into(), username.to_string()));
    }
    if !password.is_empty() {
        files.push(("connection.pass".into(), password.to_string()));
        env.push((
            "KOREOGRAF_CONNECTION_PASSWORD_FILE".into(),
            guest(container, work, "connection.pass"),
        ));
    }
    if !passphrase.is_empty() {
        files.push(("key.pass".into(), passphrase.to_string()));
        env.push((
            "KOREOGRAF_KEY_PASSPHRASE_FILE".into(),
            guest(container, work, "key.pass"),
        ));
    }
    if !become_method.is_empty() {
        env.push(("KOREOGRAF_BECOME_METHOD".into(), become_method.to_string()));
    }
    if !become_username.is_empty() {
        env.push(("KOREOGRAF_BECOME_USER".into(), become_username.to_string()));
    }
    if !become_password.is_empty() {
        files.push(("become.pass".into(), become_password.to_string()));
        env.push((
            "KOREOGRAF_BECOME_PASSWORD_FILE".into(),
            guest(container, work, "become.pass"),
        ));
    }
    let mut auth = Map::new();
    if !username.is_empty() {
        auth.insert("ansible_user".into(), Value::String(username.into()));
    }
    if !password.is_empty() {
        auth.insert("ansible_password".into(), Value::String(password.into()));
    }
    if !become_method.is_empty() {
        auth.insert("ansible_become".into(), Value::Bool(true));
        auth.insert(
            "ansible_become_method".into(),
            Value::String(become_method.into()),
        );
    }
    if !become_username.is_empty() {
        auth.insert(
            "ansible_become_user".into(),
            Value::String(become_username.into()),
        );
    }
    if !become_password.is_empty() {
        auth.insert(
            "ansible_become_password".into(),
            Value::String(become_password.into()),
        );
    }
    if !auth.is_empty() {
        files.push(("auth.json".into(), Value::Object(auth).to_string()));
        env.push((
            "KOREOGRAF_AUTH_EXTRA".into(),
            guest(container, work, "auth.json"),
        ));
    }
}

fn scm_material(
    secret: &Value,
    container: bool,
    work: &Path,
    env: &mut Vec<(String, String)>,
    files: &mut Vec<(String, String)>,
) {
    let username = text(secret, "username");
    if !username.is_empty() {
        env.push(("KOREOGRAF_GIT_USERNAME".into(), username.to_string()));
    }
    let password = text(secret, "password");
    if !password.is_empty() {
        files.push(("git.pass".into(), password.to_string()));
        env.push((
            "KOREOGRAF_GIT_PASSWORD_FILE".into(),
            guest(container, work, "git.pass"),
        ));
    }
    let key = text(secret, "private_key");
    if !key.is_empty() {
        files.push(("git_key".into(), key.to_string()));
        env.push((
            "KOREOGRAF_GIT_SSH_KEY".into(),
            guest(container, work, "git_key"),
        ));
    }
    let passphrase = text(secret, "passphrase");
    if !passphrase.is_empty() {
        files.push(("key.pass".into(), passphrase.to_string()));
        env.push((
            "KOREOGRAF_KEY_PASSPHRASE_FILE".into(),
            guest(container, work, "key.pass"),
        ));
    }
}

pub(crate) fn reserved_env_name(name: &str) -> bool {
    const EXACT: &[&str] = &["PATH", "HOME", "ENV", "BASH_ENV", "SHELL", "IFS"];
    if EXACT.iter().any(|item| name.eq_ignore_ascii_case(item)) {
        return true;
    }
    const PREFIXES: &[&str] = &[
        "KOREOGRAF_",
        "ANSIBLE_",
        "LD_",
        "PYTHON",
        "GIT_",
        "TF_",
        "BASH_",
    ];
    let upper = name.to_ascii_uppercase();
    PREFIXES.iter().any(|prefix| upper.starts_with(prefix))
}

fn env_material(secret: &Value, env: &mut Vec<(String, String)>) -> Result<(), String> {
    let Some(map) = secret.as_object() else {
        return Err("the_env_credential_must_be_a_json_object".into());
    };
    for (key, value) in map {
        if !safe_ident(key) || reserved_env_name(key) {
            continue;
        }
        let Some(text) = value.as_str() else { continue };
        if text.contains('\n') || text.contains('\0') || text.contains('\r') {
            return Err(crate::error::message("env.var_newline", &[("key", key)]));
        }
        env.push((key.clone(), text.to_string()));
    }
    Ok(())
}

fn push_env(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if !value.is_empty() {
        env.push((key.to_string(), value.to_string()));
    }
}

fn guest(container: bool, work: &Path, name: &str) -> String {
    if container {
        format!("/runner/work/{name}")
    } else {
        work.join(name).display().to_string()
    }
}

fn text<'a>(secret: &'a Value, key: &str) -> &'a str {
    secret
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
}

fn flag(secret: &Value, key: &str) -> bool {
    match secret.get(key) {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => matches!(value.trim(), "1" | "true" | "yes" | "ja"),
        _ => false,
    }
}

fn flag_default(secret: &Value, key: &str, default: bool) -> bool {
    if secret.get(key).is_none() {
        default
    } else {
        flag(secret, key)
    }
}

fn require(value: &str, message: &str) -> Result<(), String> {
    if value.is_empty() {
        Err(message.to_string())
    } else {
        Ok(())
    }
}

fn one_line(value: &str, label: &str) -> Result<(), String> {
    limited(value, 1024, label)?;
    if value.contains('\n') || value.contains('\r') {
        return Err(crate::error::message("field.newline", &[("field", label)]));
    }
    Ok(())
}

fn limited(value: &str, max: usize, label: &str) -> Result<(), String> {
    if value.contains('\0') {
        return Err(crate::error::message("field.invalid", &[("field", label)]));
    }
    if value.len() > max {
        return Err(crate::error::message("field.too_long", &[("field", label)]));
    }
    Ok(())
}

fn https_url(value: &str) -> Result<(), String> {
    one_line(value, "address")?;
    if value.starts_with("https://") && !value.contains(' ') {
        Ok(())
    } else {
        Err("the_address_must_start_with_https".into())
    }
}

fn safe_ident(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskpassKind {
    Git,
    Ssh,
}

pub fn askpass_kind<I, S>(args: I) -> Option<AskpassKind>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let first = args.into_iter().next()?;
    let name = Path::new(first.as_ref())
        .file_name()
        .and_then(|value| value.to_str())?;
    match name {
        "git-askpass" => Some(AskpassKind::Git),
        "ssh-askpass" => Some(AskpassKind::Ssh),
        _ => None,
    }
}

pub fn git_ssh_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .next()
        .and_then(|arg| {
            Path::new(arg.as_ref())
                .file_name()
                .and_then(|value| value.to_str())
                .map(|name| name == "git-ssh")
        })
        .unwrap_or(false)
}

pub fn git_ssh<I, S>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let key = std::env::var("KOREOGRAF_GIT_SSH_KEY")
        .map_err(|_| "KOREOGRAF_GIT_SSH_KEY is not set for the git ssh helper.".to_string())?;
    let mut command = std::process::Command::new("/usr/bin/ssh");
    command
        .arg("-i")
        .arg(key)
        .arg("-o")
        .arg("IdentitiesOnly=yes")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = command.exec();
        Err(format!("The git ssh helper could not start ssh ({err})."))
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .map_err(|err| format!("The git ssh helper could not start ssh ({err})."))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("The git ssh helper exited with {status}."))
        }
    }
}

pub fn askpass(kind: AskpassKind, prompt: &str) -> Result<String, String> {
    askpass_answer(kind, prompt, |name| std::env::var(name).ok())
}

fn askpass_answer(
    kind: AskpassKind,
    prompt: &str,
    env: impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    match kind {
        AskpassKind::Git => {
            if prompt.to_ascii_lowercase().contains("sername") {
                return Ok(env("KOREOGRAF_GIT_USERNAME").unwrap_or_else(|| "oauth2".into()));
            }
            read_secret(env, "KOREOGRAF_GIT_PASSWORD_FILE")
        }
        AskpassKind::Ssh => read_secret(env, "KOREOGRAF_SSH_ASKPASS_FILE"),
    }
}

fn read_secret(env: impl Fn(&str) -> Option<String>, name: &str) -> Result<String, String> {
    let path = env(name).ok_or_else(|| format!("{name} is not set for the credential helper."))?;
    let value = std::fs::read_to_string(&path)
        .map_err(|err| format!("The credential helper could not read its secret ({err})."))?;
    Ok(value.trim_end_matches(['\r', '\n']).to_string())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
