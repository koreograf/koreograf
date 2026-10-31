use std::collections::BTreeMap;

use serde::Serialize;

/// One host parsed from an Ansible INI inventory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InventoryHost {
    pub name: String,
    pub groups: Vec<String>,
    pub vars: BTreeMap<String, String>,
}

/// Combine hosts from several inventory texts. The same host keeps every group.
/// Variables from a later text replace earlier ones.
pub fn merge_hosts(bodies: &[&str]) -> Vec<InventoryHost> {
    let mut hosts: Vec<InventoryHost> = Vec::new();
    for body in bodies {
        for host in parse_hosts(body) {
            if let Some(existing) = hosts.iter_mut().find(|item| item.name == host.name) {
                for group in host.groups {
                    if !existing.groups.iter().any(|item| item == &group) {
                        existing.groups.push(group);
                    }
                }
                existing.vars.extend(host.vars);
            } else {
                hosts.push(host);
            }
        }
    }
    hosts
}

/// Read host names, groups and variables from an INI inventory or a simple YAML inventory.
///
/// Lines under `[group:vars]` and `[group:children]` are not hosts. A NetBox plugin
/// file has no static hosts until a sync replaces it with INI.
pub fn parse_hosts(body: &str) -> Vec<InventoryHost> {
    if is_plugin_file(body) {
        return Vec::new();
    }
    if is_yaml_inventory(body) {
        return parse_yaml_inventory(body);
    }
    parse_ini(body)
}

/// Find the first line of a static INI inventory that Ansible cannot read.
///
/// Returns the 1-based line number. Plugin files and YAML inventories are
/// not INI, so they have no line to report.
pub fn find_ini_error(body: &str) -> Option<usize> {
    if is_plugin_file(body) || is_yaml_inventory(body) {
        return None;
    }
    let mut listing_hosts = true;
    for (index, raw) in body.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            let Some(inner) = line
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            else {
                return Some(index + 1);
            };
            let (name, hosts_follow) = if let Some(name) = inner.strip_suffix(":vars") {
                (name, false)
            } else if let Some(name) = inner.strip_suffix(":children") {
                (name, false)
            } else {
                (inner, true)
            };
            if name.trim().is_empty() {
                return Some(index + 1);
            }
            listing_hosts = hosts_follow;
            continue;
        }
        if !listing_hosts {
            continue;
        }
        let mut parts = host_line_tokens(line).into_iter();
        let Some(name) = parts.next() else {
            continue;
        };
        if name.contains('=') {
            return Some(index + 1);
        }
        for part in parts {
            if let Some((key, _)) = part.split_once('=') {
                if key.is_empty() {
                    return Some(index + 1);
                }
            }
        }
    }
    None
}

fn parse_ini(body: &str) -> Vec<InventoryHost> {
    let mut hosts: Vec<InventoryHost> = Vec::new();
    let mut group = "ungrouped".to_string();
    let mut listing_hosts = true;
    for raw in body.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            let inner = &line[1..line.len() - 1];
            if let Some(name) = inner.strip_suffix(":vars") {
                group = name.trim().to_string();
                listing_hosts = false;
            } else if let Some(name) = inner.strip_suffix(":children") {
                group = name.trim().to_string();
                listing_hosts = false;
            } else {
                group = inner.trim().to_string();
                listing_hosts = true;
            }
            continue;
        }
        if !listing_hosts {
            continue;
        }
        let mut parts = host_line_tokens(line).into_iter();
        let Some(name) = parts.next() else {
            continue;
        };
        if name.contains('=') {
            continue;
        }
        let mut vars = BTreeMap::new();
        for part in parts {
            if let Some((key, value)) = part.split_once('=') {
                if !key.is_empty() {
                    vars.insert(key.to_string(), value.to_string());
                }
            }
        }
        if let Some(existing) = hosts.iter_mut().find(|host| host.name == name) {
            if !existing.groups.iter().any(|item| item == &group) {
                existing.groups.push(group.clone());
            }
            existing.vars.extend(vars);
        } else {
            hosts.push(InventoryHost {
                name: name.to_string(),
                groups: vec![group.clone()],
                vars,
            });
        }
    }
    hosts
}

