use std::path::PathBuf;

use serde::Serialize;

use crate::config::{LdapConfig, LlmProvider, PostgresConfig, RunnerMode};

pub(crate) const DEFAULT_ENVIRONMENT_KEYS: [&str; 3] = [
    "default_environment_ansible",
    "default_environment_opentofu",
    "default_environment_shell",
];

#[derive(Clone)]
pub struct ProxySettings {
    pub url: String,
    pub username: String,
    pub password: String,
    pub no_proxy: String,
    /// `true` after an administrator has saved. Callers then replace the
    /// process environment instead of inheriting it.
    pub explicit: bool,
}

pub struct Effective {
    /// Directory fields for the form. Login uses [`Effective::directory`].
    pub ldap: LdapConfig,
    pub proxy: ProxySettings,
    pub runner: RunnerMode,
    pub job_timeout_secs: u64,
    pub job_output_lines: u64,
    pub default_environment_ansible: String,
    pub default_environment_opentofu: String,
    pub default_environment_shell: String,
    pub llm_enabled: bool,
    pub llm_provider: LlmProvider,
    pub llm_url: String,
    pub llm_model: String,
    pub llm_api_key: String,
    pub saved: bool,
    pub unsupported_proxy: bool,
}

#[derive(Clone)]
pub struct SettingsWrite {
    pub ldap_url: String,
    pub ldap_mode: String,
    pub ldap_bind_dn: String,
    pub ldap_bind_password: String,
    pub ldap_user_base: String,
    pub ldap_user_filter: String,
    pub ldap_username_attr: String,
    pub ldap_group_attr: String,
    pub ldap_group_base: String,
    pub ldap_starttls: bool,
    pub ldap_tls_insecure: bool,
    pub proxy_url: String,
    pub proxy_username: String,
    pub proxy_password: String,
    pub proxy_no_proxy: String,
    pub runner: String,
    pub job_timeout_secs: u64,
    pub job_output_lines: u64,
    pub default_environment_ansible: String,
    pub default_environment_opentofu: String,
    pub default_environment_shell: String,
    pub clear_ldap_password: bool,
    pub clear_proxy_password: bool,
    pub llm: Option<LlmWrite>,
    pub reset: bool,
}

#[derive(Clone)]
pub struct LlmWrite {
    pub enabled: bool,
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub clear_api_key: bool,
}

#[derive(Debug)]
pub(crate) struct RestoreSummary {
    pub(crate) koreograf: String,
    pub(crate) includes: Vec<String>,
}

pub(crate) const REPORT_KEY: &str = "import_report";

#[derive(Clone, Default)]
pub(crate) struct Options {
    pub(crate) restore_from: Option<PathBuf>,
    pub(crate) import_from: Option<PathBuf>,
    pub(crate) path_map: Vec<(PathBuf, PathBuf)>,
    pub(crate) copy_cache: bool,
}

#[derive(Debug)]
pub(crate) struct ImportReport {
    pub(crate) missing_paths: Vec<String>,
    pub(crate) tofu_files: usize,
    pub(crate) path_map: Vec<String>,
}

pub const DATABASE_CHECK_IDS: [&str; 4] = ["reachable", "tls", "server_version", "rights"];

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CheckResult {
    pub id: &'static str,
    pub ok: bool,
    pub key: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TableProgress {
    pub table: &'static str,
    pub rows: i64,
    pub done: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct AuditEntry {
    pub user: String,
    pub at: String,
    pub host: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct MigrationStatus {
    pub running: bool,
    pub done: bool,
    pub failed: bool,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub target_host: String,
    pub url: String,
    pub rows: i64,
    pub error: Option<&'static str>,
    pub tables: Vec<TableProgress>,
    pub audit: Option<AuditEntry>,
}

pub(crate) struct PendingTarget {
    pub id: String,
    pub config: PostgresConfig,
}

#[derive(Default)]
pub(crate) struct Runtime {
    pub pending: Option<PendingTarget>,
    pub status: MigrationStatus,
    pub maintenance_since: Option<String>,
}

pub(crate) struct JobMeta<'a> {
    pub job_id: &'a str,
    pub team_id: &'a str,
    pub tool: &'a str,
    pub playbook: &'a str,
    pub template: &'a str,
}

pub(crate) struct JobLabels {
    pub tool: String,
    pub playbook: String,
    pub team_id: String,
    pub template: String,
}

#[derive(Clone, Debug)]
pub(crate) struct CacheGauge {
    pub team_id: String,
    pub team: String,
    pub collections: u64,
    pub roles: u64,
    pub collections_bytes: u64,
    pub roles_bytes: u64,
    pub tofu_bytes: u64,
}
