//! Koreograf runs team-scoped automation jobs.
//!
//! The binary is the whole prototype: a small HTTP API, a browser UI, and a
//! job runner that keeps Ansible collections, roles and OpenTofu plugins in a
//! persistent cache. This is not a feature-complete AWX replacement.

#![forbid(unsafe_code)]

mod askpass;
mod auth;
mod backup;
mod cacheinfo;
mod config;
mod credentials;
mod crypto;
mod db;
mod entrypoint;
mod error;
mod health;
mod hooks;
mod http;
mod inventory;
mod jobs;
mod ldap;
pub mod mcp;
mod metrics;
mod migration;
mod netbox;
mod preflight;
mod runner;
mod scan;
mod schedule;
mod seed;
mod settings;
mod tls;
mod workflow;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::ApiError;
use crate::runner::RunningProc;

pub use askpass::{askpass, askpass_kind, git_ssh, git_ssh_requested, AskpassKind};
pub use config::{Config, CookieSecure, LdapConfig, LocalJobs, RunnerMode};
pub use entrypoint::{entrypoint, entrypoint_requested};
pub use health::{healthcheck, healthcheck_requested};

/// Shared process state. Cheap to clone because it is an `Arc`.
pub struct AppState {
    pub(crate) conn: Mutex<Connection>,
    pub cfg: Config,
    pub(crate) cipher: Mutex<crypto::Cipher>,
    pub(crate) metrics: metrics::Metrics,
    /// One backup or restore at a time. Take this before the database lock.
    pub(crate) maintenance: Mutex<()>,
    pub(crate) procs: Mutex<HashMap<String, RunningProc>>,
    /// Jobs whose later output is discarded. Set when cancel commits.
    pub(crate) canceled: Mutex<HashSet<String>>,
    pub(crate) preflight: Mutex<Option<preflight::Report>>,
}

pub type Shared = Arc<AppState>;

/// Version from `Cargo.toml` at compile time.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Short git SHA recorded by the build in `KOREOGRAF_GIT_SHA`. Empty values
/// count as unset so builds without a checkout still work.
pub fn git_commit() -> Option<String> {
    std::env::var("KOREOGRAF_GIT_SHA")
        .ok()
        .filter(|value| !value.is_empty())
}

/// Build date recorded by the build in `KOREOGRAF_BUILT_AT`. Empty values
/// count as unset.
pub fn built_at() -> Option<String> {
    std::env::var("KOREOGRAF_BUILT_AT")
        .ok()
        .filter(|value| !value.is_empty())
}

/// JSON payload for `/api/version`. The commit is always present and falls
/// back to `unknown`; the build date appears only when the build recorded it.
pub fn version_payload() -> serde_json::Value {
    let mut payload = serde_json::json!({
        "version": version(),
        "commit": git_commit().unwrap_or_else(|| "unknown".to_string()),
    });
    if let Some(built) = built_at() {
        payload["built"] = serde_json::json!(built);
    }
    payload
}

/// One line for `--version`. The commit and build date appear only when the
/// build recorded them, in parentheses after the version.
pub fn version_line() -> String {
    let mut details = Vec::new();
    if let Some(commit) = git_commit() {
        details.push(commit);
    }
    if let Some(built) = built_at() {
        details.push(format!("built {built}"));
    }
    if details.is_empty() {
        version().to_string()
    } else {
        format!("{} ({})", version(), details.join(", "))
    }
}

/// True when argv asks for [`version`]. The first value is the program name.
pub fn version_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .skip(1)
        .any(|arg| arg.as_ref() == "--version")
}

/// True when argv asks for `doctor`. The first value is the program name.
pub fn doctor_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().skip(1).any(|arg| arg.as_ref() == "doctor")
}

/// Print the runner preflight and return the process exit code.
pub async fn doctor() -> anyhow::Result<i32> {
    let cfg = Config::from_env();
    let mode = settings::configured_mode(&cfg);
    let report = preflight::check(mode, cfg.container_image).await;
    println!("{}", report.log_block());
    Ok(if report.ok() { 0 } else { 2 })
}

impl AppState {
    pub fn db(&self) -> Result<std::sync::MutexGuard<'_, Connection>, ApiError> {
        self.conn
            .lock()
            .map_err(|_| ApiError::Internal("database lock is damaged".into()))
    }

    pub(crate) fn preflight_report(&self) -> Option<preflight::Report> {
        self.preflight.lock().ok().and_then(|report| report.clone())
    }

    pub(crate) fn set_preflight_report(&self, report: preflight::Report) {
        if let Ok(mut slot) = self.preflight.lock() {
            *slot = Some(report);
        }
    }

    pub(crate) fn encrypt(&self, plain: &str) -> Result<String, ApiError> {
        self.cipher
            .lock()
            .map_err(|_| ApiError::Internal("key lock is damaged".into()))?
            .encrypt(plain)
    }

    pub(crate) fn decrypt(&self, packed: &str) -> Result<String, ApiError> {
        self.cipher
            .lock()
            .map_err(|_| ApiError::Internal("key lock is damaged".into()))?
            .decrypt(packed)
    }

    pub(crate) fn replace_cipher(&self, cipher: crypto::Cipher) -> Result<(), ApiError> {
        let mut guard = self
            .cipher
            .lock()
            .map_err(|_| ApiError::Internal("key lock is damaged".into()))?;
        *guard = cipher;
        Ok(())
    }
}

