//! Credential shapes and the files a job receives.
//!
//! Secrets stay in files with mode 0600 or in process environment variables.
//! They are not placed on the command line.

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

pub fn check(kind: &str, secret: &Value) -> Result<(), String> {
    if !known_kind(kind) {
        return Err("Unbekannte Zugangsart.".into());
    }
    if !secret.is_object() {
        return Err("Geheimnis muss ein JSON-Objekt sein.".into());
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
        _ => Err("Unbekannte Zugangsart.".into()),
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
        _ => return Err("Unbekannte Zugangsart.".into()),
    }
    Ok(PreparedCredential { env, files })
}

fn check_ssh(secret: &Value) -> Result<(), String> {
    let key = text(secret, "private_key");
    let password = text(secret, "password");
    if key.is_empty() && password.is_empty() {
        return Err("SSH braucht einen Schlüssel oder ein Passwort.".into());
    }
    one_line(text(secret, "username"), "Benutzer")?;
    one_line(password, "Passwort")?;
    limited(key, 65_536, "Privater Schlüssel")?;
    one_line(text(secret, "passphrase"), "Passphrase")?;
    if !text(secret, "passphrase").is_empty() && key.is_empty() {
        return Err("Passphrase braucht einen privaten Schlüssel.".into());
    }
    let method = text(secret, "become_method");
    if !method.is_empty() && !BECOME_METHODS.contains(&method) {
        return Err("Become-Methode ist ungültig.".into());
    }
    one_line(text(secret, "become_username"), "Become-Benutzer")?;
    one_line(text(secret, "become_password"), "Become-Passwort")?;
    Ok(())
}

fn check_vault(secret: &Value) -> Result<(), String> {
    let password = text(secret, "password");
    require(password, "Vault braucht password.")?;
    one_line(password, "Passwort")?;
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
        Err("Vault-Id ist ungültig.".into())
    }
}

fn check_scm(secret: &Value) -> Result<(), String> {
    let key = text(secret, "private_key");
    let password = text(secret, "password");
    if key.is_empty() && password.is_empty() {
        return Err("Quellcode braucht einen Schlüssel oder ein Passwort.".into());
    }
    one_line(text(secret, "username"), "Benutzer")?;
    one_line(password, "Passwort")?;
    limited(key, 65_536, "Privater Schlüssel")?;
    one_line(text(secret, "passphrase"), "Passphrase")?;
    if !text(secret, "passphrase").is_empty() && key.is_empty() {
        return Err("Passphrase braucht einen privaten Schlüssel.".into());
    }
    Ok(())
}

fn check_net(secret: &Value) -> Result<(), String> {
    if text(secret, "username").is_empty() && text(secret, "password").is_empty() {
        return Err("Netzwerk braucht einen Benutzer oder ein Passwort.".into());
    }
    one_line(text(secret, "username"), "Benutzer")?;
    one_line(text(secret, "password"), "Passwort")?;
    limited(text(secret, "private_key"), 65_536, "Privater Schlüssel")?;
    one_line(text(secret, "authorize_password"), "Authorize-Passwort")?;
    Ok(())
}

fn check_aws(secret: &Value) -> Result<(), String> {
    require(text(secret, "access_key"), "AWS braucht access_key.")?;
    require(text(secret, "secret_key"), "AWS braucht secret_key.")?;
    one_line(text(secret, "access_key"), "Access Key")?;
    one_line(text(secret, "secret_key"), "Secret Key")?;
    one_line(text(secret, "security_token"), "Session-Token")?;
    Ok(())
}

fn check_azure(secret: &Value) -> Result<(), String> {
    require(
        text(secret, "subscription_id"),
        "Azure braucht subscription_id.",
    )?;
    require(text(secret, "client_id"), "Azure braucht client_id.")?;
    require(text(secret, "secret"), "Azure braucht secret.")?;
    require(text(secret, "tenant"), "Azure braucht tenant.")?;
    for (key, label) in [
        ("subscription_id", "Subscription"),
        ("client_id", "Client"),
        ("secret", "Geheimnis"),
        ("tenant", "Tenant"),
    ] {
        one_line(text(secret, key), label)?;
    }
    Ok(())
}

