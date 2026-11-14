use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::crypto;
use crate::db::{self, Delivery, Hook, Template, Workflow};
use crate::error::{ApiError, ApiJson, ApiJsonValue, AppResult};
use crate::features::identity::{AuthUser, Role};
use crate::features::jobs;
use crate::features::projects::LaunchBody;
use crate::http::{
    blank, empty_object, gate, no_content, require_description, require_name, team_header,
};
use crate::runner;
use crate::Shared;

use super::model::{Graph, GraphNode};
use super::service::{apply_overrides, validate};

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/workflows", get(list_workflows).post(create_workflow))
        .route(
            "/workflows/{id}",
            get(get_workflow)
                .put(update_workflow)
                .delete(delete_workflow),
        )
        .route("/workflows/{id}/launch", post(launch_workflow))
        .route("/workflow-jobs", get(list_workflow_jobs))
        .route("/workflow-jobs/{id}", get(get_workflow_job))
        .route("/workflow-jobs/{id}/cancel", post(cancel_workflow_job))
        .route("/workflow-jobs/{id}/relaunch", post(relaunch_workflow_job))
        .route("/hooks", get(list_hooks).post(create_hook))
        .route("/hooks/{id}", delete(delete_hook).put(update_hook))
        .route("/hooks/{id}/test", post(test_hook))
        .route("/hooks/{id}/deliveries", get(list_deliveries))
}

