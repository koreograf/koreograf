use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::backend::Backend;
use crate::db::{self, Credential};
use crate::error::{ApiError, ApiJson, AppResult};
use crate::features::identity::{AuthUser, Role};
use crate::http::{gate, no_content, require_name, team_header};
use crate::Shared;

use super::service::{check, describe, known_kind, merge_secret};

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route(
            "/credentials",
            get(list_credentials).post(create_credential),
        )
        .route(
            "/credentials/{id}",
            get(get_credential)
                .put(update_credential)
                .delete(delete_credential),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialBody {
    name: String,
    kind: String,
    secret: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialUpdate {
    name: String,
    #[serde(default)]
    secret: Value,
    #[serde(default)]
    clear: Vec<String>,
}

#[derive(Serialize)]
struct CredentialUsageResponse {
    inventory_id: Option<String>,
    inventory_name: Option<String>,
    source_id: Option<String>,
    source_name: Option<String>,
    project_id: Option<String>,
    project_name: Option<String>,
}

#[derive(Serialize)]
struct CredentialSummaryResponse {
    id: String,
    name: String,
    kind: String,
    created_at: String,
    usages: Vec<CredentialUsageResponse>,
}

#[derive(Serialize)]
#[serde(transparent)]
struct CredentialListResponse(Vec<CredentialSummaryResponse>);

#[derive(Serialize)]
struct CreatedCredentialResponse {
    id: String,
    name: String,
    kind: String,
}

#[derive(Serialize)]
struct CredentialDetailResponse {
    id: String,
    name: String,
    kind: String,
    created_at: String,
    fields: Value,
    set: Vec<String>,
    usages: Vec<CredentialUsageResponse>,
}

async fn list_credentials(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<CredentialListResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let mut items = Vec::new();
    for item in db.list_credentials(&team)? {
        items.push(CredentialSummaryResponse {
            id: item.id.clone(),
            name: item.name.clone(),
            kind: item.kind.clone(),
            created_at: item.created_at.clone(),
            usages: usage_list(db, &item.id, &team)?,
        });
    }
    Ok(Json(CredentialListResponse(items)))
}

async fn create_credential(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<CredentialBody>,
) -> AppResult<(StatusCode, Json<CreatedCredentialResponse>)> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Owner)?;
    if !known_kind(&body.kind) {
        return Err(ApiError::bad("unknown_credential_type"));
    }
    if let Err(message) = check(&body.kind, &body.secret) {
        return Err(ApiError::bad(message));
    }
    if body.secret.to_string().len() > 256 * 1024 {
        return Err(ApiError::bad("the_secret_is_too_large"));
    }
    let blob = state.encrypt(&body.secret.to_string())?;
    let item = Credential {
        id: Uuid::new_v4().to_string(),
        team_id: team,
        name: require_name(&body.name)?,
        kind: body.kind,
        secret_blob: blob,
        created_at: db::now(),
        auto: false,
        disabled: false,
    };
    let db = state.store();
    db.insert_credential(&item)?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedCredentialResponse {
            id: item.id,
            name: item.name,
            kind: item.kind,
        }),
    ))
}

async fn delete_credential(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<axum::response::Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Owner)?;
    let db = state.store();
    let names = db
        .credential_usages(&id, &team)?
        .into_iter()
        .map(|usage| usage_label(&usage))
        .collect::<Vec<_>>();
    if !names.is_empty() {
        return Err(ApiError::conflict_vars(
            "credential_is_in_use",
            [("names", names.join(", "))],
        ));
    }
    db.delete_credential(&id, &team)?;
    Ok(no_content())
}

fn usage_label(usage: &db::CredentialUsage) -> String {
    match (&usage.project_id, &usage.project_name) {
        (Some(_), Some(name)) => name.clone(),
        _ => format!(
            "{} / {}",
            usage.inventory_name.as_deref().unwrap_or(""),
            usage.source_name.as_deref().unwrap_or("")
        ),
    }
}

fn usage_list(c: &dyn Backend, id: &str, team: &str) -> AppResult<Vec<CredentialUsageResponse>> {
    Ok(c.credential_usages(id, team)?
        .into_iter()
        .map(|usage| CredentialUsageResponse {
            inventory_id: usage.inventory_id,
            inventory_name: usage.inventory_name,
            source_id: usage.source_id,
            source_name: usage.source_name,
            project_id: usage.project_id,
            project_name: usage.project_name,
        })
        .collect())
}

fn open_credential(state: &Shared, id: &str, team: &str) -> AppResult<(Credential, Value)> {
    let item = {
        let db = state.store();
        db.credential(id)?
            .filter(|item| item.team_id == team)
            .ok_or(ApiError::NotFound)?
    };
    let plain = state
        .decrypt(&item.secret_blob)
        .map_err(|_| ApiError::bad("credentials_cannot_be_read"))?;
    let secret =
        serde_json::from_str(&plain).map_err(|_| ApiError::bad("credentials_are_not_json"))?;
    Ok((item, secret))
}

fn credential_detail(
    item: &Credential,
    secret: &Value,
    usages: Vec<CredentialUsageResponse>,
) -> CredentialDetailResponse {
    let (fields, set) = describe(&item.kind, secret);
    CredentialDetailResponse {
        id: item.id.clone(),
        name: item.name.clone(),
        kind: item.kind.clone(),
        created_at: item.created_at.clone(),
        fields,
        set,
        usages,
    }
}

async fn get_credential(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<CredentialDetailResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Owner)?;
    let (item, secret) = open_credential(&state, &id, &team)?;
    let usages = {
        let db = state.store();
        usage_list(db, &item.id, &team)?
    };
    Ok(Json(credential_detail(&item, &secret, usages)))
}

async fn update_credential(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<CredentialUpdate>,
) -> AppResult<Json<CredentialDetailResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Owner)?;
    let (mut item, previous) = open_credential(&state, &id, &team)?;
    let secret =
        merge_secret(&item.kind, &previous, &body.secret, &body.clear).map_err(ApiError::bad)?;
    if let Err(message) = check(&item.kind, &secret) {
        return Err(ApiError::bad(message));
    }
    let raw = secret.to_string();
    if raw.len() > 256 * 1024 {
        return Err(ApiError::bad("the_secret_is_too_large"));
    }
    item.name = require_name(&body.name)?;
    item.secret_blob = state.encrypt(&raw)?;
    item.disabled = false;
    let db = state.store();
    db.update_credential(&item)?;
    let usages = usage_list(db, &item.id, &team)?;
    Ok(Json(credential_detail(&item, &secret, usages)))
}
