use std::collections::{BTreeMap, BTreeSet};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, SecondsFormat, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::auth::{AuthUser, Role};
use crate::backend::Backend;
use crate::cacheinfo::{self, CacheEntry};
use crate::db::{
    self, Credential, Delivery, Environment, Inventory, InventorySource, Organization, Project,
};
use crate::error::{ApiError, ApiJson, AppResult};
use crate::inventory::{self, EffectiveVar, InventoryData, InventoryHost};
use crate::jobs;
use crate::preflight;
use crate::runner;
use crate::Shared;

use super::{
    blank, check_static_ini, gate, no_content, require_description, require_name, team_header,
};

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/overview", get(overview))
        .route("/dashboard", get(dashboard))
        .route("/runner", get(runner_status))
        .route("/runner/preflight", get(runner_preflight))
        .route("/teams", get(list_teams).post(create_team))
        .route("/teams/{id}", put(update_team))
        .route("/teams/{id}/members", get(list_members).post(add_member))
        .route("/teams/{id}/members/{user_id}", delete(remove_member))
        .route("/projects", get(list_projects).post(create_project))
        .route(
            "/projects/{id}",
            get(get_project).put(update_project).delete(delete_project),
        )
        .route("/projects/{id}/sync", post(sync_project))
        .route("/inventories", get(list_inventories).post(create_inventory))
        .route("/inventories/{id}/hosts", get(list_inventory_hosts))
        .route("/inventories/{id}/hosts/{name}", get(get_inventory_host))
        .route("/inventories/{id}/groups", get(list_inventory_groups))
        .route("/inventories/{id}/groups/{name}", get(get_inventory_group))
        .route(
            "/inventories/{id}",
            get(get_inventory)
                .put(update_inventory)
                .delete(delete_inventory),
        )
        .route(
            "/organizations",
            get(list_organizations).post(create_organization),
        )
        .route(
            "/organizations/{id}",
            get(get_organization).put(update_organization),
        )
        .route(
            "/credentials",
            get(list_credentials).post(create_credential),
        )
        .route(
            "/credentials/{id}",
            get(get_credential)
                .put(update_credential)
                .delete(delete_credential),
        )
        .route(
            "/environments",
            get(list_environments).post(create_environment),
        )
        .route(
            "/environments/{id}",
            get(get_environment)
                .put(update_environment)
                .delete(delete_environment),
        )
        .route("/cache", get(cache))
        .route("/plugins", get(plugins))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamBody {
    name: String,
    #[serde(default)]
    description: String,
    ldap_group: Option<String>,
    organization_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberBody {
    username: String,
    role: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectBody {
    name: String,
    source_type: String,
    source: String,
    #[serde(default)]
    revision: String,
    organization_id: Option<String>,
    #[serde(default)]
    team_ids: Vec<String>,
    credential_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectUpdateBody {
    name: Option<String>,
    source_type: Option<String>,
    source: Option<String>,
    revision: Option<String>,
    organization_id: Option<String>,
    #[serde(default)]
    team_ids: Option<Vec<String>>,
    credential_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganizationBody {
    name: String,
    #[serde(default)]
    description: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryBody {
    name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    sources: Vec<super::sources::SourceInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryUpdateBody {
    name: String,
    #[serde(default)]
    body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialBody {
    name: String,
    kind: String,
    secret: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialUpdate {
    name: String,
    #[serde(default)]
    secret: Value,
    #[serde(default)]
    clear: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentBody {
    name: String,
    image: String,
    #[serde(default)]
    tool: String,
    #[serde(default = "default_pull")]
    pull_policy: String,
    #[serde(default)]
    network: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    scope: Option<String>,
}

fn default_pull() -> String {
    "if_not_present".into()
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
pub(super) struct TeamResponse {
    id: String,
    name: String,
    description: String,
    ldap_group: Option<String>,
    organization_id: String,
    organization_name: String,
    created_at: String,
}

impl TeamResponse {
    fn new(team: &db::Team) -> Self {
        Self {
            id: team.id.clone(),
            name: team.name.clone(),
            description: team.description.clone(),
            ldap_group: team.ldap_group.clone(),
            organization_id: team.organization_id.clone(),
            organization_name: team.organization_name.clone(),
            created_at: team.created_at.clone(),
        }
    }
}

#[derive(Serialize)]
pub(super) struct TeamRoleResponse {
    #[serde(flatten)]
    team: TeamResponse,
    role: String,
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct TeamListResponse(Vec<TeamRoleResponse>);

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct MemberListResponse(Vec<db::Member>);

#[derive(Serialize)]
pub(super) struct MemberCreatedResponse {
    user_id: String,
    role: String,
}

#[derive(Serialize)]
pub(super) struct ProjectResponse {
    id: String,
    team_id: String,
    organization_id: String,
    organization_name: String,
    name: String,
    source_type: String,
    source: String,
    revision: String,
    credential_id: Option<String>,
    created_at: String,
    team_ids: Vec<String>,
    last_sync_at: String,
    last_sync_commit: String,
    last_sync_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sync: Option<SyncResponse>,
}

impl ProjectResponse {
    fn new(project: &Project, team: &str) -> Self {
        Self {
            id: project.id.clone(),
            team_id: project.team_id.clone(),
            organization_id: project.organization_id.clone(),
            organization_name: project.organization_name.clone(),
            name: project.name.clone(),
            source_type: project.source_type.clone(),
            source: runner::mask_git_url(&project.source),
            revision: project.revision.clone(),
            credential_id: if project.team_id == team {
                project.credential_id.clone()
            } else {
                None
            },
            created_at: project.created_at.clone(),
            team_ids: project.team_ids.clone(),
            last_sync_at: project.last_sync_at.clone(),
            last_sync_commit: project.last_sync_commit.clone(),
            last_sync_status: project.last_sync_status.clone(),
            sync: None,
        }
    }
}

#[derive(Serialize)]
pub(super) struct ProjectListResponse(Vec<ProjectResponse>);

#[derive(Serialize)]
pub(super) struct SyncResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vars: Option<BTreeMap<String, String>>,
}

impl SyncResponse {
    fn from_result(result: AppResult<String>) -> Self {
        match result {
            Ok(path) => Self {
                ok: true,
                path: Some(path),
                error: None,
                vars: None,
            },
            Err(ApiError::BadRequest(message)) => Self {
                ok: false,
                path: None,
                error: Some(message.key),
                vars: Some(message.vars),
            },
            Err(err) => Self {
                ok: false,
                path: None,
                error: Some(err.to_string()),
                vars: None,
            },
        }
    }
}

#[derive(Serialize)]
pub(super) struct SyncProjectResponse {
    path: String,
}

#[derive(Serialize)]
pub(super) struct InventorySourceResponse {
    id: String,
    name: String,
    kind: String,
    source: Value,
    credential_id: Option<String>,
    host_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
}

impl InventorySourceResponse {
    fn summary(source: &InventorySource) -> Self {
        Self {
            id: source.id.clone(),
            name: source.name.clone(),
            kind: source.kind.clone(),
            source: db::json_object(&source.source_json),
            credential_id: source.credential_id.clone(),
            host_count: inventory::parse_hosts(&source.body).len(),
            body: None,
        }
    }

    fn detail(source: &InventorySource) -> Self {
        Self {
            body: Some(source.body.clone()),
            ..Self::summary(source)
        }
    }
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct InventoryListResponse(Vec<InventorySummaryResponse>);

#[derive(Serialize)]
pub(super) struct InventorySummaryResponse {
    id: String,
    team_id: String,
    name: String,
    created_at: String,
    source_kind: String,
    source: Value,
    credential_id: Option<String>,
    source_count: usize,
    sources: Vec<InventorySourceResponse>,
    host_count: usize,
    group_count: usize,
}

#[derive(Serialize)]
pub(super) struct InventoryDetailResponse {
    id: String,
    team_id: String,
    name: String,
    body: String,
    created_at: String,
    source_kind: String,
    source: Value,
    credential_id: Option<String>,
    source_count: usize,
    sources: Vec<InventorySourceResponse>,
    host_count: usize,
    group_count: usize,
    hosts: Vec<InventoryHost>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) sync: Option<InventorySyncResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) collisions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) dropped_vars: Option<Vec<String>>,
}

#[derive(Serialize)]
pub(super) struct InventorySyncResponse {
    pub ok: bool,
    pub before: usize,
    pub after: usize,
}

impl InventoryDetailResponse {
    pub(super) fn host_count(&self) -> usize {
        self.host_count
    }
}

#[derive(Serialize)]
pub(super) struct InventoryHostsResponse {
    id: String,
    total: usize,
    page: usize,
    page_size: usize,
    groups: Vec<String>,
    hosts: Vec<InventoryHost>,
}

#[derive(Serialize)]
pub(super) struct InventoryHostSourceResponse {
    id: Option<String>,
    name: String,
    kind: String,
}

#[derive(Serialize)]
pub(super) struct InventoryHostResponse {
    id: String,
    name: String,
    groups: Vec<String>,
    source: InventoryHostSourceResponse,
    vars: Vec<EffectiveVar>,
    netbox_id: Option<String>,
    netbox_url: Option<String>,
}

#[derive(Serialize)]
pub(super) struct InventoryGroupsResponse {
    id: String,
    total: usize,
    groups: Vec<InventoryGroupEntryResponse>,
}

#[derive(Serialize)]
struct InventoryGroupEntryResponse {
    name: String,
    host_count: usize,
    children: Vec<String>,
    vars: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub(super) struct InventoryGroupResponse {
    id: String,
    name: String,
    children: Vec<String>,
    parents: Vec<String>,
    vars: BTreeMap<String, String>,
    host_count: usize,
    total: usize,
    page: usize,
    page_size: usize,
    hosts: Vec<InventoryHost>,
}

#[derive(Serialize)]
pub(super) struct CredentialUsageResponse {
    inventory_id: Option<String>,
    inventory_name: Option<String>,
    source_id: Option<String>,
    source_name: Option<String>,
    project_id: Option<String>,
    project_name: Option<String>,
}

#[derive(Serialize)]
pub(super) struct CredentialSummaryResponse {
    id: String,
    name: String,
    kind: String,
    created_at: String,
    usages: Vec<CredentialUsageResponse>,
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct CredentialListResponse(Vec<CredentialSummaryResponse>);

#[derive(Serialize)]
pub(super) struct CreatedCredentialResponse {
    id: String,
    name: String,
    kind: String,
}

#[derive(Serialize)]
pub(super) struct CredentialDetailResponse {
    id: String,
    name: String,
    kind: String,
    created_at: String,
    fields: Value,
    set: Vec<String>,
    usages: Vec<CredentialUsageResponse>,
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct EnvironmentListResponse(Vec<Environment>);

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

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct OrganizationListResponse(Vec<Organization>);

#[derive(Serialize)]
pub(super) struct OrganizationDetailResponse {
    id: String,
    name: String,
    description: String,
    created_at: String,
    teams: Vec<TeamResponse>,
}

pub async fn runner_status(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<RunnerStatusResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    // The probe awaits. Load settings only after it returns.
    let effective = crate::settings::load(&state)?;
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
    let effective = crate::settings::load(&state)?;
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
    let effective = crate::settings::load(&state)?;
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
        jobs: jobs.iter().map(|job| super::job_json(job, None)).collect(),
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

pub async fn list_teams(
    State(state): State<Shared>,
    user: AuthUser,
) -> AppResult<Json<TeamListResponse>> {
    let teams = state.store().list_teams_for_user(&user.id, user.is_admin)?;
    Ok(Json(TeamListResponse(
        teams
            .into_iter()
            .map(|(team, role)| TeamRoleResponse {
                team: TeamResponse::new(&team),
                role,
            })
            .collect(),
    )))
}

pub async fn create_team(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<TeamBody>,
) -> AppResult<(StatusCode, Json<TeamResponse>)> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    let name = require_name(&body.name)?;
    let id = Uuid::new_v4().to_string();
    let ldap_group = blank(body.ldap_group);
    let organization_id = organization_for_new_team(state.store(), body.organization_id, &name)?;
    state.store().insert_team(
        &id,
        &organization_id,
        &name,
        &require_description(&body.description)?,
        ldap_group.as_deref(),
    )?;
    state
        .store()
        .upsert_membership(&id, &user.id, "owner", "manual")?;
    let team = state.store().team_by_id(&id)?.ok_or(ApiError::NotFound)?;
    Ok((StatusCode::CREATED, Json(TeamResponse::new(&team))))
}

pub async fn update_team(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<TeamBody>,
) -> AppResult<Json<TeamResponse>> {
    gate(&state, &user, &id, Role::Owner)?;
    let name = require_name(&body.name)?;
    let ldap_group = blank(body.ldap_group);
    if let Some(organization_id) = blank(body.organization_id) {
        if !user.is_admin {
            return Err(ApiError::Forbidden);
        }
        state
            .store()
            .organization(&organization_id)?
            .ok_or_else(|| ApiError::bad("organization_is_missing"))?;
        state.store().set_team_organization(&id, &organization_id)?;
    }
    state.store().update_team(
        &id,
        &name,
        &require_description(&body.description)?,
        ldap_group.as_deref(),
    )?;
    let team = state.store().team_by_id(&id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(TeamResponse::new(&team)))
}

fn organization_for_new_team(
    store: &dyn Backend,
    requested: Option<String>,
    team_name: &str,
) -> AppResult<String> {
    if let Some(organization_id) = blank(requested) {
        store
            .organization(&organization_id)?
            .ok_or_else(|| ApiError::bad("organization_is_missing"))?;
        return Ok(organization_id);
    }
    if let Some(existing) = store.organization_by_name(team_name)? {
        return Ok(existing.id);
    }
    let organization_id = Uuid::new_v4().to_string();
    store.insert_organization(&organization_id, team_name, "")?;
    Ok(organization_id)
}

pub async fn list_members(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<MemberListResponse>> {
    gate(&state, &user, &id, Role::Viewer)?;
    let members = state.store().list_members(&id)?;
    Ok(Json(MemberListResponse(members)))
}

pub async fn add_member(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<MemberBody>,
) -> AppResult<(StatusCode, Json<MemberCreatedResponse>)> {
    gate(&state, &user, &id, Role::Owner)?;
    let role =
        Role::parse(&body.role).ok_or_else(|| ApiError::bad("role_is_viewer_operator_or_owner"))?;
    let username = body.username.trim().to_ascii_lowercase();
    let member = state
        .store()
        .user_by_username(&username)?
        .ok_or_else(|| ApiError::bad("unknown_user_create_the_account_first"))?;
    state
        .store()
        .grant_manual_membership(&id, &member.id, role.as_str())?;
    Ok((
        StatusCode::CREATED,
        Json(MemberCreatedResponse {
            user_id: member.id,
            role: role.as_str().to_string(),
        }),
    ))
}

pub async fn remove_member(
    State(state): State<Shared>,
    user: AuthUser,
    Path((id, user_id)): Path<(String, String)>,
) -> AppResult<axum::response::Response> {
    gate(&state, &user, &id, Role::Owner)?;
    if user_id == user.id && !user.is_admin {
        return Err(ApiError::bad("you_cannot_remove_yourself_from_the_team"));
    }
    let db = state.store();
    db.delete_membership(&id, &user_id)?;
    Ok(no_content())
}

pub async fn get_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<ProjectResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    if !db.project_visible(&id, &team)? {
        return Err(ApiError::NotFound);
    }
    let project = db.project(&id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(ProjectResponse::new(&project, &team)))
}

pub async fn list_projects(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<ProjectListResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    Ok(Json(ProjectListResponse(
        db.list_projects(&team)?
            .iter()
            .map(|project| ProjectResponse::new(project, &team))
            .collect(),
    )))
}

pub async fn update_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ProjectUpdateBody>,
) -> AppResult<Json<ProjectResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let (updated, sync_changed, discard_checkout) = {
        let db = state.store();
        if !db.project_visible(&id, &team)? {
            return Err(ApiError::NotFound);
        }
        let project = db.project(&id)?.ok_or(ApiError::NotFound)?;
        let name = match body.name {
            Some(name) => require_name(&name)?,
            None => project.name.clone(),
        };
        let source_type = body
            .source_type
            .unwrap_or_else(|| project.source_type.clone());
        let source = body.source.unwrap_or_else(|| project.source.clone());
        let revision = body.revision.unwrap_or_else(|| project.revision.clone());
        let (source, revision) =
            validate_project_source(&state, &user, &source_type, &source, &revision)?;
        let team_row = db.team_by_id(&team)?.ok_or(ApiError::NotFound)?;
        let organization_id = match blank(body.organization_id) {
            Some(organization_id) => {
                if !user.is_admin && organization_id != team_row.organization_id {
                    return Err(ApiError::bad("a_project_stays_in_the_organization_of_the"));
                }
                db.organization(&organization_id)?
                    .ok_or_else(|| ApiError::bad("organization_is_missing"))?;
                organization_id
            }
            None => project.organization_id.clone(),
        };
        let team_ids = body.team_ids.unwrap_or_else(|| project.team_ids.clone());
        check_team_ids(db, &organization_id, &team_ids)?;
        let source_changed = project.source_type != source_type || project.source != source;
        let owner = team == project.team_id;
        let credential_id = match body.credential_id {
            Some(raw) => {
                let requested = blank(Some(raw));
                if requested.is_none() && !owner && !source_changed {
                    project.credential_id.clone()
                } else {
                    validate_project_credential(db, &team, requested.as_deref())?
                }
            }
            None if source_changed && !owner => None,
            None if source_changed => {
                validate_project_credential(db, &team, project.credential_id.as_deref())?
            }
            None => project.credential_id.clone(),
        };
        db.set_project_details(&id, &name, &source_type, &source, &revision)?;
        db.set_project_organization(&id, &organization_id)?;
        db.replace_project_teams(&id, &team_ids)?;
        db.set_project_credential(&id, credential_id.as_deref())?;
        db.set_project_admin_saved(&id, user.is_admin)?;
        let changed = project.source_type != source_type
            || project.source != source
            || project.revision != revision;
        let discard = changed && (project.source_type == "git" || source_type == "git");
        let updated = db.project(&id)?.ok_or(ApiError::NotFound)?;
        (updated, changed && source_type == "git", discard)
    };
    let mut response = ProjectResponse::new(&updated, &team);
    if discard_checkout {
        jobs::discard_project_checkout(&state, &updated);
    }
    if sync_changed {
        let sync = SyncResponse::from_result(jobs::sync_project(&state, &updated).await);
        let db = state.store();
        response = ProjectResponse::new(&db.project(&id)?.ok_or(ApiError::NotFound)?, &team);
        response.sync = Some(sync);
    }
    Ok(Json(response))
}

pub async fn create_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<ProjectBody>,
) -> AppResult<(StatusCode, Json<ProjectResponse>)> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let name = require_name(&body.name)?;
    let (source, revision) = validate_project_source(
        &state,
        &user,
        &body.source_type,
        &body.source,
        &body.revision,
    )?;
    let db = state.store();
    let current = db.team_by_id(&team)?.ok_or(ApiError::NotFound)?;
    let organization_id = match blank(body.organization_id) {
        Some(organization_id) => {
            if !user.is_admin && organization_id != current.organization_id {
                return Err(ApiError::bad("a_project_stays_in_the_organization_of_the"));
            }
            db.organization(&organization_id)?
                .ok_or_else(|| ApiError::bad("organization_is_missing"))?;
            organization_id
        }
        None => current.organization_id,
    };
    check_team_ids(db, &organization_id, &body.team_ids)?;
    let credential_id =
        validate_project_credential(db, &team, blank(body.credential_id).as_deref())?;
    let project = Project {
        id: Uuid::new_v4().to_string(),
        team_id: team.clone(),
        organization_id,
        organization_name: String::new(),
        name,
        source_type: body.source_type,
        source,
        revision,
        credential_id,
        created_at: db::now(),
        team_ids: body.team_ids,
        last_sync_at: String::new(),
        last_sync_commit: String::new(),
        last_sync_status: String::new(),
        admin_saved: user.is_admin,
    };
    db.insert_project(&project)?;
    let stored = db.project(&project.id)?.ok_or(ApiError::NotFound)?;
    Ok((
        StatusCode::CREATED,
        Json(ProjectResponse::new(&stored, &team)),
    ))
}

fn validate_project_source(
    state: &Shared,
    user: &AuthUser,
    source_type: &str,
    source: &str,
    revision: &str,
) -> AppResult<(String, String)> {
    let source = source.trim();
    let revision = revision.trim();
    if !runner::valid_revision(revision) {
        return Err(ApiError::bad(
            "revision_may_contain_only_letters_digits_dot_slash",
        ));
    }
    match source_type {
        "path" => {
            runner::authorize_project_source(
                "path",
                source,
                &state.cfg.data_dir,
                &state.cfg.project_roots,
                user.is_admin,
            )
            .map_err(|err| project_path_error(err, user.is_admin))?;
        }
        "git" => {
            if !runner::valid_git(source) {
                return Err(ApiError::bad("the_git_address_must_be_http_s_ssh"));
            }
            if runner::git_address_has_credentials(source) {
                return Err(ApiError::bad_vars(
                    "the_git_address_contains_credentials",
                    [("field", "credential_id")],
                ));
            }
            if source.starts_with("file://") {
                runner::authorize_project_source(
                    "git",
                    source,
                    &state.cfg.data_dir,
                    &state.cfg.project_roots,
                    user.is_admin,
                )
                .map_err(|err| project_path_error(err, user.is_admin))?;
            }
        }
        _ => return Err(ApiError::bad("source_is_path_or_git")),
    }
    Ok((source.to_string(), revision.to_string()))
}

fn project_path_error(err: runner::ProjectPathError, is_admin: bool) -> ApiError {
    match err {
        runner::ProjectPathError::NotDirectory if is_admin => {
            ApiError::bad("the_project_path_is_not_a_directory")
        }
        runner::ProjectPathError::NotDirectory | runner::ProjectPathError::Denied => {
            ApiError::bad("the_project_path_is_not_allowed")
        }
        runner::ProjectPathError::AdminsOnly => {
            ApiError::bad("path_projects_are_limited_to_administrators")
        }
    }
}

fn validate_project_credential(
    db: &dyn Backend,
    team: &str,
    credential_id: Option<&str>,
) -> AppResult<Option<String>> {
    let Some(id) = credential_id else {
        return Ok(None);
    };
    let credential = db.credential(id)?.ok_or(ApiError::NotFound)?;
    if credential.team_id != team {
        return Err(ApiError::NotFound);
    }
    if credential.kind != "scm" {
        return Err(ApiError::bad(
            "the_credential_is_not_a_source_control_credential",
        ));
    }
    if credential.disabled {
        return Err(ApiError::bad("the_source_control_credential_is_disabled"));
    }
    Ok(Some(credential.id))
}

fn check_team_ids(db: &dyn Backend, organization_id: &str, team_ids: &[String]) -> AppResult<()> {
    for team_id in team_ids {
        let team = db
            .team_by_id(team_id)?
            .ok_or_else(|| ApiError::bad("team_is_missing"))?;
        if team.organization_id != organization_id {
            return Err(ApiError::bad(
                "assigned_teams_must_belong_to_the_project_organization",
            ));
        }
    }
    Ok(())
}

pub async fn delete_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<axum::response::Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    state.store().delete_project(&id, &team)?;
    Ok(no_content())
}

pub async fn sync_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<SyncProjectResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let project = {
        let db = state.store();
        let project = db.project(&id)?.ok_or(ApiError::NotFound)?;
        if !db.project_visible(&id, &team)? {
            return Err(ApiError::NotFound);
        }
        project
    };
    let path = jobs::sync_project(&state, &project).await?;
    Ok(Json(SyncProjectResponse { path }))
}

pub async fn list_inventories(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<InventoryListResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let items = db.list_inventories(&team)?;
    let mut rows = Vec::with_capacity(items.len());
    for item in &items {
        rows.push(inventory_summary(db, item)?);
    }
    Ok(Json(InventoryListResponse(rows)))
}

pub async fn get_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<InventorySummaryResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let item = visible_inventory(db, &id, &team)?;
    Ok(Json(inventory_summary(db, &item)?))
}

const HOST_PAGE_SIZE: usize = 50;
const HOST_PAGE_SIZE_MAX: usize = 200;

#[derive(Deserialize)]
pub struct HostQuery {
    #[serde(default)]
    page: Option<usize>,
    #[serde(default)]
    page_size: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    search: String,
    #[serde(default)]
    group: String,
    #[serde(default)]
    sort: String,
}

pub async fn list_inventory_hosts(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<HostQuery>,
) -> AppResult<Json<InventoryHostsResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let item = visible_inventory(db, &id, &team)?;
    let sources = db.list_inventory_sources(&item.id)?;
    let data = inventory_data(&item, &sources);
    let groups = group_names(&data);
    let needle = query.search.trim().to_lowercase();
    let group = query.group.trim().to_lowercase();
    let shown: Vec<_> = data
        .hosts
        .into_iter()
        .filter(|host| matches_host(host, &needle, &group))
        .collect();
    let page = page_hosts(shown, &query);
    Ok(Json(InventoryHostsResponse {
        id: item.id,
        total: page.total,
        page: page.page,
        page_size: page.page_size,
        groups,
        hosts: page.hosts,
    }))
}

struct HostPage {
    total: usize,
    page: usize,
    page_size: usize,
    hosts: Vec<InventoryHost>,
}

fn page_hosts(mut hosts: Vec<InventoryHost>, query: &HostQuery) -> HostPage {
    sort_hosts(&mut hosts, &query.sort);
    let total = hosts.len();
    let page_size = query
        .page_size
        .or(query.limit)
        .unwrap_or(HOST_PAGE_SIZE)
        .clamp(1, HOST_PAGE_SIZE_MAX);
    let offset = query.offset.unwrap_or_else(|| {
        query
            .page
            .unwrap_or(1)
            .saturating_sub(1)
            .saturating_mul(page_size)
    });
    let page = offset / page_size + 1;
    let hosts = hosts.into_iter().skip(offset).take(page_size).collect();
    HostPage {
        total,
        page,
        page_size,
        hosts,
    }
}

pub async fn get_inventory_host(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path((id, name)): Path<(String, String)>,
) -> AppResult<Json<InventoryHostResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let item = visible_inventory(db, &id, &team)?;
    let sources = db.list_inventory_sources(&item.id)?;
    let data = inventory_data(&item, &sources);
    let Some(host) = data.hosts.iter().find(|host| host.name == name) else {
        return Err(ApiError::NotFound);
    };
    let vars = inventory::effective_vars(&data, &name);
    let netbox_id = vars
        .iter()
        .find(|item| item.name == "netbox_id")
        .map(|item| item.value.clone());
    let source = host_source(&sources, &name);
    let source_response = match source {
        Some(source) => InventoryHostSourceResponse {
            id: Some(source.id.clone()),
            name: source.name.clone(),
            kind: source.kind.clone(),
        },
        None => InventoryHostSourceResponse {
            id: None,
            name: item.name.clone(),
            kind: item.source_kind.clone(),
        },
    };
    Ok(Json(InventoryHostResponse {
        id: item.id,
        name: host.name.clone(),
        groups: inventory::host_groups(&data, &name),
        source: source_response,
        vars,
        netbox_url: netbox_url(&state, db, source, netbox_id.as_deref()),
        netbox_id,
    }))
}

pub async fn list_inventory_groups(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<InventoryGroupsResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let item = visible_inventory(db, &id, &team)?;
    let sources = db.list_inventory_sources(&item.id)?;
    let data = inventory_data(&item, &sources);
    let groups: Vec<InventoryGroupEntryResponse> = data
        .groups
        .iter()
        .map(|group| InventoryGroupEntryResponse {
            name: group.name.clone(),
            host_count: inventory::group_hosts(&data, &group.name).len(),
            children: group.children.clone(),
            vars: group.vars.clone(),
        })
        .collect();
    let total = groups.len();
    Ok(Json(InventoryGroupsResponse {
        id: item.id,
        total,
        groups,
    }))
}

pub async fn get_inventory_group(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path((id, name)): Path<(String, String)>,
    Query(query): Query<HostQuery>,
) -> AppResult<Json<InventoryGroupResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let item = visible_inventory(db, &id, &team)?;
    let sources = db.list_inventory_sources(&item.id)?;
    let data = inventory_data(&item, &sources);
    let Some(group) = data.groups.iter().find(|group| group.name == name).cloned() else {
        return Err(ApiError::NotFound);
    };
    let hosts: Vec<InventoryHost> = inventory::group_hosts(&data, &name)
        .into_iter()
        .cloned()
        .collect();
    let needle = query.search.trim().to_lowercase();
    let shown: Vec<_> = hosts
        .into_iter()
        .filter(|host| matches_host(host, &needle, ""))
        .collect();
    let page = page_hosts(shown, &query);
    let parents: Vec<String> = data
        .groups
        .iter()
        .filter(|item| item.children.iter().any(|child| child == &name))
        .map(|item| item.name.clone())
        .collect();
    Ok(Json(InventoryGroupResponse {
        id: item.id,
        name: group.name,
        children: group.children,
        parents,
        vars: group.vars,
        host_count: page.total,
        total: page.total,
        page: page.page,
        page_size: page.page_size,
        hosts: page.hosts,
    }))
}

fn visible_inventory(db: &dyn Backend, id: &str, team: &str) -> AppResult<Inventory> {
    let item = db.inventory(id)?.ok_or(ApiError::NotFound)?;
    if item.team_id != team {
        return Err(ApiError::NotFound);
    }
    Ok(item)
}

fn inventory_data(item: &Inventory, sources: &[InventorySource]) -> InventoryData {
    if sources.is_empty() {
        inventory::parse_inventory(&item.body)
    } else {
        let bodies: Vec<&str> = sources.iter().map(|source| source.body.as_str()).collect();
        inventory::merge_inventory(&bodies)
    }
}

fn group_names(data: &InventoryData) -> Vec<String> {
    data.groups
        .iter()
        .map(|group| group.name.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn group_count(data: &InventoryData) -> usize {
    data.groups
        .iter()
        .filter(|group| group.name != "all" && group.name != "ungrouped")
        .filter(|group| !inventory::group_hosts(data, &group.name).is_empty())
        .count()
}

fn host_source<'a>(sources: &'a [InventorySource], name: &str) -> Option<&'a InventorySource> {
    sources.iter().rev().find(|source| {
        inventory::parse_hosts(&source.body)
            .iter()
            .any(|host| host.name == name)
    })
}

fn netbox_url(
    state: &Shared,
    db: &dyn Backend,
    source: Option<&InventorySource>,
    netbox_id: Option<&str>,
) -> Option<String> {
    let source = source.filter(|item| item.kind == "netbox")?;
    let id = netbox_id?;
    if !id.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let credential_id = source.credential_id.as_deref()?;
    let credential = db.credential(credential_id).ok()??;
    let plain = state.decrypt(&credential.secret_blob).ok()?;
    let value: Value = serde_json::from_str(&plain).ok()?;
    let endpoint = value
        .get("url")
        .and_then(Value::as_str)?
        .trim_end_matches('/');
    if endpoint.is_empty() {
        return None;
    }
    Some(format!("{endpoint}/dcim/devices/{id}/"))
}

fn matches_host(host: &crate::inventory::InventoryHost, needle: &str, group: &str) -> bool {
    if !group.is_empty() && !host.groups.iter().any(|item| item.to_lowercase() == group) {
        return false;
    }
    needle.is_empty()
        || host.name.to_lowercase().contains(needle)
        || host
            .groups
            .iter()
            .any(|item| item.to_lowercase().contains(needle))
        || host.vars.iter().any(|(key, value)| {
            key.to_lowercase().contains(needle) || value.to_lowercase().contains(needle)
        })
}

fn sort_hosts(hosts: &mut [crate::inventory::InventoryHost], sort: &str) {
    let (field, reverse) = match sort.trim().strip_prefix('-') {
        Some(field) => (field, true),
        None => (sort.trim(), false),
    };
    hosts.sort_by(|left, right| {
        let key = |host: &crate::inventory::InventoryHost| match field {
            "address" => host
                .vars
                .get("ansible_host")
                .cloned()
                .unwrap_or_default()
                .to_lowercase(),
            "groups" => host.groups.join(",").to_lowercase(),
            _ => host.name.to_lowercase(),
        };
        let order = key(left)
            .cmp(&key(right))
            .then_with(|| left.name.cmp(&right.name));
        if reverse {
            order.reverse()
        } else {
            order
        }
    });
}

pub async fn update_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<InventoryUpdateBody>,
) -> AppResult<Json<InventoryDetailResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let name = require_name(&body.name)?;
    let db = state.store();
    let existing = db.inventory(&id)?.ok_or(ApiError::NotFound)?;
    if existing.team_id != team {
        return Err(ApiError::NotFound);
    }
    let sources = db.list_inventory_sources(&id)?;
    if !body.body.trim().is_empty() {
        if sources.len() > 1 {
            return Err(ApiError::bad(
                "this_inventory_has_several_sources_edit_the_text",
            ));
        }
        if let Some(source) = sources.first() {
            if source.kind != "ini" {
                return Err(ApiError::bad("this_source_comes_from_a_project_or_from"));
            }
        }
    }
    check_static_ini(&body.body)?;
    db.update_inventory(&id, &team, &name, &existing.body)?;
    if !body.body.trim().is_empty() {
        if let Some(source) = sources.first() {
            db.set_inventory_source_body(&source.id, &body.body)?;
            db.refresh_inventory_summary(&id)?;
        } else {
            db.set_inventory_body(&id, &team, &body.body)?;
        }
    }
    db.set_inventory_admin_saved(&id, user.is_admin)?;
    let item = db.inventory(&id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(inventory_detail(db, &item)?))
}

