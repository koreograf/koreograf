use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};
use url::{Host, Url};

use super::model::{
    EffectiveVar, FoundInventory, InventoryData, InventoryGroup, InventoryHost, InventoryVarFile,
    NetboxPreview, NetboxResult, NetboxSource, ProjectContents, DEFAULT_MAX_HOSTS, MASKED_VALUE,
    MAX_HOSTS_LIMIT,
};

const PAGE_SIZE: usize = 100;

/// At most this many `group_vars` and `host_vars` files are read per inventory.
pub const MAX_VAR_FILES: usize = 200;
/// At most this many bytes of `group_vars` and `host_vars` text are stored.
pub const MAX_VAR_BYTES: usize = 1_000_000;
const MAX_VAR_FILE_BYTES: u64 = 256 * 1024;

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

/// Merge inventory texts and apply the adjacent `group_vars` and `host_vars`
/// files in source order. Directory variables override inline inventory
/// variables of the same name, and a later source overrides an earlier one.
pub fn merge_inventory_with_vars(bodies: &[&str], var_files: &[InventoryVarFile]) -> InventoryData {
    let mut data = merge_inventory(bodies);
    apply_inventory_vars(&mut data, var_files);
    data
}

/// Apply parsed `group_vars` and `host_vars` files to one parsed inventory.
///
/// A file applies only to a group or host that the inventory already defines,
/// except that `group_vars/all` creates the implicit `all` group. A vaulted
/// file contributes a masked placeholder instead of variables; its text is
/// never decrypted here.
pub fn apply_inventory_vars(data: &mut InventoryData, var_files: &[InventoryVarFile]) {
    for file in var_files {
        let Some((scope, rest)) = file.path.split_once('/') else {
            continue;
        };
        let Some(entity) = vars_entity(rest) else {
            continue;
        };
        let vaulted = file.body.trim_start().starts_with("$ANSIBLE_VAULT;");
        match scope {
            "group_vars" => {
                if entity == "all" {
                    ensure_group(&mut data.groups, "all");
                }
                let Some(group) = data.groups.iter_mut().find(|group| group.name == entity) else {
                    continue;
                };
                if vaulted {
                    push_unique(&mut group.masked, &file.path);
                } else {
                    group.vars.extend(parse_vars_document(&file.body));
                }
            }
            "host_vars" => {
                let Some(host) = data.hosts.iter_mut().find(|host| host.name == entity) else {
                    continue;
                };
                if vaulted {
                    push_unique(&mut host.masked, &file.path);
                } else {
                    host.vars.extend(parse_vars_document(&file.body));
                }
            }
            _ => {}
        }
    }
}

fn push_unique(items: &mut Vec<String>, value: &str) {
    if !items.iter().any(|item| item == value) {
        items.push(value.to_string());
    }
}

/// The group or host name in `all/accounts.yml` or `all.yml`.
fn vars_entity(rest: &str) -> Option<&str> {
    let mut parts = rest.split('/');
    let first = parts.next()?;
    if first.is_empty() || first.starts_with('.') {
        return None;
    }
    if parts.next().is_some() {
        return Some(first);
    }
    first
        .rsplit_once('.')
        .map(|(name, _)| name)
        .filter(|name| !name.is_empty())
}

/// Read `group_vars/` and `host_vars/` next to a project inventory file.
///
/// The result is bounded by file count and bytes. Symlinks, hidden files,
/// backup files, directories below a group or host directory, and files with
/// another extension are skipped. Ordering is deterministic by path.
pub fn read_inventory_vars(root: &Path, inventory_path: &str) -> Vec<InventoryVarFile> {
    let Ok(root) = root.canonicalize() else {
        return Vec::new();
    };
    let base = match inventory_path.rsplit_once('/') {
        Some((dir, _)) => root.join(dir),
        None => root.clone(),
    };
    let Ok(base) = base.canonicalize() else {
        return Vec::new();
    };
    if !base.starts_with(&root) {
        return Vec::new();
    }
    let mut files = Vec::new();
    let mut bytes = 0usize;
    for scope in ["group_vars", "host_vars"] {
        collect_var_scope(&base, scope, &mut files, &mut bytes);
    }
    files
}

fn collect_var_scope(
    base: &Path,
    scope: &str,
    files: &mut Vec<InventoryVarFile>,
    bytes: &mut usize,
) {
    let dir = base.join(scope);
    let Ok(meta) = fs::symlink_metadata(&dir) else {
        return;
    };
    if !meta.file_type().is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    let dir_names: Vec<String> = entries
        .iter()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if ignored_var_name(&name) {
                return None;
            }
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir())
                .map(|_| name)
        })
        .collect();
    for entry in entries {
        if files.len() >= MAX_VAR_FILES {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if ignored_var_name(&name) {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            collect_var_entity_dir(&entry.path(), scope, &name, files, bytes);
        } else if kind.is_file() && var_extension(&name).is_some() {
            let stem = name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(&name);
            if dir_names.iter().any(|dir| dir == stem) {
                continue;
            }
            let path = format!("{scope}/{name}");
            collect_var_file(&entry.path(), &path, files, bytes);
        }
    }
}

fn collect_var_entity_dir(
    dir: &Path,
    scope: &str,
    entity: &str,
    files: &mut Vec<InventoryVarFile>,
    bytes: &mut usize,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if files.len() >= MAX_VAR_FILES {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if ignored_var_name(&name) || var_extension(&name).is_none() {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_file() {
            continue;
        }
        let path = format!("{scope}/{entity}/{name}");
        collect_var_file(&entry.path(), &path, files, bytes);
    }
}

fn collect_var_file(
    path: &Path,
    stored: &str,
    files: &mut Vec<InventoryVarFile>,
    bytes: &mut usize,
) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if !meta.file_type().is_file() || meta.len() > MAX_VAR_FILE_BYTES {
        return;
    }
    let Ok(body) = fs::read_to_string(path) else {
        return;
    };
    let size = body.len();
    if bytes.saturating_add(size) > MAX_VAR_BYTES {
        return;
    }
    *bytes += size;
    files.push(InventoryVarFile {
        path: stored.to_string(),
        body,
    });
}

fn ignored_var_name(name: &str) -> bool {
    name.starts_with('.') || name.ends_with('~')
}

fn var_extension(name: &str) -> Option<&str> {
    let (_, ext) = name.rsplit_once('.')?;
    match ext {
        "yml" | "yaml" | "json" => Some(ext),
        _ => None,
    }
}

