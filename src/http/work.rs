use std::collections::HashMap;
use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::stream::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{AuthUser, Role};
use crate::credentials;
use crate::crypto;
use crate::db::{self, Hook, Template, Workflow};
use crate::error::{ApiError, ApiJson, ApiJsonValue, AppResult};
use crate::jobs::{self, LaunchRequest};
use crate::runner;
use crate::workflow::{self, Graph};
use crate::Shared;

use super::{
    accepted, blank, created, gate, job_json, no_content, require_description, require_name,
    team_header,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RelaunchBody {
    #[serde(default)]
    only_failed_hosts: bool,
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
pub struct LaunchBody {
    #[serde(default = "empty_object")]
    extra_vars: Value,
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

fn empty_object() -> Value {
    json!({})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowBody {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    graph: Graph,
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

#[derive(Deserialize)]
pub struct AfterQuery {
    after: Option<i64>,
}

#[derive(Deserialize, Default)]
pub struct ListJobsQuery {
    limit: Option<String>,
    offset: Option<String>,
    q: Option<String>,
    status: Option<String>,
    tool: Option<String>,
    trigger: Option<String>,
    template_id: Option<String>,
    since: Option<String>,
    until: Option<String>,
}

fn filter_value(value: Option<String>) -> String {
    value
        .map(|item| item.trim().to_string())
        .unwrap_or_default()
}

fn query_number(value: Option<String>, fallback: i64, field: &str) -> AppResult<i64> {
    match value.map(|item| item.trim().to_string()) {
        Some(item) if !item.is_empty() => item
            .parse::<i64>()
            .map_err(|_| ApiError::bad_vars("invalid_query_parameter", [("field", field)])),
        _ => Ok(fallback),
    }
}

fn query_date(value: Option<String>, field: &str) -> AppResult<String> {
    let value = filter_value(value);
    if value.is_empty() {
        return Ok(value);
    }
    let parsed = chrono::DateTime::parse_from_rfc3339(&value)
        .map_err(|_| ApiError::bad_vars("invalid_query_parameter", [("field", field)]))?;
    Ok(parsed
        .with_timezone(&chrono::Utc)
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string())
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
    Ok(Json(json!(items
        .iter()
        .map(|item| {
            let ids = by_template.get(&item.id).map(Vec::as_slice).unwrap_or(&[]);
            template_json(item, ids)
        })
        .collect::<Vec<_>>())))
}

pub async fn create_template(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<TemplateBody>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let (mut template, credential_ids) = build_template(&state, &team, None, body)?;
    template.admin_saved = user.is_admin;
    let db = state.store();
    db.insert_template(&template, &credential_ids)?;
    Ok(created(template_json(&template, &credential_ids)))
}

pub async fn update_template(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<TemplateBody>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let (mut template, credential_ids) = build_template(&state, &team, Some(id), body)?;
    template.admin_saved = user.is_admin;
    let db = state.store();
    db.update_template(&template, &credential_ids)?;
    Ok(Json(template_json(&template, &credential_ids)))
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
) -> AppResult<Response> {
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
    Ok(accepted(job_json(&job, None)))
}

pub async fn list_jobs(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Query(query): Query<ListJobsQuery>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let limit = query_number(query.limit, 25, "limit")?.clamp(1, 100);
    let offset = query_number(query.offset, 0, "offset")?.max(0);
    let filter = db::JobQuery {
        limit,
        offset,
        q: filter_value(query.q),
        status: filter_value(query.status),
        tool: filter_value(query.tool),
        trigger: filter_value(query.trigger),
        template_id: filter_value(query.template_id),
        since: query_date(query.since, "since")?,
        until: query_date(query.until, "until")?,
    };
    let db = state.store();
    let (jobs, total) = db.list_jobs(&team, &filter)?;
    Ok(Json(json!({
        "items": jobs.iter().map(|job| job_json(job, None)).collect::<Vec<_>>(),
        "total": total,
    })))
}

pub async fn get_job(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let db = state.store();
    let job = db.job(&id)?.ok_or(ApiError::NotFound)?;
    crate::auth::authorize(db, &user, &job.team_id, Role::Viewer)?;
    let events = events_for_page(db, &id)?;
    let mut value = job_json(&job, Some(&events));
    value["output_capped"] = json!(db.events_bytes(&id)? > OUTPUT_LIMIT as i64);
    Ok(Json(value))
}

const PAGE_EVENTS: i64 = 2_000;
const PAGE_TAIL: i64 = 500;

fn events_for_page(db: &dyn crate::backend::Backend, job_id: &str) -> AppResult<Vec<db::JobEvent>> {
    let head = db.events_after(job_id, 0, PAGE_EVENTS)?;
    if (head.len() as i64) < PAGE_EVENTS {
        return Ok(head);
    }
    let tail = db.events_last(job_id, PAGE_TAIL)?;
    let keep = (PAGE_EVENTS - PAGE_TAIL) as usize;
    let next_id = head.get(keep).map(|event| event.id);
    let mut shown: Vec<db::JobEvent> = head.into_iter().take(keep).collect();
    let last_id = shown.last().map(|event| event.id).unwrap_or(0);
    let tail: Vec<db::JobEvent> = tail
        .into_iter()
        .filter(|event| event.id > last_id)
        .collect();
    let contiguous = match (next_id, tail.first()) {
        (_, None) => true,
        (Some(next), Some(first)) => first.id == next,
        (None, Some(_)) => false,
    };
    if !contiguous {
        let omitted = match tail.first() {
            Some(first) => db.events_count_between(job_id, last_id, first.id)?,
            None => 0,
        };
        shown.push(db::JobEvent {
            id: 0,
            ts: String::new(),
            stream: "gap".into(),
            line: omitted.to_string(),
        });
    }
    shown.extend(tail);
    Ok(shown)
}

const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

fn output_filename(name: &str, id: &str) -> String {
    let mut slug = String::new();
    for ch in name.chars().take(48) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let base = if slug.is_empty() { "job" } else { slug };
    let short: String = id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(8)
        .collect();
    if short.is_empty() {
        format!("{base}.txt")
    } else {
        format!("{base}-{short}.txt")
    }
}

fn render_job_output(db: &dyn crate::backend::Backend, job_id: &str) -> AppResult<String> {
    let mut out = String::new();
    let mut after = 0_i64;
    loop {
        let batch = db.events_after(job_id, after, 500)?;
        if batch.is_empty() {
            break;
        }
        for event in &batch {
            after = event.id;
            let extra = event.line.len() + 1;
            if out.len().saturating_add(extra) > OUTPUT_LIMIT {
                out.push_str("… output truncated.\n");
                return Ok(out);
            }
            out.push_str(&event.line);
            if !event.line.ends_with('\n') {
                out.push('\n');
            }
        }
        if batch.len() < 500 {
            break;
        }
    }
    Ok(out)
}

pub async fn job_output(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let (body, filename) = {
        let db = state.store();
        let job = db.job(&id)?.ok_or(ApiError::NotFound)?;
        crate::auth::authorize(db, &user, &job.team_id, Role::Viewer)?;
        let body = render_job_output(db, &id)?;
        (body, output_filename(&job.name, &job.id))
    };
    let disposition = format!("attachment; filename=\"{filename}\"");
    let disposition = HeaderValue::from_str(&disposition)
        .unwrap_or_else(|_| HeaderValue::from_static("attachment; filename=\"job.txt\""));
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            ),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        body,
    )
        .into_response())
}

pub async fn cancel_job(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let team = {
        let db = state.store();
        let job = db.job(&id)?.ok_or(ApiError::NotFound)?;
        crate::auth::authorize(db, &user, &job.team_id, Role::Operator)?;
        job.team_id
    };
    jobs::cancel(&state, &id)?;
    let _ = team;
    let db = state.store();
    let job = db.job(&id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(job_json(&job, None)))
}

pub async fn relaunch_job(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<RelaunchBody>,
) -> AppResult<Response> {
    {
        let db = state.store();
        let job = db.job(&id)?.ok_or(ApiError::NotFound)?;
        crate::auth::authorize(db, &user, &job.team_id, Role::Operator)?;
    }
    let job_id = jobs::relaunch(&state, &id, &user.username, body.only_failed_hosts)?;
    let db = state.store();
    let job = db.job(&job_id)?.ok_or(ApiError::NotFound)?;
    Ok(accepted(job_json(&job, None)))
}

pub async fn job_events(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<AfterQuery>,
) -> AppResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    {
        let db = state.store();
        let job = db.job(&id)?.ok_or(ApiError::NotFound)?;
        crate::auth::authorize(db, &user, &job.team_id, Role::Viewer)?;
    }
    let after = query.after.unwrap_or(0);
    let stream = async_stream::stream! {
        let mut last = after;
        let mut waits = 0u32;
        loop {
            let snapshot = {
                let db = state.store();
                let events = db.events_after(&id, last, 200).unwrap_or_default();
                let (status, finished_at) = db
                    .job(&id)
                    .ok()
                    .flatten()
                    .map(|job| (job.status, job.finished_at))
                    .unwrap_or_else(|| ("failed".into(), None));
                (events, status, finished_at)
            };
            let count = snapshot.0.len();
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
            let terminal = matches!(snapshot.1.as_str(), "successful" | "failed" | "canceled");
            if terminal && count < 200 {
                yield Ok(Event::default().event("end").data(json!({"status": snapshot.1, "finished_at": snapshot.2}).to_string()));
                break;
            }
            if count == 200 {
                continue;
            }
            waits += 1;
            if waits >= 8_000 {
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
    let db = state.store();
    let items = db.list_workflows(&team)?;
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
    let db = state.store();
    let item = db.workflow(&id)?.ok_or(ApiError::NotFound)?;
    crate::auth::authorize(db, &user, &item.team_id, Role::Viewer)?;
    Ok(Json(workflow_json(&item)))
}

pub async fn create_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<WorkflowBody>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let mut item = build_workflow(&state, &team, None, body)?;
    item.admin_saved = user.is_admin;
    let db = state.store();
    db.insert_workflow(&item)?;
    Ok(created(workflow_json(&item)))
}

pub async fn update_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<WorkflowBody>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let mut item = build_workflow(&state, &team, Some(id), body)?;
    item.admin_saved = user.is_admin;
    let db = state.store();
    db.update_workflow(&item)?;
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
    let db = state.store();
    db.delete_workflow(&id, &team)?;
    Ok(no_content())
}

pub async fn launch_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<LaunchBody>,
) -> AppResult<Response> {
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
    Ok(accepted(run_json(db, &run)?))
}

pub async fn get_workflow_job(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let db = state.store();
    let run = db.workflow_job(&id)?.ok_or(ApiError::NotFound)?;
    crate::auth::authorize(db, &user, &run.team_id, Role::Viewer)?;
    Ok(Json(run_json(db, &run)?))
}

pub async fn list_hooks(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let reveal = can_operate(&state, &user, &team)?;
    let db = state.store();
    let hooks = db.list_hooks(&team)?;
    Ok(Json(json!(hooks
        .iter()
        .map(|hook| hook_json(hook, reveal))
        .collect::<Vec<_>>())))
}

pub async fn create_hook(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<HookBody>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let hook = build_hook(&state, &team, None, body)?;
    let db = state.store();
    db.insert_hook(&hook)?;
    Ok(created(hook_json(&hook, true)))
}

pub async fn update_hook(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<HookBody>,
) -> AppResult<Json<Value>> {
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
    Ok(Json(hook_json(&hook, true)))
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
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    Ok(Json(json!(db.list_deliveries(&team, 30)?)))
}

pub async fn test_hook(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let hook = {
        let db = state.store();
        let hook = db.hook(&id)?.ok_or(ApiError::NotFound)?;
        crate::auth::authorize(db, &user, &hook.team_id, Role::Operator)?;
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
            "annotations": {"summary": "Test delivery from the Koreograf UI"}
        }]
    });
    let dispatch = jobs::accept_hook(&state, &hook, &payload, Some(&user.username))?;
    Ok(accepted(json!(dispatch_json(&dispatch))))
}

