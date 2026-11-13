use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{Timelike, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::backend::Backend;
use crate::db::{self, Schedule};
use crate::error::{ApiError, ApiJson, AppResult};
use crate::features::identity::{AuthUser, Role};
use crate::http::{gate, no_content, require_name, team_header};
use crate::Shared;

use super::model::{End, Frequency, Input, Rule};
use super::service;

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route(
            "/templates/{id}/schedules",
            get(list_for_template).post(create_for_template),
        )
        .route(
            "/workflows/{id}/schedules",
            get(list_for_workflow).post(create_for_workflow),
        )
        .route("/schedules", get(list_for_team))
        .route("/schedules/preview", post(preview))
        .route("/schedules/{id}", put(update).delete(remove))
        .route("/schedules/{id}/enabled", post(set_enabled))
        .route("/schedules/{id}/missed", post(answer_missed))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleBody {
    name: String,
    timezone: String,
    start_date: String,
    start_time: String,
    frequency: String,
    #[serde(default)]
    weekdays: Vec<String>,
    #[serde(default = "never")]
    end_kind: String,
    #[serde(default)]
    end_count: Option<u32>,
    #[serde(default)]
    end_date: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewBody {
    timezone: String,
    start_date: String,
    start_time: String,
    frequency: String,
    #[serde(default)]
    weekdays: Vec<String>,
    #[serde(default = "never")]
    end_kind: String,
    #[serde(default)]
    end_count: Option<u32>,
    #[serde(default)]
    end_date: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnabledBody {
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissedBody {
    accept: bool,
}

#[derive(Serialize)]
pub(super) struct ScheduleResponse {
    id: String,
    name: String,
    target_kind: String,
    target_id: String,
    timezone: String,
    start_date: String,
    start_time: String,
    frequency: String,
    weekdays: Vec<String>,
    end_kind: String,
    end_count: Option<i64>,
    end_date: Option<String>,
    enabled: bool,
    next_run_at: Option<String>,
    missed_at: Option<String>,
    created_by: String,
    updated_by: String,
    launch_block: Option<String>,
}

#[derive(Serialize)]
pub(super) struct TeamScheduleResponse {
    #[serde(flatten)]
    schedule: ScheduleResponse,
    target_name: String,
}

#[derive(Serialize)]
pub(super) struct PreviewResponse {
    occurrences: Vec<String>,
}

impl From<&Schedule> for ScheduleResponse {
    fn from(item: &Schedule) -> Self {
        Self {
            id: item.id.clone(),
            name: item.name.clone(),
            target_kind: item.target_kind.clone(),
            target_id: item.target_id.clone(),
            timezone: item.timezone.clone(),
            start_date: item.start_date.clone(),
            start_time: item.start_time.clone(),
            frequency: item.frequency.clone(),
            weekdays: item
                .weekdays
                .split(',')
                .filter(|day| !day.is_empty())
                .map(str::to_string)
                .collect(),
            end_kind: item.end_kind.clone(),
            end_count: item.end_count,
            end_date: item.end_date.clone(),
            enabled: item.enabled,
            next_run_at: item.next_run_at.clone(),
            missed_at: item.missed_at.clone(),
            created_by: item.created_by.clone(),
            updated_by: item.updated_by.clone(),
            launch_block: item.launch_block.clone(),
        }
    }
}

fn never() -> String {
    "never".to_string()
}

pub async fn list_for_team(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<Vec<TeamScheduleResponse>>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let items = db.list_team_schedules(&team)?;
    let rows = items
        .into_iter()
        .map(|(item, target_name)| TeamScheduleResponse {
            schedule: ScheduleResponse::from(&item),
            target_name,
        })
        .collect();
    Ok(Json(rows))
}

pub async fn list_for_template(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Vec<ScheduleResponse>>> {
    list(&state, &headers, &user, "template", &id)
}

pub async fn list_for_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Vec<ScheduleResponse>>> {
    list(&state, &headers, &user, "workflow", &id)
}

pub async fn create_for_template(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ScheduleBody>,
) -> AppResult<(StatusCode, Json<ScheduleResponse>)> {
    create(&state, &headers, &user, "template", &id, body).await
}

pub async fn create_for_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ScheduleBody>,
) -> AppResult<(StatusCode, Json<ScheduleResponse>)> {
    create(&state, &headers, &user, "workflow", &id, body).await
}

pub async fn preview(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<PreviewBody>,
) -> AppResult<Json<PreviewResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let rule = rule_from_preview(&body)?;
    let times = service::preview_blocking(rule, Utc::now(), 5)
        .await
        .map_err(ApiError::bad)?;
    Ok(Json(PreviewResponse {
        occurrences: times.into_iter().map(service::format_utc).collect(),
    }))
}

pub async fn update(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ScheduleBody>,
) -> AppResult<Json<ScheduleResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let name = require_name(&body.name)?;
    let rule = rule_from_fields(&body)?;
    {
        let db = state.store();
        let existing = load(db, &id, &team)?;
        let stored = rule.start_date.format("%Y-%m-%d").to_string();
        if existing.start_date != stored {
            service::ensure_start_is_recent(rule.start_date, rule.timezone)
                .map_err(ApiError::bad)?;
        }
    }
    let (count, next) = service::first_from_blocking(rule.clone(), Utc::now())
        .await
        .map_err(ApiError::bad)?;
    let db = state.store();
    let existing = load(db, &id, &team)?;
    let mut item = stored(&existing, &name, &rule, count, &next);
    item.enabled = existing.enabled;
    item.missed_at = None;
    item.created_by = existing.created_by;
    item.created_at = existing.created_at;
    item.updated_by = user.username.clone();
    item.launch_block = None;
    db.update_schedule(&item)?;
    Ok(Json(ScheduleResponse::from(&item)))
}

pub async fn remove(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.store();
    db.delete_schedule(&id, &team)?;
    Ok(no_content())
}

pub async fn set_enabled(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<EnabledBody>,
) -> AppResult<Json<ScheduleResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let mut item = {
        let db = state.store();
        load(db, &id, &team)?
    };
    let now = Utc::now();
    if body.enabled {
        if let Some(next) = item.next_run_at.as_deref().and_then(service::parse_utc) {
            if next <= now {
                let rule = service::rule_from(&item).map_err(ApiError::bad)?;
                let count = u32::try_from(item.occurrence_count).unwrap_or(0);
                let decided = service::step_blocking(rule, count, now, true)
                    .await
                    .map_err(ApiError::bad)?;
                item.occurrence_count = i64::from(decided.occurrence_count);
                item.next_run_at = decided.next_run.map(service::format_utc);
            }
        }
    }
    item.enabled = body.enabled;
    item.updated_at = db::now();
    let db = state.store();
    db.set_schedule_enabled(
        &item.id,
        &team,
        item.enabled,
        item.next_run_at.as_deref(),
        item.occurrence_count,
    )?;
    Ok(Json(ScheduleResponse::from(&item)))
}

pub async fn answer_missed(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<MissedBody>,
) -> AppResult<Json<ScheduleResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let (item, missed_at) = {
        let db = state.store();
        let item = load(db, &id, &team)?;
        let Some(missed_at) = db.claim_missed(&id, &team)? else {
            return Err(ApiError::bad("schedule_has_no_missed_run"));
        };
        (item, missed_at)
    };
    if body.accept {
        if let Err(err) = service::start(&state, &item, &user.username) {
            let db = state.store();
            if let Err(restore) = db.restore_missed(&id, &team, &missed_at) {
                tracing::error!(
                    schedule = %id,
                    error = %restore,
                    "could not restore the missed schedule run"
                );
            }
            return Err(err);
        }
    }
    let db = state.store();
    let item = load(db, &id, &team)?;
    Ok(Json(ScheduleResponse::from(&item)))
}