pub async fn create_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<InventoryBody>,
) -> AppResult<(StatusCode, Json<InventoryDetailResponse>)> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let name = require_name(&body.name)?;
    if !body.sources.is_empty() {
        return super::sources::create_with_sources(
            state,
            team,
            name,
            body.sources,
            user.is_admin,
            &user,
        )
        .await;
    }
    check_static_ini(&body.body)?;
    let item = Inventory {
        id: Uuid::new_v4().to_string(),
        team_id: team,
        name,
        body: body.body.clone(),
        created_at: db::now(),
        source_kind: "ini".into(),
        source_json: "{}".into(),
        credential_id: None,
        admin_saved: user.is_admin,
    };
    let source = if body.body.trim().is_empty() {
        None
    } else {
        Some(db::InventorySource {
            id: Uuid::new_v4().to_string(),
            inventory_id: item.id.clone(),
            name: "manual".into(),
            kind: "ini".into(),
            source_json: "{}".into(),
            credential_id: None,
            body: body.body,
            position: 0,
            created_at: item.created_at.clone(),
        })
    };
    let sources: Vec<db::InventorySource> = source.into_iter().collect();
    let db = state.store();
    db.insert_inventory(&item, &sources, false)?;
    Ok((StatusCode::CREATED, Json(inventory_detail(db, &item)?)))
}

