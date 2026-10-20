use std::collections::HashSet;
use std::time::Duration;

use ldap3::{LdapConnAsync, LdapConnSettings, Scope, SearchEntry};

use crate::config::LdapConfig;

#[derive(Debug)]
pub struct LdapIdentity {
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub groups: Vec<String>,
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
    for part in split_dn(dn) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("cn") {
            return Some(unescape_dn(value.trim()));
        }
    }
    None
}

fn split_dn(dn: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = dn.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            current.push(ch);
            if let Some(next) = chars.next() {
                current.push(next);
            }
            continue;
        }
        if ch == ',' {
            parts.push(current.trim().to_string());
            current.clear();
            continue;
        }
        current.push(ch);
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

fn unescape_dn(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Group names that should open a team: CN of each DN, plus the raw value.
pub fn group_keys(groups: &[String]) -> HashSet<String> {
    let mut keys = HashSet::new();
    for group in groups {
        keys.insert(group.to_ascii_lowercase());
        if let Some(cn) = cn_from_dn(group) {
            keys.insert(cn.to_ascii_lowercase());
        }
    }
    keys
}

pub async fn authenticate(
    cfg: &LdapConfig,
    username: &str,
    password: &str,
) -> Result<LdapIdentity, String> {
    if password.is_empty() {
        return Err("empty_password".into());
    }
    if cfg.bind_dn.is_empty() || cfg.user_base.is_empty() {
        return Err("ldap_is_not_fully_configured".into());
    }
    let settings = LdapConnSettings::new()
        .set_starttls(cfg.starttls)
        .set_no_tls_verify(cfg.tls_insecure)
        .set_conn_timeout(Duration::from_secs(8));
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
    let display = first_attr(&entry, "displayName")
        .or_else(|| first_attr(&entry, "cn"))
        .unwrap_or_else(|| username.to_string());

    ldap.simple_bind(&user_dn, password)
        .await
        .map_err(|e| format!("LDAP bind failed: {e}"))?
        .success()
        .map_err(|_| "ldap_rejected_the_password".to_string())?;

    // The user bind only checks the password. Group entries are often
    // readable by the service account alone, so search as that account.
    if groups.is_empty() {
        groups = groups_as_service(&mut ldap, cfg, &user_dn, username).await;
    }
    let _ = ldap.unbind().await;
    Ok(LdapIdentity {
        username: username.to_ascii_lowercase(),
        display_name: display,
        email: first_attr(&entry, "mail"),
        groups,
    })
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

fn redact_secret(message: &str, secret: &str) -> String {
    if secret.is_empty() {
        message.to_string()
    } else {
        message.replace(secret, "[redacted]")
    }
}

/// CN, a distinct `name`, and the DN. Matching later ignores case.
pub fn group_names_from_entry(cn: Option<&str>, name: Option<&str>, dn: &str) -> Vec<String> {
    let mut found = Vec::new();
    if let Some(cn) = cn {
        push_group_name(&mut found, cn);
    }
    if let Some(name) = name {
        push_group_name(&mut found, name);
    }
    push_group_name(&mut found, dn);
    found
}

fn push_group_name(found: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    if found
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(value))
    {
        return;
    }
    found.push(value.to_string());
}

async fn search_groups(
    ldap: &mut ldap3::Ldap,
    base: &str,
    user_dn: &str,
    username: &str,
) -> Result<Vec<String>, String> {
    let mut found = Vec::new();
    let mut errors = Vec::new();
    let filters = [
        format!(
            "(&(objectClass=groupOfNames)(member={}))",
            escape_filter(user_dn)
        ),
        format!(
            "(&(objectClass=posixGroup)(memberUid={}))",
            escape_filter(username)
        ),
        format!("(&(objectClass=group)(member={}))", escape_filter(user_dn)),
    ];
    for filter in filters {
        let searched = ldap
            .search(
                base,
                Scope::Subtree,
                &filter,
                vec!["cn", "name", "distinguishedName"],
            )
            .await;
        let entries = match searched {
            Ok(result) => match result.success() {
                Ok((entries, _)) => entries,
                Err(error) => {
                    errors.push(format!("group search rejected: {error}"));
                    continue;
                }
            },
            Err(error) => {
                errors.push(format!("group search: {error}"));
                continue;
            }
        };
        for entry in entries {
            let parsed = SearchEntry::construct(entry);
            found.extend(group_names_from_entry(
                first_attr(&parsed, "cn").as_deref(),
                first_attr(&parsed, "name").as_deref(),
                &parsed.dn,
            ));
            if let Some(distinguished) = first_attr(&parsed, "distinguishedName") {
                push_group_name(&mut found, &distinguished);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_escape_blocks_injection() {
        assert_eq!(escape_filter("a)(uid=*"), r"a\29\28uid=\2a");
    }

    #[test]
    fn reads_cn_from_ad_dn() {
        let dn = r"CN=Plattform\, Ops,OU=Gruppen,DC=example,DC=com";
        assert_eq!(cn_from_dn(dn).as_deref(), Some("Plattform, Ops"));
        let keys = group_keys(&[dn.into()]);
        assert!(keys.contains("plattform, ops"));
    }

    #[test]
    fn fallback_names_match_a_cn_and_a_dn() {
        let names = group_names_from_entry(
            Some("Plattform"),
            Some("PLATTFORM"),
            "cn=Plattform,ou=groups,dc=example,dc=com",
        );
        assert_eq!(
            names,
            vec![
                "Plattform".to_string(),
                "cn=Plattform,ou=groups,dc=example,dc=com".to_string(),
            ]
        );
        let keys = group_keys(&names);
        assert!(keys.contains("plattform"));
        assert!(keys.contains("cn=plattform,ou=groups,dc=example,dc=com"));
        let team_dn = "CN=Plattform,OU=Groups,DC=Example,DC=Com";
        assert!(keys.contains(&team_dn.to_ascii_lowercase()));
    }

    #[test]
    fn fallback_names_keep_a_distinct_name() {
        let names = group_names_from_entry(
            Some("ops"),
            Some("Operations"),
            "cn=ops,ou=groups,dc=example,dc=com",
        );
        assert_eq!(
            names,
            vec![
                "ops".to_string(),
                "Operations".to_string(),
                "cn=ops,ou=groups,dc=example,dc=com".to_string(),
            ]
        );
        let keys = group_keys(&names);
        assert!(keys.contains("operations"));
        assert!(keys.contains("cn=ops,ou=groups,dc=example,dc=com"));
    }

    #[test]
    fn fallback_names_keep_the_dn_when_attributes_are_missing() {
        let names = group_names_from_entry(None, Some("  "), "cn=ops,ou=groups,dc=example,dc=com");
        assert_eq!(
            names,
            vec!["cn=ops,ou=groups,dc=example,dc=com".to_string()]
        );
    }

    #[test]
    fn redact_secret_hides_the_bind_password() {
        assert_eq!(
            redact_secret("service re-bind rejected: secret-value", "secret-value"),
            "service re-bind rejected: [redacted]"
        );
        assert_eq!(redact_secret("no secret here", ""), "no secret here");
    }
}
