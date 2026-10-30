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
    let rdns = parse_rdns(dn);
    let first = rdns.first()?;
    if first.len() != 1 {
        return None;
    }
    let ava = &first[0];
    if ava.type_name.eq_ignore_ascii_case("cn") {
        Some(ava.value.clone())
    } else {
        None
    }
}

struct Ava {
    type_name: String,
    value: String,
}

/// RDNs in order. Each RDN is the set of its attribute-value pairs.
fn parse_rdns(dn: &str) -> Vec<Vec<Ava>> {
    let mut rdns = Vec::new();
    for raw_rdn in split_unescaped(dn, b',') {
        let raw_rdn = trim_insignificant(raw_rdn);
        if raw_rdn.is_empty() {
            continue;
        }
        let mut avas = Vec::new();
        for raw_ava in split_unescaped(raw_rdn, b'+') {
            let raw_ava = trim_insignificant(raw_ava);
            let Some((raw_type, raw_value)) = split_once_unescaped(raw_ava, b'=') else {
                continue;
            };
            let type_name = unescape_dn(trim_insignificant(raw_type));
            if type_name.is_empty() {
                continue;
            }
            avas.push(Ava {
                type_name,
                value: unescape_dn(trim_insignificant(raw_value)),
            });
        }
        if !avas.is_empty() {
            rdns.push(avas);
        }
    }
    rdns
}

fn split_unescaped(input: &str, separator: u8) -> Vec<&str> {
    let bytes = input.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += escape_span(&bytes[index..]);
            continue;
        }
        if bytes[index] == separator {
            parts.push(&input[start..index]);
            index += 1;
            start = index;
            continue;
        }
        index += 1;
    }
    parts.push(&input[start..]);
    parts
}

fn split_once_unescaped(input: &str, separator: u8) -> Option<(&str, &str)> {
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += escape_span(&bytes[index..]);
            continue;
        }
        if bytes[index] == separator {
            return Some((&input[..index], &input[index + 1..]));
        }
        index += 1;
    }
    None
}

fn escape_span(bytes: &[u8]) -> usize {
    if bytes.len() >= 3 && bytes[1].is_ascii_hexdigit() && bytes[2].is_ascii_hexdigit() {
        3
    } else if bytes.len() >= 2 {
        2
    } else {
        1
    }
}

/// Spaces next to a separator are not part of the value unless they are escaped.
fn trim_insignificant(input: &str) -> &str {
    let bytes = input.as_bytes();
    let mut start = 0;
    while start < bytes.len() && bytes[start] == b' ' {
        start += 1;
    }
    let mut end = bytes.len();
    while end > start && bytes[end - 1] == b' ' {
        let mut slashes = 0;
        let mut index = end - 1;
        while index > start && bytes[index - 1] == b'\\' {
            slashes += 1;
            index -= 1;
        }
        if slashes % 2 == 1 {
            break;
        }
        end -= 1;
    }
    &input[start..end]
}

