//! Saved admin settings, installation archives and imports, the experimental
//! PostgreSQL migration, and the Prometheus scrape.

mod model;
mod routes;
mod service;
pub(crate) mod store;

#[cfg(test)]
mod tests;

pub use model::DATABASE_CHECK_IDS;
pub(crate) use model::{JobMeta, ProxySettings, Runtime, DEFAULT_ENVIRONMENT_KEYS, REPORT_KEY};
pub(crate) use routes::routes;
#[cfg(test)]
pub(crate) use service::{audit_entry, audit_line, export, parse_form_target, restore, target_url};
pub(crate) use service::{
    busy, configured_mode, copy_database, job_labels, load, log_cache, prepare, report, scrape,
    Metrics,
};
pub use service::{db_migrate, db_migrate_requested};
