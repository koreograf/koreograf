use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Response;
use axum::Json;
use futures::stream::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{AuthUser, Role};
use crate::crypto;
use crate::db::{self, Hook, Template, Workflow};
use crate::error::{ApiError, AppResult};
use crate::jobs::{self, LaunchRequest};
use crate::runner;
use crate::workflow::{self, Graph};
use crate::Shared;

use super::{accepted, blank, created, gate, job_json, no_content, require_name, team_header};

#[derive(Deserialize)]
pub struct TemplateBody {
    name: String,
    #[serde(default)]
    description: String,
    tool: String,
    project_id: Option<String>,
    inventory_id: Option<String>,
    credential_id: Option<String>,
    environment_id: Option<String>,
    #[serde(default)]
    entrypoint: String,
    extra_vars: Option<Value>,
    spec: Option<Value>,
}

#[derive(Deserialize)]
pub struct LaunchBody {
    #[serde(default = "empty_object")]
    extra_vars: Value,
}

fn empty_object() -> Value {
    json!({})
}

#[derive(Deserialize)]
pub struct WorkflowBody {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    graph: Graph,
}

#[derive(Deserialize)]
pub struct HookBody {
    name: String,
    match_alertname: String,
    #[serde(default = "default_status")]
    match_status: String,
    template_id: Option<String>,
    workflow_id: Option<String>,
    extra_vars: Option<Value>,
}

#[derive(Deserialize)]
pub struct AfterQuery {
    after: Option<i64>,
}

fn default_status() -> String {
    "firing".into()
}

pub async fn list_templates(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.db()?;
    let items = db::list_templates(&db, &team)?;
    Ok(Json(json!(items
        .iter()
        .map(template_json)
        .collect::<Vec<_>>())))
}

pub async fn create_template(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Json(body): Json<TemplateBody>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let template = build_template(&state, &team, None, body)?;
    let db = state.db()?;
    db::insert_template(&db, &template)?;
    Ok(created(template_json(&template)))
}

pub async fn update_template(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<TemplateBody>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let template = build_template(&state, &team, Some(id), body)?;
    let db = state.db()?;
    db::update_template(&db, &template)?;
    Ok(Json(template_json(&template)))
}

pub async fn delete_template(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.db()?;
    db::delete_template(&db, &id, &team)?;
    Ok(no_content())
}

pub async fn launch_template(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<LaunchBody>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let extra = body.extra_vars;
    if !extra.is_object() {
        return Err(ApiError::bad("extra_vars muss ein Objekt sein."));
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
        },
    )?;
    let db = state.db()?;
    let job = db::job(&db, &job_id)?.ok_or(ApiError::NotFound)?;
    Ok(accepted(job_json(&job, None)))
}

pub async fn list_jobs(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.db()?;
    let jobs = db::list_jobs(&db, &team, 50)?;
    Ok(Json(json!(jobs
        .iter()
        .map(|job| job_json(job, None))
        .collect::<Vec<_>>())))
}

pub async fn get_job(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let db = state.db()?;
    let job = db::job(&db, &id)?.ok_or(ApiError::NotFound)?;
    crate::auth::authorize(&db, &user, &job.team_id, Role::Viewer)?;
    let events = db::events_after(&db, &id, 0, 2_000)?;
    Ok(Json(job_json(&job, Some(&events))))
}

pub async fn cancel_job(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let team = {
        let db = state.db()?;
        let job = db::job(&db, &id)?.ok_or(ApiError::NotFound)?;
        crate::auth::authorize(&db, &user, &job.team_id, Role::Operator)?;
        job.team_id
    };
    jobs::cancel(&state, &id)?;
    let _ = team;
    let db = state.db()?;
    let job = db::job(&db, &id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(job_json(&job, None)))
}

