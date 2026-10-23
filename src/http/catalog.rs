use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{AuthUser, Role};
use crate::cacheinfo;
use crate::db::{self, Credential, Environment, Inventory, Project};
use crate::error::{ApiError, ApiJson, AppResult};
use crate::jobs;
use crate::runner;
use crate::Shared;

use super::{blank, created, gate, no_content, require_name, team_header};

#[derive(Deserialize)]
pub struct TeamBody {
    name: String,
    #[serde(default)]
    description: String,
    ldap_group: Option<String>,
    organization_id: Option<String>,
}

#[derive(Deserialize)]
pub struct MemberBody {
    username: String,
    role: String,
}

#[derive(Deserialize)]
pub struct ProjectBody {
    name: String,
    source_type: String,
    source: String,
    #[serde(default)]
    revision: String,
    organization_id: Option<String>,
    #[serde(default)]
    team_ids: Vec<String>,
}

#[derive(Deserialize)]
pub struct ProjectAssignBody {
    organization_id: String,
    #[serde(default)]
    team_ids: Vec<String>,
}

#[derive(Deserialize)]
pub struct OrganizationBody {
    name: String,
    #[serde(default)]
    description: String,
}

#[derive(Deserialize)]
pub struct InventoryBody {
    name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    sources: Vec<super::sources::SourceInput>,
}

#[derive(Deserialize)]
pub struct CredentialBody {
    name: String,
    kind: String,
    secret: Value,
}

#[derive(Deserialize)]
pub struct CredentialUpdate {
    name: String,
    #[serde(default)]
    secret: Value,
    #[serde(default)]
    clear: Vec<String>,
}

#[derive(Deserialize)]
pub struct EnvironmentBody {
    name: String,
    image: String,
    #[serde(default = "default_pull")]
    pull_policy: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    scope: Option<String>,
}

fn default_pull() -> String {
    "if_not_present".into()
}

pub async fn overview(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    // The runtime probe awaits. Take it before the database lock so the
    // connection guard, which is not Send, is never held across that point.
    let runtime = runner::detect_runtime().await;
    let effective = crate::settings::load(&state)?;
    let db = state.db()?;
    let jobs = db::list_jobs(&db, &team, 8)?;
    let deliveries = db::list_deliveries(&db, &team, 8)?;
    let workflow = db::first_workflow(&db, &team)?;
    let hook = db::first_hook(&db, &team)?;
    let cache = cacheinfo::team_cache(&state.cfg.data_dir, &team);
    Ok(Json(json!({
        "runner_mode": effective.runner.as_str(),
        "runtime": runtime,
        "ldap": effective.directory().is_some(),
        "cache_bytes": cache.collections_bytes + cache.roles_bytes + cache.tofu_bytes,
        "jobs": jobs.iter().map(|job| super::job_json(job, None)).collect::<Vec<_>>(),
        "deliveries": deliveries,
        "workflow_id": workflow.as_ref().map(|item| &item.id),
        "workflow_name": workflow.as_ref().map(|item| &item.name),
        "hook_id": hook.as_ref().map(|item| &item.id),
        "hook_name": hook.as_ref().map(|item| &item.name),
    })))
}

pub async fn list_teams(State(state): State<Shared>, user: AuthUser) -> AppResult<Json<Value>> {
    let db = state.db()?;
    let teams = db::list_teams_for_user(&db, &user.id, user.is_admin)?;
    Ok(Json(json!(teams
        .into_iter()
        .map(|(team, role)| json!({
            "id": team.id,
            "name": team.name,
            "description": team.description,
            "ldap_group": team.ldap_group,
            "organization_id": team.organization_id,
            "organization_name": team.organization_name,
            "role": role,
            "created_at": team.created_at,
        }))
        .collect::<Vec<_>>())))
}

pub async fn create_team(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<TeamBody>,
) -> AppResult<axum::response::Response> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    let name = require_name(&body.name)?;
    let id = Uuid::new_v4().to_string();
    let ldap_group = blank(body.ldap_group);
    let db = state.db()?;
    let organization_id = organization_for_new_team(&db, body.organization_id, &name)?;
    db::insert_team(
        &db,
        &id,
        &organization_id,
        &name,
        body.description.trim(),
        ldap_group.as_deref(),
    )?;
    db::upsert_membership(&db, &id, &user.id, "owner", "manual")?;
    let team = db::team_by_id(&db, &id)?.ok_or(ApiError::NotFound)?;
    Ok(created(team_json(&team)))
}