/// JSON encoded `group_vars` and `host_vars` files stored with a source.
pub fn encode_inventory_vars(files: &[InventoryVarFile]) -> String {
    serde_json::to_string(files).unwrap_or_else(|_| "[]".to_string())
}

/// Decode the stored JSON array. A malformed value is treated as no files.
pub fn decode_inventory_vars(raw: &str) -> Vec<InventoryVarFile> {
    let files: Vec<InventoryVarFile> = serde_json::from_str(raw).unwrap_or_default();
    without_shadowed_var_files(files)
}

/// Ansible uses `group_vars/<name>/` and ignores `group_vars/<name>.yml` when
/// both exist. Drop the file form so the stored data matches that lookup.
fn without_shadowed_var_files(files: Vec<InventoryVarFile>) -> Vec<InventoryVarFile> {
    let mut directories: Vec<(String, String)> = Vec::new();
    for file in &files {
        let mut parts = file.path.split('/');
        let (Some(scope), Some(entity)) = (parts.next(), parts.next()) else {
            continue;
        };
        if !matches!(scope, "group_vars" | "host_vars") || entity.is_empty() {
            continue;
        }
        if parts.next().is_some() {
            directories.push((scope.to_string(), entity.to_string()));
        }
    }
    files
        .into_iter()
        .filter(|file| {
            let mut parts = file.path.split('/');
            let (Some(scope), Some(rest)) = (parts.next(), parts.next()) else {
                return true;
            };
            if !matches!(scope, "group_vars" | "host_vars") || parts.next().is_some() {
                return true;
            }
            let Some((entity, _)) = rest.rsplit_once('.') else {
                return true;
            };
            !directories
                .iter()
                .any(|(dir_scope, dir_entity)| dir_scope == scope && dir_entity == entity)
        })
        .collect()
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
    let mut vars = BTreeMap::new();
    for (_, group) in ordered_groups(data, &host_groups(data, host_name)) {
        if let Some(item) = data.groups.iter().find(|item| item.name == group) {
            for (key, value) in &item.vars {
                vars.insert(key.clone(), (value.clone(), group.clone(), false));
            }
            for path in &item.masked {
                vars.insert(
                    path.clone(),
                    (MASKED_VALUE.to_string(), group.clone(), true),
                );
            }
        }
    }
    for (key, value) in &host.vars {
        vars.insert(key.clone(), (value.clone(), "host".into(), false));
    }
    for path in &host.masked {
        vars.insert(
            path.clone(),
            (MASKED_VALUE.to_string(), "host".to_string(), true),
        );
    }
    vars.into_iter()
        .map(|(name, (value, origin, masked))| EffectiveVar {
            name,
            value,
            origin,
            masked,
        })
        .collect()
}

/// Effective variables of a group in Ansible precedence order.
///
/// The group's own directory variables override its parents and `all`, just as
/// they do for a host. A group page shows this inherited view.
pub fn effective_group_vars(data: &InventoryData, group_name: &str) -> Vec<EffectiveVar> {
    if !data.groups.iter().any(|group| group.name == group_name) {
        return Vec::new();
    }
    let parents = parent_map(data);
    let mut names = BTreeSet::new();
    collect_ancestors(group_name, &parents, &mut names);
    if data.groups.iter().any(|group| group.name == "all") {
        names.insert("all".to_string());
    }
    let mut vars = BTreeMap::new();
    for (_, group) in ordered_groups(data, &names.into_iter().collect::<Vec<_>>()) {
        if let Some(item) = data.groups.iter().find(|item| item.name == group) {
            for (key, value) in &item.vars {
                vars.insert(key.clone(), (value.clone(), group.clone(), false));
            }
            for path in &item.masked {
                vars.insert(
                    path.clone(),
                    (MASKED_VALUE.to_string(), group.clone(), true),
                );
            }
        }
    }
    vars.into_iter()
        .map(|(name, (value, origin, masked))| EffectiveVar {
            name,
            value,
            origin,
            masked,
        })
        .collect()
}

fn ordered_groups(data: &InventoryData, groups: &[String]) -> Vec<(usize, String)> {
    let mut ordered: Vec<(usize, String)> = groups
        .iter()
        .map(|name| (group_depth(name, data), name.clone()))
        .collect();
    ordered.sort_by(|left, right| {
        let left_all = left.1 == "all";
        let right_all = right.1 == "all";
        (!left_all, left.0, &left.1).cmp(&(!right_all, right.0, &right.1))
    });
    ordered
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
                        masked: Vec::new(),
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
        let value = yaml_text(value.trim());
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

fn yaml_text(value: &str) -> &str {
    let value = value
        .strip_prefix("!unsafe")
        .map(str::trim_start)
        .unwrap_or(value);
    value.trim_matches('"').trim_matches('\'')
}

/// Read a `group_vars` or `host_vars` YAML or JSON document.
///
/// Top-level names stay top-level; a nested mapping or sequence is rendered as
/// compact JSON so Ansible's replace-by-key precedence survives in the UI.
/// JSON files are read as JSON and YAML files through the small block parser
/// below.
pub fn parse_vars_document(body: &str) -> BTreeMap<String, String> {
    let trimmed = body.trim_start();
    let value = if trimmed.starts_with('{') {
        match serde_json::from_str::<Value>(body) {
            Ok(value) => value,
            Err(_) => {
                let lines = var_lines(body);
                parse_var_block(&lines, 0, 0).0
            }
        }
    } else {
        let lines = var_lines(body);
        let (value, _) = parse_var_block(&lines, 0, 0);
        value
    };
    let mut vars = BTreeMap::new();
    if let Value::Object(map) = value {
        for (name, value) in map {
            vars.insert(name, render_var_value(&value));
        }
    }
    vars
}

fn render_var_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

#[derive(Clone, Copy)]
struct VarLine<'a> {
    indent: usize,
    text: &'a str,
}

fn var_lines(body: &str) -> Vec<VarLine<'_>> {
    body.lines()
        .filter_map(|raw| {
            let code = strip_yaml_comment(raw);
            let text = code.trim();
            if text.is_empty() || text == "---" || text == "..." || text.starts_with('%') {
                return None;
            }
            let indent = code.chars().take_while(|ch| *ch == ' ').count();
            Some(VarLine { indent, text })
        })
        .collect()
}

/// Remove a `#` comment that is not inside single or double quotes.
fn strip_yaml_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match quote {
            Some('"') if ch == '\\' => escaped = true,
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == '#' && (index == 0 || line[..index].ends_with([' ', '\t'])) => {
                return &line[..index];
            }
            None => {}
        }
    }
    line
}

