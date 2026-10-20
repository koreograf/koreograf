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
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' && index + 1 < bytes.len() {
            let next = bytes[index + 1];
            if index + 2 < bytes.len()
                && next.is_ascii_hexdigit()
                && bytes[index + 2].is_ascii_hexdigit()
            {
                let hex = &value[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                    continue;
                }
            }
            out.push(next);
            index += 2;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Directory DNs and the CN of each DN, kept apart so a CN cannot match a DN.
#[derive(Debug, Default)]
pub struct GroupKeys {
    dns: HashSet<String>,
    cns: HashSet<String>,
}

/// Accepts memberOf values and entry DNs. Other strings are ignored.
pub fn group_keys(groups: &[String]) -> GroupKeys {
    let mut keys = GroupKeys::default();
    for group in groups {
        if !group.contains('=') {
            continue;
        }
        let normalized = normalize_dn(group);
        if normalized.is_empty() {
            continue;
        }
        keys.dns.insert(normalized);
        if let Some(cn) = cn_from_dn(group) {
            keys.cns.insert(cn.to_ascii_lowercase());
        }
    }
    keys
}

/// A configured value that contains `=` is a DN. Anything else is a CN.
pub fn group_matches(keys: &GroupKeys, ldap_group: &str) -> bool {
    let configured = ldap_group.trim();
    if configured.contains('=') {
        keys.dns.contains(&normalize_dn(configured))
    } else {
        keys.cns.contains(&configured.to_ascii_lowercase())
    }
}

fn normalize_dn(dn: &str) -> String {
    let mut parts = Vec::new();
    for part in split_dn(dn) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let key = unescape_dn(key.trim()).to_ascii_lowercase();
        let value = unescape_dn(value.trim()).to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        parts.push(format!("{key}={value}"));
    }
    parts.join(",")
}

