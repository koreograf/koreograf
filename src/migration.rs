use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};

use crate::backup;
use crate::config::Config;
use crate::crypto;
use crate::db;
use crate::error::{ApiError, AppResult};

const PATH_MAP_KEY: &str = "import_path_map";
pub(crate) const REPORT_KEY: &str = "import_report";
const DATABASE: &str = "koreograf.db";
const SECRET: &str = "secret.key";

#[derive(Clone, Default)]
pub(crate) struct Options {
    pub(crate) restore_from: Option<PathBuf>,
    pub(crate) import_from: Option<PathBuf>,
    pub(crate) path_map: Vec<(PathBuf, PathBuf)>,
    pub(crate) copy_cache: bool,
}

impl Options {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            restore_from: env_path("KOREOGRAF_RESTORE_FROM"),
            import_from: env_path("KOREOGRAF_IMPORT_FROM"),
            path_map: parse_path_map(&env_string("KOREOGRAF_PATH_MAP"))?,
            copy_cache: env_flag("KOREOGRAF_IMPORT_CACHE"),
        })
    }
}

#[derive(Debug)]
pub(crate) struct ImportReport {
    pub(crate) missing_paths: Vec<String>,
    pub(crate) tofu_files: usize,
    pub(crate) path_map: Vec<String>,
}

pub(crate) fn prepare(cfg: &Config) -> anyhow::Result<()> {
    let options = Options::from_env()?;
    prepare_with(cfg, &options)
}

pub(crate) fn prepare_with(cfg: &Config, options: &Options) -> anyhow::Result<()> {
    fs::create_dir_all(&cfg.data_dir)?;
    let data_dir = std::path::absolute(&cfg.data_dir)?;
    backup::remove_stale_staging(&data_dir);
    if let Some(file) = &options.restore_from {
        if !is_empty(&data_dir) {
            tracing::warn!(
                file = %file.display(),
                "KOREOGRAF_RESTORE_FROM is ignored because the data directory already holds data"
            );
        } else {
            let bytes = fs::read(file).map_err(|err| {
                anyhow::anyhow!("The restore file {} cannot be read: {err}", file.display())
            })?;
            let summary =
                backup::restore_startup(&data_dir, &bytes, cfg.container_image).map_err(|err| {
                    anyhow::anyhow!("Restore from {} failed: {}", file.display(), describe(&err))
                })?;
            tracing::info!(
                file = %file.display(),
                koreograf = %summary.koreograf,
                includes = %summary.includes.join(","),
                "restored the installation from the startup backup"
            );
            mark_applied(file);
        }
    }
    if let Some(source) = &options.import_from {
        if !is_empty(&data_dir) {
            tracing::warn!(
                directory = %source.display(),
                "KOREOGRAF_IMPORT_FROM is ignored because the data directory already holds data"
            );
        } else {
            let report = import_directory(cfg, source, options)?;
            for path in &report.missing_paths {
                tracing::warn!(
                    path = %path,
                    "an imported path does not exist in this container; it needs a mount or a change"
                );
            }
            tracing::info!(
                directory = %source.display(),
                missing_paths = report.missing_paths.len(),
                tofu_files = report.tofu_files,
                path_map = report.path_map.len(),
                "imported the existing data directory"
            );
        }
    }
    Ok(())
}