pub async fn update_team(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<TeamBody>,
) -> AppResult<Json<Value>> {
    gate(&state, &user, &id, Role::Owner)?;
    let name = require_name(&body.name)?;
    let ldap_group = blank(body.ldap_group);
    let db = state.db()?;
    if let Some(organization_id) = blank(body.organization_id) {
        if !user.is_admin {
            return Err(ApiError::Forbidden);
        }
        db::organization(&db, &organization_id)?
            .ok_or_else(|| ApiError::bad("organization_is_missing"))?;
        db::set_team_organization(&db, &id, &organization_id)?;
    }
    db::update_team(
        &db,
        &id,
        &name,
        body.description.trim(),
        ldap_group.as_deref(),
    )?;
    let team = db::team_by_id(&db, &id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(team_json(&team)))
}

fn organization_for_new_team(
    db: &rusqlite::Connection,
    requested: Option<String>,
    team_name: &str,
) -> AppResult<String> {
    if let Some(organization_id) = blank(requested) {
        db::organization(db, &organization_id)?
            .ok_or_else(|| ApiError::bad("organization_is_missing"))?;
        return Ok(organization_id);
    }
    if let Some(existing) = db::organization_by_name(db, team_name)? {
        return Ok(existing.id);
    }
    let organization_id = Uuid::new_v4().to_string();
    db::insert_organization(db, &organization_id, team_name, "")?;
    Ok(organization_id)
}

pub async fn list_members(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    gate(&state, &user, &id, Role::Viewer)?;
    let db = state.db()?;
    let members = db::list_members(&db, &id)?;
    Ok(Json(json!(members)))
}

pub async fn add_member(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<MemberBody>,
) -> AppResult<axum::response::Response> {
    gate(&state, &user, &id, Role::Owner)?;
    let role =
        Role::parse(&body.role).ok_or_else(|| ApiError::bad("role_is_viewer_operator_or_owner"))?;
    let username = body.username.trim().to_ascii_lowercase();
    let db = state.db()?;
    let member = db::user_by_username(&db, &username)?
        .ok_or_else(|| ApiError::bad("unknown_user_create_the_account_first"))?;
    db::grant_manual_membership(&db, &id, &member.id, role.as_str())?;
    Ok(created(
        json!({"user_id": member.id, "role": role.as_str()}),
    ))
}

pub async fn remove_member(
    State(state): State<Shared>,
    user: AuthUser,
    Path((id, user_id)): Path<(String, String)>,
) -> AppResult<axum::response::Response> {
    gate(&state, &user, &id, Role::Owner)?;
    if user_id == user.id && !user.is_admin {
        return Err(ApiError::bad("you_cannot_remove_yourself_from_the_team"));
    }
    let db = state.db()?;
    db::delete_membership(&db, &id, &user_id)?;
    Ok(no_content())
}

pub async fn get_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.db()?;
    if !db::project_visible(&db, &id, &team)? {
        return Err(ApiError::NotFound);
    }
    let project = db::project(&db, &id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(json!(project)))
}

pub async fn list_projects(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.db()?;
    Ok(Json(json!(db::list_projects(&db, &team)?)))
}

pub async fn update_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ProjectAssignBody>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.db()?;
    if !db::project_visible(&db, &id, &team)? {
        return Err(ApiError::NotFound);
    }
    let organization_id = body.organization_id.trim();
    if organization_id.is_empty() {
        return Err(ApiError::bad("organization_is_missing"));
    }
    let current = db::team_by_id(&db, &team)?.ok_or(ApiError::NotFound)?;
    if !user.is_admin && current.organization_id != organization_id {
        return Err(ApiError::bad("a_project_stays_in_the_organization_of_the"));
    }
    db::organization(&db, organization_id)?
        .ok_or_else(|| ApiError::bad("organization_is_missing"))?;
    check_team_ids(&db, organization_id, &body.team_ids)?;
    db::set_project_organization(&db, &id, organization_id)?;
    db::replace_project_teams(&db, &id, &body.team_ids)?;
    db::set_project_admin_saved(&db, &id, user.is_admin)?;
    let project = db::project(&db, &id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(json!(project)))
}