fn empty_graph() -> Value {
    let mut graph = serde_json::Map::new();
    graph.insert("nodes".into(), Value::Array(Vec::new()));
    graph.insert("edges".into(), Value::Array(Vec::new()));
    Value::Object(graph)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowBody {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    graph: Graph,
    #[serde(default)]
    version: Option<i64>,
}

#[derive(Serialize)]
pub(super) struct WorkflowResponse {
    id: String,
    team_id: String,
    name: String,
    description: String,
    graph: Value,
    created_at: String,
    version: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_run_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_run_at: Option<String>,
}

impl WorkflowResponse {
    fn new(item: &Workflow, last: Option<&db::WorkflowJob>) -> Self {
        Self {
            id: item.id.clone(),
            team_id: item.team_id.clone(),
            name: item.name.clone(),
            description: item.description.clone(),
            graph: db::json_object(&item.graph),
            created_at: item.created_at.clone(),
            version: item.version,
            last_run_id: last.map(|run| run.id.clone()),
            last_run_status: last.map(|run| run.status.clone()),
            last_run_at: last.map(|run| run.created_at.clone()),
        }
    }
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct WorkflowListResponse(Vec<WorkflowResponse>);

#[derive(Serialize)]
pub(super) struct WorkflowNodeResponse {
    node_id: String,
    phase: String,
    job_id: Option<String>,
    job_name: Option<String>,
    tool: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    job_created_at: Option<String>,
}

impl WorkflowNodeResponse {
    fn new(db: &dyn crate::backend::Backend, node: &db::WorkflowNodeRow) -> AppResult<Self> {
        let job = match &node.job_id {
            Some(id) => db.job(id)?,
            None => None,
        };
        Ok(Self {
            node_id: node.node_id.clone(),
            phase: node.phase.clone(),
            job_id: node.job_id.clone(),
            job_name: job.as_ref().map(|item| item.name.clone()),
            tool: job.as_ref().map(|item| item.tool.clone()),
            started_at: job.as_ref().and_then(|item| item.started_at.clone()),
            finished_at: job.as_ref().and_then(|item| item.finished_at.clone()),
            job_created_at: job.as_ref().map(|item| item.created_at.clone()),
        })
    }
}

#[derive(Serialize)]
pub(super) struct WorkflowJobResponse {
    id: String,
    workflow_id: Option<String>,
    team_id: String,
    name: String,
    status: String,
    extra_vars: Value,
    launched_by: String,
    trigger: String,
    created_at: String,
    finished_at: Option<String>,
    graph: Value,
    nodes: Vec<WorkflowNodeResponse>,
}

impl WorkflowJobResponse {
    fn new(db: &dyn crate::backend::Backend, run: &db::WorkflowJob) -> AppResult<Self> {
        let graph = run
            .workflow_id
            .as_deref()
            .and_then(|id| db.workflow(id).ok().flatten())
            .map(|item| db::json_object(&item.graph))
            .unwrap_or_else(empty_graph);
        let nodes = db
            .workflow_nodes(&run.id)?
            .iter()
            .map(|node| WorkflowNodeResponse::new(db, node))
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Self {
            id: run.id.clone(),
            workflow_id: run.workflow_id.clone(),
            team_id: run.team_id.clone(),
            name: run.name.clone(),
            status: run.status.clone(),
            extra_vars: db::json_object(&run.extra_vars),
            launched_by: run.launched_by.clone(),
            trigger: run.trigger.clone(),
            created_at: run.created_at.clone(),
            finished_at: run.finished_at.clone(),
            graph,
            nodes,
        })
    }
}

#[derive(Serialize)]
pub(super) struct WorkflowRunResponse {
    id: String,
    workflow_id: Option<String>,
    team_id: String,
    name: String,
    status: String,
    launched_by: String,
    trigger: String,
    created_at: String,
    finished_at: Option<String>,
}

impl WorkflowRunResponse {
    fn new(run: &db::WorkflowJob) -> Self {
        Self {
            id: run.id.clone(),
            workflow_id: run.workflow_id.clone(),
            team_id: run.team_id.clone(),
            name: run.name.clone(),
            status: run.status.clone(),
            launched_by: run.launched_by.clone(),
            trigger: run.trigger.clone(),
            created_at: run.created_at.clone(),
            finished_at: run.finished_at.clone(),
        }
    }
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct WorkflowRunListResponse(Vec<WorkflowRunResponse>);

#[derive(Deserialize, Default)]
pub struct WorkflowJobsQuery {
    workflow_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRelaunchBody {}

pub(crate) async fn list_workflows(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<WorkflowListResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let items = db.list_workflows(&team)?;
    let mut latest: HashMap<String, db::WorkflowJob> = HashMap::new();
    for run in db.list_workflow_jobs(&team, "", 1000)? {
        if let Some(workflow_id) = run.workflow_id.clone() {
            latest.entry(workflow_id).or_insert(run);
        }
    }
    Ok(Json(WorkflowListResponse(
        items
            .iter()
            .map(|item| WorkflowResponse::new(item, latest.get(&item.id)))
            .collect(),
    )))
}

pub(crate) async fn get_workflow(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<WorkflowResponse>> {
    let db = state.store();
    let item = db.workflow(&id)?.ok_or(ApiError::NotFound)?;
    crate::features::identity::authorize(db, &user, &item.team_id, Role::Viewer)?;
    let last = db.list_workflow_jobs(&item.team_id, &item.id, 1)?;
    Ok(Json(WorkflowResponse::new(&item, last.first())))
}

pub(crate) async fn create_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<WorkflowBody>,
) -> AppResult<(StatusCode, Json<WorkflowResponse>)> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let mut item = build_workflow(&state, &team, None, body)?;
    item.admin_saved = user.is_admin;
    let db = state.store();
    db.insert_workflow(&item)?;
    Ok((
        StatusCode::CREATED,
        Json(WorkflowResponse::new(&item, None)),
    ))
}

pub(crate) async fn update_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<WorkflowBody>,
) -> AppResult<Json<WorkflowResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let expected_version = body.version;
    let db = state.store();
    let current = db.workflow(&id)?.ok_or(ApiError::NotFound)?;
    if current.team_id != team {
        return Err(ApiError::NotFound);
    }
    if expected_version.is_some_and(|version| version != current.version) {
        return Err(ApiError::conflict_vars(
            "workflow_changed",
            [("version", current.version.to_string())],
        ));
    }
    let mut item = build_workflow(&state, &team, Some(id), body)?;
    item.admin_saved = user.is_admin;
    item.version = current.version;
    db.update_workflow(&item, expected_version)?;
    let item = db.workflow(&item.id)?.ok_or(ApiError::NotFound)?;
    let last = db.list_workflow_jobs(&item.team_id, &item.id, 1)?;
    Ok(Json(WorkflowResponse::new(&item, last.first())))
}

pub(crate) async fn delete_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.store();
    db.delete_workflow(&id, &team)?;
    Ok(no_content())
}

pub(crate) async fn launch_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<LaunchBody>,
) -> AppResult<(StatusCode, Json<WorkflowJobResponse>)> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let extra = body.extra_vars;
    if !extra.is_object() {
        return Err(ApiError::bad("extra_vars_must_be_an_object"));
    }
    let run_id = jobs::launch_workflow(&state, &team, &id, extra, &user.username, "manual")?;
    let db = state.store();
    let run = db.workflow_job(&run_id)?.ok_or(ApiError::NotFound)?;
    // The connection mutex is not re-entrant. A second lock here froze every
    // request, including health checks, while this handler still held it.
    Ok((
        StatusCode::ACCEPTED,
        Json(WorkflowJobResponse::new(db, &run)?),
    ))
}

