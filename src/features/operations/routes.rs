use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, SecondsFormat, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::model::{
    CheckResult, Effective, LlmWrite, MigrationStatus, PendingTarget, Runtime, SettingsWrite,
};
use super::service::{save, view};
use crate::cacheinfo::{self, CacheEntry};
use crate::db::{self, Delivery};
use crate::error::{ApiError, ApiJson, AppResult};
use crate::features::identity as auth;
use crate::features::identity::{AuthUser, Role};
use crate::http::{gate, team_header};
use crate::preflight;
use crate::runner;
use crate::Shared;

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/settings", get(get_settings).put(put_settings))
        .route("/overview", get(overview))
        .route("/dashboard", get(dashboard))
        .route("/runner", get(runner_status))
        .route("/runner/preflight", get(runner_preflight))
        .route("/cache", get(cache))
        .route("/plugins", get(plugins))
        .merge(admin_routes())
}

#[derive(Serialize)]
pub(super) struct SettingsResponse {
    source: String,
    ldap: LdapSettingsResponse,
    proxy: ProxySettingsResponse,
    runner: String,
    job_timeout_secs: u64,
    job_output_lines: u64,
    default_environment_ansible: String,
    default_environment_opentofu: String,
    default_environment_shell: String,
    llm: LlmSettingsResponse,
}

#[derive(Serialize)]
pub(super) struct LdapSettingsResponse {
    url: String,
    mode: String,
    bind_dn: String,
    bind_password_set: bool,
    user_base: String,
    user_filter: String,
    username_attr: String,
    group_attr: String,
    group_base: String,
    starttls: bool,
    tls_insecure: bool,
}

#[derive(Serialize)]
pub(super) struct ProxySettingsResponse {
    url: String,
    username: String,
    password_set: bool,
    no_proxy: String,
    unsupported_env: bool,
}

#[derive(Serialize)]
pub(super) struct LlmSettingsResponse {
    enabled: bool,
    provider: String,
    base_url: String,
    model: String,
    api_key_set: bool,
}