pub async fn create_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<ProjectBody>,
) -> AppResult<axum::response::Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let name = require_name(&body.name)?;
    let source = body.source.trim();
    let db = state.db()?;
    let current = db::team_by_id(&db, &team)?.ok_or(ApiError::NotFound)?;
    let organization_id = match blank(body.organization_id) {
        Some(organization_id) => {
            if !user.is_admin && organization_id != current.organization_id {
                return Err(ApiError::bad("a_project_stays_in_the_organization_of_the"));
            }
            db::organization(&db, &organization_id)?
                .ok_or_else(|| ApiError::bad("organization_is_missing"))?;
            organization_id
        }
        None => current.organization_id,
    };
    check_team_ids(&db, &organization_id, &body.team_ids)?;
    let revision = body.revision.trim();
    if !runner::valid_revision(revision) {
        return Err(ApiError::bad(
            "revision_may_contain_only_letters_digits_dot_slash",
        ));
    }
    match body.source_type.as_str() {
        "path" => {
            runner::authorize_project_source(
                "path",
                source,
                &state.cfg.data_dir,
                &state.cfg.project_roots,
                user.is_admin,
            )
            .map_err(project_path_error)?;
        }
        "git" => {
            if !runner::valid_git(source) {
                return Err(ApiError::bad("the_git_address_must_be_http_s_ssh"));
            }
            if source.starts_with("file://") {
                runner::authorize_project_source(
                    "git",
                    source,
                    &state.cfg.data_dir,
                    &state.cfg.project_roots,
                    user.is_admin,
                )
                .map_err(project_path_error)?;
            }
        }
        _ => return Err(ApiError::bad("source_is_path_or_git")),
    }
    let project = Project {
        id: Uuid::new_v4().to_string(),
        team_id: team,
        organization_id,
        organization_name: String::new(),
        name,
        source_type: body.source_type,
        source: source.to_string(),
        revision: revision.to_string(),
        created_at: db::now(),
        team_ids: body.team_ids,
        admin_saved: user.is_admin,
    };
    db::insert_project(&db, &project)?;
    let stored = db::project(&db, &project.id)?.ok_or(ApiError::NotFound)?;
    Ok(created(json!(stored)))
}

fn project_path_error(err: runner::ProjectPathError) -> ApiError {
    match err {
        runner::ProjectPathError::NotDirectory => {
            ApiError::bad("the_project_path_is_not_a_directory")
        }
        runner::ProjectPathError::Denied => ApiError::bad("the_project_path_is_not_allowed"),
        runner::ProjectPathError::AdminsOnly => {
            ApiError::bad("path_projects_are_limited_to_administrators")
        }
    }
}

fn check_team_ids(
    db: &rusqlite::Connection,
    organization_id: &str,
    team_ids: &[String],
) -> AppResult<()> {
    for team_id in team_ids {
        let team = db::team_by_id(db, team_id)?.ok_or_else(|| ApiError::bad("team_is_missing"))?;
        if team.organization_id != organization_id {
            return Err(ApiError::bad(
                "assigned_teams_must_belong_to_the_project_organization",
            ));
        }
    }
    Ok(())
}

pub async fn delete_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<axum::response::Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.db()?;
    db::delete_project(&db, &id, &team)?;
    Ok(no_content())
}

pub async fn sync_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let project = {
        let db = state.db()?;
        let project = db::project(&db, &id)?.ok_or(ApiError::NotFound)?;
        if !db::project_visible(&db, &id, &team)? {
            return Err(ApiError::NotFound);
        }
        project
    };
    let path = jobs::sync_project(&state, &project).await?;
    Ok(Json(json!({"path": path})))
}

pub async fn list_inventories(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.db()?;
    let items = db::list_inventories(&db, &team)?;
    let mut rows = Vec::with_capacity(items.len());
    for item in &items {
        rows.push(inventory_json(&db, item)?);
    }
    Ok(Json(json!(rows)))
}

pub async fn get_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.db()?;
    let item = db::inventory(&db, &id)?.ok_or(ApiError::NotFound)?;
    if item.team_id != team {
        return Err(ApiError::NotFound);
    }
    Ok(Json(inventory_json(&db, &item)?))
}