/// Tokens of an INI host line. Quoted values keep their spaces; a line with
/// unbalanced quotes falls back to plain whitespace splitting.
fn host_line_tokens(line: &str) -> Vec<String> {
    match shlex::split(line) {
        Some(tokens) => tokens,
        None => line.split_whitespace().map(str::to_string).collect(),
    }
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(index) => &line[..index],
        None => line,
    }
}

fn significant_lines(body: &str) -> Vec<&str> {
    body.lines()
        .map(|line| line.split('#').next().unwrap_or(line).trim())
        .filter(|line| !line.is_empty() && *line != "---")
        .collect()
}

fn is_plugin_file(body: &str) -> bool {
    significant_lines(body)
        .iter()
        .any(|line| line.starts_with("plugin:"))
}

fn is_yaml_inventory(body: &str) -> bool {
    let lines = significant_lines(body);
    let Some(first) = lines.first() else {
        return false;
    };
    !first.starts_with('[')
        && !first.starts_with('-')
        && first.ends_with(':')
        && lines.iter().any(|line| line.ends_with("hosts:"))
}

fn parse_yaml_inventory(body: &str) -> Vec<InventoryHost> {
    let mut hosts: Vec<InventoryHost> = Vec::new();
    let mut group = "all".to_string();
    let mut host: Option<String> = None;
    let mut hosts_indent: Option<usize> = None;
    let mut in_hosts = false;
    for raw in body.lines() {
        let Some(code) = raw.split('#').next() else {
            continue;
        };
        if code.trim().is_empty() || code.trim() == "---" {
            continue;
        }
        let indent = code.chars().take_while(|ch| *ch == ' ').count();
        let line = code.trim();
        if line.starts_with('-') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if key.is_empty() {
            continue;
        }
        if hosts_indent.is_some_and(|level| indent <= level) {
            in_hosts = false;
            host = None;
            hosts_indent = None;
        }
        if key == "hosts" && value.is_empty() {
            in_hosts = true;
            hosts_indent = Some(indent);
            host = None;
            continue;
        }
        if key == "children" && value.is_empty() {
            in_hosts = false;
            host = None;
            continue;
        }
        if in_hosts && value.is_empty() {
            host = Some(key.to_string());
            remember_host(&mut hosts, key, &group);
            continue;
        }
        if in_hosts {
            if let Some(name) = &host {
                if !key.is_empty() && !value.is_empty() {
                    set_var(&mut hosts, name, key, value);
                }
            }
            continue;
        }
        if value.is_empty() && key != "vars" {
            group = key.to_string();
        }
    }
    hosts
}

fn remember_host(hosts: &mut Vec<InventoryHost>, name: &str, group: &str) {
    if let Some(existing) = hosts.iter_mut().find(|host| host.name == name) {
        if !existing.groups.iter().any(|item| item == group) {
            existing.groups.push(group.to_string());
        }
    } else {
        hosts.push(InventoryHost {
            name: name.to_string(),
            groups: vec![group.to_string()],
            vars: BTreeMap::new(),
        });
    }
}