/// Open the database, load the encryption key, and seed the demo once.
pub fn boot(mut cfg: Config) -> anyhow::Result<Shared> {
    std::fs::create_dir_all(&cfg.data_dir)?;
    // A relative data directory would be resolved again from the job work
    // directory. Local runs then miss wrapper.sh and the other job files.
    cfg.data_dir = std::path::absolute(&cfg.data_dir)?;
    runner::scrub_job_dirs(&cfg.data_dir);
    let conn = db::open(&cfg.data_dir.join("koreograf.db"))?;
    let cipher = crypto::load_cipher(&cfg.data_dir)?;
    seed::seed(&conn, &cfg)?;
    cfg.admin_password = None;
    let state = Arc::new(AppState {
        conn: Mutex::new(conn),
        cfg,
        cipher: Mutex::new(cipher),
        metrics: metrics::Metrics::new(),
        maintenance: Mutex::new(()),
        procs: Mutex::new(HashMap::new()),
        canceled: Mutex::new(HashSet::new()),
        preflight: Mutex::new(None),
    });
    credentials::backfill_netbox_urls(&state)?;
    Ok(state)
}

/// Bind the HTTP server and serve until it stops.
pub async fn serve(cfg: Config) -> anyhow::Result<()> {
    let url = std::env::var("KOREOGRAF_DATABASE_URL").unwrap_or_default();
    config::database_choice(&url).map_err(anyhow::Error::msg)?;
    if let Err(error) = tls::validate_company_cas() {
        tracing::error!(%error, "Company CA directory is invalid");
        return Err(anyhow::Error::msg(error));
    }
    migration::prepare(&cfg)?;
    let state = boot(cfg)?;
    let mode = match settings::load(&state) {
        Ok(effective) => {
            tls::warn_directory(&effective.ldap);
            effective.runner
        }
        Err(_) => state.cfg.runner,
    };
    let report = preflight::check(mode, state.cfg.container_image).await;
    tracing::info!("{}", report.log_block());
    state.set_preflight_report(report);
    schedule::spawn(state.clone());
    let bind = state.cfg.bind.clone();
    let data_dir = state.cfg.data_dir.display().to_string();
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, %data_dir, "Koreograf is ready");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Axum router, including the API and the browser UI.
pub fn router(state: Shared) -> axum::Router {
    http::router(state)
}

#[cfg(test)]
mod version_tests {
    use super::{built_at, git_commit, version, version_line, version_payload, version_requested};

    #[test]
    fn the_version_flag_is_the_long_option_after_the_program_name() {
        assert!(version_requested(["koreograf", "--version"]));
        assert!(version_requested([
            "koreograf",
            "--bind",
            "127.0.0.1:9",
            "--version"
        ]));
        assert!(!version_requested(["koreograf"]));
        assert!(!version_requested(["koreograf", "--help"]));
        assert!(!version_requested(["koreograf", "--versioned"]));
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }

    // The tests in this module share the process environment, so they run as
    // one ordered sequence instead of parallel threads.
    #[test]
    fn the_version_reports_the_build_information_from_the_environment() {
        std::env::remove_var("KOREOGRAF_GIT_SHA");
        std::env::remove_var("KOREOGRAF_BUILT_AT");
        assert!(git_commit().is_none());
        assert!(built_at().is_none());
        let payload = version_payload();
        assert_eq!(payload["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(payload["commit"], "unknown");
        assert!(payload.get("built").is_none());
        assert_eq!(version_line(), env!("CARGO_PKG_VERSION"));

        std::env::set_var("KOREOGRAF_GIT_SHA", "");
        std::env::set_var("KOREOGRAF_BUILT_AT", "");
        assert_eq!(version_payload()["commit"], "unknown");
        assert_eq!(version_line(), env!("CARGO_PKG_VERSION"));

        std::env::set_var("KOREOGRAF_GIT_SHA", "4cf8792");
        std::env::set_var("KOREOGRAF_BUILT_AT", "2026-10-28");
        let payload = version_payload();
        assert_eq!(payload["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(payload["commit"], "4cf8792");
        assert_eq!(payload["built"], "2026-10-28");
        assert_eq!(
            version_line(),
            format!("{} (4cf8792, built 2026-10-28)", env!("CARGO_PKG_VERSION"))
        );

        std::env::remove_var("KOREOGRAF_BUILT_AT");
        assert_eq!(
            version_line(),
            format!("{} (4cf8792)", env!("CARGO_PKG_VERSION"))
        );
        std::env::remove_var("KOREOGRAF_GIT_SHA");
        std::env::set_var("KOREOGRAF_BUILT_AT", "2026-10-28");
        assert_eq!(
            version_line(),
            format!("{} (built 2026-10-28)", env!("CARGO_PKG_VERSION"))
        );
        std::env::remove_var("KOREOGRAF_BUILT_AT");
    }
}
