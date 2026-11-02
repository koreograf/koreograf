//! Installation archive: a consistent SQLite snapshot plus `secret.key`.
//!
//! Cache, git checkouts and job directories stay on disk. They are rebuilt
//! or cloned. The archive is the piece a later PostgreSQL import can read.

use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use rusqlite::backup::Backup;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::crypto;
use crate::db;
use crate::error::{ApiError, AppResult};
use crate::Shared;

const FORMAT: u32 = 1;
const ENGINE: &str = "sqlite";
const MAX_BYTES: u64 = 256 * 1024 * 1024;
const MANIFEST: &str = "manifest.json";
const DATABASE: &str = "koreograf.db";
const SECRET: &str = "secret.key";

const REQUIRED_TABLES: &[&str] = &[
    "users",
    "organizations",
    "teams",
    "credentials",
    "projects",
    "templates",
    "workflows",
    "hooks",
];

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    format: u32,
    engine: String,
    created_at: String,
    koreograf: String,
    includes: Vec<String>,
    omits: Vec<String>,
}

struct TempPath(PathBuf);

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(crate) fn export(state: &Shared, include_tofu_state: bool) -> AppResult<Vec<u8>> {
    let _maintenance = lock_maintenance(state)?;
    let key = read_key(&state.cfg.data_dir)?;
    let live = state.db()?;
    snapshot_zip(&state.cfg.data_dir, &live, &key, include_tofu_state)
}

pub(crate) fn restore(state: &Shared, bytes: &[u8]) -> AppResult<()> {
    let _maintenance = lock_maintenance(state)?;
    let incoming = prepare_incoming(&state.cfg.data_dir, bytes)?;
    let current_key = read_key(&state.cfg.data_dir)?;
    let snapshot = state
        .cfg
        .data_dir
        .join(format!("backup-{}.db", Uuid::new_v4()));
    let _cleanup = TempPath(snapshot.clone());
    {
        let mut live = state.db()?;
        if busy(&live)? {
            return Err(ApiError::bad("backup.still_running"));
        }
        {
            let mut saved = Connection::open(&snapshot)?;
            copy_database(&live, &mut saved)?;
        }
        let previous = snapshot_zip_from_file(&snapshot, &current_key, &state.cfg.data_dir, false)?;
        write_private(&state.cfg.data_dir.join("restore-previous.zip"), &previous)?;
        let source = Connection::open(&incoming.path)?;
        if let Err(err) = copy_database(&source, &mut live).and_then(|_| db::ensure_schema(&live)) {
            if let Err(rollback) = rollback(&snapshot, &mut live) {
                tracing::error!(
                    error = %rollback,
                    "backup rollback failed"
                );
            }
            return Err(err);
        }
    }
    let cipher = crypto::install_key(&state.cfg.data_dir, &incoming.key)?;
    state.replace_cipher(cipher)?;
    write_extra_files(&state.cfg.data_dir, &incoming.files)?;
    tracing::info!("backup restored");
    Ok(())
}

#[derive(Debug)]
pub(crate) struct RestoreSummary {
    pub(crate) koreograf: String,
    pub(crate) includes: Vec<String>,
}

pub(crate) fn restore_startup(data_dir: &Path, bytes: &[u8]) -> AppResult<RestoreSummary> {
    let incoming = prepare_incoming(data_dir, bytes)?;
    let target = data_dir.join(DATABASE);
    if target.exists() {
        return Err(ApiError::bad("the_data_directory_is_not_empty"));
    }
    fs::rename(&incoming.path, &target).map_err(|err| ApiError::Internal(err.to_string()))?;
    crypto::install_key(data_dir, &incoming.key)?;
    write_extra_files(data_dir, &incoming.files)?;
    own_imported(data_dir);
    Ok(RestoreSummary {
        koreograf: incoming.koreograf.clone(),
        includes: incoming.includes.clone(),
    })
}

