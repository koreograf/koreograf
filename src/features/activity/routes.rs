use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, AppResult};
use crate::features::identity::{AuthUser, Role};
use crate::http::{gate, team_header};
use crate::Shared;

use super::model::{Activity, ActivityQuery};

pub(crate) fn routes() -> Router<Shared> {
    Router::new().route("/activity", get(list_activity))
}

#[derive(Deserialize)]
pub struct ListActivityQuery {
    limit: Option<String>,
    offset: Option<String>,
    actor: Option<String>,
    action: Option<String>,
    object_type: Option<String>,
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
pub(crate) struct ActivityResponse {
    id: String,
    team_id: String,
    actor: String,
    action: String,
    object_type: String,
    object_id: String,
    object_name: String,
    result: String,
    created_at: String,
}

impl ActivityResponse {
    fn new(item: &Activity) -> Self {
        Self {
            id: item.id.clone(),
            team_id: item.team_id.clone(),
            actor: item.actor.clone(),
            action: item.action.clone(),
            object_type: item.object_type.clone(),
            object_id: item.object_id.clone(),
            object_name: item.object_name.clone(),
            result: item.result.clone(),
            created_at: item.created_at.clone(),
        }
    }
}

#[derive(Serialize)]
pub(crate) struct ActivityListResponse {
    items: Vec<ActivityResponse>,
    total: i64,
}

pub async fn list_activity(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Query(query): Query<ListActivityQuery>,
) -> AppResult<Json<ActivityListResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let limit = query_number(query.limit, 25, "limit")?.clamp(1, 100);
    let offset = query_number(query.offset, 0, "offset")?.max(0);
    let filter = ActivityQuery {
        limit,
        offset,
        actor: filter_value(query.actor),
        action: filter_value(query.action),
        object_type: filter_value(query.object_type),
        since: query_date(query.since, "since")?,
        until: query_date(query.until, "until")?,
    };
    let db = state.store();
    let (items, total) = db.list_activity(&team, &filter)?;
    Ok(Json(ActivityListResponse {
        items: items.iter().map(ActivityResponse::new).collect(),
        total,
    }))
}
