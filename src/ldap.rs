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

    if groups.is_empty() {
        let base = if cfg.group_base.is_empty() {
            cfg.user_base.as_str()
        } else {
            cfg.group_base.as_str()
        };
        groups = search_groups(&mut ldap, base, &user_dn, username).await;
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

async fn search_groups(
    ldap: &mut ldap3::Ldap,
    base: &str,
    user_dn: &str,
    username: &str,
) -> Vec<String> {
    let mut found = Vec::new();
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
        let Ok(result) = ldap.search(base, Scope::Subtree, &filter, vec!["cn"]).await else {
            continue;
        };
        let Ok((entries, _)) = result.success() else {
            continue;
        };
        for entry in entries {
            let parsed = SearchEntry::construct(entry);
            if let Some(cn) = first_attr(&parsed, "cn") {
                found.push(cn);
            } else {
                found.push(parsed.dn);
            }
        }
    }
    found.sort();
    found.dedup();
    found
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
}