impl From<&Effective> for SettingsResponse {
    fn from(effective: &Effective) -> Self {
        Self {
            source: if effective.saved { "saved" } else { "env" }.to_string(),
            ldap: LdapSettingsResponse {
                url: effective.ldap.url.clone(),
                mode: effective.ldap.mode.clone(),
                bind_dn: effective.ldap.bind_dn.clone(),
                bind_password_set: !effective.ldap.bind_password.is_empty(),
                user_base: effective.ldap.user_base.clone(),
                user_filter: effective.ldap.user_filter.clone(),
                username_attr: effective.ldap.username_attr.clone(),
                group_attr: effective.ldap.group_attr.clone(),
                group_base: effective.ldap.group_base.clone(),
                starttls: effective.ldap.starttls,
                tls_insecure: effective.ldap.tls_insecure,
            },
            proxy: ProxySettingsResponse {
                url: effective.proxy.url.clone(),
                username: effective.proxy.username.clone(),
                password_set: !effective.proxy.password.is_empty(),
                no_proxy: effective.proxy.no_proxy.clone(),
                unsupported_env: effective.unsupported_proxy,
            },
            runner: effective.runner.as_str().to_string(),
            job_timeout_secs: effective.job_timeout_secs,
            job_output_lines: effective.job_output_lines,
            default_environment_ansible: effective.default_environment_ansible.clone(),
            default_environment_opentofu: effective.default_environment_opentofu.clone(),
            default_environment_shell: effective.default_environment_shell.clone(),
            llm: LlmSettingsResponse {
                enabled: effective.llm_enabled,
                provider: effective.llm_provider.as_str().to_string(),
                base_url: effective.llm_url.clone(),
                model: effective.llm_model.clone(),
                api_key_set: !effective.llm_api_key.is_empty(),
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SettingsBody {
    #[serde(default, rename = "source")]
    _source: Option<Value>,
    #[serde(default)]
    reset: bool,
    ldap: Option<LdapBody>,
    proxy: Option<ProxyBody>,
    runner: Option<String>,
    job_timeout_secs: Option<u64>,
    #[serde(default)]
    job_output_lines: u64,
    #[serde(default)]
    default_environment_ansible: String,
    #[serde(default)]
    default_environment_opentofu: String,
    #[serde(default)]
    default_environment_shell: String,
    #[serde(default)]
    clear: Vec<String>,
    llm: Option<LlmBody>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct LlmBody {
    #[serde(default, rename = "api_key_set")]
    _api_key_set: Option<Value>,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    api_key: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct LdapBody {
    #[serde(default, rename = "bind_password_set")]
    _bind_password_set: Option<Value>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    bind_dn: String,
    #[serde(default)]
    bind_password: String,
    #[serde(default)]
    user_base: String,
    #[serde(default)]
    user_filter: String,
    #[serde(default)]
    username_attr: String,
    #[serde(default)]
    group_attr: String,
    #[serde(default)]
    group_base: String,
    #[serde(default)]
    starttls: bool,
    #[serde(default)]
    tls_insecure: bool,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct ProxyBody {
    #[serde(default, rename = "password_set")]
    _password_set: Option<Value>,
    #[serde(default, rename = "unsupported_env")]
    _unsupported_env: Option<Value>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    no_proxy: String,
}

pub async fn get_settings(
    State(state): State<Shared>,
    user: AuthUser,
) -> AppResult<Json<SettingsResponse>> {
    admin(&user)?;
    Ok(Json(SettingsResponse::from(&view(&state)?)))
}

pub async fn put_settings(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<SettingsBody>,
) -> AppResult<Json<SettingsResponse>> {
    admin(&user)?;
    if !body.reset {
        check_complete(&body)?;
    }
    let ldap = body.ldap.unwrap_or_default();
    let proxy = body.proxy.unwrap_or_default();
    let clear_ldap_password = body.clear.iter().any(|item| item == "ldap_bind_password");
    let clear_proxy_password = body.clear.iter().any(|item| item == "proxy_password");
    let clear_api_key = body.clear.iter().any(|item| item == "llm_api_key");
    let llm = body.llm.map(|llm| LlmWrite {
        enabled: llm.enabled,
        provider: llm.provider,
        base_url: llm.base_url,
        model: llm.model,
        api_key: llm.api_key,
        clear_api_key,
    });
    let write = SettingsWrite {
        ldap_url: ldap.url,
        ldap_mode: ldap.mode,
        ldap_bind_dn: ldap.bind_dn,
        ldap_bind_password: ldap.bind_password,
        ldap_user_base: ldap.user_base,
        ldap_user_filter: ldap.user_filter,
        ldap_username_attr: ldap.username_attr,
        ldap_group_attr: ldap.group_attr,
        ldap_group_base: ldap.group_base,
        ldap_starttls: ldap.starttls,
        ldap_tls_insecure: ldap.tls_insecure,
        proxy_url: proxy.url,
        proxy_username: proxy.username,
        proxy_password: proxy.password,
        proxy_no_proxy: proxy.no_proxy,
        runner: body.runner.unwrap_or_default(),
        job_timeout_secs: body.job_timeout_secs.unwrap_or_default(),
        job_output_lines: body.job_output_lines,
        default_environment_ansible: body.default_environment_ansible,
        default_environment_opentofu: body.default_environment_opentofu,
        default_environment_shell: body.default_environment_shell,
        clear_ldap_password,
        clear_proxy_password,
        llm,
        reset: body.reset,
    };
    Ok(Json(SettingsResponse::from(&save(&state, &write)?)))
}

fn check_complete(body: &SettingsBody) -> AppResult<()> {
    let mut missing = Vec::new();
    if body.ldap.is_none() {
        missing.push("ldap");
    }
    if body.proxy.is_none() {
        missing.push("proxy");
    }
    if body.runner.as_deref().unwrap_or("").trim().is_empty() {
        missing.push("runner");
    }
    if body.job_timeout_secs.is_none() {
        missing.push("job_timeout_secs");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ApiError::bad_vars(
            "the_settings_body_is_incomplete",
            [("fields", missing.join(", "))],
        ))
    }
}

#[derive(Default, Deserialize)]
pub struct DashboardQuery {
    #[serde(default)]
    days: String,
    #[serde(default)]
    tz: String,
    #[serde(default)]
    timezone: String,
}

#[derive(Serialize)]
pub(super) struct RunnerStatusResponse {
    runner_mode: &'static str,
    local_jobs: &'static str,
    runtime: Option<String>,
    container_image: bool,
    local_tools: LocalToolsResponse,
    default_environment_ansible: String,
    default_environment_opentofu: String,
    default_environment_shell: String,
}

#[derive(Serialize)]
struct LocalToolsResponse {
    ansible: bool,
    opentofu: bool,
    shell: bool,
}

#[derive(Serialize)]
pub(super) struct PreflightResponse {
    mode: &'static str,
    container_image: bool,
    ok: bool,
    tools: Vec<preflight::ToolStatus>,
    runtime: preflight::RuntimeStatus,
    missing: Vec<preflight::Missing>,
    messages: Vec<String>,
}

impl PreflightResponse {
    fn new(report: &preflight::Report) -> Self {
        Self {
            mode: report.mode,
            container_image: report.container_image,
            ok: report.ok(),
            tools: report.tools.clone(),
            runtime: report.runtime.clone(),
            missing: report.missing.clone(),
            messages: report.messages(),
        }
    }
}

#[derive(Serialize)]
pub(super) struct OverviewResponse {
    runner_mode: &'static str,
    runtime: Option<String>,
    preflight: Option<PreflightResponse>,
    import: Option<Value>,
    ldap: bool,
    cache_bytes: u64,
    jobs: Vec<Value>,
    deliveries: Vec<Delivery>,
    workflow_id: Option<String>,
    workflow_name: Option<String>,
    hook_id: Option<String>,
    hook_name: Option<String>,
}

#[derive(Serialize)]
pub(super) struct DashboardResponse {
    range_days: i64,
    timezone: String,
    from: String,
    to: String,
    series: Vec<DashboardSeriesResponse>,
    cache: DashboardCacheResponse,
}

#[derive(Serialize)]
struct DashboardSeriesResponse {
    date: String,
    started: u64,
    successful: u64,
    failed: u64,
    canceled: u64,
}

#[derive(Serialize)]
struct DashboardCacheResponse {
    collections: usize,
    roles: usize,
    tofu_bytes: u64,
}

#[derive(Serialize)]
pub(super) struct CacheResponse {
    collections_bytes: u64,
    roles_bytes: u64,
    tofu_bytes: u64,
    collections: Vec<CacheEntry>,
    roles: Vec<CacheEntry>,
    note: &'static str,
}

#[derive(Serialize)]
pub(super) struct PluginResponse {
    id: &'static str,
    summary: &'static str,
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct PluginListResponse(Vec<PluginResponse>);

pub async fn runner_status(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<RunnerStatusResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    // The probe awaits. Load settings only after it returns.
    let effective = super::service::load(&state)?;
    let report = crate::preflight::check(effective.runner, state.cfg.container_image).await;
    let runtime = if report.runtime.responding {
        report.runtime.binary.clone()
    } else {
        None
    };
    Ok(Json(RunnerStatusResponse {
        runner_mode: effective.runner.as_str(),
        local_jobs: state.cfg.local_jobs.as_str(),
        runtime,
        container_image: report.container_image,
        local_tools: LocalToolsResponse {
            ansible: report.tool_ready("ansible"),
            opentofu: report.tool_ready("opentofu"),
            shell: report.tool_ready("shell"),
        },
        default_environment_ansible: effective.default_environment_ansible,
        default_environment_opentofu: effective.default_environment_opentofu,
        default_environment_shell: effective.default_environment_shell,
    }))
}

pub async fn runner_preflight(
    State(state): State<Shared>,
    user: AuthUser,
) -> AppResult<Json<PreflightResponse>> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    let effective = super::service::load(&state)?;
    let report = crate::preflight::check(effective.runner, state.cfg.container_image).await;
    let response = PreflightResponse::new(&report);
    state.set_preflight_report(report);
    Ok(Json(response))
}

pub async fn overview(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<OverviewResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    // The preflight awaits. Take it before the database lock so the
    // connection guard, which is not Send, is never held across that point.
    let effective = super::service::load(&state)?;
    let runtime = runner::detect_runtime().await;
    let preflight = if user.is_admin {
        state.preflight_report()
    } else {
        None
    };
    let jobs = state.store().recent_jobs(&team, 8)?;
    let deliveries = state.store().list_deliveries(&team, 8)?;
    let workflow = state.store().first_workflow(&team)?;
    let hook = state.store().first_hook(&team)?;
    let cache = cacheinfo::team_cache(&state.cfg.data_dir, &team);
    let import = if user.is_admin {
        state.store().import_report()?
    } else {
        None
    };
    Ok(Json(OverviewResponse {
        runner_mode: effective.runner.as_str(),
        runtime,
        preflight: preflight.as_ref().map(PreflightResponse::new),
        import,
        ldap: effective.directory().is_some(),
        cache_bytes: cache.collections_bytes + cache.roles_bytes + cache.tofu_bytes,
        jobs: jobs
            .iter()
            .map(|job| crate::http::job_json(job, None))
            .collect(),
        deliveries,
        workflow_id: workflow.as_ref().map(|item| item.id.clone()),
        workflow_name: workflow.as_ref().map(|item| item.name.clone()),
        hook_id: hook.as_ref().map(|item| item.id.clone()),
        hook_name: hook.as_ref().map(|item| item.name.clone()),
    }))
}

#[derive(Clone, Default)]
struct DashboardDay {
    started: u64,
    successful: u64,
    failed: u64,
    canceled: u64,
}

fn parse_utc(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|stamp| stamp.with_timezone(&Utc))
}

fn day_offset(first: NaiveDate, date: NaiveDate, days: i64) -> Option<usize> {
    let offset = (date - first).num_days();
    if (0..days).contains(&offset) {
        Some(offset as usize)
    } else {
        None
    }
}

pub async fn dashboard(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Query(query): Query<DashboardQuery>,
) -> AppResult<Json<DashboardResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let days = query.days.trim().parse::<i64>().unwrap_or(1).clamp(1, 14);
    let zone_name = if query.tz.trim().is_empty() {
        query.timezone.trim()
    } else {
        query.tz.trim()
    };
    let timezone: Tz = zone_name.parse().unwrap_or(Tz::UTC);
    let now = Utc::now();
    let today = now.with_timezone(&timezone).date_naive();
    let first = today - ChronoDuration::days(days - 1);
    let start = first
        .and_hms_opt(0, 0, 0)
        .and_then(|local| timezone.from_local_datetime(&local).earliest())
        .map(|local| local.with_timezone(&Utc))
        .unwrap_or(now - ChronoDuration::days(days));
    let from = start.to_rfc3339_opts(SecondsFormat::Secs, true);
    let to = now.to_rfc3339_opts(SecondsFormat::Secs, true);
    let activity = state.store().job_activity(&team, &from, &to)?;
    let cache = cacheinfo::team_cache(&state.cfg.data_dir, &team);
    let mut buckets = vec![DashboardDay::default(); days as usize];
    for item in activity {
        if let Some(started) = parse_utc(item.started_at.as_deref()) {
            let date = started.with_timezone(&timezone).date_naive();
            if let Some(index) = day_offset(first, date, days) {
                buckets[index].started += 1;
            }
        }
        if let Some(finished) = parse_utc(item.finished_at.as_deref()) {
            let date = finished.with_timezone(&timezone).date_naive();
            if let Some(index) = day_offset(first, date, days) {
                match item.status.as_str() {
                    "successful" => buckets[index].successful += 1,
                    "failed" => buckets[index].failed += 1,
                    "canceled" => buckets[index].canceled += 1,
                    _ => {}
                }
            }
        }
    }
    let series = buckets
        .iter()
        .enumerate()
        .map(|(offset, day)| DashboardSeriesResponse {
            date: (first + ChronoDuration::days(offset as i64)).to_string(),
            started: day.started,
            successful: day.successful,
            failed: day.failed,
            canceled: day.canceled,
        })
        .collect();
    Ok(Json(DashboardResponse {
        range_days: days,
        timezone: timezone.name().to_string(),
        from,
        to,
        series,
        cache: DashboardCacheResponse {
            collections: cache.collections.len(),
            roles: cache.roles.len(),
            tofu_bytes: cache.tofu_bytes,
        },
    }))
}

pub async fn cache(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<CacheResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let report = cacheinfo::team_cache(&state.cfg.data_dir, &team);
    Ok(Json(CacheResponse {
        collections_bytes: report.collections_bytes,
        roles_bytes: report.roles_bytes,
        tofu_bytes: report.tofu_bytes,
        collections: report.collections,
        roles: report.roles,
        note: "the_cache_lives_under_data_cache_and_stays",
    }))
}

pub async fn plugins(user: AuthUser) -> Json<PluginListResponse> {
    let _ = user;
    Json(PluginListResponse(
        runner::tools()
            .iter()
            .map(|tool| PluginResponse {
                id: tool.id,
                summary: tool.summary,
            })
            .collect(),
    ))
}

fn admin_routes() -> Router<Shared> {
    Router::new()
        .route("/maintenance", get(maintenance_status))
        .route("/admin/backup", get(download).post(restore))
        .route("/admin/database/status", get(database_status))
        .route("/admin/database/check", post(database_check))
        .route("/admin/database/migrate", post(database_migrate))
        .layer(DefaultBodyLimit::max(256 * 1024 * 1024))
}

#[derive(Deserialize, Default)]
pub struct DownloadQuery {
    #[serde(default)]
    tofu_state: bool,
}

#[derive(Serialize)]
pub(super) struct MaintenanceResponse {
    active: bool,
    since: Option<String>,
}

impl MaintenanceResponse {
    fn new(since: Option<String>) -> Self {
        Self {
            active: since.is_some(),
            since,
        }
    }
}

#[derive(Serialize)]
pub(super) struct RestoreResponse {
    restored: bool,
    engine: &'static str,
    previous: &'static str,
}

#[derive(Serialize)]
pub(super) struct DatabaseStatusResponse {
    engine: &'static str,
    supported: bool,
    maintenance: MaintenanceResponse,
    migration: MigrationStatus,
}

#[derive(Serialize)]
pub(super) struct DatabaseCheckResponse {
    all_passed: bool,
    id: Option<String>,
    checks: Vec<CheckResult>,
}

#[derive(Serialize)]
pub(super) struct DatabaseMigrateResponse {
    started: bool,
}

pub async fn download(
    State(state): State<Shared>,
    Query(query): Query<DownloadQuery>,
    user: AuthUser,
) -> AppResult<Response> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    let bytes = super::service::export(&state, query.tofu_state)?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/zip"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"koreograf-backup.zip\""),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        bytes,
    )
        .into_response())
}