fn parse_var_block(lines: &[VarLine<'_>], start: usize, indent: usize) -> (Value, usize) {
    if start < lines.len() && lines[start].indent == indent && is_var_item(lines[start].text) {
        return parse_var_sequence(lines, start, indent);
    }
    parse_var_map(lines, start, indent)
}

fn is_var_item(text: &str) -> bool {
    text == "-" || text.starts_with("- ")
}

fn parse_var_map(lines: &[VarLine<'_>], mut index: usize, indent: usize) -> (Value, usize) {
    let mut map = serde_json::Map::new();
    while index < lines.len() {
        let line = lines[index];
        if line.indent != indent || is_var_item(line.text) {
            break;
        }
        let Some((key, rest)) = split_var_key(line.text) else {
            index += 1;
            continue;
        };
        let (value, next) = parse_var_value(lines, index + 1, indent, rest);
        map.insert(key, value);
        index = next;
    }
    (Value::Object(map), index)
}

fn parse_var_sequence(lines: &[VarLine<'_>], mut index: usize, indent: usize) -> (Value, usize) {
    let mut items = Vec::new();
    while index < lines.len() {
        let line = lines[index];
        if line.indent != indent || !is_var_item(line.text) {
            break;
        }
        let rest = line.text[1..].trim_start();
        if rest.is_empty() {
            let next = index + 1;
            if next < lines.len() && lines[next].indent > indent {
                let (value, after) = parse_var_block(lines, next, lines[next].indent);
                items.push(value);
                index = after;
            } else {
                items.push(Value::Null);
                index = next;
            }
            continue;
        }
        if split_var_key(rest).is_some() {
            // A mapping item starts on the dash line. Rebuild it as a block at
            // the item's continuation indentation.
            let child_indent = indent + 2;
            let mut sub = vec![VarLine {
                indent: child_indent,
                text: rest,
            }];
            let mut after = index + 1;
            while after < lines.len() && lines[after].indent > indent {
                sub.push(lines[after]);
                after += 1;
            }
            let (value, _) = parse_var_map(&sub, 0, child_indent);
            items.push(value);
            index = after;
        } else {
            items.push(Value::String(parse_var_scalar(rest)));
            index += 1;
        }
    }
    (Value::Array(items), index)
}

fn parse_var_value(
    lines: &[VarLine<'_>],
    index: usize,
    indent: usize,
    rest: &str,
) -> (Value, usize) {
    if rest.is_empty() {
        if index < lines.len() && lines[index].indent > indent {
            return parse_var_block(lines, index, lines[index].indent);
        }
        return (Value::Null, index);
    }
    if rest == "|" || rest == ">" || rest.starts_with("|-") || rest.starts_with(">-") {
        let folded = rest.starts_with('>');
        let mut parts = Vec::new();
        let mut after = index;
        while after < lines.len() && lines[after].indent > indent {
            parts.push(lines[after].text);
            after += 1;
        }
        let text = if folded {
            parts.join(" ")
        } else {
            parts.join("\n")
        };
        return (Value::String(text), after);
    }
    (Value::String(parse_var_scalar(rest)), index)
}

fn split_var_key(text: &str) -> Option<(String, &str)> {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match quote {
            Some('"') if ch == '\\' => escaped = true,
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == ':'
                && (index + 1 == text.len() || text[index + 1..].starts_with(' ')) =>
            {
                let key = parse_var_scalar(text[..index].trim());
                if key.is_empty() {
                    return None;
                }
                return Some((key, text[index + 1..].trim()));
            }
            None => {}
        }
    }
    None
}

fn parse_var_scalar(text: &str) -> String {
    let value = text.trim();
    if value.len() >= 2 {
        let first = value.as_bytes()[0];
        let last = value.as_bytes()[value.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
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
            masked: Vec::new(),
        });
    }
}

fn set_var(hosts: &mut [InventoryHost], name: &str, key: &str, value: &str) {
    if let Some(host) = hosts.iter_mut().find(|host| host.name == name) {
        host.vars.insert(key.to_string(), value.to_string());
    }
}

struct NbHost {
    name: String,
    ansible_host: Option<String>,
    netbox_id: Option<i64>,
    groups: Vec<String>,
    group_keys: Vec<(String, String)>,
    vars: BTreeMap<String, Value>,
    dropped_vars: Vec<String>,
}

/// A URL that is not a usable NetBox address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EndpointError {
    /// Not an `http` or `https` URL, or one of the old size or character rules
    /// failed.
    Invalid,
    /// The literal address, or a resolved address, is a loopback, link-local,
    /// metadata, unspecified, multicast, or broadcast target.
    Blocked,
}

const ENDPOINT_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

/// AWS exposes its IPv6 instance metadata service on this unique-local
/// address. The rest of `fc00::/7` stays available for an internal NetBox.
const AWS_IPV6_METADATA: Ipv6Addr = Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254);
const NAT64_PREFIX: [u8; 12] = [0x00, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0];

/// True when the URL passes the strict address policy. DNS names are resolved
/// with a bounded lookup; a name that does not resolve now is left to the
/// request so an offline setup can still save its credential.
#[cfg(test)]
pub async fn valid_endpoint(url: &str) -> bool {
    endpoint_error_with(url, false).await.is_ok()
}

/// The synchronous part of the check. Credential validation keeps this in its
/// sync secret check and runs the address check separately.
pub fn valid_endpoint_syntax(url: &str) -> bool {
    parse_endpoint(url).is_some()
}

/// The HTTP request policy. The `test-helpers` feature allows a literal
/// loopback address so the integration tests can reach their in-process mock
/// server. It never allows a hostname such as `localhost`, a resolved
/// loopback address, or a link-local target, and it is absent from release
/// builds.
pub(crate) async fn request_endpoint_error(url: &str) -> Result<(), EndpointError> {
    endpoint_error_with(url, cfg!(any(test, feature = "test-helpers"))).await
}

/// The request policy with the keyed message the API returns.
pub(crate) async fn check_request_endpoint(url: &str) -> Result<(), String> {
    match request_endpoint_error(url).await {
        Ok(()) => Ok(()),
        Err(EndpointError::Blocked) => Err("netbox_endpoint_is_blocked".into()),
        Err(EndpointError::Invalid) => Err("the_netbox_address_must_be_http_or_https".into()),
    }
}

