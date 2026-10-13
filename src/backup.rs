//! Installation archive: a consistent SQLite snapshot plus `secret.key`.
//!
//! Cache, git checkouts and job directories stay on disk. They are rebuilt
//! or cloned. The archive is the piece a later PostgreSQL import can read.

use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
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

pub(crate) fn export(state: &Shared) -> AppResult<Vec<u8>> {
    let _maintenance = lock_maintenance(state)?;
    let key = read_key(&state.cfg.data_dir)?;
    let live = state.db()?;
    snapshot_zip(&state.cfg.data_dir, &live, &key)
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
            return Err(ApiError::bad(
                "Es läuft noch ein Auftrag. Beende ihn vor dem Einspielen.",
            ));
        }
        {
            let mut saved = Connection::open(&snapshot)?;
            copy_database(&live, &mut saved)?;
        }
        let previous = snapshot_zip_from_file(&snapshot, &current_key)?;
        write_private(&state.cfg.data_dir.join("restore-previous.zip"), &previous)?;
        let source = Connection::open(&incoming.path)?;
        if let Err(err) = copy_database(&source, &mut live).and_then(|_| db::ensure_schema(&live)) {
            if let Err(rollback) = rollback(&snapshot, &mut live) {
                tracing::error!(
                    error = %rollback,
                    "Sicherung konnte nicht zurückgerollt werden"
                );
            }
            return Err(err);
        }
    }
    let cipher = crypto::install_key(&state.cfg.data_dir, &incoming.key)?;
    state.replace_cipher(cipher)?;
    tracing::info!("Sicherung eingespielt");
    Ok(())
}

fn lock_maintenance(state: &Shared) -> AppResult<std::sync::MutexGuard<'_, ()>> {
    state
        .maintenance
        .lock()
        .map_err(|_| ApiError::Internal("Sicherungssperre ist beschädigt.".into()))
}

fn read_key(data_dir: &Path) -> AppResult<Vec<u8>> {
    let key = fs::read(data_dir.join(SECRET))
        .map_err(|_| ApiError::Internal("secret.key fehlt.".into()))?;
    crypto::parse_key(&key)?;
    Ok(key)
}

fn prepare_incoming(data_dir: &Path, bytes: &[u8]) -> AppResult<Incoming> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
        return Err(ApiError::bad("Die Sicherung ist leer oder zu groß."));
    }
    let entries = unpack(bytes)?;
    let manifest = entry(&entries, MANIFEST, "Die Sicherung lässt sich nicht lesen.")?;
    let manifest: Manifest = serde_json::from_slice(manifest)
        .map_err(|_| ApiError::bad("Die Sicherung hat ein unbekanntes Format."))?;
    if manifest.format != FORMAT || manifest.engine != ENGINE {
        return Err(ApiError::bad("Die Sicherung hat ein unbekanntes Format."));
    }
    let database = entry(
        &entries,
        DATABASE,
        "Die Sicherung enthält keine Koreograf-Datenbank.",
    )?;
    let key = entry(&entries, SECRET, "Die Sicherung enthält keinen Schlüssel.")?.to_vec();
    crypto::parse_key(&key)?;
    let path = data_dir.join(format!("restore-{}.db", Uuid::new_v4()));
    fs::write(&path, database).map_err(|err| ApiError::Internal(err.to_string()))?;
    let incoming = Incoming { path, key };
    let conn = Connection::open(&incoming.path)?;
    checked_database(&conn)?;
    Ok(incoming)
}

