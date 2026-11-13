use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{self, Job, JobEvent};
use crate::error::{ApiError, ApiJson, AppResult};
use crate::features::identity::{AuthUser, Role};
use crate::http::{gate, team_header};
use crate::runner;
use crate::Shared;

use super::{cancel, relaunch};

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/jobs", get(list_jobs))
        .route("/jobs/{id}", get(get_job))
        .route("/jobs/{id}/cancel", post(cancel_job))
        .route("/jobs/{id}/relaunch", post(relaunch_job))
        .route("/jobs/{id}/output", get(job_output))
        .route("/jobs/{id}/events", get(job_events))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RelaunchBody {
    #[serde(default)]
    only_failed_hosts: bool,
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

#[derive(Serialize)]
pub(super) struct JobEventResponse {
    id: i64,
    ts: String,
    stream: String,
    line: String,
}

impl JobEventResponse {
    fn new(event: &JobEvent) -> Self {
        Self {
            id: event.id,
            ts: event.ts.clone(),
            stream: event.stream.clone(),
            line: event.line.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct JobResponse {
    id: String,
    team_id: String,
    template_id: Option<String>,
    workflow_job_id: Option<String>,
    workflow_node_id: Option<String>,
    name: String,
    tool: String,
    status: String,
    extra_vars: Value,
    launched_by: String,
    trigger: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    created_at: String,
    parameters: Value,
    failed_hosts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    events: Option<Vec<JobEventResponse>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_capped: Option<bool>,
}

impl JobResponse {
    pub(crate) fn new(job: &Job, events: Option<&[JobEvent]>) -> Self {
        Self {
            id: job.id.clone(),
            team_id: job.team_id.clone(),
            template_id: job.template_id.clone(),
            workflow_job_id: job.workflow_job_id.clone(),
            workflow_node_id: job.workflow_node_id.clone(),
            name: job.name.clone(),
            tool: job.tool.clone(),
            status: job.status.clone(),
            extra_vars: db::json_object(&job.extra_vars),
            launched_by: job.launched_by.clone(),
            trigger: job.trigger.clone(),
            started_at: job.started_at.clone(),
            finished_at: job.finished_at.clone(),
            created_at: job.created_at.clone(),
            parameters: db::json_object(&job.parameters),
            failed_hosts: runner::failed_host_names(&job.failed_hosts),
            events: events.map(|events| events.iter().map(JobEventResponse::new).collect()),
            output_capped: None,
        }
    }

    fn with_output_capped(mut self, capped: bool) -> Self {
        self.output_capped = Some(capped);
        self
    }
}

#[derive(Serialize)]
pub(super) struct JobListResponse {
    items: Vec<JobResponse>,
    total: i64,
}

#[derive(Serialize)]
struct JobEndResponse {
    status: String,
    finished_at: Option<String>,
}

pub async fn list_jobs(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Query(query): Query<ListJobsQuery>,
) -> AppResult<Json<JobListResponse>> {
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
    Ok(Json(JobListResponse {
        items: jobs.iter().map(|job| JobResponse::new(job, None)).collect(),
        total,
    }))
}

pub async fn get_job(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<JobResponse>> {
    let db = state.store();
    let job = db.job(&id)?.ok_or(ApiError::NotFound)?;
    crate::features::identity::authorize(db, &user, &job.team_id, Role::Viewer)?;
    let events = events_for_page(db, &id)?;
    let capped = db.events_bytes(&id)? > OUTPUT_LIMIT as i64;
    Ok(Json(
        JobResponse::new(&job, Some(&events)).with_output_capped(capped),
    ))
}

const PAGE_EVENTS: i64 = 2_000;

const PAGE_TAIL: i64 = 500;

pub(crate) fn events_for_page(
    db: &dyn crate::backend::Backend,
    job_id: &str,
) -> AppResult<Vec<db::JobEvent>> {
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
        crate::features::identity::authorize(db, &user, &job.team_id, Role::Viewer)?;
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
) -> AppResult<Json<JobResponse>> {
    let team = {
        let db = state.store();
        let job = db.job(&id)?.ok_or(ApiError::NotFound)?;
        crate::features::identity::authorize(db, &user, &job.team_id, Role::Operator)?;
        job.team_id
    };
    cancel(&state, &id)?;
    let _ = team;
    let db = state.store();
    let job = db.job(&id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(JobResponse::new(&job, None)))
}

pub async fn relaunch_job(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<RelaunchBody>,
) -> AppResult<(StatusCode, Json<JobResponse>)> {
    {
        let db = state.store();
        let job = db.job(&id)?.ok_or(ApiError::NotFound)?;
        crate::features::identity::authorize(db, &user, &job.team_id, Role::Operator)?;
    }
    let job_id = relaunch(&state, &id, &user.username, body.only_failed_hosts)?;
    let db = state.store();
    let job = db.job(&job_id)?.ok_or(ApiError::NotFound)?;
    Ok((StatusCode::ACCEPTED, Json(JobResponse::new(&job, None))))
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
        crate::features::identity::authorize(db, &user, &job.team_id, Role::Viewer)?;
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
                let data = JobEventResponse::new(&event);
                yield Ok(Event::default().event("log").id(event.id.to_string()).data(serde_json::to_string(&data).unwrap_or_default()));
            }
            let terminal = matches!(snapshot.1.as_str(), "successful" | "failed" | "canceled");
            if terminal && count < 200 {
                let data = JobEndResponse {
                    status: snapshot.1,
                    finished_at: snapshot.2,
                };
                yield Ok(Event::default().event("end").data(serde_json::to_string(&data).unwrap_or_default()));
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