pub(crate) fn import_directory(
    cfg: &Config,
    source: &Path,
    options: &Options,
) -> anyhow::Result<ImportReport> {
    let metadata = fs::symlink_metadata(source).map_err(|err| {
        anyhow::anyhow!(
            "The import directory {} cannot be read: {err}",
            source.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(anyhow::anyhow!(
            "KOREOGRAF_IMPORT_FROM must be a directory, but {} is not",
            source.display()
        ));
    }
    let source_db = source.join(DATABASE);
    required_file(&source_db, "database")?;
    let source_key = source.join(SECRET);
    required_file(&source_key, "secret.key")?;
    let key = fs::read(&source_key)?;
    crypto::parse_key(&key).map_err(|_| {
        anyhow::anyhow!(
            "The key in {} is not a valid Koreograf key",
            source_key.display()
        )
    })?;

    fs::create_dir_all(&cfg.data_dir)?;
    let dest_db = cfg.data_dir.join(DATABASE);
    if dest_db.exists() {
        return Err(anyhow::anyhow!(
            "The data directory already contains a database; nothing was imported"
        ));
    }
    let source_conn = open_source_database(&source_db)?;
    source_conn.busy_timeout(Duration::from_secs(5))?;
    let version = db::schema_version(&source_conn)?;
    if version > db::SCHEMA_VERSION {
        return Err(anyhow::anyhow!(
            "The database {} uses schema {version}, but this build supports {}. Upgrade Koreograf before importing.",
            source_db.display(),
            db::SCHEMA_VERSION
        ));
    }
    let staging = backup::new_staging(&cfg.data_dir, ".import");
    fs::create_dir_all(&staging)?;
    let result = import_into(&staging, &source_conn, source, cfg, &key, options);
    match result {
        Ok(report) => {
            if let Err(err) = backup::promote(&staging, &cfg.data_dir) {
                let _ = fs::remove_dir_all(&staging);
                return Err(err.into());
            }
            let _ = fs::remove_dir_all(&staging);
            backup::own_imported(&cfg.data_dir, cfg.container_image);
            Ok(report)
        }
        Err(err) => {
            let _ = fs::remove_dir_all(&staging);
            Err(err)
        }
    }
}

fn import_into(
    staging: &Path,
    source_conn: &Connection,
    source: &Path,
    cfg: &Config,
    key: &[u8],
    options: &Options,
) -> anyhow::Result<ImportReport> {
    let dest_db = staging.join(DATABASE);
    let mut dest_conn = Connection::open(&dest_db)?;
    dest_conn.busy_timeout(Duration::from_secs(5))?;
    backup::copy_database(source_conn, &mut dest_conn)?;
    db::ensure_schema(&dest_conn)?;
    crypto::install_key(staging, key)?;
    let mapped = apply_path_map(&dest_conn, &options.path_map)?;
    if mapped == 0 && !options.path_map.is_empty() {
        tracing::warn!("KOREOGRAF_PATH_MAP is already recorded and was not applied again");
    }
    let tofu_files = copy_state(source, staging, options.copy_cache)?;
    let missing_paths = missing_paths(&dest_conn, cfg)?;
    let report = json!({
        "imported_at": db::now(),
        "source": source.display().to_string(),
        "koreograf": env!("CARGO_PKG_VERSION"),
        "schema": db::SCHEMA_VERSION,
        "path_map": options.path_map.iter().map(pair_text).collect::<Vec<_>>(),
        "copy_cache": options.copy_cache,
        "tofu_files": tofu_files,
        "missing_paths": missing_paths,
    });
    set_setting(
        &dest_conn,
        REPORT_KEY,
        &serde_json::to_string(&report).map_err(|err| ApiError::Internal(err.to_string()))?,
    )?;
    drop(dest_conn);
    Ok(ImportReport {
        missing_paths,
        tofu_files,
        path_map: options.path_map.iter().map(pair_text).collect(),
    })
}

fn open_source_database(source_db: &Path) -> anyhow::Result<Connection> {
    let mut wal = source_db.as_os_str().to_os_string();
    wal.push("-wal");
    let wal = PathBuf::from(wal);
    let conn = if wal.is_file() {
        Connection::open_with_flags(
            source_db,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
    } else {
        Connection::open_with_flags(
            read_only_uri(source_db),
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI,
        )
    };
    conn.map_err(|err| {
        anyhow::anyhow!(
            "The database {} cannot be opened read-only: {err}",
            source_db.display()
        )
    })
}

fn read_only_uri(path: &Path) -> String {
    let mut uri = String::from("file:");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                uri.push(byte as char);
            }
            _ => {
                uri.push('%');
                uri.push_str(&format!("{byte:02X}"));
            }
        }
    }
    uri.push_str("?mode=ro&immutable=1");
    uri
}

pub(crate) fn report(db: &Connection) -> AppResult<Option<Value>> {
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [REPORT_KEY],
            |row| row.get(0),
        )
        .optional()?;
    Ok(raw.and_then(|text| serde_json::from_str(&text).ok()))
}