async fn endpoint_error_with(url: &str, allow_loopback_literal: bool) -> Result<(), EndpointError> {
    let parsed = parse_endpoint(url).ok_or(EndpointError::Invalid)?;
    match parsed.host().ok_or(EndpointError::Invalid)? {
        Host::Ipv4(ip) => {
            if allow_loopback_literal && ip.is_loopback() {
                return Ok(());
            }
            if blocked_ip(IpAddr::V4(ip)) {
                Err(EndpointError::Blocked)
            } else {
                Ok(())
            }
        }
        Host::Ipv6(ip) => {
            if allow_loopback_literal && ip.is_loopback() {
                return Ok(());
            }
            if blocked_ip(IpAddr::V6(ip)) {
                Err(EndpointError::Blocked)
            } else {
                Ok(())
            }
        }
        Host::Domain(domain) => {
            let port = parsed
                .port_or_known_default()
                .ok_or(EndpointError::Invalid)?;
            check_domain(domain, port).await
        }
    }
}

fn parse_endpoint(url: &str) -> Option<Url> {
    if url.len() >= 300
        || url.contains("..")
        || url.chars().any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return None;
    }
    let parsed = Url::parse(url).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    parsed.host()?;
    Some(parsed)
}

fn pinned_host(url: &str) -> Option<(String, u16)> {
    let parsed = Url::parse(url).ok()?;
    let port = parsed.port_or_known_default()?;
    match parsed.host()? {
        Host::Domain(domain) => Some((domain.to_string(), port)),
        Host::Ipv4(_) | Host::Ipv6(_) => None,
    }
}

async fn check_domain(domain: &str, port: u16) -> Result<(), EndpointError> {
    let name = domain.trim_end_matches('.').to_ascii_lowercase();
    // RFC 6761 reserves `localhost` and its subdomains for loopback.
    if name == "localhost" || name.ends_with(".localhost") {
        return Err(EndpointError::Blocked);
    }
    let lookup = tokio::time::timeout(
        ENDPOINT_LOOKUP_TIMEOUT,
        tokio::net::lookup_host((name.as_str(), port)),
    )
    .await;
    let Ok(Ok(addresses)) = lookup else {
        return Ok(());
    };
    for address in addresses {
        if blocked_ip(address.ip()) {
            return Err(EndpointError::Blocked);
        }
    }
    Ok(())
}

fn blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => blocked_ipv4(ip),
        IpAddr::V6(ip) => blocked_ipv6(ip),
    }
}

fn blocked_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        // `0.0.0.0/8` is this network and never a public target.
        || ip.octets()[0] == 0
        // Shared address space, including the metadata service on 100.100.100.200.
        || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
}

fn blocked_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback()
        || ip.is_unicast_link_local()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip == AWS_IPV6_METADATA
    {
        return true;
    }
    // IPv4-mapped (`::ffff:127.0.0.1`), IPv4-compatible (`::127.0.0.1`),
    // NAT64 (`64:ff9b::/96`), and 6to4 (`2002::/16`) forms carry an IPv4
    // address that must pass the same check.
    ip.to_ipv4().is_some_and(blocked_ipv4) || embedded_ipv4(ip).is_some_and(blocked_ipv4)
}

fn embedded_ipv4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let octets = ip.octets();
    if octets[..12] == NAT64_PREFIX {
        return Some(Ipv4Addr::new(
            octets[12], octets[13], octets[14], octets[15],
        ));
    }
    if octets[0] == 0x20 && octets[1] == 0x02 {
        return Some(Ipv4Addr::new(octets[2], octets[3], octets[4], octets[5]));
    }
    None
}

pub fn source_json(source: &NetboxSource) -> Value {
    json!({
        "validate_certs": source.validate_certs,
        "group_by": source.group_by,
        "group_names_raw": source.group_names_raw,
        "load_config_context": source.load_config_context,
        "max_hosts": source.max_hosts,
        "query": source.query,
        "virtual_machines": source.virtual_machines,
    })
}

pub fn source_json_with_collisions(source: &NetboxSource, collisions: &[String]) -> Value {
    let mut value = source_json(source);
    value["collisions"] = json!(collisions);
    value
}

pub fn loads_config_context(source_json: &str) -> bool {
    serde_json::from_str::<Value>(source_json)
        .ok()
        .and_then(|value| value.get("load_config_context").and_then(Value::as_bool))
        == Some(true)
}

pub fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() < 300
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

pub fn parse_plugin(text: &str) -> Option<NetboxSource> {
    let plugin = scalar(text, "plugin")?;
    if plugin != "netbox.netbox.nb_inventory" && plugin != "netbox" {
        return None;
    }
    let api_endpoint = scalar(text, "api_endpoint").unwrap_or_default();
    Some(NetboxSource {
        api_endpoint: api_endpoint.trim_end_matches('/').to_string(),
        validate_certs: flag(text, "validate_certs").unwrap_or(true),
        group_by: list_items(text, "group_by"),
        group_names_raw: flag(text, "group_names_raw").unwrap_or(false),
        load_config_context: flag(text, "load_config_context").unwrap_or(false),
        max_hosts: scalar(text, "max_hosts")
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(DEFAULT_MAX_HOSTS),
        query: query_items(text, "query_filters"),
        virtual_machines: flag(text, "virtual_machines").unwrap_or(true),
    })
}

pub fn parse_filters(lines: &[String]) -> Result<Vec<(String, String)>, String> {
    let mut query = Vec::new();
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .or_else(|| line.split_once(':'))
            .ok_or_else(|| {
                crate::error::message("filter_line_needs_key_value", &[("line", line)])
            })?;
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if key.is_empty() || value.is_empty() || key.contains(' ') {
            return Err(crate::error::message(
                "filter_line_is_invalid",
                &[("line", line)],
            ));
        }
        query.push((key.to_string(), value.to_string()));
    }
    Ok(query)
}