fn lock_maintenance(state: &Shared) -> AppResult<std::sync::MutexGuard<'_, ()>> {
    state
        .maintenance
        .lock()
        .map_err(|_| ApiError::Internal("backup lock is damaged".into()))
}

fn read_key(data_dir: &Path) -> AppResult<Vec<u8>> {
    let key = fs::read(data_dir.join(SECRET))
        .map_err(|_| ApiError::Internal("secret.key is missing".into()))?;
    crypto::parse_key(&key)?;
    Ok(key)
}

fn prepare_incoming(data_dir: &Path, bytes: &[u8]) -> AppResult<Incoming> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
        return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
    }
    let entries = unpack(bytes)?;
    let manifest = entry(&entries, MANIFEST, "the_backup_cannot_be_read")?;
    let manifest: Manifest = serde_json::from_slice(manifest)
        .map_err(|_| ApiError::bad("the_backup_has_an_unknown_format"))?;
    if manifest.format != FORMAT || manifest.engine != ENGINE {
        return Err(ApiError::bad("the_backup_has_an_unknown_format"));
    }
    let database = entry(
        &entries,
        DATABASE,
        "the_backup_does_not_contain_a_koreograf_database",
    )?
    .to_vec();
    let key = entry(&entries, SECRET, "the_backup_does_not_contain_a_key")?.to_vec();
    crypto::parse_key(&key)?;
    let mut files = Vec::new();
    for (name, bytes) in &entries {
        if name == MANIFEST || name == DATABASE || name == SECRET {
            continue;
        }
        if !safe_relative(name) || !name.starts_with("cache/") {
            return Err(ApiError::bad("the_backup_cannot_be_read"));
        }
        files.push((name.clone(), bytes.clone()));
    }
    let path = data_dir.join(format!("restore-{}.db", Uuid::new_v4()));
    fs::write(&path, database).map_err(|err| ApiError::Internal(err.to_string()))?;
    let incoming = Incoming {
        path,
        key,
        files,
        koreograf: manifest.koreograf,
        includes: manifest.includes,
    };
    let conn = Connection::open(&incoming.path)?;
    checked_database(&conn)?;
    let version = db::schema_version(&conn)?;
    if version > db::SCHEMA_VERSION {
        return Err(ApiError::bad("the_backup_is_from_a_newer_koreograf"));
    }
    Ok(incoming)
}

struct Incoming {
    path: PathBuf,
    key: Vec<u8>,
    files: Vec<(String, Vec<u8>)>,
    koreograf: String,
    includes: Vec<String>,
}

impl Drop for Incoming {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn entry<'a>(entries: &'a [(String, Vec<u8>)], name: &str, missing: &str) -> AppResult<&'a [u8]> {
    entries
        .iter()
        .find(|(item, _)| item == name)
        .map(|(_, bytes)| bytes.as_slice())
        .ok_or_else(|| ApiError::bad(missing))
}

fn snapshot_zip(
    data_dir: &Path,
    live: &Connection,
    key: &[u8],
    include_tofu_state: bool,
) -> AppResult<Vec<u8>> {
    let path = data_dir.join(format!("backup-{}.db", Uuid::new_v4()));
    let _cleanup = TempPath(path.clone());
    {
        let mut dest = Connection::open(&path)?;
        copy_database(live, &mut dest)?;
    }
    snapshot_zip_from_file(&path, key, data_dir, include_tofu_state)
}