fn inventory_summary(c: &dyn Backend, item: &Inventory) -> AppResult<InventorySummaryResponse> {
    let sources = c.list_inventory_sources(&item.id)?;
    let data = inventory_data(item, &sources);
    Ok(InventorySummaryResponse {
        id: item.id.clone(),
        team_id: item.team_id.clone(),
        name: item.name.clone(),
        created_at: item.created_at.clone(),
        source_kind: item.source_kind.clone(),
        source: db::json_object(&item.source_json),
        credential_id: item.credential_id.clone(),
        source_count: sources.len(),
        sources: sources
            .iter()
            .map(InventorySourceResponse::summary)
            .collect(),
        host_count: data.hosts.len(),
        group_count: group_count(&data),
    })
}

pub(super) fn inventory_detail(
    c: &dyn Backend,
    item: &Inventory,
) -> AppResult<InventoryDetailResponse> {
    let sources = c.list_inventory_sources(&item.id)?;
    let data = inventory_data(item, &sources);
    Ok(InventoryDetailResponse {
        id: item.id.clone(),
        team_id: item.team_id.clone(),
        name: item.name.clone(),
        body: item.body.clone(),
        created_at: item.created_at.clone(),
        source_kind: item.source_kind.clone(),
        source: db::json_object(&item.source_json),
        credential_id: item.credential_id.clone(),
        source_count: sources.len(),
        sources: sources
            .iter()
            .map(InventorySourceResponse::detail)
            .collect(),
        host_count: data.hosts.len(),
        group_count: group_count(&data),
        hosts: data.hosts,
        sync: None,
        collisions: None,
        dropped_vars: None,
    })
}