struct Incoming {
    path: PathBuf,
    key: Vec<u8>,
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

fn snapshot_zip(data_dir: &Path, live: &Connection, key: &[u8]) -> AppResult<Vec<u8>> {
    let path = data_dir.join(format!("backup-{}.db", Uuid::new_v4()));
    let _cleanup = TempPath(path.clone());
    {
        let mut dest = Connection::open(&path)?;
        copy_database(live, &mut dest)?;
    }
    snapshot_zip_from_file(&path, key)
}

fn snapshot_zip_from_file(path: &Path, key: &[u8]) -> AppResult<Vec<u8>> {
    let database = fs::read(path).map_err(|err| ApiError::Internal(err.to_string()))?;
    if database.is_empty() || database.len() as u64 > MAX_BYTES {
        return Err(ApiError::bad("Die Sicherung ist leer oder zu groß."));
    }
    let manifest = Manifest {
        format: FORMAT,
        engine: ENGINE.into(),
        created_at: db::now(),
        koreograf: env!("CARGO_PKG_VERSION").into(),
        includes: vec!["database".into(), "secret_key".into()],
        omits: vec!["cache".into(), "checkouts".into(), "job_workdirs".into()],
    };
    let manifest =
        serde_json::to_vec_pretty(&manifest).map_err(|err| ApiError::Internal(err.to_string()))?;
    pack(&[
        (MANIFEST, manifest.as_slice()),
        (DATABASE, database.as_slice()),
        (SECRET, key),
    ])
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
        return Err(ApiError::bad(
            "Die Datenbank in der Sicherung ist beschädigt.",
        ));
    }
    for table in REQUIRED_TABLES {
        if !has_table(c, table)? {
            return Err(ApiError::bad(
                "Die Sicherung enthält keine Koreograf-Datenbank.",
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

fn copy_database(from: &Connection, to: &mut Connection) -> AppResult<()> {
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
    Err(ApiError::Internal(
        "Datenbank-Kopie wurde nicht fertig.".into(),
    ))
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
                .map_err(|_| ApiError::Internal("Sicherung lässt sich nicht schreiben.".into()))?;
            writer
                .write_all(bytes)
                .map_err(|_| ApiError::Internal("Sicherung lässt sich nicht schreiben.".into()))?;
        }
        writer
            .finish()
            .map_err(|_| ApiError::Internal("Sicherung lässt sich nicht schreiben.".into()))?;
    }
    Ok(cursor.into_inner())
}

fn unpack(bytes: &[u8]) -> AppResult<Vec<(String, Vec<u8>)>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| ApiError::bad("Die Sicherung lässt sich nicht lesen."))?;
    if archive.is_empty() || archive.len() > 8 {
        return Err(ApiError::bad("Die Sicherung lässt sich nicht lesen."));
    }
    let mut out = Vec::new();
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|_| ApiError::bad("Die Sicherung lässt sich nicht lesen."))?;
        if !file.is_file() {
            continue;
        }
        let Some(name) = file.enclosed_name() else {
            return Err(ApiError::bad("Die Sicherung lässt sich nicht lesen."));
        };
        let name = name.to_string_lossy().replace('\\', "/");
        if name.contains("..") || file.size() > MAX_BYTES {
            return Err(ApiError::bad("Die Sicherung lässt sich nicht lesen."));
        }
        let mut buf = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut buf)
            .map_err(|_| ApiError::bad("Die Sicherung lässt sich nicht lesen."))?;
        if buf.len() as u64 > MAX_BYTES {
            return Err(ApiError::bad("Die Sicherung ist leer oder zu groß."));
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
    use crate::config::{database_choice, Config, RunnerMode};

    fn config(dir: &Path) -> Config {
        Config {
            bind: "127.0.0.1:0".into(),
            data_dir: dir.to_path_buf(),
            web_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
            examples_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
            admin_user: "admin".into(),
            admin_password: Some("test-admin-pass".into()),
            runner: RunnerMode::Local,
            job_timeout_secs: 60,
            ldap: None,
        }
    }

    #[test]
    fn postgres_url_is_rejected_and_sqlite_stays() {
        assert!(database_choice("").is_ok());
        assert!(database_choice("sqlite:data/koreograf.db").is_ok());
        let err = database_choice("postgres://koreograf@db/koreograf").expect_err("postgres");
        assert!(
            err.contains("PostgreSQL ist noch nicht angebunden"),
            "{err}"
        );
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
        let archive = export(&state).expect("export");
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

    #[test]
    fn running_job_blocks_restore() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-backup-busy-{nanos}"));
        let state = crate::boot(config(&dir)).expect("boot");
        let archive = export(&state).expect("export");
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
        assert!(err.to_string().contains("läuft noch ein Auftrag"), "{err}");
        let _ = fs::remove_dir_all(dir);
    }
}