fn check_gce(secret: &Value) -> Result<(), String> {
    let body = text(secret, "service_account");
    require(body, "Dienstkonto fehlt.")?;
    limited(body, 65_536, "Dienstkonto")?;
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Object(_)) => Ok(()),
        _ => Err("Dienstkonto ist kein JSON-Objekt.".into()),
    }
}

fn check_vmware(secret: &Value) -> Result<(), String> {
    require(text(secret, "host"), "VMware braucht host.")?;
    check_login(secret)?;
    one_line(text(secret, "host"), "Host")?;
    Ok(())
}

fn check_openstack(secret: &Value) -> Result<(), String> {
    require(text(secret, "auth_url"), "OpenStack braucht auth_url.")?;
    https_url(text(secret, "auth_url"))?;
    require(text(secret, "project"), "OpenStack braucht project.")?;
    check_login(secret)?;
    one_line(text(secret, "project"), "Projekt")?;
    one_line(text(secret, "domain"), "Domain")?;
    Ok(())
}

fn check_registry(secret: &Value) -> Result<(), String> {
    require(text(secret, "host"), "Registry braucht host.")?;
    check_login(secret)?;
    one_line(text(secret, "host"), "Host")?;
    Ok(())
}

fn check_kubernetes(secret: &Value) -> Result<(), String> {
    require(text(secret, "host"), "Kubernetes braucht host.")?;
    https_url(text(secret, "host"))?;
    require(text(secret, "token"), "Token fehlt.")?;
    one_line(text(secret, "token"), "Token")?;
    Ok(())
}

fn check_token(secret: &Value) -> Result<(), String> {
    let token = text(secret, "token");
    require(token, "Token fehlt.")?;
    one_line(token, "Token")
}

fn check_galaxy(secret: &Value) -> Result<(), String> {
    check_token(secret)?;
    let url = text(secret, "url");
    if url.is_empty() {
        Ok(())
    } else {
        one_line(url, "Adresse")
    }
}

fn check_netbox(secret: &Value) -> Result<(), String> {
    let token = text(secret, "token");
    require(token, "NetBox braucht token.")?;
    one_line(token, "Token")?;
    let url = text(secret, "url");
    if url.is_empty() {
        Ok(())
    } else {
        one_line(url, "Adresse")
    }
}

fn check_login(secret: &Value) -> Result<(), String> {
    require(text(secret, "username"), "Benutzer fehlt.")?;
    require(text(secret, "password"), "Passwort fehlt.")?;
    one_line(text(secret, "username"), "Benutzer")?;
    one_line(text(secret, "password"), "Passwort")?;
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

fn env_material(secret: &Value, env: &mut Vec<(String, String)>) -> Result<(), String> {
    let Some(map) = secret.as_object() else {
        return Err("Env-Zugang muss ein JSON-Objekt sein.".into());
    };
    for (key, value) in map {
        if !safe_ident(key) {
            continue;
        }
        let Some(text) = value.as_str() else { continue };
        if text.contains('\n') || text.contains('\0') || text.contains('\r') {
            return Err(format!(
                "Umgebungsvariable {key} enthält einen Zeilenumbruch."
            ));
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
        return Err(format!("{label} enthält einen Zeilenumbruch."));
    }
    Ok(())
}

fn limited(value: &str, max: usize, label: &str) -> Result<(), String> {
    if value.contains('\0') {
        return Err(format!("{label} ist ungültig."));
    }
    if value.len() > max {
        return Err(format!("{label} ist zu lang."));
    }
    Ok(())
}

fn https_url(value: &str) -> Result<(), String> {
    one_line(value, "Adresse")?;
    if value.starts_with("https://") && !value.contains(' ') {
        Ok(())
    } else {
        Err("Adresse muss mit https:// beginnen.".into())
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
        assert!(check(
            "aws",
            &json!({"access_key": "AKIA", "secret_key": "secret"})
        )
        .is_ok());
    }
}