pub async fn ingest(
    State(state): State<Shared>,
    Path((id, token)): Path<(String, String)>,
    ApiJsonValue(payload): ApiJsonValue,
) -> AppResult<Json<Value>> {
    let hook = {
        let db = state.store();
        db.hook(&id)?.ok_or(ApiError::NotFound)?
    };
    if !crypto::tokens_equal(&hook.token, &token) {
        return Err(ApiError::Unauthorized);
    }
    let dispatch = jobs::accept_hook(&state, &hook, &payload, None)?;
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
    runner::check_relative(body.entrypoint.trim()).map_err(ApiError::bad)?;
    let extra = body.extra_vars.unwrap_or_else(|| json!({}));
    let spec = body.spec.unwrap_or_else(|| json!({}));
    if !extra.is_object() || !spec.is_object() {
        return Err(ApiError::bad("template.vars_not_objects"));
    }
    if body.tool == "ansible" {
        runner::check_ansible_spec(&spec).map_err(ApiError::bad)?;
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
            let inventory = db
                .inventory(inventory_id)?
                .ok_or_else(|| ApiError::bad("inventory_is_missing"))?;
            if inventory.team_id != team {
                return Err(ApiError::bad("the_inventory_belongs_to_another_team"));
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
            let credential = db
                .credential(credential_id)?
                .ok_or_else(|| ApiError::bad("credential_is_missing"))?;
            loaded.push(credential);
        }
        let previous_ids = match &id {
            Some(id) => db.template_credential_ids(id)?,
            None => Vec::new(),
        };
        (created_at, loaded, previous_ids)
    };
    let chosen = credentials::assign_loaded(state, team, &loaded).map_err(ApiError::bad)?;
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

fn check_node_overrides(
    store: &dyn crate::backend::Backend,
    team: &str,
    template: &Template,
    node: &workflow::GraphNode,
) -> AppResult<()> {
    let over = &node.overrides;
    if let Some(id) = over.inventory_id.as_deref().filter(|id| !id.is_empty()) {
        let inventory = store
            .inventory(id)?
            .ok_or_else(|| ApiError::bad("inventory_is_missing"))?;
        if inventory.team_id != team {
            return Err(ApiError::bad("the_inventory_belongs_to_another_team"));
        }
    }
    if let Some(id) = over.credential_id.as_deref().filter(|id| !id.is_empty()) {
        let credential = store
            .credential(id)?
            .ok_or_else(|| ApiError::bad("credential_is_missing"))?;
        if credential.team_id != team {
            return Err(ApiError::bad("the_credential_belongs_to_another_team"));
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
        workflow::apply_overrides(&mut params, over);
        let spec = params.get("spec").cloned().unwrap_or_else(|| json!({}));
        if template.tool == "ansible" {
            runner::check_ansible_spec(&spec).map_err(ApiError::bad)?;
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
    workflow::validate(&body.graph).map_err(ApiError::bad)?;
    let db = state.store();
    for node in &body.graph.nodes {
        let template = db.template(&node.template_id)?.ok_or_else(|| {
            ApiError::bad_vars("template.missing_named", [("name", node.name.as_str())])
        })?;
        if template.team_id != team {
            return Err(ApiError::bad("a_template_belongs_to_another_team"));
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
    })
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
    let extra = body.extra_vars.unwrap_or_else(|| json!({}));
    if !extra.is_object() {
        return Err(ApiError::bad("extra_vars_must_be_an_object"));
    }
    let db = state.store();
    if let Some(template_id) = &template_id {
        let template = db
            .template(template_id)?
            .ok_or_else(|| ApiError::bad("template_is_missing"))?;
        if template.team_id != team {
            return Err(ApiError::bad("the_template_belongs_to_another_team"));
        }
    }
    if let Some(workflow_id) = &workflow_id {
        let owner = db
            .workflow_team(workflow_id)?
            .ok_or_else(|| ApiError::bad("workflow_is_missing"))?;
        if owner != team {
            return Err(ApiError::bad("the_workflow_belongs_to_another_team"));
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

fn template_json(template: &Template, credential_ids: &[String]) -> Value {
    let credential_ids = if credential_ids.is_empty() {
        template
            .credential_id
            .clone()
            .into_iter()
            .collect::<Vec<_>>()
    } else {
        credential_ids.to_vec()
    };
    json!({
        "id": template.id,
        "team_id": template.team_id,
        "name": template.name,
        "description": template.description,
        "tool": template.tool,
        "project_id": template.project_id,
        "inventory_id": template.inventory_id,
        "credential_id": template.credential_id,
        "credential_ids": credential_ids,
        "environment_id": template.environment_id,
        "no_environment": template.no_environment,
        "entrypoint": template.entrypoint,
        "extra_vars": db::json_object(&template.extra_vars),
        "spec": db::json_object(&template.spec),
        "created_at": template.created_at,
        "update_on_launch": template.update_on_launch,
        "prompt_on_launch": template.prompt_on_launch,
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

fn run_json(db: &dyn crate::backend::Backend, run: &db::WorkflowJob) -> AppResult<Value> {
    let graph = run
        .workflow_id
        .as_deref()
        .and_then(|id| db.workflow(id).ok().flatten())
        .map(|item| db::json_object(&item.graph))
        .unwrap_or_else(|| json!({"nodes": [], "edges": []}));
    let nodes = db.workflow_nodes(&run.id)?;
    let mut node_rows = Vec::with_capacity(nodes.len());
    for node in &nodes {
        let job = match &node.job_id {
            Some(id) => db.job(id)?,
            None => None,
        };
        node_rows.push(json!({
            "node_id": node.node_id,
            "phase": node.phase,
            "job_id": node.job_id,
            "job_name": job.as_ref().map(|item| &item.name),
            "tool": job.as_ref().map(|item| &item.tool),
            "started_at": job.as_ref().and_then(|item| item.started_at.as_deref()),
            "finished_at": job.as_ref().and_then(|item| item.finished_at.as_deref()),
            "job_created_at": job.as_ref().map(|item| &item.created_at),
        }));
    }
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
        "nodes": node_rows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page_state(tag: &str) -> crate::Shared {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-page-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        crate::boot(crate::Config {
            bind: "127.0.0.1:0".into(),
            data_dir: dir,
            web_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
            examples_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
            admin_user: "admin".into(),
            admin_password: Some("test-admin-pass".into()),
            runner: crate::config::RunnerMode::Local,
            container_image: false,
            job_timeout_secs: 60,
            job_output_lines: 8_000,
            local_jobs: crate::config::LocalJobs::All,
            project_roots: Vec::new(),
            hsts: false,
            cookie_secure: crate::config::CookieSecure::Auto,
            trusted_proxy: false,
            mcp: true,
            public_url: Vec::new(),
            data_host: None,
            ldap: None,
        })
        .expect("boot")
    }

    #[test]
    fn job_page_keeps_the_start_and_the_end() {
        let state = page_state("head-tail");
        {
            let db = state.db().unwrap();
            for i in 1..=2_500 {
                db::insert_event(&db, "job", "stdout", &format!("line-{i}")).unwrap();
            }
        }
        let events = events_for_page(state.store(), "job").unwrap();
        let gaps: Vec<_> = events
            .iter()
            .filter(|event| event.stream == "gap")
            .collect();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].line, "500");
        let lines: Vec<_> = events.iter().map(|event| event.line.as_str()).collect();
        assert_eq!(lines[0], "line-1");
        assert_eq!(lines[1_499], "line-1500");
        assert_eq!(*lines.last().unwrap(), "line-2500");
        assert!(!lines.contains(&"line-1600"));
        assert!(lines.contains(&"line-2001"));
        let _ = std::fs::remove_dir_all(&state.cfg.data_dir);
    }

    #[test]
    fn job_page_counts_only_this_jobs_omitted_events() {
        let state = page_state("gap-count");
        {
            let db = state.db().unwrap();
            for i in 1..=2_500 {
                db::insert_event(&db, "job", "stdout", &format!("line-{i}")).unwrap();
                db::insert_event(&db, "other", "stdout", &format!("other-{i}")).unwrap();
            }
        }
        let events = events_for_page(state.store(), "job").unwrap();
        let gaps: Vec<_> = events
            .iter()
            .filter(|event| event.stream == "gap")
            .collect();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].line, "500");
        assert_eq!(
            state
                .store()
                .events_count_between("job", 0, i64::MAX)
                .unwrap(),
            2_500
        );
        assert_eq!(
            state
                .store()
                .events_count_between("other", 0, i64::MAX)
                .unwrap(),
            2_500
        );
        assert!(events.iter().all(|event| !event.line.starts_with("other-")));
        let _ = std::fs::remove_dir_all(&state.cfg.data_dir);
    }
}