pub async fn delete_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<axum::response::Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.store();
    let mut names = db.templates_using_inventory(&id, &team)?;
    names.extend(db.workflows_using_inventory(&id, &team)?);
    if !names.is_empty() {
        return Err(ApiError::conflict_vars(
            "inventory_is_in_use",
            [("names", names.join(", "))],
        ));
    }
    db.delete_inventory(&id, &team)?;
    Ok(no_content())
}

pub async fn list_credentials(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<CredentialListResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let mut items = Vec::new();
    for item in db.list_credentials(&team)? {
        items.push(CredentialSummaryResponse {
            id: item.id.clone(),
            name: item.name.clone(),
            kind: item.kind.clone(),
            created_at: item.created_at.clone(),
            usages: usage_list(db, &item.id, &team)?,
        });
    }
    Ok(Json(CredentialListResponse(items)))
}

pub async fn create_credential(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<CredentialBody>,
) -> AppResult<(StatusCode, Json<CreatedCredentialResponse>)> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Owner)?;
    if !crate::credentials::known_kind(&body.kind) {
        return Err(ApiError::bad("unknown_credential_type"));
    }
    if let Err(message) = crate::credentials::check(&body.kind, &body.secret) {
        return Err(ApiError::bad(message));
    }
    if body.secret.to_string().len() > 256 * 1024 {
        return Err(ApiError::bad("the_secret_is_too_large"));
    }
    let blob = state.encrypt(&body.secret.to_string())?;
    let item = Credential {
        id: Uuid::new_v4().to_string(),
        team_id: team,
        name: require_name(&body.name)?,
        kind: body.kind,
        secret_blob: blob,
        created_at: db::now(),
        auto: false,
        disabled: false,
    };
    let db = state.store();
    db.insert_credential(&item)?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedCredentialResponse {
            id: item.id,
            name: item.name,
            kind: item.kind,
        }),
    ))
}