fn list(
    state: &Shared,
    headers: &HeaderMap,
    user: &AuthUser,
    kind: &str,
    id: &str,
) -> AppResult<Json<Vec<ScheduleResponse>>> {
    let team = team_header(headers)?;
    gate(state, user, &team, Role::Viewer)?;
    let db = state.store();
    target_team(db, kind, id, &team)?;
    let items = db.list_schedules(kind, id)?;
    Ok(Json(
        items.iter().map(ScheduleResponse::from).collect::<Vec<_>>(),
    ))
}

async fn create(
    state: &Shared,
    headers: &HeaderMap,
    user: &AuthUser,
    kind: &str,
    id: &str,
    body: ScheduleBody,
) -> AppResult<(StatusCode, Json<ScheduleResponse>)> {
    let team = team_header(headers)?;
    gate(state, user, &team, Role::Operator)?;
    let name = require_name(&body.name)?;
    let rule = rule_from_body(&body)?;
    let (count, next) = service::first_from_blocking(rule.clone(), Utc::now())
        .await
        .map_err(ApiError::bad)?;
    let db = state.store();
    target_team(db, kind, id, &team)?;
    let now = db::now();
    let item = Schedule {
        id: Uuid::new_v4().to_string(),
        team_id: team,
        name,
        target_kind: kind.to_string(),
        target_id: id.to_string(),
        timezone: rule.timezone.to_string(),
        start_date: rule.start_date.format("%Y-%m-%d").to_string(),
        start_time: clock(rule.start_time),
        frequency: frequency_name(rule.frequency),
        weekdays: service::weekday_codes(&rule.weekdays),
        end_kind: end_name(rule.end),
        end_count: end_count(rule.end),
        end_date: end_date(rule.end),
        enabled: true,
        next_run_at: Some(service::format_utc(next)),
        missed_at: None,
        occurrence_count: i64::from(count),
        created_by: user.username.clone(),
        updated_by: user.username.clone(),
        created_at: now.clone(),
        updated_at: now,
        launch_block: None,
    };
    db.insert_schedule(&item)?;
    Ok((StatusCode::CREATED, Json(ScheduleResponse::from(&item))))
}