pub async fn update_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<InventoryBody>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let name = require_name(&body.name)?;
    let db = state.db()?;
    let existing = db::inventory(&db, &id)?.ok_or(ApiError::NotFound)?;
    if existing.team_id != team {
        return Err(ApiError::NotFound);
    }
    let sources = db::list_inventory_sources(&db, &id)?;
    if !body.body.trim().is_empty() {
        if sources.len() > 1 {
            return Err(ApiError::bad(
                "this_inventory_has_several_sources_edit_the_text",
            ));
        }
        if let Some(source) = sources.first() {
            if source.kind != "ini" {
                return Err(ApiError::bad("this_source_comes_from_a_project_or_from"));
            }
        }
    }
    db::update_inventory(&db, &id, &team, &name, &existing.body)?;
    if !body.body.trim().is_empty() {
        if let Some(source) = sources.first() {
            db::set_inventory_source_body(&db, &source.id, &body.body)?;
            db::refresh_inventory_summary(&db, &id)?;
        } else {
            db::set_inventory_body(&db, &id, &team, &body.body)?;
        }
    }
    db::set_inventory_admin_saved(&db, &id, user.is_admin)?;
    let item = db::inventory(&db, &id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(inventory_json(&db, &item)?))
}

pub async fn create_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<InventoryBody>,
) -> AppResult<axum::response::Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let name = require_name(&body.name)?;
    if !body.sources.is_empty() {
        return super::sources::create_with_sources(state, team, name, body.sources, user.is_admin)
            .await;
    }
    let item = Inventory {
        id: Uuid::new_v4().to_string(),
        team_id: team,
        name,
        body: body.body.clone(),
        created_at: db::now(),
        source_kind: "ini".into(),
        source_json: "{}".into(),
        credential_id: None,
        admin_saved: user.is_admin,
    };
    let source = if body.body.trim().is_empty() {
        None
    } else {
        Some(db::InventorySource {
            id: Uuid::new_v4().to_string(),
            inventory_id: item.id.clone(),
            name: "manual".into(),
            kind: "ini".into(),
            source_json: "{}".into(),
            credential_id: None,
            body: body.body,
            position: 0,
            created_at: item.created_at.clone(),
        })
    };
    let mut db = state.db()?;
    let tx = db.transaction()?;
    db::insert_inventory(&tx, &item)?;
    if let Some(source) = &source {
        db::insert_inventory_source(&tx, source)?;
    }
    tx.commit()?;
    Ok(created(inventory_json(&db, &item)?))
}

pub(super) fn inventory_json(c: &Connection, item: &Inventory) -> AppResult<Value> {
    let sources = db::list_inventory_sources(c, &item.id)?;
    let hosts = if sources.is_empty() {
        crate::inventory::parse_hosts(&item.body)
    } else {
        let bodies: Vec<&str> = sources.iter().map(|source| source.body.as_str()).collect();
        crate::inventory::merge_hosts(&bodies)
    };
    Ok(json!({
        "id": item.id,
        "team_id": item.team_id,
        "name": item.name,
        "body": item.body,
        "created_at": item.created_at,
        "source_kind": item.source_kind,
        "source": db::json_object(&item.source_json),
        "credential_id": item.credential_id,
        "source_count": sources.len(),
        "sources": sources.iter().map(|source| json!({
            "id": source.id,
            "name": source.name,
            "kind": source.kind,
            "source": db::json_object(&source.source_json),
            "credential_id": source.credential_id,
            "host_count": crate::inventory::parse_hosts(&source.body).len(),
            "body": source.body,
        })).collect::<Vec<_>>(),
        "host_count": hosts.len(),
        "hosts": hosts,
    }))
}

pub async fn delete_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<axum::response::Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.db()?;
    db::delete_inventory(&db, &id, &team)?;
    Ok(no_content())
}

pub async fn list_credentials(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.db()?;
    let items = db::list_credentials(&db, &team)?;
    Ok(Json(json!(items
        .into_iter()
        .map(|item| json!({
            "id": item.id,
            "name": item.name,
            "kind": item.kind,
            "created_at": item.created_at,
        }))
        .collect::<Vec<_>>())))
}

pub async fn create_credential(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<CredentialBody>,
) -> AppResult<axum::response::Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Owner)?;
    if !crate::credentials::known_kind(&body.kind) {
        return Err(ApiError::bad("unknown_credential_type"));
    }
    if let Err(message) = crate::credentials::check(&body.kind, &body.secret) {
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
    };
    let db = state.db()?;
    db::insert_credential(&db, &item)?;
    Ok(created(json!({
        "id": item.id,
        "name": item.name,
        "kind": item.kind,
    })))
}

pub async fn delete_credential(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<axum::response::Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Owner)?;
    let db = state.db()?;
    db::delete_credential(&db, &id, &team)?;
    Ok(no_content())
}