pub async fn delete_credential(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<axum::response::Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Owner)?;
    let db = state.store();
    let names = db
        .credential_usages(&id, &team)?
        .into_iter()
        .map(|usage| usage_label(&usage))
        .collect::<Vec<_>>();
    if !names.is_empty() {
        return Err(ApiError::conflict_vars(
            "credential_is_in_use",
            [("names", names.join(", "))],
        ));
    }
    db.delete_credential(&id, &team)?;
    Ok(no_content())
}

fn usage_label(usage: &db::CredentialUsage) -> String {
    match (&usage.project_id, &usage.project_name) {
        (Some(_), Some(name)) => name.clone(),
        _ => format!(
            "{} / {}",
            usage.inventory_name.as_deref().unwrap_or(""),
            usage.source_name.as_deref().unwrap_or("")
        ),
    }
}

fn usage_list(c: &dyn Backend, id: &str, team: &str) -> AppResult<Vec<CredentialUsageResponse>> {
    Ok(c.credential_usages(id, team)?
        .into_iter()
        .map(|usage| CredentialUsageResponse {
            inventory_id: usage.inventory_id,
            inventory_name: usage.inventory_name,
            source_id: usage.source_id,
            source_name: usage.source_name,
            project_id: usage.project_id,
            project_name: usage.project_name,
        })
        .collect())
}