fn snapshot_zip_from_file(
    path: &Path,
    key: &[u8],
    data_dir: &Path,
    include_tofu_state: bool,
) -> AppResult<Vec<u8>> {
    let database = fs::read(path).map_err(|err| ApiError::Internal(err.to_string()))?;
    if database.is_empty() || database.len() as u64 > MAX_BYTES {
        return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
    }
    let mut includes = vec!["database".into(), "secret_key".into()];
    let mut omits = vec!["checkouts".into(), "job_workdirs".into()];
    let extras = if include_tofu_state {
        includes.push("opentofu_state".into());
        collect_tofu_state(data_dir)?
    } else {
        omits.push("cache".into());
        Vec::new()
    };
    let manifest = Manifest {
        format: FORMAT,
        engine: ENGINE.into(),
        created_at: db::now(),
        koreograf: env!("CARGO_PKG_VERSION").into(),
        includes,
        omits,
    };
    let manifest =
        serde_json::to_vec_pretty(&manifest).map_err(|err| ApiError::Internal(err.to_string()))?;
    let mut files: Vec<(&str, &[u8])> = vec![
        (MANIFEST, manifest.as_slice()),
        (DATABASE, database.as_slice()),
        (SECRET, key),
    ];
    for (name, bytes) in &extras {
        files.push((name.as_str(), bytes.as_slice()));
    }
    pack(&files)
}

fn collect_tofu_state(data_dir: &Path) -> AppResult<Vec<(String, Vec<u8>)>> {
    let cache = data_dir.join("cache");
    let mut out = Vec::new();
    let Ok(teams) = fs::read_dir(&cache) else {
        return Ok(out);
    };
    let mut teams: Vec<_> = teams.filter_map(Result::ok).collect();
    teams.sort_by_key(|entry| entry.file_name());
    for team in teams {
        let name = team.file_name().to_string_lossy().to_string();
        if !safe_component(&name) {
            continue;
        }
        let state = team.path().join("tofu-state");
        if !fs::symlink_metadata(&state).is_ok_and(|meta| meta.is_dir()) {
            continue;
        }
        collect_files(&state, &format!("cache/{name}/tofu-state"), &mut out)?;
    }
    Ok(out)
}

fn collect_files(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) -> AppResult<()> {
    let entries = fs::read_dir(dir).map_err(|err| ApiError::Internal(err.to_string()))?;
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if !safe_component(&name) {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|err| ApiError::Internal(err.to_string()))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let child = entry.path();
        if metadata.is_dir() {
            collect_files(&child, &format!("{prefix}/{name}"), out)?;
        } else if metadata.is_file() {
            if metadata.len() > MAX_BYTES {
                return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
            }
            let bytes = fs::read(&child).map_err(|err| ApiError::Internal(err.to_string()))?;
            out.push((format!("{prefix}/{name}"), bytes));
        }
    }
    Ok(())
}

