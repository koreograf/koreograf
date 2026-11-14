use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use serde_json::Value;

pub const LANGUAGES: [&str; 2] = ["de", "en"];

#[derive(Default)]
pub struct Catalogs {
    de: Option<Vec<u8>>,
    en: Option<Vec<u8>>,
}

impl Catalogs {
    pub fn get(&self, language: &str) -> Option<&[u8]> {
        match language {
            "de" => self.de.as_deref(),
            "en" => self.en.as_deref(),
            _ => None,
        }
    }

    fn set(&mut self, language: &str, bytes: Vec<u8>) {
        match language {
            "de" => self.de = Some(bytes),
            "en" => self.en = Some(bytes),
            _ => {}
        }
    }
}

pub fn load(web_dir: &Path) -> anyhow::Result<Catalogs> {
    let root = web_dir.join("i18n");
    if !root.is_dir() {
        return Ok(Catalogs::default());
    }
    let mut directories = Vec::new();
    for entry in std::fs::read_dir(&root).with_context(|| format!("read {}", root.display()))? {
        let entry = entry.with_context(|| format!("read {}", root.display()))?;
        let path = entry.path();
        if path.is_dir() {
            directories.push(path);
        }
    }
    directories.sort();
    let mut merged: BTreeMap<&str, BTreeMap<String, String>> = LANGUAGES
        .iter()
        .map(|language| (*language, BTreeMap::new()))
        .collect();
    let mut sources: BTreeMap<&str, BTreeMap<String, PathBuf>> = LANGUAGES
        .iter()
        .map(|language| (*language, BTreeMap::new()))
        .collect();
    for directory in directories {
        for language in LANGUAGES {
            let path = directory.join(format!("{language}.json"));
            if !path.is_file() {
                continue;
            }
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("read {}", path.display()))?;
            let value: Value =
                serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
            let Some(object) = value.as_object() else {
                bail!("{} is not a JSON object", path.display());
            };
            let Some(table) = merged.get_mut(language) else {
                bail!("unsupported catalog language {language}");
            };
            let Some(seen) = sources.get_mut(language) else {
                bail!("unsupported catalog language {language}");
            };
            for (key, item) in object {
                let Some(text) = item.as_str() else {
                    bail!("{} in {} is not a string", key, path.display());
                };
                if let Some(first) = seen.get(key) {
                    bail!(
                        "duplicate catalog key {key} in {} and {}",
                        first.display(),
                        path.display()
                    );
                }
                table.insert(key.clone(), text.to_string());
                seen.insert(key.clone(), path.clone());
            }
        }
    }
    let mut catalogs = Catalogs::default();
    for language in LANGUAGES {
        if let Some(table) = merged.remove(language) {
            if !table.is_empty() {
                catalogs.set(language, serde_json::to_vec_pretty(&table)?);
            }
        }
    }
    Ok(catalogs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_dir(label: &str) -> Result<PathBuf, std::io::Error> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "koreograf-i18n-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn write_catalog(
        path: &Path,
        entries: &[(&str, &str)],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let table = entries
            .iter()
            .map(|(key, value)| (*key, *value))
            .collect::<BTreeMap<_, _>>();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec(&table)?)?;
        Ok(())
    }

    #[test]
    fn a_duplicate_key_fails_with_both_files() -> Result<(), Box<dyn std::error::Error>> {
        let dir = temp_dir("duplicate")?;
        let web = dir.join("web");
        write_catalog(&web.join("i18n/core/de.json"), &[("shared", "Eins")])?;
        write_catalog(&web.join("i18n/core/en.json"), &[("shared", "One")])?;
        write_catalog(&web.join("i18n/jobs/de.json"), &[("shared", "Zwei")])?;
        write_catalog(&web.join("i18n/jobs/en.json"), &[("shared", "Two")])?;

        let error = match load(&web) {
            Ok(_) => return Err("duplicate key was loaded".into()),
            Err(error) => error,
        };
        let text = format!("{error:#}");
        assert!(text.contains("duplicate catalog key shared"), "{text}");
        assert!(text.contains("core/de.json"), "{text}");
        assert!(text.contains("jobs/de.json"), "{text}");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn the_merged_catalog_contains_every_key_from_every_file(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = temp_dir("merge")?;
        let web = dir.join("web");
        write_catalog(
            &web.join("i18n/core/de.json"),
            &[("nav", "Menue"), ("shared", "Geteilt")],
        )?;
        write_catalog(
            &web.join("i18n/core/en.json"),
            &[("nav", "Menu"), ("shared", "Shared")],
        )?;
        write_catalog(&web.join("i18n/jobs/de.json"), &[("run", "Starten")])?;
        write_catalog(&web.join("i18n/jobs/en.json"), &[("run", "Run")])?;

        let catalogs = load(&web)?;
        let de: BTreeMap<String, String> =
            serde_json::from_slice(catalogs.get("de").ok_or("de is missing")?)?;
        let en: BTreeMap<String, String> =
            serde_json::from_slice(catalogs.get("en").ok_or("en is missing")?)?;
        assert_eq!(de.get("nav").map(String::as_str), Some("Menue"));
        assert_eq!(de.get("shared").map(String::as_str), Some("Geteilt"));
        assert_eq!(de.get("run").map(String::as_str), Some("Starten"));
        assert_eq!(en.get("nav").map(String::as_str), Some("Menu"));
        assert_eq!(en.get("shared").map(String::as_str), Some("Shared"));
        assert_eq!(en.get("run").map(String::as_str), Some("Run"));
        assert_eq!(de.keys().collect::<Vec<_>>(), en.keys().collect::<Vec<_>>());
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_missing_catalog_directory_serves_no_catalogs() -> Result<(), Box<dyn std::error::Error>> {
        let dir = temp_dir("missing")?;
        let catalogs = load(&dir)?;
        assert!(catalogs.get("de").is_none());
        assert!(catalogs.get("en").is_none());
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
