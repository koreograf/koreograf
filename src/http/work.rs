use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::crypto;
use crate::db::{self, Delivery, Hook, Template};
use crate::error::{ApiError, ApiJson, ApiJsonValue, AppResult};
use crate::features::credentials;
use crate::features::identity::{AuthUser, Role};
use crate::features::jobs::{self, JobResponse, LaunchRequest};
use crate::runner;
use crate::Shared;

use super::{
    blank, empty_object, gate, no_content, require_description, require_name, team_header,
};

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/templates", get(list_templates).post(create_template))
        .route(
            "/templates/{id}",
            put(update_template).delete(delete_template),
        )
        .route("/templates/{id}/launch", post(launch_template))
        .route("/hooks", get(list_hooks).post(create_hook))
        .route("/hooks/{id}", delete(delete_hook).put(update_hook))
        .route("/hooks/{id}/test", post(test_hook))
        .route("/hooks/{id}/deliveries", get(list_deliveries))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateBody {
    name: String,
    #[serde(default)]
    description: String,
    tool: String,
    project_id: Option<String>,
    inventory_id: Option<String>,
    credential_id: Option<String>,
    credential_ids: Option<Vec<String>>,
    environment_id: Option<String>,
    #[serde(default)]
    no_environment: bool,
    #[serde(default)]
    entrypoint: String,
    extra_vars: Option<Value>,
    spec: Option<Value>,
    #[serde(default = "default_true")]
    update_on_launch: bool,
    #[serde(default)]
    prompt_on_launch: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaunchBody {
    #[serde(default = "empty_object")]
    pub(crate) extra_vars: Value,
    /// Empty string clears the inventory. Omit the field to keep the template value.
    #[serde(default)]
    inventory_id: Option<String>,
    /// Replaces the template credentials for this job. Omit the field to keep them.
    #[serde(default)]
    credential_ids: Option<Vec<String>>,
    /// Ansible prompt fields merged onto the template spec.
    #[serde(default)]
    spec: Option<Value>,
    #[serde(default)]
    launch_with_options: bool,
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
pub(super) struct TemplateResponse {
    id: String,
    team_id: String,
    name: String,
    description: String,
    tool: String,
    project_id: Option<String>,
    inventory_id: Option<String>,
    credential_id: Option<String>,
    credential_ids: Vec<String>,
    environment_id: Option<String>,
    no_environment: bool,
    entrypoint: String,
    extra_vars: Value,
    spec: Value,
    created_at: String,
    update_on_launch: bool,
    prompt_on_launch: bool,
}

impl TemplateResponse {
    fn new(template: &Template, credential_ids: &[String]) -> Self {
        let credential_ids = if credential_ids.is_empty() {
            template
                .credential_id
                .clone()
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            credential_ids.to_vec()
        };
        Self {
            id: template.id.clone(),
            team_id: template.team_id.clone(),
            name: template.name.clone(),
            description: template.description.clone(),
            tool: template.tool.clone(),
            project_id: template.project_id.clone(),
            inventory_id: template.inventory_id.clone(),
            credential_id: template.credential_id.clone(),
            credential_ids,
            environment_id: template.environment_id.clone(),
            no_environment: template.no_environment,
            entrypoint: template.entrypoint.clone(),
            extra_vars: db::json_object(&template.extra_vars),
            spec: db::json_object(&template.spec),
            created_at: template.created_at.clone(),
            update_on_launch: template.update_on_launch,
            prompt_on_launch: template.prompt_on_launch,
        }
    }
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct TemplateListResponse(Vec<TemplateResponse>);

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
pub(super) struct DispatchResponse {
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

pub async fn list_templates(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<TemplateListResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let items = db.list_templates(&team)?;
    let links = db.list_template_credential_links(&team)?;
    let mut by_template: HashMap<String, Vec<String>> = HashMap::new();
    for (template_id, credential_id) in links {
        by_template
            .entry(template_id)
            .or_default()
            .push(credential_id);
    }
    Ok(Json(TemplateListResponse(
        items
            .iter()
            .map(|item| {
                let ids = by_template.get(&item.id).map(Vec::as_slice).unwrap_or(&[]);
                TemplateResponse::new(item, ids)
            })
            .collect(),
    )))
}

pub async fn create_template(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<TemplateBody>,
) -> AppResult<(StatusCode, Json<TemplateResponse>)> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let (mut template, credential_ids) = build_template(&state, &team, None, body)?;
    template.admin_saved = user.is_admin;
    let db = state.store();
    db.insert_template(&template, &credential_ids)?;
    Ok((
        StatusCode::CREATED,
        Json(TemplateResponse::new(&template, &credential_ids)),
    ))
}

pub async fn update_template(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<TemplateBody>,
) -> AppResult<Json<TemplateResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let (mut template, credential_ids) = build_template(&state, &team, Some(id), body)?;
    template.admin_saved = user.is_admin;
    let db = state.store();
    db.update_template(&template, &credential_ids)?;
    Ok(Json(TemplateResponse::new(&template, &credential_ids)))
}

pub async fn delete_template(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.store();
    db.delete_template(&id, &team)?;
    Ok(no_content())
}

pub async fn launch_template(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<LaunchBody>,
) -> AppResult<(StatusCode, Json<JobResponse>)> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let extra = body.extra_vars;
    if !extra.is_object() {
        return Err(ApiError::bad("extra_vars_must_be_an_object"));
    }
    let job_id = jobs::launch(
        &state,
        LaunchRequest {
            team_id: team,
            template_id: id,
            extra_vars: extra,
            launched_by: user.username,
            trigger: "manual".into(),
            workflow_job_id: None,
            workflow_node_id: None,
            parameters_override: None,
            overrides: jobs::LaunchOverrides {
                inventory_id: body.inventory_id,
                credential_ids: body.credential_ids,
                spec: body.spec,
                launch_with_options: body.launch_with_options,
                extra_vars_are_overrides: true,
            },
        },
    )?;
    let db = state.store();
    let job = db.job(&job_id)?.ok_or(ApiError::NotFound)?;
    Ok((StatusCode::ACCEPTED, Json(JobResponse::new(&job, None))))
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

fn build_template(
    state: &Shared,
    team: &str,
    id: Option<String>,
    body: TemplateBody,
) -> AppResult<(Template, Vec<String>)> {
    let requested_ids = requested_credential_ids(&body);
    if !runner::known_tool(&body.tool) {
        return Err(ApiError::bad("tool_is_ansible_opentofu_or_shell"));
    }
    runner::check_relative(body.entrypoint.trim()).map_err(ApiError::from)?;
    let extra = body.extra_vars.unwrap_or_else(empty_object);
    let spec = body.spec.unwrap_or_else(empty_object);
    if !extra.is_object() || !spec.is_object() {
        return Err(ApiError::bad("template.vars_not_objects"));
    }
    if body.tool == "ansible" {
        runner::check_ansible_spec(&spec).map_err(ApiError::from)?;
    }
    if let Some(action) = spec.get("action").and_then(Value::as_str) {
        if !matches!(action, "plan" | "apply") {
            return Err(ApiError::bad("action_is_plan_or_apply"));
        }
    }
    let entrypoint = body.entrypoint.trim().to_string();
    let inline = spec.get("script").and_then(Value::as_str).unwrap_or("");
    if body.tool == "ansible" && entrypoint.is_empty() {
        return Err(ApiError::bad("ansible_needs_a_playbook_as_the_entry"));
    }
    if body.tool == "opentofu" && entrypoint.is_empty() {
        return Err(ApiError::bad("opentofu_needs_a_module_directory"));
    }
    if body.tool == "shell" && entrypoint.is_empty() && inline.is_empty() {
        return Err(ApiError::bad("shell_needs_a_path_or_spec_script"));
    }
    let project_id = blank(body.project_id);
    let inventory_id = blank(body.inventory_id);
    let environment_id = blank(body.environment_id);
    let no_environment = environment_id.is_none() && body.no_environment;
    let (created_at, loaded, previous_ids) = {
        let db = state.store();
        if let Some(project_id) = &project_id {
            db.project(project_id)?
                .ok_or_else(|| ApiError::bad("project_is_missing"))?;
            if !db.project_visible(project_id, team)? {
                return Err(ApiError::bad("this_project_is_not_visible_to_this_team"));
            }
        }
        if let Some(inventory_id) = &inventory_id {
            let inventory = db.inventory(inventory_id)?.ok_or(ApiError::NotFound)?;
            if inventory.team_id != team {
                return Err(ApiError::NotFound);
            }
        }
        if let Some(environment_id) = &environment_id {
            let environment = db
                .environment(environment_id)?
                .ok_or_else(|| ApiError::bad("execution_environment_is_missing"))?;
            let owner = db.team_by_id(team)?.ok_or(ApiError::NotFound)?;
            if !db::environment_visible(&environment, &owner) {
                return Err(ApiError::bad(
                    "the_execution_environment_is_not_visible_to_this",
                ));
            }
            if !runner::environment_fits_tool(&body.tool, &environment.tool) {
                return Err(ApiError::bad(
                    "the_execution_environment_is_for_another_tool",
                ));
            }
        }
        let created_at = if let Some(id) = &id {
            db.template(id)?
                .map(|item| item.created_at)
                .unwrap_or_else(db::now)
        } else {
            db::now()
        };
        let mut loaded = Vec::with_capacity(requested_ids.len());
        for credential_id in &requested_ids {
            let credential = db.credential(credential_id)?.ok_or(ApiError::NotFound)?;
            loaded.push(credential);
        }
        let previous_ids = match &id {
            Some(id) => db.template_credential_ids(id)?,
            None => Vec::new(),
        };
        (created_at, loaded, previous_ids)
    };
    let chosen = credentials::assign_loaded(state, team, &loaded)?;
    for credential in &loaded {
        if !credentials::kind_fits_tool(&body.tool, &credential.kind)
            && !previous_ids.iter().any(|id| id == &credential.id)
        {
            return Err(ApiError::bad("this_credential_kind_does_not_fit_the_tool"));
        }
    }
    let credential_ids = chosen.ids;
    let credential_id = chosen.primary_id;
    Ok((
        Template {
            id: id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            team_id: team.to_string(),
            name: require_name(&body.name)?,
            description: require_description(&body.description)?,
            tool: body.tool,
            project_id,
            inventory_id,
            credential_id,
            environment_id,
            no_environment,
            entrypoint,
            extra_vars: extra.to_string(),
            spec: spec.to_string(),
            created_at,
            update_on_launch: body.update_on_launch,
            admin_saved: false,
            prompt_on_launch: body.prompt_on_launch,
        },
        credential_ids,
    ))
}

fn requested_credential_ids(body: &TemplateBody) -> Vec<String> {
    if let Some(ids) = &body.credential_ids {
        return ids.iter().map(|id| id.trim().to_string()).collect();
    }
    blank(body.credential_id.clone()).into_iter().collect()
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