pub async fn job_events(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<AfterQuery>,
) -> AppResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    {
        let db = state.db()?;
        let job = db::job(&db, &id)?.ok_or(ApiError::NotFound)?;
        crate::auth::authorize(&db, &user, &job.team_id, Role::Viewer)?;
    }
    let after = query.after.unwrap_or(0);
    let stream = async_stream::stream! {
        let mut last = after;
        for _ in 0..8_000 {
            let snapshot = {
                let Ok(db) = state.db() else { break };
                let events = db::events_after(&db, &id, last, 200).unwrap_or_default();
                let status = db::job(&db, &id)
                    .ok()
                    .flatten()
                    .map(|job| job.status)
                    .unwrap_or_else(|| "failed".into());
                (events, status)
            };
            for event in snapshot.0 {
                last = event.id;
                let data = json!({
                    "id": event.id,
                    "ts": event.ts,
                    "stream": event.stream,
                    "line": event.line,
                });
                yield Ok(Event::default().event("log").id(event.id.to_string()).data(data.to_string()));
            }
            if matches!(snapshot.1.as_str(), "successful" | "failed" | "canceled") {
                yield Ok(Event::default().event("end").data(json!({"status": snapshot.1}).to_string()));
                break;
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

pub async fn list_workflows(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.db()?;
    let items = db::list_workflows(&db, &team)?;
    Ok(Json(json!(items
        .iter()
        .map(workflow_json)
        .collect::<Vec<_>>())))
}

pub async fn get_workflow(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let db = state.db()?;
    let item = db::workflow(&db, &id)?.ok_or(ApiError::NotFound)?;
    crate::auth::authorize(&db, &user, &item.team_id, Role::Viewer)?;
    Ok(Json(workflow_json(&item)))
}

pub async fn create_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Json(body): Json<WorkflowBody>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let item = build_workflow(&state, &team, None, body)?;
    let db = state.db()?;
    db::insert_workflow(&db, &item)?;
    Ok(created(workflow_json(&item)))
}

pub async fn update_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<WorkflowBody>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let item = build_workflow(&state, &team, Some(id), body)?;
    let db = state.db()?;
    db::update_workflow(&db, &item)?;
    Ok(Json(workflow_json(&item)))
}

pub async fn delete_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.db()?;
    db::delete_workflow(&db, &id, &team)?;
    Ok(no_content())
}

pub async fn launch_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<LaunchBody>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let extra = body.extra_vars;
    if !extra.is_object() {
        return Err(ApiError::bad("extra_vars muss ein Objekt sein."));
    }
    let run_id = jobs::launch_workflow(&state, &team, &id, extra, &user.username, "manual")?;
    let db = state.db()?;
    let run = db::workflow_job(&db, &run_id)?.ok_or(ApiError::NotFound)?;
    Ok(accepted(run_json(&state, &run)?))
}

pub async fn get_workflow_job(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let db = state.db()?;
    let run = db::workflow_job(&db, &id)?.ok_or(ApiError::NotFound)?;
    crate::auth::authorize(&db, &user, &run.team_id, Role::Viewer)?;
    drop(db);
    Ok(Json(run_json(&state, &run)?))
}

pub async fn list_hooks(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let reveal = can_operate(&state, &user, &team)?;
    let db = state.db()?;
    let hooks = db::list_hooks(&db, &team)?;
    Ok(Json(json!(hooks
        .iter()
        .map(|hook| hook_json(hook, reveal))
        .collect::<Vec<_>>())))
}

pub async fn create_hook(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Json(body): Json<HookBody>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let hook = build_hook(&state, &team, body)?;
    let db = state.db()?;
    db::insert_hook(&db, &hook)?;
    Ok(created(hook_json(&hook, true)))
}

pub async fn delete_hook(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.db()?;
    db::delete_hook(&db, &id, &team)?;
    Ok(no_content())
}

pub async fn list_deliveries(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.db()?;
    Ok(Json(json!(db::list_deliveries(&db, &team, 30)?)))
}

pub async fn test_hook(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let hook = {
        let db = state.db()?;
        let hook = db::hook(&db, &id)?.ok_or(ApiError::NotFound)?;
        crate::auth::authorize(&db, &user, &hook.team_id, Role::Operator)?;
        hook
    };
    let payload = json!({
        "status": "firing",
        "alerts": [{
            "status": "firing",
            "labels": {
                "alertname": hook.match_alertname,
                "instance": "web-1",
                "severity": "warning"
            },
            "annotations": {"summary": "Testauslösung aus der Koreograf-Oberfläche"}
        }]
    });
    let dispatch = jobs::accept_hook(&state, &hook, &payload)?;
    Ok(accepted(json!(dispatch_json(&dispatch))))
}

pub async fn ingest(
    State(state): State<Shared>,
    Path((id, token)): Path<(String, String)>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let hook = {
        let db = state.db()?;
        db::hook(&db, &id)?.ok_or(ApiError::NotFound)?
    };
    if !crypto::tokens_equal(&hook.token, &token) {
        return Err(ApiError::Unauthorized);
    }
    let dispatch = jobs::accept_hook(&state, &hook, &payload)?;
    Ok(Json(dispatch_json(&dispatch)))
}

fn dispatch_json(dispatch: &jobs::Dispatch) -> Value {
    json!({
        "status": dispatch.status,
        "summary": dispatch.summary,
        "job_id": dispatch.job_id,
        "workflow_job_id": dispatch.workflow_job_id,
    })
}

fn can_operate(state: &Shared, user: &AuthUser, team: &str) -> AppResult<bool> {
    if user.is_admin {
        return Ok(true);
    }
    let db = state.db()?;
    let Some(role) = db::role_of(&db, team, &user.id)? else {
        return Ok(false);
    };
    Ok(Role::parse(&role).is_some_and(|role| role >= Role::Operator))
}

