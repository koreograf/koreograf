use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

/// One host parsed from an Ansible INI inventory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InventoryHost {
    pub name: String,
    pub groups: Vec<String>,
    pub vars: BTreeMap<String, String>,
}

/// One group parsed from an Ansible INI or YAML inventory.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct InventoryGroup {
    pub name: String,
    pub children: Vec<String>,
    pub vars: BTreeMap<String, String>,
}

/// Hosts and groups of one or more inventory texts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct InventoryData {
    pub hosts: Vec<InventoryHost>,
    pub groups: Vec<InventoryGroup>,
}

/// One effective host variable and the place that set it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EffectiveVar {
    pub name: String,
    pub value: String,
    pub origin: String,
}

/// Combine hosts and groups from several inventory texts. The same host keeps
/// every group. Variables from a later text replace earlier ones.
pub fn merge_inventory(bodies: &[&str]) -> InventoryData {
    let mut data = InventoryData::default();
    for body in bodies {
        let parsed = parse_inventory(body);
        for host in parsed.hosts {
            if let Some(existing) = data.hosts.iter_mut().find(|item| item.name == host.name) {
                for group in host.groups {
                    if !existing.groups.iter().any(|item| item == &group) {
                        existing.groups.push(group);
                    }
                }
                existing.vars.extend(host.vars);
            } else {
                data.hosts.push(host);
            }
        }
        for group in parsed.groups {
            if let Some(existing) = data.groups.iter_mut().find(|item| item.name == group.name) {
                for child in group.children {
                    if !existing.children.iter().any(|item| item == &child) {
                        existing.children.push(child);
                    }
                }
                existing.vars.extend(group.vars);
            } else {
                data.groups.push(group);
            }
        }
    }
    data
}

/// Read host names, groups and variables from an INI inventory or a simple YAML inventory.
///
/// Lines under `[group:vars]` and `[group:children]` are not hosts. A NetBox plugin
/// file has no static hosts until a sync replaces it with INI.
pub fn parse_hosts(body: &str) -> Vec<InventoryHost> {
    parse_inventory(body).hosts
}

/// Read hosts and their groups, including group variables and child groups.
pub fn parse_inventory(body: &str) -> InventoryData {
    if is_plugin_file(body) {
        return InventoryData::default();
    }
    if is_yaml_inventory(body) {
        return parse_yaml_inventory(body);
    }
    parse_ini(body)
}

/// Group names of a host, including parents that list it through `children`.
pub fn host_groups(data: &InventoryData, host_name: &str) -> Vec<String> {
    let Some(host) = data.hosts.iter().find(|host| host.name == host_name) else {
        return Vec::new();
    };
    let parents = parent_map(data);
    let mut names = BTreeSet::new();
    for group in &host.groups {
        collect_ancestors(group, &parents, &mut names);
    }
    if data.groups.iter().any(|group| group.name == "all") {
        names.insert("all".to_string());
    }
    names.into_iter().collect()
}

/// Hosts in a group or in one of its child groups, in inventory order.
pub fn group_hosts<'a>(data: &'a InventoryData, group_name: &str) -> Vec<&'a InventoryHost> {
    if group_name == "all" {
        return data.hosts.iter().collect();
    }
    let mut names = BTreeSet::new();
    collect_descendants(group_name, data, &mut names);
    data.hosts
        .iter()
        .filter(|host| host.groups.iter().any(|group| names.contains(group)))
        .collect()
}

/// Effective variables of a host in Ansible precedence order.
///
/// `all` is lowest, a child group overrides its parent groups, and host variables
/// override every group. Variables of unrelated groups merge by name.
pub fn effective_vars(data: &InventoryData, host_name: &str) -> Vec<EffectiveVar> {
    let Some(host) = data.hosts.iter().find(|host| host.name == host_name) else {
        return Vec::new();
    };
    let groups = host_groups(data, host_name);
    let mut ordered: Vec<(usize, String)> = groups
        .into_iter()
        .map(|name| (group_depth(&name, data), name))
        .collect();
    ordered.sort_by(|left, right| {
        let left_all = left.1 == "all";
        let right_all = right.1 == "all";
        (!left_all, left.0, &left.1).cmp(&(!right_all, right.0, &right.1))
    });
    let mut vars = BTreeMap::new();
    for (_, group) in ordered {
        if let Some(item) = data.groups.iter().find(|item| item.name == group) {
            for (key, value) in &item.vars {
                vars.insert(key.clone(), (value.clone(), group.clone()));
            }
        }
    }
    for (key, value) in &host.vars {
        vars.insert(key.clone(), (value.clone(), "host".into()));
    }
    vars.into_iter()
        .map(|(name, (value, origin))| EffectiveVar {
            name,
            value,
            origin,
        })
        .collect()
}

fn parent_map(data: &InventoryData) -> BTreeMap<String, Vec<String>> {
    let mut parents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for group in &data.groups {
        for child in &group.children {
            parents
                .entry(child.clone())
                .or_default()
                .push(group.name.clone());
        }
    }
    parents
}