pub async fn restore(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    body: Bytes,
) -> AppResult<Response> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    if headers
        .get("x-restore-confirm")
        .and_then(|value| value.to_str().ok())
        != Some("ERSETZEN")
    {
        return Err(ApiError::bad("confirm_the_replacement_with_ersetzen"));
    }
    super::service::restore(&state, &body)?;
    let mut response = (
        StatusCode::OK,
        Json(RestoreResponse {
            restored: true,
            engine: state.engine(),
            previous: "restore-previous.zip",
        }),
    )
        .into_response();
    let cookie = HeaderValue::from_str(&auth::clear_cookie(auth::request_cookie_secure(
        &state.cfg, &headers,
    )))
    .map_err(|err| ApiError::Internal(err.to_string()))?;
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DatabaseTargetBody {
    host: String,
    #[serde(default = "default_port")]
    port: u16,
    database: String,
    user: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    sslmode: String,
    ca_file: Option<String>,
}

fn default_port() -> u16 {
    5432
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DatabaseStartBody {
    id: String,
}

fn admin(user: &AuthUser) -> AppResult<()> {
    if user.is_admin {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn migration_lock(state: &Shared) -> AppResult<std::sync::MutexGuard<'_, Runtime>> {
    state
        .migration
        .lock()
        .map_err(|_| ApiError::Internal("migration state is damaged".into()))
}

pub async fn maintenance_status(State(state): State<Shared>) -> Json<MaintenanceResponse> {
    Json(MaintenanceResponse::new(state.maintenance_since()))
}

pub async fn database_status(
    State(state): State<Shared>,
    user: AuthUser,
) -> AppResult<Json<DatabaseStatusResponse>> {
    admin(&user)?;
    let runtime = migration_lock(&state)?;
    Ok(Json(DatabaseStatusResponse {
        engine: state.engine(),
        supported: state.engine() == "sqlite",
        maintenance: MaintenanceResponse::new(runtime.maintenance_since.clone()),
        migration: runtime.status.clone(),
    }))
}

pub async fn database_check(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<DatabaseTargetBody>,
) -> AppResult<Json<DatabaseCheckResponse>> {
    admin(&user)?;
    if state.engine() != "sqlite" {
        return Err(ApiError::bad("database_already_on_postgresql"));
    }
    let target = super::service::parse_form_target(
        &body.host,
        body.port,
        &body.database,
        &body.user,
        &body.password,
        &body.sslmode,
        body.ca_file.as_deref(),
    )?;
    {
        let mut runtime = migration_lock(&state)?;
        if runtime.status.running || runtime.maintenance_since.is_some() {
            return Err(ApiError::conflict_vars(
                "database_migration_already_running",
                [("field", "id")],
            ));
        }
        runtime.pending = None;
    }
    let source = state.cfg.data_dir.join("koreograf.db");
    let checks = super::service::run_checks(&target, &source).await;
    let passed = super::service::all_passed(&checks);
    let id = if passed {
        let id = Uuid::new_v4().to_string();
        let mut runtime = migration_lock(&state)?;
        runtime.pending = Some(PendingTarget {
            id: id.clone(),
            config: target,
        });
        Some(id)
    } else {
        None
    };
    Ok(Json(DatabaseCheckResponse {
        all_passed: passed,
        id,
        checks,
    }))
}

pub async fn database_migrate(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<DatabaseStartBody>,
) -> AppResult<(StatusCode, Json<DatabaseMigrateResponse>)> {
    admin(&user)?;
    if state.engine() != "sqlite" {
        return Err(ApiError::bad("database_already_on_postgresql"));
    }
    let target = {
        let mut runtime = migration_lock(&state)?;
        if runtime.status.running || runtime.maintenance_since.is_some() {
            return Err(ApiError::conflict_vars(
                "database_migration_already_running",
                [("field", "id")],
            ));
        }
        let Some(pending) = runtime.pending.as_ref() else {
            return Err(ApiError::bad("run_the_checks_again"));
        };
        if pending.id != body.id {
            return Err(ApiError::bad("run_the_checks_again"));
        }
        {
            let procs = state
                .procs
                .lock()
                .map_err(|_| ApiError::Internal("job state is damaged".into()))?;
            if !procs.is_empty() {
                return Err(ApiError::conflict_vars(
                    "finish_or_cancel_running_jobs",
                    [("field", "id")],
                ));
            }
        }
        let target = match runtime.pending.take() {
            Some(pending) => pending.config,
            None => return Err(ApiError::bad("run_the_checks_again")),
        };
        let started_at = db::now();
        let audit = super::service::audit_entry(&user.username, &target);
        runtime.status = MigrationStatus {
            running: true,
            started_at: Some(started_at.clone()),
            target_host: target.target(),
            url: super::service::target_url(&target),
            tables: MigrationStatus::empty_tables(),
            audit: Some(audit.clone()),
            ..MigrationStatus::default()
        };
        runtime.maintenance_since = Some(started_at);
        target
    };
    tracing::info!(
        audit = %super::service::audit_line(&super::service::audit_entry(&user.username, &target)),
        "database migration started"
    );
    let task_state = state.clone();
    let username = user.username.clone();
    tokio::spawn(async move {
        run_migration(task_state, target, username).await;
    });
    Ok((
        StatusCode::ACCEPTED,
        Json(DatabaseMigrateResponse { started: true }),
    ))
}

async fn run_migration(state: Shared, target: crate::PostgresConfig, user: String) {
    let snapshot = state
        .cfg
        .data_dir
        .join(format!("migration-{}.db", Uuid::new_v4()));
    let result = async {
        state.store().export_sqlite(&snapshot)?;
        super::service::check_source(&snapshot)
            .map_err(|_| ApiError::bad("database_migration_failed"))?;
        let client = crate::backend::connect_client(&target)
            .await
            .map_err(|_| ApiError::bad("database_migration_failed"))?;
        super::service::precheck(&client)
            .await
            .map_err(|_| ApiError::bad("database_migration_failed"))?;
        let progress_state = state.clone();
        super::service::copy_source(&client, &snapshot, move |table, rows| {
            if let Ok(mut runtime) = progress_state.migration.lock() {
                if let Some(item) = runtime
                    .status
                    .tables
                    .iter_mut()
                    .find(|item| item.table == table)
                {
                    item.rows = rows;
                    item.done = true;
                }
                runtime.status.rows += rows;
            }
        })
        .await
        .map_err(|_| ApiError::bad("database_migration_failed"))
    }
    .await;
    remove_snapshot(&snapshot);
    let host = target.target();
    match result {
        Ok(rows) => {
            if let Ok(mut runtime) = state.migration.lock() {
                runtime.status.running = false;
                runtime.status.done = true;
                runtime.status.failed = false;
                runtime.status.rows = rows;
                runtime.status.finished_at = Some(db::now());
                runtime.status.error = None;
            }
            tracing::info!(
                user = %user,
                target = %host,
                rows,
                "database migration finished; restart with the new database URL"
            );
        }
        Err(_) => {
            if let Ok(mut runtime) = state.migration.lock() {
                runtime.status.running = false;
                runtime.status.done = false;
                runtime.status.failed = true;
                runtime.status.finished_at = Some(db::now());
                runtime.status.error = Some("database_migration_failed");
                runtime.maintenance_since = None;
            }
            tracing::error!(
                user = %user,
                target = %host,
                "database migration failed; Koreograf stays on SQLite"
            );
        }
    }
}

fn remove_snapshot(snapshot: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = snapshot.as_os_str().to_os_string();
        name.push(suffix);
        let _ = std::fs::remove_file(std::path::PathBuf::from(name));
    }
}