pub(crate) async fn get_workflow_job(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<WorkflowJobResponse>> {
    let db = state.store();
    let run = db.workflow_job(&id)?.ok_or(ApiError::NotFound)?;
    crate::features::identity::authorize(db, &user, &run.team_id, Role::Viewer)?;
    Ok(Json(WorkflowJobResponse::new(db, &run)?))
}

pub(crate) async fn list_workflow_jobs(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Query(query): Query<WorkflowJobsQuery>,
) -> AppResult<Json<WorkflowRunListResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let workflow_id = query
        .workflow_id
        .map(|id| id.trim().to_string())
        .unwrap_or_default();
    let db = state.store();
    if !workflow_id.is_empty() {
        let owner = db.workflow_team(&workflow_id)?.ok_or(ApiError::NotFound)?;
        if owner != team {
            return Err(ApiError::NotFound);
        }
    }
    let runs = db.list_workflow_jobs(&team, &workflow_id, 100)?;
    Ok(Json(WorkflowRunListResponse(
        runs.iter().map(WorkflowRunResponse::new).collect(),
    )))
}

pub(crate) async fn cancel_workflow_job(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<WorkflowJobResponse>> {
    {
        let db = state.store();
        let run = db.workflow_job(&id)?.ok_or(ApiError::NotFound)?;
        crate::features::identity::authorize(db, &user, &run.team_id, Role::Operator)?;
    }
    jobs::cancel_workflow(&state, &id)?;
    let db = state.store();
    let run = db.workflow_job(&id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(WorkflowJobResponse::new(db, &run)?))
}

pub(crate) async fn relaunch_workflow_job(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(_body): ApiJson<WorkflowRelaunchBody>,
) -> AppResult<(StatusCode, Json<WorkflowJobResponse>)> {
    {
        let db = state.store();
        let run = db.workflow_job(&id)?.ok_or(ApiError::NotFound)?;
        crate::features::identity::authorize(db, &user, &run.team_id, Role::Operator)?;
    }
    let run_id = jobs::relaunch_workflow(&state, &id, &user.username)?;
    let db = state.store();
    let run = db.workflow_job(&run_id)?.ok_or(ApiError::NotFound)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(WorkflowJobResponse::new(db, &run)?),
    ))
}

fn check_node_overrides(
    store: &dyn crate::backend::Backend,
    team: &str,
    template: &Template,
    node: &GraphNode,
) -> AppResult<()> {
    let over = &node.overrides;
    if let Some(id) = over.inventory_id.as_deref().filter(|id| !id.is_empty()) {
        let inventory = store.inventory(id)?.ok_or(ApiError::NotFound)?;
        if inventory.team_id != team {
            return Err(ApiError::NotFound);
        }
    }
    if let Some(id) = over.credential_id.as_deref().filter(|id| !id.is_empty()) {
        let credential = store.credential(id)?.ok_or(ApiError::NotFound)?;
        if credential.team_id != team {
            return Err(ApiError::NotFound);
        }
    }
    if let Some(extra) = &over.extra_vars {
        if !extra.is_object() {
            return Err(ApiError::bad("extra_variables_must_be_a_json_object"));
        }
    }
    if let Some(spec) = &over.spec {
        if !spec.is_object() {
            return Err(ApiError::bad("spec_must_be_an_object"));
        }
    }
    if over.spec.is_some() {
        let mut params = serde_json::Map::new();
        params.insert("spec".into(), db::json_object(&template.spec));
        apply_overrides(&mut params, over);
        let spec = params.get("spec").cloned().unwrap_or_else(empty_object);
        if template.tool == "ansible" {
            runner::check_ansible_spec(&spec).map_err(ApiError::from)?;
        }
        if template.tool == "opentofu" {
            if let Some(action) = spec.get("action").and_then(Value::as_str) {
                if !matches!(action, "plan" | "apply") {
                    return Err(ApiError::bad("action_is_plan_or_apply"));
                }
            }
        }
    }
    let size = serde_json::to_string(over)
        .map_err(|err| ApiError::Internal(err.to_string()))?
        .len();
    if size > 64 * 1024 {
        return Err(ApiError::bad("parameters_are_too_large"));
    }
    Ok(())
}

