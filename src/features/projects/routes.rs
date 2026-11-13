use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::db::{self, Project, Template};
use crate::error::{ApiError, ApiJson, AppResult};
use crate::features::credentials;
use crate::features::identity::{AuthUser, Role};
use crate::features::inventories::{scan_project, ProjectContents};
use crate::features::jobs::{self, JobResponse, LaunchRequest};
use crate::http::{
    blank, empty_object, gate, no_content, require_description, require_name, team_header,
};
use crate::runner;
use crate::Shared;

use super::model::{
    LaunchBody, ProjectBody, ProjectListResponse, ProjectResponse, ProjectUpdateBody,
    SyncProjectResponse, SyncResponse,
};
use super::service::{
    check_team_ids, project_dir, validate_project_credential, validate_project_source,
    visible_project,
};

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/projects", get(list_projects).post(create_project))
        .route(
            "/projects/{id}",
            get(get_project).put(update_project).delete(delete_project),
        )
        .route("/projects/{id}/sync", post(sync_project))
        .route("/projects/{id}/contents", get(project_contents))
        .route("/templates", get(list_templates).post(create_template))
        .route(
            "/templates/{id}",
            put(update_template).delete(delete_template),
        )
        .route("/templates/{id}/launch", post(launch_template))
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

pub async fn project_contents(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<ProjectContents>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let project = visible_project(&state, &id, &team)?;
    let root = project_dir(&state, &project)?;
    let contents = scan_project(&root).map_err(ApiError::bad)?;
    Ok(Json(contents))
}