fn build_template(
    state: &Shared,
    team: &str,
    id: Option<String>,
    body: TemplateBody,
) -> AppResult<Template> {
    if !runner::known_tool(&body.tool) {
        return Err(ApiError::bad("Werkzeug ist ansible, opentofu oder shell."));
    }
    runner::check_relative(body.entrypoint.trim()).map_err(ApiError::bad)?;
    let extra = body.extra_vars.unwrap_or_else(|| json!({}));
    let spec = body.spec.unwrap_or_else(|| json!({}));
    if !extra.is_object() || !spec.is_object() {
        return Err(ApiError::bad(
            "extra_vars und spec müssen JSON-Objekte sein.",
        ));
    }
    if body.tool == "ansible" {
        runner::check_ansible_spec(&spec).map_err(ApiError::bad)?;
    }
    if let Some(action) = spec.get("action").and_then(Value::as_str) {
        if !matches!(action, "plan" | "apply") {
            return Err(ApiError::bad("Aktion ist plan oder apply."));
        }
    }
    let entrypoint = body.entrypoint.trim().to_string();
    let inline = spec.get("script").and_then(Value::as_str).unwrap_or("");
    if body.tool == "ansible" && entrypoint.is_empty() {
        return Err(ApiError::bad("Ansible braucht ein Playbook als Einstieg."));
    }
    if body.tool == "opentofu" && entrypoint.is_empty() {
        return Err(ApiError::bad("OpenTofu braucht ein Modulverzeichnis."));
    }
    if body.tool == "shell" && entrypoint.is_empty() && inline.is_empty() {
        return Err(ApiError::bad("Shell braucht einen Pfad oder spec.script."));
    }
    let project_id = blank(body.project_id);
    let inventory_id = blank(body.inventory_id);
    let credential_id = blank(body.credential_id);
    let environment_id = blank(body.environment_id);
    let db = state.db()?;
    if let Some(project_id) = &project_id {
        db::project(&db, project_id)?.ok_or_else(|| ApiError::bad("Projekt fehlt."))?;
        if !db::project_visible(&db, project_id, team)? {
            return Err(ApiError::bad(
                "Das Projekt ist für dieses Team nicht sichtbar.",
            ));
        }
    }
    if let Some(inventory_id) = &inventory_id {
        let inventory =
            db::inventory(&db, inventory_id)?.ok_or_else(|| ApiError::bad("Inventar fehlt."))?;
        if inventory.team_id != team {
            return Err(ApiError::bad("Inventar gehört zu einem anderen Team."));
        }
    }
    if let Some(credential_id) = &credential_id {
        let credential =
            db::credential(&db, credential_id)?.ok_or_else(|| ApiError::bad("Zugang fehlt."))?;
        if credential.team_id != team {
            return Err(ApiError::bad("Zugang gehört zu einem anderen Team."));
        }
    }
    if let Some(environment_id) = &environment_id {
        let environment = db::environment(&db, environment_id)?
            .ok_or_else(|| ApiError::bad("Execution Environment fehlt."))?;
        if environment
            .team_id
            .as_ref()
            .is_some_and(|owner| owner != team)
        {
            return Err(ApiError::bad(
                "Execution Environment gehört zu einem anderen Team.",
            ));
        }
    }
    let created_at = if let Some(id) = &id {
        db::template(&db, id)?
            .map(|item| item.created_at)
            .unwrap_or_else(db::now)
    } else {
        db::now()
    };
    Ok(Template {
        id: id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        team_id: team.to_string(),
        name: require_name(&body.name)?,
        description: body.description.trim().to_string(),
        tool: body.tool,
        project_id,
        inventory_id,
        credential_id,
        environment_id,
        entrypoint,
        extra_vars: extra.to_string(),
        spec: spec.to_string(),
        created_at,
    })
}

fn build_workflow(
    state: &Shared,
    team: &str,
    id: Option<String>,
    body: WorkflowBody,
) -> AppResult<Workflow> {
    workflow::validate(&body.graph).map_err(ApiError::bad)?;
    let db = state.db()?;
    for node in &body.graph.nodes {
        let template = db::template(&db, &node.template_id)?
            .ok_or_else(|| ApiError::bad(format!("Vorlage für „{}“ fehlt.", node.name)))?;
        if template.team_id != team {
            return Err(ApiError::bad("Eine Vorlage gehört zu einem anderen Team."));
        }
    }
    let created_at = if let Some(id) = &id {
        db::workflow(&db, id)?
            .map(|item| item.created_at)
            .unwrap_or_else(db::now)
    } else {
        db::now()
    };
    Ok(Workflow {
        id: id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        team_id: team.to_string(),
        name: require_name(&body.name)?,
        description: body.description.trim().to_string(),
        graph: serde_json::to_string(&body.graph)
            .map_err(|err| ApiError::Internal(err.to_string()))?,
        created_at,
    })
}