pub async fn fetch(
    source: &NetboxSource,
    token: &str,
    proxy: &crate::features::operations::ProxySettings,
) -> Result<NetboxResult, String> {
    check_request_endpoint(&source.api_endpoint).await?;
    if !valid_token(token) {
        return Err("the_netbox_token_is_invalid".into());
    }
    if source.max_hosts == 0 || source.max_hosts > MAX_HOSTS_LIMIT {
        let max = MAX_HOSTS_LIMIT.to_string();
        return Err(crate::error::message(
            "netbox_host_limit_is_out_of_range",
            &[("max", &max)],
        ));
    }
    for key in &source.group_by {
        if !known_group(key) {
            return Err(crate::error::message(
                "grouping_key_is_not_supported_allowed_values_are",
                &[("key", key)],
            ));
        }
    }
    let client = client(source, proxy).await?;
    let mut hosts = Vec::new();
    let devices = PageQuery {
        client: &client,
        endpoint: &source.api_endpoint,
        token,
        path: "/api/dcim/devices/",
        query: &source.query,
        is_virtual: false,
        group_by: &source.group_by,
        group_names_raw: source.group_names_raw,
        load_config_context: source.load_config_context,
        max_hosts: source.max_hosts,
    };
    collect(&devices, &mut hosts).await?;
    if source.virtual_machines {
        let machines = PageQuery {
            path: "/api/virtualization/virtual-machines/",
            is_virtual: true,
            ..devices
        };
        collect(&machines, &mut hosts).await?;
    }
    let collisions = collisions(&hosts);
    let mut dropped_vars: Vec<String> = hosts
        .iter()
        .flat_map(|host| host.dropped_vars.iter().cloned())
        .collect();
    dropped_vars.sort();
    dropped_vars.dedup();
    let body = if source.load_config_context {
        render_yaml(&hosts)
    } else {
        render_ini(&hosts)
    };
    Ok(NetboxResult {
        body,
        vars_json: "[]".to_string(),
        collisions,
        dropped_vars,
    })
}

#[cfg(test)]
pub async fn fetch_ini(
    source: &NetboxSource,
    token: &str,
    proxy: &crate::features::operations::ProxySettings,
) -> Result<String, String> {
    Ok(fetch(source, token, proxy).await?.body)
}

pub fn preview(body: &str) -> NetboxPreview {
    let data = parse_inventory(body);
    let mut groups = BTreeMap::new();
    for host in &data.hosts {
        for group in &host.groups {
            if group != "all" && group != "ungrouped" {
                *groups.entry(group.clone()).or_insert(0usize) += 1;
            }
        }
    }
    let sample_host = data
        .hosts
        .first()
        .map(|host| (host.name.clone(), host.vars.clone()));
    NetboxPreview {
        host_count: data.hosts.len(),
        groups: groups.into_iter().collect(),
        sample_host,
    }
}

pub async fn test_connection(
    source: &NetboxSource,
    token: &str,
    proxy: &crate::features::operations::ProxySettings,
) -> Result<String, String> {
    check_request_endpoint(&source.api_endpoint).await?;
    if !valid_token(token) {
        return Err("the_netbox_token_is_invalid".into());
    }
    crate::tls::warn_netbox(&source.api_endpoint, !source.validate_certs);
    let client = client(source, proxy).await?;
    let url = format!("{}/api/status/", source.api_endpoint.trim_end_matches('/'));
    let body = get_json(&client, &url, token).await?;
    Ok(body
        .get("netbox-version")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

async fn client(
    source: &NetboxSource,
    proxy: &crate::features::operations::ProxySettings,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none());
    // Pin a DNS name to the addresses that passed the policy, so a second
    // lookup cannot swap in a loopback or link-local target between the check
    // and the request. A proxy still makes its own connection.
    if let Some((host, port)) = pinned_host(&source.api_endpoint) {
        // With a proxy the proxy resolves the name itself, so only a lookup
        // that succeeds is useful here. Without a proxy, fail closed: a name
        // that does not resolve cannot have passed the policy safely.
        let proxied = proxy.is_set() && proxy.no_proxy.is_empty();
        let lookup = tokio::time::timeout(
            ENDPOINT_LOOKUP_TIMEOUT,
            tokio::net::lookup_host((host.as_str(), port)),
        )
        .await;
        let addresses: Vec<SocketAddr> = match lookup {
            Ok(Ok(addresses)) => addresses.collect(),
            Ok(Err(_)) | Err(_) if proxied => Vec::new(),
            Ok(Err(_)) => {
                return Err(crate::error::message(
                    "netbox_is_not_reachable_err",
                    &[("err", "the NetBox address could not be resolved")],
                ));
            }
            Err(_) => {
                return Err(crate::error::message(
                    "netbox_is_not_reachable_err",
                    &[("err", "the NetBox address lookup timed out")],
                ));
            }
        };
        if addresses.is_empty() && !proxied {
            return Err(crate::error::message(
                "netbox_is_not_reachable_err",
                &[("err", "the NetBox address could not be resolved")],
            ));
        }
        if addresses.iter().any(|address| blocked_ip(address.ip())) {
            return Err("netbox_endpoint_is_blocked".into());
        }
        if !addresses.is_empty() {
            builder = builder.resolve_to_addrs(&host, &addresses);
        }
    }
    if source.validate_certs {
        match crate::tls::reqwest_roots() {
            Ok(certs) => {
                for cert in certs {
                    builder = builder.add_root_certificate(cert);
                }
            }
            Err(error) => {
                tracing::warn!(endpoint = %source.api_endpoint, %error, "NetBox TLS setup failed");
                return Err(error);
            }
        }
    } else {
        builder = builder.danger_accept_invalid_certs(true);
    }
    proxy
        .apply_reqwest(builder)?
        .build()
        .map_err(|err| err.to_string())
}

fn known_group(key: &str) -> bool {
    matches!(
        key,
        "sites"
            | "site"
            | "location"
            | "tenants"
            | "tenant"
            | "tags"
            | "tag"
            | "device_roles"
            | "role"
            | "platforms"
            | "platform"
            | "manufacturers"
            | "manufacturer"
            | "device_types"
            | "device_type"
            | "status"
            | "cluster"
            | "region"
            | "site_group"
            | "is_virtual"
    )
}

struct PageQuery<'a> {
    client: &'a reqwest::Client,
    endpoint: &'a str,
    token: &'a str,
    path: &'a str,
    query: &'a [(String, String)],
    is_virtual: bool,
    group_by: &'a [String],
    group_names_raw: bool,
    load_config_context: bool,
    max_hosts: usize,
}

async fn collect(page: &PageQuery<'_>, hosts: &mut Vec<NbHost>) -> Result<(), String> {
    let mut url = Some(page_url(page.endpoint, page.path, page.query));
    let mut seen = std::collections::BTreeSet::new();
    let mut pages = 0usize;
    let max_pages = page.max_hosts.div_ceil(PAGE_SIZE).max(1);
    while let Some(current) = url.take() {
        pages += 1;
        if pages > max_pages {
            return Err(limit_error(page.max_hosts));
        }
        if !seen.insert(current.clone()) {
            break;
        }
        let body = get_json(page.client, &current, page.token).await?;
        let results = body
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| "the_netbox_response_has_no_results".to_string())?;
        for item in results {
            if hosts.len() >= page.max_hosts {
                return Err(limit_error(page.max_hosts));
            }
            if let Some(host) = host_from_json(
                item,
                page.is_virtual,
                page.group_by,
                page.group_names_raw,
                page.load_config_context,
            ) {
                hosts.push(host);
            }
        }
        if results.len() < PAGE_SIZE {
            break;
        }
        url = match body.get("next") {
            Some(Value::String(next)) if !next.is_empty() => {
                if origin(page.endpoint) != origin(next) {
                    return Err("the_netbox_response_points_at_another_host".into());
                }
                Some(next.clone())
            }
            _ => None,
        };
    }
    Ok(())
}