fn safe_component(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

fn safe_relative(name: &str) -> bool {
    let path = Path::new(name);
    !name.contains(['\\', '\0'])
        && !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn write_extra_files(data_dir: &Path, files: &[(String, Vec<u8>)]) -> AppResult<()> {
    for (name, bytes) in files {
        if !safe_relative(name) {
            return Err(ApiError::bad("the_backup_cannot_be_read"));
        }
        let target = data_dir.join(name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| ApiError::Internal(err.to_string()))?;
        }
        write_private(&target, bytes)?;
    }
    Ok(())
}

pub(crate) fn own_imported(data_dir: &Path) {
    #[cfg(unix)]
    {
        use nix::unistd::Uid;
        if !Uid::effective().is_root() {
            return;
        }
        let _ = crate::entrypoint::own_tree(data_dir, 10001, 10001);
    }
    #[cfg(not(unix))]
    let _ = data_dir;
}

fn rollback(snapshot: &Path, live: &mut Connection) -> AppResult<()> {
    let source = Connection::open(snapshot)?;
    copy_database(&source, live)
}

fn busy(c: &Connection) -> AppResult<bool> {
    let jobs: i64 = c.query_row(
        "SELECT COUNT(*) FROM jobs WHERE status IN ('pending', 'running')",
        [],
        |row| row.get(0),
    )?;
    let flows: i64 = c.query_row(
        "SELECT COUNT(*) FROM workflow_jobs WHERE status IN ('pending', 'running')",
        [],
        |row| row.get(0),
    )?;
    Ok(jobs > 0 || flows > 0)
}

fn checked_database(c: &Connection) -> AppResult<()> {
    let status: String = c.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if status != "ok" {
        return Err(ApiError::bad("the_database_in_the_backup_is_damaged"));
    }
    for table in REQUIRED_TABLES {
        if !has_table(c, table)? {
            return Err(ApiError::bad(
                "the_backup_does_not_contain_a_koreograf_database",
            ));
        }
    }
    Ok(())
}

fn has_table(c: &Connection, name: &str) -> AppResult<bool> {
    let found: Option<i64> = c
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

pub(crate) fn copy_database(from: &Connection, to: &mut Connection) -> AppResult<()> {
    let backup = Backup::new(from, to)?;
    for _ in 0..20_000 {
        match backup.step(64)? {
            rusqlite::backup::StepResult::Done => return Ok(()),
            rusqlite::backup::StepResult::More => {}
            rusqlite::backup::StepResult::Busy | rusqlite::backup::StepResult::Locked => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    Err(ApiError::Internal("database copy did not finish".into()))
}

fn pack(files: &[(&str, &[u8])]) -> AppResult<Vec<u8>> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in files {
            writer
                .start_file(*name, options)
                .map_err(|_| ApiError::Internal("backup cannot be written".into()))?;
            writer
                .write_all(bytes)
                .map_err(|_| ApiError::Internal("backup cannot be written".into()))?;
        }
        writer
            .finish()
            .map_err(|_| ApiError::Internal("backup cannot be written".into()))?;
    }
    Ok(cursor.into_inner())
}

fn unpack(bytes: &[u8]) -> AppResult<Vec<(String, Vec<u8>)>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| ApiError::bad("the_backup_cannot_be_read"))?;
    if archive.is_empty() || archive.len() > 10_000 {
        return Err(ApiError::bad("the_backup_cannot_be_read"));
    }
    let mut out = Vec::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|_| ApiError::bad("the_backup_cannot_be_read"))?;
        if file.is_symlink() {
            return Err(ApiError::bad("the_backup_cannot_be_read"));
        }
        if !file.is_file() {
            continue;
        }
        let Some(name) = file.enclosed_name() else {
            return Err(ApiError::bad("the_backup_cannot_be_read"));
        };
        let name = name.to_string_lossy().replace('\\', "/");
        if name.contains("..") || file.size() > MAX_BYTES {
            return Err(ApiError::bad("the_backup_cannot_be_read"));
        }
        let mut buf = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut buf)
            .map_err(|_| ApiError::bad("the_backup_cannot_be_read"))?;
        if buf.len() as u64 > MAX_BYTES {
            return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
        }
        total = total.saturating_add(buf.len() as u64);
        if total > MAX_BYTES {
            return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
        }
        out.push((name, buf));
    }
    Ok(out)
}