fn unescape_dn(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            let rest = &bytes[index..];
            if rest.len() >= 3 && rest[1].is_ascii_hexdigit() && rest[2].is_ascii_hexdigit() {
                let hex = &value[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                    continue;
                }
            }
            if rest.len() >= 2 {
                out.push(rest[1]);
                index += 2;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn escape_rdn_value(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    for (index, ch) in chars.iter().copied().enumerate() {
        let first = index == 0;
        let last = index + 1 == chars.len();
        match ch {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            '+' => out.push_str("\\+"),
            '"' => out.push_str("\\\""),
            '<' => out.push_str("\\<"),
            '>' => out.push_str("\\>"),
            ';' => out.push_str("\\;"),
            '=' => out.push_str("\\="),
            ' ' if first || last => out.push_str("\\ "),
            '#' if first => out.push_str("\\#"),
            other if other.is_control() => {
                let mut buf = [0; 4];
                for byte in other.encode_utf8(&mut buf).bytes() {
                    out.push('\\');
                    out.push(hex_digit(byte >> 4));
                    out.push(hex_digit(byte & 0x0f));
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn hex_digit(nibble: u8) -> char {
    char::from(b"0123456789ABCDEF"[usize::from(nibble)])
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
    let mut rdns = Vec::new();
    for rdn in parse_rdns(dn) {
        let mut avas = rdn
            .iter()
            .map(|ava| {
                let type_name = escape_rdn_value(&ava.type_name.to_ascii_lowercase());
                let value = escape_rdn_value(&ava.value.to_ascii_lowercase());
                format!("{type_name}={value}")
            })
            .collect::<Vec<_>>();
        avas.sort();
        rdns.push(avas.join("+"));
    }
    rdns.join(",")
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
    let settings = match connection_settings(cfg) {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!(url = %cfg.url, %error, "LDAP TLS setup failed");
            return Err(format!("ldap tls setup: {error}"));
        }
    };
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
    let (account, from_directory) = directory_username(&entry, &cfg.username_attr, username);
    let display = first_attr(&entry, "displayName")
        .or_else(|| first_attr(&entry, "cn"))
        .unwrap_or_else(|| account.clone());

    ldap.simple_bind(&user_dn, password)
        .await
        .map_err(|e| format!("LDAP bind failed: {e}"))?
        .success()
        .map_err(|_| "ldap_rejected_the_password".to_string())?;
    if !from_directory {
        tracing::warn!(
            attribute = %cfg.username_attr,
            "LDAP username attribute is missing, so the entered login is the account name"
        );
    }

    // The user bind only checks the password. Group entries are often
    // readable by the service account alone, so search as that account.
    if groups.is_empty() {
        groups = groups_as_service(&mut ldap, cfg, &user_dn, &account).await;
    }
    let _ = ldap.unbind().await;
    Ok(LdapIdentity {
        username: account,
        display_name: display,
        email: first_attr(&entry, "mail"),
        groups,
    })
}

fn connection_settings(cfg: &LdapConfig) -> Result<LdapConnSettings, String> {
    let mut settings = LdapConnSettings::new()
        .set_starttls(cfg.starttls)
        .set_conn_timeout(Duration::from_secs(8));
    if let Some(config) = crate::tls::ldap_client_config(&cfg.ca_file)? {
        settings = settings.set_config(config);
    } else {
        settings = settings.set_no_tls_verify(cfg.tls_insecure);
    }
    Ok(settings)
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

/// Account name from the directory. The bool is false when the attribute is missing.
fn directory_username(entry: &SearchEntry, attr: &str, entered: &str) -> (String, bool) {
    let found = first_attr(entry, attr)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    match found {
        Some(value) => (value, true),
        None => (entered.trim().to_ascii_lowercase(), false),
    }
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
            // 1.1 requests no attributes. Matching uses the entry DN.
            .search(base, Scope::Subtree, &filter, vec!["1.1"])
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
        assert!(group_matches(
            &keys,
            "cn=Plattform\\, Ops, ou=Gruppen, dc=example, dc=com"
        ));
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
        assert!(filters
            .iter()
            .any(|filter| { filter == "(&(objectClass=posixGroup)(memberUid=alice))" }));
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
    fn the_directory_username_attribute_overrides_the_entered_login() {
        let mail = search_entry(&[("uid", "Alice"), ("mail", "alice@example.com")]);
        assert_eq!(
            directory_username(&mail, "uid", "alice@example.com"),
            ("alice".into(), true)
        );
        let ad = search_entry(&[("sAMAccountName", "Alice")]);
        assert_eq!(
            directory_username(&ad, "sAMAccountName", "Alice"),
            ("alice".into(), true)
        );
        let missing = search_entry(&[("mail", "alice@example.com")]);
        assert_eq!(
            directory_username(&missing, "uid", " Alice@Example.com "),
            ("alice@example.com".into(), false)
        );
        let blank = search_entry(&[("uid", "  ")]);
        assert_eq!(
            directory_username(&blank, "uid", "alice"),
            ("alice".into(), false)
        );
    }

    fn search_entry(attrs: &[(&str, &str)]) -> SearchEntry {
        let mut map = std::collections::HashMap::new();
        for (key, value) in attrs {
            map.insert((*key).to_string(), vec![(*value).to_string()]);
        }
        SearchEntry {
            dn: "uid=alice,ou=people,dc=example,dc=com".into(),
            attrs: map,
            bin_attrs: std::collections::HashMap::new(),
        }
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
    fn escaped_comma_and_equals_stay_inside_the_cn_value() {
        let hex = r"cn=kg-admins\2Cou\3Dgroups,dc=example,dc=com";
        let slash = r"cn=kg-admins\,ou\=groups,dc=example,dc=com";
        let split = "cn=kg-admins,ou=groups,dc=example,dc=com";
        assert_eq!(cn_from_dn(hex).as_deref(), Some("kg-admins,ou=groups"));
        assert_eq!(cn_from_dn(slash).as_deref(), Some("kg-admins,ou=groups"));
        assert_eq!(normalize_dn(hex), normalize_dn(slash));
        assert_ne!(normalize_dn(hex), normalize_dn(split));
        let keys = group_keys(&[hex.into()]);
        assert!(group_matches(&keys, slash));
        assert!(!group_matches(&keys, split));
        assert!(!group_matches(&keys, "kg-admins"));
        assert!(!group_matches(&group_keys(&[slash.into()]), split));
        assert!(!group_matches(&keys, "kg-admins,ou=groups"));
    }

    #[test]
    fn the_cn_comes_only_from_the_first_rdn() {
        let dn = "ou=sub,cn=kg-admins,ou=groups,dc=example,dc=com";
        assert!(cn_from_dn(dn).is_none());
        let keys = group_keys(&[dn.into()]);
        assert!(!group_matches(&keys, "kg-admins"));
        assert!(group_matches(&keys, dn));
        assert!(group_matches(
            &keys,
            "OU=sub,CN=kg-admins,OU=groups,DC=example,DC=com"
        ));
    }

    #[test]
    fn a_multi_valued_rdn_has_no_cn() {
        let forward = r"cn=mrdn+ou=x,dc=example,dc=com";
        let reverse = r"ou=x+cn=mrdn,dc=example,dc=com";
        assert!(cn_from_dn(forward).is_none());
        assert!(cn_from_dn(reverse).is_none());
        assert_eq!(normalize_dn(forward), normalize_dn(reverse));
        let keys = group_keys(&[forward.into()]);
        assert!(!group_matches(&keys, "mrdn"));
        assert!(group_matches(&keys, reverse));
        assert_ne!(
            normalize_dn(r"cn=a\+b,dc=example,dc=com"),
            normalize_dn(r"cn=a+ou=b,dc=example,dc=com")
        );
        assert_eq!(
            cn_from_dn(r"cn=a\+b,dc=example,dc=com").as_deref(),
            Some("a+b")
        );
    }

    #[test]
    fn memberof_hex_escapes_match_the_group() {
        let member = r"cn=Ops\2C Nord,ou=groups,dc=example,dc=com";
        let keys = group_keys(&[member.into()]);
        assert!(group_matches(&keys, "Ops, Nord"));
        assert!(group_matches(
            &keys,
            r"cn=Ops\, Nord,ou=groups,dc=example,dc=com"
        ));
        assert!(group_matches(
            &keys,
            r"CN=Ops\2C Nord,OU=Groups,DC=Example,DC=Com"
        ));
        assert!(!group_matches(
            &keys,
            "cn=Ops,ou=Nord,ou=groups,dc=example,dc=com"
        ));
    }

    #[tokio::test]
    async fn an_empty_bind_password_is_not_fully_configured() {
        let cfg = LdapConfig {
            url: "ldap://127.0.0.1:389".into(),
            mode: "ldap".into(),
            bind_dn: "cn=reader,dc=example,dc=com".into(),
            bind_password: String::new(),
            user_base: "ou=people,dc=example,dc=com".into(),
            user_filter: "(uid={username})".into(),
            username_attr: "uid".into(),
            group_attr: "memberOf".into(),
            group_base: String::new(),
            starttls: false,
            tls_insecure: false,
            ca_file: String::new(),
        };
        let error = authenticate(&cfg, "alice", "secret")
            .await
            .expect_err("empty bind password");
        assert_eq!(error, "ldap_is_not_fully_configured");
    }

    #[test]
    fn a_missing_ca_file_does_not_turn_verification_off() {
        let cfg = LdapConfig {
            url: "ldaps://ldap.example.com".into(),
            mode: "ldap".into(),
            bind_dn: "cn=reader,dc=example,dc=com".into(),
            bind_password: "secret".into(),
            user_base: "ou=people,dc=example,dc=com".into(),
            user_filter: "(uid={username})".into(),
            username_attr: "uid".into(),
            group_attr: "memberOf".into(),
            group_base: String::new(),
            starttls: false,
            tls_insecure: true,
            ca_file: "/no/such/koreograf-ca.pem".into(),
        };
        let Err(error) = connection_settings(&cfg) else {
            panic!("missing CA file was accepted");
        };
        assert_eq!(error, "the_ca_file_cannot_be_read");
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
