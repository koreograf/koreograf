use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::Json;
use chrono::{Timelike, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{AuthUser, Role};
use crate::db::{self, Schedule};
use crate::error::{ApiError, ApiJson, AppResult};
use crate::schedule::{self, Input, Rule};
use crate::Shared;

use super::{created, gate, no_content, require_name, team_header};

#[derive(Deserialize)]
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
pub struct EnabledBody {
    enabled: bool,
}

#[derive(Deserialize)]
pub struct MissedBody {
    accept: bool,
}

fn never() -> String {
    "never".to_string()
}

pub async fn list_for_team(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.db()?;
    let items = db::list_team_schedules(&db, &team)?;
    let rows = items
        .iter()
        .map(|(item, target_name)| {
            let mut value = schedule_json(item);
            value["target_name"] = json!(target_name);
            value
        })
        .collect::<Vec<_>>();
    Ok(Json(json!(rows)))
}

pub async fn list_for_template(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    list(&state, &headers, &user, "template", &id)
}

pub async fn list_for_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    list(&state, &headers, &user, "workflow", &id)
}

pub async fn create_for_template(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ScheduleBody>,
) -> AppResult<Response> {
    create(&state, &headers, &user, "template", &id, body)
}

pub async fn create_for_workflow(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ScheduleBody>,
) -> AppResult<Response> {
    create(&state, &headers, &user, "workflow", &id, body)
}

pub async fn preview(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<PreviewBody>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let rule = rule_from_preview(&body)?;
    let times = schedule::preview(&rule, Utc::now(), 5).map_err(ApiError::bad)?;
    Ok(Json(json!({
        "occurrences": times.into_iter().map(schedule::format_utc).collect::<Vec<_>>()
    })))
}

pub async fn update(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ScheduleBody>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let name = require_name(&body.name)?;
    let rule = rule_from_body(&body)?;
    let (count, next) = schedule::first_from(&rule, Utc::now()).map_err(ApiError::bad)?;
    let db = state.db()?;
    let existing = load(&db, &id, &team)?;
    let mut item = stored(&existing, &name, &rule, count, &next);
    item.enabled = existing.enabled;
    item.missed_at = None;
    item.created_by = existing.created_by;
    item.created_at = existing.created_at;
    db::update_schedule(&db, &item)?;
    Ok(Json(schedule_json(&item)))
}

pub async fn remove(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.db()?;
    db::delete_schedule(&db, &id, &team)?;
    Ok(no_content())
}

pub async fn set_enabled(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<EnabledBody>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.db()?;
    let mut item = load(&db, &id, &team)?;
    let now = Utc::now();
    if body.enabled {
        if let Some(next) = item.next_run_at.as_deref().and_then(schedule::parse_utc) {
            if next <= now {
                let rule = schedule::rule_from(&item).map_err(ApiError::bad)?;
                let count = u32::try_from(item.occurrence_count).unwrap_or(0);
                let decided = schedule::step(&rule, count, now, true).map_err(ApiError::bad)?;
                item.occurrence_count = i64::from(decided.occurrence_count);
                item.next_run_at = decided.next_run.map(schedule::format_utc);
            }
        }
    }
    item.enabled = body.enabled;
    item.updated_at = db::now();
    db::set_schedule_enabled(
        &db,
        &item.id,
        &team,
        item.enabled,
        item.next_run_at.as_deref(),
        item.occurrence_count,
    )?;
    Ok(Json(schedule_json(&item)))
}

pub async fn answer_missed(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<MissedBody>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let item = {
        let db = state.db()?;
        let item = load(&db, &id, &team)?;
        if item.missed_at.is_none() {
            return Err(ApiError::bad("schedule_has_no_missed_run"));
        }
        item
    };
    if body.accept {
        schedule::start(&state, &item, &user.username)?;
    }
    let db = state.db()?;
    db::note_miss(&db, &id, None)?;
    let mut item = load(&db, &id, &team)?;
    item.missed_at = None;
    Ok(Json(schedule_json(&item)))
}

fn list(
    state: &Shared,
    headers: &HeaderMap,
    user: &AuthUser,
    kind: &str,
    id: &str,
) -> AppResult<Json<Value>> {
    let team = team_header(headers)?;
    gate(state, user, &team, Role::Viewer)?;
    let db = state.db()?;
    target_team(&db, kind, id, &team)?;
    let items = db::list_schedules(&db, kind, id)?;
    Ok(Json(json!(items
        .iter()
        .map(schedule_json)
        .collect::<Vec<_>>())))
}

