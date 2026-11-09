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

fn sidecars(path: &Path) -> Vec<PathBuf> {
    let mut paths = vec![path.to_path_buf()];
    for suffix in ["-wal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        paths.push(PathBuf::from(name));
    }
    paths
}

impl Drop for TempPath {
    fn drop(&mut self) {
        for path in sidecars(&self.0) {
            let _ = fs::remove_file(path);
        }
    }
}

pub(crate) fn export(state: &Shared, include_tofu_state: bool) -> AppResult<Vec<u8>> {
    let _maintenance = lock_maintenance(state)?;
    let key = read_key(&state.cfg.data_dir)?;
    let snapshot = state
        .cfg
        .data_dir
        .join(format!("backup-{}.db", Uuid::new_v4()));
    let _cleanup = TempPath(snapshot.clone());
    state.store().export_sqlite(&snapshot)?;
    snapshot_zip_from_file(&snapshot, &key, &state.cfg.data_dir, include_tofu_state)
}

pub(crate) fn restore(state: &Shared, bytes: &[u8]) -> AppResult<()> {
    let _maintenance = lock_maintenance(state)?;
    let incoming = prepare_incoming(&state.cfg.data_dir, bytes)?;
    let current_key = read_key(&state.cfg.data_dir)?;
    if state.store().busy()? {
        return Err(ApiError::bad("backup.still_running"));
    }
    let snapshot = state
        .cfg
        .data_dir
        .join(format!("backup-{}.db", Uuid::new_v4()));
    let _cleanup = TempPath(snapshot.clone());
    state.store().export_sqlite(&snapshot)?;
    let previous = snapshot_zip_from_file(&snapshot, &current_key, &state.cfg.data_dir, false)?;
    write_private(&state.cfg.data_dir.join("restore-previous.zip"), &previous)?;
    let engine = state.engine();
    if let Err(err) = state.store().restore_sqlite(&incoming.path) {
        if engine == "sqlite" {
            if let Err(rollback) = state.store().restore_sqlite(&snapshot) {
                tracing::error!(
                    error = %rollback,
                    "backup rollback failed"
                );
            }
        }
        return Err(err);
    }
    let cipher = crypto::install_key(&state.cfg.data_dir, &incoming.key)?;
    state.replace_cipher(cipher)?;
    if incoming
        .includes
        .iter()
        .any(|item| item == "opentofu_state")
    {
        clear_state_dirs(&state.cfg.data_dir)?;
        write_extra_files(&state.cfg.data_dir, &incoming.files)?;
    }
    tracing::info!("backup restored");
    Ok(())
}

#[derive(Debug)]
pub(crate) struct RestoreSummary {
    pub(crate) koreograf: String,
    pub(crate) includes: Vec<String>,
}

pub(crate) fn restore_startup(
    data_dir: &Path,
    bytes: &[u8],
    container_image: bool,
) -> AppResult<RestoreSummary> {
    let target = data_dir.join(DATABASE);
    if target.exists() {
        return Err(ApiError::bad("the_data_directory_is_not_empty"));
    }
    fs::create_dir_all(data_dir).map_err(|err| ApiError::Internal(err.to_string()))?;
    let staging = new_staging(data_dir, ".restore");
    fs::create_dir_all(&staging).map_err(|err| ApiError::Internal(err.to_string()))?;
    let result = restore_startup_into(&staging, bytes).and_then(|summary| {
        promote(&staging, data_dir)?;
        Ok(summary)
    });
    match result {
        Ok(summary) => {
            let _ = fs::remove_dir_all(&staging);
            own_imported(data_dir, container_image);
            Ok(summary)
        }
        Err(err) => {
            let _ = fs::remove_dir_all(&staging);
            Err(err)
        }
    }
}

fn restore_startup_into(staging: &Path, bytes: &[u8]) -> AppResult<RestoreSummary> {
    let incoming = prepare_incoming(staging, bytes)?;
    let target = staging.join(DATABASE);
    if target.exists() {
        return Err(ApiError::bad("the_data_directory_is_not_empty"));
    }
    fs::rename(&incoming.path, &target).map_err(|err| ApiError::Internal(err.to_string()))?;
    crypto::install_key(staging, &incoming.key)?;
    write_extra_files(staging, &incoming.files)?;
    Ok(RestoreSummary {
        koreograf: incoming.koreograf.clone(),
        includes: incoming.includes.clone(),
    })
}

pub(crate) fn new_staging(data_dir: &Path, prefix: &str) -> PathBuf {
    data_dir.join(format!("{prefix}-{}", Uuid::new_v4()))
}

pub(crate) fn remove_stale_staging(data_dir: &Path) {
    let Ok(entries) = fs::read_dir(data_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(".import-") && !name.starts_with(".restore-") {
            continue;
        }
        let path = entry.path();
        if fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            let _ = fs::remove_dir_all(path);
        }
    }
}

