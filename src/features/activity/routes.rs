use std::collections::HashMap;

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
    /// The human who created a service account, so the activity view can show
    /// it next to the service-account actor.
    actor_created_by: Option<String>,
}

impl ActivityResponse {
    fn new(item: &Activity, actor_created_by: Option<String>) -> Self {
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
            actor_created_by,
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
    let mut creators: HashMap<String, Option<String>> = HashMap::new();
    let mut responses = Vec::with_capacity(items.len());
    for item in &items {
        let creator = match creators.get(&item.actor) {
            Some(value) => value.clone(),
            None => {
                let value = actor_creator(&state, &item.actor)?;
                creators.insert(item.actor.clone(), value.clone());
                value
            }
        };
        responses.push(ActivityResponse::new(item, creator));
    }
    Ok(Json(ActivityListResponse {
        items: responses,
        total,
    }))
}

fn actor_creator(state: &Shared, username: &str) -> AppResult<Option<String>> {
    let Some(user) = state.store().user_by_username(username)? else {
        return Ok(None);
    };
    if user.auth_source != "service" {
        return Ok(None);
    }
    Ok(state
        .store()
        .service_account_by_id(&user.id)?
        .and_then(|account| account.created_by_username))
}