fn stored(
    existing: &Schedule,
    name: &str,
    rule: &Rule,
    count: u32,
    next: &chrono::DateTime<Utc>,
) -> Schedule {
    Schedule {
        id: existing.id.clone(),
        team_id: existing.team_id.clone(),
        name: name.to_string(),
        target_kind: existing.target_kind.clone(),
        target_id: existing.target_id.clone(),
        timezone: rule.timezone.to_string(),
        start_date: rule.start_date.format("%Y-%m-%d").to_string(),
        start_time: clock(rule.start_time),
        frequency: frequency_name(rule.frequency),
        weekdays: service::weekday_codes(&rule.weekdays),
        end_kind: end_name(rule.end),
        end_count: end_count(rule.end),
        end_date: end_date(rule.end),
        enabled: existing.enabled,
        next_run_at: Some(service::format_utc(*next)),
        missed_at: None,
        occurrence_count: i64::from(count),
        created_by: existing.created_by.clone(),
        updated_by: existing.updated_by.clone(),
        created_at: existing.created_at.clone(),
        updated_at: db::now(),
        launch_block: existing.launch_block.clone(),
    }
}

fn load(db: &dyn Backend, id: &str, team: &str) -> AppResult<Schedule> {
    let item = db.schedule(id)?.ok_or(ApiError::NotFound)?;
    if item.team_id != team {
        return Err(ApiError::NotFound);
    }
    Ok(item)
}

fn target_team(db: &dyn Backend, kind: &str, id: &str, team: &str) -> AppResult<()> {
    let owner = match kind {
        "template" => db.template(id)?.map(|item| item.team_id),
        "workflow" => db.workflow_team(id)?,
        _ => None,
    };
    match owner {
        Some(owner) if owner == team => Ok(()),
        Some(_) => Err(ApiError::NotFound),
        None => Err(ApiError::NotFound),
    }
}

fn rule_from_body(body: &ScheduleBody) -> AppResult<Rule> {
    let rule = rule_from_fields(body)?;
    service::ensure_start_is_recent(rule.start_date, rule.timezone).map_err(ApiError::bad)?;
    Ok(rule)
}

fn rule_from_fields(body: &ScheduleBody) -> AppResult<Rule> {
    service::parse(&Input {
        timezone: &body.timezone,
        start_date: &body.start_date,
        start_time: &body.start_time,
        frequency: &body.frequency,
        weekdays: &body.weekdays,
        end_kind: &body.end_kind,
        end_count: body.end_count,
        end_date: body.end_date.as_deref(),
    })
    .map_err(ApiError::bad)
}

fn rule_from_preview(body: &PreviewBody) -> AppResult<Rule> {
    let rule = service::parse(&Input {
        timezone: &body.timezone,
        start_date: &body.start_date,
        start_time: &body.start_time,
        frequency: &body.frequency,
        weekdays: &body.weekdays,
        end_kind: &body.end_kind,
        end_count: body.end_count,
        end_date: body.end_date.as_deref(),
    })
    .map_err(ApiError::bad)?;
    service::ensure_start_is_recent(rule.start_date, rule.timezone).map_err(ApiError::bad)?;
    Ok(rule)
}

fn clock(time: chrono::NaiveTime) -> String {
    format!("{:02}:{:02}", time.hour(), time.minute())
}

fn frequency_name(frequency: Frequency) -> String {
    match frequency {
        Frequency::Once => "once",
        Frequency::Hourly => "hourly",
        Frequency::Daily => "daily",
        Frequency::Weekly => "weekly",
        Frequency::Monthly => "monthly",
    }
    .to_string()
}

fn end_name(end: End) -> String {
    match end {
        End::Never => "never",
        End::After(_) => "after",
        End::OnDate(_) => "on_date",
    }
    .to_string()
}

fn end_count(end: End) -> Option<i64> {
    match end {
        End::After(count) => Some(i64::from(count)),
        _ => None,
    }
}

fn end_date(end: End) -> Option<String> {
    match end {
        End::OnDate(date) => Some(date.format("%Y-%m-%d").to_string()),
        _ => None,
    }
}