fn build_workflow(
    state: &Shared,
    team: &str,
    id: Option<String>,
    body: WorkflowBody,
) -> AppResult<Workflow> {
    validate(&body.graph).map_err(ApiError::bad)?;
    let db = state.store();
    for node in &body.graph.nodes {
        let template = db.template(&node.template_id)?.ok_or_else(|| {
            ApiError::bad_vars("template.missing_named", [("name", node.name.as_str())])
        })?;
        if template.team_id != team {
            return Err(ApiError::NotFound);
        }
        check_node_overrides(db, team, &template, node)?;
    }
    let created_at = if let Some(id) = &id {
        db.workflow(id)?
            .map(|item| item.created_at)
            .unwrap_or_else(db::now)
    } else {
        db::now()
    };
    Ok(Workflow {
        id: id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        team_id: team.to_string(),
        name: require_name(&body.name)?,
        description: require_description(&body.description)?,
        graph: serde_json::to_string(&body.graph)
            .map_err(|err| ApiError::Internal(err.to_string()))?,
        created_at,
        admin_saved: false,
        version: 1,
    })
}
#[derive(Serialize)]
struct TestHookPayload {
    status: &'static str,
    alerts: Vec<TestHookAlert>,
}

#[derive(Serialize)]
struct TestHookAlert {
    status: &'static str,
    labels: TestHookLabels,
    annotations: TestHookAnnotations,
}

#[derive(Serialize)]
struct TestHookLabels {
    alertname: String,
    instance: &'static str,
    severity: &'static str,
}

#[derive(Serialize)]
struct TestHookAnnotations {
    summary: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookBody {
    name: String,
    match_alertname: String,
    #[serde(default = "default_status")]
    match_status: String,
    template_id: Option<String>,
    workflow_id: Option<String>,
    extra_vars: Option<Value>,
}

fn default_status() -> String {
    "firing".into()
}

#[derive(Serialize)]
pub(super) struct HookResponse {
    id: String,
    team_id: String,
    name: String,
    token: String,
    match_alertname: String,
    match_status: String,
    template_id: Option<String>,
    workflow_id: Option<String>,
    extra_vars: Value,
    enabled: bool,
    created_at: String,
}

impl HookResponse {
    fn new(hook: &Hook, reveal_token: bool) -> Self {
        Self {
            id: hook.id.clone(),
            team_id: hook.team_id.clone(),
            name: hook.name.clone(),
            token: if reveal_token {
                hook.token.clone()
            } else {
                String::new()
            },
            match_alertname: hook.match_alertname.clone(),
            match_status: hook.match_status.clone(),
            template_id: hook.template_id.clone(),
            workflow_id: hook.workflow_id.clone(),
            extra_vars: db::json_object(&hook.extra_vars),
            enabled: hook.enabled,
            created_at: hook.created_at.clone(),
        }
    }
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct HookListResponse(Vec<HookResponse>);

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct DeliveryListResponse(Vec<Delivery>);

#[derive(Serialize)]
pub(crate) struct DispatchResponse {
    status: String,
    summary: String,
    job_id: Option<String>,
    workflow_job_id: Option<String>,
}

impl DispatchResponse {
    fn new(dispatch: &jobs::Dispatch) -> Self {
        Self {
            status: dispatch.status.clone(),
            summary: dispatch.summary.clone(),
            job_id: dispatch.job_id.clone(),
            workflow_job_id: dispatch.workflow_job_id.clone(),
        }
    }
}

pub async fn list_hooks(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<HookListResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let reveal = can_operate(&state, &user, &team)?;
    let db = state.store();
    let hooks = db.list_hooks(&team)?;
    Ok(Json(HookListResponse(
        hooks
            .iter()
            .map(|hook| HookResponse::new(hook, reveal))
            .collect(),
    )))
}

pub async fn create_hook(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<HookBody>,
) -> AppResult<(StatusCode, Json<HookResponse>)> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let hook = build_hook(&state, &team, None, body)?;
    let db = state.store();
    db.insert_hook(&hook)?;
    Ok((StatusCode::CREATED, Json(HookResponse::new(&hook, true))))
}

pub async fn update_hook(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<HookBody>,
) -> AppResult<Json<HookResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let current = {
        let db = state.store();
        db.hook(&id)?.ok_or(ApiError::NotFound)?
    };
    if current.team_id != team {
        return Err(ApiError::NotFound);
    }
    let hook = build_hook(&state, &team, Some(&current), body)?;
    let db = state.store();
    db.update_hook(&hook)?;
    Ok(Json(HookResponse::new(&hook, true)))
}