fn set_var(hosts: &mut [InventoryHost], name: &str, key: &str, value: &str) {
    if let Some(host) = hosts.iter_mut().find(|host| host.name == name) {
        host.vars.insert(key.to_string(), value.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hosts_groups_and_skips_children() {
        let body = "\
# demo
[web]
web-1 ansible_host=10.0.0.5
web-2

[db]
db-1 ansible_user=ops

[web:vars]
ansible_port=22

[app:children]
web
db

[all]
web-1 ansible_connection=local
";
        let hosts = parse_hosts(body);
        assert_eq!(hosts.len(), 3);
        assert_eq!(hosts[0].name, "web-1");
        assert_eq!(hosts[0].groups, vec!["web".to_string(), "all".to_string()]);
        assert_eq!(
            hosts[0].vars.get("ansible_host").map(String::as_str),
            Some("10.0.0.5")
        );
        assert_eq!(
            hosts[0].vars.get("ansible_connection").map(String::as_str),
            Some("local")
        );
        assert_eq!(hosts[1].name, "web-2");
        assert_eq!(hosts[2].name, "db-1");
        assert!(hosts
            .iter()
            .all(|host| host.name != "web" && host.name != "db"));

        let hosts = parse_hosts("[web]\nweb-1 ansible_ssh_common_args=\"-o ProxyCommand =x\"\n");
        assert_eq!(
            hosts[0]
                .vars
                .get("ansible_ssh_common_args")
                .map(String::as_str),
            Some("-o ProxyCommand =x")
        );
    }

    #[test]
    fn reads_yaml_inventory_and_skips_plugin_files() {
        let body = "\
all:
  hosts:
    web-1:
      ansible_host: 10.0.0.8
  children:
    db:
      hosts:
        db-1:
          ansible_user: ops
";
        let hosts = parse_hosts(body);
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].name, "web-1");
        assert_eq!(hosts[0].groups, vec!["all".to_string()]);
        assert_eq!(
            hosts[0].vars.get("ansible_host").map(String::as_str),
            Some("10.0.0.8")
        );
        assert_eq!(hosts[1].name, "db-1");
        assert_eq!(hosts[1].groups, vec!["db".to_string()]);
        assert!(
            parse_hosts("plugin: netbox.netbox.nb_inventory\napi_endpoint: https://nb\n")
                .is_empty()
        );
    }

    #[test]
    fn merges_hosts_from_several_sources() {
        let hosts = merge_hosts(&[
            "[web]\nweb-1\n",
            "[db]\ndb-1\n[web]\nweb-1 ansible_host=10.0.0.8\n",
        ]);
        assert_eq!(hosts.len(), 2);
        let web = hosts.iter().find(|host| host.name == "web-1").unwrap();
        assert_eq!(web.groups, vec!["web".to_string()]);
        assert_eq!(
            web.vars.get("ansible_host").map(String::as_str),
            Some("10.0.0.8")
        );
        assert!(hosts.iter().any(|host| host.name == "db-1"));
    }

    #[test]
    fn rejects_broken_ini_lines_with_their_line_number() {
        assert_eq!(find_ini_error("[all\nh1 ansible_host=\n[[["), Some(1));
        assert_eq!(find_ini_error("[web]\nweb-1\n[db\ndb-1\n"), Some(3));
        assert_eq!(find_ini_error("[web]\nansible_host=10.0.0.5\n"), Some(2));
        assert_eq!(find_ini_error("[web]\nweb-1 =\n"), Some(2));
        assert_eq!(find_ini_error("[]\n"), Some(1));
    }

    #[test]
    fn accepts_valid_ini_and_non_ini_bodies() {
        let body = "\
# demo
[web]
web-1 ansible_host=10.0.0.5
web-2

[db]
db-1 ansible_user=ops

[web:vars]
ansible_port=22

[app:children]
web
db
";
        assert_eq!(find_ini_error(body), None);
        assert_eq!(find_ini_error(""), None);
        assert_eq!(find_ini_error("# only a comment\n"), None);
        assert_eq!(
            find_ini_error("plugin: netbox.netbox.nb_inventory\napi_endpoint: https://nb\n"),
            None
        );
        assert_eq!(
            find_ini_error("all:\n  hosts:\n    web-1:\n      ansible_host: 10.0.0.8\n"),
            None
        );
        assert_eq!(
            find_ini_error("[web]\nweb-1 ansible_ssh_common_args=\"-o ProxyCommand =x\"\n"),
            None
        );
    }
}