fn create(
    state: &Shared,
    headers: &HeaderMap,
    user: &AuthUser,
    kind: &str,
    id: &str,
    body: ScheduleBody,
) -> AppResult<Response> {
    let team = team_header(headers)?;
    gate(state, user, &team, Role::Operator)?;
    let name = require_name(&body.name)?;
    let rule = rule_from_body(&body)?;
    let (count, next) = schedule::first_from(&rule, Utc::now()).map_err(ApiError::bad)?;
    let db = state.db()?;
    target_team(&db, kind, id, &team)?;
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
        weekdays: schedule::weekday_codes(&rule.weekdays),
        end_kind: end_name(rule.end),
        end_count: end_count(rule.end),
        end_date: end_date(rule.end),
        enabled: true,
        next_run_at: Some(schedule::format_utc(next)),
        missed_at: None,
        occurrence_count: i64::from(count),
        created_by: user.username.clone(),
        created_at: now.clone(),
        updated_at: now,
    };
    db::insert_schedule(&db, &item)?;
    Ok(created(schedule_json(&item)))
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
        weekdays: schedule::weekday_codes(&rule.weekdays),
        end_kind: end_name(rule.end),
        end_count: end_count(rule.end),
        end_date: end_date(rule.end),
        enabled: existing.enabled,
        next_run_at: Some(schedule::format_utc(*next)),
        missed_at: None,
        occurrence_count: i64::from(count),
        created_by: existing.created_by.clone(),
        created_at: existing.created_at.clone(),
        updated_at: db::now(),
    }
}

fn load(db: &rusqlite::Connection, id: &str, team: &str) -> AppResult<Schedule> {
    let item = db::schedule(db, id)?.ok_or(ApiError::NotFound)?;
    if item.team_id != team {
        return Err(ApiError::NotFound);
    }
    Ok(item)
}

fn target_team(db: &rusqlite::Connection, kind: &str, id: &str, team: &str) -> AppResult<()> {
    let owner = match kind {
        "template" => db::template(db, id)?.map(|item| item.team_id),
        "workflow" => db::workflow(db, id)?.map(|item| item.team_id),
        _ => None,
    };
    match owner {
        Some(owner) if owner == team => Ok(()),
        Some(_) => Err(ApiError::NotFound),
        None => Err(ApiError::NotFound),
    }
}

fn rule_from_body(body: &ScheduleBody) -> AppResult<Rule> {
    schedule::parse(&Input {
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
    schedule::parse(&Input {
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

fn schedule_json(item: &Schedule) -> Value {
    let days: Vec<&str> = item
        .weekdays
        .split(',')
        .filter(|day| !day.is_empty())
        .collect();
    json!({
        "id": item.id,
        "name": item.name,
        "target_kind": item.target_kind,
        "target_id": item.target_id,
        "timezone": item.timezone,
        "start_date": item.start_date,
        "start_time": item.start_time,
        "frequency": item.frequency,
        "weekdays": days,
        "end_kind": item.end_kind,
        "end_count": item.end_count,
        "end_date": item.end_date,
        "enabled": item.enabled,
        "next_run_at": item.next_run_at,
        "missed_at": item.missed_at,
        "created_by": item.created_by,
    })
}

fn clock(time: chrono::NaiveTime) -> String {
    format!("{:02}:{:02}", time.hour(), time.minute())
}

fn frequency_name(frequency: schedule::Frequency) -> String {
    match frequency {
        schedule::Frequency::Once => "once",
        schedule::Frequency::Hourly => "hourly",
        schedule::Frequency::Daily => "daily",
        schedule::Frequency::Weekly => "weekly",
        schedule::Frequency::Monthly => "monthly",
    }
    .to_string()
}

fn end_name(end: schedule::End) -> String {
    match end {
        schedule::End::Never => "never",
        schedule::End::After(_) => "after",
        schedule::End::OnDate(_) => "on_date",
    }
    .to_string()
}

fn end_count(end: schedule::End) -> Option<i64> {
    match end {
        schedule::End::After(count) => Some(i64::from(count)),
        _ => None,
    }
}

fn end_date(end: schedule::End) -> Option<String> {
    match end {
        schedule::End::OnDate(date) => Some(date.format("%Y-%m-%d").to_string()),
        _ => None,
    }
}
