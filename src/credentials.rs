//! Credential shapes and the files a job receives.
//!
//! Secrets stay in files with mode 0600 or in process environment variables.
//! They are not placed on the command line.

use std::collections::HashSet;
use std::path::Path;

use serde_json::{Map, Value};

pub const KINDS: &[&str] = &[
    "ssh",
    "vault",
    "scm",
    "net",
    "aws",
    "azure",
    "gce",
    "vmware",
    "openstack",
    "registry",
    "github",
    "gitlab",
    "galaxy",
    "kubernetes",
    "insights",
    "env",
    "netbox",
];

const BECOME_METHODS: &[&str] = &[
    "sudo", "su", "pbrun", "pfexec", "doas", "dzdo", "ksu", "runas",
];

pub struct PreparedCredential {
    pub env: Vec<(String, String)>,
    pub files: Vec<(String, String)>,
}

pub fn known_kind(kind: &str) -> bool {
    KINDS.contains(&kind)
}

/// One credential chosen for a template. `vault_id` is empty for every kind except vault.
#[derive(Clone, Copy)]
pub struct AssignedCredential<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub vault_id: &'a str,
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
    if url.is_empty() {
        Ok(())
    } else {
        one_line(url, "address")
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ssh_password_stays_out_of_environment_values() {
        let secret = json!({
            "username": "deploy",
            "password": "s3cret-pass",
            "become_method": "sudo",
            "become_password": "root-pass"
        });
        check("ssh", &secret).unwrap();
        let work = std::env::temp_dir();
        let prepared = prepare("ssh", &secret, false, &work).unwrap();
        let rendered = prepared
            .env
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!rendered.contains("s3cret-pass"));
        assert!(!rendered.contains("root-pass"));
        assert!(prepared
            .env
            .iter()
            .any(|(key, _)| key == "KOREOGRAF_CONNECTION_PASSWORD_FILE"));
        assert!(prepared
            .files
            .iter()
            .any(|(name, body)| name == "connection.pass" && body == "s3cret-pass"));
        assert!(prepared
            .files
            .iter()
            .any(|(name, body)| name == "auth.json" && body.contains("ansible_password")));
        assert!(check("ssh", &json!({})).is_err());
        assert!(check("ssh", &json!({"password": "only"})).is_ok());
    }

    #[test]
    fn edit_keeps_a_blank_secret_and_hides_the_value() {
        let previous = json!({
            "username": "deploy",
            "password": "old-pass",
            "private_key": "stored-key-material",
            "become_method": "sudo"
        });
        let merged = merge_secret(
            "ssh",
            &previous,
            &json!({"username": "nas", "password": "  ", "become_method": ""}),
            &["private_key".to_string(), "username".to_string()],
        )
        .unwrap();
        assert_eq!(merged["username"], "nas");
        assert_eq!(merged["password"], "old-pass");
        assert!(merged.get("private_key").is_none());
        assert!(merged.get("become_method").is_none());
        let replaced = merge_secret(
            "ssh",
            &previous,
            &json!({"password": "new-pass"}),
            &["password".to_string()],
        )
        .unwrap();
        assert_eq!(replaced["password"], "new-pass");
        assert_eq!(replaced["private_key"], previous["private_key"]);
        let (fields, set) = describe("ssh", &replaced);
        assert_eq!(fields["username"], "deploy");
        assert!(fields.get("password").is_none());
        assert_eq!(set, vec!["password".to_string(), "private_key".to_string()]);
        let rendered = format!("{fields} {}", set.join(","));
        assert!(!rendered.contains("new-pass"));
        assert!(!rendered.contains("stored-key-material"));
        let env = merge_secret(
            "env",
            &json!({"A": "1", "B": "2"}),
            &json!({"A": "3"}),
            &["B".to_string()],
        )
        .unwrap();
        assert_eq!(env["A"], "3");
        assert!(env.get("B").is_none());
        assert!(merge_secret("ssh", &previous, &json!([]), &[]).is_err());
        assert!(check(
            "aws",
            &json!({"access_key": "AKIA", "secret_key": "secret"})
        )
        .is_ok());
    }

    #[test]
    fn one_kind_except_many_vault_credentials() {
        let ssh = AssignedCredential {
            id: "ssh",
            kind: "ssh",
            vault_id: "",
        };
        let prod = AssignedCredential {
            id: "vault-prod",
            kind: "vault",
            vault_id: "prod",
        };
        let stage = AssignedCredential {
            id: "vault-stage",
            kind: "vault",
            vault_id: "stage",
        };
        assert!(check_assignment(&[ssh, prod, stage]).is_ok());
        let other = AssignedCredential {
            id: "ssh-2",
            kind: "ssh",
            vault_id: "",
        };
        assert_eq!(
            check_assignment(&[ssh, other]).unwrap_err(),
            "only_one_credential_of_each_kind_is_allowed"
        );
        let again = AssignedCredential {
            id: "vault-prod-2",
            kind: "vault",
            vault_id: "prod",
        };
        assert_eq!(
            check_assignment(&[prod, again]).unwrap_err(),
            "this_vault_id_is_already_selected"
        );
        let plain = AssignedCredential {
            id: "vault-plain",
            kind: "vault",
            vault_id: "",
        };
        let plain_two = AssignedCredential {
            id: "vault-plain-2",
            kind: "vault",
            vault_id: "",
        };
        assert_eq!(
            check_assignment(&[plain, plain_two]).unwrap_err(),
            "only_one_vault_credential_may_omit_the_vault"
        );
        assert!(check_assignment(&[ssh, plain, prod]).is_ok());
    }
}