pub async fn authenticate(
    cfg: &LdapConfig,
    username: &str,
    password: &str,
) -> Result<LdapIdentity, String> {
    if password.is_empty() {
        return Err("empty_password".into());
    }
    // An empty service password is an unauthenticated bind. Some directories
    // accept it and then search as anonymous.
    if cfg.bind_dn.is_empty() || cfg.user_base.is_empty() || cfg.bind_password.is_empty() {
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

fn group_search_filters(user_dn: &str, username: &str) -> Vec<String> {
    let user_dn = escape_filter(user_dn);
    let username = escape_filter(username);
    vec![
        format!("(&(objectClass=groupOfNames)(member={user_dn}))"),
        format!("(&(objectClass=posixGroup)(memberUid={username}))"),
        format!("(&(objectClass=group)(member={user_dn}))"),
        format!("(&(objectClass=groupOfUniqueNames)(uniqueMember={user_dn}))"),
    ]
}

fn push_unique(found: &mut Vec<String>, value: String) {
    if !found.iter().any(|existing| existing == &value) {
        found.push(value);
    }
}

async fn search_groups(
    ldap: &mut ldap3::Ldap,
    base: &str,
    user_dn: &str,
    username: &str,
) -> Result<Vec<String>, String> {
    let mut found = Vec::new();
    let mut errors = Vec::new();
    for filter in group_search_filters(user_dn, username) {
        let searched = ldap
            .search(base, Scope::Subtree, &filter, vec!["objectClass"])
            .await;
        let entries = match searched {
            Ok(result) => match result.success() {
                Ok((entries, _)) => entries,
                Err(error) => {
                    push_unique(&mut errors, format!("group search rejected: {error}"));
                    continue;
                }
            },
            Err(error) => {
                push_unique(&mut errors, format!("group search: {error}"));
                continue;
            }
        };
        for entry in entries {
            let parsed = SearchEntry::construct(entry);
            if !parsed.dn.is_empty() {
                found.push(parsed.dn);
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
        assert!(group_matches(&keys, "Plattform, Ops"));
        assert!(group_matches(&keys, dn));
    }

    #[test]
    fn a_group_is_identified_by_its_entry_dn() {
        let dn = "cn=Plattform,ou=groups,dc=example,dc=com";
        let keys = group_keys(&[dn.into()]);
        assert!(group_matches(&keys, "plattform"));
        assert!(group_matches(&keys, dn));
        let team_dn = "CN=Plattform,OU=Groups,DC=Example,DC=Com";
        assert!(group_matches(&keys, team_dn));
        assert!(!group_matches(&keys, "operations"));
    }

    #[test]
    fn a_cn_that_decodes_to_another_dn_does_not_match_that_dn() {
        let real = "cn=kg-admins,ou=groups,dc=example,dc=com";
        let spoof = r"cn=cn\3Dkg-admins\2Cou\3Dgroups\2Cdc\3Dexample\2Cdc\3Dcom,ou=groups,dc=example,dc=com";
        let forged = group_keys(&[spoof.into()]);
        assert!(!group_matches(&forged, real));
        assert!(!group_matches(&forged, "kg-admins"));
        let real_keys = group_keys(&[real.into()]);
        assert!(group_matches(&real_keys, real));
        assert!(group_matches(&real_keys, "KG-ADMINS"));
        assert!(!group_matches(&real_keys, spoof));
        let hex = r"cn=kg\2Dadmins,ou=groups,dc=example,dc=com";
        assert!(group_matches(&group_keys(&[hex.into()]), real));
        assert!(!group_matches(
            &group_keys(&["kg-admins".into()]),
            "kg-admins"
        ));
    }

    #[test]
    fn group_search_includes_unique_members() {
        let filters = group_search_filters("cn=alice,ou=people,dc=example,dc=com", "alice");
        assert!(filters.iter().any(|filter| {
            filter == "(&(objectClass=groupOfUniqueNames)(uniqueMember=cn=alice,ou=people,dc=example,dc=com))"
        }));
        let hostile = group_search_filters("cn=a)(uid=*,ou=people", "a)(uid=*");
        assert!(hostile.iter().all(|filter| !filter.contains("uid=*")));
        assert!(hostile
            .iter()
            .any(|filter| { filter.contains(r"uniqueMember=cn=a\29\28uid=\2a,ou=people") }));
    }

    #[test]
    fn hex_escapes_match_the_same_cn_and_dn() {
        let server = r"cn=Ops\2C Nord,ou=groups,dc=example,dc=com";
        assert_eq!(cn_from_dn(server).as_deref(), Some("Ops, Nord"));
        assert_eq!(
            cn_from_dn(r"cn=a\2Bb,ou=groups,dc=example,dc=com").as_deref(),
            Some("a+b")
        );
        assert_eq!(
            cn_from_dn(r"cn=\23hash,ou=groups,dc=example,dc=com").as_deref(),
            Some("#hash")
        );
        assert_eq!(
            cn_from_dn(r"cn=a/b\3Bc,ou=groups,dc=example,dc=com").as_deref(),
            Some("a/b;c")
        );
        let decoded =
            String::from_utf8(vec![b'M', 0xC3, 0xBC, b'l', b'l', b'e', b'r']).expect("utf-8");
        assert_eq!(
            cn_from_dn(r"cn=M\C3\BCller,ou=groups,dc=example,dc=com").as_deref(),
            Some(decoded.as_str())
        );
        let keys = group_keys(&[server.into()]);
        assert!(group_matches(&keys, "Ops, Nord"));
        assert!(!group_matches(&keys, "Ops2C Nord"));
        assert!(group_matches(
            &keys,
            r"cn=Ops\, Nord,ou=groups,dc=example,dc=com"
        ));
        assert!(group_matches(
            &keys,
            r"CN=Ops\2C Nord,OU=Groups,DC=Example,DC=Com"
        ));
        let multi = group_keys(&[r"cn=mrdn+ou=x,dc=example,dc=com".into()]);
        assert!(!group_matches(&multi, "mrdn"));
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