fn open_credential(state: &Shared, id: &str, team: &str) -> AppResult<(Credential, Value)> {
    let item = {
        let db = state.store();
        db.credential(id)?
            .filter(|item| item.team_id == team)
            .ok_or(ApiError::NotFound)?
    };
    let plain = state
        .decrypt(&item.secret_blob)
        .map_err(|_| ApiError::bad("credentials_cannot_be_read"))?;
    let secret =
        serde_json::from_str(&plain).map_err(|_| ApiError::bad("credentials_are_not_json"))?;
    Ok((item, secret))
}

fn credential_detail(
    item: &Credential,
    secret: &Value,
    usages: Vec<CredentialUsageResponse>,
) -> CredentialDetailResponse {
    let (fields, set) = crate::credentials::describe(&item.kind, secret);
    CredentialDetailResponse {
        id: item.id.clone(),
        name: item.name.clone(),
        kind: item.kind.clone(),
        created_at: item.created_at.clone(),
        fields,
        set,
        usages,
    }
}

pub async fn get_credential(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<CredentialDetailResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Owner)?;
    let (item, secret) = open_credential(&state, &id, &team)?;
    let usages = {
        let db = state.store();
        usage_list(db, &item.id, &team)?
    };
    Ok(Json(credential_detail(&item, &secret, usages)))
}

