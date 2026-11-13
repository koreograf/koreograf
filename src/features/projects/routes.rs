use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use crate::db::{self, Project};
use crate::error::{ApiError, ApiJson, AppResult};
use crate::features::identity::{AuthUser, Role};
use crate::features::jobs;
use crate::http::{blank, gate, no_content, require_name, team_header};
use crate::Shared;

use super::model::{
    ProjectBody, ProjectListResponse, ProjectResponse, ProjectUpdateBody, SyncProjectResponse,
    SyncResponse,
};
use super::service::{check_team_ids, validate_project_credential, validate_project_source};

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/projects", get(list_projects).post(create_project))
        .route(
            "/projects/{id}",
            get(get_project).put(update_project).delete(delete_project),
        )
        .route("/projects/{id}/sync", post(sync_project))
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