fn required_file(path: &Path, label: &str) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|err| {
        anyhow::anyhow!(
            "The import directory is missing {label} ({}): {err}",
            path.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(anyhow::anyhow!(
            "The import directory has no {label} file at {}",
            path.display()
        ));
    }
    Ok(())
}

fn is_empty(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(true)
}

fn mark_applied(path: &Path) {
    let marker = PathBuf::from(format!("{}.applied", path.display()));
    match fs::rename(path, &marker) {
        Ok(()) => tracing::info!(
            marker = %marker.display(),
            "the startup restore marker was renamed"
        ),
        Err(err) => tracing::warn!(
            marker = %marker.display(),
            error = %err,
            "the startup restore marker could not be renamed"
        ),
    }
}

fn copy_state(source: &Path, data_dir: &Path, copy_cache: bool) -> anyhow::Result<usize> {
    let cache = source.join("cache");
    if !cache.is_dir() {
        return Ok(0);
    }
    if copy_cache {
        return copy_tree(&cache, &data_dir.join("cache"));
    }
    let mut copied = 0;
    for team in fs::read_dir(&cache)? {
        let team = team?;
        let name = team.file_name().to_string_lossy().to_string();
        if name.is_empty() || name.contains(['/', '\\', '\0']) {
            continue;
        }
        let state = team.path().join("tofu-state");
        if !fs::symlink_metadata(&state).is_ok_and(|meta| meta.is_dir()) {
            continue;
        }
        let dest = data_dir.join("cache").join(&name).join("tofu-state");
        copied += copy_tree(&state, &dest)?;
    }
    Ok(copied)
}

fn copy_tree(source: &Path, dest: &Path) -> anyhow::Result<usize> {
    let mut copied = 0;
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let target = dest.join(entry.file_name());
        if metadata.is_dir() {
            copied += copy_tree(&entry.path(), &target)?;
        } else if metadata.is_file() {
            fs::copy(entry.path(), &target)?;
            copied += 1;
        }
    }
    Ok(copied)
}

fn apply_path_map(conn: &Connection, map: &[(PathBuf, PathBuf)]) -> AppResult<usize> {
    if map.is_empty() {
        return Ok(0);
    }
    let existing: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [PATH_MAP_KEY],
            |row| row.get(0),
        )
        .optional()?;
    if existing.is_some() {
        return Ok(0);
    }
    let rows = {
        let mut stmt =
            conn.prepare("SELECT id, source FROM projects WHERE source_type = 'path'")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut changed = 0;
    for (id, source) in rows {
        let Some(mapped) = map_one(&source, map) else {
            continue;
        };
        conn.execute(
            "UPDATE projects SET source = ?2 WHERE id = ?1",
            params![id, mapped],
        )?;
        changed += 1;
    }
    set_setting(
        conn,
        PATH_MAP_KEY,
        &serde_json::to_string(&map.iter().map(pair_text).collect::<Vec<_>>())
            .map_err(|err| ApiError::Internal(err.to_string()))?,
    )?;
    Ok(changed)
}

fn map_one(source: &str, map: &[(PathBuf, PathBuf)]) -> Option<String> {
    let path = Path::new(source);
    for (old, new) in map {
        if let Ok(rest) = path.strip_prefix(old) {
            return Some(new.join(rest).display().to_string());
        }
    }
    None
}