fn limit_error(limit: usize) -> String {
    crate::error::message(
        "the_netbox_host_limit_of_was_reached_raise_max_hosts",
        &[("limit", &limit.to_string())],
    )
}

fn host_from_json(
    value: &Value,
    is_virtual: bool,
    group_by: &[String],
    group_names_raw: bool,
    load_config_context: bool,
) -> Option<NbHost> {
    let name = sanitize(value.get("name")?.as_str()?);
    if name.is_empty() {
        return None;
    }
    let mut groups = Vec::new();
    let mut group_keys = Vec::new();
    for key in group_by {
        for group in group_names(key, value, is_virtual, group_names_raw) {
            if !groups.contains(&group) {
                groups.push(group.clone());
            }
            group_keys.push((group, key.clone()));
        }
    }
    let mut dropped_vars = Vec::new();
    let vars = if load_config_context {
        value
            .get("config_context")
            .and_then(Value::as_object)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|(key, value)| {
                        if config_context_key_allowed(key) {
                            Some((key.clone(), value.clone()))
                        } else {
                            dropped_vars.push(key.clone());
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        BTreeMap::new()
    };
    Some(NbHost {
        name,
        ansible_host: primary_ip(value),
        netbox_id: value.get("id").and_then(Value::as_i64),
        groups,
        group_keys,
        vars,
        dropped_vars,
    })
}

fn config_context_key_allowed(key: &str) -> bool {
    !key.to_ascii_lowercase().starts_with("ansible_")
}

fn group_names(key: &str, value: &Value, is_virtual: bool, raw: bool) -> Vec<String> {
    let slugs = match key {
        "sites" | "site" => slugs_of(value.get("site")),
        "location" => slugs_of(value.get("location")),
        "tenants" | "tenant" => slugs_of(value.get("tenant")),
        "tags" | "tag" => slugs_of(value.get("tags")),
        "device_roles" | "role" => slugs_of(value.get("role").or_else(|| value.get("device_role"))),
        "platforms" | "platform" => slugs_of(value.get("platform")),
        "manufacturers" | "manufacturer" => slugs_of(
            value
                .get("device_type")
                .and_then(|item| item.get("manufacturer"))
                .or_else(|| value.get("manufacturer")),
        ),
        "device_types" | "device_type" => slugs_of(value.get("device_type")),
        "status" => status_slug(value.get("status")),
        "cluster" => slugs_of(value.get("cluster")),
        "region" => slugs_of(
            value
                .get("site")
                .and_then(|item| item.get("region"))
                .or_else(|| value.get("region")),
        ),
        "site_group" => slugs_of(
            value
                .get("site")
                .and_then(|item| item.get("group"))
                .or_else(|| value.get("group")),
        ),
        "is_virtual" => vec![if is_virtual { "true" } else { "false" }.to_string()],
        _ => Vec::new(),
    };
    slugs
        .into_iter()
        .filter_map(|slug| {
            let slug = sanitize_group(&slug);
            if slug.is_empty() {
                None
            } else if raw {
                Some(slug)
            } else {
                Some(format!("{key}_{slug}"))
            }
        })
        .collect()
}

fn collisions(hosts: &[NbHost]) -> Vec<String> {
    let mut keys: BTreeMap<&str, std::collections::BTreeSet<&str>> = BTreeMap::new();
    for host in hosts {
        for (group, key) in &host.group_keys {
            keys.entry(group).or_default().insert(key);
        }
    }
    keys.into_iter()
        .filter(|(_, sources)| sources.len() > 1)
        .map(|(group, sources)| {
            format!(
                "{group} ({})",
                sources.into_iter().collect::<Vec<_>>().join(", ")
            )
        })
        .collect()
}

fn slugs_of(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items.iter().filter_map(slug_of).collect(),
        Some(other) => slug_of(other).into_iter().collect(),
        None => Vec::new(),
    }
}

fn slug_of(value: &Value) -> Option<String> {
    value
        .get("slug")
        .and_then(Value::as_str)
        .or_else(|| value.get("name").and_then(Value::as_str))
        .map(str::to_string)
}

fn status_slug(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    if let Some(text) = value.as_str() {
        return vec![text.to_string()];
    }
    value
        .get("value")
        .and_then(Value::as_str)
        .map(|item| vec![item.to_string()])
        .unwrap_or_default()
}

fn primary_ip(value: &Value) -> Option<String> {
    // NetBox always sends `primary_ip4`. JSON null means there is no IPv4
    // primary, so a missing or empty address must not stop the search.
    // A present value that is not an IP address stops the search.
    let device = value.get("name").and_then(Value::as_str).unwrap_or("");
    for key in ["primary_ip4", "primary_ip", "primary_ip6"] {
        match address_host(value.get(key)) {
            HostAddress::Missing => continue,
            HostAddress::Invalid => {
                tracing::warn!(
                    device,
                    "NetBox address is not an IP address, so ansible_host was left unset"
                );
                return None;
            }
            HostAddress::Ip(address) => return Some(address),
        }
    }
    None
}

enum HostAddress {
    Missing,
    Invalid,
    Ip(String),
}

fn address_host(value: Option<&Value>) -> HostAddress {
    let Some(address) = value
        .and_then(|item| item.get("address"))
        .and_then(Value::as_str)
    else {
        return HostAddress::Missing;
    };
    let host = address.split_once('/').map_or(address, |(host, _)| host);
    let host = host.trim();
    if host.is_empty() {
        return HostAddress::Missing;
    }
    match host.parse::<IpAddr>() {
        Ok(address) => HostAddress::Ip(address.to_string()),
        Err(_) => HostAddress::Invalid,
    }
}

fn render_ini(hosts: &[NbHost]) -> String {
    let mut out = String::from("[all]\n");
    for host in hosts {
        out.push_str(&host.name);
        if let Some(address) = &host.ansible_host {
            out.push_str(" ansible_host=");
            out.push_str(address);
        }
        if let Some(id) = host.netbox_id {
            out.push_str(&format!(" netbox_id={id}"));
        }
        out.push('\n');
    }
    let mut grouped: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for host in hosts {
        for group in &host.groups {
            grouped.entry(group).or_default().push(&host.name);
        }
    }
    for (group, names) in grouped {
        out.push('\n');
        out.push('[');
        out.push_str(group);
        out.push_str("]\n");
        for name in names {
            out.push_str(name);
            out.push('\n');
        }
    }
    out
}

fn render_yaml(hosts: &[NbHost]) -> String {
    let mut out = String::from("all:\n  hosts:\n");
    for host in hosts {
        out.push_str("    ");
        out.push_str(&yaml_key(&host.name));
        out.push_str(":\n");
        let mut vars = host.vars.clone();
        if let Some(address) = &host.ansible_host {
            vars.insert("ansible_host".into(), Value::String(address.clone()));
        }
        if let Some(id) = host.netbox_id {
            vars.insert("netbox_id".into(), Value::Number(id.into()));
        }
        write_yaml_vars(&mut out, 6, &vars, &host.vars, false);
    }
    let mut grouped: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for host in hosts {
        for group in &host.groups {
            grouped.entry(group).or_default().push(&host.name);
        }
    }
    if !grouped.is_empty() {
        out.push_str("  children:\n");
        for (group, names) in grouped {
            out.push_str("    ");
            out.push_str(&yaml_key(group));
            out.push_str(":\n      hosts:\n");
            for name in names {
                out.push_str("        ");
                out.push_str(&yaml_key(name));
                out.push_str(":\n");
            }
        }
    }
    out
}

fn write_yaml_vars(
    out: &mut String,
    indent: usize,
    vars: &BTreeMap<String, Value>,
    config: &BTreeMap<String, Value>,
    nested: bool,
) {
    for (key, value) in vars {
        out.push_str(&" ".repeat(indent));
        out.push_str(&yaml_key(key));
        match value {
            Value::Object(items) if !items.is_empty() => {
                out.push_str(":\n");
                let nested_items = items
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                write_yaml_vars(out, indent + 2, &nested_items, config, true);
            }
            _ => {
                out.push_str(": ");
                let unsafe_tag = nested
                    || config.get(key).is_some_and(|value| {
                        matches!(value, Value::String(_) | Value::Array(_) | Value::Object(_))
                    });
                out.push_str(&yaml_value(value, unsafe_tag));
                out.push('\n');
            }
        }
    }
}

fn yaml_key(value: &str) -> String {
    if plain_yaml(value) {
        value.to_string()
    } else {
        serde_json::to_string(value).unwrap_or_else(|_| format!("{value:?}"))
    }
}

fn yaml_value(value: &Value, unsafe_tag: bool) -> String {
    match value {
        Value::String(text) if unsafe_tag => format!("!unsafe {}", yaml_key(text)),
        Value::String(text) => yaml_key(text),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => "null".into(),
        Value::Array(_) | Value::Object(_) => {
            let text = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
            let quoted = yaml_key(&text);
            if unsafe_tag {
                format!("!unsafe {quoted}")
            } else {
                quoted
            }
        }
    }
}

fn plain_yaml(value: &str) -> bool {
    !value.is_empty()
        && !value.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        && !matches!(
            value.to_ascii_lowercase().as_str(),
            "null" | "true" | "false" | "yes" | "no" | "on" | "off" | ".inf" | "-.inf" | ".nan"
        )
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '/' | '-'))
}