fn collect_ancestors(
    name: &str,
    parents: &BTreeMap<String, Vec<String>>,
    names: &mut BTreeSet<String>,
) {
    if !names.insert(name.to_string()) {
        return;
    }
    if let Some(items) = parents.get(name) {
        for parent in items {
            collect_ancestors(parent, parents, names);
        }
    }
}

fn collect_descendants(name: &str, data: &InventoryData, names: &mut BTreeSet<String>) {
    if !names.insert(name.to_string()) {
        return;
    }
    if let Some(group) = data.groups.iter().find(|item| item.name == name) {
        for child in &group.children {
            collect_descendants(child, data, names);
        }
    }
}

fn group_depth(name: &str, data: &InventoryData) -> usize {
    let parents = parent_map(data);
    depth_of(name, &parents, &mut BTreeSet::new())
}

fn depth_of(
    name: &str,
    parents: &BTreeMap<String, Vec<String>>,
    seen: &mut BTreeSet<String>,
) -> usize {
    if !seen.insert(name.to_string()) {
        return 0;
    }
    let depth = parents
        .get(name)
        .and_then(|items| {
            items
                .iter()
                .map(|item| depth_of(item, parents, seen) + 1)
                .max()
        })
        .unwrap_or(0);
    seen.remove(name);
    depth
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum IniSection {
    Hosts,
    Vars,
    Children,
}

fn parse_ini(body: &str) -> InventoryData {
    let mut data = InventoryData::default();
    let mut group = "ungrouped".to_string();
    let mut section = IniSection::Hosts;
    for raw in body.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            let inner = &line[1..line.len() - 1];
            if let Some(name) = inner.strip_suffix(":vars") {
                group = name.trim().to_string();
                section = IniSection::Vars;
                ensure_group(&mut data.groups, &group);
            } else if let Some(name) = inner.strip_suffix(":children") {
                group = name.trim().to_string();
                section = IniSection::Children;
                ensure_group(&mut data.groups, &group);
            } else {
                group = inner.trim().to_string();
                section = IniSection::Hosts;
                ensure_group(&mut data.groups, &group);
            }
            continue;
        }
        match section {
            IniSection::Vars => {
                for part in host_line_tokens(line) {
                    if let Some((key, value)) = part.split_once('=') {
                        if !key.is_empty() {
                            set_group_var(&mut data.groups, &group, key, value);
                        }
                    }
                }
            }
            IniSection::Children => {
                for part in host_line_tokens(line) {
                    for child in part.split(',') {
                        let child = child.trim();
                        if !child.is_empty() {
                            ensure_group(&mut data.groups, child);
                            add_child(&mut data.groups, &group, child);
                        }
                    }
                }
            }
            IniSection::Hosts => {
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
                ensure_group(&mut data.groups, &group);
                if let Some(existing) = data.hosts.iter_mut().find(|host| host.name == name) {
                    if !existing.groups.iter().any(|item| item == &group) {
                        existing.groups.push(group.clone());
                    }
                    existing.vars.extend(vars);
                } else {
                    data.hosts.push(InventoryHost {
                        name: name.to_string(),
                        groups: vec![group.clone()],
                        vars,
                    });
                }
            }
        }
    }
    data
}

fn ensure_group(groups: &mut Vec<InventoryGroup>, name: &str) {
    if !groups.iter().any(|group| group.name == name) {
        groups.push(InventoryGroup {
            name: name.to_string(),
            ..InventoryGroup::default()
        });
    }
}

fn set_group_var(groups: &mut [InventoryGroup], name: &str, key: &str, value: &str) {
    if let Some(group) = groups.iter_mut().find(|group| group.name == name) {
        group.vars.insert(key.to_string(), value.to_string());
    }
}