pub async fn update_credential(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<CredentialUpdate>,
) -> AppResult<Json<CredentialDetailResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Owner)?;
    let (mut item, previous) = open_credential(&state, &id, &team)?;
    let secret = crate::credentials::merge_secret(&item.kind, &previous, &body.secret, &body.clear)
        .map_err(ApiError::bad)?;
    if let Err(message) = crate::credentials::check(&item.kind, &secret) {
        return Err(ApiError::bad(message));
    }
    let raw = secret.to_string();
    if raw.len() > 256 * 1024 {
        return Err(ApiError::bad("the_secret_is_too_large"));
    }
    item.name = require_name(&body.name)?;
    item.secret_blob = state.encrypt(&raw)?;
    item.disabled = false;
    let db = state.store();
    db.update_credential(&item)?;
    let usages = usage_list(db, &item.id, &team)?;
    Ok(Json(credential_detail(&item, &secret, usages)))
}

pub async fn list_environments(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<EnvironmentListResponse>> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Viewer)?;
    let db = state.store();
    Ok(Json(EnvironmentListResponse(
        db.list_environments(&team_id)?,
    )))
}

fn environment_scope(raw: Option<&str>, fallback: &str) -> AppResult<String> {
    let raw = raw.unwrap_or("").trim();
    let scope = if raw.is_empty() { fallback } else { raw };
    match scope {
        "team" | "organization" | "global" => Ok(scope.to_string()),
        _ => Err(ApiError::bad("scope_is_team_organization_or_global")),
    }
}

fn may_manage_environment(user: &AuthUser, scope: &str) -> bool {
    user.is_admin || matches!(scope, "team" | "organization")
}

fn apply_environment_scope(item: &mut Environment, scope: &str, team: &db::Team) -> AppResult<()> {
    match scope {
        "team" => {
            item.team_id = Some(team.id.clone());
            item.organization_id = None;
        }
        "organization" => {
            if team.organization_id.is_empty() {
                return Err(ApiError::bad("organization_is_missing"));
            }
            item.team_id = None;
            item.organization_id = Some(team.organization_id.clone());
        }
        "global" => {
            item.team_id = None;
            item.organization_id = None;
        }
        _ => {
            return Err(ApiError::bad("scope_is_team_organization_or_global"));
        }
    }
    Ok(())
}

fn visible_environment(db: &dyn Backend, id: &str, team: &db::Team) -> AppResult<Environment> {
    let item = db.environment(id)?.ok_or(ApiError::NotFound)?;
    if !db::environment_visible(&item, team) {
        return Err(ApiError::NotFound);
    }
    Ok(item)
}

fn environment_fields(
    user: &AuthUser,
    body: &EnvironmentBody,
    existing: Option<&Environment>,
) -> AppResult<(String, String, String, String, String, String)> {
    let legacy_unchanged = existing.is_some_and(|item| {
        item.tool == "any" && body.tool == "any" && item.image == body.image.trim()
    });
    if !legacy_unchanged && !matches!(body.tool.as_str(), "ansible" | "opentofu" | "shell") {
        return Err(ApiError::bad("choose_a_tool_for_the_environment"));
    }
    if !matches!(
        body.pull_policy.as_str(),
        "if_not_present" | "always" | "never"
    ) {
        return Err(ApiError::bad("pull_policy_is_if_not_present_always_or"));
    }
    let image = body.image.trim();
    if !runner::valid_image(image) {
        return Err(ApiError::bad("the_image_name_is_invalid"));
    }
    let network = body.network.trim();
    if !runner::valid_network(network) {
        return Err(ApiError::bad("the_network_name_is_invalid"));
    }
    if runner::privileged_network(network) && !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    Ok((
        require_name(&body.name)?,
        image.to_string(),
        body.pull_policy.clone(),
        network.to_string(),
        require_description(&body.description)?,
        body.tool.clone(),
    ))
}