fn write_private(path: &Path, bytes: &[u8]) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| ApiError::Internal(err.to_string()))?;
    }
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts
        .open(path)
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    file.write_all(bytes)
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{database_choice, Config, CookieSecure, LocalJobs, RunnerMode};

    fn config(dir: &Path) -> Config {
        Config {
            bind: "127.0.0.1:0".into(),
            data_dir: dir.to_path_buf(),
            web_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
            examples_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
            admin_user: "admin".into(),
            admin_password: Some("test-admin-pass".into()),
            runner: RunnerMode::Local,
            container_image: false,
            job_timeout_secs: 60,
            job_output_lines: 8_000,
            local_jobs: LocalJobs::All,
            project_roots: Vec::new(),
            hsts: false,
            cookie_secure: CookieSecure::Auto,
            trusted_proxy: false,
            data_host: None,
            ldap: None,
        }
    }

    #[test]
    fn postgres_url_is_rejected_and_sqlite_stays() {
        assert!(database_choice("").is_ok());
        assert!(database_choice("sqlite:data/koreograf.db").is_ok());
        let err = database_choice("postgres://koreograf@db/koreograf").expect_err("postgres");
        assert!(err.contains("PostgreSQL is not connected yet"), "{err}");
        assert!(database_choice("postgresql://db/koreograf").is_err());
        assert!(database_choice("mysql://db/koreograf").is_err());
    }

    #[test]
    fn restore_brings_back_the_database_and_the_key() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-backup-{nanos}"));
        let state = crate::boot(config(&dir)).expect("boot");
        let blob = state.encrypt("token-geheim").expect("encrypt");
        let archive = export(&state, false).expect("export");
        {
            let db = state.db().expect("db");
            db.execute(
                "UPDATE organizations SET name = 'Anders' WHERE name = 'Plattform'",
                [],
            )
            .expect("rename");
        }
        let replacement = crypto::install_key(&dir, &[7u8; 32]).expect("new key");
        state.replace_cipher(replacement).expect("swap");
        assert!(state.decrypt(&blob).is_err());
        restore(&state, &archive).expect("restore");
        assert_eq!(state.decrypt(&blob).expect("decrypt"), "token-geheim");
        let name: String = state
            .db()
            .expect("db")
            .query_row(
                "SELECT name FROM organizations WHERE name = 'Plattform'",
                [],
                |row| row.get(0),
            )
            .expect("org");
        assert_eq!(name, "Plattform");
        assert!(dir.join("restore-previous.zip").is_file());
        let _ = fs::remove_dir_all(dir);
    }

    fn archive_parts(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
        unpack(archive).expect("unpack")
    }

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("koreograf-{label}-{nanos}"))
    }

    #[test]
    fn export_lists_the_tofu_state_only_when_it_is_included() {
        let dir = temp_dir("backup-tofu");
        let state = crate::boot(config(&dir)).expect("boot");
        let team: String = state
            .db()
            .expect("db")
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        let state_dir = dir
            .join("cache")
            .join(&team)
            .join("tofu-state")
            .join("tpl-1");
        fs::create_dir_all(&state_dir).expect("state dir");
        fs::write(state_dir.join("terraform.tfstate"), b"{\"serial\":1}").expect("state");

        let without = export(&state, false).expect("export without state");
        let names: Vec<String> = archive_parts(&without)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert!(
            !names.iter().any(|name| name.starts_with("cache/")),
            "{names:?}"
        );

        let with = export(&state, true).expect("export with state");
        let parts = archive_parts(&with);
        let manifest: Manifest = serde_json::from_slice(
            &parts
                .iter()
                .find(|(name, _)| name == MANIFEST)
                .expect("manifest")
                .1,
        )
        .expect("manifest json");
        assert!(manifest
            .includes
            .iter()
            .any(|item| item == "opentofu_state"));
        assert!(parts.iter().any(|(name, bytes)| {
            name == &format!("cache/{team}/tofu-state/tpl-1/terraform.tfstate")
                && bytes == b"{\"serial\":1}"
        }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn startup_restore_writes_the_database_the_key_and_the_tofu_state() {
        let source = temp_dir("startup-source");
        let state = crate::boot(config(&source)).expect("boot");
        let team: String = state
            .db()
            .expect("db")
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        let state_path = source
            .join("cache")
            .join(&team)
            .join("tofu-state")
            .join("tpl-1")
            .join("terraform.tfstate");
        fs::create_dir_all(state_path.parent().expect("parent")).expect("state dir");
        fs::write(&state_path, b"state").expect("state");
        let archive = export(&state, true).expect("export");

        let dest = temp_dir("startup-dest");
        fs::create_dir_all(&dest).expect("dest");
        let summary = restore_startup(&dest, &archive).expect("restore");
        assert_eq!(summary.koreograf, env!("CARGO_PKG_VERSION"));
        assert!(dest.join(DATABASE).is_file());
        assert!(dest.join(SECRET).is_file());
        assert_eq!(
            fs::read(
                dest.join("cache")
                    .join(&team)
                    .join("tofu-state")
                    .join("tpl-1")
                    .join("terraform.tfstate")
            )
            .expect("restored state"),
            b"state"
        );
        let conn = Connection::open(dest.join(DATABASE)).expect("db");
        assert!(has_table(&conn, "users").expect("table"));
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn startup_restore_refuses_an_unknown_manifest_format() {
        let dir = temp_dir("unknown-format");
        let state = crate::boot(config(&dir)).expect("boot");
        let archive = export(&state, false).expect("export");
        let parts = archive_parts(&archive);
        let mut files: Vec<(&str, &[u8])> = Vec::new();
        let bad = br#"{"format":999,"engine":"sqlite","created_at":"","koreograf":"x","includes":[],"omits":[]}"#;
        for (name, bytes) in &parts {
            if name == MANIFEST {
                files.push((name.as_str(), bad));
            } else {
                files.push((name.as_str(), bytes.as_slice()));
            }
        }
        let crafted = pack(&files).expect("pack");
        let err = restore_startup(&dir.join("dest"), &crafted).expect_err("format");
        assert!(
            err.to_string().contains("the_backup_has_an_unknown_format"),
            "{err}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn startup_restore_refuses_a_truncated_archive() {
        let dir = temp_dir("truncated");
        let state = crate::boot(config(&dir)).expect("boot");
        let archive = export(&state, false).expect("export");
        let dest = dir.join("dest");
        fs::create_dir_all(&dest).expect("dest");
        let err = restore_startup(&dest, &archive[..archive.len() / 2]).expect_err("truncated");
        assert!(
            err.to_string().contains("the_backup_cannot_be_read"),
            "{err}"
        );
        assert!(!dest.join(DATABASE).exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn startup_restore_refuses_traversal_and_symlink_entries() {
        let dir = temp_dir("unsafe");
        let state = crate::boot(config(&dir)).expect("boot");
        let archive = export(&state, false).expect("export");
        let parts = archive_parts(&archive);
        let mut files: Vec<(&str, &[u8])> = parts
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        files.push(("../escape.txt", b"x"));
        let crafted = pack(&files).expect("pack");
        let dest = dir.join("dest");
        fs::create_dir_all(&dest).expect("dest");
        let err = restore_startup(&dest, &crafted).expect_err("traversal");
        assert!(
            err.to_string().contains("the_backup_cannot_be_read"),
            "{err}"
        );

        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, bytes) in &parts {
                writer.start_file(name.as_str(), options).expect("file");
                writer.write_all(bytes).expect("write");
            }
            writer
                .add_symlink("cache/team/tofu-state/link", "../outside", options)
                .expect("symlink");
            writer.finish().expect("finish");
        }
        let symlink_archive = cursor.into_inner();
        let err = restore_startup(&dest, &symlink_archive).expect_err("symlink");
        assert!(
            err.to_string().contains("the_backup_cannot_be_read"),
            "{err}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn running_job_blocks_restore() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-backup-busy-{nanos}"));
        let state = crate::boot(config(&dir)).expect("boot");
        let archive = export(&state, false).expect("export");
        {
            let db = state.db().expect("db");
            let team_id: String = db
                .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
                .expect("team");
            db.execute(
                "INSERT INTO jobs (
                    id, team_id, template_id, workflow_job_id, workflow_node_id, name, tool, status,
                    extra_vars, launched_by, trigger, started_at, finished_at, created_at
                 ) VALUES (
                    'job-1', ?1, NULL, NULL, NULL, 'Lauf', 'shell', 'running',
                    '{}', 'admin', 'manual', NULL, NULL, '2026-01-01T00:00:00Z'
                 )",
                [team_id],
            )
            .expect("job");
        }
        let err = restore(&state, &archive).expect_err("busy");
        assert!(err.to_string().contains("backup.still_running"), "{err}");
        let _ = fs::remove_dir_all(dir);
    }
}