fn add_child(groups: &mut [InventoryGroup], name: &str, child: &str) {
    if let Some(group) = groups.iter_mut().find(|group| group.name == name) {
        if !group.children.iter().any(|item| item == child) {
            group.children.push(child.to_string());
        }
    }
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

fn parse_yaml_inventory(body: &str) -> InventoryData {
    let mut data = InventoryData::default();
    let mut group_stack: Vec<(usize, String)> = Vec::new();
    let mut host: Option<String> = None;
    let mut host_indent: Option<usize> = None;
    let mut var_stack: Vec<(usize, String)> = Vec::new();
    let mut hosts_indent: Option<usize> = None;
    let mut vars_indent: Option<usize> = None;
    let mut children_indent: Option<usize> = None;
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
            host = None;
            host_indent = None;
            var_stack.clear();
            hosts_indent = None;
        }
        if vars_indent.is_some_and(|level| indent <= level) {
            vars_indent = None;
        }
        if children_indent.is_some_and(|level| indent <= level) {
            children_indent = None;
        }
        while group_stack
            .last()
            .is_some_and(|(level, _)| *level >= indent)
        {
            group_stack.pop();
        }
        let group = group_stack
            .last()
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| "all".to_string());
        if value.is_empty() && key == "hosts" {
            ensure_group(&mut data.groups, &group);
            host = None;
            hosts_indent = Some(indent);
            continue;
        }
        if value.is_empty() && key == "vars" {
            ensure_group(&mut data.groups, &group);
            vars_indent = Some(indent);
            continue;
        }
        if value.is_empty() && key == "children" {
            ensure_group(&mut data.groups, &group);
            children_indent = Some(indent);
            continue;
        }
        if hosts_indent.is_some() {
            while var_stack.last().is_some_and(|(level, _)| *level >= indent) {
                var_stack.pop();
            }
            if value.is_empty() {
                if host.is_some() && host_indent.is_some_and(|level| indent > level) {
                    var_stack.push((indent, key.to_string()));
                } else {
                    host = Some(key.to_string());
                    host_indent = Some(indent);
                    var_stack.clear();
                    remember_host(&mut data, key, &group);
                }
            } else if let Some(name) = &host {
                let mut full = String::new();
                for (_, part) in &var_stack {
                    full.push_str(part);
                    full.push('.');
                }
                full.push_str(key);
                set_var(&mut data.hosts, name, &full, value);
            }
            continue;
        }
        if vars_indent.is_some() {
            if !value.is_empty() {
                set_group_var(&mut data.groups, &group, key, value);
            }
            continue;
        }
        if children_indent.is_some() {
            if value.is_empty() {
                ensure_group(&mut data.groups, key);
                add_child(&mut data.groups, &group, key);
                group_stack.push((indent, key.to_string()));
            }
            continue;
        }
        if value.is_empty() {
            ensure_group(&mut data.groups, key);
            group_stack.push((indent, key.to_string()));
        }
    }
    data
}

fn remember_host(data: &mut InventoryData, name: &str, group: &str) {
    ensure_group(&mut data.groups, group);
    if let Some(existing) = data.hosts.iter_mut().find(|host| host.name == name) {
        if !existing.groups.iter().any(|item| item == group) {
            existing.groups.push(group.to_string());
        }
    } else {
        data.hosts.push(InventoryHost {
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
    fn computes_effective_variables_for_ini() {
        let body = "\
[all:vars]
ansible_user=ansible
shared=all

[app:children]
web

[app:vars]
app_port=1

[web:vars]
http_port=8080
shared=web

[web]
web-1 ansible_user=web-user shared=host
";
        let data = parse_inventory(body);
        let vars = effective_vars(&data, "web-1");
        let find = |name: &str| vars.iter().find(|item| item.name == name).unwrap();
        assert_eq!(find("ansible_user").value, "web-user");
        assert_eq!(find("ansible_user").origin, "host");
        assert_eq!(find("shared").value, "host");
        assert_eq!(find("http_port").value, "8080");
        assert_eq!(find("http_port").origin, "web");
        assert_eq!(find("app_port").origin, "app");
        assert_eq!(host_groups(&data, "web-1"), vec!["all", "app", "web"]);
        assert_eq!(group_hosts(&data, "app").len(), 1);
    }

    #[test]
    fn child_group_overrides_its_parent() {
        let body = "\
[app:children]
web

[app:vars]
shared=app

[web:vars]
shared=web

[web]
web-1
";
        let data = parse_inventory(body);
        let vars = effective_vars(&data, "web-1");
        let shared = vars.iter().find(|item| item.name == "shared").unwrap();
        assert_eq!(shared.value, "web");
        assert_eq!(shared.origin, "web");
    }

    #[test]
    fn computes_effective_variables_for_yaml() {
        let body = "\
all:
  vars:
    ansible_user: ansible
    shared: all
  children:
    app:
      vars:
        app_port: \"1\"
      children:
        web:
          vars:
            http_port: \"8080\"
            shared: web
          hosts:
            web-1:
              ansible_user: web-user
              shared: host
";
        let data = parse_inventory(body);
        let vars = effective_vars(&data, "web-1");
        let find = |name: &str| vars.iter().find(|item| item.name == name).unwrap();
        assert_eq!(find("ansible_user").value, "web-user");
        assert_eq!(find("ansible_user").origin, "host");
        assert_eq!(find("shared").value, "host");
        assert_eq!(find("http_port").value, "8080");
        assert_eq!(find("http_port").origin, "web");
        assert_eq!(find("app_port").origin, "app");
        assert_eq!(find("ansible_user").origin, "host");
        assert_eq!(host_groups(&data, "web-1"), vec!["all", "app", "web"]);
        assert_eq!(group_hosts(&data, "all").len(), 1);
    }

    #[test]
    fn merges_hosts_from_several_sources() {
        let hosts = merge_inventory(&[
            "[web]\nweb-1\n",
            "[db]\ndb-1\n[web]\nweb-1 ansible_host=10.0.0.8\n",
        ])
        .hosts;
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
