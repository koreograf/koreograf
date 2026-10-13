use std::fs;
use std::path::Path;

use serde::Serialize;

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FoundInventory {
    pub path: String,
    pub format: String,
    pub plugin: Option<String>,
    pub api_endpoint: Option<String>,
    pub group_by: Vec<String>,
    pub query_filters: Vec<String>,
    pub validate_certs: Option<bool>,
    pub virtual_machines: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectContents {
    pub playbooks: Vec<String>,
    pub inventories: Vec<FoundInventory>,
}

pub fn scan_project(root: &Path) -> Result<ProjectContents, String> {
    if !root.is_dir() {
        return Err(format!(
            "Projektpfad {} ist kein Verzeichnis.",
            root.display()
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
            return Err("Projekt enthält zu viele Dateien für einen Scan.".into());
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
        let netbox = crate::netbox::parse_plugin(text);
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
mod tests {
    use super::*;

    #[test]
    fn demo_project_lists_the_playbook_and_the_inventory() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/demo");
        let found = scan_project(&root).unwrap();
        assert_eq!(found.playbooks, vec!["site.yml".to_string()]);
        assert!(found
            .inventories
            .iter()
            .any(|item| item.path == "inventory/hosts.ini" && item.format == "ini"));
        assert!(found.playbooks.iter().all(|path| !path.contains("tasks/")));
    }

    #[test]
    fn classifies_a_netbox_plugin_file() {
        let text = "\
plugin: netbox.netbox.nb_inventory
api_endpoint: https://netbox.example.com
validate_certs: false
group_by:
  - sites
  - device_roles
query_filters:
  - status: active
";
        let found = classify_inventory("netbox.yml", text).unwrap();
        assert_eq!(found.format, "plugin");
        assert_eq!(found.plugin.as_deref(), Some("netbox.netbox.nb_inventory"));
        assert_eq!(
            found.api_endpoint.as_deref(),
            Some("https://netbox.example.com")
        );
        assert_eq!(
            found.group_by,
            vec!["sites".to_string(), "device_roles".to_string()]
        );
        assert_eq!(found.query_filters, vec!["status=active".to_string()]);
        assert_eq!(found.validate_certs, Some(false));
    }
}
