//! Koreograf runs team-scoped automation jobs.
//!
//! The binary is the whole prototype: a small HTTP API, a browser UI, and a
//! job runner that keeps Ansible collections, roles and OpenTofu plugins in a
//! persistent cache. This is not a feature-complete AWX replacement.

#![forbid(unsafe_code)]

mod auth;
mod backup;
mod cacheinfo;
mod config;
mod credentials;
mod crypto;
mod db;
mod error;
mod hooks;
mod http;
mod inventory;
mod jobs;
mod ldap;
mod metrics;
mod netbox;
mod runner;
mod scan;
mod seed;
mod settings;
mod workflow;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::ApiError;
use crate::runner::RunningProc;

pub use config::{Config, LdapConfig, RunnerMode};

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
}

pub type Shared = Arc<AppState>;

impl AppState {
    pub fn db(&self) -> Result<std::sync::MutexGuard<'_, Connection>, ApiError> {
        self.conn
            .lock()
            .map_err(|_| ApiError::Internal("database lock is damaged".into()))
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
    let conn = db::open(&cfg.data_dir.join("koreograf.db"))?;
    let cipher = crypto::load_cipher(&cfg.data_dir)?;
    seed::seed(&conn, &cfg)?;
    cfg.admin_password = None;
    Ok(Arc::new(AppState {
        conn: Mutex::new(conn),
        cfg,
        cipher: Mutex::new(cipher),
        metrics: metrics::Metrics::new(),
        maintenance: Mutex::new(()),
        procs: Mutex::new(HashMap::new()),
        canceled: Mutex::new(HashSet::new()),
    }))
}

/// Bind the HTTP server and serve until it stops.
pub async fn serve(cfg: Config) -> anyhow::Result<()> {
    let url = std::env::var("KOREOGRAF_DATABASE_URL").unwrap_or_default();
    config::database_choice(&url).map_err(anyhow::Error::msg)?;
    let state = boot(cfg)?;
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