pub(crate) fn promote(staging: &Path, data_dir: &Path) -> AppResult<()> {
    let mut moved = Vec::new();
    let entries = fs::read_dir(staging).map_err(|err| ApiError::Internal(err.to_string()))?;
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| {
        let name = entry.file_name();
        (name.to_str() == Some(DATABASE), name)
    });
    for entry in entries {
        let target = data_dir.join(entry.file_name());
        if fs::symlink_metadata(&target).is_ok() {
            remove_moved(&moved);
            return Err(ApiError::Internal(format!(
                "{} already exists",
                target.display()
            )));
        }
        if let Err(err) = fs::rename(entry.path(), &target) {
            remove_moved(&moved);
            return Err(ApiError::Internal(err.to_string()));
        }
        moved.push(target);
    }
    Ok(())
}

fn remove_moved(paths: &[PathBuf]) {
    for path in paths.iter().rev() {
        if fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir()) {
            let _ = fs::remove_dir_all(path);
        } else {
            let _ = fs::remove_file(path);
        }
    }
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
        if state_file_parts(name).is_none() {
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
    drop(conn);
    let conn = db::open(&incoming.path)?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
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
        for path in sidecars(&self.path) {
            let _ = fs::remove_file(path);
        }
    }
}

fn entry<'a>(entries: &'a [(String, Vec<u8>)], name: &str, missing: &str) -> AppResult<&'a [u8]> {
    entries
        .iter()
        .find(|(item, _)| item == name)
        .map(|(_, bytes)| bytes.as_slice())
        .ok_or_else(|| ApiError::bad(missing))
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
    let total: u64 = files.iter().map(|(_, bytes)| bytes.len() as u64).sum();
    if total > MAX_BYTES {
        return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
    }
    let packed = pack(&files)?;
    if packed.len() as u64 > MAX_BYTES {
        return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
    }
    Ok(packed)
}

fn collect_tofu_state(data_dir: &Path) -> AppResult<Vec<(String, Vec<u8>)>> {
    let cache = data_dir.join("cache");
    let mut out = Vec::new();
    let mut total = 0u64;
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
        collect_files(
            &state,
            &format!("cache/{name}/tofu-state"),
            &mut out,
            &mut total,
        )?;
    }
    Ok(out)
}

fn collect_files(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<(String, Vec<u8>)>,
    total: &mut u64,
) -> AppResult<()> {
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
            collect_files(&child, &format!("{prefix}/{name}"), out, total)?;
        } else if metadata.is_file() {
            if metadata.len() > MAX_BYTES {
                return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
            }
            let bytes = fs::read(&child).map_err(|err| ApiError::Internal(err.to_string()))?;
            *total = total.saturating_add(bytes.len() as u64);
            if *total > MAX_BYTES {
                return Err(ApiError::bad("the_backup_is_empty_or_too_large"));
            }
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
        let parts =
            state_file_parts(name).ok_or_else(|| ApiError::bad("the_backup_cannot_be_read"))?;
        let mut dir = ensure_dir(data_dir, "cache")?;
        dir = ensure_dir(&dir, parts[1])?;
        dir = ensure_dir(&dir, "tofu-state")?;
        for part in &parts[3..parts.len() - 1] {
            dir = ensure_dir(&dir, part)?;
        }
        let target = dir.join(parts[parts.len() - 1]);
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => return Err(ApiError::bad("the_backup_cannot_be_read")),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(ApiError::Internal(err.to_string())),
        }
        write_private(&target, bytes)?;
    }
    Ok(())
}

fn clear_state_dirs(data_dir: &Path) -> AppResult<()> {
    let cache = data_dir.join("cache");
    match fs::symlink_metadata(&cache) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(ApiError::Internal(err.to_string())),
    }
    let teams = fs::read_dir(&cache).map_err(|err| ApiError::Internal(err.to_string()))?;
    for team in teams.filter_map(Result::ok) {
        let state = team.path().join("tofu-state");
        match fs::symlink_metadata(&state) {
            Ok(meta) if meta.is_dir() => {
                fs::remove_dir_all(&state).map_err(|err| ApiError::Internal(err.to_string()))?;
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(ApiError::Internal(err.to_string())),
        }
    }
    Ok(())
}

fn state_file_parts(name: &str) -> Option<Vec<&str>> {
    if !safe_relative(name) {
        return None;
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() < 4
        || parts[0] != "cache"
        || parts[2] != "tofu-state"
        || !parts.iter().all(|part| safe_component(part))
    {
        return None;
    }
    Some(parts)
}

fn ensure_dir(parent: &Path, name: &str) -> AppResult<PathBuf> {
    let path = parent.join(name);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_dir() => Ok(path),
        Ok(_) => Err(ApiError::bad("the_backup_cannot_be_read")),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&path).map_err(|err| ApiError::Internal(err.to_string()))?;
            Ok(path)
        }
        Err(err) => Err(ApiError::Internal(err.to_string())),
    }
}

pub(crate) fn own_imported(data_dir: &Path, container_image: bool) {
    #[cfg(unix)]
    {
        use nix::unistd::Uid;
        if !container_image || !Uid::effective().is_root() {
            return;
        }
        let _ = crate::entrypoint::own_tree(data_dir, 10001, 10001);
    }
    #[cfg(not(unix))]
    let _ = (data_dir, container_image);
}

