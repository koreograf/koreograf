use std::fs;
use std::path::Path;

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CacheEntry {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct CacheReport {
    pub collections_bytes: u64,
    pub roles_bytes: u64,
    pub tofu_bytes: u64,
    pub collections: Vec<CacheEntry>,
    pub roles: Vec<CacheEntry>,
}

pub fn team_cache(data_dir: &Path, team_id: &str) -> CacheReport {
    let root = data_dir.join("cache").join(team_id);
    let collections_root = root.join("collections");
    CacheReport {
        collections_bytes: dir_size(&collections_root),
        roles_bytes: dir_size(&root.join("roles")),
        tofu_bytes: dir_size(&root.join("tofu")),
        collections: list_collections(&collections_root),
        roles: list_roles(&root.join("roles")),
    }
}

pub fn dir_size(path: &Path) -> u64 {
    fn walk(path: &Path, total: &mut u64) {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                walk(&entry.path(), total);
            } else if let Ok(meta) = entry.metadata() {
                *total += meta.len();
            }
        }
    }
    let mut total = 0;
    walk(path, &mut total);
    total
}

fn list_collections(collections_root: &Path) -> Vec<CacheEntry> {
    let root = collections_root.join("ansible_collections");
    let Ok(namespaces) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for namespace in namespaces.flatten() {
        if !namespace.file_type().map(|k| k.is_dir()).unwrap_or(false) {
            continue;
        }
        let Ok(names) = fs::read_dir(namespace.path()) else {
            continue;
        };
        for name in names.flatten() {
            if !name.file_type().map(|k| k.is_dir()).unwrap_or(false) {
                continue;
            }
            let full = format!(
                "{}.{}",
                namespace.file_name().to_string_lossy(),
                name.file_name().to_string_lossy()
            );
            let version = read_version(&name.path()).unwrap_or_default();
            out.push(CacheEntry {
                name: full,
                version,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn list_roles(root: &Path) -> Vec<CacheEntry> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        if entry.file_type().map(|k| k.is_dir()).unwrap_or(false) {
            out.push(CacheEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                version: String::new(),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn read_version(dir: &Path) -> Option<String> {
    for name in ["galaxy.yml", "GALAXY.yml", "MANIFEST.json"] {
        let Some(text) = fs::read_to_string(dir.join(name)).ok() else {
            continue;
        };
        if name.ends_with(".json") {
            if let Some(version) = json_version(&text) {
                return Some(version);
            }
        } else if let Some(version) = yaml_version(&text) {
            return Some(version);
        }
    }
    None
}

fn yaml_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("version:") else {
            continue;
        };
        let version = rest.trim().trim_matches(['"', '\'']).to_string();
        if !version.is_empty() {
            return Some(version);
        }
    }
    None
}

fn json_version(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    value
        .get("collection_info")
        .and_then(|info| info.get("version"))
        .and_then(|v| v.as_str())
        .or_else(|| value.get("version").and_then(|v| v.as_str()))
        .map(str::to_string)
}