pub async fn create_environment(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<EnvironmentBody>,
) -> AppResult<(StatusCode, Json<Environment>)> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Owner)?;
    let (name, image, pull_policy, network, description, tool) =
        environment_fields(&user, &body, None)?;
    let scope = environment_scope(body.scope.as_deref(), "team")?;
    if !may_manage_environment(&user, &scope) {
        return Err(ApiError::Forbidden);
    }
    let db = state.store();
    let team = db.team_by_id(&team_id)?.ok_or(ApiError::NotFound)?;
    let mut item = Environment {
        id: Uuid::new_v4().to_string(),
        team_id: None,
        organization_id: None,
        name,
        tool,
        image,
        pull_policy,
        network,
        description,
        created_at: db::now(),
        scope: String::new(),
        organization_name: String::new(),
        team_name: String::new(),
    };
    apply_environment_scope(&mut item, &scope, &team)?;
    db.insert_environment(&item)?;
    let stored = db.environment(&item.id)?.ok_or(ApiError::NotFound)?;
    Ok((StatusCode::CREATED, Json(stored)))
}

pub async fn get_environment(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Environment>> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Viewer)?;
    let db = state.store();
    let team = db.team_by_id(&team_id)?.ok_or(ApiError::NotFound)?;
    let item = visible_environment(db, &id, &team)?;
    Ok(Json(item))
}

pub async fn update_environment(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<EnvironmentBody>,
) -> AppResult<Json<Environment>> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Owner)?;
    let db = state.store();
    let team = db.team_by_id(&team_id)?.ok_or(ApiError::NotFound)?;
    let mut item = visible_environment(db, &id, &team)?;
    let (name, image, pull_policy, network, description, tool) =
        environment_fields(&user, &body, Some(&item))?;
    let scope = environment_scope(body.scope.as_deref(), &item.scope)?;
    if !may_manage_environment(&user, &item.scope) || !may_manage_environment(&user, &scope) {
        return Err(ApiError::Forbidden);
    }
    item.name = name;
    item.image = image;
    item.tool = tool;
    item.pull_policy = pull_policy;
    item.network = network;
    item.description = description;
    apply_environment_scope(&mut item, &scope, &team)?;
    db.update_environment(&item)?;
    let stored = db.environment(&item.id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(stored))
}

pub async fn delete_environment(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<axum::response::Response> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Owner)?;
    let db = state.store();
    let team = db.team_by_id(&team_id)?.ok_or(ApiError::NotFound)?;
    let item = visible_environment(db, &id, &team)?;
    if !may_manage_environment(&user, &item.scope) {
        return Err(ApiError::Forbidden);
    }
    db.delete_environment(&id)?;
    Ok(no_content())
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

pub async fn list_organizations(
    State(state): State<Shared>,
    user: AuthUser,
) -> AppResult<Json<OrganizationListResponse>> {
    let db = state.store();
    let items = db.list_organizations(&user.id, user.is_admin)?;
    Ok(Json(OrganizationListResponse(items)))
}

pub async fn create_organization(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<OrganizationBody>,
) -> AppResult<(StatusCode, Json<Organization>)> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    let name = require_name(&body.name)?;
    let id = Uuid::new_v4().to_string();
    let db = state.store();
    db.insert_organization(&id, &name, &require_description(&body.description)?)?;
    let item = db.organization(&id)?.ok_or(ApiError::NotFound)?;
    Ok((StatusCode::CREATED, Json(item)))
}

pub async fn get_organization(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<OrganizationDetailResponse>> {
    let db = state.store();
    let item = db.organization(&id)?.ok_or(ApiError::NotFound)?;
    if !user.is_admin
        && !db
            .list_organizations(&user.id, false)?
            .iter()
            .any(|org| org.id == id)
    {
        return Err(ApiError::Forbidden);
    }
    let teams = db.teams_in_organization(&id)?;
    Ok(Json(OrganizationDetailResponse {
        id: item.id,
        name: item.name,
        description: item.description,
        created_at: item.created_at,
        teams: teams.iter().map(TeamResponse::new).collect(),
    }))
}

pub async fn update_organization(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<OrganizationBody>,
) -> AppResult<Json<Organization>> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    let name = require_name(&body.name)?;
    let db = state.store();
    db.update_organization(&id, &name, &require_description(&body.description)?)?;
    let item = db.organization(&id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(item))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> AuthUser {
        AuthUser {
            id: "user".into(),
            username: "ada".into(),
            display_name: "Ada".into(),
            auth_source: "local".into(),
            email: None,
            is_admin: false,
        }
    }

    fn body(tool: &str) -> EnvironmentBody {
        EnvironmentBody {
            name: "EE".into(),
            image: "quay.io/example/ee:1".into(),
            tool: tool.into(),
            pull_policy: "if_not_present".into(),
            network: String::new(),
            description: String::new(),
            scope: None,
        }
    }

    fn legacy_environment() -> Environment {
        Environment {
            id: "legacy".into(),
            team_id: None,
            organization_id: None,
            name: "EE".into(),
            tool: "any".into(),
            image: "quay.io/example/ee:1".into(),
            pull_policy: "if_not_present".into(),
            network: String::new(),
            description: String::new(),
            created_at: String::new(),
            scope: "global".into(),
            organization_name: String::new(),
            team_name: String::new(),
        }
    }

    #[test]
    fn new_environments_need_ansible_opentofu_or_shell() {
        for tool in ["ansible", "opentofu", "shell"] {
            let (_, _, _, _, _, parsed) = environment_fields(&user(), &body(tool), None).unwrap();
            assert_eq!(parsed, tool);
        }
        for tool in ["any", "docker", ""] {
            let err = environment_fields(&user(), &body(tool), None).unwrap_err();
            assert_eq!(err.to_string(), "choose_a_tool_for_the_environment");
        }
    }

    #[test]
    fn a_legacy_any_environment_survives_an_unchanged_save() {
        let legacy = legacy_environment();
        let (_, _, _, _, _, parsed) =
            environment_fields(&user(), &body("any"), Some(&legacy)).unwrap();
        assert_eq!(parsed, "any");
        let mut changed = body("any");
        changed.image = "quay.io/example/ee:2".into();
        let err = environment_fields(&user(), &changed, Some(&legacy)).unwrap_err();
        assert_eq!(err.to_string(), "choose_a_tool_for_the_environment");
    }
}
