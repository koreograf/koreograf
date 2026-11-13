//! Saved admin settings, installation archives and imports, the experimental
//! PostgreSQL migration, and the Prometheus scrape.

mod model;
mod routes;
mod service;
pub(crate) mod store;

#[cfg(test)]
mod tests;

pub use model::DATABASE_CHECK_IDS;
pub(crate) use model::{
    CheckResult, JobMeta, MigrationStatus, PendingTarget, ProxySettings, Runtime,
    DEFAULT_ENVIRONMENT_KEYS, REPORT_KEY,
};
pub(crate) use routes::routes;
pub(crate) use service::{
    all_passed, audit_entry, audit_line, busy, check_source, configured_mode, copy_database,
    copy_source, export, job_labels, load, log_cache, parse_form_target, precheck, prepare, report,
    restore, run_checks, scrape, target_url, Metrics,
};
pub use service::{db_migrate, db_migrate_requested};