fn open_credential(state: &Shared, id: &str, team: &str) -> AppResult<(Credential, Value)> {
    let item = {
        let db = state.db()?;
        db::credential(&db, id)?
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

fn credential_json(item: &Credential, secret: &Value) -> Value {
    let (fields, set) = crate::credentials::describe(&item.kind, secret);
    json!({
        "id": item.id,
        "name": item.name,
        "kind": item.kind,
        "created_at": item.created_at,
        "fields": fields,
        "set": set,
    })
}

pub async fn get_credential(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let (item, secret) = open_credential(&state, &id, &team)?;
    Ok(Json(credential_json(&item, &secret)))
}

pub async fn update_credential(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<CredentialUpdate>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Owner)?;
    let (mut item, previous) = open_credential(&state, &id, &team)?;
    let secret = crate::credentials::merge_secret(&item.kind, &previous, &body.secret, &body.clear)
        .map_err(ApiError::bad)?;
    if let Err(message) = crate::credentials::check(&item.kind, &secret) {
        return Err(ApiError::bad(message));
    }
    let raw = secret.to_string();
    if raw.len() > 256 * 1024 {
        return Err(ApiError::bad("the_secret_is_too_large"));
    }
    item.name = require_name(&body.name)?;
    item.secret_blob = state.encrypt(&raw)?;
    let db = state.db()?;
    db::update_credential(&db, &item)?;
    Ok(Json(credential_json(&item, &secret)))
}

pub async fn list_environments(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Viewer)?;
    let db = state.db()?;
    let team = db::team_by_id(&db, &team_id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(json!(db::list_environments(&db, &team)?)))
}

fn environment_scope(raw: Option<&str>, fallback: &str) -> AppResult<String> {
    let raw = raw.unwrap_or("").trim();
    let scope = if raw.is_empty() { fallback } else { raw };
    match scope {
        "team" | "organization" | "global" => Ok(scope.to_string()),
        _ => Err(ApiError::bad("scope_is_team_organization_or_global")),
    }
}

fn may_manage_environment(user: &AuthUser, scope: &str) -> bool {
    user.is_admin || matches!(scope, "team" | "organization")
}

fn apply_environment_scope(item: &mut Environment, scope: &str, team: &db::Team) -> AppResult<()> {
    match scope {
        "team" => {
            item.team_id = Some(team.id.clone());
            item.organization_id = None;
        }
        "organization" => {
            if team.organization_id.is_empty() {
                return Err(ApiError::bad("organization_is_missing"));
            }
            item.team_id = None;
            item.organization_id = Some(team.organization_id.clone());
        }
        "global" => {
            item.team_id = None;
            item.organization_id = None;
        }
        _ => {
            return Err(ApiError::bad("scope_is_team_organization_or_global"));
        }
    }
    Ok(())
}

fn visible_environment(db: &Connection, id: &str, team: &db::Team) -> AppResult<Environment> {
    let item = db::environment(db, id)?.ok_or(ApiError::NotFound)?;
    if !db::environment_visible(&item, team) {
        return Err(ApiError::NotFound);
    }
    Ok(item)
}

fn environment_fields(body: &EnvironmentBody) -> AppResult<(String, String, String, String)> {
    if !matches!(
        body.pull_policy.as_str(),
        "if_not_present" | "always" | "never"
    ) {
        return Err(ApiError::bad("pull_policy_is_if_not_present_always_or"));
    }
    let image = body.image.trim();
    if !runner::valid_image(image) {
        return Err(ApiError::bad("the_image_name_is_invalid"));
    }
    Ok((
        require_name(&body.name)?,
        image.to_string(),
        body.pull_policy.clone(),
        body.description.trim().to_string(),
    ))
}

pub async fn create_environment(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<EnvironmentBody>,
) -> AppResult<axum::response::Response> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Owner)?;
    let (name, image, pull_policy, description) = environment_fields(&body)?;
    let scope = environment_scope(body.scope.as_deref(), "team")?;
    if !may_manage_environment(&user, &scope) {
        return Err(ApiError::Forbidden);
    }
    let db = state.db()?;
    let team = db::team_by_id(&db, &team_id)?.ok_or(ApiError::NotFound)?;
    let mut item = Environment {
        id: Uuid::new_v4().to_string(),
        team_id: None,
        organization_id: None,
        name,
        image,
        pull_policy,
        description,
        created_at: db::now(),
        scope: String::new(),
        organization_name: String::new(),
        team_name: String::new(),
    };
    apply_environment_scope(&mut item, &scope, &team)?;
    db::insert_environment(&db, &item)?;
    let stored = db::environment(&db, &item.id)?.ok_or(ApiError::NotFound)?;
    Ok(created(json!(stored)))
}