fn sanitize(value: &str) -> String {
    sanitize_with(value, |ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')
    })
}

// Ansible warns when a group name contains `-` or `.`. Host names keep both.
fn sanitize_group(value: &str) -> String {
    sanitize_with(value, |ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn sanitize_with(value: &str, keep: impl Fn(char) -> bool) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if keep(ch) {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_string()
}

fn page_url(endpoint: &str, path: &str, query: &[(String, String)]) -> String {
    let mut parts = vec!["limit=100".to_string()];
    for (key, value) in query {
        parts.push(format!("{}={}", encode(key), encode(value)));
    }
    format!(
        "{}{}?{}",
        endpoint.trim_end_matches('/'),
        path,
        parts.join("&")
    )
}

fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    if host.is_empty() {
        None
    } else {
        Some(format!("{scheme}://{host}"))
    }
}

async fn get_json(client: &reqwest::Client, url: &str, token: &str) -> Result<Value, String> {
    let response = client
        .get(url)
        .header("Authorization", format!("Token {token}"))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|err| {
            let detail = err.to_string();
            crate::error::message("netbox_is_not_reachable_err", &[("err", &detail)])
        })?;
    let status = response.status();
    let text = response.text().await.map_err(|err| {
        let detail = err.to_string();
        crate::error::message("netbox.response_unreadable", &[("detail", &detail)])
    })?;
    if !status.is_success() {
        let snippet: String = text.chars().take(240).collect();
        return Err(status_error(status, &snippet));
    }
    serde_json::from_str(&text).map_err(|_| "the_netbox_response_is_not_json".to_string())
}

fn status_error(status: reqwest::StatusCode, snippet: &str) -> String {
    let code = status.as_u16().to_string();
    match status.as_u16() {
        401 | 403 => crate::error::message("netbox.token_rejected", &[("status", &code)]),
        404 => crate::error::message("netbox.address_not_found", &[("status", &code)]),
        _ => crate::error::message(
            "netbox.bad_status",
            &[("status", &code), ("snippet", snippet)],
        ),
    }
}