pub(crate) fn busy(c: &Connection) -> AppResult<bool> {
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
    use crate::config::{Config, CookieSecure, LocalJobs, RunnerMode};

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
            mcp: true,
            data_host: None,
            ldap: None,
        }
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
        let summary = restore_startup(&dest, &archive, false).expect("restore");
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
        let err = restore_startup(&dir.join("dest"), &crafted, false).expect_err("format");
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
        let err =
            restore_startup(&dest, &archive[..archive.len() / 2], false).expect_err("truncated");
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
        let err = restore_startup(&dest, &crafted, false).expect_err("traversal");
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
        let err = restore_startup(&dest, &symlink_archive, false).expect_err("symlink");
        assert!(
            err.to_string().contains("the_backup_cannot_be_read"),
            "{err}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn export_refuses_state_over_the_total_limit() {
        let dir = temp_dir("backup-total-limit");
        let state = crate::boot(config(&dir)).expect("boot");
        let team: String = state
            .db()
            .expect("db")
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        let state_dir = dir.join("cache").join(&team).join("tofu-state");
        for name in ["one.tfstate", "two.tfstate"] {
            let path = state_dir.join(name);
            fs::create_dir_all(&state_dir).expect("state dir");
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .expect("state file");
            file.set_len(MAX_BYTES / 2 + 1).expect("large state");
        }
        let err = export(&state, true).expect_err("too large");
        assert!(
            err.to_string().contains("the_backup_is_empty_or_too_large"),
            "{err}"
        );
        drop(state);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn http_restore_replaces_state_directories() {
        let dir = temp_dir("backup-state-replace");
        let state = crate::boot(config(&dir)).expect("boot");
        let team: String = state
            .db()
            .expect("db")
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        let state_dir = dir.join("cache").join(&team).join("tofu-state");
        let kept = state_dir.join("keep").join("terraform.tfstate");
        fs::create_dir_all(kept.parent().expect("parent")).expect("state dir");
        fs::write(&kept, b"kept").expect("state");
        let archive = export(&state, true).expect("export");

        let stale = state_dir.join("stale").join("terraform.tfstate");
        fs::create_dir_all(stale.parent().expect("parent")).expect("stale dir");
        fs::write(&stale, b"stale").expect("stale");
        let other = dir
            .join("cache")
            .join("other-team")
            .join("tofu-state")
            .join("terraform.tfstate");
        fs::create_dir_all(other.parent().expect("parent")).expect("other dir");
        fs::write(&other, b"other").expect("other");

        restore(&state, &archive).expect("restore");
        assert_eq!(fs::read(&kept).expect("kept state"), b"kept");
        assert!(!stale.exists());
        assert!(!other.exists());
        drop(state);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn http_restore_keeps_state_when_the_backup_omits_it() {
        let dir = temp_dir("backup-state-keep");
        let state = crate::boot(config(&dir)).expect("boot");
        let team: String = state
            .db()
            .expect("db")
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        let kept = dir
            .join("cache")
            .join(&team)
            .join("tofu-state")
            .join("tpl")
            .join("terraform.tfstate");
        fs::create_dir_all(kept.parent().expect("parent")).expect("state dir");
        fs::write(&kept, b"kept").expect("state");
        let archive = export(&state, false).expect("export without state");

        restore(&state, &archive).expect("restore");
        assert_eq!(fs::read(&kept).expect("kept state"), b"kept");
        drop(state);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn extra_files_stay_under_the_tofu_state_and_refuse_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("backup-extra-paths");
        let outside = temp_dir("backup-extra-outside");
        fs::create_dir_all(dir.join("cache")).expect("cache");
        fs::create_dir_all(&outside).expect("outside");
        symlink(&outside, dir.join("cache").join("team")).expect("symlink");

        let invalid = vec![("cache/team/other/file".to_string(), b"value".to_vec())];
        let err = write_extra_files(&dir, &invalid).expect_err("invalid path");
        assert!(
            err.to_string().contains("the_backup_cannot_be_read"),
            "{err}"
        );

        let linked = vec![("cache/team/tofu-state/file".to_string(), b"value".to_vec())];
        let err = write_extra_files(&dir, &linked).expect_err("symlink");
        assert!(
            err.to_string().contains("the_backup_cannot_be_read"),
            "{err}"
        );
        assert!(!outside.join("tofu-state").exists());
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn startup_restore_rolls_back_when_a_target_entry_blocks_the_move() {
        let source = temp_dir("startup-rollback-source");
        let state = crate::boot(config(&source)).expect("boot");
        let archive = export(&state, false).expect("export");
        drop(state);

        let dest = temp_dir("startup-rollback-dest");
        fs::create_dir_all(dest.join(SECRET)).expect("blocker");
        let err = restore_startup(&dest, &archive, false).expect_err("blocked");
        assert!(err.to_string().contains("already exists"), "{err}");
        assert!(!dest.join(DATABASE).exists());
        assert!(!dest.join(SECRET).join("x").exists());
        assert!(dest.join(SECRET).is_dir());
        assert!(fs::read_dir(&dest).expect("dest").all(|entry| !entry
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .starts_with(".restore-")));
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
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