pub async fn delete_hook(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.store();
    db.delete_hook(&id, &team)?;
    Ok(no_content())
}

pub async fn list_deliveries(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<DeliveryListResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    Ok(Json(DeliveryListResponse(db.list_deliveries(&team, 30)?)))
}

pub async fn test_hook(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<(StatusCode, Json<DispatchResponse>)> {
    let hook = {
        let db = state.store();
        let hook = db.hook(&id)?.ok_or(ApiError::NotFound)?;
        crate::features::identity::authorize(db, &user, &hook.team_id, Role::Operator)?;
        hook
    };
    let payload = TestHookPayload {
        status: "firing",
        alerts: vec![TestHookAlert {
            status: "firing",
            labels: TestHookLabels {
                alertname: hook.match_alertname.clone(),
                instance: "web-1",
                severity: "warning",
            },
            annotations: TestHookAnnotations {
                summary: "Test delivery from the Koreograf UI",
            },
        }],
    };
    let payload = serde_json::to_value(payload).unwrap_or_default();
    let dispatch = jobs::accept_hook(&state, &hook, &payload, Some(&user.username))?;
    Ok((StatusCode::ACCEPTED, Json(DispatchResponse::new(&dispatch))))
}

pub async fn ingest(
    State(state): State<Shared>,
    Path((id, token)): Path<(String, String)>,
    ApiJsonValue(payload): ApiJsonValue,
) -> AppResult<Json<DispatchResponse>> {
    let hook = {
        let db = state.store();
        db.hook(&id)?.ok_or(ApiError::NotFound)?
    };
    if !crypto::tokens_equal(&hook.token, &token) {
        return Err(ApiError::Unauthorized);
    }
    let dispatch = jobs::accept_hook(&state, &hook, &payload, None)?;
    Ok(Json(DispatchResponse::new(&dispatch)))
}

fn can_operate(state: &Shared, user: &AuthUser, team: &str) -> AppResult<bool> {
    if user.is_admin {
        return Ok(true);
    }
    let db = state.store();
    let Some(role) = db.role_of(team, &user.id)? else {
        return Ok(false);
    };
    Ok(Role::parse(&role).is_some_and(|role| role >= Role::Operator))
}
fn build_hook(
    state: &Shared,
    team: &str,
    existing: Option<&Hook>,
    body: HookBody,
) -> AppResult<Hook> {
    let template_id = blank(body.template_id);
    let workflow_id = blank(body.workflow_id);
    match (&template_id, &workflow_id) {
        (Some(_), Some(_)) => return Err(ApiError::bad("hook.one_target")),
        (None, None) => return Err(ApiError::bad("hook.needs_target")),
        _ => {}
    }
    if !matches!(body.match_status.as_str(), "firing" | "resolved" | "any") {
        return Err(ApiError::bad("status_is_firing_resolved_or_any"));
    }
    let alertname = body.match_alertname.trim();
    if alertname.is_empty() {
        return Err(ApiError::bad("alert_name_is_missing"));
    }
    let extra = body.extra_vars.unwrap_or_else(empty_object);
    if !extra.is_object() {
        return Err(ApiError::bad("extra_vars_must_be_an_object"));
    }
    let db = state.store();
    if let Some(template_id) = &template_id {
        let template = db.template(template_id)?.ok_or(ApiError::NotFound)?;
        if template.team_id != team {
            return Err(ApiError::NotFound);
        }
    }
    if let Some(workflow_id) = &workflow_id {
        let owner = db.workflow_team(workflow_id)?.ok_or(ApiError::NotFound)?;
        if owner != team {
            return Err(ApiError::NotFound);
        }
    }
    Ok(Hook {
        id: existing.map_or_else(|| Uuid::new_v4().to_string(), |hook| hook.id.clone()),
        team_id: team.to_string(),
        name: require_name(&body.name)?,
        token: existing.map_or_else(crypto::random_token, |hook| hook.token.clone()),
        match_alertname: alertname.to_string(),
        match_status: body.match_status,
        template_id,
        workflow_id,
        extra_vars: extra.to_string(),
        enabled: existing.is_none_or(|hook| hook.enabled),
        created_at: existing.map_or_else(db::now, |hook| hook.created_at.clone()),
    })
}