fn scalar(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if let Some(value) = line.strip_prefix(&prefix) {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn flag(text: &str, key: &str) -> Option<bool> {
    match scalar(text, key)?.to_ascii_lowercase().as_str() {
        "true" | "yes" | "ja" => Some(true),
        "false" | "no" | "nein" => Some(false),
        _ => None,
    }
}

fn list_items(text: &str, key: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut inside = false;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with(&format!("{key}:")) {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        if raw.trim_start().len() == raw.len() && line.ends_with(':') {
            break;
        }
        if let Some(value) = line.strip_prefix("- ") {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() && !value.contains(':') {
                items.push(value.to_string());
            }
        }
    }
    items
}

fn query_items(text: &str, key: &str) -> Vec<(String, String)> {
    let mut items = Vec::new();
    let mut inside = false;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with(&format!("{key}:")) {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        if raw.trim_start().len() == raw.len() && line.ends_with(':') {
            break;
        }
        let Some(value) = line.strip_prefix("- ") else {
            continue;
        };
        if let Some((left, right)) = value.split_once(':').or_else(|| value.split_once('=')) {
            let left = left.trim();
            let right = right.trim().trim_matches('"').trim_matches('\'');
            if !left.is_empty() && !right.is_empty() {
                items.push((left.to_string(), right.to_string()));
            }
        }
    }
    items
}

const SKIP_DIRS: &[&str] = &[
    ".git",
    "group_vars",
    "host_vars",
    "vars",
    "tasks",
    "handlers",
    "meta",
    "defaults",
    "molecule",
    "files",
    "templates",
    "collections",
    ".terraform",
    "node_modules",
    "target",
];
const MAX_FILES: usize = 4_000;
const MAX_READ: u64 = 256 * 1024;

pub fn scan_project(root: &Path) -> Result<ProjectContents, String> {
    if !root.is_dir() {
        let shown = root.display().to_string();
        return Err(crate::error::message(
            "project.path_not_directory",
            &[("path", &shown)],
        ));
    }
    let mut playbooks = Vec::new();
    let mut inventories = Vec::new();
    let mut seen = 0usize;
    walk(root, root, &mut seen, &mut playbooks, &mut inventories)?;
    note_ansible_cfg(root, &mut inventories)?;
    playbooks.sort();
    playbooks.dedup();
    inventories.sort_by(|left, right| left.path.cmp(&right.path));
    inventories.dedup_by(|left, right| left.path == right.path);
    Ok(ProjectContents {
        playbooks,
        inventories,
    })
}

fn walk(
    root: &Path,
    dir: &Path,
    seen: &mut usize,
    playbooks: &mut Vec<String>,
    inventories: &mut Vec<FoundInventory>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|err| err.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let name = entry.file_name();
        let label = name.to_string_lossy();
        if label.starts_with('.') {
            continue;
        }
        let file_type = entry.file_type().map_err(|err| err.to_string())?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if SKIP_DIRS.contains(&label.as_ref()) {
                continue;
            }
            walk(root, &entry.path(), seen, playbooks, inventories)?;
            continue;
        }
        *seen += 1;
        if *seen > MAX_FILES {
            return Err("the_project_has_too_many_files_to_scan".into());
        }
        let full = entry.path();
        let Ok(rel) = full.strip_prefix(root) else {
            continue;
        };
        let path = rel
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if path.split('/').any(|part| SKIP_DIRS.contains(&part)) {
            continue;
        }
        let meta = entry.metadata().map_err(|err| err.to_string())?;
        if meta.len() > MAX_READ {
            continue;
        }
        let text = fs::read_to_string(entry.path()).unwrap_or_default();
        if looks_like_playbook(&label, &text) {
            playbooks.push(path);
            continue;
        }
        if let Some(found) = classify_inventory(&path, &text) {
            inventories.push(found);
        }
    }
    Ok(())
}

pub fn looks_like_playbook(name: &str, text: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "requirements.yml"
            | "requirements.yaml"
            | "galaxy.yml"
            | "galaxy.yaml"
            | "argument_specs.yml"
            | "argument_specs.yaml"
    ) {
        return false;
    }
    if !lower.ends_with(".yml") && !lower.ends_with(".yaml") {
        return false;
    }
    let mut started = false;
    let mut has_hosts = false;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line == "---" {
            continue;
        }
        if !started {
            if !line.starts_with('-') {
                return false;
            }
            started = true;
        }
        let head = line.trim_start_matches('-').trim_start();
        if head.starts_with("hosts:") {
            has_hosts = true;
        }
    }
    started && has_hosts
}

pub fn classify_inventory(path: &str, text: &str) -> Option<FoundInventory> {
    if looks_like_playbook(file_name(path), text) {
        return None;
    }
    if let Some(plugin) = plugin_name(text) {
        let netbox = parse_plugin(text);
        return Some(FoundInventory {
            path: path.to_string(),
            format: "plugin".into(),
            plugin: Some(plugin),
            api_endpoint: netbox.as_ref().map(|item| item.api_endpoint.clone()),
            group_by: netbox
                .as_ref()
                .map(|item| item.group_by.clone())
                .unwrap_or_default(),
            query_filters: netbox
                .as_ref()
                .map(|item| {
                    item.query
                        .iter()
                        .map(|(key, value)| format!("{key}={value}"))
                        .collect()
                })
                .unwrap_or_default(),
            validate_certs: netbox.as_ref().map(|item| item.validate_certs),
            virtual_machines: netbox.as_ref().map(|item| item.virtual_machines),
        });
    }
    if inventory_path(path) {
        let format = if path.ends_with(".ini")
            || text.lines().any(|line| line.trim_start().starts_with('['))
        {
            "ini"
        } else {
            "yaml"
        };
        return Some(FoundInventory {
            path: path.to_string(),
            format: format.into(),
            plugin: None,
            api_endpoint: None,
            group_by: Vec::new(),
            query_filters: Vec::new(),
            validate_certs: None,
            virtual_machines: None,
        });
    }
    None
}

fn inventory_path(path: &str) -> bool {
    let name = file_name(path);
    if matches!(
        name,
        "hosts"
            | "hosts.ini"
            | "hosts.yml"
            | "hosts.yaml"
            | "inventory"
            | "inventory.ini"
            | "inventory.yml"
            | "inventory.yaml"
    ) {
        return true;
    }
    let parent = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    let dir = parent.rsplit('/').next().unwrap_or(parent);
    (dir == "inventory" || dir == "inventories")
        && (name.ends_with(".ini")
            || name.ends_with(".yml")
            || name.ends_with(".yaml")
            || name.ends_with(".cfg"))
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn plugin_name(text: &str) -> Option<String> {
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if let Some(value) = line.strip_prefix("plugin:") {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn note_ansible_cfg(root: &Path, inventories: &mut Vec<FoundInventory>) -> Result<(), String> {
    let path = root.join("ansible.cfg");
    if !path.is_file() {
        return Ok(());
    }
    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let mut in_defaults = false;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_defaults = line.eq_ignore_ascii_case("[defaults]");
            continue;
        }
        if !in_defaults {
            continue;
        }
        let Some(value) = line.strip_prefix("inventory") else {
            continue;
        };
        let Some(value) = value.trim().strip_prefix('=') else {
            continue;
        };
        for part in value.split(',') {
            let rel = part.trim().trim_matches('"');
            if rel.is_empty() || rel.starts_with('/') || rel.split('/').any(|item| item == "..") {
                continue;
            }
            let full = root.join(rel);
            if !full.is_file() || inventories.iter().any(|item| item.path == rel) {
                continue;
            }
            let body = fs::read_to_string(&full).unwrap_or_default();
            if let Some(found) = classify_inventory(rel, &body) {
                inventories.push(found);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