fn build_hook(state: &Shared, team: &str, body: HookBody) -> AppResult<Hook> {
    let template_id = blank(body.template_id);
    let workflow_id = blank(body.workflow_id);
    match (&template_id, &workflow_id) {
        (Some(_), Some(_)) => {
            return Err(ApiError::bad(
                "Hook hat entweder eine Vorlage oder einen Workflow.",
            ))
        }
        (None, None) => {
            return Err(ApiError::bad(
                "Hook braucht eine Vorlage oder einen Workflow.",
            ))
        }
        _ => {}
    }
    if !matches!(body.match_status.as_str(), "firing" | "resolved" | "any") {
        return Err(ApiError::bad("Status ist firing, resolved oder any."));
    }
    let alertname = body.match_alertname.trim();
    if alertname.is_empty() {
        return Err(ApiError::bad("Alertname fehlt."));
    }
    let extra = body.extra_vars.unwrap_or_else(|| json!({}));
    if !extra.is_object() {
        return Err(ApiError::bad("extra_vars muss ein Objekt sein."));
    }
    let db = state.db()?;
    if let Some(template_id) = &template_id {
        let template =
            db::template(&db, template_id)?.ok_or_else(|| ApiError::bad("Vorlage fehlt."))?;
        if template.team_id != team {
            return Err(ApiError::bad("Vorlage gehört zu einem anderen Team."));
        }
    }
    if let Some(workflow_id) = &workflow_id {
        let item =
            db::workflow(&db, workflow_id)?.ok_or_else(|| ApiError::bad("Workflow fehlt."))?;
        if item.team_id != team {
            return Err(ApiError::bad("Workflow gehört zu einem anderen Team."));
        }
    }
    Ok(Hook {
        id: Uuid::new_v4().to_string(),
        team_id: team.to_string(),
        name: require_name(&body.name)?,
        token: crypto::random_token(),
        match_alertname: alertname.to_string(),
        match_status: body.match_status,
        template_id,
        workflow_id,
        extra_vars: extra.to_string(),
        enabled: true,
        created_at: db::now(),
    })
}

fn template_json(template: &Template) -> Value {
    json!({
        "id": template.id,
        "team_id": template.team_id,
        "name": template.name,
        "description": template.description,
        "tool": template.tool,
        "project_id": template.project_id,
        "inventory_id": template.inventory_id,
        "credential_id": template.credential_id,
        "environment_id": template.environment_id,
        "entrypoint": template.entrypoint,
        "extra_vars": db::json_object(&template.extra_vars),
        "spec": db::json_object(&template.spec),
        "created_at": template.created_at,
    })
}

fn workflow_json(item: &Workflow) -> Value {
    json!({
        "id": item.id,
        "team_id": item.team_id,
        "name": item.name,
        "description": item.description,
        "graph": db::json_object(&item.graph),
        "created_at": item.created_at,
    })
}

fn hook_json(hook: &Hook, reveal_token: bool) -> Value {
    json!({
        "id": hook.id,
        "team_id": hook.team_id,
        "name": hook.name,
        "token": if reveal_token { hook.token.clone() } else { String::new() },
        "match_alertname": hook.match_alertname,
        "match_status": hook.match_status,
        "template_id": hook.template_id,
        "workflow_id": hook.workflow_id,
        "extra_vars": db::json_object(&hook.extra_vars),
        "enabled": hook.enabled,
        "created_at": hook.created_at,
    })
}

fn run_json(state: &Shared, run: &db::WorkflowJob) -> AppResult<Value> {
    let db = state.db()?;
    let graph = run
        .workflow_id
        .as_deref()
        .and_then(|id| db::workflow(&db, id).ok().flatten())
        .map(|item| db::json_object(&item.graph))
        .unwrap_or_else(|| json!({"nodes": [], "edges": []}));
    let nodes = db::workflow_nodes(&db, &run.id)?;
    Ok(json!({
        "id": run.id,
        "workflow_id": run.workflow_id,
        "team_id": run.team_id,
        "name": run.name,
        "status": run.status,
        "extra_vars": db::json_object(&run.extra_vars),
        "launched_by": run.launched_by,
        "trigger": run.trigger,
        "created_at": run.created_at,
        "finished_at": run.finished_at,
        "graph": graph,
        "nodes": nodes.iter().map(|node| json!({
            "node_id": node.node_id,
            "phase": node.phase,
            "job_id": node.job_id,
        })).collect::<Vec<_>>(),
    }))
}