fn missing_paths(conn: &Connection, cfg: &Config) -> AppResult<Vec<String>> {
    let mut missing = BTreeSet::new();
    let sources = {
        let mut stmt = conn.prepare("SELECT source FROM projects WHERE source_type = 'path'")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for source in sources {
        let path = Path::new(&source);
        if !path.is_dir() {
            missing.insert(source);
        }
    }
    for root in &cfg.project_roots {
        if !root.is_dir() {
            missing.insert(root.display().to_string());
        }
    }
    Ok(missing.into_iter().collect())
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn pair_text((old, new): &(PathBuf, PathBuf)) -> String {
    format!("{}={}", old.display(), new.display())
}

fn parse_path_map(raw: &str) -> anyhow::Result<Vec<(PathBuf, PathBuf)>> {
    let mut map = Vec::new();
    for item in raw
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let Some((old, new)) = item.split_once('=') else {
            return Err(anyhow::anyhow!(
                "KOREOGRAF_PATH_MAP must use OLD=NEW with absolute paths"
            ));
        };
        let old = PathBuf::from(old.trim());
        let new = PathBuf::from(new.trim());
        if !old.is_absolute() || !new.is_absolute() {
            return Err(anyhow::anyhow!(
                "KOREOGRAF_PATH_MAP must use OLD=NEW with absolute paths"
            ));
        }
        map.push((old, new));
    }
    Ok(map)
}

fn describe(err: &ApiError) -> String {
    let key = err.to_string();
    let prose = match key.as_str() {
        "the_backup_is_empty_or_too_large" => "The backup is empty or too large.",
        "the_backup_cannot_be_read" => "The backup cannot be read. It is truncated or not a zip.",
        "the_backup_has_an_unknown_format" => "The backup has an unknown manifest format.",
        "the_backup_does_not_contain_a_koreograf_database" => {
            "The backup does not contain a Koreograf database."
        }
        "the_backup_does_not_contain_a_key" => "The backup does not contain secret.key.",
        "the_key_in_the_backup_is_invalid" => "The key in the backup is invalid.",
        "the_database_in_the_backup_is_damaged" => "The database in the backup is damaged.",
        "the_data_directory_is_not_empty" => "The data directory is not empty.",
        _ => return key,
    };
    prose.to_string()
}

fn env_path(key: &str) -> Option<PathBuf> {
    let value = env_string(key);
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

fn env_string(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

fn env_flag(key: &str) -> bool {
    matches!(
        env_string(key).to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "ja"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, CookieSecure, LocalJobs, RunnerMode};
    use crate::db;
    use std::io::Write;

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

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("koreograf-{label}-{nanos}"))
    }

    fn project(conn: &Connection, source: &str) {
        let team: String = conn
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        conn.execute(
            "INSERT INTO projects (id, team_id, name, source_type, source, created_at)
             VALUES ('import-project', ?1, 'Import', 'path', ?2, '2026-01-01T00:00:00Z')",
            params![team, source],
        )
        .expect("project");
    }

    fn source_with_wal(label: &str) -> (PathBuf, crate::Shared) {
        let dir = temp_dir(label);
        let state = crate::boot(config(&dir)).expect("boot");
        let team: String = state
            .db()
            .expect("db")
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        let state_file = dir
            .join("cache")
            .join(&team)
            .join("tofu-state")
            .join("tpl-1")
            .join("terraform.tfstate");
        fs::create_dir_all(state_file.parent().expect("parent")).expect("state dir");
        fs::write(&state_file, b"{\"serial\":1}").expect("state");
        {
            let db = state.db().expect("db");
            db.pragma_update(None, "wal_autocheckpoint", 0)
                .expect("checkpoint");
            project(&db, "/home/user/projects/demo");
        }
        assert!(dir.join("koreograf.db-wal").is_file());
        (dir, state)
    }

    #[test]
    fn startup_restore_runs_in_an_empty_directory_and_renames_the_marker() {
        let source = temp_dir("restore-source");
        let state = crate::boot(config(&source)).expect("boot");
        let archive = backup::export(&state, true).expect("export");
        drop(state);
        let marker = source.join("restore.zip");
        fs::write(&marker, &archive).expect("marker");

        let dest = temp_dir("restore-dest");
        fs::create_dir_all(&dest).expect("dest");
        let options = Options {
            restore_from: Some(marker.clone()),
            ..Options::default()
        };
        prepare_with(&config(&dest), &options).expect("restore");
        assert!(dest.join(DATABASE).is_file());
        assert!(dest.join(SECRET).is_file());
        assert!(!marker.exists());
        assert!(PathBuf::from(format!("{}.applied", marker.display())).is_file());
        prepare_with(&config(&dest), &options).expect("second start is ignored");
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn startup_restore_and_import_are_ignored_when_the_directory_has_data() {
        let dest = temp_dir("nonempty");
        fs::create_dir_all(&dest).expect("dest");
        fs::write(dest.join("keep.txt"), b"keep").expect("keep");
        let options = Options {
            restore_from: Some(dest.join("missing.zip")),
            import_from: Some(dest.join("missing-import")),
            ..Options::default()
        };
        prepare_with(&config(&dest), &options).expect("ignored");
        assert_eq!(fs::read(dest.join("keep.txt")).expect("keep"), b"keep");
        assert!(!dest.join(DATABASE).exists());
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn startup_restore_reports_an_unknown_manifest_format() {
        let source = temp_dir("restore-format-source");
        let state = crate::boot(config(&source)).expect("boot");
        let archive = backup::export(&state, false).expect("export");
        drop(state);
        let parts = {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).expect("zip");
            let mut out = Vec::new();
            for index in 0..zip.len() {
                let mut file = zip.by_index(index).expect("entry");
                let name = file.name().expect("name").to_string();
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut bytes).expect("read");
                out.push((name, bytes));
            }
            out
        };
        let dest = temp_dir("restore-format-dest");
        fs::create_dir_all(&dest).expect("dest");
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, bytes) in &parts {
                writer.start_file(name.as_str(), options).expect("file");
                if name == "manifest.json" {
                    writer
                        .write_all(br#"{"format":7,"engine":"sqlite","created_at":"","koreograf":"x","includes":[],"omits":[]}"#)
                        .expect("write");
                } else {
                    writer.write_all(bytes).expect("write");
                }
            }
            writer.finish().expect("finish");
        }
        let file = source.join("bad.zip");
        fs::write(&file, cursor.into_inner()).expect("bad zip");
        let options = Options {
            restore_from: Some(file),
            ..Options::default()
        };
        let err = prepare_with(&config(&dest), &options).expect_err("format");
        assert!(
            format!("{err:#}").contains("unknown manifest format"),
            "{err:#}"
        );
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn import_copies_an_active_wal_database_the_key_the_state_and_applies_the_map_once() {
        let (source, state) = source_with_wal("import-wal");
        let dest = temp_dir("import-wal-dest");
        let mapped_root = temp_dir("import-mapped");
        fs::create_dir_all(mapped_root.join("demo")).expect("mapped");
        let options = Options {
            import_from: Some(source.clone()),
            path_map: vec![(PathBuf::from("/home/user/projects"), mapped_root.clone())],
            ..Options::default()
        };
        let report = import_directory(&config(&dest), &source, &options).expect("import");
        assert_eq!(report.missing_paths, Vec::<String>::new());
        assert_eq!(report.tofu_files, 1);
        assert!(dest.join(SECRET).is_file());
        let conn = Connection::open(dest.join(DATABASE)).expect("dest db");
        let stored: String = conn
            .query_row(
                "SELECT source FROM projects WHERE id = 'import-project'",
                [],
                |row| row.get(0),
            )
            .expect("project");
        assert_eq!(stored, mapped_root.join("demo").display().to_string());
        let state_file = {
            let team: String = conn
                .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
                .expect("team");
            dest.join("cache")
                .join(team)
                .join("tofu-state")
                .join("tpl-1")
                .join("terraform.tfstate")
        };
        assert_eq!(fs::read(state_file).expect("state"), b"{\"serial\":1}");
        let applied = apply_path_map(
            &conn,
            &[(
                PathBuf::from("/home/user/projects"),
                PathBuf::from("/other"),
            )],
        )
        .expect("second map");
        assert_eq!(applied, 0);
        let stored: String = conn
            .query_row(
                "SELECT source FROM projects WHERE id = 'import-project'",
                [],
                |row| row.get(0),
            )
            .expect("project");
        assert_eq!(stored, mapped_root.join("demo").display().to_string());
        drop(conn);
        assert!(report_or_none(&dest).is_some());
        drop(state);
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
        let _ = fs::remove_dir_all(mapped_root);
    }

    #[cfg(unix)]
    #[test]
    fn import_reads_a_stopped_database_from_a_read_only_directory() {
        use std::os::unix::fs::PermissionsExt;

        let source = temp_dir("import-stopped");
        {
            let state = crate::boot(config(&source)).expect("boot");
            project(&state.db().expect("db"), "/home/user/projects/demo");
        }
        assert!(!source.join("koreograf.db-wal").exists());
        assert!(!source.join("koreograf.db-shm").exists());
        fs::set_permissions(&source, fs::Permissions::from_mode(0o555)).expect("read-only");

        let dest = temp_dir("import-stopped-dest");
        let options = Options {
            import_from: Some(source.clone()),
            ..Options::default()
        };
        let report = import_directory(&config(&dest), &source, &options).expect("import");
        assert!(report
            .missing_paths
            .iter()
            .any(|path| path.contains("/demo")));
        let conn = Connection::open(dest.join(DATABASE)).expect("dest db");
        let stored: String = conn
            .query_row(
                "SELECT source FROM projects WHERE id = 'import-project'",
                [],
                |row| row.get(0),
            )
            .expect("project");
        assert_eq!(stored, "/home/user/projects/demo");
        drop(conn);

        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).expect("writable");
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_import_leaves_no_partial_data_and_the_next_start_retries() {
        use std::os::unix::fs::PermissionsExt;

        if nix::unistd::Uid::effective().is_root() {
            return;
        }
        let (source, state) = source_with_wal("import-failure");
        let team: String = state
            .db()
            .expect("db")
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        let state_file = source
            .join("cache")
            .join(&team)
            .join("tofu-state")
            .join("tpl-1")
            .join("terraform.tfstate");
        fs::set_permissions(&state_file, fs::Permissions::from_mode(0o000)).expect("unreadable");

        let dest = temp_dir("import-failure-dest");
        let options = Options {
            import_from: Some(source.clone()),
            ..Options::default()
        };
        prepare_with(&config(&dest), &options).expect_err("unreadable state");
        assert!(!dest.join(DATABASE).exists());
        assert!(!dest.join(SECRET).exists());
        assert_eq!(fs::read_dir(&dest).expect("dest").count(), 0);

        prepare_with(&config(&dest), &options).expect_err("second start retries");
        assert!(!dest.join(DATABASE).exists());
        assert!(!dest.join(SECRET).exists());

        fs::set_permissions(&state_file, fs::Permissions::from_mode(0o644)).expect("readable");
        prepare_with(&config(&dest), &options).expect("third start imports");
        assert!(dest.join(DATABASE).is_file());
        assert!(dest.join(SECRET).is_file());
        assert!(report_or_none(&dest).is_some());
        assert!(dest
            .join("cache")
            .join(&team)
            .join("tofu-state")
            .join("tpl-1")
            .join("terraform.tfstate")
            .is_file());

        drop(state);
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    fn report_or_none(dir: &Path) -> Option<Value> {
        let conn = Connection::open(dir.join(DATABASE)).ok()?;
        report(&conn).ok().flatten()
    }

    #[test]
    fn import_refuses_a_directory_without_secret_key() {
        let source = temp_dir("import-no-key");
        {
            let state = crate::boot(config(&source)).expect("boot");
            drop(state);
        }
        fs::remove_file(source.join(SECRET)).expect("remove key");
        let dest = temp_dir("import-no-key-dest");
        let options = Options {
            import_from: Some(source.clone()),
            ..Options::default()
        };
        let err = import_directory(&config(&dest), &source, &options).expect_err("key");
        assert!(format!("{err:#}").contains("secret.key"), "{err:#}");
        assert!(!dest.join(DATABASE).exists());
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn import_refuses_a_newer_schema() {
        let source = temp_dir("import-newer");
        {
            let state = crate::boot(config(&source)).expect("boot");
            let conn = state.db().expect("db");
            conn.pragma_update(None, "user_version", db::SCHEMA_VERSION + 1)
                .expect("version");
        }
        let dest = temp_dir("import-newer-dest");
        let options = Options {
            import_from: Some(source.clone()),
            ..Options::default()
        };
        let err = import_directory(&config(&dest), &source, &options).expect_err("newer");
        assert!(format!("{err:#}").contains("Upgrade Koreograf"), "{err:#}");
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn the_path_map_needs_two_absolute_paths() {
        assert_eq!(parse_path_map("").expect("empty"), Vec::new());
        assert_eq!(
            parse_path_map("/old=/new,/a=/b").expect("map"),
            vec![
                (PathBuf::from("/old"), PathBuf::from("/new")),
                (PathBuf::from("/a"), PathBuf::from("/b")),
            ]
        );
        assert!(parse_path_map("/old").is_err());
        assert!(parse_path_map("old=/new").is_err());
        assert!(parse_path_map("/old=new").is_err());
    }
}