pub async fn get_environment(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Viewer)?;
    let db = state.db()?;
    let team = db::team_by_id(&db, &team_id)?.ok_or(ApiError::NotFound)?;
    let item = visible_environment(&db, &id, &team)?;
    Ok(Json(json!(item)))
}

pub async fn update_environment(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<EnvironmentBody>,
) -> AppResult<Json<Value>> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Owner)?;
    let (name, image, pull_policy, description) = environment_fields(&body)?;
    let db = state.db()?;
    let team = db::team_by_id(&db, &team_id)?.ok_or(ApiError::NotFound)?;
    let mut item = visible_environment(&db, &id, &team)?;
    let scope = environment_scope(body.scope.as_deref(), &item.scope)?;
    if !may_manage_environment(&user, &item.scope) || !may_manage_environment(&user, &scope) {
        return Err(ApiError::Forbidden);
    }
    item.name = name;
    item.image = image;
    item.pull_policy = pull_policy;
    item.description = description;
    apply_environment_scope(&mut item, &scope, &team)?;
    db::update_environment(&db, &item)?;
    let stored = db::environment(&db, &item.id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(json!(stored)))
}

pub async fn delete_environment(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<axum::response::Response> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Owner)?;
    let db = state.db()?;
    let team = db::team_by_id(&db, &team_id)?.ok_or(ApiError::NotFound)?;
    let item = visible_environment(&db, &id, &team)?;
    if !may_manage_environment(&user, &item.scope) {
        return Err(ApiError::Forbidden);
    }
    db::delete_environment(&db, &id)?;
    Ok(no_content())
}

pub async fn cache(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let report = cacheinfo::team_cache(&state.cfg.data_dir, &team);
    Ok(Json(json!({
        "collections_bytes": report.collections_bytes,
        "roles_bytes": report.roles_bytes,
        "tofu_bytes": report.tofu_bytes,
        "collections": report.collections,
        "roles": report.roles,
        "note": "the_cache_lives_under_data_cache_and_stays",
    })))
}

pub async fn plugins(user: AuthUser) -> Json<Value> {
    let _ = user;
    Json(json!(runner::tools()
        .iter()
        .map(|tool| json!({"id": tool.id, "summary": tool.summary}))
        .collect::<Vec<_>>()))
}

pub async fn list_organizations(
    State(state): State<Shared>,
    user: AuthUser,
) -> AppResult<Json<Value>> {
    let db = state.db()?;
    let items = db::list_organizations(&db, &user.id, user.is_admin)?;
    Ok(Json(json!(items)))
}

pub async fn create_organization(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<OrganizationBody>,
) -> AppResult<axum::response::Response> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    let name = require_name(&body.name)?;
    let id = Uuid::new_v4().to_string();
    let db = state.db()?;
    db::insert_organization(&db, &id, &name, body.description.trim())?;
    let item = db::organization(&db, &id)?.ok_or(ApiError::NotFound)?;
    Ok(created(json!(item)))
}

pub async fn get_organization(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let db = state.db()?;
    let item = db::organization(&db, &id)?.ok_or(ApiError::NotFound)?;
    if !user.is_admin
        && !db::list_organizations(&db, &user.id, false)?
            .iter()
            .any(|org| org.id == id)
    {
        return Err(ApiError::Forbidden);
    }
    let teams = db::teams_in_organization(&db, &id)?;
    Ok(Json(json!({
        "id": item.id,
        "name": item.name,
        "description": item.description,
        "created_at": item.created_at,
        "teams": teams.iter().map(team_json).collect::<Vec<_>>(),
    })))
}

pub async fn update_organization(
    State(state): State<Shared>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<OrganizationBody>,
) -> AppResult<Json<Value>> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    let name = require_name(&body.name)?;
    let db = state.db()?;
    db::update_organization(&db, &id, &name, body.description.trim())?;
    let item = db::organization(&db, &id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(json!(item)))
}

fn team_json(team: &db::Team) -> Value {
    json!({
        "id": team.id,
        "name": team.name,
        "description": team.description,
        "ldap_group": team.ldap_group,
        "organization_id": team.organization_id,
        "organization_name": team.organization_name,
        "created_at": team.created_at,
    })
}
